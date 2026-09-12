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
    ui::{
        FontsResource,
        match_banner::{MatchBannerText, setup_match_banner, update_match_banner_system},
    },
};

#[test]
fn strategic_intent_finishes_a_remnant_away_from_its_former_home() {
    use std::time::Duration;

    use bevy::time::TimeUpdateStrategy;
    use top_down_2d_rts_prototype_nano_swarm::{
        gameplay_pacing::GameplayPacing,
        nanobot::{ChargePlugin, DefenderResponse},
    };
    let mut app = common::sim_app();
    let cadence = Duration::from_secs_f64(1.0 / 60.0);
    app.insert_resource(Time::<Fixed>::from_duration(cadence))
        .insert_resource(TimeUpdateStrategy::ManualDuration(cadence))
        .add_plugins((
            CombatPlugin,
            ChargePlugin,
            StrategicControllerPlugin,
            SwarmEliminationPlugin,
        ))
        .add_systems(FixedLast, nanobot_death_cleanup_system);
    app.update();
    app.insert_resource(GameplayPacing::default());
    let home = common::cell_world_center(IVec2::ZERO);
    let remnant_cell = IVec2::new(0, 3);
    let remnant_position = common::cell_world_center(remnant_cell);
    let own = common::spawn_swarm_with_nanobots(&mut app, home, &[(NanobotType::Defender, 2)]);
    app.world_mut()
        .entity_mut(own)
        .insert(StrategicController::new(SwarmId::PLAYER));
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
            *app.world().resource::<GameplayPacing>(),
            GameplayPacing::default(),
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
        if *app.world().resource::<MatchOutcome>() == MatchOutcome::Winner(SwarmId::PLAYER) {
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
            assert!(
                !app.world_mut()
                    .query::<&SwarmMember>()
                    .iter(app.world())
                    .any(|member| member.0 == enemy_id)
            );
            break;
        }
    }
    assert_eq!(
        *app.world().resource::<MatchOutcome>(),
        MatchOutcome::Winner(SwarmId::PLAYER),
        "controller must find the real remnant rather than only its former home"
    );
}

#[test]
fn strategic_intent_clears_a_structure_and_distant_hauler_concurrently() {
    use std::{collections::BTreeSet, time::Duration};

    use bevy::time::TimeUpdateStrategy;
    use top_down_2d_rts_prototype_nano_swarm::{
        gameplay_pacing::GameplayPacing,
        nanobot::{ChargePlugin, DefenderResponse, PlannedKind, Structure, StructureKind},
        resources::{ResourceKind, ResourceLedger},
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
    app.insert_resource(GameplayPacing::default());

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

    let enemy = common::spawn_opponent_swarm_with_nanobots(
        &mut app,
        common::cell_world_center(IVec2::new(0, 4)),
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
    let mut concurrent_response_seconds = None;
    for _ in 0..1800 {
        app.update();
        assert_eq!(
            *app.world().resource::<GameplayPacing>(),
            GameplayPacing::default()
        );
        let world = app.world_mut();
        let targets = world
            .query::<(&SwarmMember, &DefenderResponse)>()
            .iter(world)
            .filter(|(member, _)| member.0 == SwarmId::PLAYER)
            .map(|(_, response)| response.target)
            .collect::<BTreeSet<_>>();
        if concurrent_response_seconds.is_none()
            && targets.contains(&structure)
            && targets.contains(&hauler)
        {
            concurrent_response_seconds =
                Some(world.resource::<Time<Fixed>>().elapsed_secs_f64() - started);
        }
        if *world.resource::<MatchOutcome>() == MatchOutcome::Winner(SwarmId::PLAYER) {
            break;
        }
    }

    let elapsed = app.world().resource::<Time<Fixed>>().elapsed_secs_f64() - started;
    assert!(
        concurrent_response_seconds.is_some_and(|seconds| seconds <= 0.5),
        "structure and distant Hauler need concurrent shared responses, got {concurrent_response_seconds:?}"
    );
    assert_eq!(
        *app.world().resource::<MatchOutcome>(),
        MatchOutcome::Winner(SwarmId::PLAYER),
        "mixed cleanup must finish through shared movement and combat within thirty simulated seconds; structure health={:?}; Hauler health={:?}, position={:?}",
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
    println!("concurrent mixed-target cleanup after {elapsed:.3} simulated seconds");
}

#[test]
fn strategic_intent_destroys_an_exposed_production_facility_that_spawns_reinforcements() {
    use std::time::Duration;

    use bevy::time::TimeUpdateStrategy;
    use top_down_2d_rts_prototype_nano_swarm::{
        gameplay_pacing::GameplayPacing,
        nanobot::{ChargePlugin, Structure, StructureKind},
        resources::{ResourceKind, ResourceLedger},
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
    app.insert_resource(GameplayPacing::default());

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
        .insert(Transform::from_translation(target.extend(0.0)));
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

    let mut first_birth_tick = None;
    let mut facility_health_at_birth = None;
    let mut facility_progressed_after_birth = false;
    let mut victory_tick = None;
    for tick in 1..=1_800 {
        app.update();
        if tick == 1 {
            assert!(
                app.world()
                    .resource::<IntentGrid>()
                    .cell(target_cell)
                    .is_some_and(|intent| {
                        intent.has_owned(IntentKind::Defend, SwarmId::PLAYER)
                    }),
                "selected producer must receive player attack intent"
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
        "producer and natural reinforcements must be eliminated within thirty simulated seconds"
    );
    println!(
        "reinforcement at tick {}, natural victory at tick {}",
        first_birth_tick.unwrap(),
        victory_tick.unwrap()
    );
}

#[test]
fn strategic_combat_support_follows_defenders_as_they_advance() {
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
    common::spawn_deposit(
        &mut app,
        common::DepositFixture {
            world_pos: common::cell_world_center(gather_cell),
            amount: 72_000,
            capacity: 72_000,
            radius: 64.0,
        },
    );
    let target_cell = IVec2::new(6, 0);
    let target_position = common::cell_world_center(target_cell);
    let enemy = common::spawn_opponent_swarm_with_nanobots(&mut app, target_position, &[]);
    let sink = common::spawn_empty_completed_structure(&mut app, enemy, PlannedKind::SinkStockpile);
    app.world_mut().entity_mut(sink).insert((
        Structure::new(StructureKind::Basic),
        Transform::from_translation(target_position.extend(0.0)),
    ));

    app.update();
    {
        let grid = app.world().resource::<IntentGrid>();
        let builds = grid
            .iter_active_cells()
            .filter(|(_, cell)| cell.has_owned(IntentKind::Build, SwarmId::PLAYER))
            .map(|(position, _)| position)
            .collect::<Vec<_>>();
        assert!(!builds.is_empty(), "the force needs construction options");
        assert!(
            builds
                .iter()
                .all(|cell| (*cell - home_cell).abs().max_element() <= 1),
            "before the force leaves home, support must not open a remote construction front: {builds:?}"
        );
        assert!(
            grid.cell(target_cell)
                .unwrap()
                .has_owned(IntentKind::Defend, SwarmId::PLAYER)
        );
        assert!(
            grid.cell(gather_cell)
                .unwrap()
                .has_owned(IntentKind::Gather, SwarmId::PLAYER)
        );
    }

    for _ in 0..1800 {
        app.update();
    }
    let world = app.world_mut();
    let advanced_defenders = world
        .query::<(&SwarmMember, &NanobotType, &Transform)>()
        .iter(world)
        .filter(|(member, kind, transform)| {
            member.0 == SwarmId::PLAYER
                && **kind == NanobotType::Defender
                && transform.translation.truncate().distance(target_position)
                    <= 2.0 * top_down_2d_rts_prototype_nano_swarm::ZONE_BLOCK_SIZE
        })
        .count();
    assert!(
        advanced_defenders >= 4,
        "shared movement must advance the main force, got {advanced_defenders}"
    );
    let grid = world.resource::<IntentGrid>();
    assert!(
        grid.iter_active_cells().any(|(cell, intent)| {
            intent.has_owned(IntentKind::Build, SwarmId::PLAYER)
                && (cell - target_cell).abs().max_element() <= 2
                && (cell - home_cell).abs().max_element() >= 3
        }),
        "construction options must follow the physically advanced force"
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
fn mutual_final_combat_deaths_show_draw_in_the_same_tick() {
    use top_down_2d_rts_prototype_nano_swarm::nanobot::Health;
    let mut app = common::sim_app();
    app.add_plugins((CombatPlugin, SwarmEliminationPlugin))
        .add_systems(FixedLast, nanobot_death_cleanup_system)
        .insert_resource(FontsResource { font: default() })
        .add_systems(Startup, setup_match_banner)
        .add_systems(Update, update_match_banner_system);
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
}
