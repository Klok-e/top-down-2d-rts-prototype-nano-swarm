#[path = "../common/mod.rs"]
mod common;

use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    intent::{IntentGrid, IntentKind},
    nanobot::{
        Charger, CombatPlugin, MaintenancePlugin, MatchOutcome, Nanobot, NanobotType,
        OpponentIntentController, OpponentIntentPlugin, OwnerSwarm, PopulationDemandPlugin,
        ProductionFacility, ProductionPlugin, SwarmEliminationPlugin, SwarmEliminationState,
        SwarmId, SwarmMember, nanobot_death_cleanup_system,
    },
    resources::Stockpile,
    ui::{
        FontsResource,
        match_banner::{MatchBannerText, setup_match_banner, update_match_banner_system},
    },
};

#[test]
fn scripted_counter_assault_can_eliminate_opponent() {
    let mut app = common::sim_app();
    app.insert_resource(FontsResource { font: default() })
        .add_systems(Startup, setup_match_banner)
        .add_systems(Update, update_match_banner_system);
    app.add_plugins(MaintenancePlugin)
        .add_plugins(ProductionPlugin)
        .add_plugins(PopulationDemandPlugin)
        .add_plugins(CombatPlugin)
        .add_plugins(OpponentIntentPlugin)
        .add_plugins(SwarmEliminationPlugin)
        .add_systems(FixedLast, nanobot_death_cleanup_system);

    let player_cell = IVec2::new(-1, 0);
    let opponent_cell = IVec2::ZERO;
    let player_pos = common::cell_world_center(player_cell);
    let opponent_pos = common::cell_world_center(opponent_cell);
    let player_swarm = common::spawn_swarm_with_nanobots(
        &mut app,
        player_pos,
        &[
            (NanobotType::Worker, 2),
            (NanobotType::Hauler, 2),
            (NanobotType::Defender, 3),
        ],
    );
    let opponent_swarm = common::spawn_opponent_swarm_with_nanobots(
        &mut app,
        opponent_pos,
        &[
            (NanobotType::Worker, 1),
            (NanobotType::Hauler, 1),
            (NanobotType::Defender, 1),
        ],
    );
    let opponent_id = *app.world().entity(opponent_swarm).get::<SwarmId>().unwrap();
    app.world_mut()
        .entity_mut(opponent_swarm)
        .insert(OpponentIntentController::new(
            opponent_cell,
            player_cell,
            0,
            30,
        ));
    {
        let mut grid = app.world_mut().resource_mut::<IntentGrid>();
        grid.paint(player_cell, IntentKind::Defend, SwarmId::PLAYER);
        grid.paint(opponent_cell, IntentKind::Defend, opponent_id);
    }
    common::spawn_facility_at(&mut app, player_swarm, player_pos - Vec2::new(160.0, 0.0));
    common::spawn_facility_at(
        &mut app,
        opponent_swarm,
        opponent_pos + Vec2::new(160.0, 0.0),
    );

    app.update();
    assert_eq!(
        app.world()
            .resource::<IntentGrid>()
            .cell(player_cell)
            .unwrap()
            .owners(IntentKind::Defend)
            .collect::<Vec<_>>(),
        vec![SwarmId::PLAYER, opponent_id],
        "the opponent controller must launch a advance into independently overlapping paint",
    );

    app.world_mut().resource_mut::<IntentGrid>().paint(
        opponent_cell,
        IntentKind::Defend,
        SwarmId::PLAYER,
    );

    for _ in 0..800 {
        app.update();
        if *app.world().resource::<MatchOutcome>() == MatchOutcome::Winner(SwarmId::PLAYER) {
            assert!(
                !app.world_mut()
                    .query_filtered::<&SwarmMember, With<Nanobot>>()
                    .iter(app.world())
                    .any(|owner| owner.0 == opponent_id)
            );
            assert!(!app.world_mut().query_filtered::<&OwnerSwarm, Or<(With<ProductionFacility>, With<Stockpile>, With<Charger>)>>()
                .iter(app.world()).any(|owner| owner.0 == opponent_swarm));
            assert_eq!(
                app.world_mut()
                    .query_filtered::<&Text, With<MatchBannerText>>()
                    .single(app.world())
                    .unwrap()
                    .0,
                "VICTORY\nOpponent Swarm Eliminated"
            );
            return;
        }
    }

    let state = app.world().resource::<SwarmEliminationState>();
    panic!(
        "scripted counter-assault did not end the match: eliminated={:?}",
        state.eliminated
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
