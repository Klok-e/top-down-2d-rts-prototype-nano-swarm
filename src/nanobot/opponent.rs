//! Prepainted opponent swarm initialisation.
//!
//! An "Opponent Swarm" is a non-player swarm that uses the
//! same intent, production, logistics, maintenance, and
//! charge systems as the player swarm. Early opponents use
//! prepainted bases and fixed production ratios; no active
//! AI is required.
//!
//! The [`spawn_opponent_swarm`] helper materialises one
//! opponent: a `Swarm` entity with the [`OpponentSwarm`]
//! marker, a fixed [`SwarmProduction`] ratio, prepainted
//! intent on the shared [`IntentGrid`], and seed nanobots as
//! children. Everything the helper produces is a regular
//! Bevy component the existing systems already understand, so
//! the opponent has no parallel runtime path.

use bevy::prelude::*;

use crate::ai::AiStateComponent;
use crate::intent::{IntentGrid, IntentKind};
use crate::nanobot::autonomy::Commitment;
use crate::nanobot::components::{Health, Nanobot, Swarm, SwarmId, SwarmMember, VelocityComponent};
use crate::nanobot::production::{OpponentSwarm, ProductionRatio, SwarmProduction};
use crate::nanobot::{NanobotBundle, NanobotType};

/// One prepainted intent cell on the shared grid. The
/// opponent helper takes a slice of these at spawn time and
/// writes each layer onto the [`IntentGrid`] in one go, so
/// the opponent starts with its territory already declared.
#[derive(Debug, Clone, Copy)]
pub struct PrepaintedIntent {
    /// Grid cell to paint, in the same coordinate system the
    /// rest of the simulation uses (centered on the origin).
    pub cell: IVec2,
    /// Which intent layer to activate.
    pub kind: IntentKind,
}

impl PrepaintedIntent {
    pub fn new(cell: IVec2, kind: IntentKind) -> Self {
        Self { cell, kind }
    }
}

/// One seed nanobot entry: spawn `count` entities of `kind`
/// as children of the new swarm.
#[derive(Debug, Clone, Copy)]
pub struct SeedNanobots {
    pub kind: NanobotType,
    pub count: u32,
}

impl SeedNanobots {
    pub fn new(kind: NanobotType, count: u32) -> Self {
        Self { kind, count }
    }
}

/// Initialise an opponent swarm: spawn a [`Swarm`] entity
/// carrying the [`OpponentSwarm`] marker and a fixed
/// [`SwarmProduction`] ratio, paint the requested intent onto
/// the shared [`IntentGrid`], and seed the requested
/// nanobots as children. Returns the spawned swarm entity.
///
/// Takes `&mut World` so the helper composes with both Bevy
/// startup systems (which can take `&mut World` directly) and
/// the test harness (where `app.world_mut()` exposes the same
/// handle). The opponent is a swarm first -- the marker and
/// per-swarm ratio are the only things that make it an
/// opponent -- so no special-case runtime path is needed.
pub fn spawn_opponent_swarm(
    world: &mut World,
    world_pos: Vec2,
    ratio: ProductionRatio,
    prepainted: &[PrepaintedIntent],
    seeds: &[SeedNanobots],
) -> Entity {
    let swarm_id = next_opponent_swarm_id(world);
    let swarm = world
        .spawn((
            Swarm {},
            OpponentSwarm {},
            SwarmProduction::new(ratio),
            swarm_id,
            Transform::from_translation(world_pos.extend(0.0)),
            Visibility::default(),
        ))
        .id();

    // Cells outside the grid are silently rejected by the
    // grid itself; the helper does not gate the spawn on a
    // paint success so one out-of-bounds cell does not abort
    // an otherwise valid opponent setup. The paint is stamped
    // with the opponent's `SwarmId` so the per-swarm intent
    // filter routes the prepainted cells to opponent workers
    // only.
    {
        let mut grid = world.resource_mut::<IntentGrid>();
        for paint in prepainted {
            grid.paint_owned(paint.cell, paint.kind, Some(swarm_id));
        }
    }

    // Seed nanobots are top-level entities (issue #38 /
    // ADR-0004) so their `Transform.translation` is the
    // world position the simulation reads. Parented bots
    // would navigate to the world destination + the
    // swarm's `Transform`, which for the default opponent
    // spawn at `(6400, 256)` is a half-cell offset that
    // drove the same "top-right corner" gather bug the
    // player swarm exhibited. The opponent's
    // `SwarmId` is stamped on every seed via
    // `SwarmMember(swarm_id)` so the per-swarm intent
    // filter still routes the prepainted cells to the
    // opponent workers.
    for seed in seeds {
        for _ in 0..seed.count {
            world.spawn((
                NanobotBundle {
                    nanobot: Nanobot {},
                    nanobot_type: seed.kind,
                    velocity: VelocityComponent::default(),
                    ai_state: AiStateComponent::new(),
                    health: Health::default(),
                    swarm_member: SwarmMember::new(swarm_id),
                },
                Commitment::Idle,
                Transform::from_translation(world_pos.extend(0.0)),
            ));
        }
    }

    swarm
}

/// Monotonic counter resource for opponent [`SwarmId`]s. Kept on
/// the [`World`] so multiple opponent spawns in the same process
/// (test app, or a future scenario with several opponents) get
/// distinct ids without colliding with the reserved player id.
#[derive(Debug, Default, Resource)]
pub struct OpponentSwarmIdAlloc {
    next: u32,
}

impl OpponentSwarmIdAlloc {
    /// Take the next opponent [`SwarmId`] and bump the counter.
    /// The player id (`SwarmId::PLAYER`, 0) is skipped so the
    /// returned id is never ambiguous with the player swarm.
    pub fn allocate(&mut self) -> SwarmId {
        // Skip SwarmId::PLAYER (0); opponent ids start at 1.
        let candidate = self.next.max(1);
        self.next = candidate.saturating_add(1);
        SwarmId(candidate)
    }
}

/// Allocate the next opponent [`SwarmId`] from the world's
/// counter, inserting the counter resource on first use. Mirrors
/// the `allocate_opponent_swarm_id` helper in `tests/common` so
/// the same op produces a single id whether the caller is a test
/// or production code.
pub fn next_opponent_swarm_id(world: &mut World) -> SwarmId {
    let mut alloc =
        world.get_resource_or_insert_with::<OpponentSwarmIdAlloc>(OpponentSwarmIdAlloc::default);
    alloc.allocate()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_app() -> App {
        let mut app = App::new();
        app.insert_resource(IntentGrid::new(8, 8));
        app
    }

    #[test]
    fn spawn_opponent_swarm_creates_marker_and_swarm() {
        // The helper must produce an entity carrying both
        // `Swarm` and `OpponentSwarm` so downstream systems
        // can recognise it.
        let mut app = build_app();
        let swarm = spawn_opponent_swarm(
            app.world_mut(),
            Vec2::new(0.0, 0.0),
            ProductionRatio::new(),
            &[],
            &[],
        );
        let world = app.world();
        assert!(world.entity(swarm).get::<Swarm>().is_some());
        assert!(world.entity(swarm).get::<OpponentSwarm>().is_some());
    }

    #[test]
    fn spawn_opponent_swarm_paints_requested_intent() {
        // Every PrepaintedIntent entry must end up on the
        // shared grid.
        let mut app = build_app();
        let gather_cell = IVec2::new(0, 0);
        let defend_cell = IVec2::new(1, 0);
        let _ = spawn_opponent_swarm(
            app.world_mut(),
            Vec2::new(0.0, 0.0),
            ProductionRatio::new(),
            &[
                PrepaintedIntent::new(gather_cell, IntentKind::Gather),
                PrepaintedIntent::new(defend_cell, IntentKind::Defend),
            ],
            &[],
        );
        let grid = app.world().resource::<IntentGrid>();
        let g = grid.cell(gather_cell).unwrap();
        assert!(g.has(IntentKind::Gather));
        assert_eq!(g.owner(IntentKind::Gather), Some(SwarmId(1)));
        let d = grid.cell(defend_cell).unwrap();
        assert!(d.has(IntentKind::Defend));
        assert_eq!(d.owner(IntentKind::Defend), Some(SwarmId(1)));
    }

    #[test]
    fn spawn_opponent_swarm_attaches_production_ratio() {
        // The helper must stamp a `SwarmProduction`
        // component carrying the requested ratio so the
        // production systems route the opponent's
        // facilities through the opponent's mix.
        let mut app = build_app();
        let mut ratio = ProductionRatio::new();
        ratio.set_weight(NanobotType::Hauler, 4);
        let swarm = spawn_opponent_swarm(app.world_mut(), Vec2::new(0.0, 0.0), ratio, &[], &[]);
        let world = app.world();
        let sp = world
            .entity(swarm)
            .get::<SwarmProduction>()
            .expect("SwarmProduction must be attached");
        assert_eq!(sp.ratio.weight(NanobotType::Hauler), 4);
    }

    #[test]
    fn spawn_opponent_swarm_seeds_requested_nanobots() {
        // The helper must spawn the requested nanobots as
        // top-level entities (issue #38 / ADR-0004).
        // They are no longer children of the swarm; the
        // swarm is purely a spawn-origin / ownership
        // marker. The simulation reads the per-bot
        // `Transform` and `SwarmMember` directly, so
        // parentage is no longer part of the contract.
        let mut app = build_app();
        let opponent_pos = Vec2::new(0.0, 0.0);
        let swarm = spawn_opponent_swarm(
            app.world_mut(),
            opponent_pos,
            ProductionRatio::new(),
            &[],
            &[
                SeedNanobots::new(NanobotType::Worker, 3),
                SeedNanobots::new(NanobotType::Hauler, 2),
            ],
        );
        let mut workers = 0;
        let mut haulers = 0;
        {
            // The swarm is no longer a parent; count
            // `Nanobot` entities with `SwarmMember`
            // pointing at the swarm instead.
            let mut entities_query = app
                .world_mut()
                .query::<(Entity, &SwarmMember, &NanobotType)>();
            let world = app.world();
            let swarm_id = world
                .entity(swarm)
                .get::<SwarmId>()
                .copied()
                .unwrap_or(SwarmId::PLAYER);
            for (_entity, member, nanobot_type) in entities_query.iter(world) {
                if member.0 != swarm_id {
                    continue;
                }
                match nanobot_type {
                    NanobotType::Worker => workers += 1,
                    NanobotType::Hauler => haulers += 1,
                    _ => {}
                }
            }
        }
        assert_eq!(workers, 3);
        assert_eq!(haulers, 2);
    }
}
