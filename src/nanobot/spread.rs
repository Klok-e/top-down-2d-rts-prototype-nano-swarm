//! Idle cosmetic spread (issue #39).
//!
//! Idle nanobots (`Commitment::Idle`) standing inside a type-fit intent
//! zone redistribute across that zone so the swarm looks alive instead
//! of stacking. The nudge is purely cosmetic: it writes only to
//! [`VelocityComponent`], never inserts a [`DirectMovementComponent`],
//! never changes [`Commitment`], and never removes the bot from the
//! idle pool. An idle bot stays fully grabbable by the central demand
//! allocator at every tick -- this honours the glossary contract
//! "idle nanobots respond immediately".
//!
//! ## Spreading model (locked in during design grilling)
//!
//! 1. **Region = connected type-fit paint.** A bot's spread region is
//!    the set of cells reachable by stepping onto 8-neighbour (king-
//!    move) cells that carry at least one [`IntentKind`] the bot's
//!    [`NanobotType::fit_for`] scores `1.0`. Region membership is
//!    decided per-step by checking the neighbour cell's paint; no
//!    flood-fill is computed. A Worker treats Gather and Build cells
//!    as one region; a Hauler spreads over Corridor only; a Defender
//!    over Defend only.
//! 2. **Stranded bots seek nearest fit-paint.** An idle bot whose
//!    current cell has none of its type-fit paint drifts toward the
//!    nearest fit-paint cell instead of doing a gradient step. If no
//!    fit-paint cell exists anywhere on the grid, the bot gets no
//!    nudge.
//! 3. **In-region gradient.** A bot already on a fit-paint cell looks
//!    at its 8 king-move neighbours that are fit-painted, finds the
//!    count of *all* bots in each neighbour cell, and nudges toward a
//!    strictly-less-dense one than its own cell. Density counts every
//!    bot regardless of type, commitment, or kind; the scoring bot's
//!    own body is excluded from its own cell's count so a lone bot in
//!    a big empty region reads density `0` and falls through to the
//!    tie-break (matching "bot alone in a big empty region").
//! 4. **Tie-break = random exploration.** When no neighbour is
//!    strictly less dense, the bot nudges toward a random fit-painted
//!    neighbour. When there is no fit-painted neighbour, no nudge.
//! 5. **Velocity only.** The spread is a per-tick nudge applied to
//!    [`VelocityComponent`]. It runs before `velocity_system` so the
//!    force composes with `separation_system` and is consumed the same
//!    frame.
//!
//! The decision helpers are pure over plain Rust data so the contract
//! can be unit-tested without Bevy. The system wires them up and
//! rebuilds the per-type fit-cell list + density map each tick (per
//! the issue's "rebuild both maps every tick" scope; cross-tick
//! caching is explicitly out of scope).

use std::cmp::Ordering;
use std::collections::HashMap;

use bevy::prelude::*;
use rand::RngExt;

use crate::ai::get_world_from_zone;
use crate::intent::{IntentCell, IntentGrid, IntentKind};
use crate::nanobot::autonomy::{Commitment, NanobotType};
use crate::nanobot::components::{DirectMovementComponent, Nanobot, VelocityComponent};
use crate::nanobot::gather::world_to_cell;

/// Per-tick velocity nudge magnitude applied to idle bots spreading
/// across their type-fit region. Sized near [`BOT_SEPARATION_FORCE`]
/// (the existing pre-`velocity_system` physical de-clumper) so spread
/// and separation compose at the same scale: separation pushes
/// overlapping bots apart, spread fills free region. Gentle enough
/// that a bot crosses roughly one cell over a few hundred ticks, so a
/// freshly painted zone visibly fills without ever outrunning the
/// demand allocator's grab of the same idle bot.
pub const BOT_SPREAD_FORCE: f32 = 1.5;

/// Intent kinds a `ntype` treats as in-region: those whose
/// [`NanobotType::fit_for`] scores exactly `1.0`. The `0.5`
/// Hauler/Build partial-fit score is excluded -- spread uses
/// `== 1.0` only, matching the locked-in model. Derived from
/// `fit_for` so the spread region tracks any future fit-table
/// change automatically.
pub fn fit_kinds(ntype: NanobotType) -> Vec<IntentKind> {
    IntentKind::ALL
        .into_iter()
        .filter(|k| ntype.fit_for(*k) == 1.0)
        .collect()
}

/// True when `cell` carries at least one type-fit paint layer for
/// `ntype`. Region membership is per-cell paint, not flood-fill: a
/// cell is in-region iff it has at least one kind scoring `1.0`.
pub fn cell_is_fit_for(ntype: NanobotType, cell: &IntentCell) -> bool {
    fit_kinds(ntype).iter().any(|k| cell.has(*k))
}

/// The 8 king-move (Chebyshev-1) neighbours of `cell`, in a fixed
/// clockwise-from-bottom-left order. The order is stable so the
/// spread system's neighbour scan and the unit tests agree on which
/// neighbour is "first".
pub fn king_neighbours(cell: IVec2) -> [IVec2; 8] {
    [
        IVec2::new(cell.x - 1, cell.y - 1),
        IVec2::new(cell.x, cell.y - 1),
        IVec2::new(cell.x + 1, cell.y - 1),
        IVec2::new(cell.x - 1, cell.y),
        IVec2::new(cell.x + 1, cell.y),
        IVec2::new(cell.x - 1, cell.y + 1),
        IVec2::new(cell.x, cell.y + 1),
        IVec2::new(cell.x + 1, cell.y + 1),
    ]
}

/// In-bounds, type-fit king-move neighbours of `cell`. Out-of-bounds
/// neighbours (those for which the grid has no cell) are dropped, so
/// a bot at the grid edge only considers neighbours that exist.
pub fn fit_neighbour_cells(ntype: NanobotType, cell: IVec2, grid: &IntentGrid) -> Vec<IVec2> {
    king_neighbours(cell)
        .into_iter()
        .filter_map(|n| {
            let neighbour = grid.cell(n)?;
            cell_is_fit_for(ntype, neighbour).then_some(n)
        })
        .collect()
}

/// Choose the gradient-step target cell for an in-region idle bot.
///
/// `own_density_excl_self` is the count of *other* bots in the bot's
/// own cell (the bot's own body is excluded so a lone bot reads `0`
/// and falls through to random exploration rather than always fleeing
/// itself). `fit_neighbours` is `[(cell, density)]` for each in-bounds
/// type-fit king-move neighbour, with `density` counting every bot in
/// that neighbour cell.
///
/// Decision:
/// - If any neighbour is strictly less dense than `own_density_excl_self`,
///   pick the least-dense such neighbour; break ties uniformly at
///   random via `rng`.
/// - Otherwise (settled region, or bot alone in a big empty region)
///   pick a uniformly random fit neighbour via `rng`.
/// - If there is no fit neighbour, return `None` (no nudge).
///
/// The random tie-break is what keeps settled clumps from locking up
/// and matches the glossary's "organic-looking placement may vary"
/// ethos. It is driven entirely by `rng`, so a seeded RNG makes the
/// choice deterministic for tests.
pub fn gradient_step_target<R: RngExt>(
    own_density_excl_self: u32,
    fit_neighbours: &[(IVec2, u32)],
    rng: &mut R,
) -> Option<IVec2> {
    if fit_neighbours.is_empty() {
        return None;
    }
    // Least density among the strictly-less-dense neighbours. `None`
    // means the field is flat (settled region, or a lone bot in a big
    // empty region) -- the bot then explores a random fit neighbour,
    // the tie-break that keeps clumps from locking up.
    let target_density = fit_neighbours
        .iter()
        .map(|(_, density)| *density)
        .filter(|density| *density < own_density_excl_self)
        .min();
    let pool: Vec<IVec2> = match target_density {
        Some(target_density) => fit_neighbours
            .iter()
            .filter(|(_, density)| *density == target_density)
            .map(|(cell, _)| *cell)
            .collect(),
        None => fit_neighbours.iter().map(|(cell, _)| *cell).collect(),
    };
    let pick = rng.random_range(0..pool.len());
    Some(pool[pick])
}

/// Nearest type-fit cell to `from`, by world distance from `from` to
/// each candidate cell's center ([`get_world_from_zone`]). Ties keep
/// the first minimum in iteration order (`min_by` is stable), so the
/// choice is deterministic given the candidate order. Returns `None`
/// when the candidate list is empty -- the caller must leave the bot
/// unmoved.
pub fn nearest_fit_cell(from: Vec2, fit_cells: &[IVec2]) -> Option<IVec2> {
    fit_cells.iter().copied().min_by(|a, b| {
        let da = from.distance(get_world_from_zone(*a));
        let db = from.distance(get_world_from_zone(*b));
        da.partial_cmp(&db).unwrap_or(Ordering::Equal)
    })
}

/// Stable index of `ntype` inside [`NanobotType::ALL`]. Used to
/// address the per-type fit-cell list built each tick.
fn type_index(ntype: NanobotType) -> usize {
    NanobotType::ALL
        .into_iter()
        .position(|t| t == ntype)
        .expect("NanobotType::ALL contains every type")
}

/// Per-tick idle cosmetic spread. Runs before `velocity_system` so the
/// nudge composes with `separation_system` and is consumed the same
/// frame.
///
/// `all_bots` reads every nanobot's transform once to build the
/// density map (every bot counts, regardless of type, commitment, or
/// kind -- "occupied is occupied", matching the existing
/// [`crate::nanobot::SoftWorkSlots`] model). `idle_bots` then nudges
/// only `Commitment::Idle` bots that have no
/// [`DirectMovementComponent`] (carrying / working / moving bots are
/// never nudged). The two queries read `Transform` together (read-
/// read compatible) and only `idle_bots` writes `VelocityComponent`,
/// so they do not conflict.
#[allow(clippy::type_complexity)]
pub fn idle_spread_system(
    grid: Res<IntentGrid>,
    all_bots: Query<&Transform, With<Nanobot>>,
    mut idle_bots: Query<
        (
            &Transform,
            &NanobotType,
            &Commitment,
            &mut VelocityComponent,
        ),
        (With<Nanobot>, Without<DirectMovementComponent>),
    >,
) {
    // Per-type fit-cell list, rebuilt every tick (cross-tick caching
    // is out of scope). Built in a single pass over painted cells so
    // the per-bot loop never re-scans the grid.
    let kind_sets: [Vec<IntentKind>; NanobotType::COUNT] =
        std::array::from_fn(|i| fit_kinds(NanobotType::ALL[i]));
    let mut fit_cells: [Vec<IVec2>; NanobotType::COUNT] = std::array::from_fn(|_| Vec::new());
    for (cell, intent_cell) in grid.iter_cells() {
        if intent_cell.is_empty() {
            continue;
        }
        for (type_idx, kinds) in kind_sets.iter().enumerate() {
            if kinds.iter().any(|k| intent_cell.has(*k)) {
                fit_cells[type_idx].push(cell);
            }
        }
    }

    // Density map: cell -> count of every bot physically standing
    // there this tick. Local to this system per the issue brief so
    // spread stays self-contained (no cross-plugin ordering
    // dependency on the defender density pass).
    let mut density: HashMap<IVec2, u32> = HashMap::new();
    for transform in &all_bots {
        let cell = world_to_cell(transform.translation.truncate());
        *density.entry(cell).or_insert(0) += 1;
    }

    let mut rng = rand::rng();
    for (transform, nanobot_type, commitment, mut velocity) in &mut idle_bots {
        if *commitment != Commitment::Idle {
            continue;
        }
        let pos = transform.translation.truncate();
        let own_cell = world_to_cell(pos);
        let type_idx = type_index(*nanobot_type);

        let target = if grid
            .cell(own_cell)
            .is_some_and(|c| kind_sets[type_idx].iter().any(|k| c.has(*k)))
        {
            // In-region gradient step. Exclude the bot's own body
            // from its own cell's count so a lone bot reads density 0
            // and falls through to random exploration.
            let own_excl = density
                .get(&own_cell)
                .copied()
                .unwrap_or(0)
                .saturating_sub(1);
            let neighbours: Vec<(IVec2, u32)> = king_neighbours(own_cell)
                .into_iter()
                .filter_map(|n| {
                    let neighbour = grid.cell(n)?;
                    let fit = kind_sets[type_idx].iter().any(|k| neighbour.has(*k));
                    fit.then(|| (n, density.get(&n).copied().unwrap_or(0)))
                })
                .collect();
            gradient_step_target(own_excl, &neighbours, &mut rng)
        } else {
            // Stranded: drift toward the nearest type-fit cell.
            nearest_fit_cell(pos, &fit_cells[type_idx])
        };

        if let Some(target_cell) = target {
            let target_center = get_world_from_zone(target_cell);
            let dir = target_center - pos;
            // Near-zero direction (bot already on the target center)
            // yields no nudge rather than a NaN from normalising zero.
            if dir.length() > 1e-3 {
                velocity.value += dir.normalize() * BOT_SPREAD_FORCE;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    //! Pure-helper unit tests for the idle spread decision logic.
    //! End-to-end ECS contracts (only idle non-DMC bots nudged, no
    //! commitment / DMC change, stranded drift) live in
    //! `tests/behavior/idle_spread.rs`.

    use super::*;
    use crate::ai::get_world_from_zone;
    use rand::rngs::StdRng;
    use rand::SeedableRng;

    fn seeded() -> StdRng {
        // Fixed seed so tie-break picks are reproducible.
        StdRng::seed_from_u64(0xCAFE_F00D)
    }

    #[test]
    fn fit_kinds_match_exactly_one_dot_zero_score() {
        // Worker: Gather + Build (both 1.0). The 0.0 Defend/Corridor
        // scores are excluded.
        let worker = fit_kinds(NanobotType::Worker);
        assert!(worker.contains(&IntentKind::Gather));
        assert!(worker.contains(&IntentKind::Build));
        assert!(!worker.contains(&IntentKind::Defend));
        assert!(!worker.contains(&IntentKind::Corridor));

        // Hauler: Corridor only. The 0.5 Build partial-fit is the
        // critical exclusion -- spread uses == 1.0, not > 0.
        let hauler = fit_kinds(NanobotType::Hauler);
        assert_eq!(hauler, vec![IntentKind::Corridor]);

        // Defender: Defend only.
        let defender = fit_kinds(NanobotType::Defender);
        assert_eq!(defender, vec![IntentKind::Defend]);
    }

    #[test]
    fn cell_is_fit_for_merges_worker_gather_and_build() {
        // A cell with Gather only is fit for a Worker; a cell with
        // Build only is also fit for a Worker. The merged worker
        // region is the union of both kinds.
        let mut gather_only = IntentCell::default();
        gather_only.add(IntentKind::Gather);
        assert!(cell_is_fit_for(NanobotType::Worker, &gather_only));
        assert!(!cell_is_fit_for(NanobotType::Hauler, &gather_only));

        let mut build_only = IntentCell::default();
        build_only.add(IntentKind::Build);
        assert!(cell_is_fit_for(NanobotType::Worker, &build_only));
        assert!(!cell_is_fit_for(NanobotType::Defender, &build_only));

        // A Hauler partial-fit Build cell (0.5 score) is NOT fit for
        // a Hauler under the == 1.0 spread rule, and not fit for a
        // Worker either (Worker scores Build 1.0 -- so this cell IS
        // fit for a Worker, proving the union).
        assert!(cell_is_fit_for(NanobotType::Worker, &build_only));

        let mut corridor = IntentCell::default();
        corridor.add(IntentKind::Corridor);
        assert!(cell_is_fit_for(NanobotType::Hauler, &corridor));
        assert!(!cell_is_fit_for(NanobotType::Worker, &corridor));

        // Empty cell fits nobody.
        assert!(!cell_is_fit_for(
            NanobotType::Worker,
            &IntentCell::default()
        ));
    }

    #[test]
    fn king_neighbours_are_the_eight_chebyshev_one_cells() {
        let n = king_neighbours(IVec2::new(0, 0));
        assert_eq!(n.len(), 8);
        // Every neighbour is a king move away and the centre cell is
        // not included.
        assert!(!n.contains(&IVec2::new(0, 0)));
        for nb in n {
            let dx = nb.x.abs();
            let dy = nb.y.abs();
            assert!(dx <= 1 && dy <= 1 && (dx + dy > 0));
        }
        // A non-origin cell shifts the whole ring by the offset.
        let n2 = king_neighbours(IVec2::new(3, -2));
        assert!(n2.contains(&IVec2::new(2, -3)));
        assert!(n2.contains(&IVec2::new(4, -1)));
    }

    #[test]
    fn fit_neighbour_cells_drop_out_of_bounds_and_unpainted() {
        // A 3x3 grid spans cells -1..2 on each axis. Paint Gather at
        // the east neighbour of (0,0) and Build at the north neighbour;
        // a Worker should see both as fit neighbours, while the
        // unpainted / out-of-bounds neighbours are dropped.
        let mut grid = IntentGrid::new(3, 3);
        grid.add(IVec2::new(1, 0), IntentKind::Gather);
        grid.add(IVec2::new(0, 1), IntentKind::Build);

        let fit = fit_neighbour_cells(NanobotType::Worker, IVec2::new(0, 0), &grid);
        assert!(fit.contains(&IVec2::new(1, 0)));
        assert!(fit.contains(&IVec2::new(0, 1)));
        // Exactly the two painted neighbours.
        assert_eq!(fit.len(), 2);

        // A Hauler sees neither -- Corridor-only fit.
        let hauler_fit = fit_neighbour_cells(NanobotType::Hauler, IVec2::new(0, 0), &grid);
        assert!(hauler_fit.is_empty());
    }

    #[test]
    fn gradient_step_returns_none_when_no_fit_neighbour() {
        let mut rng = seeded();
        assert!(gradient_step_target(5, &[], &mut rng).is_none());
    }

    #[test]
    fn gradient_step_picks_strictly_less_dense_neighbour() {
        // Own density 3 (excl self). One neighbour at density 1
        // (strictly less), another at density 5 (not less). The pick
        // must be the less-dense one.
        let mut rng = seeded();
        let neighbours = vec![
            (IVec2::new(1, 0), 5),
            (IVec2::new(-1, 0), 1),
            (IVec2::new(0, 1), 9),
        ];
        let pick = gradient_step_target(3, &neighbours, &mut rng);
        assert_eq!(pick, Some(IVec2::new(-1, 0)));
    }

    #[test]
    fn gradient_step_picks_least_dense_among_strictly_less() {
        // Own density 4. Neighbours at 1, 2, and 6: the strictly-less
        // ones are densities 1 and 2; the least-dense is 1, so the
        // pick is the density-1 cell regardless of seed.
        for seed in 0..16u64 {
            let mut rng = StdRng::seed_from_u64(seed);
            let neighbours = vec![
                (IVec2::new(1, 0), 2),
                (IVec2::new(-1, 0), 6),
                (IVec2::new(0, 1), 1),
            ];
            let pick = gradient_step_target(4, &neighbours, &mut rng);
            assert_eq!(pick, Some(IVec2::new(0, 1)), "seed {seed}");
        }
    }

    #[test]
    fn gradient_step_tie_break_is_deterministic_under_seed() {
        // Two equally least-dense neighbours (both density 0, own 1):
        // the pick is random but deterministic under a fixed seed,
        // and stays one of the two candidates.
        let neighbours = vec![(IVec2::new(1, 0), 0), (IVec2::new(-1, 0), 0)];

        let mut first = seeded();
        let pick_a = gradient_step_target(1, &neighbours, &mut first);

        let mut second = seeded();
        let pick_b = gradient_step_target(1, &neighbours, &mut second);

        assert_eq!(pick_a, pick_b, "same seed must give same pick");
        assert!(neighbours.iter().any(|(c, _)| Some(*c) == pick_a));
    }

    #[test]
    fn gradient_step_flat_field_explores_random_fit_neighbour() {
        // Own density 0 (lone bot, self-excluded). All neighbours also
        // 0: none is strictly less, so the bot explores a random fit
        // neighbour. The pick must be one of the candidates and
        // deterministic under the seed.
        let neighbours = vec![
            (IVec2::new(1, 0), 0),
            (IVec2::new(0, 1), 0),
            (IVec2::new(-1, 0), 0),
        ];

        let mut first = seeded();
        let pick_a = gradient_step_target(0, &neighbours, &mut first);

        let mut second = seeded();
        let pick_b = gradient_step_target(0, &neighbours, &mut second);

        assert_eq!(pick_a, pick_b, "same seed must give same pick");
        assert!(neighbours.iter().any(|(c, _)| Some(*c) == pick_a));
    }

    #[test]
    fn nearest_fit_cell_returns_none_for_empty_candidates() {
        assert!(nearest_fit_cell(Vec2::ZERO, &[]).is_none());
    }

    #[test]
    fn nearest_fit_cell_picks_closest_by_world_distance() {
        // Bot at the center of cell (0,0). Candidate cells (1,0) and
        // (3,0): the nearer center is (1,0).
        let from = get_world_from_zone(IVec2::new(0, 0));
        let candidates = vec![IVec2::new(3, 0), IVec2::new(1, 0)];
        let pick = nearest_fit_cell(from, &candidates);
        assert_eq!(pick, Some(IVec2::new(1, 0)));
    }

    #[test]
    fn nearest_fit_cell_tie_keeps_first_in_iteration_order() {
        // Two equidistant candidates: min_by is stable and keeps the
        // first, so the pick is deterministic. From the center of
        // cell (0,0), the east cell (1,0) and the west cell (-1,0)
        // are both exactly one cell-width away.
        let from = get_world_from_zone(IVec2::new(0, 0));
        let candidates = vec![IVec2::new(1, 0), IVec2::new(-1, 0)];
        let pick = nearest_fit_cell(from, &candidates);
        assert_eq!(pick, Some(IVec2::new(1, 0)));

        // Swapping the order swaps the pick, proving the tie-break is
        // purely iteration order (no hidden randomness).
        let candidates_swapped = vec![IVec2::new(-1, 0), IVec2::new(1, 0)];
        let pick_swapped = nearest_fit_cell(from, &candidates_swapped);
        assert_eq!(pick_swapped, Some(IVec2::new(-1, 0)));
    }
}
