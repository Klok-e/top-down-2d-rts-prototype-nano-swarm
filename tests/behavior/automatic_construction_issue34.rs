//! Behavior tests for issue #34: automatic construction is demand-driven
//! and planned/completed support structures obey shared footprint rules.

use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    intent::{IntentGrid, IntentKind},
    nanobot::{
        BUILDING_FOOTPRINT_PADDING, BUILDING_FOOTPRINT_RADIUS, Commitment, DefendHold, NanobotType,
        OwnerSwarm, PlannedKind, PlannedProductionTarget, PlannedStructure, PlannedStructureClaim,
        SwarmId,
    },
    resources::ResourceDeposit,
};

#[path = "../common/mod.rs"]
mod common;

fn paint_build(app: &mut App, cell: IVec2) {
    let mut grid = app.world_mut().resource_mut::<IntentGrid>();
    assert!(grid.paint_owned(cell, IntentKind::Build, Some(SwarmId::PLAYER),));
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

/// Minimum centre-to-centre distance between a planned Source
/// Stockpile and any obstacle (deposit, planned structure,
/// completed stockpile, production facility, or charger). The
/// threshold is the sum of both half-footprints plus the
/// shared padding: 32 + 16 + 32 = 80. The placement
/// algorithm's "centre-to-centre distance is less than the
/// threshold" check rejects any candidate that would sit
/// closer than this.
const SOURCE_STOCKPILE_OBSTACLE_GAP: f32 =
    BUILDING_FOOTPRINT_RADIUS + BUILDING_FOOTPRINT_PADDING + BUILDING_FOOTPRINT_RADIUS;

fn paint_gather(app: &mut App, cell: IVec2) {
    let mut grid = app.world_mut().resource_mut::<IntentGrid>();
    assert!(grid.paint_owned(cell, IntentKind::Gather, Some(SwarmId::PLAYER),));
}

fn paint_defend(app: &mut App, cell: IVec2) {
    let mut grid = app.world_mut().resource_mut::<IntentGrid>();
    assert!(grid.paint_owned(cell, IntentKind::Defend, Some(SwarmId::PLAYER),));
}

fn planned_source_stockpile_position(app: &mut App) -> Option<Vec2> {
    let world = app.world_mut();
    world
        .query::<(&PlannedStructure, &Transform)>()
        .iter(world)
        .find(|(p, _)| p.kind == PlannedKind::SourceStockpile)
        .map(|(_, t)| t.translation.truncate())
}

#[test]
fn source_stockpile_placement_rejects_production_facility_overlap() {
    // Acceptance: "Source Stockpile placement rejects
    // candidates that overlap ... Production Facilities ...
    // including a small padding gap." A Production Facility
    // placed on the natural east-of-deposit candidate must
    // cause the demand system to skip that candidate. With
    // the haul direction effectively zero (no Build cells,
    // swarm at the deposit position) the algorithm would
    // otherwise pick the east candidate on tie-break.
    let mut app = common::sim_app_with_gather();
    let cell = IVec2::new(0, 0);
    let center = common::cell_world_center(cell);
    paint_gather(&mut app, cell);
    let _swarm = common::spawn_swarm_at(&mut app, center);
    let _worker = common::spawn_worker_at(&mut app, center);
    let _deposit = common::spawn_deposit(&mut app, center, 100);
    // Production Facility at the east-of-deposit candidate
    // (96 world units east of the deposit, no jitter). The
    // facility's footprint is `BUILDING_FOOTPRINT_RADIUS`
    // and the placement uses `BUILDING_FOOTPRINT_PADDING`,
    // so the obstacle must reject any candidate whose
    // centre is within `SOURCE_STOCKPILE_OBSTACLE_GAP`
    // world units.
    let facility_pos = center + Vec2::new(96.0, 0.0);
    let facility = common::spawn_idle_facility_at(&mut app, facility_pos);
    let _hauler = common::spawn_hauler_at(&mut app, center);

    for _ in 0..5 {
        app.update();
    }

    let pos = planned_source_stockpile_position(&mut app)
        .expect("Planned Source Stockpile must still be created at a non-overlapping candidate");
    let facility_world = app
        .world()
        .entity(facility)
        .get::<Transform>()
        .expect("facility has Transform")
        .translation
        .truncate();
    assert!(
        pos.distance(facility_world) >= SOURCE_STOCKPILE_OBSTACLE_GAP,
        "Source Stockpile placement must reject candidates that overlap a \
         Production Facility; got pos={pos:?}, facility={facility_world:?}, \
         threshold={SOURCE_STOCKPILE_OBSTACLE_GAP}"
    );
}

#[test]
fn source_stockpile_placement_rejects_charger_overlap() {
    // Acceptance: "Source Stockpile placement rejects
    // candidates that overlap ... Chargers ... including a
    // small padding gap." Same shape as the Production
    // Facility overlap test, with a Charger as the
    // obstacle.
    let mut app = common::sim_app_with_charge();
    let cell = IVec2::new(0, 0);
    let center = common::cell_world_center(cell);
    paint_gather(&mut app, cell);
    let _swarm = common::spawn_swarm_at(&mut app, center);
    let _worker = common::spawn_worker_at(&mut app, center);
    let _deposit = common::spawn_deposit(&mut app, center, 100);
    let charger_pos = center + Vec2::new(96.0, 0.0);
    let charger = common::spawn_charger_at(&mut app, IVec2::new(0, 0), 0);
    app.world_mut()
        .entity_mut(charger)
        .insert(Transform::from_translation(charger_pos.extend(0.0)));

    for _ in 0..5 {
        app.update();
    }

    let pos = planned_source_stockpile_position(&mut app)
        .expect("Planned Source Stockpile must still be created at a non-overlapping candidate");
    assert!(
        pos.distance(charger_pos) >= SOURCE_STOCKPILE_OBSTACLE_GAP,
        "Source Stockpile placement must reject candidates that overlap a \
         Charger; got pos={pos:?}, charger={charger_pos:?}, threshold={SOURCE_STOCKPILE_OBSTACLE_GAP}"
    );
}

#[test]
fn charger_placement_rejects_planned_sink_stockpile_overlap() {
    // Acceptance: "Charger placement rejects candidates
    // that overlap ... Planned Structures ... including a
    // small padding gap." A Planned Sink Stockpile in the
    // same Defend cell must be in the Charger placement's
    // obstacle list, so the demand system either picks a
    // different position or produces no plan when every
    // candidate overlaps.
    let mut app = common::sim_app_with_charge_planned();
    let cell = IVec2::new(0, 0);
    let center = common::cell_world_center(cell);
    paint_defend(&mut app, cell);
    let swarm = common::spawn_swarm_at(&mut app, center);
    let _defender = common::spawn_defender_at(&mut app, center);
    app.world_mut()
        .entity_mut(_defender)
        .insert(DefendHold { cell });
    // A Planned Sink Stockpile at the cell center. The
    // Charger's natural placement (cell center) must
    // collide with this plan, so the demand system must
    // either pick an alternative position or produce no
    // plan. The "no overlapping fallback" acceptance
    // requires no overlap with the planned Sink.
    let sink_plan =
        common::spawn_planned_structure_of_kind_at_cell(&mut app, cell, PlannedKind::SinkStockpile);
    app.world_mut()
        .entity_mut(sink_plan)
        .insert(OwnerSwarm(swarm));

    for _ in 0..5 {
        app.update();
    }

    let world = app.world_mut();
    let sink_pos = world
        .entity(sink_plan)
        .get::<Transform>()
        .expect("sink plan has Transform")
        .translation
        .truncate();
    let mut q = world.query::<(&PlannedStructure, &Transform)>();
    let mut charger_count = 0u32;
    for (planned, transform) in q.iter(world) {
        if planned.kind != PlannedKind::Charger {
            continue;
        }
        charger_count += 1;
        let charger_pos = transform.translation.truncate();
        assert!(
            charger_pos.distance(sink_pos) >= SOURCE_STOCKPILE_OBSTACLE_GAP,
            "Charger placement must not overlap a Planned Sink Stockpile; \
             got charger={charger_pos:?}, sink_plan={sink_pos:?}, \
             threshold={SOURCE_STOCKPILE_OBSTACLE_GAP}"
        );
    }
    assert!(
        charger_count > 0,
        "Defend demand must still produce a Charger plan at a non-overlapping cell offset"
    );
}
