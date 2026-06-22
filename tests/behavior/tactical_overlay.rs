//! Integration tests for issue #36: zoomed-out tactical
//! overlay as semi-transparent clustered icons.

use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    fly_camera::CameraZoom2d,
    nanobot::{Charger, NanobotType, OpponentSwarm, PlannedKind, PlannedStructure, Swarm, SwarmId},
    resources::{ResourceDeposit, Stockpile},
    tactical_overlay::{
        cluster_radius_for_zoom, TacticalClusterKey, TacticalMarker, TacticalMarkerKind,
        TacticalOverlayPlugin, TacticalOverlaySettings, TACTICAL_MARKER_ALPHA, UNOWNED_SWARM_ID,
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

fn for_each_marker(
    app: &mut App,
    mut f: impl FnMut(Entity, &TacticalClusterKey, &TacticalMarker, &Transform, &Sprite),
) {
    let world = app.world_mut();
    let mut q = world.query::<(
        Entity,
        &TacticalClusterKey,
        &TacticalMarker,
        &Transform,
        &Sprite,
    )>();
    for (e, k, m, t, s) in q.iter(world) {
        f(e, k, m, t, s);
    }
}

fn count_markers(app: &mut App) -> usize {
    app.world_mut()
        .query::<&TacticalMarker>()
        .iter(app.world())
        .count()
}

fn body_transforms(app: &mut App) -> Vec<Transform> {
    let world = app.world_mut();
    let mut q = world.query_filtered::<&Transform, With<TacticalMarker>>();
    q.iter(world).copied().collect()
}

fn marker_keys(app: &mut App) -> Vec<TacticalClusterKey> {
    let world = app.world_mut();
    let mut q = world.query::<&TacticalClusterKey>();
    q.iter(world).copied().collect()
}

fn unowned_key(kind: TacticalMarkerKind) -> TacticalClusterKey {
    TacticalClusterKey {
        kind,
        owner: UNOWNED_SWARM_ID,
        slot: (0, 0),
    }
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
    for_each_marker(&mut app, |_, key, _, _, _| {
        kinds.push(key.kind);
    });
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
// Same-kind same-owner separate clusters
// ---------------------------------------------------------------------------

#[test]
fn multiple_separate_deposit_clusters_can_coexist() {
    // Two player deposits 20,000 world units apart at
    // zoom 8.0: they do not merge (max merge radius is
    // 6000), and they land in different spatial slots
    // (slot size is 6000), so the two clusters must
    // both produce a marker entity.
    let mut app = build_app();
    common::spawn_deposit(&mut app, Vec2::new(0.0, 0.0), 1000);
    common::spawn_deposit(&mut app, Vec2::new(20_000.0, 0.0), 1000);

    set_zoom(&mut app, 8.0);
    app.update();

    // The two deposits also produce no player swarm, so
    // we expect exactly two markers (one per deposit
    // cluster).
    assert_eq!(
        count_markers(&mut app),
        2,
        "two same-kind same-owner clusters far apart must coexist as two markers"
    );

    // Both keys must share (Deposit, UNOWNED) and
    // differ in slot.
    let mut slots: Vec<(i32, i32)> = marker_keys(&mut app)
        .into_iter()
        .filter(|k| k.kind == TacticalMarkerKind::Deposit)
        .map(|k| k.slot)
        .collect();
    slots.sort();
    slots.dedup();
    assert_eq!(
        slots.len(),
        2,
        "the two deposit clusters must have distinct spatial slots"
    );
    assert_eq!(slots[0], (0, 0));
    assert_eq!(slots[1], (3, 0));
}

#[test]
fn close_same_kind_deposits_still_merge_into_one_cluster() {
    // The spatial-slot keying must not break the merge
    // step: two deposits well within the merge radius
    // must still collapse to one cluster / one marker.
    let mut app = build_app();
    common::spawn_deposit(&mut app, Vec2::new(0.0, 0.0), 1000);
    common::spawn_deposit(&mut app, Vec2::new(200.0, 0.0), 1000);

    set_zoom(&mut app, 4.0);
    app.update();

    assert_eq!(count_markers(&mut app), 1);
}

#[test]
fn marker_key_uses_spatial_slot_not_just_kind_and_owner() {
    // The acceptance criterion "clusters are not keyed
    // only by kind/owner" is the contract under test
    // here: a fresh marker spawned by the system
    // carries a non-origin slot for a cluster whose
    // world position is far from the origin.
    let mut app = build_app();
    // 20,000 world units east => slot (3, 0) with the
    // default 6,000-unit slot size.
    common::spawn_deposit(&mut app, Vec2::new(20_000.0, 0.0), 1000);
    set_zoom(&mut app, 8.0);
    app.update();

    let slot = marker_keys(&mut app)
        .into_iter()
        .find(|k| k.kind == TacticalMarkerKind::Deposit)
        .map(|k| k.slot)
        .expect("deposit marker must be spawned");
    assert_eq!(slot, (3, 0));
}

// ---------------------------------------------------------------------------
// Cluster merging by zoom
// ---------------------------------------------------------------------------

#[test]
fn nearby_deposits_merge_into_single_marker() {
    let mut app = build_app();
    // Two deposits 200 world units apart with a moderate
    // zoom (4.0) and a moderate merge radius (2048).
    // They must collapse into a single marker whose
    // body sits at the cluster centroid (100, 0).
    common::spawn_deposit(&mut app, Vec2::new(0.0, 0.0), 1000);
    common::spawn_deposit(&mut app, Vec2::new(200.0, 0.0), 1000);
    set_zoom(&mut app, 4.0);
    app.update();
    let transforms = body_transforms(&mut app);
    assert_eq!(transforms.len(), 1);
    assert!(
        (transforms[0].translation.x - 100.0).abs() < 1e-3,
        "merged cluster centroid must be (100, 0); got {:?}",
        transforms[0].translation
    );
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
    let transforms = body_transforms(&mut app);
    assert_eq!(transforms.len(), 1);
    // Centroid of (0, 0) and (4_000, 0) is (2_000, 0).
    assert!(
        (transforms[0].translation.x - 2_000.0).abs() < 1e-3,
        "far-zoom merged cluster centroid must be (2_000, 0); got {:?}",
        transforms[0].translation
    );
}

// ---------------------------------------------------------------------------
// Screen-constant size and 50% alpha (issue #36 acceptance #2 + #3)
// ---------------------------------------------------------------------------

#[test]
fn marker_world_scale_shrinks_as_zoom_grows() {
    let mut app = build_app();
    common::spawn_deposit(&mut app, Vec2::new(0.0, 0.0), 1000);
    set_zoom(&mut app, 4.0);
    app.update();
    let small_scale = body_transforms(&mut app)
        .first()
        .map(|t| t.scale.x)
        .expect("marker body must exist at zoom 4.0");

    set_zoom(&mut app, 16.0);
    app.update();
    let big_zoom_scale = body_transforms(&mut app)
        .first()
        .map(|t| t.scale.x)
        .expect("marker body must exist at zoom 16.0");

    assert!(
        big_zoom_scale < small_scale,
        "marker world scale at zoom 16 ({big_zoom_scale}) must be smaller than at zoom 4 ({small_scale})"
    );
    // The transform scale matches
    // `marker_screen_size / zoom` so the on-screen
    // footprint is `scale * custom_size * zoom =
    // 32` pixels (constant).
    assert!((small_scale - 8.0).abs() < 1e-3, "zoom 4 -> 8.0");
    assert!(
        (big_zoom_scale - 2.0).abs() < 1e-3,
        "zoom 16 -> 2.0, got {big_zoom_scale}"
    );
}

/// On-screen pixel footprint of the first marker body:
/// `custom_size.x * transform.scale.x * zoom`. The body
/// is a unit-rectangle sprite scaled by
/// `marker_screen_size / zoom`, so the result equals
/// `marker_screen_size` regardless of zoom.
fn marker_on_screen_size(app: &mut App) -> f32 {
    let world = app.world_mut();
    let zoom = world
        .query::<&CameraZoom2d>()
        .iter(world)
        .next()
        .map(|z| z.zoom)
        .expect("camera must be present");
    let mut q = world.query::<(&Transform, &Sprite)>();
    let (transform, sprite) = q.iter(world).next().expect("marker must exist");
    let custom = sprite
        .custom_size
        .expect("marker sprite must have explicit custom_size");
    custom.x * transform.scale.x * zoom
}

#[test]
fn marker_on_screen_size_is_constant_across_zoom() {
    // Issue #36 acceptance #3: the on-screen pixel
    // size of an icon stays constant across zoom. The
    // test reads the camera's zoom through
    // `CameraZoom2d` to compute the on-screen
    // footprint at three zooms and asserts they all
    // equal `marker_screen_size` (32 pixels).
    let mut app = build_app();
    common::spawn_deposit(&mut app, Vec2::new(0.0, 0.0), 1000);
    set_zoom(&mut app, 8.0);
    app.update();
    let size_at_8 = marker_on_screen_size(&mut app);
    set_zoom(&mut app, 16.0);
    app.update();
    let size_at_16 = marker_on_screen_size(&mut app);
    set_zoom(&mut app, 32.0);
    app.update();
    let size_at_32 = marker_on_screen_size(&mut app);
    let expected = 32.0;
    assert!(
        (size_at_8 - expected).abs() < 1e-3,
        "on-screen size at zoom 8 must equal marker_screen_size; got {size_at_8}"
    );
    assert!(
        (size_at_16 - expected).abs() < 1e-3,
        "on-screen size at zoom 16 must equal marker_screen_size; got {size_at_16}"
    );
    assert!(
        (size_at_32 - expected).abs() < 1e-3,
        "on-screen size at zoom 32 must equal marker_screen_size; got {size_at_32}"
    );
}

#[test]
fn marker_sprite_is_50_percent_alpha() {
    // Issue #36 acceptance #2: every tactical marker
    // body is a semi-transparent icon. The sprite
    // color's alpha must be 50%.
    let mut app = build_app();
    common::spawn_swarm_at(&mut app, Vec2::new(0.0, 0.0));
    common::spawn_deposit(&mut app, Vec2::new(10_000.0, 0.0), 1000);
    let facility_owner = common::spawn_swarm_at(&mut app, Vec2::new(-10_000.0, 0.0));
    common::spawn_facility_at(&mut app, facility_owner, Vec2::new(-10_000.0, 0.0));
    set_zoom(&mut app, 8.0);
    app.update();

    let mut alphas: Vec<f32> = Vec::new();
    for_each_marker(&mut app, |_, _, _, _, sprite| {
        let srgba = sprite.color.to_srgba();
        alphas.push(srgba.alpha);
    });
    assert!(!alphas.is_empty(), "markers must be spawned");
    for alpha in alphas {
        assert!(
            (alpha - TACTICAL_MARKER_ALPHA).abs() < 1e-4,
            "marker alpha must be 50%, got {alpha}"
        );
    }
}

#[test]
fn marker_has_no_text_label_child() {
    // Issue #36 acceptance #2: tactical markers are
    // icons, not text. A spawned marker must have no
    // `Text2d` child and no `Text2d` component on the
    // marker entity itself.
    let mut app = build_app();
    common::spawn_deposit(&mut app, Vec2::new(0.0, 0.0), 1000);
    set_zoom(&mut app, 8.0);
    app.update();

    let world = app.world_mut();
    let mut q = world.query::<(Entity, &TacticalMarker)>();
    let marker_entity = q
        .iter(world)
        .map(|(e, _)| e)
        .next()
        .expect("marker must be spawned");
    assert!(
        world.entity(marker_entity).get::<Text2d>().is_none(),
        "marker entity must not carry a Text2d component"
    );
    let children: Vec<Entity> = world
        .entity(marker_entity)
        .get::<Children>()
        .map(|c| c.to_vec())
        .unwrap_or_default();
    assert!(
        children.is_empty(),
        "marker entity must have no children (no text label child)"
    );
    for child in children {
        assert!(
            world.entity(child).get::<Text2d>().is_none(),
            "marker must not have any Text2d child entity"
        );
    }
}

#[test]
fn marker_sprite_has_unit_custom_size() {
    // The body is a unit-rectangle sprite scaled by
    // `marker_world_size`. The unit rectangle is the
    // shape the transform scale multiplies to produce
    // the on-screen footprint.
    let mut app = build_app();
    common::spawn_deposit(&mut app, Vec2::new(0.0, 0.0), 1000);
    set_zoom(&mut app, 8.0);
    app.update();

    for_each_marker(&mut app, |_, _, _, _, sprite| {
        let custom = sprite
            .custom_size
            .expect("marker sprite must have explicit custom_size");
        assert!(
            (custom.x - 1.0).abs() < 1e-4 && (custom.y - 1.0).abs() < 1e-4,
            "marker sprite must be a unit rectangle, got {custom:?}"
        );
    });
}

// ---------------------------------------------------------------------------
// Center of mass
// ---------------------------------------------------------------------------

#[test]
fn marker_position_tracks_cluster_centroid() {
    let mut app = build_app();
    common::spawn_deposit(&mut app, Vec2::new(0.0, 0.0), 1000);
    common::spawn_deposit(&mut app, Vec2::new(200.0, 0.0), 1000);
    set_zoom(&mut app, 4.0);
    app.update();
    let transform = body_transforms(&mut app)
        .into_iter()
        .next()
        .expect("cluster marker must exist");
    assert!(
        (transform.translation.x - 100.0).abs() < 1.0,
        "cluster centroid must be (100, 0), got {:?}",
        transform.translation
    );
}

// ---------------------------------------------------------------------------
// De-overlap (issue #36 acceptance #7)
// ---------------------------------------------------------------------------

#[test]
fn deoverlap_pushes_co_located_clusters_apart() {
    // Two clusters of different kinds at the origin
    // would overlap on screen at zoom 8.0 (icon size
    // 32 pixels, so non-overlap world distance is
    // 4.0). The de-overlap pass must push them apart
    // to at least 4.0 world units.
    let mut app = build_app();
    common::spawn_deposit(&mut app, Vec2::new(0.0, 0.0), 1000);
    common::spawn_idle_facility_at(&mut app, Vec2::new(0.0, 0.0));
    set_zoom(&mut app, 8.0);
    app.update();

    let transforms = body_transforms(&mut app);
    assert_eq!(transforms.len(), 2);
    let dist = (transforms[0].translation - transforms[1].translation).length();
    assert!(
        dist >= 4.0 - 1e-3,
        "two co-located clusters must be de-overlapped to at least the min world distance (4.0); got {dist}"
    );
}

#[test]
fn deoverlap_leaves_far_apart_clusters_in_place() {
    // Two clusters 1,000 world units apart at zoom 8.0
    // are well above the min world distance of 4.0.
    // The de-overlap pass must not move them.
    let mut app = build_app();
    common::spawn_deposit(&mut app, Vec2::new(0.0, 0.0), 1000);
    common::spawn_idle_facility_at(&mut app, Vec2::new(1000.0, 0.0));
    set_zoom(&mut app, 8.0);
    app.update();

    let mut sorted_xs: Vec<f32> = body_transforms(&mut app)
        .iter()
        .map(|t| t.translation.x)
        .collect();
    sorted_xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
    assert!(
        (sorted_xs[0] - 0.0).abs() < 1e-3,
        "deposit at origin must stay near origin; got x={}",
        sorted_xs[0]
    );
    assert!(
        (sorted_xs[1] - 1000.0).abs() < 1e-3,
        "facility at (1000, 0) must stay near (1000, 0); got x={}",
        sorted_xs[1]
    );
}

#[test]
fn deoverlap_separates_three_co_located_clusters() {
    // Three clusters of three different kinds at the
    // origin must be spread apart by the de-overlap
    // pass: no two of them may be within the min world
    // distance after `app.update()`.
    let mut app = build_app();
    common::spawn_deposit(&mut app, Vec2::new(0.0, 0.0), 1000);
    common::spawn_idle_facility_at(&mut app, Vec2::new(0.0, 0.0));
    common::spawn_charger_at(&mut app, IVec2::new(0, 0), 0);
    set_zoom(&mut app, 8.0);
    app.update();

    let transforms = body_transforms(&mut app);
    assert_eq!(transforms.len(), 3);
    let min_dist = 32.0 / 8.0;
    for i in 0..transforms.len() {
        for j in (i + 1)..transforms.len() {
            let d = (transforms[i].translation - transforms[j].translation).length();
            assert!(
                d >= min_dist - 1e-3,
                "pair ({i}, {j}) still overlaps after de-overlap: distance {d} < {min_dist}"
            );
        }
    }
}

#[test]
fn deoverlap_preserves_center_of_mass() {
    // A pair that gets de-overlapped moves symmetrically
    // around the original midpoint. (Each icon moves by
    // half the overlap, so the midpoint is unchanged.)
    let mut app = build_app();
    common::spawn_deposit(&mut app, Vec2::new(0.0, 0.0), 1000);
    common::spawn_idle_facility_at(&mut app, Vec2::new(0.0, 0.0));
    set_zoom(&mut app, 8.0);
    app.update();

    let transforms = body_transforms(&mut app);
    let midpoint = ((transforms[0].translation + transforms[1].translation) * 0.5).truncate();
    assert!(
        midpoint.length() < 1e-3,
        "pair centroid after de-overlap must stay at the origin; got {midpoint:?}"
    );
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

#[test]
fn multiple_clusters_survive_across_ticks() {
    // After the spatial-slot keying, two separate
    // deposit clusters must both survive a re-tick at
    // the same zoom (the existing entity per cluster
    // gets patched, neither is despawned).
    let mut app = build_app();
    common::spawn_deposit(&mut app, Vec2::new(0.0, 0.0), 1000);
    common::spawn_deposit(&mut app, Vec2::new(20_000.0, 0.0), 1000);
    set_zoom(&mut app, 8.0);
    app.update();
    let n0 = count_markers(&mut app);
    app.update();
    app.update();
    let n1 = count_markers(&mut app);
    assert_eq!(n0, 2);
    assert_eq!(
        n1, 2,
        "two separate clusters must both survive across ticks"
    );
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
    let _: TacticalMarker = TacticalMarker;
    let _: TacticalClusterKey = unowned_key(TacticalMarkerKind::Deposit);
    let _: PlannedStructure = PlannedStructure::new(PlannedKind::Charger, IVec2::ZERO);
    let _: Charger = Charger::new(IVec2::ZERO);
    let _: NanobotType = NanobotType::Worker;
    let _: ResourceDeposit = ResourceDeposit::default();
    let _: Stockpile = Stockpile::default();
}
