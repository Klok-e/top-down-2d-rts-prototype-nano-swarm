use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::nanobot::{MatchOutcome, NanobotType, SwarmId};
#[path = "../common/mod.rs"]
mod common;

#[test]
fn nanobots_without_completed_structures_do_not_keep_a_swarm_alive() {
    for kind in NanobotType::ALL {
        let mut app = common::sim_app_with_elimination();
        common::spawn_swarm_with_nanobots(&mut app, Vec2::ZERO, &[(kind, 3)]);
        let opponent = common::spawn_opponent_swarm_with_nanobots(&mut app, Vec2::X * 500.0, &[]);
        common::spawn_home_facility(&mut app, opponent);
        let opponent_id = *app.world().get::<SwarmId>(opponent).unwrap();
        app.update();
        assert_eq!(
            *app.world().resource::<MatchOutcome>(),
            MatchOutcome::Winner(opponent_id),
            "{kind:?}"
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
    use top_down_2d_rts_prototype_nano_swarm::nanobot::SwarmEliminationState;
    for expected in [
        MatchOutcome::Winner(SwarmId::PLAYER),
        MatchOutcome::Winner(SwarmId(1)),
        MatchOutcome::Draw,
    ] {
        let mut app = common::sim_app_with_elimination();
        let player = common::spawn_swarm_at(&mut app, Vec2::ZERO);
        common::spawn_home_facility(&mut app, player);
        let opponent = common::spawn_opponent_swarm_with_nanobots(&mut app, Vec2::X * 500.0, &[]);
        common::spawn_home_facility(&mut app, opponent);
        app.update();
        common::destroy_structures(&mut app, |id| match expected {
            MatchOutcome::Winner(winner) => id != winner,
            _ => true,
        });
        app.update();
        assert_eq!(*app.world().resource::<MatchOutcome>(), expected);
        // Reverse survival after the match has finished; only diagnostic flags change.
        common::destroy_structures(&mut app, |_| true);
        common::spawn_home_facility(&mut app, player);
        app.update();
        let elimination = app.world().resource::<SwarmEliminationState>();
        assert!(!elimination.is_eliminated(SwarmId::PLAYER));
        assert!(elimination.is_eliminated(SwarmId(1)));
        assert_eq!(*app.world().resource::<MatchOutcome>(), expected);
    }
}

#[test]
fn removing_both_last_structures_in_one_tick_produces_draw() {
    use top_down_2d_rts_prototype_nano_swarm::nanobot::ProductionFacility;
    let mut app = common::sim_app_with_elimination();
    let player = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    common::spawn_home_facility(&mut app, player);
    let opponent = common::spawn_opponent_swarm_with_nanobots(&mut app, Vec2::X * 500.0, &[]);
    common::spawn_home_facility(&mut app, opponent);
    app.update();
    assert_eq!(
        *app.world().resource::<MatchOutcome>(),
        MatchOutcome::InProgress
    );
    app.add_systems(
        FixedUpdate,
        |mut commands: Commands, structures: Query<Entity, With<ProductionFacility>>| {
            for entity in &structures {
                commands.entity(entity).despawn();
            }
        },
    );
    app.update();
    assert_eq!(*app.world().resource::<MatchOutcome>(), MatchOutcome::Draw);
}
