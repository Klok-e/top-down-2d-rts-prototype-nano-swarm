//! Offscreen evidence for seeded, off-center Build-Zone placement.

use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    GAMEPLAY_SPRITE_Z, ZONE_BLOCK_SIZE,
    fly_camera::CameraZoom2d,
    nanobot::{BUILDING_FOOTPRINT_RADIUS, find_build_zone_placement, overlaps_any_obstacle},
};

use crate::harness::{TestContext, TestFlow};

const CELLS: [IVec2; 4] = [
    IVec2::new(-2, 0),
    IVec2::new(-1, 0),
    IVec2::new(0, 0),
    IVec2::new(1, 0),
];

pub fn build_zone_placement(ctx: &mut TestContext) -> TestFlow {
    if ctx.frame == 2 {
        clear_scene(ctx.world);
        focus_camera(ctx.world);
        spawn_placement_examples(ctx.world);
        return TestFlow::Continue;
    }
    if ctx.frame < 12 {
        return TestFlow::Continue;
    }
    if ctx.frame == 12 {
        return TestFlow::Screenshot("build_zone_placement".to_string());
    }
    TestFlow::Exit
}

fn clear_scene(world: &mut World) {
    let entities = world
        .query_filtered::<Entity, Or<(With<Sprite>, With<Mesh2d>, With<Node>)>>()
        .iter(world)
        .collect::<Vec<_>>();
    for entity in entities {
        let _ = world.despawn(entity);
    }
}

fn focus_camera(world: &mut World) {
    let midpoint = CELLS
        .iter()
        .map(|cell| top_down_2d_rts_prototype_nano_swarm::ai::get_world_from_zone(*cell))
        .sum::<Vec2>()
        / CELLS.len() as f32;
    for (mut transform, mut projection, mut zoom) in world
        .query::<(&mut Transform, &mut Projection, &mut CameraZoom2d)>()
        .iter_mut(world)
    {
        transform.translation.x = midpoint.x;
        transform.translation.y = midpoint.y;
        zoom.zoom = 1.45;
        if let Projection::Orthographic(ortho) = &mut *projection {
            ortho.scale = 1.45;
        }
    }
}

fn spawn_placement_examples(world: &mut World) {
    for (index, cell) in CELLS.into_iter().enumerate() {
        let center = top_down_2d_rts_prototype_nano_swarm::ai::get_world_from_zone(cell);
        world.spawn((
            Sprite::from_color(
                Color::srgba(0.15, 0.38, 0.95, 0.28),
                Vec2::splat(ZONE_BLOCK_SIZE - 8.0),
            ),
            Transform::from_translation(center.extend(0.0)),
        ));
        world.spawn((
            Sprite::from_color(Color::srgb(0.12, 0.15, 0.2), Vec2::splat(10.0)),
            Transform::from_translation(center.extend(1.0)),
        ));

        let kind_seed = if index.is_multiple_of(2) { 27 } else { 26 };
        let (_, position) = find_build_zone_placement(&[cell], &[], kind_seed)
            .expect("an empty Build cell has available placement");
        assert_ne!(position, center);
        assert!(!overlaps_any_obstacle(
            position,
            BUILDING_FOOTPRINT_RADIUS,
            16.0,
            &[],
        ));
        let color = if kind_seed == 27 {
            Color::srgb(0.95, 0.55, 0.18)
        } else {
            Color::srgb(0.35, 0.9, 0.65)
        };
        world.spawn((
            Sprite::from_color(color, Vec2::splat(BUILDING_FOOTPRINT_RADIUS * 2.0)),
            Transform::from_translation(position.extend(GAMEPLAY_SPRITE_Z)),
        ));
    }
}
