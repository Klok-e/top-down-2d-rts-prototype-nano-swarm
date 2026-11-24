//! Scripted playtests for ADR-0009 regional allocation: exhausted Gather Zones
//! persist as intent without invalid worker retries, and claimed work does not
//! strand idle Workers.

use bevy::{math::Vec2, prelude::*};
use top_down_2d_rts_prototype_nano_swarm::{
    intent::{IntentGrid, IntentKind},
    nanobot::{GatherAssignment, PlannedKind, PlannedStructure, RegionalLease, SwarmId},
    resources::ResourceDeposit,
};

#[path = "../common/mod.rs"]
mod common;

fn paint_gather(app: &mut App, cell: IVec2) {
    let mut grid = app.world_mut().resource_mut::<IntentGrid>();
    assert!(grid.paint(cell, IntentKind::Gather, SwarmId::PLAYER));
}

#[test]
fn exhausted_gather_zone_persists_without_invalid_worker_retries() {
    let mut app = common::sim_app_with_gather();
    let gather_cell = IVec2::new(0, 0);
    let deposit_pos = common::cell_world_center(gather_cell);
    let deposit = common::spawn_deposit(
        &mut app,
        common::DepositFixture {
            world_pos: deposit_pos,
            amount: 1,
            capacity: 1000,
            radius: 32.0,
        },
    );
    let _stockpile = common::spawn_stockpile(&mut app, deposit_pos + Vec2::new(96.0, 0.0), 0, 100);
    let worker = common::spawn_worker_at(&mut app, deposit_pos - Vec2::new(68.0, 0.0));
    paint_gather(&mut app, gather_cell);

    for _ in 0..300 {
        app.update();
    }

    let world = app.world();
    assert_eq!(
        world
            .entity(deposit)
            .get::<ResourceDeposit>()
            .unwrap()
            .amount,
        0,
        "deposit should be exhausted by the worker"
    );
    assert!(
        world
            .resource::<IntentGrid>()
            .cell(gather_cell)
            .is_some_and(|cell| cell.has(IntentKind::Gather)),
        "Gather intent persists after local depletion"
    );
    assert!(
        world.entity(worker).get::<GatherAssignment>().is_none(),
        "worker should not keep a stale GatherAssignment for exhausted deposit"
    );
    assert!(
        world.entity(worker).get::<RegionalLease>().is_none(),
        "regional Gather lease should be revoked once supporting work disappears"
    );

    for _ in 0..300 {
        app.update();
    }

    let world = app.world();
    assert!(
        world.entity(worker).get::<GatherAssignment>().is_none(),
        "worker must not repeatedly reclaim exhausted persistent Gather intent"
    );
    assert!(
        world.entity(worker).get::<RegionalLease>().is_none(),
        "worker must not repeatedly reacquire a lease for invalid Gather work"
    );
}

#[test]
fn lone_idle_worker_gathers_while_distant_construction_is_claimed() {
    let mut app = common::sim_app_with_gather_planned();
    app.insert_resource(IntentGrid::new(32, 32));

    // A Worker in another allocation region claims long-running construction.
    let plan_cell = IVec2::new(9, 1);
    let plan = common::spawn_planned_charger_at_cell(&mut app, plan_cell);
    app.world_mut()
        .entity_mut(plan)
        .insert(PlannedStructure::new(PlannedKind::Charger, plan_cell).with_work_budget(100_000));
    let builder = common::spawn_worker_at(
        &mut app,
        common::cell_world_center(plan_cell) - Vec2::X * 68.0,
    );
    for _ in 0..120 {
        if app
            .world()
            .get::<PlannedStructure>(plan)
            .unwrap()
            .active_worker()
            == Some(builder)
        {
            break;
        }
        app.update();
    }
    assert_eq!(
        app.world()
            .get::<PlannedStructure>(plan)
            .unwrap()
            .active_worker(),
        Some(builder),
        "setup: the builder should claim the construction"
    );

    // A second Worker, alone in its own region, beside painted minerals.
    let gather_cell = IVec2::new(1, 1);
    let deposit_pos = common::cell_world_center(gather_cell);
    common::spawn_deposit(
        &mut app,
        common::DepositFixture {
            world_pos: deposit_pos,
            amount: 1000,
            capacity: 1000,
            radius: 32.0,
        },
    );
    common::spawn_stockpile(&mut app, deposit_pos + Vec2::new(96.0, 0.0), 0, 100);
    paint_gather(&mut app, gather_cell);
    let gatherer = common::spawn_worker_at(&mut app, deposit_pos - Vec2::new(68.0, 0.0));

    for _ in 0..60 {
        app.update();
    }

    let world = app.world();
    assert_eq!(
        world.get::<PlannedStructure>(plan).unwrap().active_worker(),
        Some(builder),
        "the construction should stay claimed by its own Worker"
    );
    assert!(
        world.entity(gatherer).contains::<GatherAssignment>(),
        "a lone idle Worker must not wait on construction another Worker already claimed"
    );
}
