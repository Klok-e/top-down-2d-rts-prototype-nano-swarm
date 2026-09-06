//! Separate work positions and stable waiting priority at crowded goals.
use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    nanobot::{Cargo, Commitment, DirectMovementComponent, InteractionRegion, WaitingForWork},
    resources::ResourceKind,
};

#[path = "../common/mod.rs"]
mod common;

#[test]
fn oldest_loaded_waiter_gets_open_work_position_before_closer_newcomer() {
    let mut app = common::sim_app_with_movement();
    let center = Vec2::new(400.0, 0.0);
    let site = Transform::from_translation(center.extend(0.0));
    let region = InteractionRegion::structure(&site);
    let mut east_blocker = None;
    for offset in [
        Vec2::new(-68.0, -68.0),
        Vec2::new(-68.0, 0.0),
        Vec2::new(-68.0, 68.0),
        Vec2::new(0.0, -68.0),
        Vec2::new(0.0, 68.0),
        Vec2::new(68.0, -68.0),
        Vec2::new(68.0, 0.0),
        Vec2::new(68.0, 68.0),
    ] {
        let blocker = common::spawn_worker_at(&mut app, center + offset);
        app.world_mut()
            .entity_mut(blocker)
            .insert(Commitment::Working);
        if offset == Vec2::new(68.0, 0.0) {
            east_blocker = Some(blocker);
        }
    }
    let oldest = common::spawn_hauler_at(&mut app, Vec2::new(590.0, 0.0));
    app.world_mut().entity_mut(oldest).insert((
        Cargo {
            kind: ResourceKind::Minerals,
            amount: 7,
        },
        region.movement_from(Vec2::new(590.0, 0.0)),
    ));
    for _ in 0..3 {
        app.update();
    }
    assert!(app.world().get::<WaitingForWork>(oldest).is_some());
    let newcomer = common::spawn_hauler_at(&mut app, Vec2::new(570.0, 110.0));
    app.world_mut().entity_mut(newcomer).insert((
        Cargo {
            kind: ResourceKind::Minerals,
            amount: 11,
        },
        region.movement_from(Vec2::new(570.0, 110.0)),
    ));
    for _ in 0..3 {
        app.update();
    }
    // Cooperatively moving aside must preserve the older bot's place.
    app.world_mut()
        .get_mut::<Transform>(oldest)
        .unwrap()
        .translation = Vec3::new(610.0, -20.0, 0.0);
    app.world_mut().despawn(east_blocker.unwrap());
    let mut arrived = None;
    for _ in 0..500 {
        app.update();
        for bot in [oldest, newcomer] {
            if app.world().get::<DirectMovementComponent>(bot).is_none() {
                arrived = Some(bot);
                break;
            }
        }
        if arrived.is_some() {
            break;
        }
    }
    assert_eq!(
        arrived,
        Some(oldest),
        "new arrivals must not overtake an older yielding delivery; oldest {:?}, newcomer {:?}, waiting {:?}",
        app.world().get::<Transform>(oldest).unwrap().translation,
        app.world().get::<Transform>(newcomer).unwrap().translation,
        app.world().get::<WaitingForWork>(oldest)
    );
    assert_eq!(app.world().get::<Cargo>(oldest).unwrap().amount, 7);
    assert_eq!(app.world().get::<Cargo>(newcomer).unwrap().amount, 11);
}

#[test]
fn empty_worker_leaves_saturated_build_site_and_builds_an_alternate_site() {
    use top_down_2d_rts_prototype_nano_swarm::nanobot::{PlannedStructure, PlannedStructureClaim};

    let mut app = common::sim_app_with_planned();
    let crowded = common::spawn_planned_structure_at_cell(&mut app, IVec2::ZERO);
    let alternate = common::spawn_planned_structure_at_cell(&mut app, IVec2::X);
    for site in [crowded, alternate] {
        let plan = *app.world().get::<PlannedStructure>(site).unwrap();
        app.world_mut()
            .entity_mut(site)
            .insert(plan.with_work_remaining(50));
    }
    let center = common::cell_world_center(IVec2::ZERO);
    for x in [-68.0, 0.0, 68.0] {
        for y in [-68.0, 0.0, 68.0] {
            if x == 0.0 && y == 0.0 {
                continue;
            }
            let blocker = common::spawn_hauler_at(&mut app, center + Vec2::new(x, y));
            app.world_mut()
                .entity_mut(blocker)
                .insert(Commitment::Working);
        }
    }
    let start = center + Vec2::new(190.0, 0.0);
    let worker = common::spawn_worker_at(&mut app, start);
    assert!(
        app.world_mut()
            .get_mut::<PlannedStructure>(crowded)
            .unwrap()
            .try_claim(worker)
    );
    let region = InteractionRegion::structure(app.world().get::<Transform>(crowded).unwrap());
    app.world_mut().entity_mut(worker).insert((
        Commitment::Working,
        PlannedStructureClaim {
            target: crowded,
            cell: IVec2::ZERO,
        },
        region.movement_from(start),
    ));

    for _ in 0..300 {
        app.update();
        if app
            .world()
            .get::<PlannedStructure>(alternate)
            .unwrap()
            .available_work()
            < 50
        {
            break;
        }
    }
    assert_eq!(
        app.world()
            .get::<PlannedStructure>(crowded)
            .unwrap()
            .available_work(),
        50
    );
    assert!(
        app.world()
            .get::<PlannedStructure>(alternate)
            .unwrap()
            .available_work()
            < 50,
        "the rejected near site must not prevent useful work at the free alternate site"
    );
}

#[test]
fn distant_loaded_bot_approaches_an_occupied_goal_before_waiting() {
    use bevy::time::TimeUpdateStrategy;
    use std::time::Duration;
    let mut app = common::sim_app_with_movement();
    let cadence = Duration::from_secs_f64(1.0 / 60.0);
    app.insert_resource(Time::<Fixed>::from_duration(cadence));
    app.insert_resource(TimeUpdateStrategy::ManualDuration(cadence));
    let site = common::spawn_structure_at(&mut app, Vec2::new(400.0, 0.0));
    let region = InteractionRegion::structure(app.world().get::<Transform>(site).unwrap());
    for offset in [
        Vec2::new(-68.0, -68.0),
        Vec2::new(-68.0, 0.0),
        Vec2::new(-68.0, 68.0),
        Vec2::new(0.0, -68.0),
        Vec2::new(0.0, 68.0),
        Vec2::new(68.0, -68.0),
        Vec2::new(68.0, 0.0),
        Vec2::new(68.0, 68.0),
    ] {
        let blocker = common::spawn_worker_at(&mut app, Vec2::new(400.0, 0.0) + offset);
        app.world_mut()
            .entity_mut(blocker)
            .insert(Commitment::Working);
    }
    let start = Vec2::new(800.0, 0.0);
    let bot = common::spawn_hauler_at(&mut app, start);
    app.world_mut().entity_mut(bot).insert((
        Cargo {
            kind: ResourceKind::Minerals,
            amount: 20,
        },
        region.movement_from(start),
    ));
    for _ in 0..30 {
        app.update();
    }
    let position = app
        .world()
        .get::<Transform>(bot)
        .unwrap()
        .translation
        .truncate();
    assert!(
        position.x < 700.0,
        "distant cargo must approach even when every standing position is occupied: {position:?}"
    );
    assert!(app.world().get::<WaitingForWork>(bot).is_none());
    for _ in 0..120 {
        app.update();
    }
    assert!(
        app.world().get::<WaitingForWork>(bot).is_some(),
        "a full local destination permits waiting"
    );
    assert_eq!(app.world().get::<Cargo>(bot).unwrap().amount, 20);
}

#[test]
fn clear_final_hauler_approach_moves_without_search_capacity() {
    use top_down_2d_rts_prototype_nano_swarm::navigation_runtime::NavigationBudget;
    let mut app = common::sim_app_with_movement();
    app.insert_resource(NavigationBudget(0));
    let site = common::spawn_structure_at(&mut app, Vec2::new(400.0, 0.0));
    let region = InteractionRegion::structure(app.world().get::<Transform>(site).unwrap());
    let start = Vec2::new(500.0, 0.0);
    let bot = common::spawn_hauler_at(&mut app, start);
    app.world_mut().entity_mut(bot).insert((
        Cargo {
            kind: ResourceKind::Minerals,
            amount: 20,
        },
        region.movement_from(start),
    ));
    app.update();
    assert!(
        app.world().get::<Transform>(bot).unwrap().translation.x < 500.0,
        "a clear final approach must begin on its first movement tick without a graph search"
    );
    for _ in 0..60 {
        app.update();
    }
    assert!(
        app.world().get::<DirectMovementComponent>(bot).is_none(),
        "the direct approach must reach standing space even while global search is unavailable"
    );
    assert_eq!(app.world().get::<Cargo>(bot).unwrap().amount, 20);
}
