//! Offscreen evidence for binary zone presence, colours, overlap, and opacity.

use std::path::Path;

use bevy::{prelude::*, render::storage::ShaderStorageBuffer};
use top_down_2d_rts_prototype_nano_swarm::{
    MAP_HEIGHT, MAP_WIDTH, ZONE_BLOCK_SIZE,
    fly_camera::CameraZoom2d,
    intent::{IntentGrid, IntentKind},
    nanobot::{Nanobot, NanobotVisual, SwarmId},
    zones::{ZoneMaterial, ZoneMaterialHandleComponent, ZoneOwnership, ZonePointData},
};

use crate::harness::{TestContext, TestFlow};

const PRESENT_CELLS: [IVec2; 4] = [
    IVec2::new(-3, 5),
    IVec2::new(-2, 5),
    IVec2::new(-1, 5),
    IVec2::new(0, 5),
];
const OVERLAP_CELL: IVec2 = IVec2::new(2, 5);
const ABSENT_CELL: IVec2 = IVec2::new(4, 5);
// Absent, player Gather, opponent Build, overlapping Defend, player Corridor,
// all four player-owned layers, and player Build overlapping enemy Gather.
// Presence occupies bits 0..=3 and each ownership class occupies two bits
// beginning at bit 4.
const DISPLAY_VALUES: [u32; 7] = [0, 17, 130, 772, 1032, 1375, 99];
const CAPTURE_FRAME: u32 = 60;
const FRAMING_SCALE: f32 = 2.8;
const PATCH_SIZE: u32 = 32;
const PIXEL_TOLERANCE: u8 = 20;
const MIN_SOLID_MATCHING_PIXELS: usize = 920;
const MIN_STRIPE_MATCHING_PIXELS: usize = 256;

pub fn zone_binary_overlay(ctx: &mut TestContext) -> TestFlow {
    if ctx.frame == 2 {
        focus_camera(ctx.world);
        hide_existing_scene(ctx.world);
        paint_examples(ctx.world);
        spawn_binary_display(ctx.world);
        return TestFlow::Continue;
    }
    if ctx.frame < CAPTURE_FRAME {
        return TestFlow::Continue;
    }
    if ctx.frame == CAPTURE_FRAME {
        assert_mirror(ctx.world);
        return TestFlow::Screenshot("zone_binary_overlay".to_string());
    }
    TestFlow::Exit
}

pub fn validate_zone_binary_overlay(path: &Path) -> Result<(), String> {
    let image = image::open(path)
        .map_err(|error| format!("decode zone screenshot {}: {error}", path.display()))?
        .to_rgba8();
    let cell_width = ZONE_BLOCK_SIZE / FRAMING_SCALE;
    let first_center_x =
        image.width() as f32 / 2.0 - (DISPLAY_VALUES.len() as f32 - 1.0) * cell_width / 2.0;
    let center_y = image.height() as f32 / 2.0;
    let center = |index: usize| Vec2::new(first_center_x + index as f32 * cell_width, center_y);

    require_solid_patch(&image, center(0), [20, 20, 26], "absent zone")?;
    require_solid_patch(&image, center(1), [231, 5, 7], "player Gather zone")?;
    require_striped_patch(
        &image,
        center(2),
        [221, 53, 179],
        [170, 39, 137],
        "opponent Build zone",
    )?;
    require_striped_patch(
        &image,
        center(3),
        [5, 5, 231],
        [231, 88, 73],
        "independent overlapping Defend zone",
    )?;
    require_solid_patch(&image, center(4), [231, 231, 7], "player Corridor zone")?;
    require_solid_patch(
        &image,
        center(5),
        [204, 124, 170],
        "overlapping player zones",
    )?;
    require_striped_patch(
        &image,
        center(6),
        [231, 5, 231],
        [231, 88, 73],
        "player Build preserved under enemy Gather hatch",
    )
}

fn require_solid_patch(
    image: &image::RgbaImage,
    center: Vec2,
    expected: [u8; 3],
    label: &str,
) -> Result<(), String> {
    let pixels = patch_pixels(image, center);
    let matching = matching_pixels(&pixels, expected);
    if matching < MIN_SOLID_MATCHING_PIXELS {
        return Err(format!(
            "{label} rendered the wrong color: expected at least {MIN_SOLID_MATCHING_PIXELS}/{} pixels near {expected:?} within ±{PIXEL_TOLERANCE}, got {matching}",
            pixels.len()
        ));
    }
    Ok(())
}

fn require_striped_patch(
    image: &image::RgbaImage,
    center: Vec2,
    first: [u8; 3],
    second: [u8; 3],
    label: &str,
) -> Result<(), String> {
    let pixels = patch_pixels(image, center);
    let first_matching = matching_pixels(&pixels, first);
    let second_matching = matching_pixels(&pixels, second);
    if first_matching < MIN_STRIPE_MATCHING_PIXELS || second_matching < MIN_STRIPE_MATCHING_PIXELS {
        return Err(format!(
            "{label} did not render both stripe colors: expected at least {MIN_STRIPE_MATCHING_PIXELS}/{} pixels near {first:?} and {second:?} within ±{PIXEL_TOLERANCE}, got {first_matching} and {second_matching}",
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

fn matching_pixels(pixels: &[[u8; 3]], expected: [u8; 3]) -> usize {
    pixels
        .iter()
        .filter(|pixel| {
            pixel
                .iter()
                .zip(expected)
                .all(|(actual, expected)| actual.abs_diff(expected) <= PIXEL_TOLERANCE)
        })
        .count()
}

fn paint_examples(world: &mut World) {
    let mut grid = world.resource_mut::<IntentGrid>();
    for (cell, kind) in PRESENT_CELLS.into_iter().zip(IntentKind::ALL) {
        grid.paint(cell, kind, SwarmId::PLAYER);
    }
    for kind in IntentKind::ALL {
        grid.paint(OVERLAP_CELL, kind, SwarmId::PLAYER);
        grid.paint(OVERLAP_CELL, kind, SwarmId(9));
    }
}

/// Spawn a test strip through production [`ZoneMaterial`] with absent,
/// player-only, enemy-only, and same-kind and cross-kind overlap examples. Static test data makes rendered colour evidence
/// independent from asynchronous main-world storage-buffer extraction, while
/// [`assert_mirror`] separately proves simulation-to-material mirroring.
fn spawn_binary_display(world: &mut World) {
    let zone_data = DISPLAY_VALUES
        .into_iter()
        .map(|active| ZonePointData { active })
        .collect::<Vec<_>>();
    let zone_map = world
        .resource_mut::<Assets<ShaderStorageBuffer>>()
        .add(ShaderStorageBuffer::from(DISPLAY_VALUES.to_vec()));
    let material = world
        .resource_mut::<Assets<ZoneMaterial>>()
        .add(ZoneMaterial {
            zone_map,
            zone_data,
            width: DISPLAY_VALUES.len() as u32,
            height: 1,
        });
    let mesh = world
        .resource_mut::<Assets<Mesh>>()
        .add(Mesh::from(Rectangle::default()));
    let width = DISPLAY_VALUES.len() as f32 * ZONE_BLOCK_SIZE;

    world.spawn((
        Sprite::from_color(
            Color::srgb(0.08, 0.08, 0.1),
            Vec2::new(width, ZONE_BLOCK_SIZE),
        ),
        Transform::from_xyz(0.0, 0.0, 0.0),
    ));
    world.spawn((
        Mesh2d(mesh),
        MeshMaterial2d(material),
        Transform::from_xyz(0.0, 0.0, 1.0).with_scale(Vec3::new(width, ZONE_BLOCK_SIZE, 1.0)),
    ));
}

fn assert_mirror(world: &mut World) {
    let handle = world
        .query::<&ZoneMaterialHandleComponent>()
        .iter(world)
        .next()
        .expect("zone material handle must exist")
        .handle
        .clone();
    let grid = world.resource::<IntentGrid>();
    let materials = world.resource::<Assets<ZoneMaterial>>();
    let material = materials.get(&handle).expect("zone material must exist");

    for (cell, expected_kind) in PRESENT_CELLS.into_iter().zip(IntentKind::ALL) {
        let sim = grid.cell(cell).unwrap();
        let mirrored = material.zone_data[buffer_index(cell)];
        for kind in IntentKind::ALL {
            let expected = kind == expected_kind;
            assert_eq!(sim.has(kind), expected);
            assert_eq!(mirrored.present(kind.index() as u32), expected);
            assert_eq!(
                mirrored.ownership(kind.index() as u32),
                if expected {
                    ZoneOwnership::Player
                } else {
                    ZoneOwnership::Absent
                }
            );
        }
    }
    for kind in IntentKind::ALL {
        assert!(grid.cell(OVERLAP_CELL).unwrap().has(kind));
        assert!(material.zone_data[buffer_index(OVERLAP_CELL)].present(kind.index() as u32));
        assert_eq!(
            material.zone_data[buffer_index(OVERLAP_CELL)].ownership(kind.index() as u32),
            ZoneOwnership::Overlap
        );
        assert!(!grid.cell(ABSENT_CELL).unwrap().has(kind));
        assert!(!material.zone_data[buffer_index(ABSENT_CELL)].present(kind.index() as u32));
    }
}

fn buffer_index(cell: IVec2) -> usize {
    let mut index = cell + IVec2::new(MAP_WIDTH as i32 / 2, MAP_HEIGHT as i32 / 2);
    index.y = MAP_HEIGHT as i32 - index.y - 1;
    index.y as usize * MAP_WIDTH as usize + index.x as usize
}

fn focus_camera(world: &mut World) {
    let mut query = world.query::<(&mut Transform, &mut Projection, &mut CameraZoom2d)>();
    for (mut transform, mut projection, mut zoom) in query.iter_mut(world) {
        transform.translation.x = 0.0;
        transform.translation.y = 0.0;
        zoom.zoom = FRAMING_SCALE;
        if let Projection::Orthographic(ortho) = &mut *projection {
            ortho.scale = FRAMING_SCALE;
        }
    }
}

fn hide_existing_scene(world: &mut World) {
    let mesh_entities = world
        .query_filtered::<Entity, With<Mesh2d>>()
        .iter(world)
        .collect::<Vec<_>>();
    let nanobot_entities = world
        .query_filtered::<Entity, With<Nanobot>>()
        .iter(world)
        .collect::<Vec<_>>();
    let sprite_entities = world
        .query_filtered::<Entity, (With<Sprite>, Without<NanobotVisual>)>()
        .iter(world)
        .collect::<Vec<_>>();
    let ui_entities = world
        .query_filtered::<Entity, With<Node>>()
        .iter(world)
        .collect::<Vec<_>>();
    for entity in mesh_entities
        .into_iter()
        .chain(nanobot_entities)
        .chain(sprite_entities)
        .chain(ui_entities)
    {
        world.entity_mut(entity).insert(Visibility::Hidden);
    }
}
