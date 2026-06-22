//! Central demand allocator with Minimum Category Activation.
//!
//! Issue #35 contract: per swarm and per work category, when there is
//! valid demand and at least one eligible idle nanobot, the central
//! allocator assigns one nanobot to that category *before* the
//! per-category assignment systems can independently pick from the
//! idle pool. This is the "Minimum Category Activation" half of the
//! project glossary: each valid work category must keep at least
//! one worker active, so newly drawn intent visibly receives a
//! worker response promptly even when another category would
//! otherwise score higher.
//!
//! The module is the single path for "first allocation" of an
//! idle nanobot. After minimum activation is satisfied, the
//! per-category systems (Gather, Planned Structure) continue to
//! score remaining idle nanobots and assign them through their
//! own normal-scoring paths. The per-category systems
//! naturally skip a nanobot the central allocator has already
//! claimed, because the claim inserts the same
//! per-category marker plus a `DirectMovementComponent` -- and
//! every per-category assignment system filters
//! `Without<DirectMovementComponent>`.
//!
//! ## Categories in v1
//!
//! ```text
//!   Gather        -- a Worker is en route to a deposit in a
//!                    painted Gather cell (Source Stockpile
//!                    demand is a follow-on concern, not a
//!                    first-class category here).
//!   PlannedBuild  -- a Worker is en route to an unclaimed
//!                    `PlannedStructure` (any kind: Source
//!                    Stockpile, Sink Stockpile, Production
//!                    Facility, Charger). The "Planned Structure"
//!                    language in the issue maps to this single
//!                    category because the Build lifecycle is
//!                    identical across kinds.
//! ```
//!
//! Future work categories (Defend, Corridor, ...) plug in by
//! adding a variant to [`DemandCategory`] and matching it in
//! the enumeration and eligibility helpers below.
//!
//! ## What "eligible idle" means in v1
//!
//! A nanobot is eligible for both v1 categories when:
//! - it is a `Worker` (the project's "direct work" type),
//! - its `Commitment` is `Idle`,
//! - it has no in-flight assignment marker
//!   (`GatherAssignment`, `PlannedStructureClaim`,
//!   `DirectMovementComponent`, etc.),
//! - and it belongs to the same `SwarmId` as the demand.
//!
//! The eligibility helper is a single function so future
//! categories can refine the rule without rewriting the
//! central allocator.

use std::collections::HashMap;

use bevy::math::Vec2;
use bevy::prelude::*;

use crate::ai::get_world_from_zone;
use crate::intent::{IntentGrid, IntentKind};
use crate::nanobot::autonomy::{Commitment, NanobotType, SoftWorkSlots};
use crate::nanobot::components::{DirectMovementComponent, Nanobot, SwarmId, SwarmMember};
use crate::nanobot::gather::{cell_overlaps_circle, GatherAssignment};
use crate::nanobot::planned::{PlannedStructure, PlannedStructureClaim, PlannedStructureProgress};
use crate::nanobot::production::OwnerSwarm;
use crate::resources::{ResourceDeposit, ResourceKind};

/// Top-level work categories the central allocator reasons about.
///
/// The order in the enum is also the index in the
/// `[u32; COUNT]` arrays on the per-(swarm, category)
/// resources. Adding a new category means appending a
/// variant and growing [`DemandCategory::ALL`] /
/// [`DemandCategory::COUNT`] accordingly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DemandCategory {
    /// A worker is committed to extracting from a deposit
    /// in a painted Gather cell.
    Gather,
    /// A worker is committed to building a
    /// [`PlannedStructure`] (any kind) that the demand
    /// systems have already created.
    PlannedBuild,
}

impl DemandCategory {
    /// All categories in stable declaration order. Used to
    /// size the per-(swarm, category) tables and to iterate
    /// every category in the publication / enforcement
    /// systems.
    pub const ALL: [DemandCategory; Self::COUNT] =
        [DemandCategory::Gather, DemandCategory::PlannedBuild];

    /// Number of distinct categories. Matches
    /// [`DemandCategory::ALL`].
    pub const COUNT: usize = 2;

    /// Stable per-category index in `[0, COUNT)`.
    pub const fn index(self) -> usize {
        match self {
            DemandCategory::Gather => 0,
            DemandCategory::PlannedBuild => 1,
        }
    }
}

/// One unit of valid work in one (swarm, category).
///
/// `world_position` is the world-space anchor a worker
/// must travel to (deposit centre for Gather, planned
/// structure position for PlannedBuild).
///
/// `target` is the entity backing the demand: the
/// [`ResourceDeposit`] for [`DemandCategory::Gather`] and
/// the [`PlannedStructure`] for
/// [`DemandCategory::PlannedBuild`]. The central
/// allocator's minimum-activation claim uses it to insert
/// the per-category marker (e.g.
/// [`GatherAssignment::new`]) with the right target.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DemandItem {
    pub cell: IVec2,
    pub category: DemandCategory,
    pub paint_strength: u8,
    pub world_position: Vec2,
    pub target: Entity,
}

/// Swarm-wide snapshot of the demand items the central
/// allocator observed this tick.
///
/// The resource is updated once per tick by
/// [`central_demand_allocation_system`]. Tests and
/// observers can read it to assert "the allocator saw
/// these demand items" without re-running the
/// enumeration helpers. The map is keyed by [`SwarmId`];
/// swarms with no demand are absent from the map.
#[derive(Debug, Default, Clone, Resource)]
pub struct DemandSnapshot {
    pub by_swarm: HashMap<SwarmId, Vec<DemandItem>>,
}

impl DemandSnapshot {
    /// Demand items for `swarm`, or an empty slice.
    pub fn for_swarm(&self, swarm: SwarmId) -> &[DemandItem] {
        self.by_swarm.get(&swarm).map(Vec::as_slice).unwrap_or(&[])
    }

    /// True when `swarm` has at least one valid demand
    /// item in `category`.
    pub fn has_demand(&self, swarm: SwarmId, category: DemandCategory) -> bool {
        self.for_swarm(swarm).iter().any(|i| i.category == category)
    }
}

/// True when a nanobot of `nanobot_type` is eligible to do
/// work in a v1 category. v1 has a single Worker type that
/// handles both Gather and Planned Build. Future types can
/// refine the rule by replacing this helper.
///
/// The function is a plain pure helper so tests can pin
/// the eligibility contract without instantiating Bevy.
pub fn is_eligible_for(nanobot_type: NanobotType) -> bool {
    matches!(nanobot_type, NanobotType::Worker)
}

/// Snapshot of active worker counts per (swarm, category).
///
/// The central allocator reads this to decide which
/// categories need minimum activation. "Active" means
/// "has the per-category assignment marker", which
/// already implies the worker is on the way (or
/// arrived) at a work site:
///
/// - Gather: `With<GatherAssignment>` (subsumes
///   `ExtractProgress`, `ReturningToStockpile`, etc.)
/// - PlannedBuild: `With<PlannedStructureClaim>`
///   (subsumes `PlannedStructureProgress`).
///
/// The resource is updated once per tick by
/// [`central_demand_allocation_system`].
#[derive(Debug, Default, Clone, Resource)]
pub struct ActiveWorkerCounts {
    by_swarm: HashMap<SwarmId, [u32; DemandCategory::COUNT]>,
}

impl ActiveWorkerCounts {
    /// Active count for `(swarm, category)`. `0` when the
    /// swarm has no recorded entry yet.
    pub fn count(&self, swarm: SwarmId, category: DemandCategory) -> u32 {
        self.by_swarm
            .get(&swarm)
            .and_then(|arr| arr.get(category.index()))
            .copied()
            .unwrap_or(0)
    }

    fn increment(&mut self, swarm: SwarmId, category: DemandCategory) {
        self.by_swarm.entry(swarm).or_default()[category.index()] += 1;
    }

    /// Reset all counts to zero. Called at the start of
    /// every central-allocator tick before the per-(swarm,
    /// category) recount.
    fn reset(&mut self) {
        for arr in self.by_swarm.values_mut() {
            for v in arr.iter_mut() {
                *v = 0;
            }
        }
    }
}

/// One minimum-activation claim the central allocator
/// wants to insert. The system reads the vector and
/// applies each claim (marker + `DirectMovementComponent`)
/// using the same code path the per-category systems
/// would.
#[derive(Debug, Clone, Copy)]
pub struct MinActivationClaim {
    pub worker: Entity,
    pub swarm: SwarmId,
    pub category: DemandCategory,
    pub cell: IVec2,
    pub world_position: Vec2,
    /// For Gather: the deposit entity. For PlannedBuild:
    /// the planned structure entity.
    pub target: Entity,
}

/// Lightweight snapshot of an eligible idle nanobot.
/// Built by the system from a `Query` and fed to the
/// pure picker helper.
#[derive(Debug, Clone, Copy)]
pub struct IdleBot {
    pub entity: Entity,
    pub swarm: SwarmId,
    pub position: Vec2,
}

impl IdleBot {
    /// Build the `IdleBot` from a query row, applying the
    /// "eligible idle" filter (Worker type, Idle
    /// commitment). Returns `None` otherwise.
    pub fn from_query(
        entity: Entity,
        nanobot_type: &NanobotType,
        commitment: &Commitment,
        transform: &Transform,
        swarm: &SwarmMember,
    ) -> Option<Self> {
        if !is_eligible_for(*nanobot_type) {
            return None;
        }
        if *commitment != Commitment::Idle {
            return None;
        }
        Some(Self {
            entity,
            swarm: swarm.0,
            position: transform.translation.truncate(),
        })
    }
}

/// Pick the nearest eligible idle nanobot to the
/// demand's `world_position`, restricted to the same
/// swarm. Returns `None` when no eligible bot exists.
pub fn pick_nearest_idle<'a>(
    item: &DemandItem,
    demand_swarm: SwarmId,
    idle_bots: &'a [IdleBot],
) -> Option<&'a IdleBot> {
    idle_bots
        .iter()
        .filter(|bot| bot.swarm == demand_swarm)
        .min_by(|a, b| {
            let da = a.position.distance(item.world_position);
            let db = b.position.distance(item.world_position);
            da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
        })
}

/// Choose the demand item inside `category` whose
/// `world_position` is nearest to `bot_position`. Used
/// by the system to translate a (swarm, category) into a
/// concrete (cell, target) pair for the claim.
pub fn nearest_demand_item(
    snapshot: &DemandSnapshot,
    swarm: SwarmId,
    category: DemandCategory,
    bot_position: Vec2,
) -> Option<&DemandItem> {
    snapshot
        .for_swarm(swarm)
        .iter()
        .filter(|item| item.category == category)
        .min_by(|a, b| {
            let da = a.world_position.distance(bot_position);
            let db = b.world_position.distance(bot_position);
            da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
        })
}

/// Compute every (swarm, category) minimum-activation
/// claim the central allocator should issue this tick.
///
/// Pure helper: the system builds the `snapshot` and
/// `idle_bots` from the Bevy queries, then calls this
/// function to decide which claims to apply. Splitting
/// the decision from the application keeps the
/// allocation rule testable without instantiating
/// Bevy.
///
/// The function returns one claim per (swarm, category)
/// with demand and zero active workers and at least one
/// eligible idle bot, choosing the nearest eligible bot
/// to the nearest demand item in that category. The
/// returned `MinActivationClaim::target` is the demand
/// item's `target` (deposit for Gather, planned
/// structure for PlannedBuild), which is exactly the
/// entity the per-category marker needs.
///
/// The returned vector is ordered by
/// [`DemandCategory::ALL`] so two categories compete
/// for a single idle bot deterministically: the
/// first-listed category in the result wins the bot
/// and the second-listed category is left without a
/// claim for this tick. The `pick_nearest_idle` step
/// inside the loop is what enforces the "one bot per
/// claim" rule.
pub fn minimum_activation_claims(
    snapshot: &DemandSnapshot,
    active: &ActiveWorkerCounts,
    idle_bots: &[IdleBot],
) -> Vec<MinActivationClaim> {
    let mut out = Vec::new();
    let mut claimed: std::collections::HashSet<Entity> = std::collections::HashSet::new();
    for swarm in snapshot.by_swarm.keys() {
        for category in DemandCategory::ALL {
            if active.count(*swarm, category) > 0 {
                continue;
            }
            if !snapshot.has_demand(*swarm, category) {
                continue;
            }
            // Find the best demand item in this (swarm,
            // category): the one whose world anchor is
            // nearest to the eventual bot. We use the
            // centroid of the eligible idle bots as a
            // tie-breaker proxy so the choice is stable
            // even with multiple eligible bots.
            let bots_in_swarm: Vec<&IdleBot> = idle_bots
                .iter()
                .filter(|b| b.swarm == *swarm && !claimed.contains(&b.entity))
                .collect();
            if bots_in_swarm.is_empty() {
                continue;
            }
            let centroid: Vec2 = {
                let mut sum = Vec2::ZERO;
                for b in &bots_in_swarm {
                    sum += b.position;
                }
                sum / bots_in_swarm.len() as f32
            };
            let Some(item) = nearest_demand_item(snapshot, *swarm, category, centroid) else {
                continue;
            };
            let Some(bot) = pick_nearest_idle(item, *swarm, idle_bots) else {
                continue;
            };
            if claimed.contains(&bot.entity) {
                continue;
            }
            claimed.insert(bot.entity);
            out.push(MinActivationClaim {
                worker: bot.entity,
                swarm: *swarm,
                category,
                cell: item.cell,
                world_position: item.world_position,
                target: item.target,
            });
        }
    }
    out
}

/// Central demand allocation system. Runs once per
/// tick, after movement and before the per-category
/// assignment systems (`worker_gather_assignment_system`,
/// `worker_planned_structure_claim_system`).
///
/// The system has three responsibilities:
/// 1. Publish the current per-swarm demand snapshot.
/// 2. Recount active workers per (swarm, category).
/// 3. For each (swarm, category) with demand, zero
///    active workers, and at least one eligible idle
///    bot, claim one bot via the per-category marker
///    and a `DirectMovementComponent`.
///
/// The per-category systems that follow naturally skip
/// the pre-claimed bot because they all filter
/// `Without<DirectMovementComponent>`. The bot then
/// drives through the normal work lifecycle: the
/// movement system routes it to the demand anchor, the
/// arrive / claim / work systems promote it to the
/// working state, and the category-specific completion
/// path takes over.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub fn central_demand_allocation_system(
    mut commands: Commands,
    grid: Res<IntentGrid>,
    mut snapshot: ResMut<DemandSnapshot>,
    mut active: ResMut<ActiveWorkerCounts>,
    mut slots: ResMut<SoftWorkSlots>,
    deposits: Query<(Entity, &ResourceDeposit, &Transform)>,
    mut planned_q: Query<(
        Entity,
        &mut PlannedStructure,
        &Transform,
        Option<&OwnerSwarm>,
    )>,
    gather_q: Query<&SwarmMember, (With<Nanobot>, With<GatherAssignment>)>,
    planned_claim_q: Query<&SwarmMember, (With<Nanobot>, With<PlannedStructureClaim>)>,
    idle_q: Query<
        (Entity, &Transform, &Commitment, &NanobotType, &SwarmMember),
        (
            With<Nanobot>,
            Without<GatherAssignment>,
            Without<PlannedStructureClaim>,
            Without<PlannedStructureProgress>,
            Without<DirectMovementComponent>,
        ),
    >,
    swarms: Query<&SwarmId>,
) {
    // Step 1: publish the per-swarm demand snapshot.
    let mut new_snapshot = DemandSnapshot::default();

    // Gather demand: every swarm whose paint is visible
    // to it can produce a Gather demand item.
    for (cell_pos, cell) in grid.iter_cells() {
        // Collect every swarm that can see this cell's
        // Gather paint. Owner-aware: a cell painted by
        // swarm A is visible to swarm A and to any
        // unowned reader.
        let cell_owners: Vec<SwarmId> = swarms
            .iter()
            .filter(|swarm| cell.visible_to(IntentKind::Gather, **swarm))
            .copied()
            .collect();
        if cell_owners.is_empty() {
            continue;
        }
        // Per-cell deposit anchor.
        let cell_center = get_world_from_zone(cell_pos);
        let mut anchor: Option<(Vec2, Entity)> = None;
        for (entity, deposit, transform) in &deposits {
            if deposit.kind != ResourceKind::Minerals || deposit.amount == 0 {
                continue;
            }
            let deposit_pos = transform.translation.truncate();
            if !cell_overlaps_circle(cell_pos, deposit_pos, deposit.radius) {
                continue;
            }
            let closer = match anchor {
                None => true,
                Some((current, _)) => {
                    (deposit_pos - cell_center).length() < (current - cell_center).length()
                }
            };
            if closer {
                anchor = Some((deposit_pos, entity));
            }
        }
        let (deposit_pos, deposit_entity) = match anchor {
            Some((p, e)) => (p, e),
            None => continue,
        };
        for owner in &cell_owners {
            new_snapshot
                .by_swarm
                .entry(*owner)
                .or_default()
                .push(DemandItem {
                    cell: cell_pos,
                    category: DemandCategory::Gather,
                    paint_strength: cell.strength(IntentKind::Gather),
                    world_position: deposit_pos,
                    target: deposit_entity,
                });
        }
    }

    // Planned Build demand: every unclaimed planned
    // structure is a demand item for its owner (or for
    // every swarm when unowned).
    for (entity, planned, transform, owner) in &planned_q {
        if !planned.is_unclaimed() {
            continue;
        }
        let owner_swarm_id: Option<SwarmId> = owner.and_then(|o| swarms.get(o.0).ok().copied());
        let target_swarms: Vec<SwarmId> = match owner_swarm_id {
            None => swarms.iter().copied().collect(),
            Some(s) => vec![s],
        };
        for owner in &target_swarms {
            new_snapshot
                .by_swarm
                .entry(*owner)
                .or_default()
                .push(DemandItem {
                    cell: planned.cell,
                    category: DemandCategory::PlannedBuild,
                    paint_strength: 1,
                    world_position: transform.translation.truncate(),
                    target: entity,
                });
        }
    }

    *snapshot = new_snapshot;

    // Step 2: recount active workers per (swarm,
    // category) by reading the per-category marker
    // queries.
    active.reset();
    for swarm_member in &gather_q {
        active.increment(swarm_member.0, DemandCategory::Gather);
    }
    for swarm_member in &planned_claim_q {
        active.increment(swarm_member.0, DemandCategory::PlannedBuild);
    }

    // Step 3: build the eligible-idle list and compute
    // minimum-activation claims through the pure helper.
    let idle_bots: Vec<IdleBot> = idle_q
        .iter()
        .filter_map(|(entity, transform, commitment, nanobot_type, swarm)| {
            IdleBot::from_query(entity, nanobot_type, commitment, transform, swarm)
        })
        .collect();

    let claims = minimum_activation_claims(&snapshot, &active, &idle_bots);

    for claim in claims {
        // Update the active count immediately so a
        // subsequent claim in the same tick sees the
        // pre-claimed worker as already active. The
        // marker insertion via `Commands` is deferred
        // to the end of the schedule, but the resource
        // is mutated in-place here.
        active.increment(claim.swarm, claim.category);
        match claim.category {
            DemandCategory::Gather => {
                slots.occupy(claim.cell, IntentKind::Gather);
                commands.entity(claim.worker).insert((
                    GatherAssignment::new(claim.cell, claim.target),
                    DirectMovementComponent {
                        xy: claim.world_position,
                    },
                ));
            }
            DemandCategory::PlannedBuild => {
                if let Ok((_, mut planned_state, _, _)) = planned_q.get_mut(claim.target) {
                    planned_state.active_worker = Some(claim.worker);
                }
                commands.entity(claim.worker).insert((
                    PlannedStructureClaim {
                        cell: claim.cell,
                        target: claim.target,
                    },
                    DirectMovementComponent {
                        xy: claim.world_position,
                    },
                ));
            }
        }
    }
}

/// Plugin that wires the central demand allocator into
/// the Update schedule. The system runs after
/// `move_velocity_system` (so the active / idle state
/// reflects the post-move world) and before the
/// per-category assignment systems (so the first
/// allocation of an idle nanobot flows through the
/// central allocator).
pub struct CentralDemandPlugin;

impl Plugin for CentralDemandPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DemandSnapshot>()
            .init_resource::<ActiveWorkerCounts>()
            .add_systems(
                Update,
                central_demand_allocation_system
                    .after(crate::nanobot::move_velocity_system)
                    .before(crate::nanobot::gather::worker_gather_assignment_system)
                    .before(crate::nanobot::planned::worker_planned_structure_claim_system),
            );
    }
}

#[cfg(test)]
mod tests {
    //! Pure-helper unit tests. The end-to-end minimum-
    //! activation contract is covered by
    //! `tests/behavior/central_demand_allocator.rs`.

    use super::*;

    #[test]
    fn demand_category_all_covers_gather_and_planned_build() {
        let kinds: Vec<DemandCategory> = DemandCategory::ALL.to_vec();
        assert_eq!(kinds.len(), DemandCategory::COUNT);
        assert!(kinds.contains(&DemandCategory::Gather));
        assert!(kinds.contains(&DemandCategory::PlannedBuild));
        for (i, k) in kinds.iter().enumerate() {
            assert_eq!(k.index(), i, "index must match declaration order");
        }
    }

    #[test]
    fn only_worker_is_eligible_for_v1_categories() {
        // The v1 contract: every category is Worker
        // work. Future categories can refine the rule.
        assert!(is_eligible_for(NanobotType::Worker));
        assert!(!is_eligible_for(NanobotType::Hauler));
        assert!(!is_eligible_for(NanobotType::Defender));
    }

    #[test]
    fn demand_snapshot_starts_empty_and_reports_no_demand() {
        let snap = DemandSnapshot::default();
        assert!(snap.for_swarm(SwarmId::PLAYER).is_empty());
        assert!(!snap.has_demand(SwarmId::PLAYER, DemandCategory::Gather));
        assert!(!snap.has_demand(SwarmId::PLAYER, DemandCategory::PlannedBuild));
    }

    #[test]
    fn demand_snapshot_reports_demand_only_for_pushed_items() {
        let mut snap = DemandSnapshot::default();
        snap.by_swarm.insert(
            SwarmId::PLAYER,
            vec![DemandItem {
                cell: IVec2::new(0, 0),
                category: DemandCategory::Gather,
                paint_strength: 1,
                world_position: Vec2::ZERO,
                target: Entity::from_raw_u32(7).expect("test entity"),
            }],
        );
        assert!(snap.has_demand(SwarmId::PLAYER, DemandCategory::Gather));
        assert!(!snap.has_demand(SwarmId::PLAYER, DemandCategory::PlannedBuild));
        // A different swarm sees no demand.
        let other = SwarmId(42);
        assert!(!snap.has_demand(other, DemandCategory::Gather));
    }

    #[test]
    fn active_counts_reset_between_ticks() {
        let mut active = ActiveWorkerCounts::default();
        active.increment(SwarmId::PLAYER, DemandCategory::Gather);
        assert_eq!(active.count(SwarmId::PLAYER, DemandCategory::Gather), 1);
        active.reset();
        assert_eq!(active.count(SwarmId::PLAYER, DemandCategory::Gather), 0);
    }

    #[test]
    fn pick_nearest_idle_prefers_closest_to_demand_anchor() {
        let item = DemandItem {
            cell: IVec2::new(0, 0),
            category: DemandCategory::Gather,
            paint_strength: 1,
            world_position: Vec2::new(100.0, 0.0),
            target: Entity::from_raw_u32(99).expect("test entity"),
        };
        let bots = vec![
            IdleBot {
                entity: Entity::from_raw_u32(1).expect("test entity"),
                swarm: SwarmId::PLAYER,
                position: Vec2::new(0.0, 0.0),
            },
            IdleBot {
                entity: Entity::from_raw_u32(2).expect("test entity"),
                swarm: SwarmId::PLAYER,
                position: Vec2::new(110.0, 0.0),
            },
            IdleBot {
                entity: Entity::from_raw_u32(3).expect("test entity"),
                swarm: SwarmId::PLAYER,
                position: Vec2::new(200.0, 0.0),
            },
        ];
        let picked = pick_nearest_idle(&item, SwarmId::PLAYER, &bots).expect("must pick a bot");
        assert_eq!(picked.entity, Entity::from_raw_u32(2).expect("test entity"));
    }

    #[test]
    fn pick_nearest_idle_skips_other_swarms() {
        let item = DemandItem {
            cell: IVec2::new(0, 0),
            category: DemandCategory::Gather,
            paint_strength: 1,
            world_position: Vec2::new(100.0, 0.0),
            target: Entity::from_raw_u32(99).expect("test entity"),
        };
        let bots = vec![
            IdleBot {
                entity: Entity::from_raw_u32(1).expect("test entity"),
                swarm: SwarmId(99),
                position: Vec2::new(100.0, 0.0),
            },
            IdleBot {
                entity: Entity::from_raw_u32(2).expect("test entity"),
                swarm: SwarmId::PLAYER,
                position: Vec2::new(200.0, 0.0),
            },
        ];
        let picked =
            pick_nearest_idle(&item, SwarmId::PLAYER, &bots).expect("must pick the player bot");
        assert_eq!(picked.entity, Entity::from_raw_u32(2).expect("test entity"));
    }

    #[test]
    fn pick_nearest_idle_returns_none_when_no_matching_swarm() {
        let item = DemandItem {
            cell: IVec2::new(0, 0),
            category: DemandCategory::Gather,
            paint_strength: 1,
            world_position: Vec2::ZERO,
            target: Entity::from_raw_u32(99).expect("test entity"),
        };
        let bots = vec![IdleBot {
            entity: Entity::from_raw_u32(1).expect("test entity"),
            swarm: SwarmId(42),
            position: Vec2::ZERO,
        }];
        assert!(pick_nearest_idle(&item, SwarmId::PLAYER, &bots).is_none());
    }

    #[test]
    fn nearest_demand_item_picks_closest_in_category() {
        let mut snap = DemandSnapshot::default();
        snap.by_swarm.insert(
            SwarmId::PLAYER,
            vec![
                DemandItem {
                    cell: IVec2::new(0, 0),
                    category: DemandCategory::Gather,
                    paint_strength: 1,
                    world_position: Vec2::new(0.0, 0.0),
                    target: Entity::from_raw_u32(1).expect("test entity"),
                },
                DemandItem {
                    cell: IVec2::new(2, 0),
                    category: DemandCategory::Gather,
                    paint_strength: 1,
                    world_position: Vec2::new(200.0, 0.0),
                    target: Entity::from_raw_u32(2).expect("test entity"),
                },
                DemandItem {
                    cell: IVec2::new(0, 0),
                    category: DemandCategory::PlannedBuild,
                    paint_strength: 1,
                    world_position: Vec2::new(999.0, 0.0),
                    target: Entity::from_raw_u32(3).expect("test entity"),
                },
            ],
        );
        let picked = nearest_demand_item(
            &snap,
            SwarmId::PLAYER,
            DemandCategory::Gather,
            Vec2::new(180.0, 0.0),
        )
        .expect("must pick a Gather item");
        assert_eq!(picked.cell, IVec2::new(2, 0));
    }

    #[test]
    fn minimum_activation_claims_one_per_swarm_category_with_demand() {
        // A snapshot with both Gather and PlannedBuild
        // demand for the player swarm. With one idle
        // player Worker available, the central
        // allocator should claim exactly one of the
        // two categories (the first in ALL order) and
        // leave the other unclaimed for this tick --
        // a single bot cannot satisfy two minimum
        // activations at once.
        let mut snap = DemandSnapshot::default();
        snap.by_swarm.insert(
            SwarmId::PLAYER,
            vec![
                DemandItem {
                    cell: IVec2::new(0, 0),
                    category: DemandCategory::Gather,
                    paint_strength: 1,
                    world_position: Vec2::new(100.0, 0.0),
                    target: Entity::from_raw_u32(1).expect("test entity"),
                },
                DemandItem {
                    cell: IVec2::new(0, 0),
                    category: DemandCategory::PlannedBuild,
                    paint_strength: 1,
                    world_position: Vec2::new(200.0, 0.0),
                    target: Entity::from_raw_u32(2).expect("test entity"),
                },
            ],
        );
        let active = ActiveWorkerCounts::default();
        let bots = vec![IdleBot {
            entity: Entity::from_raw_u32(7).expect("test entity"),
            swarm: SwarmId::PLAYER,
            position: Vec2::new(150.0, 0.0),
        }];
        let claims = minimum_activation_claims(&snap, &active, &bots);
        assert_eq!(claims.len(), 1, "one bot cannot satisfy two activations");
        assert_eq!(claims[0].category, DemandCategory::Gather);
    }

    #[test]
    fn minimum_activation_claims_one_per_swarm_category_when_bot_available() {
        // With two idle bots and two categories, the
        // central allocator should issue one claim per
        // category.
        let mut snap = DemandSnapshot::default();
        snap.by_swarm.insert(
            SwarmId::PLAYER,
            vec![
                DemandItem {
                    cell: IVec2::new(0, 0),
                    category: DemandCategory::Gather,
                    paint_strength: 1,
                    world_position: Vec2::new(100.0, 0.0),
                    target: Entity::from_raw_u32(1).expect("test entity"),
                },
                DemandItem {
                    cell: IVec2::new(0, 0),
                    category: DemandCategory::PlannedBuild,
                    paint_strength: 1,
                    world_position: Vec2::new(200.0, 0.0),
                    target: Entity::from_raw_u32(2).expect("test entity"),
                },
            ],
        );
        let active = ActiveWorkerCounts::default();
        let bots = vec![
            IdleBot {
                entity: Entity::from_raw_u32(7).expect("test entity"),
                swarm: SwarmId::PLAYER,
                position: Vec2::new(80.0, 0.0),
            },
            IdleBot {
                entity: Entity::from_raw_u32(8).expect("test entity"),
                swarm: SwarmId::PLAYER,
                position: Vec2::new(220.0, 0.0),
            },
        ];
        let claims = minimum_activation_claims(&snap, &active, &bots);
        assert_eq!(claims.len(), 2, "two bots, two categories, two claims");
        let categories: Vec<DemandCategory> = claims.iter().map(|c| c.category).collect();
        assert!(categories.contains(&DemandCategory::Gather));
        assert!(categories.contains(&DemandCategory::PlannedBuild));
    }

    #[test]
    fn minimum_activation_skips_categories_with_active_workers() {
        // A category with one active worker does not
        // need another claim: the min-activation
        // rule says "at least one", which the
        // existing worker already satisfies.
        let mut snap = DemandSnapshot::default();
        snap.by_swarm.insert(
            SwarmId::PLAYER,
            vec![DemandItem {
                cell: IVec2::new(0, 0),
                category: DemandCategory::Gather,
                paint_strength: 1,
                world_position: Vec2::new(100.0, 0.0),
                target: Entity::from_raw_u32(1).expect("test entity"),
            }],
        );
        let mut active = ActiveWorkerCounts::default();
        active.increment(SwarmId::PLAYER, DemandCategory::Gather);
        let bots = vec![IdleBot {
            entity: Entity::from_raw_u32(7).expect("test entity"),
            swarm: SwarmId::PLAYER,
            position: Vec2::new(0.0, 0.0),
        }];
        let claims = minimum_activation_claims(&snap, &active, &bots);
        assert!(
            claims.is_empty(),
            "category already has an active worker; no claim needed"
        );
    }

    #[test]
    fn minimum_activation_skips_swarms_without_demand() {
        // A swarm with no demand must not be claimed.
        let snap = DemandSnapshot::default();
        let active = ActiveWorkerCounts::default();
        let bots = vec![IdleBot {
            entity: Entity::from_raw_u32(7).expect("test entity"),
            swarm: SwarmId::PLAYER,
            position: Vec2::new(0.0, 0.0),
        }];
        let claims = minimum_activation_claims(&snap, &active, &bots);
        assert!(claims.is_empty());
    }

    #[test]
    fn minimum_activation_skips_swarms_without_idle_bots() {
        // A swarm with demand but no idle bot is
        // left unactivated for this tick. The rule
        // does not invent work.
        let mut snap = DemandSnapshot::default();
        snap.by_swarm.insert(
            SwarmId::PLAYER,
            vec![DemandItem {
                cell: IVec2::new(0, 0),
                category: DemandCategory::Gather,
                paint_strength: 1,
                world_position: Vec2::new(100.0, 0.0),
                target: Entity::from_raw_u32(1).expect("test entity"),
            }],
        );
        let active = ActiveWorkerCounts::default();
        let bots = vec![IdleBot {
            entity: Entity::from_raw_u32(7).expect("test entity"),
            swarm: SwarmId(42),
            position: Vec2::new(0.0, 0.0),
        }];
        let claims = minimum_activation_claims(&snap, &active, &bots);
        assert!(claims.is_empty());
    }
}
