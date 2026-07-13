//! Offscreen evidence for binary zone overlay presence, colours, overlap, and opacity.

use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    fly_camera::CameraZoom2d,
    intent::{IntentGrid, IntentKind},
    nanobot::SwarmId,
    zones::{ZoneMaterial, ZoneMaterialHandleComponent},
    MAP_HEIGHT, MAP_WIDTH, ZONE_BLOCK_SIZE,
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
const CAPTURE_FRAME: u32 = 8;
const FRAMING_SCALE: f32 = 3.2;

pub fn zone_binary_overlay(ctx: &mut TestContext) -> TestFlow {
    if ctx.frame == 2 {
        focus_camera(ctx.world);
        hide_existing_sprites(ctx.world);
        hide_existing_ui(ctx.world);
        paint_examples(ctx.world);
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
    let center = Vec2::new(0.5 * ZONE_BLOCK_SIZE, 5.5 * ZONE_BLOCK_SIZE);
    let mut query = world.query::<(&mut Transform, &mut Projection, &mut CameraZoom2d)>();
    for (mut transform, mut projection, mut zoom) in query.iter_mut(world) {
        transform.translation.x = center.x;
        transform.translation.y = center.y;
        zoom.zoom = FRAMING_SCALE;
        if let Projection::Orthographic(ortho) = &mut *projection {
            ortho.scale = FRAMING_SCALE;
        }
    }
}

fn hide_existing_sprites(world: &mut World) {
    let entities: Vec<Entity> = world
        .query_filtered::<Entity, With<Sprite>>()
        .iter(world)
        .collect();
    for entity in entities {
        world.entity_mut(entity).insert(Visibility::Hidden);
    }
}

fn hide_existing_ui(world: &mut World) {
    let entities: Vec<Entity> = world
        .query_filtered::<Entity, With<Node>>()
        .iter(world)
        .collect();
    for entity in entities {
        world.entity_mut(entity).insert(Visibility::Hidden);
    }
}
