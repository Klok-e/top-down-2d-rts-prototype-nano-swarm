//! Congested approaches must not stop physical economy trips.

use bevy::{prelude::*, time::TimeUpdateStrategy};
use std::time::Duration;
use top_down_2d_rts_prototype_nano_swarm::{
    nanobot::{Cargo, Commitment, OwnerSwarm, PlannedStructure, ProductionFacility},
    resources::Stockpile,
};

#[path = "../common/mod.rs"]
mod common;

#[test]
fn hauler_sustains_twenty_deliveries_around_occupied_goal_at_runtime_cadence() {
    let mut app = common::sim_app_with_gather_haul();
    let cadence = Duration::from_secs_f64(1.0 / 60.0);
    app.insert_resource(Time::<Fixed>::from_duration(cadence));
    app.insert_resource(TimeUpdateStrategy::ManualDuration(cadence));
    let swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let source = common::spawn_stockpile(&mut app, Vec2::new(150.0, 0.0), 400, 500);
    let sink = common::spawn_sink_stockpile(&mut app, Vec2::new(400.0, 0.0), 0, 500);
    for endpoint in [source, sink] {
        app.world_mut()
            .entity_mut(endpoint)
            .insert(OwnerSwarm(swarm));
    }
    let mut workers = Vec::new();
    for y in [-68.0, 0.0, 68.0] {
        let position = Vec2::new(332.0, y);
        let worker = common::spawn_worker_at(&mut app, position);
        app.world_mut()
            .entity_mut(worker)
            .insert(Commitment::Working);
        workers.push((worker, position));
    }
    let hauler = common::spawn_hauler_at(&mut app, Vec2::ZERO);
    let mut delivered = 0;
    let mut next_milestone = 20;
    let mut completed_deliveries = 0;
    let mut previous_carried = 0;
    let mut milestone_time = Duration::ZERO;

    while delivered < 400 {
        app.update();
        let now = app.world().resource::<Time<Fixed>>().elapsed();
        let next_delivered = app.world().get::<Stockpile>(sink).unwrap().amount;
        let carried = app
            .world()
            .get::<Cargo>(hauler)
            .map_or(0, |cargo| cargo.amount);
        assert_eq!(
            app.world().get::<Stockpile>(source).unwrap().amount + carried + next_delivered,
            400,
            "twenty deliveries must conserve all physical minerals",
        );
        assert!(
            carried <= 20,
            "each physical trip carries at most twenty minerals"
        );
        if next_delivered > delivered && previous_carried > 0 && carried == 0 {
            completed_deliveries += 1;
        }
        previous_carried = carried;
        let position = app
            .world()
            .get::<Transform>(hauler)
            .unwrap()
            .translation
            .truncate();
        for (worker, original) in &workers {
            let other = app
                .world()
                .get::<Transform>(*worker)
                .unwrap()
                .translation
                .truncate();
            assert!(
                other.distance(*original) < 0.001,
                "sustained traffic preserves active workers"
            );
            if next_delivered > delivered {
                assert!(
                    position.distance(other) >= 67.999,
                    "every delivery needs separate standing space"
                );
            }
        }
        assert!(
            next_delivered >= delivered,
            "delivered cargo must remain at the sink"
        );
        delivered = next_delivered;
        if delivered >= next_milestone {
            next_milestone += 20;
            milestone_time = now;
        }
        assert!(
            now.saturating_sub(milestone_time) < Duration::from_secs(30),
            "sustained hauling stopped before milestone {next_milestone}: delivered {delivered}, cargo {carried}, position {position:?}, simulation time {now:?}",
        );
    }

    assert_eq!(delivered, 400);
    assert_eq!(
        completed_deliveries, 20,
        "twenty separate cargo loads were unloaded"
    );
    assert_eq!(app.world().get::<Stockpile>(source).unwrap().amount, 0);
    assert!(app.world().get::<Cargo>(hauler).is_none());
}

#[test]
fn hauler_repeatedly_delivers_around_occupied_terminal_approach() {
    for (cadence, max_updates) in [
        (Duration::from_millis(100), 1200),
        (Duration::from_secs_f64(1.0 / 60.0), 7200),
    ] {
        let mut app = common::sim_app_with_gather_haul();
        app.insert_resource(Time::<Fixed>::from_duration(cadence));
        app.insert_resource(TimeUpdateStrategy::ManualDuration(cadence));
        let swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
        let source = common::spawn_sink_stockpile(&mut app, Vec2::new(150.0, 0.0), 40, 100);
        app.world_mut().entity_mut(source).insert(OwnerSwarm(swarm));
        let terminal = common::spawn_facility_at(&mut app, swarm, Vec2::new(400.0, 0.0));
        app.world_mut()
            .get_mut::<ProductionFacility>(terminal)
            .unwrap()
            .input_amount = 0;
        let mut working_bots = Vec::new();
        for y in [-68.0, 0.0, 68.0] {
            let working_bot = common::spawn_worker_at(&mut app, Vec2::new(332.0, y));
            app.world_mut()
                .entity_mut(working_bot)
                .insert(Commitment::Working);
            working_bots.push(working_bot);
        }
        let hauler = common::spawn_hauler_at(&mut app, Vec2::ZERO);

        let mut delivered = 0;
        for _ in 0..max_updates {
            app.update();
            let next_delivered = app
                .world()
                .get::<ProductionFacility>(terminal)
                .unwrap()
                .input_amount;
            let cargo = app
                .world()
                .get::<Cargo>(hauler)
                .map_or(0, |cargo| cargo.amount);
            assert_eq!(
                app.world().get::<Stockpile>(source).unwrap().amount + cargo + next_delivered,
                40,
                "congestion must conserve all physical minerals",
            );
            if next_delivered > delivered {
                let position = app
                    .world()
                    .get::<Transform>(hauler)
                    .unwrap()
                    .translation
                    .truncate();
                for bot in &working_bots {
                    let other = app
                        .world()
                        .get::<Transform>(*bot)
                        .unwrap()
                        .translation
                        .truncate();
                    assert!(
                        position.distance(other) >= 67.999,
                        "unloading requires separate standing space"
                    );
                }
            }
            delivered = next_delivered;
            for (bot, y) in working_bots.iter().zip([-68.0, 0.0, 68.0]) {
                let position = app
                    .world()
                    .get::<Transform>(*bot)
                    .unwrap()
                    .translation
                    .truncate();
                assert!(
                    position.distance(Vec2::new(332.0, y)) < 0.001,
                    "unloading traffic must preserve active workers' positions"
                );
            }
            if app
                .world()
                .get::<ProductionFacility>(terminal)
                .unwrap()
                .input_amount
                == 40
            {
                break;
            }
        }

        assert_eq!(
            app.world()
                .get::<ProductionFacility>(terminal)
                .unwrap()
                .input_amount,
            40,
            "an occupied west approach must not stop unloading through free terminal faces; hauler at {:?}, cargo {:?}, cadence {cadence:?}",
            app.world().get::<Transform>(hauler).unwrap().translation,
            app.world().get::<Cargo>(hauler),
        );
        assert_eq!(app.world().get::<Stockpile>(source).unwrap().amount, 0);
        assert!(app.world().get::<Cargo>(hauler).is_none());
    }
}

#[test]
fn worker_builds_around_occupied_site_approach() {
    for (cadence, max_updates) in [
        (Duration::from_millis(100), 600),
        (Duration::from_secs_f64(1.0 / 60.0), 3600),
    ] {
        let mut app = common::sim_app_with_planned();
        app.insert_resource(Time::<Fixed>::from_duration(cadence));
        app.insert_resource(TimeUpdateStrategy::ManualDuration(cadence));
        let cell = IVec2::ZERO;
        let center = common::cell_world_center(cell);
        let site = common::spawn_planned_structure_at_cell(&mut app, cell);
        let plan = *app.world().get::<PlannedStructure>(site).unwrap();
        app.world_mut()
            .entity_mut(site)
            .insert(plan.with_work_remaining(5));
        let mut working_bots = Vec::new();
        for y in [-68.0, 0.0, 68.0] {
            let working_bot = common::spawn_hauler_at(&mut app, center + Vec2::new(-68.0, y));
            app.world_mut()
                .entity_mut(working_bot)
                .insert(Commitment::Working);
            working_bots.push(working_bot);
        }
        let worker = common::spawn_worker_at(&mut app, center + Vec2::new(-300.0, 0.0));

        let mut work_remaining = 5;
        for _ in 0..max_updates {
            app.update();
            let next_remaining = app
                .world()
                .get::<PlannedStructure>(site)
                .map_or(0, |plan| plan.available_work());
            if next_remaining < work_remaining {
                let position = app
                    .world()
                    .get::<Transform>(worker)
                    .unwrap()
                    .translation
                    .truncate();
                for bot in &working_bots {
                    let other = app
                        .world()
                        .get::<Transform>(*bot)
                        .unwrap()
                        .translation
                        .truncate();
                    assert!(
                        position.distance(other) >= 67.999,
                        "building requires separate standing space"
                    );
                }
            }
            work_remaining = next_remaining;
            for (bot, y) in working_bots.iter().zip([-68.0, 0.0, 68.0]) {
                let position = app
                    .world()
                    .get::<Transform>(*bot)
                    .unwrap()
                    .translation
                    .truncate();
                assert!(
                    position.distance(center + Vec2::new(-68.0, y)) < 0.001,
                    "construction traffic must preserve active workers' positions"
                );
            }
            if app.world().get::<Stockpile>(site).is_some() {
                break;
            }
        }

        assert!(
            app.world().get::<Stockpile>(site).is_some(),
            "a worker must use free site faces and complete construction; worker at {:?}, site {:?}, cadence {cadence:?}",
            app.world().get::<Transform>(worker).unwrap().translation,
            app.world().get::<PlannedStructure>(site),
        );
    }
}

#[test]
fn loaded_workers_deliver_without_remote_waiting_or_removing_unloaded_workers() {
    use top_down_2d_rts_prototype_nano_swarm::{
        nanobot::{LogisticsReservation, ReturningToStockpile, SwarmId},
        resources::{ResourceKind, ResourceLedger},
    };
    let mut app = common::sim_app_with_gather();
    let cadence = Duration::from_secs_f64(1.0 / 60.0);
    app.insert_resource(Time::<Fixed>::from_duration(cadence));
    app.insert_resource(TimeUpdateStrategy::ManualDuration(cadence));
    common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let sink = common::spawn_stockpile(&mut app, Vec2::new(700.0, 0.0), 0, 1000);
    let workers: Vec<_> = (0..11)
        .map(|i| {
            let worker = common::spawn_worker_at(&mut app, Vec2::new(300.0, i as f32 * 75.0));
            let mut reservation =
                LogisticsReservation::new(Entity::PLACEHOLDER, sink, ResourceKind::Minerals, 4);
            reservation.source_remaining = 0;
            app.world_mut().entity_mut(worker).insert((
                Cargo {
                    kind: ResourceKind::Minerals,
                    amount: 4,
                },
                ReturningToStockpile { stockpile: sink },
                reservation,
                Commitment::Carrying,
            ));
            worker
        })
        .collect();
    app.world_mut().resource_mut::<ResourceLedger>().add_for(
        SwarmId::PLAYER,
        ResourceKind::Minerals,
        44,
    );
    for _ in 0..1200 {
        app.update();

        let delivered = app.world().get::<Stockpile>(sink).unwrap().amount;
        let carried: u32 = workers
            .iter()
            .map(|worker| {
                app.world()
                    .get::<Cargo>(*worker)
                    .map_or(0, |cargo| cargo.amount)
            })
            .sum();
        assert_eq!(
            delivered + carried,
            44,
            "waiting and yielding preserve physical cargo"
        );
        assert_eq!(
            app.world()
                .resource::<ResourceLedger>()
                .total_for(SwarmId::PLAYER, ResourceKind::Minerals,),
            44,
            "delivery preserves the swarm mineral ledger"
        );
        if delivered == 44 {
            break;
        }
    }
    assert_eq!(
        app.world().get::<Stockpile>(sink).unwrap().amount,
        44,
        "all eleven workers must unload within twenty simulation seconds without removing idle bodies: {:?}",
        workers
            .iter()
            .map(|e| (
                *e,
                app.world()
                    .get::<Transform>(*e)
                    .unwrap()
                    .translation
                    .truncate(),
                app.world().get::<Cargo>(*e),
                app.world()
                    .get::<top_down_2d_rts_prototype_nano_swarm::nanobot::WorkApproach>(*e)
            ))
            .collect::<Vec<_>>()
    );
    for worker in workers {
        assert!(
            app.world().get_entity(worker).is_ok(),
            "unloaded workers remain in the world"
        );
        assert!(app.world().get::<Cargo>(worker).is_none());
    }
}

#[test]
fn loaded_haulers_approach_before_waiting_and_each_complete_delivery() {
    use top_down_2d_rts_prototype_nano_swarm::{
        nanobot::{
            ApproachPhase, HaulerAssignment, InteractionRegion, LogisticsReservation, WorkApproach,
        },
        resources::ResourceKind,
    };
    let count = 12;
    let total = 640;
    let mut app = common::sim_app_with_gather_haul();
    let cadence = Duration::from_secs_f64(1.0 / 60.0);
    app.insert_resource(Time::<Fixed>::from_duration(cadence));
    app.insert_resource(TimeUpdateStrategy::ManualDuration(cadence));
    let swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let source = common::spawn_stockpile(&mut app, Vec2::new(150.0, 0.0), 400, 1000);
    let sink = common::spawn_sink_stockpile(&mut app, Vec2::new(700.0, 0.0), 0, 1000);
    for e in [source, sink] {
        app.world_mut().entity_mut(e).insert(OwnerSwarm(swarm));
    }
    let region = InteractionRegion::structure(app.world().get::<Transform>(sink).unwrap());
    let bots: Vec<_> = (0..count)
        .map(|i| {
            let e = common::spawn_hauler_at(&mut app, Vec2::new(300.0, i as f32 * 75.0));
            let mut reservation =
                LogisticsReservation::new(source, sink, ResourceKind::Minerals, 20);
            reservation.source_remaining = 0;
            app.world_mut().entity_mut(e).insert((
                HaulerAssignment { source, sink },
                reservation,
                Cargo {
                    kind: ResourceKind::Minerals,
                    amount: 20,
                },
                Commitment::Idle,
            ));
            e
        })
        .collect();
    let mut unloaded = std::collections::HashSet::new();
    for tick in 0..3600 {
        app.update();
        let carried: u32 = bots
            .iter()
            .map(|e| app.world().get::<Cargo>(*e).map_or(0, |c| c.amount))
            .sum();
        assert_eq!(
            app.world().get::<Stockpile>(source).unwrap().amount
                + app.world().get::<Stockpile>(sink).unwrap().amount
                + carried,
            total
        );
        for e in &bots {
            if app.world().get::<Cargo>(*e).is_none() {
                unloaded.insert(*e);
            }
            if let Some(approach) = app.world().get::<WorkApproach>(*e)
                && approach.phase == ApproachPhase::Waiting
                && approach.region == region
            {
                let position = app
                    .world()
                    .get::<Transform>(*e)
                    .unwrap()
                    .translation
                    .truncate();
                assert!(
                    position.distance(region.approach(position)) < 222.0,
                    "loaded hauler waited far from destination: {position:?}"
                );
            }
        }
        if tick == 1199 {
            assert_eq!(
                unloaded.len(),
                count,
                "new trips must not starve an original loaded hauler"
            );
        }
        if app.world().get::<Stockpile>(sink).unwrap().amount == total {
            break;
        }
    }
    assert_eq!(
        app.world().get::<Stockpile>(sink).unwrap().amount,
        total,
        "repeated loaded trips must drain the source within sixty simulation seconds"
    );
    assert_eq!(
        unloaded.len(),
        count,
        "every loaded hauler gets a turn at the destination"
    );
}
