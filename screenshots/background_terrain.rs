//! Offscreen proof of subdued mineral ground and input-only cell guides.

use crate::harness::{TestContext, TestFlow};
use bevy::prelude::*;
use std::path::Path;
use top_down_2d_rts_prototype_nano_swarm::{BACKGROUND_OVERLAY_Z, fly_camera::CameraZoom2d};

const CAMERA_SCALE: f32 = 2.0;

pub fn background_terrain(ctx: &mut TestContext) -> TestFlow {
    if ctx.frame == 2 {
        focus_camera(ctx.world);
    }
    if ctx.frame >= 2 {
        hide_non_background_scene(ctx.world);
    }
    if ctx.frame == 5 {
        return TestFlow::Screenshot("background_terrain".to_string());
    }
    if ctx.frame == 6 {
        validate_background_terrain(Path::new(
            "target/playtest-screenshots/background_terrain.png",
        ))
        .unwrap();
        let mut window = Window {
            resolution: (1280, 720).into(),
            ..default()
        };
        window.set_cursor_position(Some(Vec2::new(640.0, 360.0)));
        ctx.world.spawn(window);
        ctx.world
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Left);
    }
    if ctx.frame == 9 {
        return TestFlow::Screenshot("background_terrain_painting".to_string());
    }
    if ctx.frame == 10 {
        validate_background_terrain(Path::new(
            "target/playtest-screenshots/background_terrain_painting.png",
        ))
        .unwrap();
        ctx.world
            .resource_mut::<ButtonInput<MouseButton>>()
            .release(MouseButton::Left);
    }
    if ctx.frame == 13 {
        return TestFlow::Screenshot("background_terrain_released".to_string());
    }
    if ctx.frame > 13 {
        return TestFlow::Exit;
    }
    TestFlow::Continue
}

pub fn validate_background_terrain(path: &Path) -> Result<(), String> {
    let image = image::open(path)
        .map_err(|error| error.to_string())?
        .to_rgba8();
    let painting = path
        .file_stem()
        .unwrap()
        .to_string_lossy()
        .ends_with("_painting");
    let mut guide_pixels = 0;
    let mut min = u8::MAX;
    let mut max = 0;
    for pixel in image.pixels() {
        let [r, g, b, _] = pixel.0;
        if (r > 65 || g > 70 || b > 75) && r < 90 && b >= r && b - r < 20 {
            guide_pixels += 1;
            continue;
        }
        if !(30..=65).contains(&r)
            || !(35..=70).contains(&g)
            || !(40..=75).contains(&b)
            || b < r
            || b - r > 18
        {
            return Err(format!(
                "ground must remain subdued cool charcoal, got {:?}",
                pixel.0
            ));
        }
        min = min.min(r);
        max = max.max(r);
    }
    if painting && guide_pixels < 1500 {
        return Err(format!(
            "painting must reveal cell guides; only {guide_pixels} guide pixels"
        ));
    }
    if !painting && guide_pixels != 0 {
        return Err(format!(
            "idle/released ground must hide cell guides; {guide_pixels} remain"
        ));
    }
    if max - min < 5 {
        return Err(format!(
            "mineral ground has no visible tonal variation: {min}..{max}"
        ));
    }
    Ok(())
}

fn focus_camera(world: &mut World) {
    let mut cameras = world.query::<(&mut Transform, &mut Projection, &mut CameraZoom2d)>();
    for (mut transform, mut projection, mut zoom) in cameras.iter_mut(world) {
        transform.translation.x = 0.0;
        transform.translation.y = 0.0;
        zoom.zoom = CAMERA_SCALE;
        if let Projection::Orthographic(orthographic) = &mut *projection {
            orthographic.scale = CAMERA_SCALE;
        }
    }
}

fn hide_non_background_scene(world: &mut World) {
    let sprites = world
        .query_filtered::<Entity, With<Sprite>>()
        .iter(world)
        .collect::<Vec<_>>();
    for entity in sprites {
        world.entity_mut(entity).remove::<Sprite>();
    }
    let meshes = world
        .query_filtered::<(Entity, &Transform), With<Mesh2d>>()
        .iter(world)
        .filter_map(|(entity, transform)| {
            ((transform.translation.z - BACKGROUND_OVERLAY_Z).abs() > f32::EPSILON)
                .then_some(entity)
        })
        .collect::<Vec<_>>();
    let ui = world
        .query_filtered::<Entity, With<Node>>()
        .iter(world)
        .collect::<Vec<_>>();
    for entity in meshes.into_iter().chain(ui) {
        world
            .entity_mut(entity)
            .remove::<Mesh2d>()
            .insert(Visibility::Hidden);
    }
}
