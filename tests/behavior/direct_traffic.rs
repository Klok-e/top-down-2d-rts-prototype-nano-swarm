use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    intent::{IntentGrid, IntentKind},
    nanobot::{
        Cargo, DirectMovementComponent, InteractionRegion, SwarmId, SwarmMember, WorkApproach,
    },
    navigation::Navigation,
    navigation_runtime::NavigationBudget,
    resources::ResourceKind,
};

#[path = "../common/mod.rs"]
mod common;

const LANE_Y: f32 = 108.0;
const START: Vec2 = Vec2::new(180.0, LANE_Y);

fn counterflow(app: &mut App, swarm: SwarmId) -> [Entity; 2] {
    [660.0, 680.0].map(|x| {
        let bot = common::spawn_worker_at(app, Vec2::new(x, LANE_Y));
        app.world_mut().entity_mut(bot).insert((
            SwarmMember(swarm),
            DirectMovementComponent {
                xy: Vec2::new(230.0, LANE_Y),
                stop_radius: 0.0,
                speed: None,
                interaction: None,
            },
        ));
        bot
    })
}

fn position(app: &App, bot: Entity) -> Vec2 {
    app.world()
        .get::<Transform>(bot)
        .unwrap()
        .translation
        .truncate()
}

fn work_site(app: &mut App, x: f32) -> InteractionRegion {
    let site = common::spawn_structure_at(app, Vec2::new(x, LANE_Y));
    InteractionRegion::structure(app.world().get::<Transform>(site).unwrap())
}

#[test]
fn new_worker_follows_a_quieter_bend_past_counterflow_and_keeps_its_work_goal() {
    for (swarm, cancel_before_choice, should_bend) in [
        (SwarmId::PLAYER, false, true),
        (SwarmId::PLAYER, true, false),
        (SwarmId(7), false, false),
    ] {
        let mut app = common::sim_app_with_movement();
        let region = work_site(&mut app, 900.0);
        let traffic = counterflow(&mut app, swarm);
        for _ in 0..2 {
            app.update();
        }
        assert!(
            position(&app, traffic[0]).x < 660.0,
            "counterflow must be underway"
        );

        if cancel_before_choice {
            for bot in traffic {
                app.world_mut()
                    .entity_mut(bot)
                    .remove::<DirectMovementComponent>();
            }
        }
        common::spawn_structure_at(&mut app, Vec2::new(1300.0, 1100.0));
        let traveller = common::spawn_worker_at(&mut app, START);
        app.world_mut()
            .entity_mut(traveller)
            .insert(region.movement_from(START));
        app.update();
        assert!(
            position(&app, traveller).x > START.x,
            "a legal route starts immediately"
        );
        let approach = *app.world().get::<WorkApproach>(traveller).unwrap();
        for bot in traffic {
            app.world_mut().despawn(bot);
        }

        let mut max_offset = 0.0_f32;
        for _ in 0..240 {
            app.update();
            max_offset = max_offset.max((position(&app, traveller).y - LANE_Y).abs());
            if let Some(current) = app.world().get::<WorkApproach>(traveller) {
                assert_eq!(current.region, region);
                assert_eq!(current.since, approach.since);
            } else {
                break;
            }
        }
        let arrived = position(&app, traveller);
        assert!(
            region.contains(arrived) && (arrived.y - LANE_Y).abs() < 2.0,
            "the Worker completes the turn and reaches the original position: {arrived}"
        );
        assert_eq!(
            max_offset > 60.0,
            should_bend,
            "committed route is chosen from active same-swarm routes at departure: {max_offset}"
        );
    }
}

#[test]
fn painted_hauler_uses_the_only_clear_lane_without_waiting_for_corridor_optimization() {
    for budget in [0, NavigationBudget::default().0] {
        let mut app = common::sim_app_with_movement();
        app.insert_resource(NavigationBudget(budget));
        let region = work_site(&mut app, 900.0);
        for y in [36.0, 180.0] {
            let wall = common::spawn_structure_at(&mut app, Vec2::new(540.0, y));
            app.world_mut().get_mut::<Transform>(wall).unwrap().scale = Vec3::new(16.0, 1.125, 1.0);
        }
        app.world_mut().resource_mut::<IntentGrid>().paint(
            IVec2::Y,
            IntentKind::Corridor,
            SwarmId::PLAYER,
        );
        let traffic = counterflow(&mut app, SwarmId::PLAYER);
        for _ in 0..2 {
            app.update();
        }
        assert!(
            position(&app, traffic[0]).x < 660.0,
            "counterflow must be underway"
        );
        let traveller = common::spawn_hauler_at(&mut app, START);
        app.world_mut()
            .entity_mut(traveller)
            .insert(region.movement_from(START));
        app.update();
        assert!(
            position(&app, traveller).x > START.x,
            "the clear lane starts immediately with budget {budget}"
        );

        for _ in 0..240 {
            app.update();
            let current = position(&app, traveller);
            assert!(
                (current.y - LANE_Y).abs() <= 2.001,
                "Hauler left the only lane: {current}"
            );
            if region.contains(current) {
                break;
            }
        }
        assert!(
            region.contains(position(&app, traveller)),
            "the Hauler arrives through counterflow with budget {budget}"
        );
    }
}

#[test]
fn direct_range_approach_follows_a_bend_to_a_clear_point_before_the_target_center() {
    let mut app = common::sim_app_with_movement();
    let center = Vec2::new(900.0, LANE_Y);
    work_site(&mut app, center.x);
    counterflow(&mut app, SwarmId::PLAYER);
    for _ in 0..2 {
        app.update();
    }
    let traveller = common::spawn_worker_at(&mut app, START);
    let range = 96.0;
    app.world_mut()
        .entity_mut(traveller)
        .insert(DirectMovementComponent {
            xy: center,
            stop_radius: range,
            speed: None,
            interaction: None,
        });
    app.update();
    assert!(position(&app, traveller).x > START.x);
    let mut max_offset = 0.0_f32;
    for _ in 0..240 {
        app.update();
        max_offset = max_offset.max((position(&app, traveller).y - LANE_Y).abs());
        if app
            .world()
            .get::<DirectMovementComponent>(traveller)
            .is_none()
        {
            break;
        }
    }
    let arrived = position(&app, traveller);
    assert!(
        max_offset > 60.0,
        "the selected bend was skipped: {max_offset}"
    );
    assert!((arrived.distance(center) - range).abs() < 5.0, "{arrived}");
}

#[test]
fn nearby_loaded_hauler_starts_unloading_approach_without_navigation_budget() {
    let mut app = common::sim_app_with_movement();
    app.insert_resource(NavigationBudget(0));
    let region = work_site(&mut app, 380.0);
    assert!(START.distance(region.approach(START)) <= 144.0);
    let hauler = common::spawn_hauler_at(&mut app, START);
    app.world_mut().entity_mut(hauler).insert((
        Cargo {
            kind: ResourceKind::Minerals,
            amount: 7,
        },
        region.movement_from(START),
    ));
    app.world_mut().resource_mut::<IntentGrid>().paint(
        IVec2::Y,
        IntentKind::Corridor,
        SwarmId::PLAYER,
    );

    app.update();

    assert!(
        position(&app, hauler).x > START.x,
        "the direct approach must begin on the first tick"
    );
    assert_eq!(app.world().resource::<Navigation>().work().work, 0);
    for _ in 0..60 {
        app.update();
        if app.world().get::<DirectMovementComponent>(hauler).is_none() {
            break;
        }
    }
    assert!(region.contains(position(&app, hauler)));
    assert_eq!(app.world().get::<Cargo>(hauler).unwrap().amount, 7);
}

#[test]
fn reopened_local_approach_preempts_a_pending_replacement_at_zero_budget() {
    let mut app = common::sim_app_with_movement();
    let region = work_site(&mut app, 900.0);
    let hauler = common::spawn_hauler_at(&mut app, START);
    app.world_mut().entity_mut(hauler).insert((
        Cargo {
            kind: ResourceKind::Minerals,
            amount: 7,
        },
        region.movement_from(START),
    ));
    for _ in 0..150 {
        app.update();
        if position(&app, hauler).x >= 580.0 {
            break;
        }
    }
    assert!(position(&app, hauler).x >= 580.0);
    assert!(position(&app, hauler).distance(region.approach(position(&app, hauler))) > 144.0);

    app.insert_resource(NavigationBudget(0));
    let blocker = common::spawn_structure_at(&mut app, Vec2::new(780.0, LANE_Y));
    for _ in 0..120 {
        app.update();
    }
    let blocked = position(&app, hauler);
    assert!(
        (680.0..715.0).contains(&blocked.x),
        "the Hauler reached the safe prefix: {blocked}"
    );
    assert!(
        app.world()
            .resource::<Navigation>()
            .pending_movement_ticks(hauler)
            .is_some(),
        "the obstructed route must be waiting for a replacement",
    );

    app.world_mut().despawn(blocker);
    app.update();
    assert!(
        position(&app, hauler).x > blocked.x + 0.1,
        "the local approach resumes on the first tick without a replacement result",
    );
    for _ in 0..120 {
        app.update();
        if app.world().get::<DirectMovementComponent>(hauler).is_none() {
            break;
        }
    }
    assert!(region.contains(position(&app, hauler)));
    assert_eq!(app.world().resource::<Navigation>().work().work, 0);
}
