//! Scripted playtest for ADR-0009: exhausted Gather Zones persist as intent
//! without making idle workers repeatedly claim invalid resource work.

use bevy::{math::Vec2, prelude::*};
use top_down_2d_rts_prototype_nano_swarm::{
    intent::{IntentGrid, IntentKind},
    nanobot::{GatherAssignment, RegionalLease, SwarmId},
    resources::ResourceDeposit,
};

#[path = "../common/mod.rs"]
mod common;

fn paint_gather(app: &mut App, cell: IVec2) {
    let mut grid = app.world_mut().resource_mut::<IntentGrid>();
    assert!(grid.paint_owned(cell, IntentKind::Gather, Some(SwarmId::PLAYER),));
}

#[test]
fn exhausted_gather_zone_persists_without_invalid_worker_retries() {
    let mut app = common::sim_app_with_gather();
    let gather_cell = IVec2::new(0, 0);
    let deposit_pos = common::cell_world_center(gather_cell);
    let deposit = common::spawn_deposit(&mut app, deposit_pos, 1);
    let _stockpile = common::spawn_stockpile(&mut app, deposit_pos + Vec2::new(96.0, 0.0), 0, 100);
    let worker = common::spawn_worker_at(&mut app, deposit_pos);
    paint_gather(&mut app, gather_cell);

    for _ in 0..8 {
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

    for _ in 0..20 {
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
