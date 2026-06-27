//! Integration tests for zoom-aware fill bars above structures and loaded haulers.

use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    fly_camera::CameraZoom2d,
    nanobot::{
        Charger, HaulerLoad, PlannedKind, PlannedStructure, ProductionFacility,
        HAULER_CARRY_CAPACITY,
    },
    resources::{ResourceDeposit, ResourceKind, Stockpile, StockpileRole},
    structure_overlay::{
        fill_fraction, overlay_bar_size, overlay_fill_color, overlay_label_offset_y,
        StructureOverlay, StructureOverlayFill, StructureOverlayKind, StructureOverlayPlugin,
        StructureOverlaySettings, STRUCTURE_FOOTPRINT_LABEL_GAP, STRUCTURE_OVERLAY_Z,
    },
    GAMEPLAY_SPRITE_Z, MAP_HEIGHT, MAP_WIDTH, ZONE_BLOCK_SIZE,
};

#[path = "../common/mod.rs"]
mod common;

fn build_app() -> App {
    common::sim_app_with_overlay()
}

#[test]
fn deposit_overlay_bar_uses_amount_over_capacity() {
    let mut app = build_app();
    let deposit = common::spawn_deposit(&mut app, Vec2::ZERO, 500);

    app.update();

    let overlay = find_overlay_for(&mut app, deposit);
    assert_eq!(kind_of(&app, overlay), StructureOverlayKind::Deposit);
    assert_fill_fraction(&app, overlay, StructureOverlayKind::Deposit, 0.5);
}

#[test]
fn stockpile_overlay_bar_uses_amount_over_capacity() {
    let mut app = build_app();
    let stockpile = common::spawn_stockpile(&mut app, Vec2::ZERO, 120, 1000);

    app.update();

    let overlay = find_overlay_for(&mut app, stockpile);
    assert_eq!(kind_of(&app, overlay), StructureOverlayKind::Stockpile);
    assert_fill_fraction(&app, overlay, StructureOverlayKind::Stockpile, 0.12);
}

#[test]
fn empty_structure_still_shows_empty_bar() {
    let mut app = build_app();
    let stockpile = common::spawn_stockpile(&mut app, Vec2::ZERO, 0, 1000);

    app.update();

    let overlay = find_overlay_for(&mut app, stockpile);
    assert_fill_fraction(&app, overlay, StructureOverlayKind::Stockpile, 0.0);
}

#[test]
fn facility_overlay_uses_input_hopper_not_production_progress() {
    let mut app = build_app();
    let mut facility = ProductionFacility::new();
    facility.input_amount = 50;
    facility.input_capacity = 200;
    facility.current_target =
        Some(top_down_2d_rts_prototype_nano_swarm::nanobot::NanobotType::Worker);
    facility.progress = 4;
    let entity = app
        .world_mut()
        .spawn((facility, Transform::from_translation(Vec3::ZERO)))
        .id();

    app.update();

    let overlay = find_overlay_for(&mut app, entity);
    assert_eq!(kind_of(&app, overlay), StructureOverlayKind::Facility);
    assert_fill_fraction(&app, overlay, StructureOverlayKind::Facility, 0.25);
}

#[test]
fn planned_overlay_bar_uses_build_progress() {
    let mut app = build_app();
    let planned = common::spawn_planned_structure_of_kind_at_cell(
        &mut app,
        IVec2::ZERO,
        PlannedKind::SinkStockpile,
    );
    app.world_mut()
        .entity_mut(planned)
        .get_mut::<PlannedStructure>()
        .unwrap()
        .work_remaining = 3;

    app.update();

    let overlay = find_overlay_for(&mut app, planned);
    assert_eq!(kind_of(&app, overlay), StructureOverlayKind::Planned);
    assert_fill_fraction(&app, overlay, StructureOverlayKind::Planned, 0.4);
}

#[test]
fn charger_overlay_bar_uses_amount_over_capacity() {
    let mut app = build_app();
    let charger = common::spawn_charger_at(&mut app, IVec2::ZERO, 25);
    app.world_mut()
        .entity_mut(charger)
        .get_mut::<Charger>()
        .unwrap()
        .capacity = 100;

    app.update();

    let overlay = find_overlay_for(&mut app, charger);
    assert_eq!(kind_of(&app, overlay), StructureOverlayKind::Charger);
    assert_fill_fraction(&app, overlay, StructureOverlayKind::Charger, 0.25);
}

#[test]
fn hauler_overlay_spawns_only_when_loaded_and_uses_load_capacity() {
    let mut app = build_app();
    let hauler = common::spawn_hauler_at(&mut app, Vec2::ZERO);

    app.update();
    assert_no_overlay_for(&mut app, hauler);

    app.world_mut().entity_mut(hauler).insert(HaulerLoad {
        kind: ResourceKind::Minerals,
        amount: HAULER_CARRY_CAPACITY / 2,
    });
    app.update();

    let overlay = find_overlay_for(&mut app, hauler);
    assert_eq!(kind_of(&app, overlay), StructureOverlayKind::Hauler);
    assert_fill_fraction(&app, overlay, StructureOverlayKind::Hauler, 0.5);
}

#[test]
fn hauler_overlay_despawns_when_load_is_removed() {
    let mut app = build_app();
    let hauler = common::spawn_hauler_at(&mut app, Vec2::ZERO);
    app.world_mut().entity_mut(hauler).insert(HaulerLoad {
        kind: ResourceKind::Minerals,
        amount: HAULER_CARRY_CAPACITY,
    });

    app.update();
    let overlay = find_overlay_for(&mut app, hauler);
    assert!(app.world().get_entity(overlay).is_ok());

    app.world_mut().entity_mut(hauler).remove::<HaulerLoad>();
    app.update();

    assert!(app.world().get_entity(overlay).is_err());
    assert_no_overlay_for(&mut app, hauler);
}

#[test]
fn no_text_overlay_entities_are_spawned() {
    let mut app = build_app();
    let _deposit = common::spawn_deposit(&mut app, Vec2::ZERO, 500);
    let hauler = common::spawn_hauler_at(&mut app, Vec2::new(64.0, 0.0));
    app.world_mut().entity_mut(hauler).insert(HaulerLoad {
        kind: ResourceKind::Minerals,
        amount: 10,
    });

    app.update();

    let mut q = app
        .world_mut()
        .query::<(&StructureOverlay, Option<&Text2d>)>();
    for (overlay, text) in q.iter(app.world()) {
        assert!(
            text.is_none(),
            "overlay {:?} for {:?} must be bar-only, not Text2d",
            overlay.kind,
            overlay.target
        );
    }
}

#[test]
fn overlay_fill_reflects_live_state_changes() {
    let mut app = build_app();
    let deposit = common::spawn_deposit(&mut app, Vec2::ZERO, 1000);

    app.update();
    let overlay = find_overlay_for(&mut app, deposit);
    assert_fill_fraction(&app, overlay, StructureOverlayKind::Deposit, 1.0);

    app.world_mut()
        .entity_mut(deposit)
        .get_mut::<ResourceDeposit>()
        .unwrap()
        .amount = 250;
    app.update();

    assert_fill_fraction(&app, overlay, StructureOverlayKind::Deposit, 0.25);
}

#[test]
fn overlay_position_tracks_target_world_transform() {
    let mut app = build_app();
    let far_pos = Vec2::new(
        -(MAP_WIDTH as f32 * ZONE_BLOCK_SIZE) * 0.5 + 16.0,
        -(MAP_HEIGHT as f32 * ZONE_BLOCK_SIZE) * 0.5 + 16.0,
    );
    let deposit = common::spawn_deposit(&mut app, far_pos, 100);
    let deposit_radius = app
        .world()
        .entity(deposit)
        .get::<ResourceDeposit>()
        .unwrap()
        .radius;
    let expected_offset =
        overlay_label_offset_y(StructureOverlayKind::Deposit, Some(deposit_radius));

    app.update();

    let overlay = find_overlay_for(&mut app, deposit);
    let translation = app
        .world()
        .entity(overlay)
        .get::<Transform>()
        .unwrap()
        .translation;
    assert!((translation.x - far_pos.x).abs() < 1.0);
    assert!((translation.y - (far_pos.y + expected_offset)).abs() < 1.0);
    assert_eq!(translation.z, STRUCTURE_OVERLAY_Z);
}

#[test]
fn deposit_overlay_sits_above_deposit_circle() {
    let mut app = build_app();
    let pos = Vec2::new(2048.0, 1024.0);
    let deposit = common::spawn_deposit_with_radius(&mut app, pos, 250, 96.0);

    app.update();

    let overlay = find_overlay_for(&mut app, deposit);
    let translation = app.world().entity(overlay).get::<Transform>().unwrap();
    let expected_y = pos.y + 96.0 + STRUCTURE_FOOTPRINT_LABEL_GAP;
    assert!((translation.translation.y - expected_y).abs() < 1.0);
    assert!(translation.translation.y > pos.y);
}

#[test]
fn overlay_renders_above_gameplay_sprites() {
    let mut app = build_app();
    let deposit = common::spawn_deposit(&mut app, Vec2::ZERO, 100);

    app.update();

    let overlay = find_overlay_for(&mut app, deposit);
    let z = app
        .world()
        .entity(overlay)
        .get::<Transform>()
        .unwrap()
        .translation
        .z;
    assert!(z > GAMEPLAY_SPRITE_Z);
    assert_eq!(z, STRUCTURE_OVERLAY_Z);
}

#[test]
fn default_threshold_hides_overlay_at_boundary_and_shows_below() {
    let mut app = build_app();
    let deposit = common::spawn_deposit(&mut app, Vec2::ZERO, 100);
    let camera = app
        .world_mut()
        .spawn(CameraZoom2d {
            zoom: 7.99,
            ..default()
        })
        .id();

    app.update();
    let overlay = find_overlay_for(&mut app, deposit);
    assert_eq!(visibility_of(&app, overlay), Visibility::Inherited);

    app.world_mut()
        .entity_mut(camera)
        .get_mut::<CameraZoom2d>()
        .unwrap()
        .zoom = 8.0;
    app.update();
    assert_eq!(visibility_of(&app, overlay), Visibility::Hidden);
}

#[test]
fn configured_threshold_gates_visibility() {
    let mut app = build_app();
    let deposit = common::spawn_deposit(&mut app, Vec2::ZERO, 100);
    app.world_mut()
        .resource_mut::<StructureOverlaySettings>()
        .hide_zoom_threshold = 0.5;
    app.world_mut().spawn(CameraZoom2d {
        zoom: 1.0,
        ..default()
    });

    app.update();

    let overlay = find_overlay_for(&mut app, deposit);
    assert_eq!(visibility_of(&app, overlay), Visibility::Hidden);
}

#[test]
fn overlay_visibility_does_not_touch_unrelated_entities() {
    let mut app = build_app();
    let deposit = common::spawn_deposit(&mut app, Vec2::ZERO, 100);
    let unrelated = app
        .world_mut()
        .spawn((Transform::default(), Visibility::Inherited))
        .id();
    app.world_mut().spawn(CameraZoom2d {
        zoom: 10.0,
        ..default()
    });

    app.update();

    let overlay = find_overlay_for(&mut app, deposit);
    assert_eq!(visibility_of(&app, overlay), Visibility::Hidden);
    assert_eq!(visibility_of(&app, unrelated), Visibility::Inherited);
}

#[test]
fn overlay_is_removed_when_target_despawns() {
    let mut app = build_app();
    let deposit = common::spawn_deposit(&mut app, Vec2::ZERO, 100);

    app.update();
    let overlay = find_overlay_for(&mut app, deposit);
    let fill = app
        .world()
        .entity(overlay)
        .get::<StructureOverlay>()
        .unwrap()
        .fill;

    app.world_mut().despawn(deposit);
    app.update();

    assert!(app.world().get_entity(overlay).is_err());
    assert!(app.world().get_entity(fill).is_err());
}

#[test]
fn overlay_spawns_once_and_persists_across_state_changes() {
    let mut app = build_app();
    let deposit = common::spawn_deposit(&mut app, Vec2::ZERO, 1000);

    app.update();
    let overlay = find_overlay_for(&mut app, deposit);

    for new_amount in [900, 500, 100, 0] {
        app.world_mut()
            .entity_mut(deposit)
            .get_mut::<ResourceDeposit>()
            .unwrap()
            .amount = new_amount;
        app.update();
        assert_fill_fraction(
            &app,
            overlay,
            StructureOverlayKind::Deposit,
            fill_fraction(new_amount, 1000),
        );
        assert!(app.world().get_entity(overlay).is_ok());
    }

    for _ in 0..5 {
        app.update();
    }
    let mut q = app.world_mut().query::<&StructureOverlay>();
    let count = q.iter(app.world()).filter(|o| o.target == deposit).count();
    assert_eq!(count, 1);
}

#[test]
fn overlay_spawns_for_every_kind_at_once() {
    let mut app = build_app();
    let cell = IVec2::ZERO;
    let deposit = common::spawn_deposit(&mut app, Vec2::ZERO, 100);
    let stockpile = common::spawn_stockpile(&mut app, Vec2::new(64.0, 0.0), 50, 200);
    let facility = app
        .world_mut()
        .spawn((
            ProductionFacility::new(),
            Transform::from_translation(Vec3::new(128.0, 0.0, 0.0)),
        ))
        .id();
    let planned =
        common::spawn_planned_structure_of_kind_at_cell(&mut app, cell, PlannedKind::SinkStockpile);
    let charger = common::spawn_charger_at(&mut app, cell, 30);
    let hauler = common::spawn_hauler_at(&mut app, Vec2::new(256.0, 0.0));
    app.world_mut().entity_mut(hauler).insert(HaulerLoad {
        kind: ResourceKind::Minerals,
        amount: 20,
    });

    app.update();

    let targets = [deposit, stockpile, facility, planned, charger, hauler];
    let mut kinds = Vec::with_capacity(targets.len());
    for target in targets {
        let overlay = find_overlay_for(&mut app, target);
        kinds.push(kind_of(&app, overlay));
    }
    assert_eq!(
        kinds,
        vec![
            StructureOverlayKind::Deposit,
            StructureOverlayKind::Stockpile,
            StructureOverlayKind::Facility,
            StructureOverlayKind::Planned,
            StructureOverlayKind::Charger,
            StructureOverlayKind::Hauler,
        ]
    );
}

#[test]
fn fill_colors_are_pairwise_distinct() {
    let colors: Vec<_> = StructureOverlayKind::ALL
        .iter()
        .map(|k| overlay_fill_color(*k))
        .collect();
    for i in 0..colors.len() {
        for j in (i + 1)..colors.len() {
            assert_ne!(colors[i], colors[j]);
        }
    }
}

// Regression: when a PlannedStructure is promoted in place to
// a Stockpile (the production code path), the stale Planned
// overlay must be replaced by a Stockpile overlay whose fill
// tracks the stockpile's amount/capacity. Before the fix the
// Planned overlay was left attached, kept reading the removed
// PlannedStructure component, and rendered a permanently empty
// bar even as the stockpile filled.
#[test]
fn overlay_kind_refreshes_when_planned_structure_is_promoted() {
    let mut app = build_app();
    let planned = common::spawn_planned_structure_of_kind_at_cell(
        &mut app,
        IVec2::ZERO,
        PlannedKind::SinkStockpile,
    );

    app.update();
    let planned_overlay = find_overlay_for(&mut app, planned);
    assert_eq!(
        kind_of(&app, planned_overlay),
        StructureOverlayKind::Planned
    );

    // Promote in place: drop PlannedStructure, stamp the
    // completed Stockpile payload. Mirrors
    // promote_planned_to_completion in src/nanobot/planned.rs.
    app.world_mut()
        .entity_mut(planned)
        .remove::<PlannedStructure>()
        .insert((
            Stockpile {
                kind: ResourceKind::Minerals,
                amount: 30,
                capacity: 100,
                radius: 32.0,
            },
            StockpileRole::Sink,
        ));

    app.update();

    // The stale Planned overlay must be gone; a fresh Stockpile
    // overlay must point at the same target and render the
    // stockpile's fill fraction.
    assert!(app.world().get_entity(planned_overlay).is_err());
    let overlay = find_overlay_for(&mut app, planned);
    assert_eq!(kind_of(&app, overlay), StructureOverlayKind::Stockpile);
    assert_fill_fraction(&app, overlay, StructureOverlayKind::Stockpile, 0.3);

    // And it must keep tracking as the stockpile fills further.
    app.world_mut()
        .entity_mut(planned)
        .get_mut::<Stockpile>()
        .unwrap()
        .amount = 80;
    app.update();
    assert_fill_fraction(&app, overlay, StructureOverlayKind::Stockpile, 0.8);
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn find_overlay_for(app: &mut App, target: Entity) -> Entity {
    let world = app.world_mut();
    let mut q = world.query::<(Entity, &StructureOverlay)>();
    q.iter(world)
        .find(|(_, o)| o.target == target)
        .map(|(e, _)| e)
        .unwrap_or_else(|| panic!("no StructureOverlay points at target {target:?}"))
}

fn assert_no_overlay_for(app: &mut App, target: Entity) {
    let world = app.world_mut();
    let mut q = world.query::<&StructureOverlay>();
    assert!(q.iter(world).all(|o| o.target != target));
}

fn kind_of(app: &App, overlay: Entity) -> StructureOverlayKind {
    app.world()
        .entity(overlay)
        .get::<StructureOverlay>()
        .unwrap()
        .kind
}

fn visibility_of(app: &App, entity: Entity) -> Visibility {
    app.world()
        .entity(entity)
        .get::<Visibility>()
        .copied()
        .unwrap()
}

fn assert_fill_fraction(app: &App, overlay: Entity, kind: StructureOverlayKind, expected: f32) {
    let overlay_component = app
        .world()
        .entity(overlay)
        .get::<StructureOverlay>()
        .unwrap();
    let fill = overlay_component.fill;
    let expected_width = overlay_bar_size(kind).x * expected;
    let (actual_size, actual_x, marker_present) = {
        let fill_ref = app.world().entity(fill);
        let sprite = fill_ref.get::<Sprite>().unwrap();
        let transform = fill_ref.get::<Transform>().unwrap();
        (
            sprite.custom_size.unwrap(),
            transform.translation.x,
            fill_ref.contains::<StructureOverlayFill>(),
        )
    };
    assert!(marker_present, "fill child must carry StructureOverlayFill");
    assert!(
        (actual_size.x - expected_width).abs() < 0.01,
        "fill width {} must match expected {} for {expected}",
        actual_size.x,
        expected_width
    );
    assert_eq!(actual_size.y, overlay_bar_size(kind).y);
    let expected_x = -(overlay_bar_size(kind).x - expected_width) / 2.0;
    assert!((actual_x - expected_x).abs() < 0.01);
}

#[allow(dead_code)]
fn _exports() {
    let _: Charger = Charger::new(IVec2::ZERO);
    let _: PlannedStructure = PlannedStructure::new(PlannedKind::Charger, IVec2::ZERO);
    let _: ProductionFacility = ProductionFacility::new();
    let _: StructureOverlayPlugin = StructureOverlayPlugin;
}
