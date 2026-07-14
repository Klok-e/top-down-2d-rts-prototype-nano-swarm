//! Integration coverage for the zone-painting fix: the background
//! and zone overlays must use `translation.z` for draw order (not
//! `scale.z`) and the resulting transforms must be wired the way
//! the production spawn does, so a future regression that swaps
//! `scale.z` back in for `translation.z` is caught here. The
//! matching pure-helper unit tests live in
//! `src/lib.rs::overlay_transform_tests`.

use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    GAMEPLAY_SPRITE_Z, MAP_HEIGHT, MAP_WIDTH, ZONE_BLOCK_SIZE, background_overlay_transform,
    zone_overlay_transform,
};

/// Spawn the two overlay entities using the production transform
/// helpers, mirroring the production setup. The tests then assert
/// the contract relationships (zone draws in front of background;
/// scale.z stays 1.0; world dimensions preserved on x/y).
fn spawn_overlays(app: &mut App) -> (Entity, Entity) {
    app.init_resource::<Assets<Mesh>>();
    let mut meshes = app.world_mut().resource_mut::<Assets<Mesh>>();
    let mesh = meshes.add(Mesh::from(Rectangle::default()));
    let mesh_handle = mesh.clone();

    let bg = app
        .world_mut()
        .spawn((
            Mesh2d(mesh),
            background_overlay_transform(
                MAP_WIDTH as f32 * ZONE_BLOCK_SIZE,
                MAP_HEIGHT as f32 * ZONE_BLOCK_SIZE,
            ),
        ))
        .id();

    let zone = app
        .world_mut()
        .spawn((
            Mesh2d(mesh_handle),
            zone_overlay_transform(
                MAP_WIDTH as f32 * ZONE_BLOCK_SIZE,
                MAP_HEIGHT as f32 * ZONE_BLOCK_SIZE,
            ),
        ))
        .id();

    (bg, zone)
}

#[test]
fn background_overlay_translation_z_is_below_zone_overlay() {
    let mut app = App::new();
    let (bg, zone) = spawn_overlays(&mut app);

    let bg_z = app
        .world()
        .entity(bg)
        .get::<Transform>()
        .expect("background must have a Transform")
        .translation
        .z;
    let zone_z = app
        .world()
        .entity(zone)
        .get::<Transform>()
        .expect("zone must have a Transform")
        .translation
        .z;

    assert!(
        zone_z > bg_z,
        "zone overlay (z={zone_z}) must draw in front of the background (z={bg_z})"
    );
    assert!(
        zone_z < GAMEPLAY_SPRITE_Z,
        "zone overlay must sit below gameplay sprites at z={GAMEPLAY_SPRITE_Z}"
    );
}

#[test]
fn background_and_zone_meshes_keep_scale_z_at_one() {
    let mut app = App::new();
    let (bg, zone) = spawn_overlays(&mut app);

    let bg_scale = app
        .world()
        .entity(bg)
        .get::<Transform>()
        .expect("background must have a Transform")
        .scale;
    let zone_scale = app
        .world()
        .entity(zone)
        .get::<Transform>()
        .expect("zone must have a Transform")
        .scale;

    // scale.z must stay 1.0; the old bug put a negative z on the
    // scale (mistaking it for draw order), which mirrored the
    // unit rectangle on the z axis.
    assert_eq!(bg_scale.z, 1.0);
    assert_eq!(zone_scale.z, 1.0);
    assert_eq!(bg_scale.x, MAP_WIDTH as f32 * ZONE_BLOCK_SIZE);
    assert_eq!(bg_scale.y, MAP_HEIGHT as f32 * ZONE_BLOCK_SIZE);
    assert_eq!(zone_scale.x, MAP_WIDTH as f32 * ZONE_BLOCK_SIZE);
    assert_eq!(zone_scale.y, MAP_HEIGHT as f32 * ZONE_BLOCK_SIZE);
}
