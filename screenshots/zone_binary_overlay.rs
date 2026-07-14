//! Offscreen evidence for binary zone presence, colours, overlap, and opacity.

use bevy::{prelude::*, render::storage::ShaderStorageBuffer};
use top_down_2d_rts_prototype_nano_swarm::{
    MAP_HEIGHT, MAP_WIDTH, ZONE_BLOCK_SIZE,
    fly_camera::CameraZoom2d,
    intent::{IntentGrid, IntentKind},
    nanobot::SwarmId,
    zones::{ZoneMaterial, ZoneMaterialHandleComponent, ZonePointData},
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
const DISPLAY_BITS: [u32; 6] = [0, 1, 2, 4, 8, 15];
const CAPTURE_FRAME: u32 = 60;
const FRAMING_SCALE: f32 = 2.8;

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

fn paint_examples(world: &mut World) {
    let mut grid = world.resource_mut::<IntentGrid>();
    for (cell, kind) in PRESENT_CELLS.into_iter().zip(IntentKind::ALL) {
        grid.add_owned(cell, kind, Some(SwarmId::PLAYER));
    }
    for kind in IntentKind::ALL {
        grid.add_owned(OVERLAP_CELL, kind, Some(SwarmId::PLAYER));
    }
}

/// Spawn one test strip through production [`ZoneMaterial`]: absent, each kind,
/// then all kinds overlapped. Static test data makes rendered colour evidence
/// independent from asynchronous main-world storage-buffer extraction, while
/// [`assert_mirror`] separately proves simulation-to-material mirroring.
fn spawn_binary_display(world: &mut World) {
    let zone_data = DISPLAY_BITS
        .into_iter()
        .map(|active| ZonePointData { active })
        .collect::<Vec<_>>();
    let zone_map = world
        .resource_mut::<Assets<ShaderStorageBuffer>>()
        .add(ShaderStorageBuffer::from(DISPLAY_BITS.to_vec()));
    let material = world
        .resource_mut::<Assets<ZoneMaterial>>()
        .add(ZoneMaterial {
            zone_map,
            zone_data,
            width: DISPLAY_BITS.len() as u32,
            height: 1,
        });
    let mesh = world
        .resource_mut::<Assets<Mesh>>()
        .add(Mesh::from(Rectangle::default()));
    let width = DISPLAY_BITS.len() as f32 * ZONE_BLOCK_SIZE;

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
        }
    }
    for kind in IntentKind::ALL {
        assert!(grid.cell(OVERLAP_CELL).unwrap().has(kind));
        assert!(material.zone_data[buffer_index(OVERLAP_CELL)].present(kind.index() as u32));
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
    let sprite_entities = world
        .query_filtered::<Entity, With<Sprite>>()
        .iter(world)
        .collect::<Vec<_>>();
    let ui_entities = world
        .query_filtered::<Entity, With<Node>>()
        .iter(world)
        .collect::<Vec<_>>();
    for entity in mesh_entities
        .into_iter()
        .chain(sprite_entities)
        .chain(ui_entities)
    {
        world.entity_mut(entity).insert(Visibility::Hidden);
    }
}
