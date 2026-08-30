//! Offscreen proof that the production background material renders its checkerboard.

use std::path::Path;

use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{BACKGROUND_OVERLAY_Z, fly_camera::CameraZoom2d};

use crate::harness::{TestContext, TestFlow, clear_nanobots_and_sprite_entities};

const CAMERA_SCALE: f32 = 2.0;
const PATCH_SIZE: u32 = 16;
const PIXEL_TOLERANCE: u8 = 20;
const MIN_MATCHING_PIXELS: usize = 230;

pub fn background_checkerboard(ctx: &mut TestContext) -> TestFlow {
    if ctx.frame == 2 {
        focus_camera(ctx.world);
        hide_non_background_scene(ctx.world);
        return TestFlow::Continue;
    }
    if ctx.frame < 5 {
        return TestFlow::Continue;
    }
    if ctx.frame == 5 {
        return TestFlow::Screenshot("background_checkerboard".to_string());
    }
    TestFlow::Exit
}

pub fn validate_background_checkerboard(path: &Path) -> Result<(), String> {
    let image = image::open(path)
        .map_err(|error| format!("decode background screenshot {}: {error}", path.display()))?
        .to_rgba8();
    let center = Vec2::new(image.width() as f32 / 2.0, image.height() as f32 / 2.0);
    let dark_center = center + Vec2::new(50.0 / CAMERA_SCALE, -50.0 / CAMERA_SCALE);
    let bright_center = center + Vec2::new(150.0 / CAMERA_SCALE, -50.0 / CAMERA_SCALE);

    require_patch_color(&image, dark_center, [0, 188, 0], "dark checker cell")?;
    require_patch_color(&image, bright_center, [0, 255, 0], "bright checker cell")
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
    clear_nanobots_and_sprite_entities(world);
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
        world.entity_mut(entity).insert(Visibility::Hidden);
    }
}

fn require_patch_color(
    image: &image::RgbaImage,
    center: Vec2,
    expected: [u8; 3],
    label: &str,
) -> Result<(), String> {
    let pixels = patch_pixels(image, center);
    let matching = pixels
        .iter()
        .filter(|pixel| color_matches(pixel, expected))
        .count();
    if matching < MIN_MATCHING_PIXELS {
        return Err(format!(
            "{label} rendered the wrong color: expected at least {MIN_MATCHING_PIXELS}/{} pixels near {expected:?} within ±{PIXEL_TOLERANCE}, got {matching}",
            pixels.len()
        ));
    }
    Ok(())
}

fn patch_pixels(image: &image::RgbaImage, center: Vec2) -> Vec<[u8; 3]> {
    let left = center.x.round() as u32 - PATCH_SIZE / 2;
    let top = center.y.round() as u32 - PATCH_SIZE / 2;
    (top..top + PATCH_SIZE)
        .flat_map(|y| {
            (left..left + PATCH_SIZE).map(move |x| {
                let pixel = image.get_pixel(x, y).0;
                [pixel[0], pixel[1], pixel[2]]
            })
        })
        .collect()
}

fn color_matches(actual: &&[u8; 3], expected: [u8; 3]) -> bool {
    actual
        .iter()
        .zip(expected)
        .all(|(actual, expected)| actual.abs_diff(expected) <= PIXEL_TOLERANCE)
}
