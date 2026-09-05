use bevy::{ecs::system::RunSystemOnce, prelude::*};
use top_down_2d_rts_prototype_nano_swarm::{
    nanobot::{PlannedKind, PlannedStructure, StructureClearing},
    navigation::Navigation,
    physical_world::{PhysicalGeometry, PhysicalWorld},
    resources::Stockpile,
};
#[path = "../common/mod.rs"]
mod common;

fn geometry(world: PhysicalWorld) -> PhysicalGeometry {
    world.snapshot()
}

#[test]
fn pending_and_clearing_have_distinct_entry_exit_and_spawn_permissions() {
    let mut app = common::minimal_app();
    let plan = app
        .world_mut()
        .spawn((
            PlannedStructure::new(PlannedKind::SinkStockpile, IVec2::ZERO).with_work_remaining(0),
            Transform::from_xyz(36.0, 36.0, 0.0).with_scale(Vec3::new(1.125, 1.125, 1.0)),
            StructureClearing::awaiting_validation(Vec2::new(-36.0, 36.0)),
        ))
        .id();
    let pending = app.world_mut().run_system_once(geometry).unwrap();
    assert!(pending.can_occupy(Vec2::new(36.0, 36.0)));
    assert!(pending.movement_clear(Vec2::new(-36.0, 36.0), Vec2::new(108.0, 36.0)));

    app.world_mut()
        .entity_mut(plan)
        .insert(StructureClearing::validated(Vec2::new(-36.0, 36.0), 7));
    let clearing = app.world_mut().run_system_once(geometry).unwrap();
    assert!(
        !clearing.can_occupy(Vec2::new(36.0, 36.0)),
        "a retained output cannot spawn inside clearing"
    );
    assert!(
        !clearing.movement_clear(Vec2::new(-36.0, 36.0), Vec2::new(36.0, 36.0)),
        "new entrants must stop"
    );
    assert!(
        clearing.movement_clear(Vec2::new(36.0, 36.0), Vec2::new(-36.0, 36.0)),
        "an existing occupant can leave"
    );
    assert!(
        clearing.can_occupy(Vec2::new(-36.0, 36.0)),
        "the exterior keeps literal 36-unit surface clearance"
    );
}

#[test]
fn activation_publishes_collision_to_routes_and_body_placement_in_the_same_tick() {
    let mut app = common::sim_app_with_planned();
    common::spawn_swarm_at(&mut app, Vec2::ZERO);
    common::spawn_worker_at(&mut app, Vec2::new(-36.0, 36.0));
    let plan = app
        .world_mut()
        .spawn((
            PlannedStructure::new(PlannedKind::SinkStockpile, IVec2::ZERO).with_work_remaining(0),
            Transform::from_xyz(36.0, 36.0, 0.0).with_scale(Vec3::new(1.125, 1.125, 1.0)),
            StructureClearing::awaiting_validation(Vec2::new(-36.0, 36.0)),
        ))
        .id();
    for _ in 0..100 {
        app.update();
        if app.world().get::<Stockpile>(plan).is_some() {
            let physical = app.world_mut().run_system_once(geometry).unwrap();
            assert!(!physical.can_occupy(Vec2::new(36.0, 36.0)));
            assert!(!physical.movement_clear(Vec2::new(36.0, 36.0), Vec2::new(-36.0, 36.0)));
            assert!(
                !app.world()
                    .resource::<Navigation>()
                    .point_clear(Vec2::new(36.0, 36.0)),
                "new operational capacity and route collision must be visible together"
            );
            return;
        }
    }
    panic!("clear, reachable site did not activate within its work allowance");
}

#[test]
fn changed_layout_keeps_entry_barred_while_revalidation_has_no_budget() {
    use top_down_2d_rts_prototype_nano_swarm::{
        nanobot::Structure, navigation_runtime::NavigationBudget,
    };
    let mut app = common::sim_app_with_planned();
    common::spawn_swarm_at(&mut app, Vec2::ZERO);
    common::spawn_worker_at(&mut app, Vec2::new(-36.0, 36.0));
    common::spawn_defender_at(&mut app, Vec2::new(36.0, 36.0));
    let plan = app
        .world_mut()
        .spawn((
            PlannedStructure::new(PlannedKind::SinkStockpile, IVec2::ZERO).with_work_remaining(0),
            Transform::from_xyz(36.0, 36.0, 0.0).with_scale(Vec3::new(1.125, 1.125, 1.0)),
            StructureClearing::validated(Vec2::new(-36.0, 36.0), 7),
        ))
        .id();
    app.world_mut().resource_mut::<NavigationBudget>().0 = 0;
    app.world_mut()
        .spawn((Structure::default(), Transform::from_xyz(540.0, 540.0, 0.0)));
    for _ in 0..4 {
        app.update();
        assert!(app.world().get::<PlannedStructure>(plan).is_some());
        assert!(app.world().get::<Stockpile>(plan).is_none());
        let physical = app.world_mut().run_system_once(geometry).unwrap();
        assert!(!physical.can_occupy(Vec2::new(36.0, 36.0)));
        assert!(!physical.movement_clear(Vec2::new(-36.0, 36.0), Vec2::new(36.0, 36.0)));
        assert!(physical.movement_clear(Vec2::new(36.0, 36.0), Vec2::new(-36.0, 36.0)));
    }
}
