//! Deterministic Defender combat and physical Defend Contest presence.

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::battle_statistics::{BattleCounters, BattleEvent};
use bevy::prelude::*;

use crate::nanobot::{
    Charge, CombatIdentity, CombatIdentityPlugin, Health, Nanobot, NanobotType, OwnerSwarm,
    Structure, StructureKind, Swarm, SwarmId, SwarmMember, combat_key, effective_attack,
    effective_defense,
};
use crate::spatial::FixedSpatialBuckets;
use crate::structure_sprites::StructureVisual;
use crate::tuning::{Tuning, elapsed_fixed_ticks, rescale_ticks};

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

/// Stable victim state for one nanobot or structure destroyed by resolved combat.
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

/// Fixed-tick progress toward acquiring a shot and recovering from the previous one.
#[derive(Debug, Component, Default, Clone, Copy, PartialEq, Eq)]
pub struct DefenderAttackCooldown {
    pub ticks_remaining: u32,
    acquisition_ticks_remaining: Option<u16>,
}

impl DefenderAttackCooldown {
    /// Cooldown for a Defender that has already acquired a target.
    pub fn new(ticks_remaining: u32) -> Self {
        Self {
            ticks_remaining,
            acquisition_ticks_remaining: Some(0),
        }
    }
}

#[derive(Clone, Copy)]
struct Combatant {
    entity: Entity,
    key: u64,
    position: Vec2,
    swarm: SwarmId,
    kind: NanobotType,
    charge: Option<f32>,
    cooldown: Option<DefenderAttackCooldown>,
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
    key: u64,
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
    fn priority(self) -> u8 {
        match self {
            Self::Nanobot(target) if target.kind == NanobotType::Defender => 0,
            Self::Nanobot(_) => 1,
            Self::Structure(_) => 2,
        }
    }

    fn key(self) -> u64 {
        match self {
            Self::Nanobot(target) => target.key,
            Self::Structure(target) => target.key,
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

pub(super) fn combat_hash(key: u64, tick: u64) -> u64 {
    let seed = key.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ tick.rotate_left(32);
    let mixed = (seed ^ (seed >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    let mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    mixed ^ (mixed >> 31)
}

fn hit_interval(key: u64, tick: u64, tuning: &Tuning) -> u32 {
    let interval = i32::from(tuning.combat.attack_interval_ticks);
    let sample = 2.0 * combat_hash(key, tick) as f64 / u64::MAX as f64 - 1.0;
    let jitter = (f64::from(interval) * f64::from(tuning.combat.cadence_jitter) * sample).round();
    (interval + jitter as i32).max(1) as u32
}

fn damage_after_defense(attack: f32, defense: f32) -> u32 {
    if attack <= 0.0 {
        return 0;
    }
    (attack / (1.0 + defense / 10.0)).round().max(1.0) as u32
}

fn structure_hit_damage(attack: f32, tuning: &Tuning) -> u32 {
    damage_after_defense(attack * tuning.combat.structure_damage_factor, 0.0)
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

/// Resolve one simultaneous attack snapshot. Living Defenders prioritize in-range
/// hostile Defenders, other nanobots, then structures. Damage is applied after
/// target selection so entity iteration order cannot change the exchange.
#[allow(clippy::type_complexity)]
pub fn defender_combat_system(
    tuning: Res<Tuning>,
    fixed_time: Option<Res<Time<Fixed>>>,
    mut combatants: ParamSet<(
        Query<
            (
                Entity,
                &Transform,
                &SwarmMember,
                &NanobotType,
                &Health,
                Option<&Charge>,
                Option<&mut DefenderAttackCooldown>,
                Option<&CombatIdentity>,
            ),
            With<Nanobot>,
        >,
        Query<(
            Entity,
            &Transform,
            &OwnerSwarm,
            &Structure,
            Option<&StructureVisual>,
            Option<&CombatIdentity>,
        )>,
        Query<&mut Health, With<Nanobot>>,
        Query<&mut Structure>,
    )>,
    swarms: Query<&SwarmId, With<Swarm>>,
    mut commands: Commands,
    mut facts: MessageWriter<ResolvedCombatFact>,
    mut counters: Option<ResMut<BattleCounters>>,
) {
    let tick = fixed_time.as_deref().map_or(0, elapsed_fixed_ticks);
    let snapshot = combatants
        .p0()
        .iter()
        .filter(|(_, _, _, _, health, ..)| health.current > 0)
        .map(
            |(entity, transform, member, kind, _, charge, cooldown, identity)| Combatant {
                entity,
                key: combat_key(entity, identity),
                position: transform.translation.truncate(),
                swarm: member.0,
                kind: *kind,
                charge: charge.map(|charge| charge.current),
                cooldown: cooldown.copied(),
            },
        )
        .collect::<Vec<_>>();
    let structures = combatants
        .p1()
        .iter()
        .filter_map(|(entity, transform, owner, structure, visual, identity)| {
            if !structure.is_operational() {
                return None;
            }
            Some(StructureTarget {
                entity,
                key: combat_key(entity, identity),
                position: transform.translation.truncate(),
                swarm: swarms.get(owner.0).ok().copied()?,
                kind: structure.kind,
                visual: visual.copied(),
            })
        })
        .collect::<Vec<_>>();
    let mut target_buckets = FixedSpatialBuckets::new(tuning.combat.attack_range);
    for target in snapshot.iter().copied() {
        target_buckets.insert(target.position, CombatTarget::Nanobot(target));
    }
    for target in structures.iter().copied() {
        target_buckets.insert(target.position, CombatTarget::Structure(target));
    }

    let mut nanobot_damage = BTreeMap::<(u64, Entity), BTreeMap<SwarmId, u32>>::new();
    let mut structure_damage = BTreeMap::<(u64, Entity), BTreeMap<SwarmId, u32>>::new();
    let mut resolved_hits = Vec::<(u64, ResolvedCombatHit)>::new();
    let mut cooldown_updates = Vec::new();
    let range_squared = tuning.combat.attack_range.powi(2);
    for attacker in snapshot
        .iter()
        .filter(|combatant| combatant.kind == NanobotType::Defender)
    {
        let previous = attacker.cooldown.unwrap_or_default();
        let acquisition_delay = previous.acquisition_ticks_remaining.unwrap_or_else(|| {
            (combat_hash(attacker.key, 0) % u64::from(tuning.combat.attack_interval_ticks)) as u16
        });
        let ready = previous.ticks_remaining == 0 && acquisition_delay == 0;
        let attacker_bucket = target_buckets.bucket_for_position(attacker.position);
        let mut targets_in_range = target_buckets
            .neighbourhood(attacker_bucket, 1)
            .flat_map(|(_, targets)| targets)
            .copied()
            .filter(|target| target.swarm() != attacker.swarm)
            .filter_map(|target| {
                let distance = attacker.position.distance_squared(target.position());
                (distance <= range_squared).then_some((distance, target))
            });
        let target = if ready {
            targets_in_range.min_by(|(left_distance, left), (right_distance, right)| {
                left.priority()
                    .cmp(&right.priority())
                    .then_with(|| left_distance.total_cmp(right_distance))
                    .then_with(|| left.key().cmp(&right.key()))
            })
        } else {
            targets_in_range.next()
        };
        // Losing every target cancels acquisition, while recovery from a hit
        // continues. After reacquisition, both clocks must finish before firing.
        let mut cooldown = DefenderAttackCooldown {
            ticks_remaining: previous.ticks_remaining.saturating_sub(1),
            acquisition_ticks_remaining: target.map(|_| acquisition_delay.saturating_sub(1)),
        };

        if ready && let Some((_, target)) = target {
            let attack = effective_attack(attacker.charge.unwrap_or_default(), &tuning);
            let (damage, target_snapshot, target_damage) = match target {
                CombatTarget::Nanobot(target) => {
                    let defense = if target.kind == NanobotType::Defender {
                        effective_defense(target.charge.unwrap_or_default(), &tuning)
                    } else {
                        0.0
                    };
                    (
                        damage_after_defense(attack, defense),
                        target.presentation_snapshot(),
                        &mut nanobot_damage,
                    )
                }
                CombatTarget::Structure(target) => (
                    structure_hit_damage(attack, &tuning),
                    target.presentation_snapshot(),
                    &mut structure_damage,
                ),
            };
            if damage > 0 {
                let contribution = target_damage
                    .entry((target.key(), target_snapshot.entity))
                    .or_default()
                    .entry(attacker.swarm)
                    .or_default();
                *contribution = contribution.saturating_add(damage);
                resolved_hits.push((
                    attacker.key,
                    ResolvedCombatHit {
                        attacker: attacker.presentation_snapshot(),
                        target: target_snapshot,
                        damage,
                        target_destroyed: false,
                    },
                ));
                cooldown.ticks_remaining =
                    hit_interval(attacker.key, tick, &tuning).saturating_sub(1);
            }
        }

        if cooldown != previous {
            cooldown_updates.push((attacker.key, attacker.entity, cooldown));
        }
    }
    {
        let mut attackers = combatants.p0();
        cooldown_updates.sort_unstable_by_key(|(key, ..)| *key);
        for (_, entity, cooldown) in cooldown_updates {
            if let Ok((.., Some(mut current), _)) = attackers.get_mut(entity) {
                *current = cooldown;
            } else {
                commands.entity(entity).insert(cooldown);
            }
        }
    }
    if resolved_hits.is_empty() {
        return;
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
        for ((key, entity), contributions) in nanobot_damage {
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
                    combat_deaths.push((key, ResolvedCombatDeath { victim }));
                }
            }
        }
    }
    let mut conditions = combatants.p3();
    for ((key, entity), contributions) in structure_damage {
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
                    combat_deaths.push((key, ResolvedCombatDeath { victim }));
                    if let Some(counters) = counters.as_deref_mut() {
                        counters.record(victim.swarm, BattleEvent::StructureLost);
                    }
                }
                commands.entity(entity).despawn();
            }
        }
    }
    resolved_hits.sort_unstable_by_key(|(key, _)| *key);
    for (_, mut hit) in resolved_hits {
        if !resolved_targets.contains(&hit.target.entity) {
            continue;
        }
        hit.target_destroyed = destroyed_targets.contains(&hit.target.entity);
        facts.write(ResolvedCombatFact::Hit(hit));
    }
    combat_deaths.sort_unstable_by_key(|(key, _)| *key);
    for (_, death) in combat_deaths {
        facts.write(ResolvedCombatFact::Death(death));
    }
}

pub fn reconcile_combat_tuning(world: &mut World, old: &Tuning, new: &Tuning) {
    if old.combat.attack_interval_ticks != new.combat.attack_interval_ticks {
        let old_interval = u32::from(old.combat.attack_interval_ticks);
        let new_interval = u32::from(new.combat.attack_interval_ticks);
        let max_jitter = (f64::from(new_interval) * f64::from(old.combat.cadence_jitter)).round();
        let max_cooldown = (new_interval + max_jitter as u32).saturating_sub(1);
        for mut cooldown in world.query::<&mut DefenderAttackCooldown>().iter_mut(world) {
            cooldown.ticks_remaining =
                rescale_ticks(cooldown.ticks_remaining, old_interval, new_interval)
                    .min(max_cooldown);
            if let Some(remaining) = cooldown.acquisition_ticks_remaining.as_mut() {
                *remaining = rescale_ticks(u32::from(*remaining), old_interval, new_interval)
                    .min(new_interval - 1) as u16;
            }
        }
    }
    if old.combat.nanobot_max_health != new.combat.nanobot_max_health {
        for mut health in world
            .query_filtered::<&mut Health, With<Nanobot>>()
            .iter_mut(world)
        {
            health.current = if health.current == 0 {
                0
            } else {
                ((u64::from(health.current) * u64::from(new.combat.nanobot_max_health)
                    / u64::from(health.max.max(1))) as u32)
                    .clamp(1, new.combat.nanobot_max_health)
            };
            health.max = new.combat.nanobot_max_health;
        }
    }
}

pub struct CombatPlugin;

impl Plugin for CombatPlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<CombatIdentityPlugin>() {
            app.add_plugins(CombatIdentityPlugin);
        }
        app.init_resource::<Tuning>()
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
