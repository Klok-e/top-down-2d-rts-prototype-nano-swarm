//! Dumb autonomy for nanobots.
//!
//! Per the project glossary a nanobot is "aware of player-painted intent
//! globally, but executes it through simple scoring rather than optimal
//! assignment". The scoring lives in this module as pure functions over
//! plain Rust data so the simulation contract can be tested without Bevy
//! rendering or scheduler setup.
//!
//! Six factors feed the score, matching the issue's acceptance criteria:
//! type fit, paint strength, need, distance, commitment, and soft work
//! slot pressure. The score is a plain `f32`; callers pick the
//! best-scoring candidate across all painted cells (the "global intent
//! awareness" contract).
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

/// Specialization of a nanobot. The player does not assign individual
/// nanobots to types manually (see the project glossary); types emerge
/// from the production ratio and are stored on the entity.
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

    /// Crowding multiplier applied by [`score_intent`]. Values in
    /// `(0, 1]`. 1 occupied nanobot is no penalty (no crowding yet);
    /// each additional nanobot above 1 reduces the multiplier. The
    /// soft work slot is "soft" because the multiplier never reaches
    /// zero -- an overcrowded work site is *less attractive*, not
    /// forbidden.
    ///
    /// `slot_count` is the number of nanobots already committed to
    /// this cell+kind, *not* including the one being scored.
    pub fn crowding_factor(slot_count: u32) -> f32 {
        // 1/(1 + n) is the textbook soft penalty: never zero, always
        // decreasing. Multiplied with the rest of the score it lets
        // very strong paint strength still pull an extra nanobot into
        // an overcrowded site, but only when nothing else is better.
        1.0 / (1.0 + slot_count as f32)
    }
}

/// One scored intent candidate for a nanobot. The candidate records the
/// scoring factors so callers can debug "why did this nanobot pick this
/// cell?" without recomputing the score.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IntentCandidate {
    pub cell: IVec2,
    pub kind: IntentKind,
    pub score: f32,
    pub paint_strength: u8,
    pub need: f32,
    pub distance: f32,
    pub slot_count: u32,
}

/// Per-cell weight representing how much useful work the cell currently
/// has. Modelled as a separate factor from paint strength and the slot
/// count so tests can isolate the three. The production system is
/// expected to feed this in; for the first implementation a default of
/// `1.0` per active intent layer is fine.
fn need_from_cell(cell: &IntentCell, kind: IntentKind) -> f32 {
    if cell.has(kind) {
        1.0
    } else {
        0.0
    }
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
    paint_strength: u8,
    need: f32,
    slot_count: u32,
    cell_size: f32,
) -> f32 {
    // Type fit is the multiplicative base: a worker scoring 0 for
    // Defend intent can never beat a defender scoring 1 for the same
    // cell regardless of paint strength.
    let type_fit = nanobot_type.fit_for(candidate_kind);

    // Paint strength is a positive linear term. Normalize by the cap so
    // the multiplier stays in [0, 1] and "two bots painting the same
    // cell" does not get an outsized effect on relative scores.
    let paint_norm = paint_strength as f32 / 16.0;

    // Distance penalty: closer is better. Scale by cell size so the
    // factor behaves the same regardless of how the intent grid is
    // quantized. Use a smooth falloff so very close cells are clearly
    // preferred but nothing is "instant retargeting".
    let candidate_pos = Vec2::new(
        (candidate_cell.x as f32 + 0.5) * cell_size,
        (candidate_cell.y as f32 + 0.5) * cell_size,
    );
    let raw_distance = nanobot_pos.distance(candidate_pos);
    let distance_penalty = 1.0 / (1.0 + raw_distance / cell_size.max(1.0));

    // Soft work slot pressure. Extra nanobots above the first reduce
    // usefulness but never fully reject the cell.
    let crowding = SoftWorkSlots::crowding_factor(slot_count);

    // Commitment gates reassessment symmetrically: see the module
    // docstring for the simplification.
    let reassess = commitment.reassess_factor();

    type_fit * paint_norm * need.max(0.0) * distance_penalty * crowding * reassess
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
pub fn best_candidate(
    grid: &IntentGrid,
    nanobot_type: NanobotType,
    commitment: Commitment,
    nanobot_pos: Vec2,
    slots: &SoftWorkSlots,
    cell_size: f32,
    kinds: &[IntentKind],
) -> Option<IntentCandidate> {
    let mut best: Option<IntentCandidate> = None;

    for (cell, intent_cell) in grid.iter_cells() {
        if intent_cell.is_empty() {
            continue;
        }
        for &kind in kinds {
            if !intent_cell.has(kind) {
                continue;
            }
            let paint_strength = intent_cell.strength(kind);
            let need = need_from_cell(intent_cell, kind);
            let slot_count = slots.occupied(cell, kind);
            let score = score_intent(
                nanobot_type,
                commitment,
                nanobot_pos,
                cell,
                kind,
                paint_strength,
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
                    paint_strength,
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

    #[allow(clippy::too_many_arguments)]
    fn score(
        ty: NanobotType,
        commitment: Commitment,
        pos: Vec2,
        cell: IVec2,
        kind: IntentKind,
        paint_strength: u8,
        need: f32,
        slot_count: u32,
    ) -> f32 {
        score_intent(
            ty,
            commitment,
            pos,
            cell,
            kind,
            paint_strength,
            need,
            slot_count,
            cell_size(),
        )
    }

    #[test]
    fn score_uses_type_fit_as_multiplicative_base() {
        // Worker fits Gather, Defender does not. Holding everything
        // else equal, the defender's score must be 0.
        let worker = score(
            NanobotType::Worker,
            Commitment::Idle,
            Vec2::new(0.0, 0.0),
            IVec2::new(0, 0),
            IntentKind::Gather,
            8,
            1.0,
            0,
        );
        let defender = score(
            NanobotType::Defender,
            Commitment::Idle,
            Vec2::new(0.0, 0.0),
            IVec2::new(0, 0),
            IntentKind::Gather,
            8,
            1.0,
            0,
        );
        assert!(worker > 0.0);
        assert_eq!(defender, 0.0, "defender must not fit gather");
    }

    #[test]
    fn score_grows_monotonically_with_paint_strength() {
        // Three points are enough to pin monotonicity; the formula
        // is linear in paint strength so the rest follows.
        let pos = Vec2::new(0.0, 0.0);
        let cell = IVec2::new(0, 0);
        let low = score(
            NanobotType::Worker,
            Commitment::Idle,
            pos,
            cell,
            IntentKind::Gather,
            2,
            1.0,
            0,
        );
        let mid = score(
            NanobotType::Worker,
            Commitment::Idle,
            pos,
            cell,
            IntentKind::Gather,
            8,
            1.0,
            0,
        );
        let high = score(
            NanobotType::Worker,
            Commitment::Idle,
            pos,
            cell,
            IntentKind::Gather,
            16,
            1.0,
            0,
        );
        assert!(low > 0.0);
        assert!(mid > low);
        assert!(high > mid);
    }

    #[test]
    fn score_is_zero_when_need_is_zero() {
        // Even a perfectly painted, perfectly close, perfectly fit
        // candidate is not picked when there is no useful work.
        assert_eq!(
            score(
                NanobotType::Worker,
                Commitment::Idle,
                Vec2::new(0.0, 0.0),
                IVec2::new(0, 0),
                IntentKind::Gather,
                16,
                0.0,
                0,
            ),
            0.0
        );
    }

    #[test]
    fn score_falls_off_with_distance() {
        // Same cell type, same paint strength, same need. Closer is
        // better. The penalty is smooth so any extra distance reduces
        // the score, but never to zero.
        //
        // We step the nanobot along the positive x-axis with a
        // candidate cell at (-5, 0). The candidate world position is
        // (-4.5, 0.5), so as `d` grows the world distance grows
        // monotonically (no U-shaped curve from passing the cell).
        let cell = IVec2::new(-5, 0);
        let mut prev = f32::INFINITY;
        for d in [0, 4, 12] {
            let pos = Vec2::new(d as f32, 0.0);
            let v = score(
                NanobotType::Worker,
                Commitment::Idle,
                pos,
                cell,
                IntentKind::Gather,
                8,
                1.0,
                0,
            );
            assert!(
                v < prev,
                "score must strictly decrease with distance at d={d}"
            );
            assert!(v > 0.0, "score must remain positive at d={d}");
            prev = v;
        }
    }

    #[test]
    fn score_uses_need_as_multiplicative_factor() {
        // Two candidates at the same cell, same paint, same distance,
        // same type. Doubling the need doubles the score.
        let pos = Vec2::new(0.0, 0.0);
        let cell = IVec2::new(0, 0);
        let low = score(
            NanobotType::Worker,
            Commitment::Idle,
            pos,
            cell,
            IntentKind::Gather,
            8,
            0.5,
            0,
        );
        let high = score(
            NanobotType::Worker,
            Commitment::Idle,
            pos,
            cell,
            IntentKind::Gather,
            8,
            1.0,
            0,
        );
        assert!(high > low);
        assert!(
            (high - 2.0 * low).abs() < 1e-5,
            "need must be linear in the score, got low={low} high={high}"
        );
    }

    #[test]
    fn score_uses_soft_slot_pressure_to_reduce_usefulness() {
        // More nanobots committed -> lower score, but never zero.
        let pos = Vec2::new(0.0, 0.0);
        let cell = IVec2::new(0, 0);
        let mut prev = f32::INFINITY;
        for n in 0..=4u32 {
            let v = score(
                NanobotType::Worker,
                Commitment::Idle,
                pos,
                cell,
                IntentKind::Gather,
                8,
                1.0,
                n,
            );
            assert!(v > 0.0, "soft slots must not hard-reject at n={n}");
            assert!(v < prev, "soft slots must strictly reduce score at n={n}");
            prev = v;
        }
    }

    #[test]
    fn commitment_reduces_score_even_for_high_paint() {
        // A carrying nanobot at a strongly painted cell still scores
        // much lower than an idle nanobot at the same cell, because
        // the carrying factor (0.1) is small. This is the
        // "usually finish delivery before reassessing" contract.
        let pos = Vec2::new(0.0, 0.0);
        let cell = IVec2::new(0, 0);
        let idle = score(
            NanobotType::Worker,
            Commitment::Idle,
            pos,
            cell,
            IntentKind::Gather,
            16,
            1.0,
            0,
        );
        let working = score(
            NanobotType::Worker,
            Commitment::Working,
            pos,
            cell,
            IntentKind::Gather,
            16,
            1.0,
            0,
        );
        let carrying = score(
            NanobotType::Worker,
            Commitment::Carrying,
            pos,
            cell,
            IntentKind::Gather,
            16,
            1.0,
            0,
        );
        assert!(idle > working);
        assert!(working > carrying);
    }

    #[test]
    fn global_awareness_picks_closer_painted_cell() {
        // Two cells with the same paint strength: the closer one
        // wins. A 4x4 grid spans [-2, 2) on both axes, so cells stay
        // in bounds.
        let mut grid = IntentGrid::new(4, 4);
        let near_cell = IVec2::new(0, 0);
        let far_cell = IVec2::new(1, 1);
        assert!(grid.paint(near_cell, IntentKind::Gather, 16));
        assert!(grid.paint(far_cell, IntentKind::Gather, 16));

        let slots = SoftWorkSlots::new();
        let picked = best_candidate(
            &grid,
            NanobotType::Worker,
            Commitment::Idle,
            Vec2::new(0.0, 0.0),
            &slots,
            cell_size(),
            &IntentKind::ALL,
        )
        .expect("must find a candidate");

        assert_eq!(picked.cell, near_cell);
        assert_eq!(picked.kind, IntentKind::Gather);
    }

    #[test]
    fn global_awareness_uses_paint_strength() {
        // Two painted cells, same distance from the bot. The strongly
        // painted cell must win because need is the same (both have a
        // Gather layer) and the slot count is zero for both.
        let mut grid = IntentGrid::new(4, 4);
        let weak_cell = IVec2::new(-1, 0);
        let strong_cell = IVec2::new(1, 0);
        assert!(grid.paint(weak_cell, IntentKind::Gather, 2));
        assert!(grid.paint(strong_cell, IntentKind::Gather, 14));

        let slots = SoftWorkSlots::new();
        let pos = Vec2::new(0.0, 0.0);
        let picked = best_candidate(
            &grid,
            NanobotType::Worker,
            Commitment::Idle,
            pos,
            &slots,
            cell_size(),
            &IntentKind::ALL,
        )
        .expect("must find a candidate");
        assert_eq!(picked.cell, strong_cell);
    }

    #[test]
    fn global_awareness_filters_by_type_fit() {
        // A defender looking at a single Gather cell should find no
        // candidate because type fit is zero. Painting a Defend cell
        // nearby gives the defender a candidate.
        let mut grid = IntentGrid::new(4, 4);
        let gather_cell = IVec2::new(-1, 0);
        let defend_cell = IVec2::new(1, 0);
        assert!(grid.paint(gather_cell, IntentKind::Gather, 16));
        assert!(grid.paint(defend_cell, IntentKind::Defend, 16));

        let slots = SoftWorkSlots::new();
        let pos = Vec2::new(0.0, 0.0);
        let picked = best_candidate(
            &grid,
            NanobotType::Defender,
            Commitment::Idle,
            pos,
            &slots,
            cell_size(),
            &IntentKind::ALL,
        )
        .expect("defender must find a defend candidate");
        assert_eq!(picked.cell, defend_cell);
        assert_eq!(picked.kind, IntentKind::Defend);
    }

    #[test]
    fn global_awareness_applies_soft_slot_pressure() {
        // Two identical candidates; the crowded one must lose.
        let mut grid = IntentGrid::new(4, 4);
        let a = IVec2::new(-1, 0);
        let b = IVec2::new(1, 0);
        assert!(grid.paint(a, IntentKind::Gather, 8));
        assert!(grid.paint(b, IntentKind::Gather, 8));

        let mut slots = SoftWorkSlots::new();
        // Pile 4 nanobots on cell `a` for Gather; cell `b` is empty.
        for _ in 0..4 {
            slots.occupy(a, IntentKind::Gather);
        }
        let pos = Vec2::new(0.0, 0.0);
        let picked = best_candidate(
            &grid,
            NanobotType::Worker,
            Commitment::Idle,
            pos,
            &slots,
            cell_size(),
            &IntentKind::ALL,
        )
        .expect("must find a candidate");
        assert_eq!(
            picked.cell, b,
            "crowded cell must lose to empty cell even with equal paint"
        );
    }

    #[test]
    fn global_awareness_returns_none_for_empty_grid() {
        let grid = IntentGrid::new(4, 4);
        let slots = SoftWorkSlots::new();
        assert!(best_candidate(
            &grid,
            NanobotType::Worker,
            Commitment::Idle,
            Vec2::new(0.0, 0.0),
            &slots,
            cell_size(),
            &IntentKind::ALL,
        )
        .is_none());
    }
}
