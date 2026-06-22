//! Integration tests for issue #30: zoom-aware structure
//! status overlays.

use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    fly_camera::CameraZoom2d,
    nanobot::{Charger, PlannedKind, PlannedStructure, ProductionFacility, ProductionRatio},
    resources::ResourceDeposit,
    structure_overlay::{
        overlay_background_color, overlay_label_offset_y, StructureOverlay, StructureOverlayKind,
        StructureOverlayPlugin, StructureOverlaySettings, STRUCTURE_FOOTPRINT_LABEL_GAP,
        STRUCTURE_OVERLAY_Z,
    },
    GAMEPLAY_SPRITE_Z, MAP_HEIGHT, MAP_WIDTH, ZONE_BLOCK_SIZE,
};

#[path = "../common/mod.rs"]
mod common;

fn build_app() -> App {
    common::sim_app_with_overlay()
}

#[test]
fn deposit_overlay_shows_remaining_amount() {
    let mut app = build_app();
    let deposit = common::spawn_deposit(&mut app, Vec2::new(0.0, 0.0), 840);

    app.update();

    let overlay = find_overlay_for(&mut app, deposit);
    assert_eq!(text_for(&mut app, overlay), "Deposit 840");
    assert_eq!(kind_of(&mut app, overlay), StructureOverlayKind::Deposit);
}

#[test]
fn stockpile_overlay_shows_amount_and_capacity() {
    let mut app = build_app();
    let stockpile = common::spawn_stockpile(&mut app, Vec2::new(0.0, 0.0), 120, 1000);

    app.update();

    let overlay = find_overlay_for(&mut app, stockpile);
    assert_eq!(text_for(&mut app, overlay), "Stockpile 120/1000");
    assert_eq!(kind_of(&mut app, overlay), StructureOverlayKind::Stockpile);
}

#[test]
fn facility_overlay_shows_idle_or_progress() {
    let mut app = build_app();
    app.world_mut().insert_resource(ProductionRatio::default());

    let idle = common::spawn_idle_facility_at(&mut app, Vec2::new(0.0, 0.0));
    let working = common::spawn_busy_facility_at(
        &mut app,
        Vec2::new(64.0, 0.0),
        top_down_2d_rts_prototype_nano_swarm::nanobot::NanobotType::Worker,
    );

    app.update();

    let idle_overlay = find_overlay_for(&mut app, idle);
    let working_overlay = find_overlay_for(&mut app, working);
    assert_eq!(text_for(&mut app, idle_overlay), "Facility: idle");
    assert_eq!(
        kind_of(&mut app, idle_overlay),
        StructureOverlayKind::Facility
    );
    let working_text = text_for(&mut app, working_overlay);
    assert!(working_text.starts_with("Facility: Worker "));
    assert!(working_text.ends_with('%'));
}

#[test]
fn planned_overlay_shows_kind_and_percent() {
    let mut app = build_app();
    let planned = common::spawn_planned_structure_of_kind_at_cell(
        &mut app,
        IVec2::ZERO,
        PlannedKind::SinkStockpile,
    );

    app.update();

    let overlay = find_overlay_for(&mut app, planned);
    let text = text_for(&mut app, overlay);
    assert!(text.starts_with("Building Stockpile "));
    assert!(text.ends_with('%'));
    assert_eq!(kind_of(&mut app, overlay), StructureOverlayKind::Planned);
}

#[test]
fn charger_overlay_shows_amount_and_capacity() {
    let mut app = build_app();
    let charger = common::spawn_charger_at(&mut app, IVec2::ZERO, 12);

    app.update();

    let overlay = find_overlay_for(&mut app, charger);
    let text = text_for(&mut app, overlay);
    assert!(text.starts_with("Charger 12/"), "got {text}");
    assert_eq!(kind_of(&mut app, overlay), StructureOverlayKind::Charger);
}

#[test]
fn overlay_label_reflects_live_state_changes() {
    let mut app = build_app();
    let deposit = common::spawn_deposit(&mut app, Vec2::new(0.0, 0.0), 100);

    app.update();
    let overlay = find_overlay_for(&mut app, deposit);
    assert_eq!(text_for(&mut app, overlay), "Deposit 100");

    {
        let world = app.world_mut();
        let mut e = world.entity_mut(deposit);
        let mut d = *e.get::<ResourceDeposit>().unwrap();
        d.amount = 42;
        e.insert(d);
    }
    app.update();
    assert_eq!(text_for(&mut app, overlay), "Deposit 42");
}

#[test]
fn overlay_position_tracks_target_world_transform() {
    let mut app = build_app();
    let far_pos = Vec2::new(
        -(MAP_WIDTH as f32 * ZONE_BLOCK_SIZE) * 0.5 + 16.0,
        -(MAP_HEIGHT as f32 * ZONE_BLOCK_SIZE) * 0.5 + 16.0,
    );
    // The per-kind offset helper consumes the deposit's
    // actual radius, so the test queries it back from
    // the entity instead of duplicating the constant.
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
        .translation
        .truncate();
    assert!(
        (translation.x - far_pos.x).abs() < 1.0,
        "overlay X ({}) must equal target X ({})",
        translation.x,
        far_pos.x
    );
    assert!(
        (translation.y - (far_pos.y + expected_offset)).abs() < 1.0,
        "overlay Y ({}) must equal target Y ({}) + per-kind offset ({expected_offset})",
        translation.y,
        far_pos.y
    );
}

#[test]
fn deposit_overlay_sits_above_deposit_circle() {
    // Issue #33: Resource Deposit labels render above
    // the deposit circle/sprite. The Y offset must equal
    // `deposit_radius + gap`, so a 96-unit-radius deposit
    // pins its label 108 units above its centre.
    let mut app = build_app();
    let pos = Vec2::new(2048.0, 1024.0);
    let deposit = common::spawn_deposit_with_radius(&mut app, pos, 250, 96.0);

    app.update();

    let overlay = find_overlay_for(&mut app, deposit);
    let translation = app.world().entity(overlay).get::<Transform>().unwrap();
    let expected_y = pos.y + 96.0 + STRUCTURE_FOOTPRINT_LABEL_GAP;
    assert!(
        (translation.translation.y - expected_y).abs() < 1.0,
        "deposit overlay Y ({}) must equal deposit Y ({}) + radius (96) + gap ({}) = {}",
        translation.translation.y,
        pos.y,
        STRUCTURE_FOOTPRINT_LABEL_GAP,
        expected_y,
    );
    assert!(
        translation.translation.y > pos.y,
        "deposit overlay must sit above the deposit centre"
    );
}

#[test]
fn structure_overlay_sits_above_structure_footprint() {
    // Issue #33: Stockpile, Production Facility,
    // Planned Structure, and Charger labels render
    // above the structure's footprint. The offset is
    // `PLANNED_STRUCTURE_FOOTPRINT/2 + gap`, so every
    // non-deposit kind uses the same per-kind offset.
    let mut app = build_app();
    app.world_mut().insert_resource(ProductionRatio::default());
    let cell = IVec2::new(2, 3);
    let center = common::cell_world_center(cell);
    let stockpile = common::spawn_stockpile(&mut app, center, 50, 200);
    let facility = common::spawn_idle_facility_at(&mut app, center + Vec2::new(128.0, 0.0));
    let planned =
        common::spawn_planned_structure_of_kind_at_cell(&mut app, cell, PlannedKind::Charger);
    let charger = common::spawn_charger_at(&mut app, cell, 30);

    app.update();

    let expected_y_stockpile =
        center.y + overlay_label_offset_y(StructureOverlayKind::Stockpile, None);
    let expected_y_facility = (center + Vec2::new(128.0, 0.0)).y
        + overlay_label_offset_y(StructureOverlayKind::Facility, None);
    let expected_y_planned = center.y + overlay_label_offset_y(StructureOverlayKind::Planned, None);
    let expected_y_charger = center.y + overlay_label_offset_y(StructureOverlayKind::Charger, None);

    for (target, expected_y, label) in [
        (stockpile, expected_y_stockpile, "stockpile"),
        (facility, expected_y_facility, "facility"),
        (planned, expected_y_planned, "planned"),
        (charger, expected_y_charger, "charger"),
    ] {
        let overlay = find_overlay_for(&mut app, target);
        let translation = app
            .world()
            .entity(overlay)
            .get::<Transform>()
            .unwrap()
            .translation
            .y;
        assert!(
            (translation - expected_y).abs() < 1.0,
            "{label} overlay Y ({translation}) must equal footprint Y ({expected_y})",
        );
        assert!(
            translation > center.y,
            "{label} overlay ({translation}) must sit above the structure centre ({})",
            center.y
        );
    }
}

#[test]
fn overlay_renders_above_gameplay_sprites() {
    // Issue #33: overlay text/background must render
    // above gameplay sprites. The z translation of the
    // overlay's `Transform` is the contract; it must sit
    // strictly above `GAMEPLAY_SPRITE_Z`.
    let mut app = build_app();
    let deposit = common::spawn_deposit(&mut app, Vec2::new(0.0, 0.0), 100);
    let stockpile = common::spawn_stockpile(&mut app, Vec2::new(128.0, 0.0), 50, 200);
    let facility = common::spawn_idle_facility_at(&mut app, Vec2::new(256.0, 0.0));
    let planned = common::spawn_planned_structure_of_kind_at_cell(
        &mut app,
        IVec2::new(4, 0),
        PlannedKind::Charger,
    );
    let charger = common::spawn_charger_at(&mut app, IVec2::new(5, 0), 30);

    app.update();

    let targets = [deposit, stockpile, facility, planned, charger];
    for target in targets {
        let overlay = find_overlay_for(&mut app, target);
        let z = app
            .world()
            .entity(overlay)
            .get::<Transform>()
            .unwrap()
            .translation
            .z;
        assert!(
            z > GAMEPLAY_SPRITE_Z,
            "overlay z ({z}) must sit above gameplay sprite z ({GAMEPLAY_SPRITE_Z}) \
             so the label renders in front of the sprite"
        );
        assert_eq!(
            z, STRUCTURE_OVERLAY_Z,
            "overlay z is pinned to STRUCTURE_OVERLAY_Z"
        );
    }
}

#[test]
fn default_threshold_hides_overlay_at_boundary_and_shows_below() {
    // Issue #33: structure status overlays hide at
    // camera zoom `>= 8.0`, not the legacy `>= 4.0`. The
    // boundary is pinned so a future revert to `4.0`
    // fails this test.
    let mut app = build_app();
    let deposit = common::spawn_deposit(&mut app, Vec2::new(0.0, 0.0), 100);
    let camera = app
        .world_mut()
        .spawn(CameraZoom2d {
            zoom: 7.99,
            ..default()
        })
        .id();

    app.update();
    let overlay = find_overlay_for(&mut app, deposit);
    assert_eq!(
        app.world()
            .entity(overlay)
            .get::<Visibility>()
            .copied()
            .unwrap(),
        Visibility::Inherited,
        "overlay must stay visible just below the threshold"
    );

    // Cross the boundary and confirm the overlay hides.
    app.world_mut()
        .entity_mut(camera)
        .get_mut::<CameraZoom2d>()
        .unwrap()
        .zoom = 8.0;
    app.update();
    assert_eq!(
        app.world()
            .entity(overlay)
            .get::<Visibility>()
            .copied()
            .unwrap(),
        Visibility::Hidden,
        "overlay must hide at the threshold boundary"
    );
}

#[test]
fn configured_threshold_gates_visibility() {
    let mut app = build_app();
    let deposit = common::spawn_deposit(&mut app, Vec2::new(0.0, 0.0), 100);

    // The configured threshold gates visibility, not a
    // baked-in constant. Lowering it to 0.5 hides the
    // overlay at the same zoom, proving the resource is
    // what drives the gate.
    app.world_mut()
        .resource_mut::<StructureOverlaySettings>()
        .hide_zoom_threshold = 0.5;
    app.world_mut().spawn(CameraZoom2d {
        zoom: 1.0,
        ..default()
    });

    app.update();

    let overlay = find_overlay_for(&mut app, deposit);
    assert_eq!(
        app.world()
            .entity(overlay)
            .get::<Visibility>()
            .copied()
            .unwrap(),
        Visibility::Hidden,
        "configured threshold of 0.5 must hide the overlay at zoom 1.0"
    );
}

#[test]
fn overlay_visibility_does_not_touch_unrelated_entities() {
    let mut app = build_app();
    let deposit = common::spawn_deposit(&mut app, Vec2::new(0.0, 0.0), 100);
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
    assert_eq!(
        app.world()
            .entity(overlay)
            .get::<Visibility>()
            .copied()
            .unwrap(),
        Visibility::Hidden
    );
    assert_eq!(
        app.world()
            .entity(unrelated)
            .get::<Visibility>()
            .copied()
            .unwrap(),
        Visibility::Inherited,
        "visibility system must not touch entities without a StructureOverlay"
    );
}

#[test]
fn overlay_is_removed_when_target_despawns() {
    let mut app = build_app();
    let deposit = common::spawn_deposit(&mut app, Vec2::new(0.0, 0.0), 100);

    app.update();
    let overlay = find_overlay_for(&mut app, deposit);
    assert!(app.world().get_entity(overlay).is_ok());

    app.world_mut().despawn(deposit);
    app.update();

    assert!(
        app.world().get_entity(overlay).is_err(),
        "overlay must despawn when its target despawns"
    );
}

#[test]
fn overlay_spawns_once_and_persists_across_state_changes() {
    let mut app = build_app();
    let deposit = common::spawn_deposit(&mut app, Vec2::new(0.0, 0.0), 100);

    app.update();
    let overlay = find_overlay_for(&mut app, deposit);

    for new_amount in [99, 50, 10, 0] {
        {
            let world = app.world_mut();
            let mut e = world.entity_mut(deposit);
            let mut d = *e.get::<ResourceDeposit>().unwrap();
            d.amount = new_amount;
            e.insert(d);
        }
        app.update();
        assert_eq!(text_for(&mut app, overlay), format!("Deposit {new_amount}"));
        assert!(app.world().get_entity(overlay).is_ok());
    }

    // No duplicate overlay on repeated ticks.
    for _ in 0..5 {
        app.update();
    }
    let mut q = app.world_mut().query::<&StructureOverlay>();
    let world = app.world();
    let count = q.iter(world).filter(|o| o.target == deposit).count();
    assert_eq!(count, 1, "exactly one overlay must exist for the target");
}

#[test]
fn overlay_spawns_for_every_kind_at_once() {
    let mut app = build_app();
    app.world_mut().insert_resource(ProductionRatio::default());
    let cell = IVec2::ZERO;
    let deposit = common::spawn_deposit(&mut app, Vec2::new(0.0, 0.0), 100);
    let stockpile = common::spawn_stockpile(&mut app, Vec2::new(64.0, 0.0), 50, 200);
    let facility = common::spawn_idle_facility_at(&mut app, Vec2::new(128.0, 0.0));
    let planned =
        common::spawn_planned_structure_of_kind_at_cell(&mut app, cell, PlannedKind::SinkStockpile);
    let charger = common::spawn_charger_at(&mut app, cell, 30);

    app.update();

    let targets = [deposit, stockpile, facility, planned, charger];
    let mut kinds: Vec<StructureOverlayKind> = Vec::with_capacity(targets.len());
    for &target in &targets {
        let overlay = find_overlay_for(&mut app, target);
        kinds.push(kind_of(&mut app, overlay));
    }
    // Every kind is present and the deposit / stockpile
    // markers are different so a kind-swap bug is
    // caught.
    assert_eq!(
        kinds,
        vec![
            StructureOverlayKind::Deposit,
            StructureOverlayKind::Stockpile,
            StructureOverlayKind::Facility,
            StructureOverlayKind::Planned,
            StructureOverlayKind::Charger,
        ]
    );
}

#[test]
fn overlay_background_colors_are_pairwise_distinct() {
    // The visual contract: each kind gets a distinct
    // colored panel.
    let colors: Vec<_> = StructureOverlayKind::ALL
        .iter()
        .map(|k| overlay_background_color(*k))
        .collect();
    for i in 0..colors.len() {
        for j in (i + 1)..colors.len() {
            assert_ne!(
                colors[i],
                colors[j],
                "kinds {:?} and {:?} must have distinct background colors",
                StructureOverlayKind::ALL[i],
                StructureOverlayKind::ALL[j]
            );
        }
    }
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
        .unwrap_or_else(|| {
            panic!("no StructureOverlay points at target {target:?} after app.update()")
        })
}

fn text_for(app: &mut App, overlay: Entity) -> String {
    app.world()
        .entity(overlay)
        .get::<Text2d>()
        .map(|t| t.0.clone())
        .expect("overlay must carry a Text2d component")
}

fn kind_of(app: &mut App, overlay: Entity) -> StructureOverlayKind {
    app.world()
        .entity(overlay)
        .get::<StructureOverlay>()
        .unwrap()
        .kind
}

// Reference the public API used by gameplay so a
// sweep that drops the exports surfaces as a build
// error rather than a missing-import test failure.
#[allow(dead_code)]
fn _exports() {
    let _: Charger = Charger::new(IVec2::ZERO);
    let _: PlannedStructure = PlannedStructure::new(PlannedKind::Charger, IVec2::ZERO);
    let _: ProductionFacility = ProductionFacility::new();
    let _: StructureOverlayPlugin = StructureOverlayPlugin;
}
