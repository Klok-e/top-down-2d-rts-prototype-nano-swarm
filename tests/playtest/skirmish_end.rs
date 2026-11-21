use bevy::{asset::AssetPlugin, prelude::*};
use top_down_2d_rts_prototype_nano_swarm::{
    nanobot::{ActiveSkirmishRings, NanobotPresentationPlugin, SwarmId, swarm_presentation_color},
    tuning::Tuning,
};

#[path = "../common/combat_scenes.rs"]
mod scenes;

use scenes::{CombatMetrics, Scene, Variant, scene_app, spawn_scene};

#[test]
fn resolved_skirmish_plays_a_ring_in_the_winners_color() {
    let mut app = scene_app();
    app.add_plugins((TaskPoolPlugin::default(), AssetPlugin::default()))
        .init_asset::<Image>()
        .add_plugins((TransformPlugin, NanobotPresentationPlugin));
    app.world_mut().resource_mut::<Tuning>().combat.rout_ratio = 0.0;
    let scene = Scene::Skirmish;
    spawn_scene(app.world_mut(), scene, Variant::Standard);
    let mut metrics = CombatMetrics::new(app.world_mut(), scene);
    app.update();

    let resolution_tick = (1..=3_600)
        .find_map(|tick| {
            app.update();
            metrics.observe_tick(app.world(), tick, scene.name());
            metrics.resolution_tick
        })
        .expect("the skirmish should resolve within 60 seconds");
    let winner = match metrics.summary().final_alive {
        [players, 0] if players > 0 => SwarmId::PLAYER,
        [0, opponents] if opponents > 0 => SwarmId(1),
        survivors => panic!("expected one surviving side, found {survivors:?}"),
    };
    let expected_color = swarm_presentation_color(winner).with_alpha(0.4);

    let deadline = app.world().resource::<Time>().elapsed() + std::time::Duration::from_secs(3);
    while app.world().resource::<Time>().elapsed() <= deadline {
        if let Some(ring) = app
            .world()
            .resource::<ActiveSkirmishRings>()
            .iter()
            .find(|ring| ring.radius() > 0.0)
        {
            assert_eq!(ring.color, expected_color, "ring at {:?}", ring.center);
            return;
        }
        app.update();
    }
    panic!("no visible winner ring within 3 seconds of resolution at tick {resolution_tick}");
}
