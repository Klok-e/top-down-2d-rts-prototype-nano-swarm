//! Full, partially extracted, and exhausted mineral silhouettes in the real renderer.
use crate::harness::{TestContext, TestFlow, clear_nanobots_and_sprite_entities};
use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    GAMEPLAY_SPRITE_Z,
    fly_camera::CameraZoom2d,
    resources::{ResourceDeposit, ResourceKind},
};

pub fn deposit_presentation(ctx: &mut TestContext) -> TestFlow {
    ctx.world.resource_mut::<Time<Virtual>>().pause();
    if ctx.frame == 2 {
        clear_nanobots_and_sprite_entities(ctx.world);
        for entity in ctx
            .world
            .query_filtered::<Entity, With<Node>>()
            .iter(ctx.world)
            .collect::<Vec<_>>()
        {
            ctx.world.entity_mut(entity).insert(Visibility::Hidden);
        }
        for (mut transform, mut projection, mut zoom) in ctx
            .world
            .query::<(&mut Transform, &mut Projection, &mut CameraZoom2d)>()
            .iter_mut(ctx.world)
        {
            transform.translation.x = 0.0;
            transform.translation.y = 0.0;
            zoom.zoom = 0.8;
            if let Projection::Orthographic(ortho) = &mut *projection {
                ortho.scale = 0.8;
            }
        }
        for (x, amount) in [(-220.0, 1000), (0.0, 250), (220.0, 0)] {
            ctx.world.spawn((
                ResourceDeposit {
                    kind: ResourceKind::Minerals,
                    amount,
                    capacity: 1000,
                    radius: 64.0,
                },
                Transform::from_xyz(x, 0.0, GAMEPLAY_SPRITE_Z).with_scale(Vec3::splat(2.0)),
            ));
        }
    }
    if ctx.frame == 10 {
        return TestFlow::Screenshot("deposit_full_partial_depleted".into());
    }
    if ctx.frame > 10 {
        TestFlow::Exit
    } else {
        TestFlow::Continue
    }
}

pub fn validate_deposit_presentation(path: &std::path::Path) -> Result<(), String> {
    let image = image::open(path)
        .map_err(|error| error.to_string())?
        .to_rgba8();
    let cyan_area = |center: u32| {
        let mut count = 0;
        let mut light = 0u64;
        for y in 290..430 {
            for x in center - 70..center + 70 {
                let [r, g, b, _] = image.get_pixel(x, y).0;
                if g > r.saturating_add(45) && b > r.saturating_add(45) {
                    count += 1;
                    light += u64::from(g);
                }
            }
        }
        (count, light)
    };
    let (full, full_light) = cyan_area(365);
    let (partial, partial_light) = cyan_area(640);
    let (empty, _) = cyan_area(915);
    if full < 1500 || partial < 200 || partial * 2 >= full || empty != 0 {
        return Err(format!(
            "crystal area must shrink with extraction and vanish at depletion: full={full}, partial={partial}, empty={empty}"
        ));
    }
    if partial_light / partial as u64 >= full_light / full as u64 {
        return Err("partially extracted crystals must be dimmer than full crystals".into());
    }
    for center in [365, 640, 915] {
        let [r, g, b, _] = image.get_pixel(center, 420).0;
        if !(r > 60 && g > 80 && b > 90 && b < r + 50) {
            return Err(format!(
                "permanent radius-64 base missing at x={center}: {r},{g},{b}"
            ));
        }
    }
    Ok(())
}
