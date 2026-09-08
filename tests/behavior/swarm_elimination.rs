use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    intent::{IntentGrid, IntentKind},
    nanobot::{MatchOutcome, NanobotType, SwarmId},
};
#[path = "../common/mod.rs"]
mod common;

#[test]
fn stranded_defenders_keep_both_swarms_in_progress() {
    let mut app = common::sim_app_with_elimination();
    common::spawn_swarm_with_nanobots(&mut app, Vec2::ZERO, &[(NanobotType::Defender, 1)]);
    let opponent = common::spawn_opponent_swarm_with_nanobots(
        &mut app,
        Vec2::new(500.0, 0.0),
        &[(NanobotType::Defender, 1)],
    );
    let opponent_id = *app.world().get::<SwarmId>(opponent).unwrap();
    for owner in [SwarmId::PLAYER, opponent_id] {
        for x in -4..4 {
            app.world_mut().resource_mut::<IntentGrid>().paint(
                IVec2::new(x, 0),
                IntentKind::Defend,
                owner,
            );
        }
    }
    app.update();
    assert_eq!(
        *app.world().resource::<MatchOutcome>(),
        MatchOutcome::InProgress
    );
}

#[test]
fn each_nanobot_type_survives_until_its_final_death_is_cleaned_up() {
    use top_down_2d_rts_prototype_nano_swarm::nanobot::{Health, Nanobot, SwarmMember};
    for kind in NanobotType::ALL {
        let mut app = common::sim_app_with_elimination();
        common::spawn_swarm_with_nanobots(&mut app, Vec2::ZERO, &[(kind, 1)]);
        let opponent =
            common::spawn_opponent_swarm_with_nanobots(&mut app, Vec2::X * 500.0, &[(kind, 1)]);
        let opponent_id = *app.world().get::<SwarmId>(opponent).unwrap();
        app.update();
        assert_eq!(
            *app.world().resource::<MatchOutcome>(),
            MatchOutcome::InProgress,
            "{kind:?}"
        );
        let victim = app
            .world_mut()
            .query_filtered::<(Entity, &SwarmMember), With<Nanobot>>()
            .iter(app.world())
            .find(|(_, owner)| owner.0.is_player())
            .unwrap()
            .0;
        app.world_mut().get_mut::<Health>(victim).unwrap().current = 0;
        app.update();
        assert!(app.world().get_entity(victim).is_err());
        assert_eq!(
            *app.world().resource::<MatchOutcome>(),
            MatchOutcome::Winner(opponent_id),
            "{kind:?}: loss must be detected in the cleanup tick"
        );
    }
}

#[test]
fn each_empty_completed_structure_prevents_elimination_only_for_its_owner() {
    use top_down_2d_rts_prototype_nano_swarm::nanobot::PlannedKind;
    for kind in [
        PlannedKind::ProductionFacility,
        PlannedKind::SourceStockpile,
        PlannedKind::SinkStockpile,
        PlannedKind::Charger,
    ] {
        for remove_player in [false, true] {
            let mut app = common::sim_app_with_elimination();
            let player = common::spawn_swarm_at(&mut app, Vec2::ZERO);
            let opponent =
                common::spawn_opponent_swarm_with_nanobots(&mut app, Vec2::X * 500.0, &[]);
            let opponent_id = *app.world().get::<SwarmId>(opponent).unwrap();
            let player_structure = common::spawn_empty_completed_structure(&mut app, player, kind);
            let opponent_structure =
                common::spawn_empty_completed_structure(&mut app, opponent, kind);
            app.update();
            assert_eq!(
                *app.world().resource::<MatchOutcome>(),
                MatchOutcome::InProgress,
                "{kind:?}"
            );
            app.world_mut().despawn(if remove_player {
                player_structure
            } else {
                opponent_structure
            });
            app.update();
            assert_eq!(
                *app.world().resource::<MatchOutcome>(),
                MatchOutcome::Winner(if remove_player {
                    opponent_id
                } else {
                    SwarmId::PLAYER
                }),
                "{kind:?}: foreign structure cannot keep an empty swarm alive"
            );
        }
    }
}

#[test]
fn plans_and_owned_terrain_do_not_keep_an_empty_swarm_alive() {
    use top_down_2d_rts_prototype_nano_swarm::nanobot::{OwnerSwarm, PlannedKind};
    let mut app = common::sim_app_with_elimination();
    let player = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let opponent = common::spawn_opponent_swarm_with_nanobots(&mut app, Vec2::X * 500.0, &[]);
    for owner in [player, opponent] {
        for kind in [
            PlannedKind::ProductionFacility,
            PlannedKind::SourceStockpile,
            PlannedKind::SinkStockpile,
            PlannedKind::Charger,
        ] {
            let plan = common::spawn_planned_structure_of_kind_at_cell(&mut app, IVec2::ZERO, kind);
            app.world_mut().entity_mut(plan).insert(OwnerSwarm(owner));
        }
        let deposit = common::spawn_deposit(
            &mut app,
            common::DepositFixture {
                world_pos: Vec2::Y * 500.0,
                amount: 100,
                capacity: 100,
                radius: 32.0,
            },
        );
        app.world_mut()
            .entity_mut(deposit)
            .insert(OwnerSwarm(owner));
    }
    app.update();
    assert_eq!(*app.world().resource::<MatchOutcome>(), MatchOutcome::Draw);
}

#[test]
fn every_terminal_outcome_remains_latched_when_survival_changes() {
    use top_down_2d_rts_prototype_nano_swarm::nanobot::{
        Health, Nanobot, SwarmEliminationState, SwarmMember,
    };
    for expected in [
        MatchOutcome::Winner(SwarmId::PLAYER),
        MatchOutcome::Winner(SwarmId(1)),
        MatchOutcome::Draw,
    ] {
        let mut app = common::sim_app_with_elimination();
        common::spawn_swarm_with_nanobots(&mut app, Vec2::ZERO, &[(NanobotType::Defender, 1)]);
        common::spawn_opponent_swarm_with_nanobots(
            &mut app,
            Vec2::X * 500.0,
            &[(NanobotType::Defender, 1)],
        );
        app.update();
        for (member, mut health) in app
            .world_mut()
            .query_filtered::<(&SwarmMember, &mut Health), With<Nanobot>>()
            .iter_mut(app.world_mut())
        {
            if expected == MatchOutcome::Draw || (member.0 != expected_winner(expected)) {
                health.current = 0;
            }
        }
        app.update();
        assert_eq!(*app.world().resource::<MatchOutcome>(), expected);
        // Reverse survival after the match has finished; only diagnostic flags change.
        for mut health in app
            .world_mut()
            .query_filtered::<&mut Health, With<Nanobot>>()
            .iter_mut(app.world_mut())
        {
            health.current = 0;
        }
        common::spawn_worker_at(&mut app, Vec2::ZERO);
        app.update();
        assert!(
            !app.world()
                .resource::<SwarmEliminationState>()
                .is_eliminated(SwarmId::PLAYER)
        );
        assert!(
            app.world()
                .resource::<SwarmEliminationState>()
                .is_eliminated(SwarmId(1))
        );
        assert_eq!(*app.world().resource::<MatchOutcome>(), expected);
    }
}

fn expected_winner(outcome: MatchOutcome) -> SwarmId {
    match outcome {
        MatchOutcome::Winner(id) => id,
        _ => SwarmId::PLAYER,
    }
}

#[test]
fn last_structure_removal_and_nanobot_death_in_one_tick_produce_draw() {
    use top_down_2d_rts_prototype_nano_swarm::nanobot::{
        Health, Nanobot, PlannedKind, ProductionFacility,
    };
    let mut app = common::sim_app_with_elimination();
    let player = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    common::spawn_empty_completed_structure(&mut app, player, PlannedKind::ProductionFacility);
    common::spawn_opponent_swarm_with_nanobots(
        &mut app,
        Vec2::X * 500.0,
        &[(NanobotType::Defender, 1)],
    );
    app.update();
    assert_eq!(
        *app.world().resource::<MatchOutcome>(),
        MatchOutcome::InProgress
    );
    app.add_systems(
        FixedUpdate,
        |mut commands: Commands,
         structures: Query<Entity, With<ProductionFacility>>,
         mut health: Query<&mut Health, With<Nanobot>>| {
            for entity in &structures {
                commands.entity(entity).despawn();
            }
            for mut health in &mut health {
                health.current = 0;
            }
        },
    );
    app.update();
    assert_eq!(*app.world().resource::<MatchOutcome>(), MatchOutcome::Draw);
}

#[test]
fn completed_birth_in_the_last_structure_removal_tick_prevents_elimination() {
    use top_down_2d_rts_prototype_nano_swarm::nanobot::{
        Nanobot, PlannedKind, ProductionFacility, SwarmMember,
    };
    let mut app = common::sim_app_with_elimination();
    let player = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    common::spawn_empty_completed_structure(&mut app, player, PlannedKind::ProductionFacility);
    common::spawn_opponent_swarm_with_nanobots(
        &mut app,
        Vec2::X * 500.0,
        &[(NanobotType::Defender, 1)],
    );
    app.add_systems(
        FixedUpdate,
        |mut commands: Commands, structures: Query<Entity, With<ProductionFacility>>| {
            for entity in &structures {
                commands.entity(entity).despawn();
                commands.spawn((Nanobot {}, SwarmMember(SwarmId::PLAYER)));
            }
        },
    );
    app.update();
    assert_eq!(
        *app.world().resource::<MatchOutcome>(),
        MatchOutcome::InProgress
    );
}
