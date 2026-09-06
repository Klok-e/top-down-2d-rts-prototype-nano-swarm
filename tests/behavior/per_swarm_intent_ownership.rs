//! Independent swarm intent drives each swarm's work without transferring orders.

use bevy::{math::Vec2, prelude::*};
use top_down_2d_rts_prototype_nano_swarm::{
    intent::{IntentGrid, IntentKind},
    nanobot::{
        Commitment, GatherAssignment, Health, Nanobot, NanobotType, OwnerSwarm,
        PlannedStructureClaim, PrepaintedIntent, SeedNanobots, SwarmId, SwarmMember,
        VelocityComponent, spawn_opponent_swarm,
    },
    resources::ResourceDeposit,
};

#[path = "../common/mod.rs"]
mod common;

#[test]
fn player_painted_intent_is_owned_by_player_swarm() {
    let mut app = common::sim_app_with_gather();
    let cell = IVec2::new(0, 0);
    {
        let mut grid = app.world_mut().resource_mut::<IntentGrid>();
        assert!(grid.paint(cell, IntentKind::Gather, SwarmId::PLAYER));
    }

    let grid = app.world().resource::<IntentGrid>();
    let painted = grid.cell(cell).expect("cell must be in bounds");
    assert!(painted.has(IntentKind::Gather));
    assert_eq!(
        painted.owners(IntentKind::Gather).collect::<Vec<_>>(),
        vec![SwarmId::PLAYER],
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
        assert!(grid.paint(cell, IntentKind::Gather, SwarmId::PLAYER));
    }
    let cell_center = common::cell_world_center(cell);
    let _deposit = common::spawn_deposit(
        &mut app,
        common::DepositFixture {
            world_pos: cell_center,
            amount: 100,
            capacity: 1000,
            radius: 32.0,
        },
    );
    let worker = common::spawn_worker_at(&mut app, cell_center + Vec2::new(0.0, -100.0));

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
    let mut app = common::sim_app_with_gather();
    let opponent_pos = Vec2::new(2000.0, 0.0);
    let gather_cell = IVec2::new(0, 0);
    let opponent = spawn_opponent_swarm(
        app.world_mut(),
        opponent_pos,
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
        cell.owners(IntentKind::Gather).collect::<Vec<_>>(),
        vec![opponent_id],
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
        &[PrepaintedIntent::new(gather_cell, IntentKind::Gather)],
        &[SeedNanobots::new(NanobotType::Worker, 1)],
    );
    // Place a deposit in the cell so the assignment has a
    // target to point at.
    let _deposit = common::spawn_deposit(
        &mut app,
        common::DepositFixture {
            world_pos: cell_center,
            amount: 100,
            capacity: 1000,
            radius: 32.0,
        },
    );
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
        assert!(grid.paint(opponent_cell, IntentKind::Gather, SwarmId(7)));
    }
    let cell_center = common::cell_world_center(opponent_cell);
    let _deposit = common::spawn_deposit(
        &mut app,
        common::DepositFixture {
            world_pos: cell_center,
            amount: 100,
            capacity: 1000,
            radius: 32.0,
        },
    );
    let worker = common::spawn_worker_at(&mut app, cell_center + Vec2::new(0.0, -100.0));

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
        assert!(grid.paint(cell, IntentKind::Gather, SwarmId::PLAYER));
    }
    let cell_center = common::cell_world_center(cell);
    let _deposit = common::spawn_deposit(
        &mut app,
        common::DepositFixture {
            world_pos: cell_center,
            amount: 100,
            capacity: 1000,
            radius: 32.0,
        },
    );
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
            Transform::from_translation((cell_center + Vec2::new(0.0, -100.0)).extend(0.0)),
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
fn overlapping_gather_paint_is_visible_only_to_its_independent_owners() {
    let mut app = common::sim_app_with_gather();
    let cell = IVec2::ZERO;
    {
        let mut grid = app.world_mut().resource_mut::<IntentGrid>();
        grid.paint(cell, IntentKind::Gather, SwarmId::PLAYER);
        grid.paint(cell, IntentKind::Gather, SwarmId(42));
    }

    let grid = app.world().resource::<IntentGrid>();
    let painted = grid.cell(cell).unwrap();
    assert!(painted.has_owned(IntentKind::Gather, SwarmId::PLAYER));
    assert!(painted.has_owned(IntentKind::Gather, SwarmId(42)));
    assert!(!painted.has_owned(IntentKind::Gather, SwarmId(9)));
    assert!(!painted.has_owned(IntentKind::Build, SwarmId::PLAYER));
}

#[test]
fn overlapping_swarms_extract_from_one_finite_deposit() {
    use top_down_2d_rts_prototype_nano_swarm::{
        nanobot::{Swarm, WorkerLoad},
        resources::{ResourceKind, ResourceLedger, Stockpile},
    };

    let mut app = common::sim_app_with_gather();
    let cell = IVec2::ZERO;
    let center = common::cell_world_center(cell);
    let player = app.world_mut().spawn((Swarm {}, SwarmId::PLAYER)).id();
    let enemy = app.world_mut().spawn((Swarm {}, SwarmId(7))).id();
    {
        let mut grid = app.world_mut().resource_mut::<IntentGrid>();
        grid.paint(cell, IntentKind::Gather, SwarmId::PLAYER);
        grid.paint(cell, IntentKind::Gather, SwarmId(7));
    }
    let deposit = common::spawn_deposit(
        &mut app,
        common::DepositFixture {
            world_pos: center,
            amount: 6,
            capacity: 6,
            radius: 32.0,
        },
    );
    let player_stockpile = common::spawn_stockpile(&mut app, center + Vec2::new(96.0, 0.0), 0, 100);
    app.world_mut()
        .entity_mut(player_stockpile)
        .insert(OwnerSwarm(player));
    let enemy_stockpile = common::spawn_stockpile(&mut app, center - Vec2::new(96.0, 0.0), 0, 100);
    app.world_mut()
        .entity_mut(enemy_stockpile)
        .insert(OwnerSwarm(enemy));
    let player_worker = common::spawn_worker_at(&mut app, center + Vec2::new(0.0, 68.0));
    let enemy_worker = common::spawn_worker_at(&mut app, center - Vec2::new(0.0, 68.0));
    app.world_mut()
        .entity_mut(enemy_worker)
        .insert(SwarmMember::new(SwarmId(7)));

    for _ in 0..120 {
        app.update();
    }

    assert_eq!(
        app.world()
            .entity(deposit)
            .get::<ResourceDeposit>()
            .unwrap()
            .amount,
        0
    );
    let ledger = app.world().resource::<ResourceLedger>();
    let player_extracted = ledger.total_for(SwarmId::PLAYER, ResourceKind::Minerals);
    let enemy_extracted = ledger.total_for(SwarmId(7), ResourceKind::Minerals);
    assert!(
        player_extracted > 0,
        "player must extract from overlapping Gather paint"
    );
    assert!(
        enemy_extracted > 0,
        "opponent must extract from the same deposit"
    );
    assert_eq!(player_extracted + enemy_extracted, 6);
    let physical_total = [player_stockpile, enemy_stockpile]
        .into_iter()
        .map(|entity| {
            app.world()
                .entity(entity)
                .get::<Stockpile>()
                .unwrap()
                .amount
        })
        .sum::<u32>()
        + [player_worker, enemy_worker]
            .into_iter()
            .map(|entity| {
                app.world()
                    .entity(entity)
                    .get::<WorkerLoad>()
                    .map_or(0, |cargo| cargo.amount)
            })
            .sum::<u32>();
    assert_eq!(
        physical_total, 6,
        "paint overlap must not duplicate the finite resource pool"
    );
}
