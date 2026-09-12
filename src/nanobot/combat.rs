//! Deterministic Defender combat and physical Defend Contest presence.

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::battle_statistics::{BattleCounters, BattleEvent};
use bevy::prelude::*;

use crate::nanobot::{
    Charge, DefenderResponse, Health, Nanobot, NanobotType, OwnerSwarm, Structure, StructureKind,
    Swarm, SwarmId, SwarmMember, effective_attack, effective_defense,
};
use crate::spatial::FixedSpatialBuckets;
use crate::structure_sprites::StructureVisual;

/// Defender attack reach in world units.
pub const DEFENDER_ATTACK_RANGE: f32 = 96.0;

/// Baseline fixed-tick interval between delivered Defender attacks.
pub const DEFENDER_ATTACK_INTERVAL_TICKS: u16 = 15;

/// Structure damage multiplier for a fully charged Defender attack.
pub const DEFENDER_STRUCTURE_DAMAGE_FACTOR: f32 = 0.5;

/// Stable support-structure identity retained after gameplay destruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StructureCombatAppearance {
    /// Gameplay structure kind used by simulation.
    pub kind: StructureKind,
    /// Completed presentation identity when the target owns a rendered visual.
    pub visual: Option<StructureVisual>,
}

/// Stable visual identity carried by resolved combat after gameplay changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CombatAppearance {
    Nanobot(NanobotType),
    Structure(StructureCombatAppearance),
}

/// Presentation-ready snapshot captured from one combat participant.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CombatVisualSnapshot {
    pub entity: Entity,
    pub position: Vec2,
    pub swarm: SwarmId,
    pub appearance: CombatAppearance,
}

/// A delivered attack after its gameplay damage has resolved.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ResolvedCombatHit {
    pub attacker: CombatVisualSnapshot,
    pub target: CombatVisualSnapshot,
    pub damage: u32,
    pub target_destroyed: bool,
}

/// Stable victim state for one nanobot destroyed by resolved combat.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ResolvedCombatDeath {
    pub victim: CombatVisualSnapshot,
}

/// Facts published by fixed-step combat for optional render-time presentation.
#[derive(Debug, Clone, Copy, PartialEq, Message)]
pub enum ResolvedCombatFact {
    Hit(ResolvedCombatHit),
    Death(ResolvedCombatDeath),
}

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
    responding: bool,
    cooldown: Option<u16>,
}

impl Combatant {
    fn presentation_snapshot(self) -> CombatVisualSnapshot {
        CombatVisualSnapshot {
            entity: self.entity,
            position: self.position,
            swarm: self.swarm,
            appearance: CombatAppearance::Nanobot(self.kind),
        }
    }
}

#[derive(Clone, Copy)]
struct StructureTarget {
    entity: Entity,
    position: Vec2,
    swarm: SwarmId,
    kind: StructureKind,
    visual: Option<StructureVisual>,
}

impl StructureTarget {
    fn presentation_snapshot(self) -> CombatVisualSnapshot {
        CombatVisualSnapshot {
            entity: self.entity,
            position: self.position,
            swarm: self.swarm,
            appearance: CombatAppearance::Structure(StructureCombatAppearance {
                kind: self.kind,
                visual: self.visual,
            }),
        }
    }
}

#[derive(Clone, Copy)]
enum CombatTarget {
    Nanobot(Combatant),
    Structure(StructureTarget),
}

impl CombatTarget {
    fn entity(self) -> Entity {
        match self {
            Self::Nanobot(target) => target.entity,
            Self::Structure(target) => target.entity,
        }
    }

    fn position(self) -> Vec2 {
        match self {
            Self::Nanobot(target) => target.position,
            Self::Structure(target) => target.position,
        }
    }

    fn swarm(self) -> SwarmId {
        match self {
            Self::Nanobot(target) => target.swarm,
            Self::Structure(target) => target.swarm,
        }
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

fn nominal_damage(contributions: &BTreeMap<SwarmId, u32>) -> u32 {
    contributions.values().copied().fold(0, u32::saturating_add)
}

fn allocate_damage(
    contributions: &BTreeMap<SwarmId, u32>,
    effective_damage: u32,
) -> Vec<(SwarmId, u32)> {
    if effective_damage == 0 {
        return Vec::new();
    }
    let nominal_total = contributions.values().copied().map(u64::from).sum::<u64>();
    debug_assert!(nominal_total > 0);
    let mut allocations = contributions
        .iter()
        .map(|(&swarm, &nominal)| {
            let amount = (u64::from(effective_damage) * u64::from(nominal) / nominal_total) as u32;
            (swarm, amount)
        })
        .collect::<Vec<_>>();
    let allocated = allocations.iter().map(|(_, amount)| *amount).sum::<u32>();
    for (_, amount) in allocations
        .iter_mut()
        .take(effective_damage.saturating_sub(allocated) as usize)
    {
        *amount += 1;
    }
    allocations
}

fn record_combat_damage(
    counters: &mut BattleCounters,
    contributions: &BTreeMap<SwarmId, u32>,
    effective_damage: u32,
    effective_event: fn(u32) -> BattleEvent,
) {
    let effective_allocations = allocate_damage(contributions, effective_damage);
    for (swarm, amount) in effective_allocations {
        if amount > 0 {
            counters.record(swarm, effective_event(amount));
        }
    }
}

/// Resolve one simultaneous attack snapshot. Every responding Defender chooses
/// the nearest hostile in range independently of its pursuit claim; damage is
/// applied after target selection so entity iteration order cannot change the exchange.
#[allow(clippy::type_complexity)]
pub fn defender_combat_system(
    pacing: Res<crate::gameplay_pacing::GameplayPacing>,
    mut combatants: ParamSet<(
        Query<
            (
                Entity,
                &Transform,
                &SwarmMember,
                &NanobotType,
                &Health,
                Option<&Charge>,
                Option<&DefenderResponse>,
                Option<&DefenderAttackCooldown>,
            ),
            With<Nanobot>,
        >,
        Query<(
            Entity,
            &Transform,
            &OwnerSwarm,
            &Structure,
            Option<&StructureVisual>,
        )>,
        Query<&mut Health, With<Nanobot>>,
        Query<&mut Structure>,
    )>,
    swarms: Query<&SwarmId, With<Swarm>>,
    mut commands: Commands,
    mut facts: MessageWriter<ResolvedCombatFact>,
    mut counters: Option<ResMut<BattleCounters>>,
) {
    let snapshot = combatants
        .p0()
        .iter()
        .filter(|(_, _, _, _, health, _, _, _)| health.current > 0)
        .map(
            |(entity, transform, member, kind, _, charge, response, cooldown)| Combatant {
                entity,
                position: transform.translation.truncate(),
                swarm: member.0,
                kind: *kind,
                charge: charge.map(|charge| charge.current),
                responding: response.is_some(),
                cooldown: cooldown.map(|cooldown| cooldown.ticks_remaining),
            },
        )
        .collect::<Vec<_>>();
    let structures = combatants
        .p1()
        .iter()
        .filter_map(|(entity, transform, owner, structure, visual)| {
            if !structure.is_operational() {
                return None;
            }
            Some(StructureTarget {
                entity,
                position: transform.translation.truncate(),
                swarm: swarms.get(owner.0).ok().copied()?,
                kind: structure.kind,
                visual: visual.copied(),
            })
        })
        .collect::<Vec<_>>();
    let mut nanobot_buckets = FixedSpatialBuckets::new(DEFENDER_ATTACK_RANGE);
    for target in snapshot.iter().copied() {
        nanobot_buckets.insert(target.position, target);
    }
    let mut structure_buckets = FixedSpatialBuckets::new(DEFENDER_ATTACK_RANGE);
    for target in structures.iter().copied() {
        structure_buckets.insert(target.position, target);
    }

    let mut nanobot_damage = HashMap::<Entity, BTreeMap<SwarmId, u32>>::new();
    let mut structure_damage = HashMap::<Entity, BTreeMap<SwarmId, u32>>::new();
    let mut resolved_hits = Vec::<ResolvedCombatHit>::new();
    for attacker in snapshot
        .iter()
        .filter(|combatant| combatant.kind == NanobotType::Defender && combatant.responding)
    {
        let attack = effective_attack(attacker.charge.unwrap_or_default());
        let cooldown_ready = attacker.cooldown.is_none_or(|ticks| ticks == 0);
        let mut delivered_attack = false;
        let attacker_bucket = nanobot_buckets.bucket_for_position(attacker.position);
        let nearest_target = nanobot_buckets
            .neighbourhood(attacker_bucket, 1)
            .flat_map(|(_, targets)| targets)
            .map(|target| CombatTarget::Nanobot(*target))
            .chain(
                structure_buckets
                    .neighbourhood(attacker_bucket, 1)
                    .flat_map(|(_, targets)| targets)
                    .map(|target| CombatTarget::Structure(*target)),
            )
            .filter(|target| target.swarm() != attacker.swarm)
            .filter_map(|target| {
                let distance = attacker.position.distance(target.position());
                (distance <= DEFENDER_ATTACK_RANGE).then_some((distance, target))
            })
            .min_by(|(left_distance, left), (right_distance, right)| {
                left_distance
                    .total_cmp(right_distance)
                    .then_with(|| left.entity().to_bits().cmp(&right.entity().to_bits()))
            })
            .map(|(_, target)| target);

        if cooldown_ready {
            match nearest_target {
                Some(CombatTarget::Nanobot(target)) => {
                    let defense = if target.kind == NanobotType::Defender {
                        effective_defense(target.charge.unwrap_or_default())
                    } else {
                        0.0
                    };
                    let damage = damage_after_defense(attack, defense);
                    if damage > 0 {
                        *nanobot_damage
                            .entry(target.entity)
                            .or_default()
                            .entry(attacker.swarm)
                            .or_default() += damage;
                        resolved_hits.push(ResolvedCombatHit {
                            attacker: attacker.presentation_snapshot(),
                            target: target.presentation_snapshot(),
                            damage,
                            target_destroyed: false,
                        });
                        delivered_attack = true;
                    }
                }
                Some(CombatTarget::Structure(target)) => {
                    let damage = structure_hit_damage(attack);
                    if damage > 0 {
                        *structure_damage
                            .entry(target.entity)
                            .or_default()
                            .entry(attacker.swarm)
                            .or_default() += damage;
                        resolved_hits.push(ResolvedCombatHit {
                            attacker: attacker.presentation_snapshot(),
                            target: target.presentation_snapshot(),
                            damage,
                            target_destroyed: false,
                        });
                        delivered_attack = true;
                    }
                }
                None => {}
            }
        }

        if delivered_attack || attacker.cooldown.is_some() {
            commands
                .entity(attacker.entity)
                .insert(DefenderAttackCooldown {
                    ticks_remaining: if delivered_attack {
                        pacing.attack_interval_ticks.saturating_sub(1)
                    } else {
                        attacker.cooldown.unwrap_or_default().saturating_sub(1)
                    },
                });
        }
    }

    let nanobot_snapshots = snapshot
        .iter()
        .map(|combatant| (combatant.entity, combatant.presentation_snapshot()))
        .collect::<HashMap<_, _>>();
    let structure_snapshots = structures
        .iter()
        .map(|structure| (structure.entity, structure.presentation_snapshot()))
        .collect::<HashMap<_, _>>();
    let mut destroyed_targets = HashSet::new();
    let mut resolved_targets = HashSet::new();
    let mut combat_deaths = Vec::new();
    {
        let mut health = combatants.p2();
        for (entity, contributions) in nanobot_damage {
            if let Ok(mut target) = health.get_mut(entity) {
                let was_alive = target.current > 0;
                let previous_health = target.current;
                target.current = target
                    .current
                    .saturating_sub(nominal_damage(&contributions));
                let effective_damage = previous_health - target.current;
                if let Some(counters) = counters.as_deref_mut() {
                    record_combat_damage(
                        counters,
                        &contributions,
                        effective_damage,
                        BattleEvent::EffectiveNanobotDamage,
                    );
                }
                if was_alive {
                    resolved_targets.insert(entity);
                }
                if target.current == 0 {
                    destroyed_targets.insert(entity);
                }
                if was_alive
                    && target.current == 0
                    && let Some(victim) = nanobot_snapshots.get(&entity).copied()
                {
                    combat_deaths.push(ResolvedCombatDeath { victim });
                }
            }
        }
    }
    let mut conditions = combatants.p3();
    for (entity, contributions) in structure_damage {
        if let Ok(mut target) = conditions.get_mut(entity) {
            let was_alive = target.health > 0;
            let previous_health = target.health;
            target.health = target.health.saturating_sub(nominal_damage(&contributions));
            let effective_damage = previous_health - target.health;
            if let Some(counters) = counters.as_deref_mut() {
                record_combat_damage(
                    counters,
                    &contributions,
                    effective_damage,
                    BattleEvent::EffectiveStructureDamage,
                );
            }
            if was_alive {
                resolved_targets.insert(entity);
            }
            if target.health == 0 {
                destroyed_targets.insert(entity);
                if was_alive && let Some(victim) = structure_snapshots.get(&entity).copied() {
                    combat_deaths.push(ResolvedCombatDeath { victim });
                    if let Some(counters) = counters.as_deref_mut() {
                        counters.record(victim.swarm, BattleEvent::StructureLost);
                    }
                }
                commands.entity(entity).despawn();
            }
        }
    }
    for mut hit in resolved_hits {
        if !resolved_targets.contains(&hit.target.entity) {
            continue;
        }
        hit.target_destroyed = destroyed_targets.contains(&hit.target.entity);
        facts.write(ResolvedCombatFact::Hit(hit));
    }
    combat_deaths.sort_by_key(|death| death.victim.entity.to_bits());
    for death in combat_deaths {
        facts.write(ResolvedCombatFact::Death(death));
    }
}

pub struct CombatPlugin;

impl Plugin for CombatPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<crate::gameplay_pacing::GameplayPacing>()
            .add_message::<ResolvedCombatFact>()
            .add_systems(
                FixedUpdate,
                defender_combat_system
                    .in_set(crate::nanobot::NanobotSimulationSet::Combat)
                    .after(crate::nanobot::RegionalAllocationSet::Acquire)
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
