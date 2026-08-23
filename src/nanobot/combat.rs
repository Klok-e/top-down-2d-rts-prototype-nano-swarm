//! Deterministic Defender combat and Defend-cell threat pressure.

use std::collections::{HashMap, HashSet};

use bevy::prelude::*;

use crate::intent::{IntentGrid, IntentKind};
use crate::nanobot::{
    Charge, DefendHold, DefendPressure, DirectMovementComponent, Health, Nanobot, NanobotType,
    OwnerSwarm, Structure, Swarm, SwarmId, SwarmMember, effective_attack, effective_defense,
    world_to_cell,
};
use crate::spatial::FixedSpatialBuckets;

/// Defender attack reach in world units.
pub const DEFENDER_ATTACK_RANGE: f32 = 96.0;

/// Fixed-tick interval between delivered Defender attacks.
pub const DEFENDER_ATTACK_INTERVAL_TICKS: u16 = 15;

/// Structure damage multiplier for a fully charged Defender attack.
pub const DEFENDER_STRUCTURE_DAMAGE_FACTOR: f32 = 0.5;

/// Per-Defender cooldown after a delivered attack.
#[derive(Debug, Component, Clone, Copy, PartialEq, Eq)]
pub struct DefenderAttackCooldown {
    pub ticks_remaining: u16,
}

#[derive(Clone, Copy)]
struct Combatant {
    entity: Entity,
    position: Vec2,
    swarm: SwarmId,
    kind: NanobotType,
    charge: Option<f32>,
    hold: Option<IVec2>,
    cooldown: Option<u16>,
}

#[derive(Clone, Copy)]
struct StructureTarget {
    entity: Entity,
    position: Vec2,
    swarm: SwarmId,
}

/// Resolve tracked Defend contests after both swarms have reached the cell.
/// Zero-health Defenders no longer count as holders, allowing the survivor to
/// claim the existing Defend layer before the next allocation projection.
pub fn defend_contest_resolution_system(
    mut grid: ResMut<IntentGrid>,
    defenders: Query<(&DefendHold, &SwarmMember, &Health), With<Nanobot>>,
) {
    let occupants = defenders
        .iter()
        .filter(|(_, _, health)| health.current > 0)
        .map(|(hold, member, _)| (hold.cell, member.0))
        .collect::<HashSet<_>>();
    for (cell, incumbent, challenger) in grid.defend_contests() {
        grid.update_defend_contest_presence(
            cell,
            occupants.contains(&(cell, incumbent)),
            occupants.contains(&(cell, challenger)),
        );
    }
}

fn damage_after_defense(attack: f32, defense: f32) -> u32 {
    if attack <= 0.0 {
        return 0;
    }
    (attack / (1.0 + defense / 10.0)).round().max(1.0) as u32
}

fn structure_hit_damage(attack: f32) -> u32 {
    damage_after_defense(attack * DEFENDER_STRUCTURE_DAMAGE_FACTOR, 0.0)
}

/// Rebuild pressure from hostile nanobots physically occupying owned Defend cells.
pub fn defend_threat_pressure_system(
    grid: Res<IntentGrid>,
    nanobots: Query<(&Transform, &SwarmMember), With<Nanobot>>,
    mut pressure: ResMut<DefendPressure>,
) {
    let mut hostile_counts = HashMap::<(SwarmId, IVec2), u32>::new();
    for (transform, member) in &nanobots {
        let cell = world_to_cell(transform.translation.truncate());
        let Some(intent) = grid.cell(cell) else {
            continue;
        };
        if let Some((incumbent, challenger)) = grid.defend_contest(cell) {
            for owner in [incumbent, challenger] {
                if member.0 != owner {
                    *hostile_counts.entry((owner, cell)).or_default() += 1;
                }
            }
            continue;
        }
        let Some(owner) = intent.owner(IntentKind::Defend) else {
            continue;
        };
        if member.0 != owner {
            *hostile_counts.entry((owner, cell)).or_default() += 1;
        }
    }

    let mut next = DefendPressure::default();
    for ((owner, cell), hostile_count) in hostile_counts {
        next.set_for(owner, cell, 1.0 + hostile_count as f32);
    }
    if *pressure != next {
        *pressure = next;
    }
}

/// Resolve one simultaneous attack snapshot. Every holding Defender chooses a
/// hostile nanobot first, then a hostile support structure; damage is applied
/// after target selection so entity iteration order cannot change the exchange.
#[allow(clippy::type_complexity)]
pub fn defender_combat_system(
    mut combatants: ParamSet<(
        Query<
            (
                Entity,
                &Transform,
                &SwarmMember,
                &NanobotType,
                Option<&Charge>,
                Option<&DefendHold>,
                Option<&DefenderAttackCooldown>,
            ),
            With<Nanobot>,
        >,
        Query<(Entity, &Transform, &OwnerSwarm), With<Structure>>,
        Query<&mut Health, With<Nanobot>>,
        Query<&mut Structure>,
    )>,
    swarms: Query<&SwarmId, With<Swarm>>,
    mut commands: Commands,
) {
    let snapshot = combatants
        .p0()
        .iter()
        .map(
            |(entity, transform, member, kind, charge, hold, cooldown)| Combatant {
                entity,
                position: transform.translation.truncate(),
                swarm: member.0,
                kind: *kind,
                charge: charge.map(|charge| charge.current),
                hold: hold.map(|hold| hold.cell),
                cooldown: cooldown.map(|cooldown| cooldown.ticks_remaining),
            },
        )
        .collect::<Vec<_>>();
    let structures = combatants
        .p1()
        .iter()
        .filter_map(|(entity, transform, owner)| {
            Some(StructureTarget {
                entity,
                position: transform.translation.truncate(),
                swarm: swarms.get(owner.0).ok().copied()?,
            })
        })
        .collect::<Vec<_>>();
    let mut structures_by_cell = HashMap::<IVec2, Vec<StructureTarget>>::new();
    for target in structures.iter().copied() {
        structures_by_cell
            .entry(world_to_cell(target.position))
            .or_default()
            .push(target);
    }
    let mut nanobot_buckets = FixedSpatialBuckets::new(DEFENDER_ATTACK_RANGE);
    for target in snapshot.iter().copied() {
        nanobot_buckets.insert(target.position, target);
    }
    let mut nanobots_by_cell = HashMap::<IVec2, Vec<Combatant>>::new();
    for target in snapshot.iter().copied() {
        nanobots_by_cell
            .entry(world_to_cell(target.position))
            .or_default()
            .push(target);
    }
    let mut structure_buckets = FixedSpatialBuckets::new(DEFENDER_ATTACK_RANGE);
    for target in structures.iter().copied() {
        structure_buckets.insert(target.position, target);
    }

    let mut nanobot_damage = HashMap::<Entity, u32>::new();
    let mut structure_damage = HashMap::<Entity, u32>::new();
    for attacker in snapshot
        .iter()
        .filter(|combatant| combatant.kind == NanobotType::Defender && combatant.hold.is_some())
    {
        let attack = effective_attack(attacker.charge.unwrap_or_default());
        let cooldown_ready = attacker.cooldown.is_none_or(|ticks| ticks == 0);
        let mut delivered_attack = false;
        let attacker_bucket = nanobot_buckets.bucket_for_position(attacker.position);
        let nanobot_target = nanobot_buckets
            .neighbourhood(attacker_bucket, 1)
            .flat_map(|(_, targets)| targets)
            .filter(|target| target.swarm != attacker.swarm)
            .filter_map(|target| {
                let distance = attacker.position.distance(target.position);
                (distance <= DEFENDER_ATTACK_RANGE).then_some((distance, target))
            })
            .min_by(|(left_distance, left), (right_distance, right)| {
                left_distance
                    .total_cmp(right_distance)
                    .then_with(|| left.entity.to_bits().cmp(&right.entity.to_bits()))
            })
            .map(|(_, target)| target);
        if let Some(target) = nanobot_target {
            let defense = if target.kind == NanobotType::Defender {
                effective_defense(target.charge.unwrap_or_default())
            } else {
                0.0
            };
            if cooldown_ready {
                let damage = damage_after_defense(attack, defense);
                if damage > 0 {
                    *nanobot_damage.entry(target.entity).or_default() += damage;
                    delivered_attack = true;
                }
            }
        } else {
            let held_cell = attacker.hold.expect("holding attacker has a Defend cell");
            let approach_target = nanobots_by_cell
                .get(&held_cell)
                .into_iter()
                .flatten()
                .filter(|target| target.swarm != attacker.swarm)
                .min_by(|left, right| {
                    attacker
                        .position
                        .distance(left.position)
                        .total_cmp(&attacker.position.distance(right.position))
                        .then_with(|| left.entity.to_bits().cmp(&right.entity.to_bits()))
                });
            if let Some(target) = approach_target {
                commands
                    .entity(attacker.entity)
                    .insert(DirectMovementComponent {
                        xy: target.position,
                        stop_radius: DEFENDER_ATTACK_RANGE,
                    });
            } else {
                let structure_target = structure_buckets
                    .neighbourhood(attacker_bucket, 1)
                    .flat_map(|(_, targets)| targets)
                    .filter(|target| target.swarm != attacker.swarm)
                    .filter_map(|target| {
                        let distance = attacker.position.distance(target.position);
                        (distance <= DEFENDER_ATTACK_RANGE).then_some((distance, target.entity))
                    })
                    .min_by(|(left_distance, left), (right_distance, right)| {
                        left_distance
                            .total_cmp(right_distance)
                            .then_with(|| left.to_bits().cmp(&right.to_bits()))
                    })
                    .map(|(_, entity)| entity);
                if let Some(target) = structure_target {
                    if cooldown_ready {
                        let damage = structure_hit_damage(attack);
                        if damage > 0 {
                            *structure_damage.entry(target).or_default() += damage;
                            delivered_attack = true;
                        }
                    }
                } else if let Some(target) = structures_by_cell
                    .get(&held_cell)
                    .into_iter()
                    .flatten()
                    .filter(|target| target.swarm != attacker.swarm)
                    .min_by(|left, right| {
                        attacker
                            .position
                            .distance(left.position)
                            .total_cmp(&attacker.position.distance(right.position))
                            .then_with(|| left.entity.to_bits().cmp(&right.entity.to_bits()))
                    })
                {
                    commands
                        .entity(attacker.entity)
                        .insert(DirectMovementComponent {
                            xy: target.position,
                            stop_radius: DEFENDER_ATTACK_RANGE,
                        });
                }
            }
        }

        if delivered_attack || attacker.cooldown.is_some() {
            commands
                .entity(attacker.entity)
                .insert(DefenderAttackCooldown {
                    ticks_remaining: if delivered_attack {
                        DEFENDER_ATTACK_INTERVAL_TICKS.saturating_sub(1)
                    } else {
                        attacker.cooldown.unwrap_or_default().saturating_sub(1)
                    },
                });
        }
    }

    {
        let mut health = combatants.p2();
        for (entity, amount) in nanobot_damage {
            if let Ok(mut target) = health.get_mut(entity) {
                target.current = target.current.saturating_sub(amount);
            }
        }
    }
    let mut conditions = combatants.p3();
    for (entity, amount) in structure_damage {
        if let Ok(mut target) = conditions.get_mut(entity) {
            target.health = target.health.saturating_sub(amount);
            if target.health == 0 {
                commands.entity(entity).despawn();
            }
        }
    }
}

pub struct CombatPlugin;

impl Plugin for CombatPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DefendPressure>()
            .add_systems(
                FixedUpdate,
                defend_contest_resolution_system
                    .in_set(crate::nanobot::NanobotSimulationSet::Threat)
                    .before(defend_threat_pressure_system)
                    .before(crate::nanobot::RegionalAllocationSet::Project),
            )
            .add_systems(
                FixedUpdate,
                defend_threat_pressure_system
                    .in_set(crate::nanobot::NanobotSimulationSet::Threat)
                    .before(crate::nanobot::RegionalAllocationSet::Project),
            )
            .add_systems(
                FixedUpdate,
                defender_combat_system
                    .in_set(crate::nanobot::NanobotSimulationSet::Combat)
                    .after(crate::nanobot::defender_hold_system)
                    .after(crate::nanobot::defender_charger_work_system),
            );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defense_reduces_but_does_not_negate_damage() {
        let undefended = damage_after_defense(10.0, 0.0);
        let defended = damage_after_defense(10.0, 10.0);
        assert!(defended > 0);
        assert!(defended < undefended);
    }
}
