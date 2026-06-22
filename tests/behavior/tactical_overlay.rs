//! Integration tests for issue #31: zoomed-out tactical
//! overlay with clustering.

use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    fly_camera::CameraZoom2d,
    nanobot::{Charger, NanobotType, OpponentSwarm, PlannedKind, PlannedStructure, Swarm, SwarmId},
    resources::{ResourceDeposit, Stockpile},
    tactical_overlay::{
        cluster_radius_for_zoom, TacticalClusterKey, TacticalMarker, TacticalMarkerKind,
        TacticalOverlayPlugin, TacticalOverlaySettings,
    },
};

#[path = "../common/mod.rs"]
mod common;

fn build_app() -> App {
    common::sim_app_with_tactical()
}

fn set_zoom(app: &mut App, zoom: f32) {
    // Drop any existing camera so the visibility /
    // clustering systems see exactly one zoom value.
    let mut to_despawn: Vec<Entity> = Vec::new();
    {
        let world = app.world_mut();
        let mut q = world.query::<(Entity, &CameraZoom2d)>();
        for (e, _) in q.iter(world) {
            to_despawn.push(e);
        }
    }
    for e in to_despawn {
        app.world_mut().despawn(e);
    }
    app.world_mut().spawn(CameraZoom2d { zoom, ..default() });
}

fn markers_with_key(
    app: &mut App,
    key: TacticalClusterKey,
) -> Vec<(Entity, TacticalMarker, Transform, String)> {
    let world = app.world_mut();
    let mut q = world.query::<(Entity, &TacticalClusterKey, &TacticalMarker, &Transform)>();
    let mut hits: Vec<(Entity, TacticalMarker, Transform, String)> = Vec::new();
    for (e, k, m, t) in q.iter(world) {
        if *k == key {
            let label = world
                .get::<Children>(e)
                .and_then(|c| {
                    c.iter()
                        .find_map(|child| world.get::<Text2d>(child).map(|txt| txt.0.clone()))
                })
                .unwrap_or_default();
            hits.push((e, *m, *t, label));
        }
    }
    hits
}

fn count_markers(app: &mut App) -> usize {
    app.world_mut()
        .query::<&TacticalMarker>()
        .iter(app.world())
        .count()
}

fn body_transform(app: &mut App) -> Option<Transform> {
    let world = app.world_mut();
    let mut q = world.query_filtered::<&Transform, With<TacticalMarker>>();
    q.iter(world).next().copied()
}

// ---------------------------------------------------------------------------
// Visibility gating by zoom
// ---------------------------------------------------------------------------

#[test]
fn overlay_marks_sources_below_threshold_but_stays_hidden() {
    let mut app = build_app();
    common::spawn_swarm_at(&mut app, Vec2::new(0.0, 0.0));
    set_zoom(&mut app, 1.0);
    app.update();

    // The marker exists (the swarm still produces a
    // base cluster) but is hidden at the default play
    // zoom because the threshold check fails.
    assert_eq!(count_markers(&mut app), 1);
    let visibility = app
        .world_mut()
        .query::<&Visibility>()
        .iter(app.world())
        .next()
        .copied();
    assert_eq!(visibility, Some(Visibility::Hidden));
}

#[test]
fn overlay_spawns_a_marker_when_zoom_passes_threshold() {
    let mut app = build_app();
    common::spawn_swarm_at(&mut app, Vec2::new(0.0, 0.0));
    set_zoom(&mut app, 5.0);
    app.update();

    assert_eq!(count_markers(&mut app), 1);
}

#[test]
fn visibility_system_toggles_marker_at_threshold() {
    let mut app = build_app();
    common::spawn_swarm_at(&mut app, Vec2::new(0.0, 0.0));

    set_zoom(&mut app, 7.99);
    app.update();
    let marker = app
        .world_mut()
        .query::<(Entity, &TacticalMarker)>()
        .iter(app.world())
        .map(|(e, _)| e)
        .next()
        .expect("cluster system must spawn the marker regardless of visibility");
    assert_eq!(
        app.world().entity(marker).get::<Visibility>().copied(),
        Some(Visibility::Hidden),
        "just below the threshold the tactical overlay must stay hidden"
    );

    set_zoom(&mut app, 8.0);
    app.update();
    assert_eq!(
        app.world().entity(marker).get::<Visibility>().copied(),
        Some(Visibility::Inherited),
        "at the threshold the overlay flips to visible"
    );
}

// ---------------------------------------------------------------------------
// Marker bodies per category
// ---------------------------------------------------------------------------

#[test]
fn one_marker_per_category_per_owner() {
    let mut app = build_app();
    let _player_swarm = common::spawn_swarm_at(&mut app, Vec2::new(0.0, 0.0));
    let opponent_pos = Vec2::new(10_000.0, 0.0);
    let opponent = app
        .world_mut()
        .spawn((
            Swarm {},
            OpponentSwarm {},
            SwarmId(7),
            Transform::from_translation(opponent_pos.extend(0.0)),
        ))
        .id();
    let _deposit = common::spawn_deposit(&mut app, Vec2::new(0.0, 0.0), 1000);
    let _stockpile = common::spawn_stockpile(&mut app, Vec2::new(200.0, 0.0), 50, 200);
    let _facility = common::spawn_idle_facility_at(&mut app, Vec2::new(400.0, 0.0));
    let _planned = common::spawn_planned_structure_of_kind_at_cell(
        &mut app,
        IVec2::new(1, 0),
        PlannedKind::SinkStockpile,
    );
    let _charger = common::spawn_charger_at(&mut app, IVec2::new(2, 0), 30);

    set_zoom(&mut app, 5.0);
    app.update();

    // Two swarms -> two base markers. Each landmark is
    // far from the others in world space, so no merging
    // happens.
    let mut kinds: Vec<TacticalMarkerKind> = Vec::new();
    {
        let world = app.world_mut();
        let mut q = world.query::<&TacticalClusterKey>();
        for key in q.iter(world) {
            kinds.push(key.kind);
        }
    }
    kinds.sort_by_key(|k| *k as u32);
    assert_eq!(
        kinds,
        vec![
            TacticalMarkerKind::PlayerBase,
            TacticalMarkerKind::OpponentBase,
            TacticalMarkerKind::Deposit,
            TacticalMarkerKind::Facility,
            TacticalMarkerKind::Stockpile,
            TacticalMarkerKind::Planned,
            TacticalMarkerKind::Charger,
        ]
    );
    // Sanity: opponent swarm entity still exists, the
    // marker doesn't shadow it.
    assert!(app.world().get_entity(opponent).is_ok());
}

// ---------------------------------------------------------------------------
// Cluster merging by zoom
// ---------------------------------------------------------------------------

#[test]
fn nearby_deposits_merge_into_single_marker() {
    let mut app = build_app();
    // Two deposits 200 world units apart with a moderate
    // zoom (4.0) and a moderate merge radius (2048).
    // They must collapse.
    common::spawn_deposit(&mut app, Vec2::new(0.0, 0.0), 1000);
    common::spawn_deposit(&mut app, Vec2::new(200.0, 0.0), 1000);
    set_zoom(&mut app, 4.0);
    app.update();
    assert_eq!(count_markers(&mut app), 1);
    let key = TacticalClusterKey {
        kind: TacticalMarkerKind::Deposit,
        owner: top_down_2d_rts_prototype_nano_swarm::nanobot::SwarmId(u32::MAX),
    };
    let hits = markers_with_key(&mut app, key);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].1.count, 2);
    assert_eq!(hits[0].3, "Deposit x2");
}

#[test]
fn far_apart_deposits_stay_separate_at_moderate_zoom() {
    let mut app = build_app();
    // Two deposits 5000 world units apart with a moderate
    // merge radius (2048). They must stay separate.
    common::spawn_deposit(&mut app, Vec2::new(0.0, 0.0), 1000);
    common::spawn_deposit(&mut app, Vec2::new(5_000.0, 0.0), 1000);
    set_zoom(&mut app, 4.0);
    app.update();
    assert_eq!(count_markers(&mut app), 2);
}

#[test]
fn merge_radius_grows_with_zoom_to_collapse_far_landmarks() {
    let mut app = build_app();
    common::spawn_deposit(&mut app, Vec2::new(0.0, 0.0), 1000);
    common::spawn_deposit(&mut app, Vec2::new(5_000.0, 0.0), 1000);

    // At zoom 4.0 the two deposits stay separate.
    set_zoom(&mut app, 4.0);
    app.update();
    assert_eq!(count_markers(&mut app), 2);

    // Bumping zoom up to the far threshold grows the
    // merge radius to 6000 world units, so the two
    // deposits collapse.
    set_zoom(&mut app, 16.0);
    app.update();
    assert_eq!(
        count_markers(&mut app),
        1,
        "very far zoom must collapse nearby same-kind markers"
    );
}

#[test]
fn far_apart_deposits_collapse_at_far_zoom() {
    let mut app = build_app();
    common::spawn_deposit(&mut app, Vec2::new(0.0, 0.0), 1000);
    common::spawn_deposit(&mut app, Vec2::new(4_000.0, 0.0), 1000);
    set_zoom(&mut app, 16.0);
    app.update();
    assert_eq!(count_markers(&mut app), 1);
    let key = TacticalClusterKey {
        kind: TacticalMarkerKind::Deposit,
        owner: top_down_2d_rts_prototype_nano_swarm::nanobot::SwarmId(u32::MAX),
    };
    let hits = markers_with_key(&mut app, key);
    assert_eq!(hits[0].1.count, 2);
}

// ---------------------------------------------------------------------------
// Screen-constant size
// ---------------------------------------------------------------------------

#[test]
fn marker_world_scale_shrinks_as_zoom_grows() {
    let mut app = build_app();
    common::spawn_deposit(&mut app, Vec2::new(0.0, 0.0), 1000);
    set_zoom(&mut app, 4.0);
    app.update();
    let small_scale = body_transform(&mut app)
        .map(|t| t.scale.x)
        .expect("marker body must exist at zoom 4.0");

    set_zoom(&mut app, 16.0);
    app.update();
    let big_zoom_scale = body_transform(&mut app)
        .map(|t| t.scale.x)
        .expect("marker body must exist at zoom 16.0");

    assert!(
        big_zoom_scale < small_scale,
        "marker world scale at zoom 16 ({big_zoom_scale}) must be smaller than at zoom 4 ({small_scale})"
    );
    // On-screen pixel size = world_scale * zoom. At
    // zoom 4 the screen size is 32 / 4 = 8. At zoom
    // 16 it must be 32 / 16 = 2.
    assert!((small_scale - 8.0).abs() < 1e-3, "zoom 4 -> 8.0");
    assert!(
        (big_zoom_scale - 2.0).abs() < 1e-3,
        "zoom 16 -> 2.0, got {big_zoom_scale}"
    );
}

#[test]
fn marker_position_tracks_cluster_centroid() {
    let mut app = build_app();
    common::spawn_deposit(&mut app, Vec2::new(0.0, 0.0), 1000);
    common::spawn_deposit(&mut app, Vec2::new(200.0, 0.0), 1000);
    set_zoom(&mut app, 4.0);
    app.update();
    let transform = body_transform(&mut app).expect("cluster marker must exist");
    assert!(
        (transform.translation.x - 100.0).abs() < 1.0,
        "cluster centroid must be (100, 0), got {:?}",
        transform.translation
    );
}

// ---------------------------------------------------------------------------
// Label updates on count change
// ---------------------------------------------------------------------------

#[test]
fn cluster_label_includes_count_when_several_merge() {
    let mut app = build_app();
    common::spawn_deposit(&mut app, Vec2::new(0.0, 0.0), 1000);
    common::spawn_deposit(&mut app, Vec2::new(100.0, 0.0), 1000);
    common::spawn_deposit(&mut app, Vec2::new(200.0, 0.0), 1000);
    set_zoom(&mut app, 4.0);
    app.update();
    let key = TacticalClusterKey {
        kind: TacticalMarkerKind::Deposit,
        owner: top_down_2d_rts_prototype_nano_swarm::nanobot::SwarmId(u32::MAX),
    };
    let hits = markers_with_key(&mut app, key);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].1.count, 3);
    assert_eq!(hits[0].3, "Deposit x3");
}

#[test]
fn base_marker_label_is_you_for_player() {
    let mut app = build_app();
    common::spawn_swarm_at(&mut app, Vec2::new(0.0, 0.0));
    set_zoom(&mut app, 5.0);
    app.update();
    let key = TacticalClusterKey {
        kind: TacticalMarkerKind::PlayerBase,
        owner: SwarmId::PLAYER,
    };
    let hits = markers_with_key(&mut app, key);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].3, "You");
}

#[test]
fn base_marker_label_is_enemy_for_opponent() {
    let mut app = build_app();
    app.world_mut().spawn((
        Swarm {},
        OpponentSwarm {},
        SwarmId(7),
        Transform::from_translation(Vec2::new(0.0, 0.0).extend(0.0)),
    ));
    set_zoom(&mut app, 5.0);
    app.update();
    let key = TacticalClusterKey {
        kind: TacticalMarkerKind::OpponentBase,
        owner: SwarmId(7),
    };
    let hits = markers_with_key(&mut app, key);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].3, "Enemy");
}

// ---------------------------------------------------------------------------
// Stale marker cleanup when sources disappear
// ---------------------------------------------------------------------------

#[test]
fn stale_marker_is_despawned_when_source_disappears() {
    let mut app = build_app();
    let deposit = common::spawn_deposit(&mut app, Vec2::new(0.0, 0.0), 1000);
    set_zoom(&mut app, 5.0);
    app.update();
    assert_eq!(count_markers(&mut app), 1);
    let marker_exists = app
        .world_mut()
        .query::<&TacticalMarker>()
        .iter(app.world())
        .next()
        .is_some();
    assert!(marker_exists, "marker must exist before source despawn");

    app.world_mut().despawn(deposit);
    app.update();
    assert_eq!(
        count_markers(&mut app),
        0,
        "marker must be despawned once its source is gone"
    );
}

#[test]
fn marker_count_is_stable_across_ticks() {
    let mut app = build_app();
    common::spawn_deposit(&mut app, Vec2::new(0.0, 0.0), 1000);
    set_zoom(&mut app, 5.0);
    app.update();
    let n0 = count_markers(&mut app);
    app.update();
    app.update();
    app.update();
    assert_eq!(count_markers(&mut app), n0, "marker count must be stable");
    assert_eq!(n0, 1);
}

// ---------------------------------------------------------------------------
// Pure helper reachable from integration tests
// ---------------------------------------------------------------------------

#[test]
fn cluster_radius_for_zoom_helper_is_reachable() {
    // This is a smoke test: the integration crate
    // consumes the helper indirectly through the
    // plugin. Pin the helper signature here so a sweep
    // that drops the export surfaces as a build error
    // rather than a silent behavior change.
    let s = TacticalOverlaySettings::default();
    let r = cluster_radius_for_zoom(s.show_zoom_threshold, &s);
    assert!(r > 0.0);
}

// Reference public types so a refactor that drops a
// re-export surfaces as a compile error here rather than
// a missing-import failure in an unrelated test.
#[allow(dead_code)]
fn _exports() {
    let _: TacticalOverlayPlugin = TacticalOverlayPlugin;
    let _: TacticalMarker = TacticalMarker { count: 1 };
    let _: TacticalClusterKey = TacticalClusterKey {
        kind: TacticalMarkerKind::Deposit,
        owner: SwarmId(1),
    };
    let _: PlannedStructure = PlannedStructure::new(PlannedKind::Charger, IVec2::ZERO);
    let _: Charger = Charger::new(IVec2::ZERO);
    let _: NanobotType = NanobotType::Worker;
    let _: ResourceDeposit = ResourceDeposit::default();
    let _: Stockpile = Stockpile::default();
}
