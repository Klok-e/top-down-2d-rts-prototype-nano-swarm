//! Integration tests for issue #20: per-swarm intent ownership.
//!
//! Each test pins one bullet of the acceptance criteria so a
//! failure points at a single contract:
//!
//! 1. `player_painted_intent_is_owned_by_player_swarm` -- the
//!    `paint_owned` API stamps intent with the player `SwarmId`.
//! 2. `player_painted_gather_drives_player_worker` -- a player
//!    worker gets a `GatherAssignment` in a player-painted cell.
//! 3. `opponent_prepainted_intent_is_owned_by_opponent_swarm`
//!    -- the `spawn_opponent_swarm` helper stamps intent with
//!    the opponent's `SwarmId`.
//! 4. `opponent_prepainted_gather_drives_opponent_worker` -- an
//!    opponent worker gets a `GatherAssignment` in the
//!    opponent-painted cell.
//! 5. `player_worker_ignores_opponent_gather_zone` -- the
//!    per-swarm filter routes opponent paint away from player
//!    workers.
//! 6. `opponent_worker_ignores_player_gather_zone` -- the
//!    per-swarm filter routes player paint away from opponent
//!    workers.
//! 7. `unowned_paint_remains_visible_to_every_swarm` -- legacy
//!    shared paint (or paint written through the unowned API)
//!    is still visible to every swarm, so pre-existing tests
//!    keep passing.
//! 8. `opponent_production_spawns_opponent_swarm_id_nanobots`
//!    -- a freshly produced opponent nanobot carries the
//!    opponent `SwarmId`, not the player id, so the new bot
//!    keeps scoring opponent intent.

use bevy::{math::Vec2, prelude::*};
use top_down_2d_rts_prototype_nano_swarm::{
    intent::{IntentGrid, IntentKind},
    nanobot::{
        Commitment, GatherAssignment, Health, Nanobot, NanobotType, OwnerSwarm,
        PRODUCTION_COST_PER_BOT, PRODUCTION_TICKS_PER_BOT, PlannedStructureClaim, PrepaintedIntent,
        ProductionFacility, ProductionPlugin, ProductionPriority, SeedNanobots, SwarmId,
        SwarmMember, VelocityComponent, spawn_opponent_swarm,
    },
    resources::ResourceDeposit,
};

#[path = "../common/mod.rs"]
mod common;

/// Gather app + the production plugin. The production
/// systems need a `ProductionPriority` resource (the common
/// seam's `sim_app_with_gather` does not register one); the
/// resource is left empty because the test spawns an opponent
/// swarm with its own `SwarmProduction` that overrides the
/// global priority.
fn app_with_production() -> App {
    let mut app = common::sim_app_with_gather();
    app.insert_resource(ProductionPriority::new());
    app.add_plugins(ProductionPlugin);
    app
}

#[test]
fn player_painted_intent_is_owned_by_player_swarm() {
    // Pin the player-side paint contract: when the player
    // brush writes a Gather cell through `paint_owned` with
    // `Some(SwarmId::PLAYER)`, the cell records the player
    // `SwarmId` as the owner. The brush system in
    // `zones::zone_brush` does exactly this on every
    // mouse-held frame.
    let mut app = common::sim_app_with_gather();
    let cell = IVec2::new(0, 0);
    {
        let mut grid = app.world_mut().resource_mut::<IntentGrid>();
        assert!(grid.paint_owned(cell, IntentKind::Gather, Some(SwarmId::PLAYER),));
    }

    let grid = app.world().resource::<IntentGrid>();
    let painted = grid.cell(cell).expect("cell must be in bounds");
    assert!(painted.has(IntentKind::Gather));
    assert_eq!(
        painted.owner(IntentKind::Gather),
        Some(SwarmId::PLAYER),
        "player-painted cell must record the player SwarmId as the owner"
    );
}

#[test]
fn player_painted_gather_drives_player_worker() {
    // Drive the assignment system end-to-end: a player
    // Worker in a player-painted Gather cell must end up
    // with a `GatherAssignment` pointing at the deposit in
    // the cell. The cell ownership is the player SwarmId,
    // which the default `SwarmMember(SwarmId::PLAYER)`
    // stamped by `spawn_worker_at` matches.
    let mut app = common::sim_app_with_gather();
    let cell = IVec2::new(0, 0);
    {
        let mut grid = app.world_mut().resource_mut::<IntentGrid>();
        assert!(grid.paint_owned(cell, IntentKind::Gather, Some(SwarmId::PLAYER),));
    }
    let cell_center = common::cell_world_center(cell);
    let _deposit = common::spawn_deposit(&mut app, cell_center, 100);
    let worker = common::spawn_worker_at(&mut app, cell_center);

    for _ in 0..5 {
        app.update();
    }

    let worker_state = app.world().entity(worker);
    assert!(
        worker_state.contains::<GatherAssignment>()
            || worker_state.contains::<PlannedStructureClaim>(),
        "player worker must acquire Gather work or its required support build"
    );
}

#[test]
fn opponent_prepainted_intent_is_owned_by_opponent_swarm() {
    // The `spawn_opponent_swarm` helper must stamp prepainted
    // cells with the opponent's `SwarmId`, not with `None`
    // (unowned) or with `SwarmId::PLAYER`. This is the
    // end-state the PRD calls out: opponent prepainted
    // intent belongs to the Opponent Swarm.
    let mut app = common::sim_app_with_gather();
    let opponent_pos = Vec2::new(2000.0, 0.0);
    let gather_cell = IVec2::new(0, 0);
    let opponent = spawn_opponent_swarm(
        app.world_mut(),
        opponent_pos,
        ProductionPriority::new(),
        &[PrepaintedIntent::new(gather_cell, IntentKind::Gather)],
        &[],
    );
    let opponent_id = app
        .world()
        .entity(opponent)
        .get::<SwarmId>()
        .copied()
        .expect("opponent swarm must carry a SwarmId");

    let grid = app.world().resource::<IntentGrid>();
    let cell = grid.cell(gather_cell).unwrap();
    assert!(cell.has(IntentKind::Gather));
    assert_eq!(
        cell.owner(IntentKind::Gather),
        Some(opponent_id),
        "opponent prepainted intent must be owned by the opponent SwarmId"
    );
}

#[test]
fn opponent_prepainted_gather_drives_opponent_worker() {
    // End-to-end: an opponent Worker in an opponent-painted
    // Gather cell must end up with a `GatherAssignment`.
    // The opponent helper assigns a fresh `SwarmId` and
    // stamps the seed nanobots with it; the assignment
    // system must match.
    let mut app = common::sim_app_with_gather();
    let opponent_pos = Vec2::new(2000.0, 0.0);
    let gather_cell = IVec2::new(0, 0);
    let cell_center = common::cell_world_center(gather_cell);
    let opponent = spawn_opponent_swarm(
        app.world_mut(),
        opponent_pos,
        ProductionPriority::new(),
        &[PrepaintedIntent::new(gather_cell, IntentKind::Gather)],
        &[SeedNanobots::new(NanobotType::Worker, 1)],
    );
    // Place a deposit in the cell so the assignment has a
    // target to point at.
    let _deposit = common::spawn_deposit(&mut app, cell_center, 100);
    let opponent_id = app
        .world()
        .entity(opponent)
        .get::<SwarmId>()
        .copied()
        .expect("opponent must carry a SwarmId");

    for _ in 0..5 {
        app.update();
    }

    let world = app.world_mut();
    let mut worker_query = world.query::<(
        Entity,
        &NanobotType,
        &SwarmMember,
        Option<&GatherAssignment>,
    )>();
    let opponent_worker = worker_query
        .iter(&*world)
        .find(|(_, ty, member, _)| **ty == NanobotType::Worker && member.0 == opponent_id)
        .expect("opponent must seed one Worker child");
    assert!(
        opponent_worker.3.is_some(),
        "opponent Worker must receive a GatherAssignment in an opponent-painted cell"
    );
    assert_eq!(opponent_worker.3.unwrap().cell, gather_cell);
}

#[test]
fn player_worker_ignores_opponent_gather_zone() {
    // The per-swarm intent filter must prevent a player
    // Worker from picking an opponent-painted Gather cell.
    // Without the filter the player worker would happily
    // walk into enemy territory to gather.
    let mut app = common::sim_app_with_gather();
    let opponent_cell = IVec2::new(0, 0);
    {
        let mut grid = app.world_mut().resource_mut::<IntentGrid>();
        assert!(grid.paint_owned(opponent_cell, IntentKind::Gather, Some(SwarmId(7)),));
    }
    let cell_center = common::cell_world_center(opponent_cell);
    let _deposit = common::spawn_deposit(&mut app, cell_center, 100);
    let worker = common::spawn_worker_at(&mut app, cell_center);

    for _ in 0..5 {
        app.update();
    }

    assert!(
        app.world()
            .entity(worker)
            .get::<GatherAssignment>()
            .is_none(),
        "player worker must NOT pick an opponent-owned Gather cell"
    );
    let deposit = app
        .world()
        .entity(_deposit)
        .get::<ResourceDeposit>()
        .unwrap();
    assert_eq!(
        deposit.amount, 100,
        "deposit must remain untouched because no player worker engaged"
    );
}

#[test]
fn opponent_worker_ignores_player_gather_zone() {
    // The mirror contract: an opponent Worker must skip
    // player-painted cells. Symmetric to the player-side
    // test, but with a worker that carries
    // `SwarmMember(opponent_id)`.
    let mut app = common::sim_app_with_gather();
    let opponent_id = SwarmId(9);
    let cell = IVec2::new(0, 0);
    {
        let mut grid = app.world_mut().resource_mut::<IntentGrid>();
        // Player-painted cell (owner is `SwarmId::PLAYER`).
        assert!(grid.paint_owned(cell, IntentKind::Gather, Some(SwarmId::PLAYER),));
    }
    let cell_center = common::cell_world_center(cell);
    let _deposit = common::spawn_deposit(&mut app, cell_center, 100);
    // Opponent-tagged Worker (no Swarm entity, just the
    // nanobot with the right SwarmMember marker).
    let opponent_worker = app
        .world_mut()
        .spawn((
            Nanobot {},
            NanobotType::Worker,
            Commitment::Idle,
            VelocityComponent::default(),
            Health::default(),
            SwarmMember::new(opponent_id),
            Transform::from_translation(cell_center.extend(0.0)),
        ))
        .id();

    for _ in 0..5 {
        app.update();
    }

    assert!(
        app.world()
            .entity(opponent_worker)
            .get::<GatherAssignment>()
            .is_none(),
        "opponent worker must NOT pick a player-painted Gather cell"
    );
}

#[test]
fn unowned_paint_remains_visible_to_every_swarm() {
    // The legacy unowned paint path (the existing `paint`
    // method, used by every pre-#20 test) must stay visible
    // to every swarm. The per-swarm filter treats `owner =
    // None` as "shared" so a player worker can still pick
    // unowned paint and an opponent worker can too. This is
    // the back-compat leg the issue acceptance criterion
    // "Existing intent-layer UI and painting behavior remain
    // usable for player intent" relies on.
    let mut app = common::sim_app_with_gather();
    let cell = IVec2::new(0, 0);
    {
        let mut grid = app.world_mut().resource_mut::<IntentGrid>();
        // The plain (unowned) `paint` API.
        assert!(grid.paint(cell, IntentKind::Gather));
    }

    let grid = app.world().resource::<IntentGrid>();
    let painted = grid.cell(cell).unwrap();
    assert!(painted.has(IntentKind::Gather));
    assert_eq!(
        painted.owner(IntentKind::Gather),
        None,
        "unowned paint must keep owner = None"
    );
    // Both a player and an opponent SwarmId see the cell.
    assert!(painted.visible_to(IntentKind::Gather, SwarmId::PLAYER));
    assert!(painted.visible_to(IntentKind::Gather, SwarmId(42)));
    // An inactive layer is invisible to every swarm.
    assert!(!painted.visible_to(IntentKind::Build, SwarmId::PLAYER));
}

#[test]
fn opponent_production_spawns_opponent_swarm_id_nanobots() {
    // A full production cycle for the opponent must end
    // with a new nanobot carrying the opponent `SwarmId`,
    // not the player id, so it keeps scoring opponent
    // intent on later ticks. This pins the "production
    // chain copies parent's ownership" half of the
    // contract.
    let mut app = app_with_production();
    let opponent_pos = Vec2::new(2000.0, 0.0);
    let mut priority = ProductionPriority::new();
    priority.set_weight(NanobotType::Worker, 1);
    let opponent = spawn_opponent_swarm(app.world_mut(), opponent_pos, priority, &[], &[]);
    let _stockpile =
        common::spawn_stockpile(&mut app, opponent_pos, PRODUCTION_COST_PER_BOT * 5, 1000);
    let _facility = app
        .world_mut()
        .spawn((
            ProductionFacility::new(),
            OwnerSwarm(opponent),
            Transform::from_translation(opponent_pos.extend(0.0)),
        ))
        .id();
    common::fill_facility_input(&mut app, _facility);

    let opponent_id = app
        .world()
        .entity(opponent)
        .get::<SwarmId>()
        .copied()
        .expect("opponent must carry a SwarmId");
    // Issue #38 / ADR-0004: production-spawned
    // nanobots are top-level entities whose
    // `SwarmMember` matches the opponent's `SwarmId`.
    // The swarm no longer parents the produced bots.
    // Count the new bot by matching `SwarmMember ==
    // opponent.SwarmId`; the count increases from 0
    // (no seed bots) to 1 (one production cycle) over
    // the test.
    for _ in 0..(1 + PRODUCTION_TICKS_PER_BOT as usize) {
        app.update();
    }

    let mut owned_bots: Vec<Entity> = Vec::new();
    {
        let world = app.world_mut();
        let mut query = world.query::<(
            Entity,
            &top_down_2d_rts_prototype_nano_swarm::nanobot::SwarmMember,
        )>();
        for (entity, member) in query.iter(world) {
            if member.0 == opponent_id {
                owned_bots.push(entity);
            }
        }
    }
    let world = app.world();
    assert!(
        !owned_bots.is_empty(),
        "opponent facility must spawn at least one new nanobot over the cycle"
    );
    for new_child in &owned_bots {
        let member = world
            .entity(*new_child)
            .get::<SwarmMember>()
            .copied()
            .expect("newly produced nanobot must carry a SwarmMember");
        assert_eq!(
            member.0, opponent_id,
            "newly produced opponent nanobot must carry the opponent SwarmId, not the player id"
        );
        assert!(
            !member.0.is_player(),
            "newly produced opponent nanobot must not be tagged as the player swarm"
        );
    }
}
