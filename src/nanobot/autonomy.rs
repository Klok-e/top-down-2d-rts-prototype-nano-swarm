//! Dumb autonomy for nanobots.
//!
//! Per the project glossary a nanobot is "aware of player-painted intent
//! globally, but executes it through simple scoring rather than optimal
//! assignment". The scoring lives in this module as pure functions over
//! plain Rust data so the simulation contract can be tested without Bevy
//! rendering or scheduler setup.
//!
//! Five factors feed score: type fit, need, distance, commitment, and soft work
//! slot pressure. Score is plain `f32`; callers pick best candidate across all
//! painted cells.
//!
//! ## Commitment simplification
//!
//! [`score_intent`] applies the commitment factor symmetrically to all
//! candidates. The proper contract ("carrying nanobots usually finish
//! delivery") is approximated by a small multiplier for the carrying
//! state, so any new work has to be much more attractive to pull the
//! nanobot away. A "boost the current cell" hook is a follow-up issue.

use std::collections::HashMap;

use bevy::{
    math::Vec2,
    prelude::{Component, IVec2, Resource},
    reflect::Reflect,
};

use crate::intent::{IntentCell, IntentGrid, IntentKind};
use crate::nanobot::components::SwarmId;

/// Specialization of a nanobot. The player does not assign individual
/// nanobots to types manually (see the project glossary); types emerge
/// from the production priority and are stored on the entity.
#[derive(Debug, Clone, Copy, Component, PartialEq, Eq, Hash, Reflect, Default)]
pub enum NanobotType {
    /// Performs direct work at resource deposits and construction sites,
    /// and can carry small resource amounts when needed.
    #[default]
    Worker,
    /// Specialized for transporting resources between stockpiles,
    /// facilities, and other resource needs. Carries much more than a
    /// worker.
    Hauler,
    /// Protects swarm assets and fights threats inside defend zones.
    Defender,
}

impl NanobotType {
    /// Number of distinct nanobot types. Matches the project glossary:
    /// Worker, Hauler, Defender.
    pub const COUNT: usize = 3;

    /// All nanobot types in stable glossary order.
    pub const ALL: [NanobotType; Self::COUNT] = [
        NanobotType::Worker,
        NanobotType::Hauler,
        NanobotType::Defender,
    ];

    /// Type-fit contribution for a given [`IntentKind`]. Higher means
    /// the nanobot is a better match for that intent layer. The
    /// Corridor layer is hauler path guidance rather than a
    /// work-producing intent, so it scores 0 for every type -- no
    /// nanobot is "fit" for a corridor because corridors do not
    /// create work.
    pub fn fit_for(self, kind: IntentKind) -> f32 {
        match (self, kind) {
            (NanobotType::Worker, IntentKind::Gather) => 1.0,
            (NanobotType::Worker, IntentKind::Build) => 1.0,
            (NanobotType::Worker, IntentKind::Defend) => 0.0,
            (NanobotType::Worker, IntentKind::Corridor) => 0.0,

            (NanobotType::Hauler, IntentKind::Gather) => 0.0,
            (NanobotType::Hauler, IntentKind::Build) => 0.5,
            (NanobotType::Hauler, IntentKind::Defend) => 0.0,
            (NanobotType::Hauler, IntentKind::Corridor) => 1.0,

            (NanobotType::Defender, IntentKind::Gather) => 0.0,
            (NanobotType::Defender, IntentKind::Build) => 0.0,
            (NanobotType::Defender, IntentKind::Defend) => 1.0,
            (NanobotType::Defender, IntentKind::Corridor) => 0.0,
        }
    }
}

/// A nanobot's tendency to finish its current short task before
/// reconsidering player intent. Per the project glossary: idle
/// nanobots react immediately, carrying nanobots usually finish
/// delivery, and active workers usually finish a short work chunk
/// before reassessing.
#[derive(Debug, Clone, Copy, Component, PartialEq, Eq, Hash, Reflect, Default)]
pub enum Commitment {
    /// No current task. Reacts immediately to useful global intent.
    #[default]
    Idle,
    /// Currently carrying resources to a sink. Usually finishes delivery
    /// before reassessing.
    Carrying,
    /// Actively working on a short task at a cell. Usually finishes that
    /// short work chunk before reassessing.
    Working,
}

impl Commitment {
    /// Multiplier the scoring function applies to the score. Values are
    /// in `(0, 1]`: closer to `1.0` means the nanobot will reassess
    /// more eagerly, closer to `0.0` means it will keep its current
    /// work. The acceptance criteria only require "carrying nanobots
    /// usually finish delivery" and "active workers finish short work
    /// chunks", not a specific multiplier.
    pub fn reassess_factor(self) -> f32 {
        match self {
            Commitment::Idle => 1.0,
            // Carrying nanobots strongly prefer to finish delivery; the
            // factor is small so other intent rarely wins.
            Commitment::Carrying => 0.1,
            // Active workers finish a short work chunk first. They can
            // still be tempted by a much higher score, which is the
            // "soft" part of the commitment contract.
            Commitment::Working => 0.4,
        }
    }
}

/// Tracks how many nanobots are currently committed to each
/// `(cell, intent kind)` pair. Inserted as a Bevy [`Resource`] so
/// scoring can read the current crowding without recomputing it from
/// scratch every frame.
#[derive(Debug, Default, Clone, Resource)]
pub struct SoftWorkSlots {
    /// Keyed by `(cell, intent kind)`. Values are the current number of
    /// nanobots committed to that cell+kind.
    counts: HashMap<(IVec2, IntentKind), u32>,
}

impl SoftWorkSlots {
    /// Build a new empty slot tracker.
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of nanobots currently committed to `cell` for `kind`.
    /// Returns 0 when the cell+kind has no entry yet.
    pub fn occupied(&self, cell: IVec2, kind: IntentKind) -> u32 {
        self.counts.get(&(cell, kind)).copied().unwrap_or(0)
    }

    /// Increment the slot count for `cell` + `kind` by 1. Returns the
    /// new count.
    pub fn occupy(&mut self, cell: IVec2, kind: IntentKind) -> u32 {
        let entry = self.counts.entry((cell, kind)).or_insert(0);
        *entry += 1;
        *entry
    }

    /// Decrement the slot count for `cell` + `kind` by 1, floored at
    /// zero. Returns the new count. Removing from an empty cell+kind
    /// is a no-op (it will not underflow).
    pub fn release(&mut self, cell: IVec2, kind: IntentKind) -> u32 {
        let entry = self.counts.entry((cell, kind)).or_insert(0);
        if *entry > 0 {
            *entry -= 1;
        }
        *entry
    }

    /// Total number of (cell, kind) pairs currently tracked. Useful
    /// for tests and debug overlays.
    pub fn len(&self) -> usize {
        self.counts.len()
    }

    /// True when no slots are tracked.
    pub fn is_empty(&self) -> bool {
        self.counts.is_empty()
    }

    /// Crowding multiplier in `(0, 1]`. Each additional nanobot reduces
    /// attractiveness without hard-rejecting crowded work.
    pub fn crowding_factor(slot_count: u32) -> f32 {
        1.0 / (1.0 + slot_count as f32)
    }
}

/// One scored intent candidate for a nanobot.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IntentCandidate {
    pub cell: IVec2,
    pub kind: IntentKind,
    pub score: f32,
    pub need: f32,
    pub distance: f32,
    pub slot_count: u32,
}

/// Per-cell useful-work weight.
fn need_from_cell(cell: &IntentCell, kind: IntentKind) -> f32 {
    if cell.has(kind) { 1.0 } else { 0.0 }
}

/// Score one `(nanobot, candidate cell, intent kind)` triple against
/// the dumb-autonomy factors. All inputs are plain Rust data so the
/// function is fully testable without Bevy.
///
/// Higher score = the nanobot is more likely to pick this cell+kind.
/// The function does not return a `None`; callers iterate every active
/// intent cell and pick the maximum, which matches the "global intent
/// awareness" contract.
#[allow(clippy::too_many_arguments)]
pub fn score_intent(
    nanobot_type: NanobotType,
    commitment: Commitment,
    nanobot_pos: Vec2,
    candidate_cell: IVec2,
    candidate_kind: IntentKind,
    need: f32,
    slot_count: u32,
    cell_size: f32,
) -> f32 {
    let type_fit = nanobot_type.fit_for(candidate_kind);
    let candidate_pos = Vec2::new(
        (candidate_cell.x as f32 + 0.5) * cell_size,
        (candidate_cell.y as f32 + 0.5) * cell_size,
    );
    let raw_distance = nanobot_pos.distance(candidate_pos);
    let distance_penalty = 1.0 / (1.0 + raw_distance / cell_size.max(1.0));
    let crowding = SoftWorkSlots::crowding_factor(slot_count);
    let reassess = commitment.reassess_factor();

    type_fit * need.max(0.0) * distance_penalty * crowding * reassess
}

/// Score every painted intent cell for one nanobot and return the best
/// candidate. Cells with no active layer for any kind in `kinds` are
/// skipped; the function does not return a "no work" sentinel -- the
/// caller decides what to do when the result is `None`.
///
/// `nanobot_pos` is the world position of the nanobot; `cell_size` is
/// the size of one intent grid cell in world units. The slot count is
/// looked up from `slots` per (cell, kind) pair, so callers can model
/// current swarm pressure without recomputing it.
///
/// `nanobot_swarm` is the [`SwarmId`] the calling nanobot belongs
/// to. Cells whose owner is a different swarm are skipped: the
/// per-swarm intent ownership contract from issue #20. Cells
/// with `owner == None` (legacy shared paint, or paint written
/// through the unowned API) are visible to every swarm.
#[allow(clippy::too_many_arguments)]
pub fn best_candidate(
    grid: &IntentGrid,
    nanobot_type: NanobotType,
    commitment: Commitment,
    nanobot_pos: Vec2,
    slots: &SoftWorkSlots,
    cell_size: f32,
    kinds: &[IntentKind],
    nanobot_swarm: SwarmId,
) -> Option<IntentCandidate> {
    let mut best: Option<IntentCandidate> = None;

    for (cell, intent_cell) in grid.iter_active_cells() {
        if intent_cell.is_empty() {
            continue;
        }
        for &kind in kinds {
            if !intent_cell.has(kind) {
                continue;
            }
            if !intent_cell.visible_to(kind, nanobot_swarm) {
                continue;
            }
            let need = need_from_cell(intent_cell, kind);
            let slot_count = slots.occupied(cell, kind);
            let score = score_intent(
                nanobot_type,
                commitment,
                nanobot_pos,
                cell,
                kind,
                need,
                slot_count,
                cell_size,
            );
            if best.is_none_or(|c| score > c.score) {
                let candidate_pos = Vec2::new(
                    (cell.x as f32 + 0.5) * cell_size,
                    (cell.y as f32 + 0.5) * cell_size,
                );
                best = Some(IntentCandidate {
                    cell,
                    kind,
                    score,
                    need,
                    distance: nanobot_pos.distance(candidate_pos),
                    slot_count,
                });
            }
        }
    }

    best
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intent::IntentGrid;
    use bevy::prelude::IVec2;

    fn cell_size() -> f32 {
        1.0
    }

    #[test]
    fn nanobot_type_default_is_worker() {
        // The glossary is explicit: the player does not assign
        // individual nanobots to types manually, so the default type
        // is just a placeholder for the bundle default.
        assert_eq!(NanobotType::default(), NanobotType::Worker);
    }

    #[test]
    fn nanobot_type_all_covers_worker_hauler_defender() {
        let kinds: Vec<NanobotType> = NanobotType::ALL.to_vec();
        assert_eq!(kinds.len(), NanobotType::COUNT);
        assert!(kinds.contains(&NanobotType::Worker));
        assert!(kinds.contains(&NanobotType::Hauler));
        assert!(kinds.contains(&NanobotType::Defender));
    }

    #[test]
    fn worker_fits_gather_and_build_only() {
        let worker = NanobotType::Worker;
        assert!(worker.fit_for(IntentKind::Gather) > 0.0);
        assert!(worker.fit_for(IntentKind::Build) > 0.0);
        assert_eq!(worker.fit_for(IntentKind::Defend), 0.0);
        // Corridor is hauler path guidance, not work -- no type fits.
        assert_eq!(worker.fit_for(IntentKind::Corridor), 0.0);
    }

    #[test]
    fn hauler_fits_corridor_and_build_not_defend() {
        let hauler = NanobotType::Hauler;
        // Corridor is the hauler's main guidance layer.
        assert!(hauler.fit_for(IntentKind::Corridor) > hauler.fit_for(IntentKind::Build));
        assert!(hauler.fit_for(IntentKind::Build) > 0.0);
        assert_eq!(hauler.fit_for(IntentKind::Gather), 0.0);
        assert_eq!(hauler.fit_for(IntentKind::Defend), 0.0);
    }

    #[test]
    fn defender_fits_defend_only() {
        let defender = NanobotType::Defender;
        assert!(defender.fit_for(IntentKind::Defend) > 0.0);
        assert_eq!(defender.fit_for(IntentKind::Gather), 0.0);
        assert_eq!(defender.fit_for(IntentKind::Build), 0.0);
        assert_eq!(defender.fit_for(IntentKind::Corridor), 0.0);
    }

    #[test]
    fn commitment_default_is_idle() {
        assert_eq!(Commitment::default(), Commitment::Idle);
    }

    #[test]
    fn idle_responds_fully_to_new_intent() {
        assert_eq!(Commitment::Idle.reassess_factor(), 1.0);
    }

    #[test]
    fn carrying_nanobot_finishes_delivery_first() {
        // The contract is "usually finish delivery before
        // reassessing". A small factor means the carrying nanobot
        // will rarely pick a new candidate over finishing its
        // current delivery.
        let factor = Commitment::Carrying.reassess_factor();
        assert!(factor < Commitment::Working.reassess_factor());
        assert!(factor < Commitment::Idle.reassess_factor());
    }

    #[test]
    fn working_nanobot_finishes_short_chunk_first() {
        // The contract is "active workers finish short work chunks
        // before reassessing". Working sits between idle (reacts
        // immediately) and carrying (delivery lock-in).
        let working = Commitment::Working.reassess_factor();
        assert!(working < Commitment::Idle.reassess_factor());
        assert!(working > Commitment::Carrying.reassess_factor());
    }

    #[test]
    fn soft_work_slots_occupy_and_release() {
        let mut slots = SoftWorkSlots::new();
        let cell = IVec2::new(0, 0);
        assert_eq!(slots.occupied(cell, IntentKind::Gather), 0);

        assert_eq!(slots.occupy(cell, IntentKind::Gather), 1);
        assert_eq!(slots.occupied(cell, IntentKind::Gather), 1);
        assert_eq!(slots.occupy(cell, IntentKind::Gather), 2);
        assert_eq!(slots.occupied(cell, IntentKind::Gather), 2);

        // Different kind on the same cell is independent.
        assert_eq!(slots.occupied(cell, IntentKind::Build), 0);
        assert_eq!(slots.occupy(cell, IntentKind::Build), 1);
        assert_eq!(slots.occupied(cell, IntentKind::Gather), 2);

        assert_eq!(slots.release(cell, IntentKind::Gather), 1);
        assert_eq!(slots.occupied(cell, IntentKind::Gather), 1);
    }

    #[test]
    fn soft_work_slots_release_floors_at_zero() {
        let mut slots = SoftWorkSlots::new();
        let cell = IVec2::new(1, 1);
        // Releasing a cell+kind with no entry must not underflow.
        assert_eq!(slots.release(cell, IntentKind::Gather), 0);
        assert_eq!(slots.release(cell, IntentKind::Gather), 0);
    }

    #[test]
    fn crowding_factor_softens_but_never_rejects() {
        // Zero nanobots: no penalty, full attractiveness.
        assert!((SoftWorkSlots::crowding_factor(0) - 1.0).abs() < 1e-6);
        // First extra nanobot halves the factor.
        assert!((SoftWorkSlots::crowding_factor(1) - 0.5).abs() < 1e-6);
        // Crowding must stay strictly positive and strictly
        // decreasing across the slot range that tests ever use.
        let mut prev = SoftWorkSlots::crowding_factor(0);
        for n in 1..=16u32 {
            let next = SoftWorkSlots::crowding_factor(n);
            assert!(next > 0.0, "soft slots must never hard-reject at n={n}");
            assert!(next < prev, "crowding must be strictly decreasing at n={n}");
            prev = next;
        }
    }

    fn score(
        ty: NanobotType,
        commitment: Commitment,
        pos: Vec2,
        cell: IVec2,
        kind: IntentKind,
        need: f32,
        slot_count: u32,
    ) -> f32 {
        score_intent(
            ty,
            commitment,
            pos,
            cell,
            kind,
            need,
            slot_count,
            cell_size(),
        )
    }

    #[test]
    fn score_uses_fit_need_distance_crowding_and_commitment() {
        let base = score(
            NanobotType::Worker,
            Commitment::Idle,
            Vec2::ZERO,
            IVec2::ZERO,
            IntentKind::Gather,
            1.0,
            0,
        );
        assert!(base > 0.0);
        assert_eq!(
            score(
                NanobotType::Defender,
                Commitment::Idle,
                Vec2::ZERO,
                IVec2::ZERO,
                IntentKind::Gather,
                1.0,
                0,
            ),
            0.0
        );
        assert_eq!(
            score(
                NanobotType::Worker,
                Commitment::Idle,
                Vec2::ZERO,
                IVec2::ZERO,
                IntentKind::Gather,
                0.0,
                0,
            ),
            0.0
        );
        let crowded = score(
            NanobotType::Worker,
            Commitment::Idle,
            Vec2::ZERO,
            IVec2::ZERO,
            IntentKind::Gather,
            1.0,
            3,
        );
        let carrying = score(
            NanobotType::Worker,
            Commitment::Carrying,
            Vec2::ZERO,
            IVec2::ZERO,
            IntentKind::Gather,
            1.0,
            0,
        );
        assert!(crowded < base);
        assert!(carrying < base);
    }

    #[test]
    fn global_awareness_picks_closer_binary_paint() {
        let mut grid = IntentGrid::new(4, 4);
        let near = IVec2::ZERO;
        let far = IVec2::ONE;
        grid.paint(near, IntentKind::Gather);
        grid.paint(far, IntentKind::Gather);

        let picked = best_candidate(
            &grid,
            NanobotType::Worker,
            Commitment::Idle,
            Vec2::ZERO,
            &SoftWorkSlots::new(),
            cell_size(),
            &IntentKind::ALL,
            SwarmId::PLAYER,
        )
        .unwrap();
        assert_eq!(picked.cell, near);
    }

    #[test]
    fn global_awareness_applies_soft_slot_pressure() {
        let mut grid = IntentGrid::new(4, 4);
        let crowded = IVec2::new(-1, 0);
        let empty = IVec2::new(1, 0);
        grid.paint(crowded, IntentKind::Gather);
        grid.paint(empty, IntentKind::Gather);
        let mut slots = SoftWorkSlots::new();
        for _ in 0..4 {
            slots.occupy(crowded, IntentKind::Gather);
        }

        let picked = best_candidate(
            &grid,
            NanobotType::Worker,
            Commitment::Idle,
            Vec2::ZERO,
            &slots,
            cell_size(),
            &IntentKind::ALL,
            SwarmId::PLAYER,
        )
        .unwrap();
        assert_eq!(picked.cell, empty);
    }

    #[test]
    fn global_awareness_filters_owner_and_type() {
        let mut grid = IntentGrid::new(4, 4);
        grid.paint_owned(IVec2::new(-1, 0), IntentKind::Defend, Some(SwarmId(9)));
        grid.paint_owned(IVec2::new(1, 0), IntentKind::Defend, Some(SwarmId::PLAYER));

        let picked = best_candidate(
            &grid,
            NanobotType::Defender,
            Commitment::Idle,
            Vec2::ZERO,
            &SoftWorkSlots::new(),
            cell_size(),
            &IntentKind::ALL,
            SwarmId::PLAYER,
        )
        .unwrap();
        assert_eq!(picked.cell, IVec2::new(1, 0));
    }

    #[test]
    fn global_awareness_returns_none_for_empty_grid() {
        assert!(
            best_candidate(
                &IntentGrid::new(4, 4),
                NanobotType::Worker,
                Commitment::Idle,
                Vec2::ZERO,
                &SoftWorkSlots::new(),
                cell_size(),
                &IntentKind::ALL,
                SwarmId::PLAYER,
            )
            .is_none()
        );
    }
}
