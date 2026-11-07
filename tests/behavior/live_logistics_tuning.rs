use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    nanobot::{
        Cargo, HaulerAssignment, HaulerLoading, LogisticsReservation, MaterialWorkProgress,
        OwnerSwarm, SwarmId, reconcile_logistics_tuning,
    },
    resources::{ResourceKind, ResourceLedger, Stockpile, StockpileRole},
    tuning::Tuning,
};

#[path = "../common/mod.rs"]
mod common;

#[test]
fn live_logistics_capacity_shrink_preserves_minerals_and_releases_excess_claims() {
    let mut app = common::sim_app_with_gather_haul();
    let swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let source = common::spawn_stockpile(&mut app, Vec2::ZERO, 32, 150);
    let destination = common::spawn_sink_stockpile(&mut app, Vec2::new(300.0, 0.0), 8, 150);
    for entity in [source, destination] {
        app.world_mut().entity_mut(entity).insert(OwnerSwarm(swarm));
    }
    let hauler = common::spawn_hauler_at(&mut app, Vec2::new(68.0, 0.0));
    let mut reservation =
        LogisticsReservation::new(source, destination, ResourceKind::Minerals, 20);
    reservation.source_remaining = 14;
    app.world_mut().entity_mut(hauler).insert((
        Cargo {
            kind: ResourceKind::Minerals,
            amount: 6,
        },
        HaulerAssignment {
            source,
            sink: destination,
            pickup_role: StockpileRole::Source,
        },
        HaulerLoading,
        MaterialWorkProgress { ticks_elapsed: 30 },
        reservation,
    ));
    app.world_mut().resource_mut::<ResourceLedger>().add_for(
        SwarmId::PLAYER,
        ResourceKind::Minerals,
        46,
    );
    let old = Tuning::default();
    let mut cosmetic_edit = old.clone();
    cosmetic_edit.presentation.camera_max_speed += 1.0;
    reconcile_logistics_tuning(app.world_mut(), &old, &cosmetic_edit);
    assert_eq!(
        *app.world().get::<LogisticsReservation>(hauler).unwrap(),
        reservation
    );
    assert_eq!(
        app.world()
            .get::<MaterialWorkProgress>(hauler)
            .unwrap()
            .ticks_elapsed,
        30
    );
    assert!(app.world().get::<HaulerLoading>(hauler).is_some());
    let mut new = old.clone();
    new.logistics.hauler_carry_capacity = 3;
    new.logistics.stockpile_capacity = 10;
    app.insert_resource(new.clone());

    reconcile_logistics_tuning(app.world_mut(), &old, &new);

    assert_eq!(app.world().get::<Cargo>(hauler).unwrap().amount, 6);
    assert!(
        app.world().get::<HaulerLoading>(hauler).is_none(),
        "overfull cargo must leave loading"
    );
    assert_eq!(app.world().get::<Stockpile>(source).unwrap().amount, 32);
    assert_eq!(app.world().get::<Stockpile>(source).unwrap().capacity, 10);
    assert_eq!(app.world().get::<Stockpile>(destination).unwrap().amount, 8);
    let reservation = app.world().get::<LogisticsReservation>(hauler).unwrap();
    assert_eq!(reservation.source_remaining, 0);
    assert_eq!(
        reservation.destination_remaining, 2,
        "only the next partial delivery claims the remaining capacity"
    );
    assert_eq!(
        app.world()
            .resource::<ResourceLedger>()
            .total_for(SwarmId::PLAYER, ResourceKind::Minerals),
        46
    );
}

#[test]
fn live_material_duration_preserves_fraction_until_the_next_physical_pulse() {
    let old = Tuning::default();
    let mut new = old.clone();
    new.logistics.material_work_ticks = 30;
    let mut world = World::new();
    let worker = world.spawn(MaterialWorkProgress { ticks_elapsed: 30 }).id();

    reconcile_logistics_tuning(&mut world, &old, &new);

    let mut progress = world.get_mut::<MaterialWorkProgress>(worker).unwrap();
    assert_eq!(
        progress.ticks_elapsed, 12,
        "40% completed work stays 40% completed"
    );
    for _ in 0..17 {
        assert!(!progress.completes_unit(new.logistics.material_work_ticks));
    }
    assert!(progress.completes_unit(new.logistics.material_work_ticks));
    assert_eq!(progress.ticks_elapsed, 0);
}

#[test]
fn live_corridor_cost_reopens_completed_requests_with_the_new_route_cost() {
    use top_down_2d_rts_prototype_nano_swarm::{
        intent::{IntentGrid, IntentKind},
        navigation::{Navigation, RouteGoal, RoutePriority, RouteStatus},
    };
    let mut grid = IntentGrid::new(2, 2);
    grid.paint(IVec2::ZERO, IntentKind::Corridor, SwarmId::PLAYER);
    let mut app = common::minimal_app();
    app.insert_resource(grid.clone());
    app.world_mut()
        .resource_mut::<Tuning>()
        .logistics
        .corridor_cost_multiplier = 0.2;
    app.update();
    let navigation = app.world().resource::<Navigation>();
    let request = navigation.request(
        Vec2::new(72.0, 72.0),
        RouteGoal::Point(Vec2::new(216.0, 72.0)),
        SwarmId::PLAYER,
        true,
        RoutePriority::Routine,
    );
    for _ in 0..200 {
        navigation.advance(&grid, 2048);
        if matches!(navigation.poll(request), RouteStatus::Found(_)) {
            break;
        }
    }
    let RouteStatus::Found(discounted) = navigation.poll(request) else {
        panic!("initial Corridor route did not complete");
    };
    assert!((discounted.cost - 28.8).abs() < 0.01);

    let old = app.world().resource::<Tuning>().clone();
    let mut new = old.clone();
    new.logistics.corridor_cost_multiplier = 1.0;
    app.insert_resource(new.clone());
    reconcile_logistics_tuning(app.world_mut(), &old, &new);
    let navigation = app.world().resource::<Navigation>();

    assert!(
        matches!(navigation.poll(request), RouteStatus::Pending),
        "completed results using the old cost must be invalidated"
    );
    for _ in 0..200 {
        navigation.advance(&grid, 2048);
        if matches!(navigation.poll(request), RouteStatus::Found(_)) {
            break;
        }
    }
    let RouteStatus::Found(updated) = navigation.poll(request) else {
        panic!("updated Corridor route did not complete");
    };
    assert!((updated.cost - 144.0).abs() < 0.01);
}

#[test]
fn live_smaller_stockpiles_accept_existing_worker_and_hauler_loads_in_segments() {
    use top_down_2d_rts_prototype_nano_swarm::nanobot::ReturningToStockpile;
    for (is_hauler, initial_load) in [(false, 4_u32), (true, 20_u32)] {
        let mut app = common::sim_app_with_gather_haul();
        let swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
        let mut buffers = Vec::new();
        for index in 0..initial_load.div_ceil(3) {
            let position = Vec2::new(128.0 + index as f32 * 256.0, 0.0);
            let stockpile = if is_hauler {
                common::spawn_sink_stockpile(&mut app, position, 0, 150)
            } else {
                common::spawn_stockpile(&mut app, position, 0, 150)
            };
            app.world_mut()
                .entity_mut(stockpile)
                .insert(OwnerSwarm(swarm));
            buffers.push(stockpile);
        }
        let bot = if is_hauler {
            common::spawn_hauler_at(&mut app, Vec2::ZERO)
        } else {
            common::spawn_worker_at(&mut app, Vec2::ZERO)
        };
        let mut reservation =
            LogisticsReservation::new(buffers[0], buffers[0], ResourceKind::Minerals, initial_load);
        reservation.source_remaining = 0;
        app.world_mut().entity_mut(bot).insert((
            Cargo {
                kind: ResourceKind::Minerals,
                amount: initial_load,
            },
            reservation,
        ));
        if is_hauler {
            app.world_mut().entity_mut(bot).insert(HaulerAssignment {
                source: buffers[0],
                sink: buffers[0],
                pickup_role: StockpileRole::Source,
            });
        } else {
            app.world_mut()
                .entity_mut(bot)
                .insert(ReturningToStockpile {
                    stockpile: buffers[0],
                });
        }
        app.world_mut().resource_mut::<ResourceLedger>().add_for(
            SwarmId::PLAYER,
            ResourceKind::Minerals,
            initial_load,
        );
        let old = Tuning::default();
        let mut new = old.clone();
        new.logistics.stockpile_capacity = 3;
        new.logistics.material_work_ticks = 1;
        app.insert_resource(new.clone());
        reconcile_logistics_tuning(app.world_mut(), &old, &new);

        for _ in 0..4000 {
            app.update();
            let stored = buffers
                .iter()
                .map(|entity| {
                    let stockpile = app.world().get::<Stockpile>(*entity).unwrap();
                    assert!(stockpile.amount <= stockpile.capacity);
                    stockpile.amount
                })
                .sum::<u32>();
            let carried = app
                .world()
                .get::<Cargo>(bot)
                .map_or(0, |cargo| cargo.amount);
            assert_eq!(stored + carried, initial_load);
            assert_eq!(
                app.world()
                    .resource::<ResourceLedger>()
                    .total_for(SwarmId::PLAYER, ResourceKind::Minerals),
                initial_load
            );
            if carried == 0 {
                break;
            }
        }
        assert!(
            app.world().get::<Cargo>(bot).is_none(),
            "the entire retained load must unload across smaller buffers (hauler={is_hauler})"
        );
    }
}
