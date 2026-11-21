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
    };
    let charger = StructureState {
        id: 42,
        owner,
        position: world(IVec2::new(1, 0)),
        kind: StructureKind::Charger,
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
