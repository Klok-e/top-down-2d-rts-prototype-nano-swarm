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
use crate::nanobot::components::{Health, Nanobot, Swarm, VelocityComponent};
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
    /// Paint strength to apply. Values are clamped to the
    /// shared [`crate::intent::PAINT_STRENGTH_CAP`].
    pub strength: u8,
}

impl PrepaintedIntent {
    pub fn new(cell: IVec2, kind: IntentKind, strength: u8) -> Self {
        Self {
            cell,
            kind,
            strength,
        }
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
    let swarm = world
        .spawn((
            Swarm {},
            OpponentSwarm {},
            SwarmProduction::new(ratio),
            Transform::from_translation(world_pos.extend(0.0)),
            Visibility::default(),
        ))
        .id();

    // Cells outside the grid are silently rejected by the
    // grid itself; the helper does not gate the spawn on a
    // paint success so one out-of-bounds cell does not abort
    // an otherwise valid opponent setup.
    {
        let mut grid = world.resource_mut::<IntentGrid>();
        for paint in prepainted {
            grid.paint(paint.cell, paint.kind, paint.strength);
        }
    }

    // Seed nanobots parented to the swarm. The opponent's
    // production systems will top them up as needed.
    world.entity_mut(swarm).with_children(|p| {
        for seed in seeds {
            for _ in 0..seed.count {
                p.spawn((
                    NanobotBundle {
                        nanobot: Nanobot {},
                        nanobot_type: seed.kind,
                        velocity: VelocityComponent::default(),
                        ai_state: AiStateComponent::new(),
                        health: Health::default(),
                    },
                    Commitment::Idle,
                    Transform::from_translation(world_pos.extend(0.0)),
                ));
            }
        }
    });

    swarm
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intent::PAINT_STRENGTH_CAP;

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
                PrepaintedIntent::new(gather_cell, IntentKind::Gather, PAINT_STRENGTH_CAP),
                PrepaintedIntent::new(defend_cell, IntentKind::Defend, 8),
            ],
            &[],
        );
        let grid = app.world().resource::<IntentGrid>();
        let g = grid.cell(gather_cell).unwrap();
        assert!(g.has(IntentKind::Gather));
        assert_eq!(g.strength(IntentKind::Gather), PAINT_STRENGTH_CAP);
        let d = grid.cell(defend_cell).unwrap();
        assert!(d.has(IntentKind::Defend));
        assert_eq!(d.strength(IntentKind::Defend), 8);
    }

    #[test]
    fn spawn_opponent_swarm_attaches_production_ratio() {
        // The helper must stamp a `SwarmProduction`
        // component carrying the requested ratio so the
        // production systems route the opponent's
        // facilities through the opponent's mix.
        let mut app = build_app();
        let mut ratio = ProductionRatio::new();
        ratio.set_target(NanobotType::Hauler, 4);
        let swarm = spawn_opponent_swarm(app.world_mut(), Vec2::new(0.0, 0.0), ratio, &[], &[]);
        let world = app.world();
        let sp = world
            .entity(swarm)
            .get::<SwarmProduction>()
            .expect("SwarmProduction must be attached");
        assert_eq!(sp.ratio.target(NanobotType::Hauler), 4);
    }

    #[test]
    fn spawn_opponent_swarm_seeds_requested_nanobots() {
        // The helper must spawn the requested nanobots as
        // children of the new swarm so the same scoring
        // systems drive them.
        let mut app = build_app();
        let swarm = spawn_opponent_swarm(
            app.world_mut(),
            Vec2::new(0.0, 0.0),
            ProductionRatio::new(),
            &[],
            &[
                SeedNanobots::new(NanobotType::Worker, 3),
                SeedNanobots::new(NanobotType::Hauler, 2),
            ],
        );
        let world = app.world();
        let children: Vec<Entity> = world
            .get::<Children>(swarm)
            .map(|c| c.iter().collect())
            .unwrap_or_default();
        assert_eq!(children.len(), 5, "all seed nanobots must be children");
        let workers = children
            .iter()
            .filter(|c| {
                world.entity(**c).get::<NanobotType>().copied() == Some(NanobotType::Worker)
            })
            .count();
        let haulers = children
            .iter()
            .filter(|c| {
                world.entity(**c).get::<NanobotType>().copied() == Some(NanobotType::Hauler)
            })
            .count();
        assert_eq!(workers, 3);
        assert_eq!(haulers, 2);
    }
}
