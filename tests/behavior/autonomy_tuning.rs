#[path = "../common/mod.rs"]
mod common;

use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    intent::IntentGrid,
    nanobot::{
        AllocationClock, AllocationRegion, LeaseDecision, OpportunityCategory, OpportunityTarget,
        RegionalLease, Swarm, SwarmId, evaluate_lease, reconcile_autonomy_tuning,
    },
    reachability::SwarmReachabilityMap,
    strategic_controller::PLANNING_WORK_BUDGET,
    strategic_runtime::{
        ControllerTelemetry, StrategicController, StrategicControllerPlugin,
        reconcile_controller_tuning,
    },
    tuning::Tuning,
};

#[test]
fn live_lease_duration_preserves_elapsed_fraction_and_recorded_progress() {
    let mut world = World::new();
    world.init_resource::<AllocationClock>();
    let mut clock = world.resource_mut::<AllocationClock>();
    clock.advance_by(std::time::Duration::from_secs(1));
    let target = OpportunityTarget::Maintenance {
        structure: Entity::from_bits(10),
    };
    let entity = world
        .spawn(RegionalLease::new(
            AllocationRegion { x: 0, y: 0 },
            OpportunityCategory::Maintenance,
            target,
            Some(SwarmId::PLAYER),
            0,
            7,
            30,
        ))
        .id();
    let old = Tuning::default();
    let mut new = old.clone();
    new.autonomy.lease_no_progress_ttl_ticks = 60;
    reconcile_autonomy_tuning(&mut world, &old, &new);
    let mut lease = *world.get::<RegionalLease>(entity).unwrap();
    assert_eq!(lease.progress_checkpoint(), 7);
    assert_eq!(
        evaluate_lease(&mut lease, 49, 7, true, 60),
        LeaseDecision::Keep
    );
    assert_eq!(
        evaluate_lease(&mut lease, 50, 7, true, 60),
        LeaseDecision::RevokeNoProgress
    );
}

#[test]
fn live_controller_tuning_reviews_both_swarms_between_regular_reviews() {
    let mut app = common::minimal_app();
    app.add_plugins(StrategicControllerPlugin);
    let owners = [SwarmId::PLAYER, SwarmId(2)];
    for owner in owners {
        app.world_mut().spawn((
            Swarm {},
            owner,
            Transform::default(),
            StrategicController::new(owner),
        ));
    }
    let reachability = SwarmReachabilityMap::new(
        app.world().resource::<IntentGrid>(),
        [],
        owners.into_iter().map(|owner| (owner, Vec2::ZERO)),
    )
    .unwrap();
    app.insert_resource(reachability);
    app.update();
    app.update();
    let before = app.world().resource::<ControllerTelemetry>().profiles();
    let old = app.world().resource::<Tuning>().clone();
    let mut new = old.clone();
    new.autonomy.strategy_hold_base = 300.0;
    reconcile_controller_tuning(app.world_mut(), &old, &new);
    app.insert_resource(new);
    app.update();
    let after = app.world().resource::<ControllerTelemetry>().profiles();
    for owner in owners {
        assert_eq!(before[&owner.0].reviews, 1);
        assert_eq!(
            after[&owner.0].reviews, 2,
            "every gameplay controller must receive the edit"
        );
        assert!(after[&owner.0].work_units <= PLANNING_WORK_BUDGET as u64);
    }
}

#[test]
fn live_category_weights_redirect_new_workers_without_revoking_existing_work() {
    use top_down_2d_rts_prototype_nano_swarm::{
        intent::IntentKind,
        nanobot::{InteractionRegion, RegionalAllocationPlugin},
        navigation::{ConnectivityStatus, Navigation, RouteGoal},
    };
    let mut app = common::minimal_app();
    app.insert_resource(IntentGrid::new(32, 32));
    app.add_plugins(RegionalAllocationPlugin);
    let position = common::cell_world_center(IVec2::ZERO);
    common::spawn_swarm_at(&mut app, position);
    common::spawn_deposit(
        &mut app,
        common::DepositFixture {
            world_pos: position,
            amount: 1_000_000,
            capacity: 1_000_000,
            radius: 24.0,
        },
    );
    app.world_mut().resource_mut::<IntentGrid>().paint(
        IVec2::ZERO,
        IntentKind::Gather,
        SwarmId::PLAYER,
    );
    let mut targets = Vec::new();
    for y in [1, 3, 5] {
        for x in [1, 3, 5] {
            let target = common::spawn_planned_structure_at_cell(&mut app, IVec2::new(x, y));
            targets.push(*app.world().get::<Transform>(target).unwrap());
        }
    }
    let start = position - Vec2::Y * 120.0;
    let mut all_routes_ready = false;
    for _ in 0..600 {
        let navigation = app.world().resource::<Navigation>();
        let ready = targets
            .iter()
            .filter(|target| {
                matches!(
                    navigation.query_connectivity(
                        start,
                        RouteGoal::Interaction(InteractionRegion::structure(target))
                    ),
                    ConnectivityStatus::Connected { .. }
                )
            })
            .count();
        all_routes_ready = ready == targets.len();
        if all_routes_ready {
            break;
        }
        app.update();
    }
    assert!(
        all_routes_ready,
        "all construction approaches must be reachable before measuring category pressure"
    );
    let first: Vec<_> = (0..9)
        .map(|_| common::spawn_worker_at(&mut app, position - Vec2::Y * 120.0))
        .collect();
    for _ in 0..60 {
        app.update();
    }
    let gathers = first
        .iter()
        .filter(|entity| {
            app.world()
                .get::<RegionalLease>(**entity)
                .is_some_and(|lease| lease.category == OpportunityCategory::Gather)
        })
        .count();
    assert!(
        gathers >= 6,
        "normal weights must give most initial capacity to the large deposit, got {gathers} Gather leases: {:?}",
        first
            .iter()
            .map(|entity| app.world().get::<RegionalLease>(*entity))
            .collect::<Vec<_>>()
    );
    let retained: Vec<_> = first
        .iter()
        .map(|entity| {
            (
                *entity,
                app.world().get::<RegionalLease>(*entity).unwrap().target,
            )
        })
        .collect();

    let old = app.world().resource::<Tuning>().clone();
    let mut new = old.clone();
    new.autonomy.planned_build_weight = 1_000_000;
    reconcile_autonomy_tuning(app.world_mut(), &old, &new);
    app.insert_resource(new);
    let second: Vec<_> = (0..9)
        .map(|_| common::spawn_worker_at(&mut app, position - Vec2::Y * 120.0))
        .collect();
    for _ in 0..60 {
        app.update();
    }
    let builders = second
        .iter()
        .filter(|entity| {
            app.world()
                .get::<RegionalLease>(**entity)
                .is_some_and(|lease| lease.category == OpportunityCategory::PlannedBuild)
        })
        .count();
    assert!(
        builders >= 6,
        "live Build weight must redirect newly acquired capacity to construction, got {builders}: {:?}",
        second
            .iter()
            .map(|entity| app.world().get::<RegionalLease>(*entity))
            .collect::<Vec<_>>()
    );
    for (entity, target) in retained {
        assert_eq!(
            app.world().get::<RegionalLease>(entity).unwrap().target,
            target,
            "changing category pressure must retain acquired work"
        );
    }
}
