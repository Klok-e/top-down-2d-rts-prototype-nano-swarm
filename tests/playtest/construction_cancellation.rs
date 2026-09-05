//! Unsafe completion releases capacity before demand chooses a replacement site.
use bevy::{ecs::system::RunSystemOnce, prelude::*};
use top_down_2d_rts_prototype_nano_swarm::{
    intent::{IntentGrid, IntentKind},
    nanobot::{
        OwnerSwarm, PlannedKind, PlannedStructure, ProductionFacility, ProductionPressure,
        ProductionPriority, StructureClearing, SwarmId,
        construction_access::{CancelledSites, ConstructionAccess},
    },
    physical_world::PhysicalWorld,
    structure_overlay::CancelledPlanVisual,
};

#[path = "../common/mod.rs"]
mod common;

#[test]
fn cancelled_facility_restarts_pressure_then_plans_one_alternative() {
    let mut app = common::sim_app_with_production_planned();
    let swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    common::spawn_worker_at(&mut app, Vec2::new(-1024., -1024.));
    let mut priority = ProductionPriority::new();
    priority.set_weight(
        top_down_2d_rts_prototype_nano_swarm::nanobot::NanobotType::Hauler,
        10,
    );
    app.insert_resource(priority);
    for cell in [IVec2::ZERO, IVec2::X] {
        app.world_mut().resource_mut::<IntentGrid>().paint_owned(
            cell,
            IntentKind::Build,
            Some(SwarmId::PLAYER),
        );
    }
    let site = Transform::from_xyz(252., 252., 0.).with_scale(Vec3::new(1.125, 1.125, 1.));
    let plan = app
        .world_mut()
        .spawn((
            PlannedStructure::new(PlannedKind::ProductionFacility, IVec2::ZERO)
                .with_work_remaining(0),
            StructureClearing::awaiting_validation(Vec2::new(180., 252.)),
            OwnerSwarm(swarm),
            site,
        ))
        .id();
    // A new solid occupies the builder's final access position.
    common::spawn_structure_at(&mut app, Vec2::new(180., 252.));
    for _ in 0..100 {
        app.update();
        if app.world().get_entity(plan).is_err() {
            break;
        }
    }
    assert!(
        app.world().get_entity(plan).is_err(),
        "unsafe final access cancels the facility"
    );
    assert_eq!(
        app.world()
            .resource::<ProductionPressure>()
            .ticks_for(SwarmId::PLAYER),
        0
    );
    assert_eq!(
        app.world_mut()
            .query::<&ProductionFacility>()
            .iter(app.world())
            .count(),
        0
    );
    assert_eq!(
        app.world_mut()
            .query::<&CancelledPlanVisual>()
            .iter(app.world())
            .count(),
        1
    );
    let layout = app
        .world_mut()
        .run_system_once(|access: ConstructionAccess| access.snapshot())
        .unwrap();
    assert!(app.world().resource::<CancelledSites>().excludes(
        &layout,
        SwarmId::PLAYER,
        PlannedKind::ProductionFacility,
        &site,
    ));
    let physical = app
        .world_mut()
        .run_system_once(|world: PhysicalWorld| world.snapshot())
        .unwrap();
    assert!(
        physical.can_occupy(Vec2::new(252., 252.)),
        "the fading outline reserves no space"
    );
    for elapsed in 1..60 {
        app.update();
        assert_eq!(
            app.world()
                .resource::<ProductionPressure>()
                .ticks_for(SwarmId::PLAYER),
            elapsed
        );
        assert_eq!(
            facility_plans(app.world_mut()).len(),
            0,
            "fresh pressure must accumulate before replacement"
        );
    }
    let mut replacement = None;
    for _ in 0..160 {
        app.update();
        let plans = facility_plans(app.world_mut());
        assert!(
            plans.len() <= 1,
            "one unfinished capacity commitment per swarm"
        );
        if let Some(&(entity, position)) = plans.first() {
            assert!(
                position.distance(site.translation.truncate()) > 1.,
                "replacement must choose another site"
            );
            if let Some(first) = replacement {
                assert_eq!(
                    entity, first,
                    "continued demand preserves its replacement commitment"
                );
            }
            replacement = Some(entity);
        }
    }
    assert!(
        replacement.is_some(),
        "continued demand eventually places a valid replacement"
    );
}

fn facility_plans(world: &mut World) -> Vec<(Entity, Vec2)> {
    world
        .query::<(Entity, &PlannedStructure, &Transform)>()
        .iter(world)
        .filter(|(_, plan, _)| plan.kind == PlannedKind::ProductionFacility)
        .map(|(entity, _, transform)| (entity, transform.translation.truncate()))
        .collect()
}

#[test]
fn worker_finishing_an_unsafe_plan_is_released_in_the_cancellation_tick() {
    use top_down_2d_rts_prototype_nano_swarm::nanobot::{
        PlannedStructureClaim, PlannedStructureProgress,
    };
    let mut app = common::sim_app_with_planned();
    let plan = common::spawn_planned_structure_at_cell(&mut app, IVec2::ZERO);
    let site = Transform::from_xyz(252., 252., 0.).with_scale(Vec3::new(1.125, 1.125, 1.));
    app.world_mut().entity_mut(plan).insert(site);
    let worker = common::spawn_worker_at(&mut app, Vec2::new(180., 252.));
    {
        let mut state = app.world_mut().get_mut::<PlannedStructure>(plan).unwrap();
        *state = state.with_work_remaining(1);
        assert!(state.try_claim(worker));
    }
    app.world_mut().entity_mut(worker).insert((
        PlannedStructureClaim {
            cell: IVec2::ZERO,
            target: plan,
        },
        PlannedStructureProgress {
            cell: IVec2::ZERO,
            target: plan,
        },
    ));
    // A blocker appears after placement and invalidates the combined layout.
    common::spawn_structure_at(&mut app, Vec2::new(252., 252.));
    for _ in 0..3 {
        app.update();
        if app.world().get_entity(plan).is_err() {
            break;
        }
    }
    assert!(app.world().get_entity(plan).is_err());
    assert!(app.world().get::<PlannedStructureClaim>(worker).is_none());
    assert!(
        app.world()
            .get::<PlannedStructureProgress>(worker)
            .is_none()
    );
    assert_eq!(
        app.world_mut()
            .query::<&CancelledPlanVisual>()
            .iter(app.world())
            .count(),
        1
    );
}
