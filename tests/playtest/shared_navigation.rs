//! Every movement role uses the same physical route and swept-body clearance.
use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::nanobot::{
    DirectMovementComponent, NanobotType, SwarmId, SwarmMember,
};
#[path = "../common/mod.rs"]
mod common;

#[test]
fn both_swarms_and_all_types_detour_around_completed_structure() {
    for swarm in [SwarmId::PLAYER, SwarmId(42)] {
        for kind in [
            NanobotType::Worker,
            NanobotType::Hauler,
            NanobotType::Defender,
        ] {
            for blocker_kind in ["stockpile", "charger", "facility", "deposit"] {
                let mut app = common::sim_app_with_movement();
                let owner = common::spawn_swarm_at(&mut app, Vec2::new(900.0, 900.0));
                let obstacle = match blocker_kind {
                    "stockpile" => common::spawn_stockpile(&mut app, Vec2::ZERO, 0, 100),
                    "charger" => common::spawn_charger(
                        &mut app,
                        common::ChargerFixture {
                            cell: IVec2::ZERO,
                            amount: 0,
                            ticks_since_maintained: 0,
                        },
                    ),
                    "deposit" => common::spawn_deposit(
                        &mut app,
                        common::DepositFixture {
                            world_pos: Vec2::ZERO,
                            amount: 20,
                            capacity: 20,
                            radius: 216.0,
                        },
                    ),
                    _ => common::spawn_idle_facility_at(&mut app, Vec2::ZERO),
                };
                app.world_mut().entity_mut(obstacle).insert(
                    top_down_2d_rts_prototype_nano_swarm::nanobot::OwnerSwarm(owner),
                );
                app.world_mut()
                    .get_mut::<Transform>(obstacle)
                    .unwrap()
                    .translation = Vec3::ZERO;
                app.world_mut()
                    .get_mut::<Transform>(obstacle)
                    .unwrap()
                    .scale = Vec3::new(2.25, 6.75, 1.0);
                let bot = common::spawn_worker_at(&mut app, Vec2::new(-300.0, 0.0));
                app.world_mut().entity_mut(bot).insert((
                    kind,
                    SwarmMember::new(swarm),
                    DirectMovementComponent {
                        speed: None,
                        interaction: None,
                        xy: Vec2::new(300.0, 0.0),
                        stop_radius: 2.0,
                    },
                ));
                let mut previous = Vec2::new(-300.0, 0.0);
                let mut detoured = false;
                for _ in 0..300 {
                    app.update();
                    let position = app
                        .world()
                        .get::<Transform>(bot)
                        .unwrap()
                        .translation
                        .truncate();
                    // Independently sample each short movement segment against the literal rectangle.
                    for sample in 0..=10 {
                        let point = previous.lerp(position, sample as f32 / 10.0);
                        let clearance = if blocker_kind == "deposit" {
                            point.length() - 216.0
                        } else {
                            ((point.abs() - Vec2::new(72.0, 216.0)).max(Vec2::ZERO)).length()
                        };
                        assert!(
                            clearance >= 33.999,
                            "{swarm:?}/{kind:?} crossed solid: {point:?}"
                        );
                    }
                    detoured |= position.y.abs() > 249.0;
                    previous = position;
                    if position.distance(Vec2::new(300.0, 0.0)) <= 2.01 {
                        break;
                    }
                }
                assert!(detoured, "{swarm:?}/{kind:?} must travel around the wall");
                assert!(
                    previous.distance(Vec2::new(300.0, 0.0)) <= 2.01,
                    "{swarm:?}/{kind:?}/{blocker_kind} stopped at {previous:?}"
                );
            }
        }
    }
}

#[test]
fn existing_deposit_blocks_after_depletion_and_removal_wakes_waiting_route() {
    use top_down_2d_rts_prototype_nano_swarm::{intent::IntentGrid, resources::ResourceDeposit};
    let mut app = common::sim_app_with_movement();
    app.world_mut().insert_resource(IntentGrid::new(2, 2));
    // A circular deposit covers the entire map height, separating left and right.
    let deposit = common::spawn_deposit(
        &mut app,
        common::DepositFixture {
            world_pos: Vec2::ZERO,
            amount: 1,
            capacity: 1,
            radius: 480.0,
        },
    );
    let bot = common::spawn_worker_at(&mut app, Vec2::new(-500.0, -500.0));
    let goal = Vec2::new(500.0, 500.0);
    app.world_mut()
        .entity_mut(bot)
        .insert(DirectMovementComponent {
            speed: None,
            interaction: None,
            xy: goal,
            stop_radius: 2.0,
        });
    for _ in 0..5 {
        app.update();
    }
    let before = app
        .world()
        .get::<Transform>(bot)
        .unwrap()
        .translation
        .truncate();
    assert!(before.distance(Vec2::new(-500.0, -500.0)) < 0.001);
    app.world_mut()
        .get_mut::<ResourceDeposit>(deposit)
        .unwrap()
        .amount = 0;
    for _ in 0..5 {
        app.update();
    }
    let depleted = app
        .world()
        .get::<Transform>(bot)
        .unwrap()
        .translation
        .truncate();
    assert!(
        depleted.distance(before) < 0.001,
        "depletion must not open a route"
    );
    app.world_mut().despawn(deposit);
    for _ in 0..300 {
        app.update();
    }
    let arrived = app
        .world()
        .get::<Transform>(bot)
        .unwrap()
        .translation
        .truncate();
    assert!(
        arrived.distance(goal) <= 2.01,
        "removal must reopen the route: {arrived:?}"
    );
}

#[test]
fn worker_gathers_from_reachable_side_when_nearest_deposit_face_is_blocked() {
    use top_down_2d_rts_prototype_nano_swarm::{
        intent::{IntentGrid, IntentKind},
        resources::ResourceDeposit,
    };
    let mut app = common::sim_app_with_gather();
    app.world_mut().resource_mut::<IntentGrid>().paint(
        IVec2::ZERO,
        IntentKind::Gather,
        top_down_2d_rts_prototype_nano_swarm::nanobot::SwarmId::PLAYER,
    );
    let deposit = common::spawn_deposit(
        &mut app,
        common::DepositFixture {
            world_pos: Vec2::new(140.0, 140.0),
            amount: 8,
            capacity: 8,
            radius: 48.0,
        },
    );
    let source = common::spawn_stockpile(&mut app, Vec2::new(400.0, 140.0), 0, 100);
    app.world_mut()
        .entity_mut(source)
        .insert(top_down_2d_rts_prototype_nano_swarm::resources::StockpileRole::Source);
    let wall = common::spawn_structure_at(&mut app, Vec2::new(20.0, 140.0));
    app.world_mut().get_mut::<Transform>(wall).unwrap().scale = Vec3::splat(1.125);
    let bot = common::spawn_worker_at(&mut app, Vec2::new(-180.0, 140.0));
    for _ in 0..200 {
        app.update();
        let pos = app
            .world()
            .get::<Transform>(bot)
            .unwrap()
            .translation
            .truncate();
        let rectangle_distance = ((pos - Vec2::new(20.0, 140.0)).abs() - Vec2::splat(36.0))
            .max(Vec2::ZERO)
            .length();
        assert!(rectangle_distance >= 33.999 && pos.distance(Vec2::new(140.0, 140.0)) >= 81.999);
        if app.world().get::<ResourceDeposit>(deposit).unwrap().amount < 8 {
            return;
        }
    }
    panic!(
        "reachable exterior work failed: position={:?} goal={:?} assignment={:?}",
        app.world().get::<Transform>(bot),
        app.world().get::<DirectMovementComponent>(bot),
        app.world()
            .get::<top_down_2d_rts_prototype_nano_swarm::nanobot::GatherAssignment>(bot)
    );
}

#[test]
fn planned_structure_allows_crossing_but_completion_replans_before_entry() {
    use top_down_2d_rts_prototype_nano_swarm::nanobot::{
        PlannedStructure, Structure, StructureKind,
    };
    let mut app = common::sim_app_with_movement();
    let plan = common::spawn_planned_structure_at_cell(&mut app, IVec2::ZERO);
    app.world_mut()
        .get_mut::<Transform>(plan)
        .unwrap()
        .translation = Vec3::ZERO;
    app.world_mut().get_mut::<Transform>(plan).unwrap().scale = Vec3::new(2.25, 6.75, 1.0);
    let bot = common::spawn_worker_at(&mut app, Vec2::new(-200.0, 0.0));
    app.world_mut()
        .entity_mut(bot)
        .insert(DirectMovementComponent {
            xy: Vec2::new(200.0, 0.0),
            stop_radius: 2.0,
            interaction: None,
            speed: None,
        });
    for _ in 0..85 {
        app.update();
    }
    let pos = app
        .world()
        .get::<Transform>(bot)
        .unwrap()
        .translation
        .truncate();
    assert!(
        pos.distance(Vec2::new(200.0, 0.0)) <= 2.01,
        "planned footprint must allow direct crossing"
    );
    app.world_mut()
        .entity_mut(bot)
        .insert(DirectMovementComponent {
            xy: Vec2::new(-200.0, 0.0),
            stop_radius: 2.0,
            interaction: None,
            speed: None,
        });
    for _ in 0..5 {
        app.update();
    }
    app.world_mut()
        .entity_mut(plan)
        .remove::<PlannedStructure>()
        .insert(Structure::new(StructureKind::Basic));
    let mut previous = app
        .world()
        .get::<Transform>(bot)
        .unwrap()
        .translation
        .truncate();
    for _ in 0..220 {
        app.update();
        let pos = app
            .world()
            .get::<Transform>(bot)
            .unwrap()
            .translation
            .truncate();
        for step in 0..=10 {
            let point = previous.lerp(pos, step as f32 / 10.0);
            assert!(
                (point.abs() - Vec2::new(72.0, 216.0))
                    .max(Vec2::ZERO)
                    .length()
                    >= 33.999
            );
        }
        previous = pos;
        if pos.distance(Vec2::new(-200.0, 0.0)) <= 2.01 {
            return;
        }
    }
    panic!("completion should replan the active route: {previous:?}");
}

#[test]
fn ranged_pursuit_uses_an_accessible_side_of_the_attack_region() {
    let mut app = common::sim_app_with_movement();
    common::spawn_deposit(
        &mut app,
        common::DepositFixture {
            world_pos: Vec2::ZERO,
            amount: 1,
            capacity: 1,
            radius: 16.0,
        },
    );
    let wall = common::spawn_structure_at(&mut app, Vec2::new(-108.0, 0.0));
    app.world_mut().get_mut::<Transform>(wall).unwrap().scale = Vec3::splat(1.125);
    let defender = common::spawn_defender_at(&mut app, Vec2::new(-300.0, 0.0));
    app.world_mut()
        .entity_mut(defender)
        .insert(DirectMovementComponent {
            xy: Vec2::ZERO,
            stop_radius: 96.0,
            interaction: None,
            speed: None,
        });
    for _ in 0..150 {
        app.update();
        let point = app
            .world()
            .get::<Transform>(defender)
            .unwrap()
            .translation
            .truncate();
        assert!(point.length() >= 49.999);
        assert!(
            ((point - Vec2::new(-108.0, 0.0)).abs() - Vec2::splat(36.0))
                .max(Vec2::ZERO)
                .length()
                >= 33.999
        );
        if point.length() <= 96.0 {
            return;
        }
    }
    panic!("reachable attack region should be approached from its open side");
}

#[test]
fn staging_keeps_roaming_through_free_edge_cells_around_an_occupied_center() {
    use top_down_2d_rts_prototype_nano_swarm::intent::{IntentGrid, IntentKind};
    let mut app = common::sim_app();
    app.world_mut().resource_mut::<IntentGrid>().paint(
        IVec2::ZERO,
        IntentKind::Defend,
        SwarmId::PLAYER,
    );
    let solid = common::spawn_structure_at(&mut app, Vec2::splat(256.0));
    app.world_mut().get_mut::<Transform>(solid).unwrap().scale = Vec3::new(5.625, 5.625, 1.0);
    let defender = common::spawn_defender_at(&mut app, Vec2::splat(20.0));
    let mut earlier = Vec2::ZERO;
    for tick in 0..400 {
        app.update();
        let position = app
            .world()
            .get::<Transform>(defender)
            .unwrap()
            .translation
            .truncate();
        assert!(
            ((position - Vec2::splat(256.0)).abs() - Vec2::splat(180.0))
                .max(Vec2::ZERO)
                .length()
                >= 33.999
        );
        assert_eq!(
            top_down_2d_rts_prototype_nano_swarm::nanobot::world_to_cell(position),
            IVec2::ZERO
        );
        if tick == 300 {
            earlier = position;
        }
    }
    let final_position = app
        .world()
        .get::<Transform>(defender)
        .unwrap()
        .translation
        .truncate();
    assert!(
        final_position.distance(earlier) > 5.0,
        "a blocked central roaming area must not park Defenders: {earlier:?} -> {final_position:?}"
    );
}

#[test]
fn corridor_edits_preserve_active_travel_and_guide_the_next_leg() {
    use top_down_2d_rts_prototype_nano_swarm::{
        intent::{IntentGrid, IntentKind},
        navigation_runtime::NavigationBudget,
    };
    let mut app = common::sim_app_with_movement();
    app.world_mut().insert_resource(IntentGrid::new(6, 6));
    let bot = common::spawn_worker_at(&mut app, Vec2::new(-900.0, 400.0));
    app.world_mut().entity_mut(bot).insert((
        NanobotType::Hauler,
        SwarmMember::new(SwarmId::PLAYER),
        DirectMovementComponent {
            xy: Vec2::new(900.0, 400.0),
            stop_radius: 0.0,
            interaction: None,
            speed: Some(10.0),
        },
    ));
    for _ in 0..1000 {
        app.update();
        if app.world().get::<Transform>(bot).unwrap().translation.x > -890.0 {
            break;
        }
    }
    let before = app.world().get::<Transform>(bot).unwrap().translation;
    assert!(before.x > -890.0 && before.x < 0.0);
    for x in -2..=1 {
        app.world_mut().resource_mut::<IntentGrid>().paint(
            IVec2::new(x, 1),
            IntentKind::Corridor,
            SwarmId::PLAYER,
        );
    }
    app.world_mut().resource_mut::<NavigationBudget>().0 = 0;
    for _ in 0..500 {
        app.update();
        let position = app.world().get::<Transform>(bot).unwrap().translation;
        if app.world().get::<DirectMovementComponent>(bot).is_none() {
            break;
        }
        assert!(
            (position.y - 400.0).abs() < 0.001,
            "paint changed the active leg: {position:?}"
        );
    }
    let arrived = app
        .world()
        .get::<Transform>(bot)
        .unwrap()
        .translation
        .truncate();
    assert!(
        arrived.distance(Vec2::new(900.0, 400.0)) < 3.0,
        "first leg stopped at {arrived:?}"
    );
    app.world_mut()
        .entity_mut(bot)
        .insert(DirectMovementComponent {
            xy: Vec2::new(-900.0, 400.0),
            stop_radius: 0.0,
            interaction: None,
            speed: Some(10.0),
        });
    app.world_mut().resource_mut::<NavigationBudget>().0 = 32_768;
    let mut used_corridor = false;
    for _ in 0..1000 {
        app.update();
        let position = app.world().get::<Transform>(bot).unwrap().translation;
        used_corridor |= position.y >= 512.0;
        if app.world().get::<DirectMovementComponent>(bot).is_none() {
            break;
        }
    }
    assert!(
        used_corridor,
        "the next leg ignored new owned Corridor paint"
    );
    let returned = app
        .world()
        .get::<Transform>(bot)
        .unwrap()
        .translation
        .truncate();
    assert!(
        returned.distance(Vec2::new(-900.0, 400.0)) < 3.0,
        "return leg stopped at {returned:?}"
    );
}

#[test]
fn unrelated_obstacle_removal_preserves_travel_with_search_budget_stopped() {
    use top_down_2d_rts_prototype_nano_swarm::navigation_runtime::NavigationBudget;
    let mut app = common::sim_app_with_movement();
    let obstacle = common::spawn_structure_at(&mut app, Vec2::new(400.0, 800.0));
    let bot = common::spawn_worker_at(&mut app, Vec2::new(200.0, 300.0));
    app.world_mut()
        .entity_mut(bot)
        .insert(DirectMovementComponent {
            xy: Vec2::new(600.0, 300.0),
            stop_radius: 0.0,
            interaction: None,
            speed: Some(5.0),
        });
    for _ in 0..10 {
        app.update();
    }
    let before = app.world().get::<Transform>(bot).unwrap().translation;
    assert!(before.x > 200.0 && before.x < 600.0);
    app.world_mut().despawn(obstacle);
    app.world_mut().resource_mut::<NavigationBudget>().0 = 0;
    for _ in 0..100 {
        app.update();
    }
    let arrived = app.world().get::<Transform>(bot).unwrap().translation;
    assert!(
        arrived.truncate().distance(Vec2::new(600.0, 300.0)) < 3.0,
        "unrelated removal made valid travel wait for another search: {arrived:?}"
    );
}
