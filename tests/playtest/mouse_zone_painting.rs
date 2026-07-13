//! Scripted playtest for the visible mouse-painting regression
//! (issue #18). Drives the full capture -> brush -> mirror chain
//! and asserts deterministic ECS state on the visible side of the
//! render mirror.

use bevy::{camera::RenderTargetInfo, prelude::*, render::storage::ShaderStorageBuffer};
use top_down_2d_rts_prototype_nano_swarm::{
    intent::{
        brush_key_for_kind, brush_selection_keyboard_system, BrushSelection, IntentGrid, IntentKind,
    },
    ui::{check_ui_interaction, UiHandling},
    zones::{
        mirror_intent_to_zone_material_system, zone_brush_system, ZoneMaterial,
        ZoneMaterialHandleComponent, ZonePointData,
    },
    MAP_HEIGHT, MAP_WIDTH, ZONE_BLOCK_SIZE,
};

#[path = "../common/mod.rs"]
mod common;

use common::cell_world_center;

fn build_app() -> App {
    // No `DefaultPlugins`: the headless schedule has no render world
    // and no window, and the swarm simulation plugins pull in
    // resources the brush does not read. `TransformPlugin` keeps
    // `GlobalTransform` in sync so `viewport_to_world_2d` projects
    // to the camera's actual world position.
    let mut app = App::new();
    app.add_plugins(bevy::time::TimePlugin)
        .add_plugins(bevy::transform::TransformPlugin)
        .insert_resource(UiHandling::default())
        .init_resource::<BrushSelection>()
        .insert_resource(IntentGrid::new(MAP_WIDTH as i32, MAP_HEIGHT as i32))
        .init_resource::<ButtonInput<KeyCode>>()
        .init_resource::<ButtonInput<MouseButton>>()
        .init_resource::<Assets<ZoneMaterial>>()
        .init_resource::<Assets<ShaderStorageBuffer>>()
        .add_systems(
            Update,
            (
                brush_selection_keyboard_system,
                check_ui_interaction,
                zone_brush_system,
                mirror_intent_to_zone_material_system,
            )
                .chain(),
        );
    app
}

fn spawn_window(app: &mut App) -> Entity {
    app.world_mut()
        .spawn(Window {
            resolution: (1280, 720).into(),
            ..default()
        })
        .id()
}

fn set_cursor(app: &mut App, window: Entity, cursor: Vec2) {
    let world = app.world_mut();
    let mut entity = world.entity_mut(window);
    let mut window_ref = entity
        .get_mut::<Window>()
        .expect("window entity must carry a Window component");
    window_ref.set_cursor_position(Some(cursor));
}

/// Spawn a [`Camera2d`] at `world_pos`. The render graph does not
/// run in the headless test schedule, so we pre-fill
/// `computed.target_info`; otherwise `viewport_to_world_2d` returns
/// an error and the brush silently no-ops.
fn spawn_camera(app: &mut App, world_pos: Vec2) -> Entity {
    let entity = app
        .world_mut()
        .spawn((
            Camera2d,
            Transform::from_translation(world_pos.extend(0.0)),
            GlobalTransform::default(),
        ))
        .id();
    let mut entity_ref = app.world_mut().entity_mut(entity);
    let mut camera = entity_ref
        .get_mut::<Camera>()
        .expect("Camera2d spawn must include a Camera component");
    camera.computed.target_info = Some(RenderTargetInfo {
        physical_size: UVec2::new(1280, 720),
        scale_factor: 1.0,
    });
    entity
}

/// Spawn a [`ZoneMaterial`] matching the production setup plus the
/// `ZoneMaterialHandleComponent` the mirror system reads.
fn spawn_zone_material(app: &mut App) -> Entity {
    // `resource_scope` lets us hold the two ResMuts at once and
    // delegate the buffer + material wiring to `ZoneMaterial::new`.
    let handle = app.world_mut().resource_scope(
        |world, mut buffers: Mut<'_, Assets<ShaderStorageBuffer>>| {
            let mut zone_mats = world
                .get_resource_mut::<Assets<ZoneMaterial>>()
                .expect("Assets<ZoneMaterial> must be initialised before spawning a zone material");
            zone_mats.add(ZoneMaterial::new(MAP_WIDTH, MAP_HEIGHT, &mut buffers))
        },
    );
    app.world_mut()
        .spawn(ZoneMaterialHandleComponent { handle })
        .id()
}

fn press_key(app: &mut App, key: KeyCode) {
    let mut keyboard = ButtonInput::<KeyCode>::default();
    keyboard.press(key);
    app.insert_resource(keyboard);
    app.update();
}

fn press_mouse(app: &mut App, button: MouseButton) {
    let mut input = ButtonInput::<MouseButton>::default();
    input.press(button);
    app.insert_resource(input);
}

fn clear_mouse(app: &mut App) {
    app.insert_resource(ButtonInput::<MouseButton>::default());
}

/// Map a centered grid cell to its linear index inside the GPU
/// buffer. The mirror system applies the same `y = height - y - 1`
/// flip the production code uses.
fn cell_buffer_index(cell: IVec2) -> usize {
    let half = IVec2::new(MAP_WIDTH as i32 / 2, MAP_HEIGHT as i32 / 2);
    let mut idx = cell + half;
    idx.y = MAP_HEIGHT as i32 - idx.y - 1;
    (idx.y as usize) * (MAP_WIDTH as usize) + (idx.x as usize)
}

/// Read the [`ZonePointData`] the mirror pass wrote for `cell`.
fn zone_cell(app: &App, material_entity: Entity, cell: IVec2) -> ZonePointData {
    let zone_handle = app
        .world()
        .entity(material_entity)
        .get::<ZoneMaterialHandleComponent>()
        .expect("zone material entity must keep its handle")
        .handle
        .clone();
    let zone_mats = app.world().resource::<Assets<ZoneMaterial>>();
    let zone_mat = zone_mats
        .get(&zone_handle)
        .expect("zone material must still be alive after the mirror pass");
    zone_mat.zone_data[cell_buffer_index(cell)]
}

#[test]
fn scripted_mouse_paint_writes_visible_zone_overlay_at_cursor() {
    // Frame 0: set up world, camera, window, cursor, zone material.
    // The cursor sits at the world origin (the screen centre for the
    // 1280x720 viewport with the camera at the origin); the brush
    // maps that to the (0, 0) cell of the intent grid.
    let mut app = build_app();
    let window = spawn_window(&mut app);
    set_cursor(&mut app, window, Vec2::new(640.0, 360.0));
    spawn_camera(&mut app, Vec2::ZERO);
    let material_entity = spawn_zone_material(&mut app);

    // The default brush is Gather; rely on the resource contract.
    assert_eq!(
        app.world().resource::<BrushSelection>().kind,
        IntentKind::Gather,
        "default brush must be Gather"
    );

    // Same frame the player holds left mouse the chain fires in
    // production order: capture -> brush -> mirror.
    press_mouse(&mut app, MouseButton::Left);
    app.update();

    let cursor_cell = IVec2::new(0, 0);
    {
        let grid = app.world().resource::<IntentGrid>();
        let cell = grid
            .cell(cursor_cell)
            .expect("cursor cell must be inside the intent grid");
        assert!(
            cell.has(IntentKind::Gather),
            "Gather must be active at the cursor cell after the player paints"
        );
        for kind in IntentKind::ALL {
            if kind == IntentKind::Gather {
                continue;
            }
            assert!(
                !cell.has(kind),
                "no other intent layer must be active after a Gather paint"
            );
        }
    }

    let cell = zone_cell(&app, material_entity, cursor_cell);
    assert!(
        cell.present(IntentKind::Gather.index() as u32),
        "ZoneMaterial must mirror Gather presence after paint"
    );
    for kind in IntentKind::ALL {
        if kind == IntentKind::Gather {
            continue;
        }
        assert!(
            !cell.present(kind.index() as u32),
            "non-painted layer {kind:?} must remain absent"
        );
    }
}

#[test]
fn scripted_mouse_paint_off_world_does_not_touch_zone_overlay() {
    // The grid is `MAP_WIDTH x MAP_HEIGHT` cells centred on the
    // origin. Place the camera far past the world edge so the
    // cursor (screen centre) projects off-grid; the brush's
    // bounds check must then drop the input.
    let mut app = build_app();
    let window = spawn_window(&mut app);
    let far_world = Vec2::new((MAP_WIDTH as f32 / 2.0 + 100.0) * ZONE_BLOCK_SIZE, 0.0);
    spawn_camera(&mut app, far_world);
    set_cursor(&mut app, window, Vec2::new(640.0, 360.0));
    let material_entity = spawn_zone_material(&mut app);

    press_mouse(&mut app, MouseButton::Left);
    app.update();

    let grid = app.world().resource::<IntentGrid>();
    assert_eq!(grid.render_dirty_count(), 0);

    let cell = zone_cell(&app, material_entity, IVec2::new(0, 0));
    for kind in IntentKind::ALL {
        assert!(!cell.present(kind.index() as u32));
    }
}

#[test]
fn scripted_layer_switch_then_paint_writes_second_layer_at_same_cell() {
    // Switch the active brush via the keyboard (the seam the
    // player uses) and paint a different layer on top of the
    // first. The mirror must keep both bits set in the same cell;
    // this is the overlap contract.
    let mut app = build_app();
    let window = spawn_window(&mut app);
    set_cursor(&mut app, window, Vec2::new(640.0, 360.0));
    spawn_camera(&mut app, Vec2::ZERO);
    let material_entity = spawn_zone_material(&mut app);

    press_mouse(&mut app, MouseButton::Left);
    app.update();
    clear_mouse(&mut app);

    press_key(
        &mut app,
        brush_key_for_kind(IntentKind::Defend).expect("Defend must have a key binding"),
    );
    assert_eq!(
        app.world().resource::<BrushSelection>().kind,
        IntentKind::Defend,
        "pressing 3 must switch the active layer to Defend"
    );

    press_mouse(&mut app, MouseButton::Left);
    app.update();
    clear_mouse(&mut app);

    let grid = app.world().resource::<IntentGrid>();
    let cell = grid
        .cell(IVec2::new(0, 0))
        .expect("cursor cell must be inside the intent grid");
    assert!(cell.has(IntentKind::Gather));
    assert!(cell.has(IntentKind::Defend));

    let cell = zone_cell(&app, material_entity, IVec2::new(0, 0));
    assert!(cell.present(IntentKind::Gather.index() as u32));
    assert!(cell.present(IntentKind::Defend.index() as u32));
    assert!(!cell.present(IntentKind::Build.index() as u32));
    assert!(!cell.present(IntentKind::Corridor.index() as u32));
}

#[test]
fn scripted_right_mouse_erase_clears_visible_bit_at_cursor() {
    // Right mouse erases the active layer. The same frame's mirror
    // pass must clear the bit; otherwise a phantom pixel would
    // survive the erase.
    let mut app = build_app();
    let window = spawn_window(&mut app);
    set_cursor(&mut app, window, Vec2::new(640.0, 360.0));
    spawn_camera(&mut app, Vec2::ZERO);
    let material_entity = spawn_zone_material(&mut app);

    press_mouse(&mut app, MouseButton::Left);
    app.update();
    clear_mouse(&mut app);

    // One right-click frame clears binary Gather paint immediately.
    press_mouse(&mut app, MouseButton::Right);
    app.update();
    clear_mouse(&mut app);

    let grid = app.world().resource::<IntentGrid>();
    let cell = grid
        .cell(IVec2::new(0, 0))
        .expect("cursor cell must be inside the intent grid");
    assert!(
        !cell.has(IntentKind::Gather),
        "Gather must be removed from the cell after a one-frame right-click"
    );

    let cell = zone_cell(&app, material_entity, IVec2::new(0, 0));
    assert!(!cell.present(IntentKind::Gather.index() as u32));
    for kind in IntentKind::ALL {
        assert!(!cell.present(kind.index() as u32));
    }
}

#[test]
fn scripted_paint_at_world_corner_lands_in_corner_cell() {
    // Catches a regression in `get_zone_pos_from_world` that would
    // shift every paint towards the origin.
    let mut app = build_app();
    let window = spawn_window(&mut app);
    // Place the camera at the world corner's cell centre so a
    // cursor at the screen centre projects exactly to that cell.
    let corner_cell = IVec2::new(-(MAP_WIDTH as i32) / 2, -(MAP_HEIGHT as i32) / 2);
    spawn_camera(&mut app, cell_world_center(corner_cell));
    set_cursor(&mut app, window, Vec2::new(640.0, 360.0));
    let material_entity = spawn_zone_material(&mut app);

    press_mouse(&mut app, MouseButton::Left);
    app.update();

    let cell = zone_cell(&app, material_entity, corner_cell);
    assert!(cell.present(IntentKind::Gather.index() as u32));

    let grid = app.world().resource::<IntentGrid>();
    let cell = grid
        .cell(corner_cell)
        .expect("corner cell must be inside the intent grid");
    assert!(
        cell.has(IntentKind::Gather),
        "world-corner paint must activate Gather in the corner cell"
    );
}
