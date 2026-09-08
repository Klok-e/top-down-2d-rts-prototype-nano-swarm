//! Planned Production Facility behavior under population demand.

use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::battle_statistics::BattleCounters;
use top_down_2d_rts_prototype_nano_swarm::{
    intent::{IntentGrid, IntentKind},
    nanobot::{
        DEFAULT_PLANNED_WORK_TICKS, OwnerSwarm, PRODUCTION_PRESSURE_TICKS, PlannedKind,
        PlannedStructure, ProductionFacility, ProductionPressure, SwarmId, completed_visual_color,
        planned_visual_color,
    },
};

#[path = "../common/mod.rs"]
mod common;

fn paint(app: &mut App, cell: IVec2, kind: IntentKind) {
    assert!(
        app.world_mut()
            .resource_mut::<IntentGrid>()
            .paint(cell, kind, SwarmId::PLAYER)
    );
}

fn production_plans(app: &mut App) -> Vec<Entity> {
    let world = app.world_mut();
    world
        .query::<(Entity, &PlannedStructure)>()
        .iter(world)
        .filter_map(|(entity, planned)| {
            (planned.kind == PlannedKind::ProductionFacility).then_some(entity)
        })
        .collect()
}

#[test]
fn sustained_uncovered_demand_creates_one_owned_plan_after_pressure_window() {
    let mut app = common::sim_app_with_production_planned();
    let swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    common::spawn_worker_at(&mut app, Vec2::new(-1024.0, -1024.0));
    paint(&mut app, IVec2::ZERO, IntentKind::Build);
    paint(&mut app, IVec2::X, IntentKind::Build);

    for _ in 0..PRODUCTION_PRESSURE_TICKS - 1 {
        app.update();
    }
    assert_eq!(
        app.world()
            .resource::<ProductionPressure>()
            .ticks_for(SwarmId::PLAYER),
        PRODUCTION_PRESSURE_TICKS - 1,
    );
    assert!(production_plans(&mut app).is_empty());

    for _ in 0..101 {
        app.update();
        if !production_plans(&mut app).is_empty() {
            break;
        }
    }
    let plans = production_plans(&mut app);
    assert_eq!(plans.len(), 1, "pending capacity prevents duplicate plans");
    let plan = app.world().entity(plans[0]);
    assert_eq!(plan.get::<OwnerSwarm>().map(|owner| owner.0), Some(swarm));
    assert_eq!(
        plan.get::<Sprite>().expect("planned visual").color,
        planned_visual_color(),
    );
    assert!(plan.get::<ProductionFacility>().is_none());
}

#[test]
fn demand_without_an_owned_build_zone_creates_no_facility_plan() {
    let mut app = common::sim_app_with_production_planned();
    common::spawn_swarm_at(&mut app, Vec2::ZERO);
    paint(&mut app, IVec2::ZERO, IntentKind::Gather);
    common::spawn_deposit(
        &mut app,
        common::DepositFixture {
            world_pos: common::cell_world_center(IVec2::ZERO),
            amount: 100,
            capacity: 100,
            radius: 32.0,
        },
    );

    for _ in 0..PRODUCTION_PRESSURE_TICKS + 100 {
        app.update();
    }
    assert!(production_plans(&mut app).is_empty());
}

#[test]
fn worker_promotes_an_owned_plan_to_an_idle_unfunded_facility() {
    let mut app = common::sim_app_with_production_planned();
    app.init_resource::<BattleCounters>();
    let cell = IVec2::ZERO;
    let plan = common::spawn_planned_production_facility_at_cell(&mut app, cell);
    let owner = app.world().get::<OwnerSwarm>(plan).copied().unwrap();
    let center = common::cell_world_center(cell);
    common::spawn_worker_at(&mut app, center + Vec2::X * 72.0);

    for _ in 0..DEFAULT_PLANNED_WORK_TICKS + 250 {
        app.update();
        if app.world().get::<PlannedStructure>(plan).is_none() {
            break;
        }
    }

    let entity = app.world().entity(plan);
    assert!(entity.get::<PlannedStructure>().is_none());
    assert_eq!(
        entity.get::<OwnerSwarm>().map(|owner| owner.0),
        Some(owner.0)
    );
    let facility = entity
        .get::<ProductionFacility>()
        .expect("completed plan becomes a facility");
    assert_eq!(facility.current_target, None);
    assert_eq!(facility.input_amount, 0);
    assert_eq!(
        entity.get::<Sprite>().expect("completed visual").color,
        completed_visual_color(),
    );
    assert_eq!(
        app.world()
            .resource::<BattleCounters>()
            .totals_for(SwarmId::PLAYER)
            .structures_built,
        1
    );
}
