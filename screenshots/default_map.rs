//! Full-app offscreen evidence of the authored routes, mineral pockets, and base shelter.
use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    fly_camera::CameraZoom2d, nanobot::ProductionFacility, resources::ResourceDeposit,
    scenario::cell_origin, terrain::RockFormation,
};

use crate::harness::{TestContext, TestFlow};

pub fn default_map(ctx: &mut TestContext) -> TestFlow {
    ctx.world.resource_mut::<Time<Virtual>>().pause();
    if ctx.frame == 0 {
        for entity in ctx
            .world
            .query_filtered::<Entity, With<Node>>()
            .iter(ctx.world)
            .collect::<Vec<_>>()
        {
            ctx.world.entity_mut(entity).insert(Visibility::Hidden);
        }
        focus(ctx.world, cell_origin(IVec2::new(12, 12)), 24.0);
    }
    if ctx.frame == 2 {
        assert_eq!(
            ctx.world
                .query_filtered::<Entity, With<ResourceDeposit>>()
                .iter(ctx.world)
                .count(),
            6,
            "overview must show both home, expansion, and contested mineral pairs"
        );
        assert_eq!(
            ctx.world
                .query_filtered::<Entity, With<ProductionFacility>>()
                .iter(ctx.world)
                .count(),
            2,
            "overview must retain both starting bases"
        );
        assert!(
            ctx.world
                .query_filtered::<Entity, With<RockFormation>>()
                .iter(ctx.world)
                .next()
                .is_some(),
            "default startup must render the authored rock terrain"
        );
        return TestFlow::Screenshot("default_map_overview".into());
    }
    if ctx.frame == 3 {
        let image = image::open("target/playtest-screenshots/default_map_overview.png")
            .unwrap()
            .to_rgb8();
        // Sample the northern mesa and the open central basin at overview zoom.
        let rock = image.get_pixel(676, 166).0;
        let ground = image.get_pixel(640, 360).0;
        assert!(
            rock[0] > ground[0] + 25 && rock[2] > rock[0],
            "rock masses must read as lighter cool gray against graphite ground: {rock:?} / {ground:?}"
        );
        focus(ctx.world, cell_origin(IVec2::new(0, 0)), 4.0);
        return TestFlow::Continue;
    }
    if ctx.frame == 5 {
        return TestFlow::Screenshot("default_map_player_base".into());
    }
    if ctx.frame > 5 {
        let image = image::open("target/playtest-screenshots/default_map_player_base.png")
            .unwrap()
            .to_rgb8();
        let edge = image.get_pixel(386, 360).0;
        let face = image.get_pixel(401, 360).0;
        assert!(
            edge[0] > face[0] + 8,
            "lit boundary must show an inset bevel, not a flat collision rectangle: {edge:?} / {face:?}"
        );
        TestFlow::Exit
    } else {
        TestFlow::Continue
    }
}

fn focus(world: &mut World, center: Vec2, scale: f32) {
    for (mut transform, mut projection, mut zoom) in world
        .query::<(&mut Transform, &mut Projection, &mut CameraZoom2d)>()
        .iter_mut(world)
    {
        transform.translation.x = center.x;
        transform.translation.y = center.y;
        zoom.zoom = scale;
        if let Projection::Orthographic(orthographic) = &mut *projection {
            orthographic.scale = scale;
        }
    }
}
