#[path = "../common/mod.rs"]
mod common;

use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    intent::{IntentGrid, IntentKind},
    nanobot::{
        CombatPlugin, MatchOutcome, Nanobot, NanobotType, OwnerSwarm, ProductionFacility,
        StrategicController, StrategicControllerPlugin, SwarmEliminationPlugin, SwarmId,
        SwarmMember, nanobot_death_cleanup_system,
    },
    reachability::SwarmReachabilityMap,
    tuning::Tuning,
    ui::{
        FontsResource, FpsText, NanoswarmUiSetupPlugin,
        match_banner::{
            MatchBannerDismiss, MatchBannerRoot, MatchBannerText, setup_match_banner,
            update_match_banner_system,
        },
    },
};

fn install_empty_reachability(app: &mut App) {
    let reachability = {
        let grid = app.world().resource::<IntentGrid>();
        SwarmReachabilityMap::new(grid, [], [(SwarmId::PLAYER, Vec2::ZERO)])
            .expect("the empty strategic playtest map has a reachable player home")
    };
    app.insert_resource(reachability);
}

#[test]
fn strategic_intent_finishes_a_remnant_away_from_its_former_home() {
    use std::time::Duration;

    use bevy::time::TimeUpdateStrategy;
    use top_down_2d_rts_prototype_nano_swarm::{
        nanobot::{ChargePlugin, DefenderResponse},
        tuning::Tuning,
    };
    let mut app = common::sim_app();
    let cadence = Duration::from_secs_f64(1.0 / 60.0);
    app.insert_resource(Time::<Fixed>::from_duration(cadence))
        .insert_resource(TimeUpdateStrategy::ManualDuration(cadence))
        .add_plugins((CombatPlugin, ChargePlugin, StrategicControllerPlugin))
        .add_systems(FixedLast, nanobot_death_cleanup_system);
    app.update();
    app.insert_resource(Tuning::default());
    let home = common::cell_world_center(IVec2::ZERO);
    let remnant_cell = IVec2::new(0, 3);
    let remnant_position = common::cell_world_center(remnant_cell);
    let own = common::spawn_swarm_with_nanobots(&mut app, home, &[(NanobotType::Defender, 2)]);
    app.world_mut()
        .entity_mut(own)
        .insert(StrategicController::new(SwarmId::PLAYER));
    install_empty_reachability(&mut app);
    let enemy = common::spawn_opponent_swarm_with_nanobots(
        &mut app,
        remnant_position,
        &[(NanobotType::Worker, 1)],
    );
    let enemy_id = *app.world().get::<SwarmId>(enemy).unwrap();
    app.world_mut()
        .entity_mut(enemy)
        .insert(Transform::from_translation(
            common::cell_world_center(IVec2::new(3, 0)).extend(0.0),
        ));
    let mut first_response = None;
    for tick in 1..=1200 {
        app.update();
        assert_eq!(
            *app.world().resource::<Tuning>(),
            Tuning::default(),
            "the playtest must exercise shipped pacing"
        );
        assert!(
            (app.world()
                .resource::<Time<Fixed>>()
                .timestep()
                .as_secs_f64()
                - 1.0 / 60.0)
                .abs()
                < 1e-8,
            "the playtest's tick-to-second conversion requires the real 60 Hz cadence"
        );
        if first_response.is_none()
            && app
                .world_mut()
                .query::<&DefenderResponse>()
                .iter(app.world())
                .next()
                .is_some()
        {
            first_response = Some(tick);
        }
        if !enemy_nanobots_remain(&mut app, enemy_id) {
            println!(
                "first response at tick {}, elimination after {:.3} simulated seconds",
                first_response.unwrap_or_default(),
                f64::from(tick) / 60.0
            );
            assert!(
                first_response.is_some_and(|tick| tick <= 2),
                "useful owned intent must promptly produce a shared-autonomy response: {first_response:?}"
            );
            assert!(
                tick > 60,
                "travel and combat must not become an instant remote kill"
            );
            break;
        }
    }
    assert!(
        !enemy_nanobots_remain(&mut app, enemy_id),
        "controller must find the real remnant rather than only its former home"
    );
}

fn enemy_nanobots_remain(app: &mut App, enemy: SwarmId) -> bool {
    app.world_mut()
        .query::<&SwarmMember>()
        .iter(app.world())
        .any(|member| member.0 == enemy)
}

#[test]
fn strategic_intent_finishes_a_hauler_on_the_reachable_flanks_rim() {
    use top_down_2d_rts_prototype_nano_swarm::{
        navigation::Navigation, scenario::default_rock_geometry,
    };

    let mut app = common::sim_app();
    app.insert_resource(IntentGrid::new(64, 64))
        .add_plugins((CombatPlugin, StrategicControllerPlugin))
        .add_systems(FixedLast, nanobot_death_cleanup_system);
    let home = Vec2::new(-200.0, 3269.0374);
    let hauler_position = Vec2::new(-520.12854, 3269.0374);
    let hauler_cell = IVec2::new(-2, 6);
    let player = common::spawn_swarm_with_nanobots(&mut app, home, &[(NanobotType::Defender, 2)]);
    app.world_mut()
        .entity_mut(player)
        .insert(StrategicController::new(SwarmId::PLAYER));
    let enemy = common::spawn_opponent_swarm_with_nanobots(
        &mut app,
        hauler_position,
        &[(NanobotType::Hauler, 1)],
    );
    let enemy_id = *app.world().get::<SwarmId>(enemy).unwrap();
    let geometry = default_rock_geometry();
    let terrain = geometry
        .iter()
        .map(|(rock, transform)| rock.obstacle(transform))
        .collect::<Vec<_>>();
    let grid = app.world().resource::<IntentGrid>();
    let navigation = Navigation::new(grid, terrain.clone());
    assert!(!navigation.point_clear(common::cell_world_center(hauler_cell)));
    assert!(navigation.point_clear(hauler_position));
    let reachability = SwarmReachabilityMap::new(grid, terrain, [(SwarmId::PLAYER, home)]).unwrap();
    app.insert_resource(reachability);
    for rock in geometry {
        app.world_mut().spawn(rock);
    }

    app.update();
    assert!(
        app.world()
            .resource::<IntentGrid>()
            .cell(hauler_cell)
            .is_some_and(|cell| cell.has_owned(IntentKind::Defend, SwarmId::PLAYER))
    );
    for _ in 0..600 {
        if !enemy_nanobots_remain(&mut app, enemy_id) {
            break;
        }
        app.update();
    }
    assert!(!enemy_nanobots_remain(&mut app, enemy_id));
}

#[test]
fn strategic_intent_clears_a_structure_and_distant_hauler() {
    use std::time::Duration;

    use bevy::time::TimeUpdateStrategy;
    use top_down_2d_rts_prototype_nano_swarm::{
        nanobot::{ChargePlugin, PlannedKind, Structure, StructureKind},
        resources::{ResourceKind, ResourceLedger},
        terrain::RockFormation,
        tuning::Tuning,
    };

    let mut app = common::sim_app();
    let cadence = Duration::from_secs_f64(1.0 / 60.0);
    app.insert_resource(IntentGrid::new(64, 64))
        .insert_resource(Time::<Fixed>::from_duration(cadence))
        .insert_resource(TimeUpdateStrategy::ManualDuration(cadence))
        .add_plugins((
            CombatPlugin,
            ChargePlugin,
            StrategicControllerPlugin,
            SwarmEliminationPlugin,
        ))
        .add_systems(FixedLast, nanobot_death_cleanup_system);
    app.update();
    app.insert_resource(Tuning::default());

    let home = common::cell_world_center(IVec2::ZERO);
    let own = common::spawn_swarm_with_nanobots(&mut app, home, &[(NanobotType::Defender, 8)]);
    app.world_mut()
        .entity_mut(own)
        .insert(StrategicController::new(SwarmId::PLAYER));
    app.world_mut().resource_mut::<ResourceLedger>().add_for(
        SwarmId::PLAYER,
        ResourceKind::Minerals,
        2_000,
    );
    let support = common::spawn_charger(
        &mut app,
        common::ChargerFixture {
            cell: IVec2::new(-1, 0),
            amount: 100,
            ticks_since_maintained: 0,
        },
    );
    app.world_mut().entity_mut(support).insert(OwnerSwarm(own));

    let hauler_start = common::cell_world_center(IVec2::new(0, 4));
    let rock = RockFormation::Rectangle {
        half: Vec2::new(1_024.0, 72.0),
    };
    let rock_transform = Transform::from_translation((hauler_start + Vec2::Y * 280.0).extend(0.0));
    app.world_mut().spawn((rock, rock_transform));
    let reachability = SwarmReachabilityMap::new(
        app.world().resource::<IntentGrid>(),
        [rock.obstacle(&rock_transform)],
        [(SwarmId::PLAYER, home)],
    )
    .unwrap();
    app.insert_resource(reachability);
    let enemy = common::spawn_opponent_swarm_with_nanobots(
        &mut app,
        hauler_start,
        &[(NanobotType::Hauler, 1)],
    );
    let enemy_id = *app.world().get::<SwarmId>(enemy).unwrap();
    let hauler = {
        let world = app.world_mut();
        world
            .query::<(Entity, &SwarmMember)>()
            .iter(world)
            .find(|(_, member)| member.0 == enemy_id)
            .map(|(entity, _)| entity)
            .unwrap()
    };
    let enemy_home = common::cell_world_center(IVec2::new(6, 0));
    app.world_mut()
        .entity_mut(enemy)
        .insert(Transform::from_translation(enemy_home.extend(0.0)));
    let structure =
        common::spawn_empty_completed_structure(&mut app, enemy, PlannedKind::SinkStockpile);
    app.world_mut().entity_mut(structure).insert((
        Structure::new(StructureKind::Basic),
        Transform::from_translation(enemy_home.extend(0.0)),
    ));

    let started = app.world().resource::<Time<Fixed>>().elapsed_secs_f64();
    for _ in 0..2400 {
        app.update();
        if *app.world().resource::<MatchOutcome>() == MatchOutcome::Winner(SwarmId::PLAYER) {
            break;
        }
    }

    let elapsed = app.world().resource::<Time<Fixed>>().elapsed_secs_f64() - started;
    assert_eq!(
        *app.world().resource::<MatchOutcome>(),
        MatchOutcome::Winner(SwarmId::PLAYER),
        "mixed cleanup must finish through shared movement and combat within forty simulated seconds; structure health={:?}; Hauler health={:?}, position={:?}",
        app.world()
            .get::<Structure>(structure)
            .map(|structure| structure.health),
        app.world()
            .get::<top_down_2d_rts_prototype_nano_swarm::nanobot::Health>(hauler)
            .map(|health| health.current),
        app.world()
            .get::<Transform>(hauler)
            .map(|transform| transform.translation)
    );
    assert!(elapsed > 1.0, "distant targets require physical travel");
    assert!(app.world().get_entity(structure).is_err());
    assert!(app.world().get_entity(hauler).is_err());
    println!("mixed-target cleanup after {elapsed:.3} simulated seconds");
}

#[test]
fn strategic_intent_destroys_an_exposed_production_facility_that_spawns_reinforcements() {
    use std::time::Duration;

    use bevy::time::TimeUpdateStrategy;
    use top_down_2d_rts_prototype_nano_swarm::{
        nanobot::{ChargePlugin, Structure, StructureKind},
        resources::{ResourceKind, ResourceLedger},
        tuning::Tuning,
    };

    let mut app = common::sim_app_with_production();
    let cadence = Duration::from_secs_f64(1.0 / 60.0);
    app.insert_resource(IntentGrid::new(64, 64))
        .insert_resource(Time::<Fixed>::from_duration(cadence))
        .insert_resource(TimeUpdateStrategy::ManualDuration(cadence))
        .add_plugins((
            CombatPlugin,
            ChargePlugin,
            StrategicControllerPlugin,
            SwarmEliminationPlugin,
        ))
        .add_systems(FixedLast, nanobot_death_cleanup_system);
    app.update();
    app.insert_resource(Tuning::default());

    let home = common::cell_world_center(IVec2::ZERO);
    let target_cell = IVec2::new(6, 0);
    let target = common::cell_world_center(target_cell);
    let own = common::spawn_swarm_with_nanobots(&mut app, home, &[(NanobotType::Defender, 8)]);
    app.world_mut().resource_mut::<ResourceLedger>().add_for(
        SwarmId::PLAYER,
        ResourceKind::Minerals,
        2_000,
    );
    let support = common::spawn_charger(
        &mut app,
        common::ChargerFixture {
            cell: IVec2::new(-1, 0),
            amount: 100,
            ticks_since_maintained: 0,
        },
    );
    app.world_mut().entity_mut(support).insert(OwnerSwarm(own));

    let enemy = common::spawn_opponent_swarm_with_nanobots(&mut app, target, &[]);
    let enemy_id = *app.world().get::<SwarmId>(enemy).unwrap();
    app.world_mut()
        .entity_mut(enemy)
        .insert(Transform::from_translation(
            common::cell_world_center(target_cell + IVec2::X * 3).extend(0.0),
        ));
    let mine = common::spawn_deposit(
        &mut app,
        common::DepositFixture {
            world_pos: common::cell_world_center(target_cell + IVec2::Y),
            amount: 500,
            capacity: 500,
            radius: 80.0,
        },
    );
    app.world_mut().entity_mut(mine).insert(OwnerSwarm(enemy));
    let facility = common::spawn_facility_at(&mut app, enemy, target);
    app.world_mut()
        .entity_mut(facility)
        .insert(Structure::new(StructureKind::Basic));
    {
        let mut grid = app.world_mut().resource_mut::<IntentGrid>();
        for offset in [IVec2::ZERO, IVec2::X, IVec2::Y, IVec2::ONE] {
            grid.paint(target_cell + offset, IntentKind::Defend, enemy_id);
        }
    }

    let mut production_underway = false;
    for _ in 0..90 {
        app.update();
        let enemy_population = app
            .world_mut()
            .query_filtered::<&SwarmMember, With<Nanobot>>()
            .iter(app.world())
            .filter(|member| member.0 == enemy_id)
            .count();
        assert_eq!(
            enemy_population, 0,
            "warm-up must stop before the first reinforcement is born"
        );
        let state = app.world().get::<ProductionFacility>(facility).unwrap();
        if state.current_target == Some(NanobotType::Defender) && state.progress >= 60 {
            production_underway = true;
            break;
        }
    }
    assert!(
        production_underway,
        "real enemy Defend demand must start a Defender production cycle"
    );
    app.world_mut()
        .entity_mut(own)
        .insert(StrategicController::new(SwarmId::PLAYER));
    install_empty_reachability(&mut app);

    let mut first_birth_tick = None;
    let mut facility_health_at_birth = None;
    let mut facility_progressed_after_birth = false;
    let mut victory_tick = None;
    for tick in 1..=3_600 {
        app.update();
        if tick == 1 {
            assert!(
                app.world()
                    .resource::<IntentGrid>()
                    .iter_active_cells()
                    .any(|(cell, intent)| {
                        (0..target_cell.x).contains(&cell.x)
                            && intent.has_owned(IntentKind::Defend, SwarmId::PLAYER)
                    }),
                "the attack on the producer needs a reachable staging block"
            );
        }

        let enemy_population = app
            .world_mut()
            .query_filtered::<&SwarmMember, With<Nanobot>>()
            .iter(app.world())
            .filter(|member| member.0 == enemy_id)
            .count();
        if first_birth_tick.is_none() && enemy_population > 0 {
            first_birth_tick = Some(tick);
            facility_health_at_birth = app
                .world()
                .get::<Structure>(facility)
                .map(|structure| structure.health);
        }
        if let Some(health_at_birth) = facility_health_at_birth {
            facility_progressed_after_birth |= app
                .world()
                .get::<Structure>(facility)
                .is_none_or(|structure| structure.health < health_at_birth);
        }
        if *app.world().resource::<MatchOutcome>() == MatchOutcome::Winner(SwarmId::PLAYER) {
            victory_tick = Some(tick);
            break;
        }
    }

    assert!(
        first_birth_tick.is_some(),
        "the underway real production cycle must release a reinforcement after attack intent starts"
    );
    assert!(
        facility_progressed_after_birth,
        "the attack must continue damaging the exposed producer after a new target appears"
    );
    assert!(
        app.world().get_entity(facility).is_err(),
        "the exact exposed producer must be destroyed"
    );
    assert_eq!(
        *app.world().resource::<MatchOutcome>(),
        MatchOutcome::Winner(SwarmId::PLAYER),
        "producer and natural reinforcements must be eliminated within sixty simulated seconds"
    );
    println!(
        "reinforcement at tick {}, natural victory at tick {}",
        first_birth_tick.unwrap(),
        victory_tick.unwrap()
    );
}

#[test]
fn strategic_front_advances_when_shared_movement_reaches_the_staging_block() {
    use std::time::Duration;

    use bevy::time::TimeUpdateStrategy;
    use top_down_2d_rts_prototype_nano_swarm::{
        nanobot::{PlannedKind, Structure, StructureKind},
        resources::{ResourceKind, ResourceLedger},
    };

    // Movement and allocation execute normally; omitting combat keeps the objective alive.
    let mut app = common::sim_app();
    let cadence = Duration::from_secs_f64(1.0 / 60.0);
    app.insert_resource(IntentGrid::new(64, 64))
        .insert_resource(Time::<Fixed>::from_duration(cadence))
        .insert_resource(TimeUpdateStrategy::ManualDuration(cadence))
        .add_plugins(StrategicControllerPlugin);
    app.update();

    let home_cell = IVec2::ZERO;
    let home = common::cell_world_center(home_cell);
    let own = common::spawn_swarm_with_nanobots(&mut app, home, &[(NanobotType::Defender, 8)]);
    app.world_mut()
        .entity_mut(own)
        .insert(StrategicController::new(SwarmId::PLAYER));
    install_empty_reachability(&mut app);
    app.world_mut().resource_mut::<ResourceLedger>().add_for(
        SwarmId::PLAYER,
        ResourceKind::Minerals,
        2_000,
    );
    {
        let world = app.world_mut();
        let offsets = [
            (-144.0, -144.0),
            (0.0, -144.0),
            (144.0, -144.0),
            (-144.0, 0.0),
            (144.0, 0.0),
            (-144.0, 144.0),
            (0.0, 144.0),
            (144.0, 144.0),
        ];
        for (mut transform, (x, y)) in world
            .query_filtered::<&mut Transform, With<Nanobot>>()
            .iter_mut(world)
            .zip(offsets)
        {
            transform.translation = (home + Vec2::new(x, y)).extend(0.0);
        }
    }
    let support = common::spawn_charger(
        &mut app,
        common::ChargerFixture {
            cell: IVec2::new(-1, 0),
            amount: 100,
            ticks_since_maintained: 0,
        },
    );
    app.world_mut().entity_mut(support).insert(OwnerSwarm(own));
    let gather_cell = IVec2::new(-1, -1);
    let deposit = common::spawn_deposit(
        &mut app,
        common::DepositFixture {
            world_pos: common::cell_world_center(gather_cell),
            amount: 72_000,
            capacity: 72_000,
            radius: 64.0,
        },
    );
    app.world_mut().entity_mut(deposit).insert(OwnerSwarm(own));
    let target_cell = IVec2::new(9, 0);
    let target_position = common::cell_world_center(target_cell);
    let enemy = common::spawn_opponent_swarm_with_nanobots(&mut app, target_position, &[]);
    let sink = common::spawn_empty_completed_structure(&mut app, enemy, PlannedKind::SinkStockpile);
    app.world_mut().entity_mut(sink).insert((
        Structure::new(StructureKind::Basic),
        Transform::from_translation(target_position.extend(0.0)),
    ));

    app.update();
    let first_stage = {
        let grid = app.world().resource::<IntentGrid>();
        let first = grid
            .iter_active_cells()
            .find(|(cell, intent)| {
                cell.x >= 2
                    && intent.has_owned(IntentKind::Build, SwarmId::PLAYER)
                    && intent.has_owned(IntentKind::Defend, SwarmId::PLAYER)
            })
            .expect("a staging block and its Charger Build must lead the army");
        assert!(first.0.x < target_cell.x);
        assert!(
            !grid
                .cell(target_cell)
                .unwrap()
                .has_owned(IntentKind::Defend, SwarmId::PLAYER)
        );
        first.0
    };

    let mut advanced_defenders = 0;
    for _ in 0..5_400 {
        app.update();
        let world = app.world_mut();
        let target_support = world
            .resource::<IntentGrid>()
            .cell(target_cell)
            .is_some_and(|cell| {
                cell.has_owned(IntentKind::Build, SwarmId::PLAYER)
                    && cell.has_owned(IntentKind::Defend, SwarmId::PLAYER)
            });
        if !target_support {
            continue;
        }
        advanced_defenders = world
            .query::<(&SwarmMember, &NanobotType, &Transform)>()
            .iter(world)
            .filter(|(member, kind, transform)| {
                member.0 == SwarmId::PLAYER
                    && **kind == NanobotType::Defender
                    && transform.translation.truncate().distance(target_position)
                        <= 2.0 * top_down_2d_rts_prototype_nano_swarm::ZONE_BLOCK_SIZE
            })
            .count();
        if advanced_defenders >= 4 {
            break;
        }
    }
    let world = app.world();
    let profiles = world
        .resource::<top_down_2d_rts_prototype_nano_swarm::strategic_runtime::ControllerTelemetry>()
        .profiles();
    assert!(
        advanced_defenders >= 4,
        "shared movement must advance the main force, got {advanced_defenders} ({})",
        profiles[&SwarmId::PLAYER.0].last_explanation
    );
    let grid = world.resource::<IntentGrid>();
    assert!(
        grid.iter_active_cells().any(|(cell, intent)| {
            intent.has_owned(IntentKind::Build, SwarmId::PLAYER)
                && (cell - target_cell).abs().max_element() <= 2
                && (cell - home_cell).abs().max_element() >= 3
        }),
        "Charger Build must accompany the final staging block"
    );
    assert!(
        !grid
            .cell(first_stage)
            .unwrap()
            .has_owned(IntentKind::Defend, SwarmId::PLAYER),
        "the early staging point must be erased"
    );
    assert!(
        grid.cell(home_cell)
            .unwrap()
            .has_owned(IntentKind::Build, SwarmId::PLAYER)
    );
    assert!(
        grid.cell(gather_cell)
            .unwrap()
            .has_owned(IntentKind::Gather, SwarmId::PLAYER)
    );
    assert!(
        grid.cell(target_cell)
            .unwrap()
            .has_owned(IntentKind::Defend, SwarmId::PLAYER)
    );
}

#[test]
fn mutual_final_combat_deaths_show_a_dismissible_draw_in_the_same_tick() {
    use top_down_2d_rts_prototype_nano_swarm::nanobot::Health;
    let mut app = common::sim_app();
    app.add_plugins((CombatPlugin, SwarmEliminationPlugin))
        .add_systems(FixedLast, nanobot_death_cleanup_system)
        .insert_resource(FontsResource { font: default() })
        .add_systems(Startup, setup_match_banner)
        .add_systems(Update, update_match_banner_system);
    // Keep both first shots on the same tick so the test exercises mutual elimination.
    app.world_mut()
        .resource_mut::<Tuning>()
        .combat
        .attack_interval_ticks = 1;
    let center = common::cell_world_center(IVec2::ZERO);
    common::spawn_swarm_with_nanobots(
        &mut app,
        center - Vec2::X * 36.0,
        &[(NanobotType::Defender, 1)],
    );
    let opponent = common::spawn_opponent_swarm_with_nanobots(
        &mut app,
        center + Vec2::X * 36.0,
        &[(NanobotType::Defender, 1)],
    );
    let opponent_id = *app.world().get::<SwarmId>(opponent).unwrap();
    for mut health in app
        .world_mut()
        .query::<&mut Health>()
        .iter_mut(app.world_mut())
    {
        health.current = 1;
    }
    for owner in [SwarmId::PLAYER, opponent_id] {
        app.world_mut()
            .resource_mut::<IntentGrid>()
            .paint(IVec2::ZERO, IntentKind::Defend, owner);
    }
    app.update();
    assert_eq!(
        app.world_mut()
            .query_filtered::<Entity, With<Nanobot>>()
            .iter(app.world())
            .count(),
        0,
        "both final Defenders must die through the real combat and cleanup systems"
    );
    assert_eq!(*app.world().resource::<MatchOutcome>(), MatchOutcome::Draw);
    assert_eq!(
        app.world_mut()
            .query_filtered::<&Text, With<MatchBannerText>>()
            .single(app.world())
            .unwrap()
            .0,
        "DRAW\nBoth Swarms Eliminated"
    );
    assert!(
        app.world_mut()
            .query::<&Text>()
            .iter(app.world())
            .any(|text| text.0 == "Esc for menu")
    );
    let dismiss_button = app
        .world_mut()
        .query_filtered::<Entity, (With<Button>, With<MatchBannerDismiss>)>()
        .single(app.world())
        .expect("match outcome can be dismissed");
    common::press_button(&mut app, dismiss_button);
    assert_eq!(*app.world().resource::<MatchOutcome>(), MatchOutcome::Draw);
    assert_eq!(
        app.world_mut()
            .query_filtered::<&Visibility, With<MatchBannerRoot>>()
            .single(app.world())
            .unwrap(),
        &Visibility::Hidden
    );
}

fn app_with_hud() -> App {
    use bevy::diagnostic::FrameCountPlugin;
    use top_down_2d_rts_prototype_nano_swarm::intent::BrushSelection;

    let mut app = common::minimal_app();
    app.add_plugins((
        TaskPoolPlugin::default(),
        AssetPlugin::default(),
        FrameCountPlugin,
    ))
    .init_asset::<Font>()
    .init_asset_loader::<bevy::text::FontLoader>()
    .init_resource::<ButtonInput<KeyCode>>()
    .init_resource::<BrushSelection>()
    .add_plugins(NanoswarmUiSetupPlugin);
    app
}

fn hud_shows(app: &mut App, message: &str) -> bool {
    app.world_mut()
        .query::<(&Text, &Node)>()
        .iter(app.world())
        .any(|(text, node)| node.display != Display::None && text.0.contains(message))
}

fn visible_combat_alert(app: &mut App) -> Option<BackgroundColor> {
    app.world_mut()
        .query::<(&Name, &Node, &BackgroundColor, Option<&Interaction>)>()
        .iter(app.world())
        .find_map(|(name, node, background, interaction)| {
            (name.as_str() == "Combat alert" && node.display != Display::None).then(|| {
                assert!(interaction.is_none(), "the combat alert is passive");
                *background
            })
        })
}

#[test]
fn swarm_hud_shows_distinct_graphical_combat_alerts_and_delivery_status() {
    use top_down_2d_rts_prototype_nano_swarm::nanobot::{
        ActionableProjection, Health, PopulationDemandPlugin, TerritorySnapshot,
        population_demand_system, project_territory_snapshot_system,
    };

    let mut app = app_with_hud();
    app.init_resource::<ActionableProjection>()
        .init_resource::<TerritorySnapshot>()
        .add_plugins((CombatPlugin, SwarmEliminationPlugin, PopulationDemandPlugin))
        .add_systems(
            FixedUpdate,
            project_territory_snapshot_system.before(population_demand_system),
        )
        .add_systems(FixedLast, nanobot_death_cleanup_system);
    app.world_mut()
        .resource_mut::<Tuning>()
        .combat
        .attack_interval_ticks = 1;
    let home = common::cell_world_center(IVec2::ZERO);
    let player = common::spawn_swarm_with_nanobots(&mut app, home, &[]);
    let worker = common::spawn_worker_at(&mut app, home);
    let opponent = common::spawn_opponent_swarm_with_nanobots(&mut app, home, &[]);
    common::spawn_home_facility(&mut app, opponent);
    let opponent_id = *app.world().get::<SwarmId>(opponent).unwrap();
    let intruder = common::spawn_defender_at(&mut app, home + Vec2::X * 160.0);
    app.world_mut()
        .entity_mut(intruder)
        .insert(SwarmMember::new(opponent_id));
    let mut producing = ProductionFacility::new();
    producing.current_target = Some(NanobotType::Hauler);
    let facility = app
        .world_mut()
        .spawn((
            producing,
            OwnerSwarm(player),
            Transform::from_translation((home + Vec2::Y * 256.0).extend(0.0)),
        ))
        .id();
    app.world_mut().resource_mut::<IntentGrid>().paint(
        IVec2::ZERO,
        IntentKind::Build,
        SwarmId::PLAYER,
    );

    app.update();
    let intrusion = visible_combat_alert(&mut app).expect("a hostile is inside player territory");
    assert!(!hud_shows(&mut app, "Hostiles in your territory"));
    assert!(hud_shows(&mut app, "Workers: 1 / 0"));
    assert!(hud_shows(&mut app, "Production: 1 Hauler"));
    let starting_health = app.world().get::<Health>(worker).unwrap().current;

    app.world_mut()
        .get_mut::<Transform>(intruder)
        .unwrap()
        .translation = (home + Vec2::X * 72.0).extend(0.0);
    app.update();
    assert!(app.world().get::<Health>(worker).unwrap().current < starting_health);
    let under_attack = visible_combat_alert(&mut app).expect("the player swarm took damage");
    assert_ne!(under_attack, intrusion);
    assert!(!hud_shows(&mut app, "Swarm under attack"));

    app.world_mut().get_mut::<Health>(worker).unwrap().current = 1;
    app.world_mut()
        .get_mut::<ProductionFacility>(facility)
        .unwrap()
        .current_target = None;
    app.update();
    assert!(app.world().get_entity(worker).is_err());
    assert_eq!(
        *app.world().resource::<MatchOutcome>(),
        MatchOutcome::InProgress
    );
    assert!(hud_shows(&mut app, "No nanobots remaining"));
    assert!(hud_shows(&mut app, "Production: no Haulers to deliver"));
    assert!(visible_combat_alert(&mut app).is_none());
}

#[test]
fn f3_shows_and_hides_the_fps_counter() {
    let mut app = app_with_hud();
    app.update();
    let counter = app
        .world_mut()
        .query_filtered::<Entity, With<FpsText>>()
        .single(app.world())
        .unwrap();
    assert_eq!(
        app.world().get::<Visibility>(counter),
        Some(&Visibility::Hidden)
    );

    for expected in [Visibility::Visible, Visibility::Hidden] {
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::F3);
        app.update();
        assert_eq!(app.world().get::<Visibility>(counter), Some(&expected));
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .reset(KeyCode::F3);
    }
}

#[test]
fn strategic_opening_keeps_neutral_gather_pending_through_the_first_extraction_pulse() {
    use top_down_2d_rts_prototype_nano_swarm::resources::{ResourceDeposit, Stockpile};

    let mut app = common::sim_app_with_gather_haul();
    app.insert_resource(IntentGrid::new(16, 16))
        .add_plugins(StrategicControllerPlugin);
    install_empty_reachability(&mut app);

    let owner = SwarmId::PLAYER;
    let home = common::cell_world_center(IVec2::ZERO);
    let swarm = common::spawn_swarm_with_nanobots(
        &mut app,
        home,
        &[(NanobotType::Worker, 4), (NanobotType::Hauler, 2)],
    );
    app.world_mut()
        .entity_mut(swarm)
        .insert(StrategicController::new(owner));

    let opening = IVec2::new(2, 0);
    let neutral = IVec2::new(5, 0);
    let [mine, _] = [opening, neutral].map(|cell| {
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
    let source = common::spawn_stockpile(
        &mut app,
        common::cell_world_center(opening) - Vec2::X * 140.0,
        0,
        60,
    );
    app.world_mut().entity_mut(source).insert(OwnerSwarm(swarm));

    let load = app
        .world()
        .resource::<Tuning>()
        .logistics
        .worker_carry_capacity;
    let mut observed_extraction = false;
    for _ in 0..900 {
        app.update();
        let world = app.world();
        let grid = world.resource::<IntentGrid>();
        assert!(
            grid.cell(opening)
                .unwrap()
                .has_owned(IntentKind::Gather, owner),
            "the opening mine must keep its Gather intent"
        );
        assert!(
            !grid
                .cell(neutral)
                .is_some_and(|cell| cell.has_owned(IntentKind::Gather, owner)),
            "a neutral claim appeared before a Worker finished bringing minerals to its Source"
        );
        let deposit = world.get::<ResourceDeposit>(mine).unwrap();
        let stockpile = world.get::<Stockpile>(source).unwrap();
        observed_extraction |= deposit.amount < deposit.capacity && stockpile.amount == 0;
        if stockpile.amount >= load {
            assert!(
                observed_extraction,
                "extraction must precede the first delivery"
            );
            return;
        }
    }
    panic!("the opening never produced a real Worker delivery");
}

#[test]
fn strategic_retarget_erases_superseded_corridor_in_the_same_tick() {
    use top_down_2d_rts_prototype_nano_swarm::resources::ResourceDeposit;

    let mut app = common::minimal_app();
    let mut grid = IntentGrid::new(64, 64);
    let owner = SwarmId::PLAYER;
    let foreign = SwarmId(9);
    let first_cell = IVec2::new(8, 0);
    let next_cell = IVec2::new(0, 6);
    let old_corridor = IVec2::new(6, 0);
    for y in -32..-16 {
        for x in -32..32 {
            grid.paint(IVec2::new(x, y), IntentKind::Corridor, foreign);
        }
    }
    grid.paint(old_corridor, IntentKind::Corridor, foreign);
    app.insert_resource(grid)
        .add_plugins(StrategicControllerPlugin);
    install_empty_reachability(&mut app);
    let swarm =
        common::spawn_swarm_with_nanobots(&mut app, common::cell_world_center(IVec2::ZERO), &[]);
    app.world_mut()
        .entity_mut(swarm)
        .insert(StrategicController::new(owner));
    let first = common::spawn_deposit(
        &mut app,
        common::DepositFixture {
            world_pos: common::cell_world_center(first_cell),
            amount: 72_000,
            capacity: 72_000,
            radius: 80.0,
        },
    );
    app.update();
    assert!(
        app.world()
            .resource::<IntentGrid>()
            .cell(old_corridor)
            .unwrap()
            .has_owned(IntentKind::Corridor, owner)
    );

    app.world_mut()
        .get_mut::<ResourceDeposit>(first)
        .unwrap()
        .amount = 0;
    common::spawn_deposit(
        &mut app,
        common::DepositFixture {
            world_pos: common::cell_world_center(next_cell),
            amount: 72_000,
            capacity: 72_000,
            radius: 80.0,
        },
    );

    for _ in 0..30 {
        app.update();
        let grid = app.world().resource::<IntentGrid>();
        if grid
            .cell(next_cell)
            .unwrap()
            .has_owned(IntentKind::Gather, owner)
        {
            assert!(
                !grid
                    .cell(old_corridor)
                    .unwrap()
                    .has_owned(IntentKind::Corridor, owner),
                "the old corridor must disappear in the tick that shows the new Gather intent"
            );
            assert!(
                grid.cell(old_corridor)
                    .unwrap()
                    .has_owned(IntentKind::Corridor, foreign)
            );
            return;
        }
    }
    panic!("the controller did not select the replacement deposit within one review interval");
}
