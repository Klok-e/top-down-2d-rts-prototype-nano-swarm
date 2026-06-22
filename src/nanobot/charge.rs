//! Defender Charge sustain loop and Charger logistics.
//!
//! Issue #14 contract: Chargers are automatic structures that
//! resupply defenders with `Charge`. Defenders use Charge, low
//! Charge weakens their attack and defense, empty/ignored
//! Charge causes defender health loss, and defenders rotate to
//! working chargers automatically. Chargers emerge from Defend
//! Zone load and existing charger busyness, and require
//! logistics support via physical resources so isolated defenses
//! degrade when haulers cannot reach them.
//!
//! Defender state machine carried on the defender by marker
//! components:
//!
//! ```text
//!   Holding (DefendHold + Charge)
//!     -> (charge low + working charger available)
//!     -> Moving (ChargerAssignment + DMC, DefendHold removed)
//!   Moving
//!     -> (arrive at charger)
//!     -> Charging (ChargerAssignment + ChargerProgress)
//!   Charging
//!     -> (charge full OR charger empty)
//!     -> Idle (markers cleared; defend assignment re-picks)
//! ```
//!
//! The "ignored Charge" health-loss case is the holding path
//! with empty charge and no working charger reachable: the
//! defender stays in hold and drains health per tick. The
//! rotation case is the holding path with empty/low charge and
//! a working charger reachable: the defender leaves hold,
//! walks to the charger, and starts charging.
//!
//! Soft work slot occupancy is reused: a defender holding a
//! Defend cell occupies `(cell, Defend)`. The rotation system
//! releases the slot when the defender leaves hold. A separate
//! slot for the charger path is not modelled in the first
//! implementation because each defender visits at most one
//! charger at a time, so the slot would not add new pressure.
//!
//! Logistics: a `Charger` carries a `Stockpile`-shaped physical
//! buffer of `ResourceKind::Minerals`. Defenders charging from
//! the buffer drain it at a fixed per-tick rate; when the
//! buffer is empty, the charger is not "working" and defenders
//! will not rotate to it. Haulers (issue #8) deliver material
//! to the buffer so a defended cell with active logistics
//! stays charged. A defended cell with no haulers reaching it
//! gradually loses charger material and the defenders degrade.

use bevy::prelude::*;

use crate::intent::{IntentGrid, IntentKind};
use crate::nanobot::autonomy::NanobotType;
use crate::nanobot::autonomy::SoftWorkSlots;
use crate::nanobot::components::{DirectMovementComponent, Health, Nanobot, Swarm, SwarmId};
use crate::nanobot::defend::DefendHold;
use crate::nanobot::gather::world_to_cell;
use crate::nanobot::placement::{find_build_zone_placement, BUILDING_FOOTPRINT_RADIUS};
use crate::nanobot::planned::{planned_visual_components, PlannedKind, PlannedStructure};
use crate::nanobot::production::{OwnerSwarm, ProductionFacility};
use crate::resources::{ResourceDeposit, ResourceKind, Stockpile};
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

/// Passive Charge drain per `app.update()` tick for every
/// defender with a `Charge` component. The drain models
/// "Defenders use Charge" -- the sustain resource is consumed
/// just by existing, not by active combat in the first
/// implementation. The number is small enough that a
/// `MAX_CHARGE` charge lasts on the order of a few hundred
/// ticks so test scenarios do not need to drive the simulation
/// for thousands of ticks to see the rotation trigger.
pub const CHARGE_DRAIN_PER_TICK: f32 = 0.005;

/// Charge refilled per tick while a defender is in a
/// working charger's radius. Significantly larger than
/// [`CHARGE_DRAIN_PER_TICK`] so the charge trends upward while
/// a defender is charging rather than oscillating around a
/// steady state. With `0.05` refill and `0.005` drain, an
/// empty defender at a working charger recovers fully in
/// `1.0 / (0.05 - 0.005) ~= 22` ticks; a partially charged
/// defender recovers faster.
pub const CHARGE_REFILL_PER_TICK: f32 = 0.05;

/// Charge per tick of `ResourceKind::Minerals` drained from a
/// charger's `amount` while a defender is charging from it.
/// The 1:1 ratio keeps the math obvious in the tests: a
/// charger with `amount = 30` can sustain one defender for 30
/// ticks. Production-side tuning can rebalance the ratio
/// without changing the public contracts.
pub const CHARGER_MATERIAL_DRAIN_PER_TICK: u32 = 1;

/// Charge level below which a defender's attack and defense
/// are weakened. Above the threshold the modifier is 1.0;
/// below it the modifier scales linearly with charge so an
/// empty defender has 0.0 attack and 0.0 defense. The
/// threshold sits well above zero so a defender that just
/// lost the weaken threshold is still in "weak but alive"
/// territory rather than "instantly dead".
pub const WEAKENED_CHARGE_THRESHOLD: f32 = 0.3;

/// Charge level at or below which a holding defender is
/// eligible to rotate to a working charger. The threshold sits
/// a notch above [`WEAKENED_CHARGE_THRESHOLD`] so a defender
/// starts looking for a charger *before* it is too weak to
/// fight, keeping the sustain loop preventative rather than
/// reactive. A defender whose charge is between
/// `LOW_CHARGE_THRESHOLD` and `WEAKENED_CHARGE_THRESHOLD` is
/// still weakened but not yet rotating; below the low
/// threshold the rotation kicks in.
pub const LOW_CHARGE_THRESHOLD: f32 = 0.5;

/// Health lost per tick by a defender with `charge <= 0.0`
/// that is not currently charging (no `ChargerAssignment` and
/// no `ChargerProgress`). The loss is the "ignored Charge"
/// half of the acceptance criterion: a defender that empties
/// its charge and has no working charger reachable drains
/// health per tick, eventually collapsing the defender.
pub const EMPTY_CHARGE_HEALTH_LOSS_PER_TICK: u32 = 2;

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

/// Material buffer an auto-created charger starts with. Less
/// than the capacity so a freshly emerged charger has room
/// for an immediate hauler delivery, but enough to sustain
/// one or two defenders for a meaningful number of ticks.
pub const AUTO_CHARGER_INITIAL_AMOUNT: u32 = 10;

/// Maximum defenders that can be actively charging from a
/// single charger at once before a new charger is allowed to
/// emerge. The "busyness" half of the issue's charger
/// auto-creation contract: a cell whose existing charger is
/// already at this cap spawns an additional charger.
pub const MAX_DEFENDERS_PER_CHARGER: u32 = 3;

/// Maximum chargers a single Defend cell can hold. The
/// emergence rule spawns chargers to satisfy
/// `ceil(load / MAX_DEFENDERS_PER_CHARGER)` up to this cap,
/// so a cell with 8 defenders can hold 3 chargers; a cell
/// with 20 defenders can also hold 3 chargers (the cap is the
/// ceiling, not the floor).
pub const MAX_CHARGERS_PER_CELL: u32 = 3;

// ---------------------------------------------------------------------------
// Components
// ---------------------------------------------------------------------------

/// A local support structure that refills defender Charge.
/// Spawned automatically in Defend cells by
/// [`charger_auto_creation_system`] and refilled physically by
/// haulers (the hauler's sink selection includes chargers
/// with free space).
///
/// `amount` is the physical resource buffer; when `amount ==
/// 0` the charger is "empty" and is not a valid rotation
/// target. `capacity` caps the buffer; a freshly auto-created
/// charger starts with [`AUTO_CHARGER_INITIAL_AMOUNT`] units
/// and the auto-creation system leaves room for an immediate
/// delivery.
#[derive(Debug, Component, Clone, Copy)]
pub struct Charger {
    /// Defend cell the charger lives in. Used by the
    /// auto-creation system to find existing chargers in a
    /// cell and by the rotation system to check "is this
    /// charger actually in the cell the defender is
    /// defending?".
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
    /// Build a new charger in `cell` with the default kind,
    /// capacity, radius, and starting amount. Used by the
    /// auto-creation system and by tests.
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
    /// work system uses this to decide when to release a
    /// defender back to the defend pool.
    pub fn is_full(&self) -> bool {
        self.current >= self.max
    }

    /// True when `current <= 0.0`. The health-loss system uses
    /// this to decide which holding defenders are "ignored
    /// Charge" candidates.
    pub fn is_empty(&self) -> bool {
        self.current <= 0.0
    }

    /// True when the charge is low enough that the rotation
    /// system should pull the defender away from the defend
    /// cell toward a working charger.
    pub fn needs_rotation(&self) -> bool {
        self.current <= LOW_CHARGE_THRESHOLD
    }
}

/// Marks a Defender as committed to a specific Charger. Set
/// by the rotation system when the defender's charge is low
/// and a working charger is reachable; cleared when the
/// defender reaches the charger (the `ChargerProgress`
/// marker takes over) or when the work system finishes the
/// charging cycle.
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

// ---------------------------------------------------------------------------
// Pure helpers
// ---------------------------------------------------------------------------

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
/// from [`charge_strength_multiplier`]. The first
/// implementation has no actual combat to consume the value;
/// the function is the contract a future combat system reads
/// to decide how much damage a defender's attack deals.
pub fn effective_attack(charge: f32) -> f32 {
    DEFENDER_BASE_ATTACK * charge_strength_multiplier(charge)
}

/// Effective defense for a defender with `charge` current.
/// Mirrors [`effective_attack`]: base defense scaled by the
/// charge multiplier. A future combat system reads this to
/// decide how much damage a defender takes from an incoming
/// attack.
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
/// defender's current state (holding, in transit, charging).
/// A defender that is currently charging from a working
/// charger recovers faster than the drain (see
/// [`CHARGE_REFILL_PER_TICK`]) so the charge trends upward
/// while the defender is at the charger and trends downward
/// everywhere else.
///
/// The system uses `ChangeTrackers` would be a future
/// optimisation; the first implementation iterates all
/// defenders with `Charge` because the cost is trivial
/// (a single `f32` decrement per defender per tick).
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

/// Drain Health for every defender whose Charge is empty and
/// who is not currently addressing that empty charge (i.e.
/// not in [`ChargerAssignment`] or [`ChargerProgress`]). The
/// "ignored Charge" half of the issue: a defender that has
/// dropped to zero charge and has no working charger to walk
/// to loses health per tick.
///
/// The system is the "fallback" loop: if a defender has
/// empty charge but is en route to a charger (ChargerAssignment)
/// or already charging (ChargerProgress), the rotation chain
/// has already picked them up and the defender is no longer
/// "ignoring" the situation. The system is therefore a no-op
/// for those defenders and only fires for holding defenders
/// with empty charge and no reachable working charger.
#[allow(clippy::type_complexity)]
pub fn defender_health_loss_when_empty_system(
    mut commands: Commands,
    mut defenders: Query<
        (Entity, &mut Health, &Charge),
        (
            With<Nanobot>,
            With<NanobotType>,
            With<Charge>,
            Without<ChargerAssignment>,
            Without<ChargerProgress>,
        ),
    >,
) {
    for (entity, mut health, charge) in &mut defenders {
        if !charge.is_empty() {
            continue;
        }
        let next = health
            .current
            .saturating_sub(EMPTY_CHARGE_HEALTH_LOSS_PER_TICK);
        if next == 0 {
            // Defender collapsed. Despawn cleanly; the
            // soft-slot pressure in the defend system is
            // released by the next assignment tick because
            // the entity no longer holds the (cell, Defend)
            // slot.
            commands.entity(entity).despawn();
        } else {
            health.current = next;
        }
    }
}

/// Walk the [`IntentGrid`] and ensure every Defend cell with
/// load (defenders committed to that cell) has enough
/// chargers to cover the demand. As of issue #28 the demand
/// is satisfied through the Planned Structure lifecycle: a
/// new [`PlannedStructure`] of [`PlannedKind::Charger`]
/// emerges in a cell when:
///
/// 1. the cell is painted with `IntentKind::Defend`,
/// 2. the cell has at least one defender holding or assigned
///    to it, AND
/// 3. the cell's current `(chargers + planned_chargers) *
///    MAX_DEFENDERS_PER_CHARGER` is below the load.
///
/// The "existing charger busyness" half of the issue lives
/// here: a cell with a single charger and 4 defenders spawns
/// a second (planned) charger, so the existing charger is
/// not asked to serve more than [`MAX_DEFENDERS_PER_CHARGER`]
/// defenders at once. The busyness count INCLUDES planned
/// chargers in the same cell: a cell with one built charger
/// and a pending plan must not pile a second plan, otherwise
/// the auto-creation loop would emit one plan per tick.
///
/// A cell whose existing chargers + planned chargers are
/// already at [`MAX_CHARGERS_PER_CELL`] does not get more
/// plans; the cap is the hard ceiling and a follow-up issue
/// can revisit it if defenders starve in practice.
///
/// Ownership: the plan is stamped with [`OwnerSwarm`] from
/// the Defend cell's intent owner (issue #20's per-swarm
/// intent ownership). Unowned Defend paint falls back to
/// the first [`Swarm`] in the world, matching the
/// unowned-paint contract in the rest of the simulation.
/// The promotion path preserves [`OwnerSwarm`] on the
/// completed charger.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub fn charger_auto_creation_system(
    mut commands: Commands,
    grid: Res<IntentGrid>,
    structure_sprites: Res<StructureSprites>,
    chargers: Query<(Entity, &Charger, &Transform)>,
    planned_chargers: Query<(&PlannedStructure, &Transform), With<PlannedStructure>>,
    structure_obstacles: Query<&Transform, Or<(With<Stockpile>, With<ProductionFacility>)>>,
    deposits: Query<(&ResourceDeposit, &Transform)>,
    defenders_in_cell: Query<
        (&Transform, &NanobotType),
        Or<(
            With<DefendHold>,
            With<crate::nanobot::defend::DefendAssignment>,
        )>,
    >,
    swarms: Query<(Entity, &SwarmId), With<Swarm>>,
) {
    // Index existing chargers by cell so per-cell logic is
    // O(1) per cell rather than O(chargers * cells).
    let mut chargers_per_cell: std::collections::HashMap<IVec2, u32> =
        std::collections::HashMap::new();
    let mut obstacles: Vec<(Vec2, f32)> = deposits
        .iter()
        .map(|(deposit, transform)| (transform.translation.truncate(), deposit.radius))
        .collect();
    for transform in &structure_obstacles {
        obstacles.push((transform.translation.truncate(), BUILDING_FOOTPRINT_RADIUS));
    }
    for (_, charger, transform) in &chargers {
        *chargers_per_cell.entry(charger.cell).or_insert(0) += 1;
        obstacles.push((transform.translation.truncate(), BUILDING_FOOTPRINT_RADIUS));
    }
    for (planned, transform) in &planned_chargers {
        // Every planned structure (not just Chargers) is
        // in the obstacle list. The issue #34 "shared
        // footprint" contract says "Charger placement
        // rejects candidates that overlap ... Planned
        // Structures" -- a planned Sink Stockpile or
        // Production Facility must block a Charger
        // candidate the same way a planned Charger
        // would. The busyness count is still scoped to
        // Planned Chargers, since only Chargers satisfy
        // Defend demand.
        obstacles.push((transform.translation.truncate(), BUILDING_FOOTPRINT_RADIUS));
        if planned.kind == PlannedKind::Charger {
            *chargers_per_cell.entry(planned.cell).or_insert(0) += 1;
        }
    }

    // Count defenders per cell: holding or assigned. Defenders
    // in transit or charging are intentionally NOT counted so
    // a defender rotating to a charger does not double-count
    // the cell's load.
    let mut defenders_per_cell: std::collections::HashMap<IVec2, u32> =
        std::collections::HashMap::new();
    for (transform, nanobot_type) in &defenders_in_cell {
        if *nanobot_type != NanobotType::Defender {
            continue;
        }
        let cell = world_to_cell(transform.translation.truncate());
        *defenders_per_cell.entry(cell).or_insert(0) += 1;
    }

    let swarm_by_id: std::collections::HashMap<SwarmId, Entity> =
        swarms.iter().map(|(e, id)| (*id, e)).collect();
    let fallback_owner = swarms.iter().next().map(|(e, _)| e);

    for (cell, intent_cell) in grid.iter_cells() {
        if !intent_cell.has(IntentKind::Defend) {
            continue;
        }
        let load = *defenders_per_cell.get(&cell).unwrap_or(&0);
        if load == 0 {
            // No demand: a previously-occupied cell whose
            // defenders have all left (e.g. wiped) is left
            // alone. The existing chargers and pending
            // plans remain; the follow-up "charger collapse"
            // issue can decide whether to despawn them.
            continue;
        }
        let existing = *chargers_per_cell.get(&cell).unwrap_or(&0);
        // Demand in charger units: each charger covers up to
        // MAX_DEFENDERS_PER_CHARGER defenders.
        let needed_chargers = load.div_ceil(MAX_DEFENDERS_PER_CHARGER);
        let target_chargers = needed_chargers.min(MAX_CHARGERS_PER_CELL);
        if existing >= target_chargers {
            continue;
        }
        let to_spawn = (target_chargers - existing).min(MAX_CHARGERS_PER_CELL - existing);
        // Per-swarm intent ownership: the Defend cell's
        // owner is the swarm that painted it. Unowned
        // paint falls back to the first Swarm, matching
        // the unowned-paint contract in the rest of the
        // simulation.
        let owner = intent_cell
            .owner(IntentKind::Defend)
            .and_then(|id| swarm_by_id.get(&id).copied())
            .or(fallback_owner);
        for _ in 0..to_spawn {
            let Some((placement_cell, placement_pos)) =
                find_build_zone_placement(&[cell], &obstacles, 28)
            else {
                break;
            };
            let mut entity_commands = commands.spawn((
                PlannedStructure::new(PlannedKind::Charger, placement_cell),
                planned_visual_components(PlannedKind::Charger, &structure_sprites, placement_pos),
            ));
            obstacles.push((placement_pos, BUILDING_FOOTPRINT_RADIUS));
            if let Some(swarm_entity) = owner {
                entity_commands.insert(OwnerSwarm(swarm_entity));
            }
        }
    }
}

/// Find the nearest working charger to `pos` -- a charger
/// with [`Charger::has_supply`] returning `true`. Returns
/// `None` when no working charger exists in the world. The
/// helper is pure over query data so the rotation system
/// stays small.
pub fn find_nearest_working_charger(
    pos: Vec2,
    chargers: &Query<(Entity, &Charger, &Transform)>,
) -> Option<(Entity, Vec2)> {
    let mut best: Option<(f32, Entity, Vec2)> = None;
    for (entity, charger, transform) in chargers.iter() {
        if !charger.has_supply() {
            continue;
        }
        let d = pos.distance(transform.translation.truncate());
        if best.is_none_or(|(bd, _, _)| d < bd) {
            best = Some((d, entity, transform.translation.truncate()));
        }
    }
    best.map(|(_, e, pos)| (e, pos))
}

/// For every holding defender whose charge is low, walk to
/// the nearest working charger. The system releases the
/// `(cell, Defend)` soft work slot and the `DefendHold`
/// marker, then inserts a `ChargerAssignment` and a
/// `DirectMovementComponent` aimed at the charger. A holding
/// defender with no working charger reachable stays in hold;
/// the empty-charge health loss system will drain them per
/// tick until a charger becomes reachable.
///
/// The system filters on `Without<ChargerAssignment>` and
/// `Without<ChargerProgress>` so a defender who is already
/// en route to a charger or already at one is not re-rotated.
/// The rotation is also "soft": if no working charger is
/// reachable, the defender simply stays in hold.
#[allow(clippy::type_complexity)]
pub fn defender_rotation_to_charger_system(
    mut commands: Commands,
    mut slots: ResMut<SoftWorkSlots>,
    defenders: Query<
        (Entity, &DefendHold, &Transform, &Charge, &NanobotType),
        (
            With<Nanobot>,
            With<NanobotType>,
            With<DefendHold>,
            With<Charge>,
            Without<ChargerAssignment>,
            Without<ChargerProgress>,
        ),
    >,
    chargers: Query<(Entity, &Charger, &Transform)>,
) {
    for (entity, hold, transform, charge, nanobot_type) in &defenders {
        if *nanobot_type != NanobotType::Defender {
            continue;
        }
        if !charge.needs_rotation() {
            continue;
        }
        let pos = transform.translation.truncate();
        let Some((charger_entity, charger_pos)) = find_nearest_working_charger(pos, &chargers)
        else {
            continue;
        };
        // Release the soft work slot for the held Defend cell
        // before inserting the rotation marker. Without this
        // release a fresh defender would see the cell as
        // still busy and the held cell would not be
        // available for re-assignment after the rotating
        // defender returns from the charger.
        slots.release(hold.cell, IntentKind::Defend);
        commands.entity(entity).remove::<DefendHold>();
        commands.entity(entity).insert((
            ChargerAssignment {
                charger: charger_entity,
            },
            DirectMovementComponent { xy: charger_pos },
        ));
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
    defenders: Query<
        (Entity, &ChargerAssignment, &Transform),
        (
            With<Nanobot>,
            With<ChargerAssignment>,
            Without<DirectMovementComponent>,
            Without<ChargerProgress>,
        ),
    >,
    chargers: Query<(&Charger, &Transform)>,
) {
    for (entity, assignment, transform) in &defenders {
        let Ok((charger, charger_transform)) = chargers.get(assignment.charger) else {
            // Charger entity disappeared (e.g. despawned by a
            // future collapse system). Drop the assignment
            // and let the defend assignment pool re-pick the
            // defender on the next tick.
            commands.entity(entity).remove::<ChargerAssignment>();
            continue;
        };
        let distance = transform
            .translation
            .truncate()
            .distance(charger_transform.translation.truncate());
        if distance <= charger.radius {
            commands.entity(entity).insert(ChargerProgress {
                charger: assignment.charger,
            });
        }
    }
}

/// Defender charging work system. For every defender with a
/// `ChargerProgress`, refill `Charge` by
/// [`CHARGE_REFILL_PER_TICK`] and drain the charger's
/// `amount` by [`CHARGER_MATERIAL_DRAIN_PER_TICK`]. The
/// defender is released back to the defend assignment pool
/// when the charge is full or the charger runs out of supply.
///
/// The system always runs in the same chain as the rotation
/// and arrive systems; a defender at a fresh charger with
/// empty charge refills on the same tick it arrives, and a
/// defender whose charger empties mid-charge is released on
/// the same tick. The release is a marker remove; the defend
/// assignment system picks the defender back up on the next
/// tick.
#[allow(clippy::type_complexity)]
pub fn defender_charger_work_system(
    mut commands: Commands,
    mut defenders: Query<
        (Entity, &mut Charge, &ChargerAssignment),
        (With<Nanobot>, With<ChargerProgress>),
    >,
    mut chargers: Query<&mut Charger>,
) {
    for (entity, mut charge, assignment) in &mut defenders {
        let Ok(mut charger) = chargers.get_mut(assignment.charger) else {
            // Charger disappeared mid-charge. Drop both
            // markers and let the defender be re-assigned.
            commands.entity(entity).remove::<ChargerAssignment>();
            commands.entity(entity).remove::<ChargerProgress>();
            continue;
        };
        if !charger.has_supply() {
            // Charger emptied between the previous tick and
            // this one. Release the defender; the rotation
            // system will re-pick them on the next tick if
            // another working charger is reachable, and
            // otherwise the empty-charge health-loss system
            // starts to fire.
            commands.entity(entity).remove::<ChargerAssignment>();
            commands.entity(entity).remove::<ChargerProgress>();
            continue;
        }
        // Refill charge and drain charger material FIRST, then
        // check whether the refill brought the charge to max.
        // The drain system runs before the work system in the
        // chain, so the pre-refill charge is one tick's worth
        // below the value the work system left it at; checking
        // `is_full()` *after* the refill is the only way to
        // detect "this tick's refill finished the job".
        //
        // The drain is per-tick and per-defender; multiple
        // defenders at the same charger each drain one unit
        // per tick.
        charge.current = (charge.current + CHARGE_REFILL_PER_TICK).min(charge.max);
        charger.amount = charger
            .amount
            .saturating_sub(CHARGER_MATERIAL_DRAIN_PER_TICK);
        if charge.is_full() {
            // Refill brought the charge to max. Release
            // immediately so the defender returns to the
            // defend pool on the next tick.
            commands.entity(entity).remove::<ChargerAssignment>();
            commands.entity(entity).remove::<ChargerProgress>();
        }
    }
}

// ---------------------------------------------------------------------------
// Plugin
// ---------------------------------------------------------------------------

/// Plugin that wires the charge-sustain systems into the
/// Update schedule.
///
/// As of issue #28, the demand side and the consumer side
/// are split across two update chains so the demand
/// `charger_auto_creation_system` runs **before** the
/// planned-structure claim system (so freshly planned
/// chargers are visible to the next claim tick) and the
/// consumer systems run **after** the planned-structure
/// work system (so freshly promoted chargers are visible
/// to the rotation system).
///
/// Demand chain (single system, ordered with the planned
/// structure plugin's claim system):
///
/// 1. [`charger_auto_creation_system`] -- spawn new planned
///    chargers from current load. The plan is visible
///    immediately; a Worker builds it through the
///    planned-structure lifecycle.
///
/// Consumer chain (after the planned structure work
/// system, so the promotion has fired before rotation
/// reads chargers):
///
/// 1. [`defender_charge_drain_system`] -- passive drain
///    first so the rotation trigger sees the post-drain
///    value.
/// 2. [`defender_health_loss_when_empty_system`] -- health
///    loss fires for holding defenders with empty charge
///    that have not been picked up by the rotation chain.
/// 3. [`defender_rotation_to_charger_system`] -- rotate
///    low-charge holding defenders to working chargers.
/// 4. [`defender_charger_arrive_system`] -- transition
///    arrived defenders into the charging state.
/// 5. [`defender_charger_work_system`] -- refill charge and
///    drain charger material.
pub struct ChargePlugin;

impl Plugin for ChargePlugin {
    fn build(&self, app: &mut App) {
        // Demand: spawn planned chargers from current load
        // before the planned-structure claim system runs so
        // the claim system can pick up a freshly planned
        // charger on the same tick. The chain runs after
        // `move_velocity_system` (so the defenders' cell
        // positions are stable) and after the defend hold
        // system (so load is counted correctly).
        app.add_systems(
            Update,
            charger_auto_creation_system
                .before(crate::nanobot::planned::worker_planned_structure_claim_system)
                .after(crate::nanobot::move_velocity_system)
                .after(crate::nanobot::defend::defender_hold_system),
        );
        // Consumer: drain, health-loss, rotation, arrive,
        // work. The chain runs after the planned-structure
        // work system so a freshly promoted charger is
        // visible to the rotation system's "find nearest
        // working charger" scan in the same tick.
        app.add_systems(
            Update,
            (
                defender_charge_drain_system,
                defender_health_loss_when_empty_system,
                defender_rotation_to_charger_system,
                defender_charger_arrive_system,
                defender_charger_work_system,
            )
                .chain()
                .after(crate::nanobot::move_velocity_system)
                .after(crate::nanobot::defend::defender_hold_system)
                .after(crate::nanobot::planned::worker_planned_structure_work_system),
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

    use super::*;

    #[test]
    fn charge_default_starts_full() {
        let c = Charge::default();
        assert_eq!(c.current, MAX_CHARGE);
        assert_eq!(c.max, MAX_CHARGE);
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
    fn charger_starts_with_initial_amount_and_capacity() {
        let charger = Charger::new(IVec2::new(1, -1));
        assert_eq!(charger.cell, IVec2::new(1, -1));
        assert_eq!(charger.kind, AUTO_CHARGER_KIND);
        assert_eq!(charger.amount, AUTO_CHARGER_INITIAL_AMOUNT);
        assert_eq!(charger.capacity, AUTO_CHARGER_CAPACITY);
        assert_eq!(charger.radius, AUTO_CHARGER_RADIUS);
        assert!(charger.has_supply());
        assert_eq!(
            charger.free_space(),
            AUTO_CHARGER_CAPACITY - AUTO_CHARGER_INITIAL_AMOUNT
        );
    }

    #[test]
    fn charger_has_supply_only_while_amount_is_positive() {
        let mut charger = Charger::new(IVec2::new(0, 0));
        assert!(charger.has_supply());
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
    fn refill_outpaces_drain_so_charge_recovers_at_a_charger() {
        // The contract: a defender at a working charger
        // recovers faster than the passive drain, so the
        // charge trends upward. A test asserting
        // REFILL > DRAIN pins the relationship so a future
        // tuning pass cannot silently break the sustain
        // loop.
        const { assert!(CHARGE_REFILL_PER_TICK > CHARGE_DRAIN_PER_TICK) };
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
        assert_eq!(charge_strength_multiplier(0.0), 0.0);
        assert_eq!(
            charge_strength_multiplier(-0.5),
            0.0,
            "negative input is clamped to zero"
        );
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
        assert_eq!(effective_attack(0.0), 0.0);
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
        assert_eq!(effective_defense(0.0), 0.0);
    }

    #[test]
    fn auto_charger_constants_form_a_consistent_buffer() {
        // A freshly auto-created charger has free space for
        // a hauler delivery AND a non-zero amount so it is
        // already a valid rotation target. The first
        // implementation relies on this: the emergence
        // system gives the swarm a head start instead of
        // expecting the hauler to fill the buffer before
        // the first charge can happen.
        const { assert!(AUTO_CHARGER_INITIAL_AMOUNT < AUTO_CHARGER_CAPACITY) };
        const { assert!(AUTO_CHARGER_INITIAL_AMOUNT > 0) };
    }

    #[test]
    fn cell_charger_cap_is_positive() {
        // Sanity: a cell with no chargers can spawn at
        // least one, otherwise the auto-creation system
        // would do nothing for every defended cell.
        const { assert!(MAX_CHARGERS_PER_CELL >= 1) };
        const { assert!(MAX_DEFENDERS_PER_CHARGER >= 1) };
    }
}
