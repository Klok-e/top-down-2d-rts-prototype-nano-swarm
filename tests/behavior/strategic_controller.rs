static DEFAULT_CHARGE: std::sync::LazyLock<
    top_down_2d_rts_prototype_nano_swarm::tuning::ChargeTuning,
> = std::sync::LazyLock::new(top_down_2d_rts_prototype_nano_swarm::tuning::ChargeTuning::default);
#[path = "../common/mod.rs"]
mod common;

/// Bounded real-time wait for shared defender allocation at 60 Hz.
const DEFENDER_RESPONSE_TICK_BUDGET: u32 = 300;

use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    intent::{IntentGrid, IntentKind},
    nanobot::{MatchOutcome, Swarm, SwarmId},
    reachability::SwarmReachabilityMap,
    resources::{ResourceKind, ResourceLedger},
    strategic_controller::{Decision, IntentEditAction},
    strategic_runtime::{StrategicController, StrategicControllerPlugin},
};

fn apply_decision(grid: &mut IntentGrid, owner: SwarmId, decision: &Decision) {
    for edit in &decision.edits {
        match edit.action {
            IntentEditAction::Paint => grid.paint(edit.cell, edit.kind, owner),
            IntentEditAction::Erase => grid.erase(edit.cell, edit.kind, owner),
        };
    }
}

fn install_empty_reachability(app: &mut App, owners: impl IntoIterator<Item = SwarmId>) {
    let reachability = {
        let grid = app.world().resource::<IntentGrid>();
        SwarmReachabilityMap::new(
            grid,
            [],
            owners.into_iter().map(|owner| (owner, Vec2::ZERO)),
        )
        .expect("the controller fixture has clear swarm homes")
    };
    app.insert_resource(reachability);
}

#[test]
fn strategic_adapter_applies_only_its_owners_intent_without_mutating_resources() {
    let mut app = common::minimal_app();
    app.add_plugins(StrategicControllerPlugin);
    let owner = SwarmId(7);
    let old = IVec2::new(2, 0);
    let next = IVec2::ZERO;
    app.world_mut().spawn((
        Swarm {},
        owner,
        Transform::default(),
        StrategicController::new(owner),
    ));
    install_empty_reachability(&mut app, [owner]);
    for swarm in [SwarmId::PLAYER, owner] {
        app.world_mut()
            .resource_mut::<IntentGrid>()
            .paint(old, IntentKind::Defend, swarm);
        app.world_mut()
            .resource_mut::<ResourceLedger>()
            .add_for(swarm, ResourceKind::Minerals, 37);
    }
    app.update();
    let grid = app.world().resource::<IntentGrid>();
    assert!(
        grid.cell(next)
            .unwrap()
            .has_owned(IntentKind::Defend, owner)
    );
    assert!(!grid.cell(old).unwrap().has_owned(IntentKind::Defend, owner));
    assert!(
        grid.cell(old)
            .unwrap()
            .has_owned(IntentKind::Defend, SwarmId::PLAYER)
    );
    assert!(
        !grid
            .cell(next)
            .unwrap()
            .has_owned(IntentKind::Defend, SwarmId::PLAYER)
    );
    for swarm in [SwarmId::PLAYER, owner] {
        assert_eq!(
            app.world()
                .resource::<ResourceLedger>()
                .total_for(swarm, ResourceKind::Minerals),
            37
        );
    }
    let revision = grid.revision();
    app.insert_resource(MatchOutcome::Winner(owner));
    for _ in 0..4 {
        app.update();
    }
    assert_eq!(app.world().resource::<IntentGrid>().revision(), revision);
}

#[test]
fn strategic_adapter_connects_the_deposit_to_home_around_rock() {
    use top_down_2d_rts_prototype_nano_swarm::terrain::RockFormation;

    let mut app = common::minimal_app();
    app.insert_resource(IntentGrid::new(16, 16));
    app.add_plugins(StrategicControllerPlugin);
    let owner = SwarmId::PLAYER;
    let home_cell = IVec2::new(-3, 0);
    let deposit_cell = IVec2::new(3, 0);
    let home = common::cell_world_center(home_cell);
    let rock = RockFormation::Rectangle {
        half: Vec2::new(256.0, 650.0),
    };
    let rock_transform =
        Transform::from_translation(common::cell_world_center(IVec2::ZERO).extend(0.0));
    app.world_mut().spawn((
        Swarm {},
        owner,
        Transform::from_translation(home.extend(0.0)),
        StrategicController::new(owner),
    ));
    app.world_mut().spawn((rock, rock_transform));
    common::spawn_deposit(
        &mut app,
        common::DepositFixture {
            world_pos: common::cell_world_center(deposit_cell),
            amount: 72_000,
            capacity: 72_000,
            radius: 80.0,
        },
    );
    let reachability = SwarmReachabilityMap::new(
        app.world().resource::<IntentGrid>(),
        [rock.obstacle(&rock_transform)],
        [(owner, home)],
    )
    .unwrap();
    app.insert_resource(reachability);
    app.update();
    let grid = app.world().resource::<IntentGrid>();
    let corridor = |cell| {
        grid.cell(cell)
            .is_some_and(|intent| intent.has_owned(IntentKind::Corridor, owner))
    };
    assert!(corridor(home_cell));
    assert!(!corridor(IVec2::ZERO), "the direct corridor is blocked");

    let mut connected = std::collections::HashSet::from([home_cell]);
    let mut frontier = vec![home_cell];
    while let Some(cell) = frontier.pop() {
        for next in (-1..=1).flat_map(|y| (-1..=1).map(move |x| cell + IVec2::new(x, y))) {
            if corridor(next) && connected.insert(next) {
                frontier.push(next);
            }
        }
    }
    assert!(
        connected.contains(&deposit_cell),
        "corridor cells: {connected:?}"
    );
}

#[test]
fn strategic_adapter_contests_a_deposit_when_a_hostile_worker_moves_toward_it() {
    use top_down_2d_rts_prototype_nano_swarm::{nanobot::SwarmMember, tuning::Tuning};

    let mut app = common::minimal_app();
    app.insert_resource(IntentGrid::new(16, 16));
    app.add_plugins(StrategicControllerPlugin);
    let owner = SwarmId(7);
    let enemy = SwarmId(9);
    let deposit = IVec2::new(5, 0);
    let deposit_position = common::cell_world_center(deposit);
    app.world_mut().spawn((
        Swarm {},
        owner,
        Transform::default(),
        StrategicController::new(owner),
    ));
    install_empty_reachability(&mut app, [owner]);
    common::spawn_deposit(
        &mut app,
        common::DepositFixture {
            world_pos: deposit_position,
            amount: 72_000,
            capacity: 72_000,
            radius: 80.0,
        },
    );
    let worker = common::spawn_worker_at(&mut app, deposit_position + Vec2::X * 400.0);
    app.world_mut()
        .entity_mut(worker)
        .insert(SwarmMember::new(enemy));
    let mut tuning = app.world_mut().resource_mut::<Tuning>();
    tuning.autonomy.controller_review_period_ticks = 1;
    tuning.autonomy.strategy_expansion_income_weight = 0.0;
    tuning.autonomy.strategy_staging_spacing_cells = 64;

    let owns_deposit_block = |app: &App| {
        app.world()
            .resource::<IntentGrid>()
            .cell(deposit)
            .is_some_and(|cell| cell.has_owned(IntentKind::Defend, owner))
    };
    app.update();
    assert!(!owns_deposit_block(&app));

    app.world_mut()
        .get_mut::<Transform>(worker)
        .unwrap()
        .translation
        .x -= 4.0;
    app.update();
    let review = &app
        .world()
        .resource::<top_down_2d_rts_prototype_nano_swarm::strategic_runtime::ControllerTelemetry>()
        .profiles()[&owner.0];
    assert!(owns_deposit_block(&app), "{review:?}");
}

#[test]
fn controller_telemetry_counts_invalidation_checks_between_reviews() {
    use top_down_2d_rts_prototype_nano_swarm::strategic_runtime::ControllerTelemetry;

    let mut app = common::minimal_app();
    app.add_plugins(StrategicControllerPlugin);
    let owner = SwarmId(7);
    app.world_mut().spawn((
        Swarm {},
        owner,
        Transform::default(),
        StrategicController::new(owner),
    ));
    install_empty_reachability(&mut app, [owner]);
    common::spawn_deposit(
        &mut app,
        common::DepositFixture {
            world_pos: common::cell_world_center(IVec2::new(-1, -1)),
            amount: 72_000,
            capacity: 72_000,
            radius: 80.0,
        },
    );
    app.update();
    let initial = app.world().resource::<ControllerTelemetry>().profiles()[&owner.0].clone();
    assert_eq!(initial.reviews, 1);
    app.update();
    let next = app.world().resource::<ControllerTelemetry>().profiles()[&owner.0].clone();
    assert_eq!(next.reviews, initial.reviews);
    assert_eq!(next.intent_edits, initial.intent_edits);
    assert!(
        next.work_units > initial.work_units,
        "checking the retained deposit consumes work even without a strategy review"
    );
}

#[test]
fn controller_eventually_erases_stale_owned_intent_after_large_foreign_prefix() {
    use top_down_2d_rts_prototype_nano_swarm::strategic_controller::{
        Controller, GameState, SwarmState,
    };

    let owner = SwarmId(4);
    let foreign = SwarmId(9);
    let foreign_sentinel = IVec2::new(-32, -32);
    let stale = IVec2::new(31, 31);
    let mut grid = IntentGrid::new(64, 64);
    for y in -32..-27 {
        for x in -32..32 {
            grid.paint(IVec2::new(x, y), IntentKind::Gather, foreign);
        }
    }
    grid.paint(stale, IntentKind::Defend, owner);

    let swarms = [SwarmState {
        id: owner,
        home: Vec2::ZERO,
        minerals: 0,
        mineral_income_per_second: 0.0,
        production_saturated: false,
    }];
    let mut controller = Controller::new(owner);
    for review in 0..3 {
        let decision = controller.decide(
            &top_down_2d_rts_prototype_nano_swarm::tuning::AutonomyTuning::default(),
            &GameState {
                grid: &grid,
                swarms: &swarms,
                bots: &[],
                structures: &[],
                deposits: &[],
                terrain: &[],
                charge: &DEFAULT_CHARGE,
                reachability: None,
                route_query: None,
                mine_query: None,
                tick: review * 30,
                finished: false,
            },
        );
        apply_decision(&mut grid, owner, &decision);
    }

    assert!(
        !grid
            .cell(stale)
            .unwrap()
            .has_owned(IntentKind::Defend, owner),
        "bounded cleanup must eventually pass foreign cells and erase stale owned intent"
    );
    assert!(
        grid.cell(foreign_sentinel)
            .unwrap()
            .has_owned(IntentKind::Gather, foreign),
        "cleanup must preserve foreign intent"
    );
}

#[test]
fn controller_reviews_immediately_when_an_owned_deposit_is_exhausted() {
    use top_down_2d_rts_prototype_nano_swarm::strategic_controller::{
        Controller, DepositState, GameState, SwarmState,
    };

    let mut grid = IntentGrid::new(32, 32);
    let owner = SwarmId(4);
    let world = |cell: IVec2| {
        (cell.as_vec2() + Vec2::splat(0.5)) * top_down_2d_rts_prototype_nano_swarm::ZONE_BLOCK_SIZE
    };
    let swarms = [SwarmState {
        id: owner,
        home: world(IVec2::ZERO),
        minerals: 0,
        mineral_income_per_second: 0.0,
        production_saturated: false,
    }];
    let deposits = [
        DepositState {
            id: 20,
            owner: None,
            position: world(IVec2::new(2, 0)),
            amount: 1_000,
            radius: 100.0,
        },
        DepositState {
            id: 21,
            owner: None,
            position: world(IVec2::new(4, 0)),
            amount: 50,
            radius: 100.0,
        },
    ];
    let mut controller = Controller::new(owner);
    let initial = controller.decide(
        &top_down_2d_rts_prototype_nano_swarm::tuning::AutonomyTuning::default(),
        &GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &[],
            structures: &[],
            deposits: &deposits,
            terrain: &[],
            charge: &DEFAULT_CHARGE,
            reachability: None,
            route_query: None,
            mine_query: None,
            tick: 0,
            finished: false,
        },
    );
    apply_decision(&mut grid, owner, &initial);

    let depleted = [
        DepositState {
            amount: 0,
            ..deposits[0]
        },
        deposits[1],
    ];
    let urgent = controller.decide(
        &top_down_2d_rts_prototype_nano_swarm::tuning::AutonomyTuning::default(),
        &GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &[],
            structures: &[],
            deposits: &depleted,
            terrain: &[],
            charge: &DEFAULT_CHARGE,
            reachability: None,
            route_query: None,
            mine_query: None,
            tick: 1,
            finished: false,
        },
    );

    assert!(urgent.reviewed, "depletion must bypass the regular cadence");
    assert!(urgent.explanation.contains("exhausted"));
    assert!(urgent.edits.iter().any(|edit| {
        edit.action == IntentEditAction::Paint
            && edit.kind == IntentKind::Gather
            && edit.cell == IVec2::new(4, 0)
    }));
    assert!(urgent.edits.iter().any(|edit| {
        edit.action == IntentEditAction::Erase
            && edit.kind == IntentKind::Gather
            && edit.cell == IVec2::new(2, 0)
    }));
}

#[test]
fn controller_urgently_replans_when_key_support_is_lost() {
    use top_down_2d_rts_prototype_nano_swarm::strategic_controller::{
        Controller, DepositState, GameState, StructureKind, StructureState, SwarmState,
    };

    let mut grid = IntentGrid::new(32, 32);
    let owner = SwarmId(4);
    let world = |cell: IVec2| {
        (cell.as_vec2() + Vec2::splat(0.5)) * top_down_2d_rts_prototype_nano_swarm::ZONE_BLOCK_SIZE
    };
    let swarms = [SwarmState {
        id: owner,
        home: world(IVec2::ZERO),
        minerals: 20,
        mineral_income_per_second: 0.0,
        production_saturated: false,
    }];
    let deposits = [DepositState {
        id: 20,
        owner: None,
        position: world(IVec2::new(2, 0)),
        amount: 500,
        radius: 100.0,
    }];
    let charger = [StructureState {
        id: 42,
        owner,
        position: world(IVec2::new(1, 0)),
        kind: StructureKind::Charger,
        operational: true,
        health: 100,
        max_health: 100,
        minerals: 25,
    }];
    let mut controller = Controller::new(owner);
    let initial = controller.decide(
        &top_down_2d_rts_prototype_nano_swarm::tuning::AutonomyTuning::default(),
        &GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &[],
            structures: &charger,
            deposits: &deposits,
            terrain: &[],
            charge: &DEFAULT_CHARGE,
            reachability: None,
            route_query: None,
            mine_query: None,
            tick: 0,
            finished: false,
        },
    );
    apply_decision(&mut grid, owner, &initial);

    let urgent = controller.decide(
        &top_down_2d_rts_prototype_nano_swarm::tuning::AutonomyTuning::default(),
        &GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &[],
            structures: &[],
            deposits: &deposits,
            terrain: &[],
            charge: &DEFAULT_CHARGE,
            reachability: None,
            route_query: None,
            mine_query: None,
            tick: 1,
            finished: false,
        },
    );

    assert!(urgent.reviewed);
    assert!(urgent.explanation.contains("key Charger 42 was lost"));
}

#[test]
fn regular_and_urgent_reviews_share_one_public_work_allowance() {
    use top_down_2d_rts_prototype_nano_swarm::{
        nanobot::NanobotType,
        strategic_controller::{
            BotState, Controller, DepositState, GameState, PLANNING_WORK_BUDGET, StructureKind,
            StructureState, SwarmState,
        },
    };

    let mut grid = IntentGrid::new(64, 64);
    assert_eq!(PLANNING_WORK_BUDGET, 100_000);
    let owner = SwarmId(4);
    let enemy = SwarmId(9);
    let world = |cell: IVec2| {
        (cell.as_vec2() + Vec2::splat(0.5)) * top_down_2d_rts_prototype_nano_swarm::ZONE_BLOCK_SIZE
    };
    let swarms = [
        SwarmState {
            id: owner,
            home: world(IVec2::ZERO),
            minerals: 100,
            mineral_income_per_second: 0.0,
            production_saturated: false,
        },
        SwarmState {
            id: enemy,
            home: world(IVec2::new(24, 24)),
            minerals: 0,
            mineral_income_per_second: 0.0,
            production_saturated: false,
        },
    ];
    let deposit = [DepositState {
        id: 20,
        owner: None,
        position: world(IVec2::new(-1, 0)),
        amount: 500,
        radius: 100.0,
    }];
    let remnant = BotState {
        id: 50,
        owner: enemy,
        kind: NanobotType::Worker,
        position: world(IVec2::new(12, 3)),
        health: 20,
        max_health: 100,
        charge: 1.0,
        max_charge: 1.0,
        cargo: 0,
        displacement: Vec2::ZERO,
    };
    let charger = StructureState {
        id: 42,
        owner,
        position: world(IVec2::new(1, 0)),
        kind: StructureKind::Charger,
        operational: true,
        health: 100,
        max_health: 100,
        minerals: 25,
    };
    let mut controller = Controller::new(owner);
    let initial = controller.decide(
        &top_down_2d_rts_prototype_nano_swarm::tuning::AutonomyTuning::default(),
        &GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &[remnant],
            structures: &[charger],
            deposits: &deposit,
            terrain: &[],
            charge: &DEFAULT_CHARGE,
            reachability: None,
            route_query: None,
            mine_query: None,
            tick: 0,
            finished: false,
        },
    );
    apply_decision(&mut grid, owner, &initial);

    let crowded_structures = (0..60_000)
        .map(|id| StructureState {
            id: 1_000 + id,
            owner,
            position: swarms[0].home,
            kind: StructureKind::Planned,
            operational: false,
            health: 100,
            max_health: 100,
            minerals: 0,
        })
        .collect::<Vec<_>>();
    let support_loss = controller.decide(
        &top_down_2d_rts_prototype_nano_swarm::tuning::AutonomyTuning::default(),
        &GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &[remnant],
            structures: &crowded_structures,
            deposits: &deposit,
            terrain: &[],
            charge: &DEFAULT_CHARGE,
            reachability: None,
            route_query: None,
            mine_query: None,
            tick: 1,
            finished: false,
        },
    );
    apply_decision(&mut grid, owner, &support_loss);

    let moved = BotState {
        position: world(IVec2::new(13, 3)),
        ..remnant
    };
    let mut crowded_bots = (0..60_000)
        .map(|id| BotState {
            id: 1_000 + id,
            owner,
            kind: NanobotType::Worker,
            position: swarms[0].home,
            health: 100,
            max_health: 100,
            charge: 1.0,
            max_charge: 1.0,
            cargo: 0,
            displacement: Vec2::ZERO,
        })
        .collect::<Vec<_>>();
    crowded_bots.push(moved);
    let target_move = controller.decide(
        &top_down_2d_rts_prototype_nano_swarm::tuning::AutonomyTuning::default(),
        &GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &crowded_bots,
            structures: &[],
            deposits: &deposit,
            terrain: &[],
            charge: &DEFAULT_CHARGE,
            reachability: None,
            route_query: None,
            mine_query: None,
            tick: 2,
            finished: false,
        },
    );

    assert!(support_loss.reviewed);
    assert!(
        initial.work_units + support_loss.work_units + target_move.work_units
            <= PLANNING_WORK_BUDGET
    );
    assert!(!target_move.reviewed);
    assert!(target_move.edits.is_empty());
}

#[test]
fn strategic_adapter_waits_for_mine_saturation_despite_a_stocked_source() {
    use top_down_2d_rts_prototype_nano_swarm::{
        nanobot::{NanobotType, OwnerSwarm},
        resources::ResourceDeposit,
    };

    let owner = SwarmId::PLAYER;
    let opening = IVec2::new(2, 0);
    let expansion = IVec2::new(5, 0);
    let opening_position = common::cell_world_center(opening);
    let mut app = common::minimal_app();
    app.insert_resource(IntentGrid::new(16, 16));
    app.add_plugins(StrategicControllerPlugin);
    let swarm = common::spawn_swarm_with_nanobots(
        &mut app,
        common::cell_world_center(IVec2::ZERO),
        &[(NanobotType::Worker, 4), (NanobotType::Hauler, 2)],
    );
    app.world_mut()
        .entity_mut(swarm)
        .insert(StrategicController::new(owner));
    install_empty_reachability(&mut app, [owner]);
    let [mine, _] = [opening, expansion].map(|cell| {
        common::spawn_deposit(
            &mut app,
            common::DepositFixture {
                world_pos: common::cell_world_center(cell),
                amount: 72_000,
                capacity: 72_000,
                radius: 64.0,
            },
        )
    });
    let source = common::spawn_stockpile(&mut app, opening_position + Vec2::Y * 300.0, 59, 60);
    app.world_mut().entity_mut(source).insert(OwnerSwarm(swarm));

    app.update();
    app.world_mut()
        .get_mut::<ResourceDeposit>(mine)
        .unwrap()
        .amount -= 1;
    for _ in 0..120 {
        app.update();
    }
    let grid = app.world().resource::<IntentGrid>();
    assert!(
        grid.cell(opening)
            .unwrap()
            .has_owned(IntentKind::Gather, owner)
    );
    assert!(
        !grid
            .cell(expansion)
            .is_some_and(|cell| cell.has_owned(IntentKind::Gather, owner)),
        "existing stock and an extraction pulse cannot stand in for sustained Worker drop-offs"
    );
}

#[test]
fn strategic_adapter_uses_sustained_local_dropoffs_and_reachable_source_capacity() {
    use top_down_2d_rts_prototype_nano_swarm::{
        nanobot::{
            Cargo, ExtractProgress, GatherAssignment, InteractionRegion, LogisticsReservation,
            NanobotType, OwnerSwarm, ReturningToStockpile, SwarmMember,
            worker_gather_delivery_system,
        },
        resources::Stockpile,
        terrain::RockFormation,
        tuning::Tuning,
    };

    enum Deliveries {
        Ongoing,
        OtherSwarm,
        OtherMine,
        Stall,
        Burst,
    }

    let owner = SwarmId::PLAYER;
    let foreign = SwarmId(3);
    let home = common::cell_world_center(IVec2::ZERO);
    let opening = IVec2::new(2, 0);
    let expansion = IVec2::new(0, 5);
    let deposit_position = common::cell_world_center(opening);
    let source_position = deposit_position + Vec2::new(-100.0, 340.0);
    for (capacity, deliveries, split_work_ring, should_expand) in [
        (4, Deliveries::Ongoing, false, true),
        (4, Deliveries::Ongoing, true, true),
        (60, Deliveries::Ongoing, false, false),
        (4, Deliveries::OtherSwarm, false, false),
        (4, Deliveries::OtherMine, false, false),
        (4, Deliveries::Stall, false, false),
        (4, Deliveries::Burst, false, false),
    ] {
        let delivery_owner = if matches!(deliveries, Deliveries::OtherSwarm) {
            foreign
        } else {
            owner
        };
        let mut app = common::minimal_app();
        app.insert_resource(IntentGrid::new(16, 16))
            .add_plugins(StrategicControllerPlugin)
            .add_systems(FixedUpdate, worker_gather_delivery_system);
        app.world_mut()
            .resource_mut::<Tuning>()
            .logistics
            .source_proximity_radius = 300.0;
        let swarm = common::spawn_swarm_with_nanobots(
            &mut app,
            home,
            &[(NanobotType::Worker, 4), (NanobotType::Hauler, 3)],
        );
        app.world_mut()
            .entity_mut(swarm)
            .insert(StrategicController::new(owner));
        let foreign_home = Vec2::new(-2000.0, 0.0);
        let foreign_swarm = app
            .world_mut()
            .spawn((
                Swarm {},
                foreign,
                Transform::from_translation(foreign_home.extend(0.0)),
            ))
            .id();
        let mut obstacles = Vec::new();
        if split_work_ring {
            let rock = RockFormation::Rectangle {
                half: Vec2::new(22.0, 5000.0),
            };
            let transform = Transform::from_translation(deposit_position.extend(0.0));
            obstacles.push(rock.obstacle(&transform));
            app.world_mut().spawn((rock, transform));
            common::spawn_stockpile(&mut app, deposit_position + Vec2::X * 160.0, 0, 60);
        }
        let reachability = SwarmReachabilityMap::new(
            app.world().resource::<IntentGrid>(),
            obstacles,
            [(owner, home), (foreign, foreign_home)],
        )
        .unwrap();
        app.insert_resource(reachability);

        let [mine, _] = [opening, expansion].map(|cell| {
            common::spawn_deposit(
                &mut app,
                common::DepositFixture {
                    world_pos: common::cell_world_center(cell),
                    amount: 72_000,
                    capacity: 72_000,
                    radius: 64.0,
                },
            )
        });
        let second_mine = common::spawn_deposit(
            &mut app,
            common::DepositFixture {
                world_pos: deposit_position + Vec2::Y * 430.0,
                amount: 72_000,
                capacity: 72_000,
                radius: 64.0,
            },
        );
        app.world_mut()
            .entity_mut(second_mine)
            .insert(OwnerSwarm(foreign_swarm));
        let source = common::spawn_stockpile(&mut app, source_position, capacity - 4, capacity);
        let source_region =
            InteractionRegion::structure(app.world().get::<Transform>(source).unwrap());
        let deposit_approach = InteractionRegion::deposit(
            &Transform::from_translation(deposit_position.extend(0.0)),
            64.0,
        )
        .approach(source_position);
        let worker = common::spawn_worker_at(&mut app, deposit_approach);
        let other_deposit = matches!(deliveries, Deliveries::OtherMine);
        let delivery_worker = if delivery_owner == owner && !other_deposit {
            worker
        } else {
            let entity =
                common::spawn_worker_at(&mut app, source_region.approach(deposit_position));
            app.world_mut()
                .entity_mut(entity)
                .insert(SwarmMember::new(delivery_owner));
            entity
        };
        let origin = if other_deposit { second_mine } else { mine };
        let mut transfers = 0;
        for tick in 0..960 {
            let active_claim = LogisticsReservation::new(mine, source, ResourceKind::Minerals, 4);
            app.world_mut().entity_mut(worker).insert((
                GatherAssignment::new(opening, mine),
                ExtractProgress::default(),
                active_claim,
                Transform::from_translation(deposit_approach.extend(0.0)),
            ));
            let deliver = match deliveries {
                Deliveries::Burst => matches!(tick, 357 | 362),
                Deliveries::Stall => tick % 120 == 30 && tick < 300,
                _ => tick % 120 == 30,
            };
            if deliver {
                let mut reservation =
                    LogisticsReservation::new(origin, source, ResourceKind::Minerals, 4);
                reservation.source_remaining = 0;
                app.world_mut()
                    .entity_mut(delivery_worker)
                    .insert((
                        ReturningToStockpile { stockpile: source },
                        reservation,
                        Cargo {
                            kind: ResourceKind::Minerals,
                            amount: 1,
                        },
                        Transform::from_translation(
                            source_region.approach(deposit_position).extend(0.0),
                        ),
                    ))
                    .remove::<ExtractProgress>();
            }
            if matches!(deliveries, Deliveries::Stall) && tick == 300 {
                app.world_mut().get_mut::<Stockpile>(source).unwrap().amount = capacity;
            }
            if capacity > 4 {
                app.world_mut().get_mut::<Stockpile>(source).unwrap().amount = 0;
            }
            app.update();
            if deliver {
                let mut stock = app.world_mut().get_mut::<Stockpile>(source).unwrap();
                assert!(
                    stock.amount > if capacity == 4 { capacity - 4 } else { 0 },
                    "the Worker fixture must actually transfer its load to the Source"
                );
                transfers += 1;
                stock.amount -= 1;
            }
            let grid = app.world().resource::<IntentGrid>();
            assert!(
                grid.cell(opening)
                    .is_some_and(|cell| cell.has_owned(IntentKind::Gather, owner))
            );
            let expanded = grid
                .cell(expansion)
                .is_some_and(|cell| cell.has_owned(IntentKind::Gather, owner));
            if expanded {
                assert!(
                    should_expand && tick >= 12 * 60,
                    "unexpected claim at tick {tick} with capacity {capacity}, source deposits from {delivery_owner:?}, other deposit {other_deposit}"
                );
                break;
            }
            assert!(
                tick < 959 || !should_expand,
                "one active Worker with {transfers} completed drop-offs can sustain a four-unit reachable Source with its capacity reserved (split work ring {split_work_ring})"
            );
        }
    }
}

#[test]
fn controller_keeps_several_deposits_extracting_and_hauling() {
    use top_down_2d_rts_prototype_nano_swarm::{
        nanobot::{Cargo, HaulerAssignment, NanobotType, OwnerSwarm},
        resources::ResourceDeposit,
    };

    let mut app = common::sim_app_with_gather_haul();
    app.add_plugins(StrategicControllerPlugin);
    let home = common::cell_world_center(IVec2::ZERO);
    let own = common::spawn_swarm_with_nanobots(
        &mut app,
        home,
        &[(NanobotType::Worker, 9), (NanobotType::Hauler, 4)],
    );
    app.world_mut()
        .entity_mut(own)
        .insert(StrategicController::new(SwarmId::PLAYER));
    install_empty_reachability(&mut app, [SwarmId::PLAYER]);

    let cells = [IVec2::new(-1, 0), IVec2::new(1, 0), IVec2::new(0, 1)];
    for cell in cells {
        app.world_mut().resource_mut::<IntentGrid>().paint(
            cell,
            IntentKind::Gather,
            SwarmId::PLAYER,
        );
    }
    let sites = cells.map(|cell| {
        let position = common::cell_world_center(cell);
        let deposit = common::spawn_deposit(
            &mut app,
            common::DepositFixture {
                world_pos: position,
                amount: 72_000,
                capacity: 72_000,
                radius: 80.0,
            },
        );
        let source = common::spawn_stockpile(
            &mut app,
            position + (home - position).normalize() * 140.0,
            0,
            200,
        );
        app.world_mut().entity_mut(source).insert(OwnerSwarm(own));
        (deposit, source)
    });
    let sink = common::spawn_sink_stockpile(&mut app, home - Vec2::Y * 180.0, 0, 2_000);
    app.world_mut().entity_mut(sink).insert(OwnerSwarm(own));

    let mut loaded_from_site = [false; 3];
    for _ in 0..3600 {
        app.update();
        let world = app.world_mut();
        for (assignment, cargo) in world.query::<(&HaulerAssignment, &Cargo)>().iter(world) {
            for (index, (_, source)) in sites.iter().enumerate() {
                loaded_from_site[index] |= assignment.source == *source && cargo.amount > 0;
            }
        }
        if loaded_from_site.iter().all(|loaded| *loaded) {
            break;
        }
    }
    let world = app.world();
    let grid = world.resource::<IntentGrid>();
    for (index, (deposit, _)) in sites.into_iter().enumerate() {
        assert!(
            grid.cell(cells[index])
                .is_some_and(|cell| cell.has_owned(IntentKind::Gather, SwarmId::PLAYER)),
            "site {} lost Gather paint",
            cells[index]
        );
        assert!(world.get::<ResourceDeposit>(deposit).unwrap().amount < 72_000);
        assert!(
            loaded_from_site[index],
            "site {} never filled a hauler",
            cells[index]
        );
    }
}

#[test]
fn controller_retains_the_progressing_fronts_shared_defender_response() {
    use top_down_2d_rts_prototype_nano_swarm::nanobot::{
        DefenderResponse, OwnerSwarm, PlannedKind, Structure, StructureKind,
    };
    use top_down_2d_rts_prototype_nano_swarm::strategic_runtime::ControllerTelemetry;

    let mut app = common::sim_app();
    app.insert_resource(IntentGrid::new(64, 64));
    app.add_plugins(StrategicControllerPlugin);
    let home = common::cell_world_center(IVec2::ZERO);
    let own = common::spawn_swarm_at(&mut app, home);
    app.world_mut()
        .entity_mut(own)
        .insert(StrategicController::new(SwarmId::PLAYER));
    install_empty_reachability(&mut app, [SwarmId::PLAYER]);
    app.world_mut()
        .resource_mut::<top_down_2d_rts_prototype_nano_swarm::tuning::Tuning>()
        .autonomy
        .strategy_staging_spacing_cells = 64;
    let enemy_cell = IVec2::new(12, 0);
    let enemy = app
        .world_mut()
        .spawn((
            Swarm {},
            SwarmId(9),
            Transform::from_translation(common::cell_world_center(enemy_cell).extend(0.0)),
        ))
        .id();
    app.world_mut().resource_mut::<ResourceLedger>().add_for(
        SwarmId::PLAYER,
        ResourceKind::Minerals,
        2_000,
    );
    let support = common::spawn_charger(
        &mut app,
        common::ChargerFixture {
            cell: IVec2::ZERO,
            amount: 100,
            ticks_since_maintained: 0,
        },
    );
    app.world_mut().entity_mut(support).insert(OwnerSwarm(own));
    let old_front =
        common::spawn_empty_completed_structure(&mut app, enemy, PlannedKind::SinkStockpile);
    app.world_mut().entity_mut(old_front).insert((
        Structure::new(StructureKind::Basic),
        Transform::from_translation(common::cell_world_center(enemy_cell).extend(0.0)),
    ));
    let defenders = (0..8)
        .map(|index| {
            common::spawn_defender_at(
                &mut app,
                common::cell_world_center(enemy_cell)
                    + Vec2::new(-160.0, index as f32 * 48.0 - 168.0),
            )
        })
        .collect::<Vec<_>>();
    let mut original_responder = None;
    for _ in 0..DEFENDER_RESPONSE_TICK_BUDGET {
        app.update();
        original_responder = defenders.iter().copied().find(|defender| {
            app.world()
                .get::<DefenderResponse>(*defender)
                .is_some_and(|response| response.target == old_front)
        });
        if original_responder.is_some() {
            break;
        }
    }
    let original_responder = original_responder.unwrap_or_else(|| {
        panic!(
            "the existing front must have a real shared-autonomy response; profiles={:?}",
            app.world()
                .resource::<top_down_2d_rts_prototype_nano_swarm::strategic_runtime::ControllerTelemetry>()
                .profiles()
        )
    });

    app.world_mut()
        .get_mut::<Structure>(old_front)
        .unwrap()
        .health = 80;
    let new_front = common::spawn_charger(
        &mut app,
        common::ChargerFixture {
            cell: IVec2::new(3, 5),
            amount: 100,
            ticks_since_maintained: 0,
        },
    );
    app.world_mut()
        .entity_mut(new_front)
        .insert(OwnerSwarm(enemy));
    app.world_mut()
        .get_mut::<Structure>(new_front)
        .unwrap()
        .health = 20;
    let prior_reviews = app.world().resource::<ControllerTelemetry>().profiles()[&0].reviews;
    for _ in 0..DEFENDER_RESPONSE_TICK_BUDGET {
        app.update();
        if app.world().resource::<ControllerTelemetry>().profiles()[&0].reviews > prior_reviews {
            break;
        }
    }
    assert!(app.world().resource::<ControllerTelemetry>().profiles()[&0].reviews > prior_reviews);

    assert!(
        !app.world()
            .resource::<IntentGrid>()
            .cell(IVec2::new(3, 5))
            .unwrap()
            .has_owned(IntentKind::Defend, SwarmId::PLAYER),
        "progress at an active front must keep a competing target from taking its place"
    );
    assert_eq!(
        app.world()
            .get::<DefenderResponse>(original_responder)
            .map(|response| response.target),
        Some(old_front),
        "the progressing attack must preserve its shared-autonomy response"
    );
}

#[test]
fn economic_defend_paint_changes_real_defender_population_demand() {
    use top_down_2d_rts_prototype_nano_swarm::{
        nanobot::{NanobotType, PopulationDemand},
        strategic_controller::{Controller, GameState, StructureKind, StructureState, SwarmState},
        tuning::AutonomyTuning,
    };

    let mut app = common::sim_app_with_population_demand();
    app.insert_resource(IntentGrid::new(24, 24));
    let owner = SwarmId::PLAYER;
    let home = common::cell_world_center(IVec2::ZERO);
    app.world_mut().spawn((
        Swarm {},
        owner,
        Transform::from_translation(home.extend(0.0)),
    ));
    let tuning = AutonomyTuning::default();
    let mut controller = Controller::new(owner);
    let mut swarms = [SwarmState {
        id: owner,
        home,
        minerals: tuning.strategy_mineral_reserve,
        mineral_income_per_second: 0.0,
        production_saturated: false,
    }];
    let primary = StructureState {
        id: 1,
        owner,
        position: home,
        kind: StructureKind::Charger,
        operational: true,
        health: 100,
        max_health: 100,
        minerals: tuning.strategy_charger_reserve,
    };
    let chargers = [
        primary,
        StructureState {
            id: primary.id + 1,
            position: home + Vec2::Y * top_down_2d_rts_prototype_nano_swarm::ZONE_BLOCK_SIZE,
            ..primary
        },
    ];
    let mut demand_after_review = |app: &mut App, swarms: &[SwarmState], tick| {
        let decision = controller.decide(
            &tuning,
            &GameState {
                grid: app.world().resource::<IntentGrid>(),
                swarms,
                bots: &[],
                structures: &chargers,
                deposits: &[],
                terrain: &[],
                charge: &DEFAULT_CHARGE,
                reachability: None,
                route_query: None,
                mine_query: None,
                tick,
                finished: false,
            },
        );
        apply_decision(
            &mut app.world_mut().resource_mut::<IntentGrid>(),
            owner,
            &decision,
        );
        app.update();
        app.world()
            .resource::<PopulationDemand>()
            .desired_for(owner, NanobotType::Defender)
    };
    let initial = demand_after_review(&mut app, &swarms, 0);
    assert!(initial > 0, "initial paint creates real Defender demand");
    swarms[0].mineral_income_per_second = 12.0;
    let funded = demand_after_review(&mut app, &swarms, tuning.controller_review_period_ticks);
    assert!(
        funded > initial,
        "expanded paint increased demand from {initial} to {funded}"
    );
    swarms[0].minerals = 0;
    let depleted =
        demand_after_review(&mut app, &swarms, tuning.controller_review_period_ticks * 2);
    assert!(
        depleted < funded,
        "erasing unaffordable paint reduces real demand"
    );
}

#[test]
fn saturated_economy_supplies_build_room_for_real_production_pressure() {
    use top_down_2d_rts_prototype_nano_swarm::{
        nanobot::{NanobotType, OwnerSwarm, PlannedKind, PlannedStructure, ProductionFacility},
        terrain::RockFormation,
        tuning::Tuning,
    };

    for existing_facility in [true, false] {
        let mut app = common::sim_app_with_production();
        app.add_plugins(StrategicControllerPlugin);
        let owner = SwarmId::PLAYER;
        let home_cell = IVec2::ZERO;
        // Home is in a narrow reachable strip; a new facility needs a neighboring tile.
        let home = Vec2::new(496.0, 256.0);
        let rock = RockFormation::Rectangle {
            half: Vec2::new(236.0, 512.0),
        };
        let rock_transform = Transform::from_xyz(220.0, 512.0, 0.0);
        let swarm = app
            .world_mut()
            .spawn((
                Swarm {},
                owner,
                Transform::from_translation(home.extend(0.0)),
                StrategicController::new(owner),
            ))
            .id();
        app.world_mut().spawn((rock, rock_transform));
        common::spawn_worker_at(&mut app, Vec2::new(600.0, 256.0));
        let facility_site = common::cell_world_center(IVec2::new(2, 1));
        if existing_facility {
            let facility = common::spawn_facility_at(&mut app, swarm, facility_site);
            app.world_mut()
                .get_mut::<ProductionFacility>(facility)
                .unwrap()
                .current_target = Some(NanobotType::Defender);
        }
        app.world_mut()
            .resource_mut::<Tuning>()
            .production
            .pressure_ticks = 1;
        let threshold = app
            .world()
            .resource::<Tuning>()
            .autonomy
            .strategy_production_mineral_threshold;
        let reachability = SwarmReachabilityMap::new(
            app.world().resource::<IntentGrid>(),
            [rock.obstacle(&rock_transform)],
            [(owner, home)],
        )
        .unwrap();
        app.insert_resource(reachability);

        let reserve_site = common::cell_world_center(IVec2::new(2, 0));
        let reserve = common::spawn_stockpile(&mut app, reserve_site, threshold + 1, threshold + 1);
        app.world_mut()
            .entity_mut(reserve)
            .insert(OwnerSwarm(swarm));
        app.world_mut().resource_mut::<ResourceLedger>().add_for(
            owner,
            ResourceKind::Minerals,
            threshold + 1,
        );
        app.world_mut().despawn(reserve);

        app.update();
        let grid = app.world().resource::<IntentGrid>();
        let builds = grid
            .iter_active_cells()
            .filter(|(_, cell)| cell.has_owned(IntentKind::Build, owner))
            .map(|(cell, _)| cell)
            .collect::<Vec<_>>();
        assert_eq!(
            builds,
            [home_cell],
            "a destroyed reserve cannot fund extra Build room"
        );

        let reserve = common::spawn_stockpile(&mut app, reserve_site, threshold + 1, threshold + 1);
        app.world_mut()
            .entity_mut(reserve)
            .insert(OwnerSwarm(swarm));
        let mut planned_cell = None;
        for _ in 0..90 {
            app.update();
            let world = app.world_mut();
            planned_cell = world
                .query::<&PlannedStructure>()
                .iter(world)
                .find(|plan| plan.kind == PlannedKind::ProductionFacility)
                .map(|plan| plan.cell);
            if planned_cell.is_some() {
                break;
            }
        }
        let cell =
            planned_cell.expect("Production Pressure can use new room when busy or recovering");
        assert_eq!((cell - home_cell).abs().element_sum(), 1);
        assert!(
            app.world()
                .resource::<IntentGrid>()
                .cell(cell)
                .unwrap()
                .has_owned(IntentKind::Build, owner)
        );
    }
}

mod route_reuse {
    use super::*;
    use top_down_2d_rts_prototype_nano_swarm::{
        strategic_runtime::{ControllerTelemetry, reconcile_controller_tuning},
        terrain::RockFormation,
        tuning::Tuning,
    };

    const OWNER: SwarmId = SwarmId::PLAYER;

    /// A home and a deposit separated by a rock, so every review needs a route around it.
    fn rock_fixture() -> App {
        let mut app = common::minimal_app();
        app.insert_resource(IntentGrid::new(16, 16));
        app.add_plugins(StrategicControllerPlugin);
        let home = common::cell_world_center(IVec2::new(-3, 0));
        let rock = RockFormation::Rectangle {
            half: Vec2::new(256.0, 650.0),
        };
        let rock_transform =
            Transform::from_translation(common::cell_world_center(IVec2::ZERO).extend(0.0));
        app.world_mut().spawn((
            Swarm {},
            OWNER,
            Transform::from_translation(home.extend(0.0)),
            StrategicController::new(OWNER),
        ));
        app.world_mut().spawn((rock, rock_transform));
        common::spawn_deposit(
            &mut app,
            common::DepositFixture {
                world_pos: common::cell_world_center(IVec2::new(3, 0)),
                amount: 72_000,
                capacity: 72_000,
                radius: 80.0,
            },
        );
        let reachability = SwarmReachabilityMap::new(
            app.world().resource::<IntentGrid>(),
            [rock.obstacle(&rock_transform)],
            [(OWNER, home)],
        )
        .unwrap();
        app.insert_resource(reachability);
        app.update();
        app
    }

    fn work(app: &App) -> u64 {
        app.world().resource::<ControllerTelemetry>().profiles()[&OWNER.0].work_units
    }

    /// Force one review through a tuning edit and return the work units it spent.
    fn forced_review_work(app: &mut App) -> u64 {
        let before = work(app);
        let old = app.world().resource::<Tuning>().clone();
        let mut new = old.clone();
        new.autonomy.strategy_home_income_lead += 1.0;
        reconcile_controller_tuning(app.world_mut(), &old, &new);
        app.insert_resource(new);
        app.update();
        work(app) - before
    }

    #[test]
    fn revision_bump_from_a_structure_off_the_route_costs_no_extra_search() {
        let mut app = rock_fixture();
        let steady = forced_review_work(&mut app);
        common::spawn_structure_at(&mut app, Vec2::new(256.0, 1000.0));
        assert_eq!(forced_review_work(&mut app), steady);
    }

    #[test]
    fn structure_across_the_cached_route_forces_a_new_search() {
        let mut app = rock_fixture();
        let steady = forced_review_work(&mut app);
        common::spawn_structure_at(&mut app, Vec2::new(256.0, -450.0));
        assert!(forced_review_work(&mut app) > steady);
    }

    #[test]
    fn repeated_review_of_the_same_tiles_spends_less_once_planning_points_are_cached() {
        use top_down_2d_rts_prototype_nano_swarm::{
            nanobot::NanobotType,
            strategic_controller::{BotState, Controller, GameState, SwarmState},
            tuning::AutonomyTuning,
        };
        let grid = IntentGrid::new(64, 64);
        let enemy = SwarmId(9);
        let world = |cell: IVec2| common::cell_world_center(cell);
        let swarm = |id, cell| SwarmState {
            id,
            home: world(cell),
            minerals: 100,
            mineral_income_per_second: 0.0,
            production_saturated: false,
        };
        let swarms = [swarm(OWNER, IVec2::ZERO), swarm(enemy, IVec2::new(24, 24))];
        let remnant = BotState {
            id: 50,
            owner: enemy,
            kind: NanobotType::Worker,
            position: world(IVec2::new(12, 3)),
            health: 20,
            max_health: 100,
            charge: 1.0,
            max_charge: 1.0,
            cargo: 0,
            displacement: Vec2::ZERO,
        };
        let rock = RockFormation::Rectangle {
            half: Vec2::new(256.0, 650.0),
        };
        let rock_transform = Transform::from_translation(world(IVec2::new(2, 0)).extend(0.0));
        let terrain = [rock.obstacle(&rock_transform)];
        let homes = [
            (OWNER, world(IVec2::ZERO)),
            (enemy, world(IVec2::new(24, 24))),
        ];
        let map = SwarmReachabilityMap::new(&grid, terrain, homes).unwrap();
        // Warm the cell reachability memo so the first review's cost is geometry only.
        for owner in [OWNER, enemy] {
            for y in -32..32 {
                for x in -32..32 {
                    map.reachable_point(owner, IVec2::new(x, y), &grid);
                }
            }
        }
        let tuning = AutonomyTuning::default();
        let state = GameState {
            reachability: Some(&map),
            terrain: &terrain,
            grid: &grid,
            swarms: &swarms,
            bots: &[remnant],
            structures: &[],
            deposits: &[],
            charge: &DEFAULT_CHARGE,
            route_query: None,
            mine_query: None,
            tick: 0,
            finished: false,
        };
        let first = Controller::new(OWNER).decide(&tuning, &state);
        let second = Controller::new(OWNER).decide(&tuning, &state);
        assert!(first.reviewed && second.reviewed);
        assert!(second.work_units < first.work_units);
    }
}
