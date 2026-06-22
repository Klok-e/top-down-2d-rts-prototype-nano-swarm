//! Behavior tests for issue #34: automatic construction is demand-driven
//! and planned/completed support structures obey shared footprint rules.

use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    intent::{IntentGrid, IntentKind, PAINT_STRENGTH_CAP},
    nanobot::{
        Commitment, NanobotType, OwnerSwarm, PlannedKind, PlannedProductionTarget,
        PlannedStructure, PlannedStructureClaim, SwarmId,
    },
    resources::ResourceDeposit,
};

#[path = "../common/mod.rs"]
mod common;

fn paint_build(app: &mut App, cell: IVec2) {
    let mut grid = app.world_mut().resource_mut::<IntentGrid>();
    assert!(grid.paint_owned(
        cell,
        IntentKind::Build,
        PAINT_STRENGTH_CAP,
        Some(SwarmId::PLAYER),
    ));
}

fn spawn_owned_planned_production(app: &mut App, cell: IVec2) -> Entity {
    let owner = common::spawn_swarm_at(app, common::cell_world_center(cell));
    app.world_mut()
        .spawn((
            PlannedStructure::new(PlannedKind::ProductionFacility, cell),
            PlannedProductionTarget(NanobotType::Worker),
            OwnerSwarm(owner),
            Transform::from_translation(common::cell_world_center(cell).extend(0.0)),
        ))
        .id()
}

#[test]
fn build_paint_alone_creates_no_plan_and_pulls_no_worker() {
    let mut app = common::sim_app_with_planned();
    let cell = IVec2::new(0, 0);
    paint_build(&mut app, cell);
    let worker = common::spawn_worker_at(&mut app, common::cell_world_center(cell));

    app.update();

    let world = app.world_mut();
    let plan_count = world.query::<&PlannedStructure>().iter(world).count();
    assert_eq!(
        plan_count, 0,
        "Build paint alone must not create any PlannedStructure; got {plan_count}"
    );
    assert!(
        world
            .entity(worker)
            .get::<PlannedStructureClaim>()
            .is_none(),
        "Build paint alone must not pull an idle Worker into construction"
    );
    assert_eq!(
        *world
            .entity(worker)
            .get::<Commitment>()
            .expect("worker must keep Commitment"),
        Commitment::Idle,
        "Build paint alone must leave Worker idle"
    );
}

#[test]
fn pending_consumer_creates_non_overlapping_sink_stockpile() {
    let mut app = common::sim_app_with_planned();
    let cell = IVec2::new(0, 0);
    paint_build(&mut app, cell);
    let consumer = spawn_owned_planned_production(&mut app, cell);

    app.update();

    let world = app.world_mut();
    let consumer_pos = world
        .entity(consumer)
        .get::<Transform>()
        .expect("consumer must have transform")
        .translation
        .truncate();
    let mut q = world.query::<(&PlannedStructure, &Transform)>();
    let sink_pos = q
        .iter(world)
        .find_map(|(planned, transform)| {
            (planned.kind == PlannedKind::SinkStockpile).then_some(transform.translation.truncate())
        })
        .expect("pending production consumer must create a Planned Sink Stockpile");
    assert_eq!(
        top_down_2d_rts_prototype_nano_swarm::nanobot::world_to_cell(sink_pos),
        cell,
        "sink plan must stay inside the Build Zone cell"
    );
    assert!(
        sink_pos.distance(consumer_pos) >= 80.0,
        "sink plan must not overlap consumer footprint plus padding; consumer={consumer_pos:?} sink={sink_pos:?}"
    );
}

#[test]
fn blocked_build_cell_creates_no_overlapping_sink_fallback() {
    let mut app = common::sim_app_with_planned();
    let cell = IVec2::new(0, 0);
    let center = common::cell_world_center(cell);
    paint_build(&mut app, cell);
    spawn_owned_planned_production(&mut app, cell);
    app.world_mut().spawn((
        ResourceDeposit {
            kind: top_down_2d_rts_prototype_nano_swarm::resources::ResourceKind::Minerals,
            amount: 1000,
            capacity: 1000,
            radius: 300.0,
        },
        Transform::from_translation(center.extend(0.0)),
    ));

    app.update();

    let world = app.world_mut();
    let mut q = world.query::<&PlannedStructure>();
    let sink_count = q
        .iter(world)
        .filter(|planned| planned.kind == PlannedKind::SinkStockpile)
        .count();
    assert_eq!(
        sink_count, 0,
        "blocked Build cell must not create overlapping Sink Stockpile fallback"
    );
}
