use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    nanobot::MatchOutcome,
    ui::{
        FontsResource,
        match_banner::{
            MatchBannerRoot, MatchBannerText, setup_match_banner, update_match_banner_system,
        },
    },
};

#[test]
fn match_banner_shows_each_outcome_and_hides_in_progress() {
    let mut app = App::new();
    app.insert_resource(FontsResource { font: default() })
        .add_systems(Startup, setup_match_banner)
        .add_systems(Update, update_match_banner_system);
    app.update();
    assert_banner(app.world_mut(), Visibility::Hidden, "");
    for (outcome, text) in [
        (MatchOutcome::Victory, "VICTORY\nOpponent Swarm Eliminated"),
        (MatchOutcome::Defeat, "DEFEAT\nPlayer Swarm Eliminated"),
        (MatchOutcome::Draw, "DRAW\nBoth Swarms Eliminated"),
    ] {
        app.insert_resource(outcome);
        app.update();
        assert_banner(app.world_mut(), Visibility::Visible, text);
    }
    app.insert_resource(MatchOutcome::InProgress);
    app.update();
    assert_banner(app.world_mut(), Visibility::Hidden, "");
}

fn assert_banner(world: &mut World, visibility: Visibility, text: &str) {
    assert_eq!(
        *world
            .query_filtered::<&Visibility, With<MatchBannerRoot>>()
            .single(world)
            .unwrap(),
        visibility
    );
    assert_eq!(
        world
            .query_filtered::<&Text, With<MatchBannerText>>()
            .single(world)
            .unwrap()
            .0,
        text
    );
}
