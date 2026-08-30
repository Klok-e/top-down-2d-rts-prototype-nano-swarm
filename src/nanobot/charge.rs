//! Defender Charge sustain loop and Charger logistics.
//!
//! Issue #14 contract: Chargers are automatic structures that
//! resupply defenders with `Charge`. Defenders use Charge, low
//! Charge weakens their attack and defense, empty/ignored
//! Charge causes defender health loss, and defenders rotate to
//! working chargers automatically. Unserved low Charge creates
//! owner-scoped Charger plans in eligible Defend paint, and Chargers require
//! logistics support via physical resources so isolated defenses
//! degrade when haulers cannot reach them.
//!
//! Defender state machine carried on the defender by marker
//! components:
//!
//! ```text
//!   Current duty (Charge)
//!     -> (charge low + working charger available)
//!     -> Moving (ChargerAssignment + DMC, prior allocation released)
//!   Moving
//!     -> (arrive at charger)
//!     -> Charging (ChargerAssignment + ChargerProgress)
//!   Charging
//!     -> (charge full OR charger empty)
//!     -> Unengaged (markers cleared; current allocation re-picks)
//! ```
//!
//! A Defender without available service continues its current duty while empty
//! Charge drains health. Charge departure releases current regional ownership;
//! completion or invalidation returns through current allocation without
//! reclaiming prior work.
//!
//! Logistics: a `Charger` carries a `Stockpile`-shaped physical
//! buffer of `ResourceKind::Minerals`. Defenders charging from
//! the buffer consume one mineral per supplied pulse; when the
//! buffer is empty, the charger is not "working" and defenders
//! will not rotate to it. Haulers (issue #8) deliver material
//! to the buffer so a defended cell with active logistics
//! stays charged. A defended cell with no haulers reaching it
//! gradually loses charger material and the defenders degrade.

use std::collections::HashMap;

use bevy::prelude::*;

use crate::intent::{IntentGrid, IntentKind};
use crate::nanobot::allocation::DefenderResponse;
use crate::nanobot::allocation::RegionalLease;
use crate::nanobot::allocation::runtime::RegionalAllocationWake;
use crate::nanobot::autonomy::NanobotType;
use crate::nanobot::components::{
    DirectMovementComponent, Health, Nanobot, Swarm, SwarmId, SwarmMember,
};
use crate::nanobot::maintenance::SupportCondition;
use crate::nanobot::placement::{
    BUILDING_FOOTPRINT_RADIUS, find_nearest_defend_zone_placement, scaled_building_footprint_radius,
};
use crate::nanobot::planned::{PlannedKind, PlannedStructure, planned_visual_components};
use crate::nanobot::production::{OwnerSwarm, ProductionFacility};
use crate::resources::{ResourceDeposit, ResourceKind, ResourceLedger, Stockpile};
use crate::structure_sprites::StructureSprites;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Maximum value of a defender's `Charge::current`. The
/// component is a `f32` so the score can drain smoothly
/// without quantising to integers. A defender with `current ==
/// MAX_CHARGE` is "fully charged"; a defender with `current ==
/// 0.0` is "empty" and starts losing health if not at a
/// working charger.
pub const MAX_CHARGE: f32 = 1.0;

/// Passive Charge drain per fixed simulation tick.
pub const CHARGE_DRAIN_PER_TICK: f32 = 0.00025;

/// Number of fixed ticks between supplied recharge pulses.
pub const CHARGE_PULSE_INTERVAL_TICKS: u16 = 10;

/// Charge granted by one supplied recharge pulse.
pub const CHARGE_PER_PULSE: f32 = 0.03;

/// Minerals consumed by one supplied recharge pulse.
pub const CHARGER_MATERIAL_PER_PULSE: u32 = 1;

/// Charge level below which a defender's attack and defense
/// are weakened. Above the threshold the modifier is 1.0;
/// below it the modifier scales linearly with charge so an
/// empty defender has 0.0 attack and 0.0 defense. The
/// threshold sits well above zero so a defender that just
/// lost the weaken threshold is still in "weak but alive"
/// territory rather than "instantly dead".
pub const WEAKENED_CHARGE_THRESHOLD: f32 = 0.3;

/// Charge level at or below which a Defender is
/// eligible to rotate to a working charger. The threshold sits
/// a notch above [`WEAKENED_CHARGE_THRESHOLD`] so a defender
/// starts looking for a charger *before* it is too weak to
/// fight, keeping the sustain loop preventative rather than
/// reactive. A defender whose charge is between
/// `LOW_CHARGE_THRESHOLD` and `WEAKENED_CHARGE_THRESHOLD` is
/// still weakened but not yet rotating; below the low
/// threshold the rotation kicks in.
pub const LOW_CHARGE_THRESHOLD: f32 = 0.5;

/// Fixed ticks between health damage pulses at empty charge.
pub const EMPTY_CHARGE_DAMAGE_INTERVAL_TICKS: u8 = 6;

/// Health lost by one empty-charge damage pulse.
pub const EMPTY_CHARGE_HEALTH_DAMAGE: u32 = 1;

/// Material cost (in `ResourceKind::Minerals`) to fully stock
/// a freshly auto-created charger. Sized to fit at least one
/// full hauler load ([`crate::nanobot::haul::HAULER_CARRY_CAPACITY`])
/// with headroom so the first hauler trip can complete in
/// one go, and to give a defended cell a meaningful logistics
/// target before the next visit is needed.
pub const AUTO_CHARGER_CAPACITY: u32 = 60;

/// World-units reach of a charger's charging radius. Matches
/// the default stockpile radius so the hauler's "free
/// space" reasoning and the defender's "am I in range?"
/// reasoning share the same scale. A charger with the default
/// radius covers its own cell with a comfortable margin.
pub const AUTO_CHARGER_RADIUS: f32 = 64.0;

/// Material buffer an auto-created charger starts with. New chargers begin
/// empty so all minerals enter through physical logistics.
pub const AUTO_CHARGER_INITIAL_AMOUNT: u32 = 0;

/// Maximum Defenders that one completed or pending Charger reserves in the
/// swarm-wide service pool. An unserved low-Charge Defender creates another
/// plan only after all reserved slots are consumed.
pub const MAX_DEFENDERS_PER_CHARGER: u32 = 3;

// ---------------------------------------------------------------------------
// Components
// ---------------------------------------------------------------------------

/// A local support structure that refills defender Charge.
/// Planned automatically in owned Defend paint by
/// [`charger_auto_creation_system`] when observed low-Charge service need
/// exceeds available capacity, then refilled physically by haulers (the
/// hauler's sink selection includes chargers with free space).
///
/// `amount` is the physical resource buffer; when `amount == 0` the
/// charger is "empty" and is not a valid rotation target. `capacity`
/// caps the buffer; freshly completed chargers begin empty.
#[derive(Debug, Component, Clone, Copy)]
pub struct Charger {
    /// Physical cell used to validate matching owner-scoped Defend paint for
    /// Defender service and Charger-specific Maintenance.
    pub cell: IVec2,
    /// Resource backing the charger. Always
    /// [`ResourceKind::Minerals`] in the first implementation;
    /// follows the project's "shared cost" pattern.
    pub kind: ResourceKind,
    /// Current amount of `kind` in the charger's buffer. A
    /// charger with `amount == 0` cannot refill defenders.
    pub amount: u32,
    /// Maximum amount of `kind` the charger can hold.
    pub capacity: u32,
    /// World-units radius at which a defender can charge from
    /// this charger. The default
    /// ([`AUTO_CHARGER_RADIUS`]) covers the cell with a
    /// comfortable margin.
    pub radius: f32,
}

impl Charger {
    /// Build a new empty charger in `cell` with default kind, capacity,
    /// and radius. Used by auto-creation and tests.
    pub fn new(cell: IVec2) -> Self {
        Self {
            cell,
            kind: AUTO_CHARGER_KIND,
            amount: AUTO_CHARGER_INITIAL_AMOUNT,
            capacity: AUTO_CHARGER_CAPACITY,
            radius: AUTO_CHARGER_RADIUS,
        }
    }

    /// True when the charger still has material to give. A
    /// defender will not rotate to a charger that returns
    /// `false` from this method.
    pub fn has_supply(&self) -> bool {
        self.amount > 0
    }

    /// Free capacity for hauler delivery. Mirrors
    /// [`Stockpile::free_space`] so the same hauler sink
    /// selection can use both kinds interchangeably.
    pub fn free_space(&self) -> u32 {
        self.capacity.saturating_sub(self.amount)
    }
}

/// Default kind for an auto-created charger.
pub const AUTO_CHARGER_KIND: ResourceKind = ResourceKind::Minerals;

/// Defender sustain resource. Inserted on every Defender; the
/// charge systems filter on `With<Charge>` so the rest of the
/// simulation can stay oblivious to it. Only Defenders carry
/// this component -- per the issue's "only Defenders use
/// Charge" acceptance criterion, the assignment and rotation
/// systems both gate on `NanobotType::Defender`.
///
/// `current` is in `[0, max]`. `max` is fixed at
/// [`MAX_CHARGE`] in the first implementation; the field is
/// on the component so a future "veteran defender with a
/// bigger battery" issue can extend the contract without
/// changing the type shape.
#[derive(Debug, Component, Clone, Copy)]
pub struct Charge {
    pub current: f32,
    pub max: f32,
}

impl Default for Charge {
    fn default() -> Self {
        Self {
            current: MAX_CHARGE,
            max: MAX_CHARGE,
        }
    }
}

impl Charge {
    /// True when `current` has reached or exceeded `max`. The
    /// work system uses this to return a Defender through the
    /// current response and staging allocation.
    pub fn is_full(&self) -> bool {
        self.current >= self.max
    }

    /// True when `current <= 0.0`. The health-loss system uses
    /// this to decide which Defenders take empty-Charge damage.
    pub fn is_empty(&self) -> bool {
        self.current <= 0.0
    }

    /// True when the charge is low enough that the rotation
    /// system should route the Defender toward a working Charger.
    pub fn needs_rotation(&self) -> bool {
        self.current <= LOW_CHARGE_THRESHOLD
    }
}

/// Marks a Defender as committed to a specific Charger. Set
/// by the rotation system when the defender's charge is low
/// and a working charger is reachable; retained while the
/// defender is in transit or charging, then cleared when
/// the work system finishes the charging cycle.
#[derive(Debug, Component, Clone, Copy)]
pub struct ChargerAssignment {
    pub charger: Entity,
}

/// Marks a Defender that has arrived at its assigned charger
/// and is currently being refilled. The `Without<ChargerProgress>`
/// filter on the rotation system makes the charging phase
/// idempotent: a defender that is already charging is not
/// re-rotated.
#[derive(Debug, Component, Clone, Copy)]
pub struct ChargerProgress {
    pub charger: Entity,
}

/// Fixed-tick progress toward the next supplied recharge pulse.
#[derive(Debug, Component, Clone, Copy, Default)]
pub struct ChargerPulseProgress {
    pub ticks_elapsed: u16,
}

/// Fixed-tick progress toward the next empty-charge damage pulse.
#[derive(Debug, Component, Clone, Copy, Default)]
pub struct EmptyChargeProgress {
    pub ticks_elapsed: u8,
}

// ---------------------------------------------------------------------------
// Pure helpers
// ---------------------------------------------------------------------------

/// Duty priority used when Charge rotation capacity is scarce.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DefenderRotationDuty {
    Staged,
    Tactical,
}

/// Observable inputs used to choose which low-Charge Defenders rotate.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DefenderRotationCandidate {
    pub entity: Entity,
    pub charge: f32,
    pub duty: DefenderRotationDuty,
}

/// Maximum simultaneous Charge rotations for one living Defender population.
pub fn defender_rotation_capacity(living_defenders: u32) -> u32 {
    if living_defenders == 0 {
        0
    } else {
        (living_defenders / 2).max(1)
    }
}

/// Choose low-Charge Defenders for remaining swarm-wide rotation capacity.
pub fn select_defenders_for_rotation(
    candidates: &[DefenderRotationCandidate],
    living_defenders: u32,
    already_rotating: u32,
) -> Vec<Entity> {
    let remaining = defender_rotation_capacity(living_defenders).saturating_sub(already_rotating);
    let mut ordered = candidates.to_vec();
    ordered.sort_by(|left, right| {
        left.charge
            .total_cmp(&right.charge)
            .then_with(|| left.duty.cmp(&right.duty))
            .then_with(|| left.entity.to_bits().cmp(&right.entity.to_bits()))
    });
    ordered
        .into_iter()
        .take(remaining as usize)
        .map(|candidate| candidate.entity)
        .collect()
}

/// Minerals consumed while refilling one defender from `current` to `max`.
/// Each pulse spans [`CHARGE_PULSE_INTERVAL_TICKS`] drain ticks, then grants
/// [`CHARGE_PER_PULSE`] charge when one mineral is available.
pub fn minerals_to_fully_charge(current: f32, max: f32) -> u32 {
    if current >= max || max <= 0.0 {
        return 0;
    }
    let net_refill =
        CHARGE_PER_PULSE - CHARGE_DRAIN_PER_TICK * f32::from(CHARGE_PULSE_INTERVAL_TICKS);
    debug_assert!(net_refill > 0.0);
    let missing_ticks = (max - current.max(0.0)) / net_refill;
    let rounding_tolerance = f32::EPSILON * missing_ticks.abs().max(1.0) * 8.0;
    (missing_ticks - rounding_tolerance).ceil() as u32 * CHARGER_MATERIAL_PER_PULSE
}

/// Linear multiplier in `[0, 1]` derived from `charge`. A
/// defender at full charge has a `1.0` multiplier; a defender
/// at or above [`WEAKENED_CHARGE_THRESHOLD`] is treated as
/// "still strong" and also has a `1.0` multiplier. Below the
/// threshold the multiplier scales linearly with charge so an
/// empty defender has `0.0` attack and `0.0` defense. The
/// function is pure and lives next to the charge data so unit
/// tests can pin the contract without a Bevy `App`.
pub fn charge_strength_multiplier(charge: f32) -> f32 {
    if charge >= WEAKENED_CHARGE_THRESHOLD {
        return 1.0;
    }
    if charge <= 0.0 {
        return 0.0;
    }
    charge / WEAKENED_CHARGE_THRESHOLD
}

/// Effective attack for a defender with `charge` current. The
/// base attack is a project constant; the multiplier comes
/// from [`charge_strength_multiplier`]. Combat uses this
/// value for delivered Defender damage.
pub fn effective_attack(charge: f32) -> f32 {
    DEFENDER_BASE_ATTACK * charge_strength_multiplier(charge)
}

/// Effective defense for a defender with `charge` current.
/// Mirrors [`effective_attack`]: base defense scaled by the
/// charge multiplier. Combat uses this value when resolving
/// incoming Defender attacks.
pub fn effective_defense(charge: f32) -> f32 {
    DEFENDER_BASE_DEFENSE * charge_strength_multiplier(charge)
}

/// Base attack a defender deals when fully charged. A
/// "shared combat stats" constant for the first
/// implementation; the project glossary does not pin a
/// specific number so this is a sensible unit-scale value
/// that makes the test math obvious.
pub const DEFENDER_BASE_ATTACK: f32 = 10.0;

/// Base defense a defender has when fully charged. Mirror of
/// [`DEFENDER_BASE_ATTACK`] for the defense side of the
/// combat stats.
pub const DEFENDER_BASE_DEFENSE: f32 = 10.0;

// ---------------------------------------------------------------------------
// Systems
// ---------------------------------------------------------------------------

/// Drain Charge by [`CHARGE_DRAIN_PER_TICK`] for every
/// defender that has a `Charge` component. The system runs
/// every tick so the drain is uniform regardless of the
/// defender's current state (staging, responding, in transit, or charging).
/// A defender that is currently charging from a supplied charger recovers
/// through discrete pulses while the charge trends downward everywhere else.
///
/// The system iterates all defenders with `Charge`; the work
/// is a single `f32` decrement per Defender per fixed tick.
pub fn defender_charge_drain_system(
    mut defenders: Query<(&mut Charge, &NanobotType), With<Nanobot>>,
) {
    for (mut charge, nanobot_type) in &mut defenders {
        if *nanobot_type != NanobotType::Defender {
            continue;
        }
        charge.current = (charge.current - CHARGE_DRAIN_PER_TICK).max(0.0);
    }
}

/// Apply one health damage pulse every six fixed ticks to an empty defender
/// that is neither en route to nor charging at a Charger.
#[allow(clippy::type_complexity)]
pub fn defender_health_loss_when_empty_system(
    mut commands: Commands,
    mut defenders: Query<
        (
            Entity,
            &mut Health,
            &Charge,
            Option<&mut EmptyChargeProgress>,
        ),
        (
            With<Nanobot>,
            With<NanobotType>,
            With<Charge>,
            Without<ChargerAssignment>,
            Without<ChargerProgress>,
        ),
    >,
) {
    for (entity, mut health, charge, progress) in &mut defenders {
        if !charge.is_empty() {
            if progress.is_some() {
                commands.entity(entity).remove::<EmptyChargeProgress>();
            }
            continue;
        }
        if let Some(mut progress) = progress {
            progress.ticks_elapsed = progress.ticks_elapsed.saturating_add(1);
            if progress.ticks_elapsed >= EMPTY_CHARGE_DAMAGE_INTERVAL_TICKS {
                progress.ticks_elapsed = 0;
                health.current = health.current.saturating_sub(EMPTY_CHARGE_HEALTH_DAMAGE);
            }
        } else {
            commands
                .entity(entity)
                .insert(EmptyChargeProgress { ticks_elapsed: 1 });
        }
    }
}

/// Plan owner-scoped Charger capacity for low-Charge Defenders that cannot use
/// a valid completed or pending Charger. Each plan reserves the same three-user
/// capacity as a completed Charger, keeping repeated fixed steps idempotent.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub fn charger_auto_creation_system(
    mut commands: Commands,
    grid: Res<IntentGrid>,
    structure_sprites: Res<StructureSprites>,
    chargers: Query<(
        Entity,
        &Charger,
        &Transform,
        Option<&OwnerSwarm>,
        Option<&SupportCondition>,
    )>,
    planned_chargers: Query<
        (&PlannedStructure, &Transform, Option<&OwnerSwarm>),
        With<PlannedStructure>,
    >,
    structure_obstacles: Query<&Transform, Or<(With<Stockpile>, With<ProductionFacility>)>>,
    deposits: Query<(&ResourceDeposit, &Transform)>,
    defenders: Query<
        (
            Entity,
            &Transform,
            &Charge,
            &NanobotType,
            &SwarmMember,
            Option<&Health>,
            Option<&ChargerAssignment>,
            Option<&ChargerProgress>,
            Option<&DefenderResponse>,
        ),
        (With<Nanobot>, With<Charge>),
    >,
    swarms: Query<(Entity, &SwarmId), With<Swarm>>,
) {
    let swarm_by_id: HashMap<SwarmId, Entity> =
        swarms.iter().map(|(entity, id)| (*id, entity)).collect();
    let swarm_id_by_entity: HashMap<Entity, SwarmId> =
        swarms.iter().map(|(entity, id)| (entity, *id)).collect();

    let mut obstacles: Vec<(Vec2, f32)> = deposits
        .iter()
        .map(|(deposit, transform)| (transform.translation.truncate(), deposit.radius))
        .collect();
    for transform in &structure_obstacles {
        obstacles.push((
            transform.translation.truncate(),
            scaled_building_footprint_radius(transform),
        ));
    }
    for (_, _, transform, _, _) in &chargers {
        obstacles.push((
            transform.translation.truncate(),
            scaled_building_footprint_radius(transform),
        ));
    }
    for (_, transform, _) in &planned_chargers {
        obstacles.push((
            transform.translation.truncate(),
            scaled_building_footprint_radius(transform),
        ));
    }

    let mut living_by_swarm = HashMap::<SwarmId, u32>::new();
    let mut rotating_by_swarm = HashMap::<SwarmId, u32>::new();
    let mut charger_loads = HashMap::<Entity, u32>::new();
    for (_, _, _, kind, member, health, assignment, progress, _) in &defenders {
        if *kind != NanobotType::Defender || health.is_some_and(|health| health.current == 0) {
            continue;
        }
        *living_by_swarm.entry(member.0).or_default() += 1;
        if let Some(charger) = assignment
            .map(|assignment| assignment.charger)
            .or_else(|| progress.map(|progress| progress.charger))
        {
            *charger_loads.entry(charger).or_default() += 1;
            *rotating_by_swarm.entry(member.0).or_default() += 1;
        }
    }

    let mut available_capacity = HashMap::<SwarmId, u32>::new();
    for (entity, charger, _, owner, condition) in &chargers {
        let Some(swarm) = owner.and_then(|owner| swarm_id_by_entity.get(&owner.0).copied()) else {
            continue;
        };
        let valid = charger_can_serve_in_owned_zone(charger, swarm, condition, &grid);
        if valid {
            let spare = MAX_DEFENDERS_PER_CHARGER
                .saturating_sub(charger_loads.get(&entity).copied().unwrap_or_default());
            *available_capacity.entry(swarm).or_default() += spare;
        }
    }

    for (planned, _, owner) in &planned_chargers {
        if planned.kind != PlannedKind::Charger {
            continue;
        }
        let Some(swarm) = owner.and_then(|owner| swarm_id_by_entity.get(&owner.0).copied()) else {
            continue;
        };
        if grid
            .cell(planned.cell)
            .is_some_and(|cell| cell.owner(IntentKind::Defend) == Some(swarm))
        {
            *available_capacity.entry(swarm).or_default() += MAX_DEFENDERS_PER_CHARGER;
        }
    }

    let candidates = defenders
        .iter()
        .filter(|(_, _, charge, kind, _, health, assignment, progress, _)| {
            **kind == NanobotType::Defender
                && !health.is_some_and(|health| health.current == 0)
                && charge.needs_rotation()
                && assignment.is_none()
                && progress.is_none()
        })
        .map(
            |(entity, transform, charge, _, member, _, _, _, response)| {
                (
                    member.0,
                    DefenderRotationCandidate {
                        entity,
                        charge: charge.current,
                        duty: if response.is_some() {
                            DefenderRotationDuty::Tactical
                        } else {
                            DefenderRotationDuty::Staged
                        },
                    },
                    transform.translation.truncate(),
                )
            },
        )
        .collect::<Vec<_>>();
    let candidate_positions = candidates
        .iter()
        .map(|(_, candidate, position)| (candidate.entity, *position))
        .collect::<HashMap<_, _>>();
    let mut candidate_swarms = candidates
        .iter()
        .map(|(swarm, _, _)| *swarm)
        .collect::<Vec<_>>();
    candidate_swarms.sort_unstable();
    candidate_swarms.dedup();
    let mut selected_candidates = Vec::new();
    for swarm in candidate_swarms {
        let swarm_candidates = candidates
            .iter()
            .filter_map(|(candidate_swarm, candidate, _)| {
                (*candidate_swarm == swarm).then_some(*candidate)
            })
            .collect::<Vec<_>>();
        for entity in select_defenders_for_rotation(
            &swarm_candidates,
            living_by_swarm.get(&swarm).copied().unwrap_or_default(),
            rotating_by_swarm.get(&swarm).copied().unwrap_or_default(),
        ) {
            if let Some(position) = candidate_positions.get(&entity) {
                selected_candidates.push((swarm, *position));
            }
        }
    }

    for (swarm, defender_pos) in selected_candidates {
        let capacity = available_capacity.entry(swarm).or_default();
        if *capacity > 0 {
            *capacity -= 1;
            continue;
        }
        let Some(owner) = swarm_by_id.get(&swarm).copied() else {
            continue;
        };
        let defend_cells = grid
            .iter_active_cells()
            .filter_map(|(cell, intent)| {
                (intent.owner(IntentKind::Defend) == Some(swarm)).then_some(cell)
            })
            .collect::<Vec<_>>();
        let Some((cell, placement_pos)) =
            find_nearest_defend_zone_placement(&defend_cells, &obstacles, defender_pos)
        else {
            continue;
        };
        commands.spawn((
            PlannedStructure::new(PlannedKind::Charger, cell),
            OwnerSwarm(owner),
            planned_visual_components(PlannedKind::Charger, &structure_sprites, placement_pos),
        ));
        obstacles.push((placement_pos, BUILDING_FOOTPRINT_RADIUS));
        *capacity = MAX_DEFENDERS_PER_CHARGER - 1;
    }
}

/// Whether a Charger can serve its owner from its current physical state.
pub(crate) fn charger_can_serve_in_owned_zone(
    charger: &Charger,
    swarm: SwarmId,
    condition: Option<&SupportCondition>,
    grid: &IntentGrid,
) -> bool {
    grid.cell(charger.cell)
        .is_some_and(|cell| cell.owner(IntentKind::Defend) == Some(swarm))
        && charger.has_supply()
        && condition.is_some_and(SupportCondition::is_operational)
}

/// Apply the shared service-validity rule used throughout a Charge trip.
fn charger_is_eligible(
    charger: &Charger,
    owner: Option<&OwnerSwarm>,
    condition: Option<&SupportCondition>,
    swarm: SwarmId,
    grid: &IntentGrid,
    swarms: &Query<&SwarmId, With<Swarm>>,
) -> bool {
    let charger_swarm = owner.and_then(|owner| swarms.get(owner.0).ok()).copied();
    charger_swarm == Some(swarm) && charger_can_serve_in_owned_zone(charger, swarm, condition, grid)
}

/// Find the nearest eligible Charger in the Defender's swarm.
///
/// Loads include assignments made earlier in the same fixed tick. Entity bits
/// break equal-distance ties deterministically.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub fn find_swarm_capacity_aware_charger(
    pos: Vec2,
    swarm: SwarmId,
    grid: &IntentGrid,
    chargers: &Query<(
        Entity,
        &Charger,
        &Transform,
        Option<&OwnerSwarm>,
        Option<&SupportCondition>,
    )>,
    swarms: &Query<&SwarmId, With<Swarm>>,
    charger_loads: &HashMap<Entity, u32>,
) -> Option<(Entity, Vec2)> {
    let mut best: Option<(f32, Entity, Vec2)> = None;
    for (entity, charger, transform, owner, condition) in chargers.iter() {
        if !charger_is_eligible(charger, owner, condition, swarm, grid, swarms)
            || charger_loads.get(&entity).copied().unwrap_or_default() >= MAX_DEFENDERS_PER_CHARGER
        {
            continue;
        }
        let position = transform.translation.truncate();
        let distance = pos.distance(position);
        let better = best.is_none_or(|(best_distance, best_entity, _)| {
            distance.total_cmp(&best_distance).is_lt()
                || (distance.total_cmp(&best_distance).is_eq()
                    && entity.to_bits() < best_entity.to_bits())
        });
        if better {
            best = Some((distance, entity, position));
        }
    }
    best.map(|(_, entity, position)| (entity, position))
}

/// Rotate low-Charge Defenders to eligible Chargers across their swarm.
///
/// The system filters on `Without<ChargerAssignment>` and
/// `Without<ChargerProgress>` so a defender who is already
/// en route to a charger or already at one is not re-rotated.
/// A Defender without working capacity continues its current duty.
#[allow(clippy::type_complexity)]
pub fn defender_rotation_to_charger_system(
    mut commands: Commands,
    mut allocation_wake: Option<ResMut<RegionalAllocationWake>>,
    grid: Res<IntentGrid>,
    defenders: Query<
        (
            Entity,
            &Transform,
            &Charge,
            &NanobotType,
            &SwarmMember,
            Option<&Health>,
            Option<&DefenderResponse>,
        ),
        (
            With<Nanobot>,
            With<NanobotType>,
            With<Charge>,
            Without<ChargerAssignment>,
            Without<ChargerProgress>,
        ),
    >,
    defender_states: Query<
        (
            &NanobotType,
            &SwarmMember,
            Option<&Health>,
            Option<&ChargerAssignment>,
            Option<&ChargerProgress>,
        ),
        With<Nanobot>,
    >,
    chargers: Query<(
        Entity,
        &Charger,
        &Transform,
        Option<&OwnerSwarm>,
        Option<&SupportCondition>,
    )>,
    swarms: Query<&SwarmId, With<Swarm>>,
) {
    let mut living_by_swarm = HashMap::<SwarmId, u32>::new();
    let mut rotating_by_swarm = HashMap::<SwarmId, u32>::new();
    let mut charger_loads = HashMap::<Entity, u32>::new();
    for (kind, member, health, charger_assignment, charger_progress) in &defender_states {
        if *kind != NanobotType::Defender || health.is_some_and(|health| health.current == 0) {
            continue;
        }
        *living_by_swarm.entry(member.0).or_default() += 1;
        let active_charger = charger_assignment
            .map(|assignment| assignment.charger)
            .or_else(|| charger_progress.map(|progress| progress.charger));
        if let Some(charger) = active_charger {
            *charger_loads.entry(charger).or_default() += 1;
            *rotating_by_swarm.entry(member.0).or_default() += 1;
        }
    }

    let candidates = defenders
        .iter()
        .filter(|(_, _, _, nanobot_type, _, health, _)| {
            **nanobot_type == NanobotType::Defender
                && !health.is_some_and(|health| health.current == 0)
        })
        .filter(|(_, _, charge, _, _, _, _)| charge.needs_rotation())
        .map(|(entity, transform, charge, _, member, _, response)| {
            (
                member.0,
                DefenderRotationCandidate {
                    entity,
                    charge: charge.current,
                    duty: if response.is_some() {
                        DefenderRotationDuty::Tactical
                    } else {
                        DefenderRotationDuty::Staged
                    },
                },
                transform.translation.truncate(),
            )
        })
        .collect::<Vec<_>>();
    let candidate_positions = candidates
        .iter()
        .map(|(_, candidate, position)| (candidate.entity, *position))
        .collect::<HashMap<_, _>>();
    let mut candidate_swarms = candidates
        .iter()
        .map(|(swarm, _, _)| *swarm)
        .collect::<Vec<_>>();
    candidate_swarms.sort_unstable();
    candidate_swarms.dedup();

    for swarm in candidate_swarms {
        let swarm_candidates = candidates
            .iter()
            .filter_map(|(candidate_swarm, candidate, _)| {
                (*candidate_swarm == swarm).then_some(*candidate)
            })
            .collect::<Vec<_>>();
        let selected = select_defenders_for_rotation(
            &swarm_candidates,
            living_by_swarm.get(&swarm).copied().unwrap_or_default(),
            rotating_by_swarm.get(&swarm).copied().unwrap_or_default(),
        );
        for entity in selected {
            let Some(pos) = candidate_positions.get(&entity) else {
                continue;
            };
            let Some((charger_entity, charger_pos)) = find_swarm_capacity_aware_charger(
                *pos,
                swarm,
                &grid,
                &chargers,
                &swarms,
                &charger_loads,
            ) else {
                continue;
            };
            let charger_radius = chargers
                .get(charger_entity)
                .map(|(_, charger, _, _, _)| charger.radius)
                .unwrap_or(0.0);
            commands
                .entity(entity)
                .remove::<RegionalLease>()
                .remove::<DefenderResponse>()
                .remove::<DirectMovementComponent>()
                .insert((
                    ChargerAssignment {
                        charger: charger_entity,
                    },
                    DirectMovementComponent {
                        xy: charger_pos,
                        stop_radius: charger_radius,
                    },
                ));
            if let Some(wake) = allocation_wake.as_deref_mut() {
                wake.request_current_pass();
            }
            *charger_loads.entry(charger_entity).or_default() += 1;
        }
    }
}

fn release_charger_state(
    commands: &mut Commands,
    entity: Entity,
    allocation_wake: &mut Option<ResMut<RegionalAllocationWake>>,
) {
    commands
        .entity(entity)
        .remove::<ChargerAssignment>()
        .remove::<ChargerProgress>()
        .remove::<ChargerPulseProgress>()
        .remove::<DirectMovementComponent>()
        .remove::<RegionalLease>();
    if let Some(wake) = allocation_wake.as_deref_mut() {
        wake.request_current_pass();
    }
}

/// Detect a defender that has arrived at its assigned charger
/// and transition it into the `ChargerProgress` state. The
/// arrival trigger is the same as the rest of the
/// simulation: the movement system removes the
/// `DirectMovementComponent` when the bot is within
/// [`STOP_THRESHOLD`] of its target.
///
/// The `Without<ChargerProgress>` filter makes arrival
/// idempotent. The `ChargerAssignment` is kept on the entity
/// so the work system can read which charger the defender is
/// at without re-querying the grid.
#[allow(clippy::type_complexity)]
pub fn defender_charger_arrive_system(
    mut commands: Commands,
    mut allocation_wake: Option<ResMut<RegionalAllocationWake>>,
    mut defenders: Query<
        (
            Entity,
            &ChargerAssignment,
            &Transform,
            Option<&DirectMovementComponent>,
            &SwarmMember,
        ),
        (
            With<Nanobot>,
            With<ChargerAssignment>,
            Without<ChargerProgress>,
        ),
    >,
    grid: Res<IntentGrid>,
    chargers: Query<(
        &Charger,
        &Transform,
        Option<&OwnerSwarm>,
        Option<&SupportCondition>,
    )>,
    swarms: Query<&SwarmId, With<Swarm>>,
) {
    for (entity, assignment, transform, movement, member) in &mut defenders {
        let Ok((charger, charger_transform, owner, condition)) = chargers.get(assignment.charger)
        else {
            release_charger_state(&mut commands, entity, &mut allocation_wake);
            continue;
        };
        if !charger_is_eligible(charger, owner, condition, member.0, &grid, &swarms) {
            release_charger_state(&mut commands, entity, &mut allocation_wake);
            continue;
        }
        if movement.is_some() {
            continue;
        }
        let distance = transform
            .translation
            .truncate()
            .distance(charger_transform.translation.truncate());
        if distance > charger.radius {
            release_charger_state(&mut commands, entity, &mut allocation_wake);
            continue;
        }
        commands.entity(entity).insert((
            ChargerProgress {
                charger: assignment.charger,
            },
            ChargerPulseProgress::default(),
        ));
    }
}

/// Defender charging work system. For every defender with a
/// `ChargerProgress`, grant [`CHARGE_PER_PULSE`] every
/// [`CHARGE_PULSE_INTERVAL_TICKS`] fixed ticks, and drain one mineral from the
/// Charger. The Defender is released back to current allocation when the
/// Charge is full or the Charger runs out of supply.
///
/// The system always runs in the same chain as the rotation
/// and arrive systems; a defender at a fresh charger with
/// empty charge refills on the same tick it arrives, and a
/// Defender whose Charger empties mid-charge is released on
/// the same tick. The release is a marker remove; current response allocation
/// may pick the Defender during the same pass.
#[allow(clippy::type_complexity)]
pub fn defender_charger_work_system(
    mut commands: Commands,
    mut allocation_wake: Option<ResMut<RegionalAllocationWake>>,
    mut defenders: Query<
        (
            Entity,
            &mut Charge,
            &ChargerAssignment,
            Option<&mut ChargerPulseProgress>,
            &SwarmMember,
        ),
        (With<Nanobot>, With<ChargerProgress>),
    >,
    grid: Res<IntentGrid>,
    mut chargers: Query<(&mut Charger, Option<&OwnerSwarm>, Option<&SupportCondition>)>,
    swarms: Query<&SwarmId, With<Swarm>>,
    mut ledger: ResMut<ResourceLedger>,
) {
    let mut ordered_defenders = defenders
        .iter_mut()
        .map(|(entity, _, _, _, _)| entity)
        .collect::<Vec<_>>();
    ordered_defenders.sort_by_key(|entity| entity.to_bits());

    for entity in ordered_defenders {
        let Ok((entity, mut charge, assignment, pulse, member)) = defenders.get_mut(entity) else {
            continue;
        };
        let Ok((mut charger, owner, condition)) = chargers.get_mut(assignment.charger) else {
            // Charger disappeared mid-charge. Drop both
            // markers and let the defender be re-assigned.
            release_charger_state(&mut commands, entity, &mut allocation_wake);
            continue;
        };
        if !charger_is_eligible(&charger, owner, condition, member.0, &grid, &swarms) {
            release_charger_state(&mut commands, entity, &mut allocation_wake);
            continue;
        }
        let Some(mut pulse) = pulse else {
            commands
                .entity(entity)
                .insert(ChargerPulseProgress::default());
            continue;
        };
        pulse.ticks_elapsed = pulse.ticks_elapsed.saturating_add(1);
        if pulse.ticks_elapsed < CHARGE_PULSE_INTERVAL_TICKS {
            continue;
        }
        pulse.ticks_elapsed = 0;

        let consumed = CHARGER_MATERIAL_PER_PULSE.min(charger.amount);
        if consumed == 0 {
            release_charger_state(&mut commands, entity, &mut allocation_wake);
            continue;
        }
        charger.amount -= consumed;
        ledger.remove_for(member.0, charger.kind, consumed);
        charge.current = (charge.current + CHARGE_PER_PULSE).min(charge.max);
        if charge.is_full() || !charger.has_supply() {
            release_charger_state(&mut commands, entity, &mut allocation_wake);
        }
    }
}

// ---------------------------------------------------------------------------
// Plugin
// ---------------------------------------------------------------------------

/// Plugin that wires the charge-sustain systems into the
/// fixed simulation schedule.
///
/// Demand and consumer state use separate chains. Demand observes current
/// staging after acquisition. Consumer state settles before regional projection
/// so Charge lifecycle changes are visible to the current acquisition pass.
///
/// Demand chain (single system, ordered after current regional acquisition
/// and before planned-structure work):
///
/// 1. [`charger_auto_creation_system`] -- spawn new planned
///    Chargers from unmet low-Charge service need. The regional allocator sees
///    the plan on its next projection/acquisition pass; a
///    Worker then builds it through the planned-structure
///    lifecycle.
///
/// Consumer chain (after movement and before regional projection):
///
/// 1. [`defender_charge_drain_system`] -- passive drain
///    first so the rotation trigger sees the post-drain
///    value.
/// 2. [`defender_health_loss_when_empty_system`] -- health
///    loss fires for unserved Defenders with empty Charge.
/// 3. [`defender_rotation_to_charger_system`] -- rotate
///    low-Charge Defenders to working Chargers.
/// 4. [`defender_charger_arrive_system`] -- transition
///    arrived defenders into the charging state.
/// 5. [`defender_charger_work_system`] -- refill charge and
///    drain charger material.
pub struct ChargePlugin;

impl Plugin for ChargePlugin {
    fn build(&self, app: &mut App) {
        // Demand: spawn planned Chargers from unmet service need
        // after movement and current Charge state settle. The
        // regional allocator projects and claims the new plan
        // on a later allocation pass; no legacy worker-claim
        // system is registered. Run before planned work so
        // the plan is present before that lifecycle reads it.
        app.add_systems(
            FixedUpdate,
            charger_auto_creation_system
                .after(crate::nanobot::RegionalAllocationSet::Acquire)
                .before(crate::nanobot::planned::worker_planned_structure_work_system),
        );
        // Consumer state settles before regional projection so Charge departure,
        // completion, and invalidation release old allocation before acquisition.
        app.add_systems(
            FixedUpdate,
            (
                defender_charge_drain_system,
                defender_health_loss_when_empty_system,
                defender_rotation_to_charger_system,
                defender_charger_arrive_system,
                defender_charger_work_system,
            )
                .chain()
                .after(crate::nanobot::NanobotSimulationSet::Movement)
                .before(crate::nanobot::RegionalAllocationSet::Project),
        );
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    //! Pure-helper unit tests for the charge data and
    //! constants. The end-to-end contracts (creation,
    //! logistics, weakening, health loss, rotation) live in
    //! `tests/charger_behavior.rs`.

    use approx::assert_abs_diff_eq;

    use super::*;

    #[test]
    fn charge_default_starts_full() {
        let c = Charge::default();
        assert_abs_diff_eq!(c.current, MAX_CHARGE, epsilon = 1e-5);
        assert_abs_diff_eq!(c.max, MAX_CHARGE, epsilon = 1e-5);
        assert!(c.is_full());
        assert!(!c.is_empty());
        assert!(!c.needs_rotation());
    }

    #[test]
    fn charge_is_full_only_at_or_above_max() {
        let mut c = Charge::default();
        assert!(c.is_full());
        c.current = c.max - f32::EPSILON;
        assert!(!c.is_full(), "just-below-max must not report full");
        c.current = c.max + 0.1;
        assert!(c.is_full(), "above-max must report full");
    }

    #[test]
    fn charge_is_empty_at_or_below_zero() {
        let mut c = Charge::default();
        assert!(!c.is_empty());
        c.current = 0.0;
        assert!(c.is_empty());
        c.current = -0.5;
        assert!(c.is_empty(), "negative charge still reports empty");
    }

    #[test]
    fn charge_needs_rotation_at_or_below_low_threshold() {
        let mut c = Charge::default();
        assert!(!c.needs_rotation());
        c.current = LOW_CHARGE_THRESHOLD;
        assert!(c.needs_rotation(), "at threshold must trigger rotation");
        c.current = LOW_CHARGE_THRESHOLD - 0.05;
        assert!(c.needs_rotation());
        c.current = LOW_CHARGE_THRESHOLD + 0.05;
        assert!(
            !c.needs_rotation(),
            "above threshold must not trigger rotation"
        );
    }

    #[test]
    fn charge_mineral_need_uses_net_pulse_boundaries() {
        let net_refill =
            CHARGE_PER_PULSE - CHARGE_DRAIN_PER_TICK * f32::from(CHARGE_PULSE_INTERVAL_TICKS);
        assert_eq!(minerals_to_fully_charge(MAX_CHARGE, MAX_CHARGE), 0);
        assert_eq!(
            minerals_to_fully_charge(MAX_CHARGE - net_refill, MAX_CHARGE),
            CHARGER_MATERIAL_PER_PULSE
        );
        assert_eq!(
            minerals_to_fully_charge(MAX_CHARGE - net_refill * 1.01, MAX_CHARGE),
            CHARGER_MATERIAL_PER_PULSE * 2
        );
    }

    #[test]
    fn charge_mineral_need_clamps_empty_and_out_of_range_charge() {
        let net_refill =
            CHARGE_PER_PULSE - CHARGE_DRAIN_PER_TICK * f32::from(CHARGE_PULSE_INTERVAL_TICKS);
        let full_refill_pulses = (MAX_CHARGE / net_refill).ceil() as u32;
        let full_refill_minerals = full_refill_pulses.saturating_mul(CHARGER_MATERIAL_PER_PULSE);
        assert_eq!(
            minerals_to_fully_charge(0.0, MAX_CHARGE),
            full_refill_minerals
        );
        assert_eq!(
            minerals_to_fully_charge(-1.0, MAX_CHARGE),
            full_refill_minerals
        );
        assert_eq!(minerals_to_fully_charge(MAX_CHARGE + 1.0, MAX_CHARGE), 0);
    }

    #[test]
    fn charger_starts_empty_with_full_capacity() {
        let charger = Charger::new(IVec2::new(1, -1));
        assert_eq!(charger.cell, IVec2::new(1, -1));
        assert_eq!(charger.kind, AUTO_CHARGER_KIND);
        assert_eq!(charger.amount, 0);
        assert_eq!(charger.capacity, AUTO_CHARGER_CAPACITY);
        assert_abs_diff_eq!(charger.radius, AUTO_CHARGER_RADIUS, epsilon = 1e-5);
        assert!(!charger.has_supply());
        assert_eq!(charger.free_space(), AUTO_CHARGER_CAPACITY);
    }

    #[test]
    fn charger_has_supply_only_while_amount_is_positive() {
        let mut charger = Charger::new(IVec2::new(0, 0));
        assert!(!charger.has_supply());
        charger.amount = 1;
        assert!(charger.has_supply());
        charger.amount = 0;
        assert!(!charger.has_supply(), "empty charger must not have supply");
    }

    #[test]
    fn charger_free_space_floors_at_zero() {
        let mut charger = Charger::new(IVec2::new(0, 0));
        charger.amount = charger.capacity;
        assert_eq!(charger.free_space(), 0);
        charger.amount = charger.capacity + 5;
        assert_eq!(charger.free_space(), 0, "free space never goes negative");
    }

    #[test]
    fn supplied_pulse_outpaces_field_drain() {
        let net_refill =
            CHARGE_PER_PULSE - CHARGE_DRAIN_PER_TICK * f32::from(CHARGE_PULSE_INTERVAL_TICKS);
        assert!(net_refill > 0.0);
    }

    #[test]
    fn low_threshold_sits_above_weakened_threshold() {
        // The rotation must trigger *before* the defender is
        // fully weakened so the sustain loop is preventative
        // rather than reactive. A test asserting
        // LOW_CHARGE_THRESHOLD > WEAKENED_CHARGE_THRESHOLD
        // pins the relative ordering.
        const { assert!(LOW_CHARGE_THRESHOLD > WEAKENED_CHARGE_THRESHOLD) };
    }

    #[test]
    fn charge_strength_multiplier_is_one_above_weakened_threshold() {
        // Pin the "fully charged defenders are at full
        // strength" half of the contract. A defender at
        // exactly the threshold or above must be at full
        // multiplier.
        assert!((charge_strength_multiplier(WEAKENED_CHARGE_THRESHOLD) - 1.0).abs() < 1e-6);
        assert!((charge_strength_multiplier(MAX_CHARGE) - 1.0).abs() < 1e-6);
        assert!((charge_strength_multiplier(0.5) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn charge_strength_multiplier_scales_linearly_below_weakened_threshold() {
        // Pin the linear scaling: half the threshold gives
        // half the multiplier, quarter gives a quarter.
        // The implementation uses a direct `charge / threshold`
        // ratio so the linearity is exact (not approximate).
        let half = charge_strength_multiplier(WEAKENED_CHARGE_THRESHOLD * 0.5);
        let quarter = charge_strength_multiplier(WEAKENED_CHARGE_THRESHOLD * 0.25);
        assert!(
            (half - 0.5).abs() < 1e-6,
            "half charge -> half multiplier; got {half}"
        );
        assert!(
            (quarter - 0.25).abs() < 1e-6,
            "quarter charge -> quarter multiplier; got {quarter}"
        );
    }

    #[test]
    fn charge_strength_multiplier_is_zero_at_or_below_zero() {
        // Empty charge means zero attack and zero defense.
        assert_abs_diff_eq!(charge_strength_multiplier(0.0), 0.0, epsilon = 1e-5);
        assert_abs_diff_eq!(charge_strength_multiplier(-0.5), 0.0, epsilon = 1e-5);
    }

    #[test]
    fn charge_strength_multiplier_clamps_above_max() {
        // Out-of-range inputs (e.g. an overfilling system)
        // must not produce a multiplier above 1.0.
        let at_max = charge_strength_multiplier(MAX_CHARGE);
        let above_max = charge_strength_multiplier(MAX_CHARGE * 2.0);
        assert!((at_max - 1.0).abs() < 1e-6);
        assert!((above_max - 1.0).abs() < 1e-6, "above-max clamps to 1.0");
    }

    #[test]
    fn effective_attack_tracks_charge() {
        // Acceptance: "Low Charge reduces Defender attack/defense."
        // A defender at full charge has full attack; a
        // defender at half the weakened threshold has half
        // attack; a defender at empty charge has zero attack.
        assert!((effective_attack(MAX_CHARGE) - DEFENDER_BASE_ATTACK).abs() < 1e-5);
        assert!((effective_attack(WEAKENED_CHARGE_THRESHOLD) - DEFENDER_BASE_ATTACK).abs() < 1e-5);
        let half_attack = effective_attack(WEAKENED_CHARGE_THRESHOLD * 0.5);
        assert!(
            (half_attack - DEFENDER_BASE_ATTACK * 0.5).abs() < 1e-5,
            "half charge -> half attack; got {half_attack}"
        );
        assert_abs_diff_eq!(effective_attack(0.0), 0.0, epsilon = 1e-5);
    }

    #[test]
    fn effective_defense_tracks_charge() {
        // Mirror of `effective_attack` for the defense
        // side. The same linearity test pins the contract.
        assert!((effective_defense(MAX_CHARGE) - DEFENDER_BASE_DEFENSE).abs() < 1e-5);
        assert!(
            (effective_defense(WEAKENED_CHARGE_THRESHOLD) - DEFENDER_BASE_DEFENSE).abs() < 1e-5
        );
        let half_defense = effective_defense(WEAKENED_CHARGE_THRESHOLD * 0.5);
        assert!(
            (half_defense - DEFENDER_BASE_DEFENSE * 0.5).abs() < 1e-5,
            "half charge -> half defense; got {half_defense}"
        );
        assert_abs_diff_eq!(effective_defense(0.0), 0.0, epsilon = 1e-5);
    }

    #[test]
    fn auto_charger_constants_form_a_consistent_buffer() {
        // New chargers rely on physical logistics for all material.
        const { assert!(AUTO_CHARGER_INITIAL_AMOUNT == 0) };
        const { assert!(AUTO_CHARGER_INITIAL_AMOUNT < AUTO_CHARGER_CAPACITY) };
    }

    #[test]
    fn swarm_rotation_capacity_keeps_at_least_half_of_living_defenders_on_duty() {
        let cases = [(0, 0), (1, 1), (2, 1), (3, 1), (4, 2), (5, 2)];

        for (living, expected) in cases {
            assert_eq!(
                defender_rotation_capacity(living),
                expected,
                "living population {living}"
            );
        }
    }

    #[test]
    fn scarce_rotation_slots_prefer_charge_then_duty_then_identity() {
        let first_spawned = Entity::from_bits(1);
        let staged = Entity::from_bits(2);
        let tactical = Entity::from_bits(3);
        let lowest_charge = Entity::from_bits(4);
        let candidates = [
            DefenderRotationCandidate {
                entity: tactical,
                charge: 0.2,
                duty: DefenderRotationDuty::Tactical,
            },
            DefenderRotationCandidate {
                entity: staged,
                charge: 0.2,
                duty: DefenderRotationDuty::Staged,
            },
            DefenderRotationCandidate {
                entity: first_spawned,
                charge: 0.2,
                duty: DefenderRotationDuty::Staged,
            },
            DefenderRotationCandidate {
                entity: lowest_charge,
                charge: 0.1,
                duty: DefenderRotationDuty::Tactical,
            },
        ];

        assert_eq!(
            select_defenders_for_rotation(&candidates, 8, 0),
            vec![lowest_charge, first_spawned, staged, tactical]
        );
        assert_eq!(
            select_defenders_for_rotation(&candidates, 6, 2),
            vec![lowest_charge]
        );
    }
}
