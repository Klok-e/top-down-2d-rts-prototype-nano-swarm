//! Zoomed-out tactical overlay (issue #36).
//!
//! When the camera zooms past the structure overlay's hide
//! threshold the always-visible status labels disappear.
//! The tactical overlay takes their place: semi-transparent
//! icon markers for the player base, opponent base,
//! deposits, facilities, stockpiles, planned structures,
//! and chargers, with progressive merging as the player
//! zooms farther out.
//!
//! Markers stay screen-constant by setting the body's
//! `Transform::scale` to `screen_size / zoom` against a
//! unit-rectangle sprite, so the on-screen footprint
//! stays constant regardless of the orthographic
//! projection's scale. The `cluster_tactical_markers`
//! algorithm stamps every cluster with a spatial slot
//! (see [`CLUSTER_SPATIAL_SLOT_SIZE`]) so two same-kind
//! same-owner clusters that survive the merge pass keep
//! distinct marker entities. A de-overlap pass nudges
//! cluster positions apart in screen space so visible
//! icons do not overlap.

use bevy::prelude::*;
use std::collections::{HashMap, HashSet};

use crate::fly_camera::CameraZoom2d;
use crate::nanobot::{
    Charger, OpponentSwarm, PlannedStructure, ProductionFacility, Swarm, SwarmId,
};
use crate::resources::{ResourceDeposit, Stockpile};

/// Camera zoom at or above which the tactical overlay
/// appears. Matches
/// [`crate::structure_overlay::DEFAULT_OVERLAY_HIDE_ZOOM_THRESHOLD`]
/// so the two layers fade at the same boundary.
pub const DEFAULT_TACTICAL_SHOW_ZOOM_THRESHOLD: f32 = 8.0;

/// World-units radius at which two same-key markers
/// collapse into a single cluster at moderate zoom.
pub const DEFAULT_TACTICAL_MERGE_RADIUS_WORLD: f32 = 2048.0;

/// World-units radius used at the far end of the camera's
/// zoom range.
pub const DEFAULT_TACTICAL_FAR_MERGE_RADIUS_WORLD: f32 = 6000.0;

/// Camera zoom at which the merge radius reaches the far
/// value. The radius scales linearly between
/// [`DEFAULT_TACTICAL_MERGE_RADIUS_WORLD`] and the far
/// value across
/// `[show_zoom_threshold, far_merge_zoom]`.
pub const DEFAULT_TACTICAL_FAR_MERGE_ZOOM: f32 = 16.0;

/// On-screen pixel size for the marker body.
pub const DEFAULT_TACTICAL_MARKER_SCREEN_SIZE: f32 = 32.0;

/// Alpha (0..=1) of every tactical marker body. Issue
/// #36 commits the overlay to a semi-transparent icon
/// look so the player can still see the underlying map
/// through the markers at far zoom.
pub const TACTICAL_MARKER_ALPHA: f32 = 0.5;

/// Z-translation for the marker body sprites. Sits between
/// the zone overlay and the gameplay sprites so a marker
/// never eclipses a real structure.
pub const TACTICAL_MARKER_Z: f32 = 0.5;

/// Spatial slot size used to differentiate same-key
/// clusters. Two clusters whose positions fall in
/// different slots keep distinct marker entities even
/// when they share `(kind, owner)`. The default sits at
/// the maximum merge radius so any two clusters that
/// survive the merge pass (i.e. are more than
/// [`DEFAULT_TACTICAL_FAR_MERGE_RADIUS_WORLD`] apart in
/// world space) also land in different slots.
pub const CLUSTER_SPATIAL_SLOT_SIZE: f32 = DEFAULT_TACTICAL_FAR_MERGE_RADIUS_WORLD;

/// Number of relaxation iterations the de-overlap pass
/// runs. Eight passes is enough to spread a dozen
/// co-located clusters apart without burning more time
/// per tick.
pub const DEOVERLAP_ITERATIONS: u32 = 8;

/// Synthetic owner id stamped on every landmark source
/// (deposits, facilities, stockpiles, planned structures,
/// chargers) so unowned landmarks never collide with a
/// real [`SwarmId`].
pub const UNOWNED_SWARM_ID: SwarmId = SwarmId(u32::MAX);

/// Configuration for the tactical overlay layer. Inserted
/// as a Bevy [`Resource`] so it can be mutated by the
/// player or by tests. Setting `show_zoom_threshold` to
/// `0.0` keeps the overlay always visible; setting it to
/// `f32::INFINITY` hides it for every zoom.
#[derive(Debug, Resource, Clone, Copy, PartialEq)]
pub struct TacticalOverlaySettings {
    pub show_zoom_threshold: f32,
    pub merge_radius_world: f32,
    pub far_merge_radius_world: f32,
    pub far_merge_zoom: f32,
    pub marker_screen_size: f32,
}

impl Default for TacticalOverlaySettings {
    fn default() -> Self {
        Self {
            show_zoom_threshold: DEFAULT_TACTICAL_SHOW_ZOOM_THRESHOLD,
            merge_radius_world: DEFAULT_TACTICAL_MERGE_RADIUS_WORLD,
            far_merge_radius_world: DEFAULT_TACTICAL_FAR_MERGE_RADIUS_WORLD,
            far_merge_zoom: DEFAULT_TACTICAL_FAR_MERGE_ZOOM,
            marker_screen_size: DEFAULT_TACTICAL_MARKER_SCREEN_SIZE,
        }
    }
}

/// Marker category. The cluster key is
/// `(kind, owner_swarm_id, slot)` so two deposits
/// belonging to different swarms stay separate, two
/// player-owned facilities next to each other collapse,
/// and two player-owned deposits in different parts of
/// the map keep distinct markers.
#[derive(Debug, Component, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TacticalMarkerKind {
    /// A player's home base cluster. Spawned once per
    /// player swarm.
    PlayerBase,
    /// An opponent's home base cluster. Spawned once per
    /// opponent swarm.
    OpponentBase,
    /// A resource deposit.
    Deposit,
    /// A production facility.
    Facility,
    /// A stockpile. Source and sink stockpiles share the
    /// marker kind.
    Stockpile,
    /// A planned structure.
    Planned,
    /// A charger.
    Charger,
}

/// The cluster key for a marker. Two markers with the same
/// `(kind, owner, slot)` and a world distance below the
/// current merge radius collapse to a single cluster.
/// The spatial `slot` (see [`cluster_spatial_slot`]) lets
/// two same-kind same-owner clusters coexist when the
/// player zooms out far enough to keep them apart.
#[derive(Debug, Component, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TacticalClusterKey {
    pub kind: TacticalMarkerKind,
    pub owner: SwarmId,
    /// Quantized world-space slot of the cluster. The
    /// slot grid has cell size
    /// [`CLUSTER_SPATIAL_SLOT_SIZE`].
    pub slot: (i32, i32),
}

/// Marker on every spawned tactical icon entity. The
/// body is a single `Sprite` (no child text entity):
/// issue #36 drops text labels in favour of a
/// semi-transparent icon whose shape and color encode
/// the cluster kind. Only the cluster key is needed to
/// reconcile the entity on subsequent ticks; the cluster
/// itself carries the count of merged sources.
#[derive(Debug, Component, Clone, Copy, Default)]
pub struct TacticalMarker;

/// Source point for the cluster algorithm.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TacticalSource {
    pub position: Vec2,
    pub kind: TacticalMarkerKind,
    pub owner: SwarmId,
}

/// One merged cluster. `position` is the running average
/// of the merged source positions; `count` is the number
/// of sources that collapsed. The `slot` mirrors the
/// keying so callers can use it without re-quantizing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TacticalCluster {
    pub position: Vec2,
    pub kind: TacticalMarkerKind,
    pub owner: SwarmId,
    pub count: u32,
    /// Quantized world-space slot. See
    /// [`cluster_spatial_slot`].
    pub slot: (i32, i32),
}

/// Pick the merge radius for the current camera zoom.
/// Grows linearly from
/// [`TacticalOverlaySettings::merge_radius_world`] at
/// `zoom = show_zoom_threshold` to
/// [`TacticalOverlaySettings::far_merge_radius_world`] at
/// `zoom = far_merge_zoom`, then stays at the far value.
///
/// Edge cases:
///
/// - `merge_radius_world <= 0.0` returns 0 (every marker
///   stands alone regardless of zoom).
/// - `far_merge_radius_world <= merge_radius_world`
///   collapses the linear ramp to a constant.
pub fn cluster_radius_for_zoom(zoom: f32, settings: &TacticalOverlaySettings) -> f32 {
    if settings.merge_radius_world <= 0.0 {
        return 0.0;
    }
    if settings.far_merge_radius_world <= settings.merge_radius_world {
        return settings.merge_radius_world;
    }
    let effective_zoom = zoom.max(0.0);
    if effective_zoom <= settings.show_zoom_threshold {
        return settings.merge_radius_world;
    }
    if effective_zoom >= settings.far_merge_zoom {
        return settings.far_merge_radius_world;
    }
    let t = (effective_zoom - settings.show_zoom_threshold)
        / (settings.far_merge_zoom - settings.show_zoom_threshold);
    settings.merge_radius_world
        + t * (settings.far_merge_radius_world - settings.merge_radius_world)
}

/// Quantize a world position to a discrete `(i32, i32)`
/// slot. Two positions that fall in the same slot are
/// "near" enough that the player can treat their
/// markers as fungible; positions in different slots
/// must keep separate markers.
///
/// The slot grid uses [`CLUSTER_SPATIAL_SLOT_SIZE`] as
/// its cell size, so any two positions that survive the
/// cluster merge pass (i.e. are more than the maximum
/// merge radius apart) also land in different slots.
/// Positions in the same slot may either merge into one
/// cluster (when within the merge radius) or survive as
/// two clusters only if the player intentionally puts
/// them very close together.
pub fn cluster_spatial_slot(position: Vec2) -> (i32, i32) {
    let slot_x = (position.x / CLUSTER_SPATIAL_SLOT_SIZE).floor() as i32;
    let slot_y = (position.y / CLUSTER_SPATIAL_SLOT_SIZE).floor() as i32;
    (slot_x, slot_y)
}

/// Merge a slice of [`TacticalSource`] entries into
/// [`TacticalCluster`]s using a greedy single-link
/// algorithm: scan the inputs in order, and any source
/// within `merge_radius` of an existing cluster's
/// `position` joins that cluster; otherwise it seeds a new
/// cluster.
///
/// Each cluster's `slot` is stamped on creation and
/// refreshed on every merge, so a centroid that drifts
/// across a slot boundary picks up the new slot.
///
/// `merge_radius <= 0.0` produces one cluster per source
/// (no merging). `sources` may be empty; the function
/// returns an empty vec. The output preserves no particular
/// order between clusters.
pub fn cluster_tactical_markers(
    sources: &[TacticalSource],
    merge_radius: f32,
) -> Vec<TacticalCluster> {
    let radius = merge_radius.max(0.0);
    let mut clusters: Vec<TacticalCluster> = Vec::new();
    for src in sources {
        let radius_sq = radius * radius;
        let mut joined_index: Option<usize> = None;
        for (idx, cluster) in clusters.iter().enumerate() {
            if cluster.kind != src.kind || cluster.owner != src.owner {
                continue;
            }
            let delta = cluster.position - src.position;
            if delta.length_squared() <= radius_sq {
                joined_index = Some(idx);
                break;
            }
        }
        if let Some(idx) = joined_index {
            let cluster = &mut clusters[idx];
            let total = cluster.count + 1;
            // Running average so the cluster position
            // tracks the centroid of the merged sources.
            cluster.position =
                (cluster.position * cluster.count as f32 + src.position) / total as f32;
            cluster.count = total;
            cluster.slot = cluster_spatial_slot(cluster.position);
        } else {
            clusters.push(TacticalCluster {
                position: src.position,
                kind: src.kind,
                owner: src.owner,
                count: 1,
                slot: cluster_spatial_slot(src.position),
            });
        }
    }
    clusters
}

/// Nudge a vector of [`TacticalCluster`] positions apart
/// in world space so on-screen icons do not overlap.
/// The function returns a fresh `Vec` with updated
/// positions and refreshed slots; the input is not
/// mutated.
///
/// Algorithm:
///
/// 1. Convert `icon_screen_size` to the minimum world
///    distance for non-overlap: `min_world =
///    icon_screen_size / zoom`. Two icons are
///    non-overlapping when their centres are at least
///    `min_world` apart in world space.
/// 2. Run [`DEOVERLAP_ITERATIONS`] relaxation passes.
///    Each pass scans every unordered pair, and for any
///    pair whose distance is below `min_world` pushes the
///    two positions apart by `(min_world - dist) / 2`
///    along the line connecting them. Co-located
///    clusters (distance <= 1e-6) get a fixed-axis
///    separation direction derived from the pair index
///    so the relaxation is reproducible across runs.
///
/// `zoom < 1.0` clamps to `1.0` (matching
/// [`marker_world_size_for_zoom`]) so a misconfigured
/// camera cannot produce an infinitely small nudge.
/// Empty or single-cluster input is returned unchanged.
pub fn deoverlap_clusters(
    clusters: Vec<TacticalCluster>,
    zoom: f32,
    icon_screen_size: f32,
) -> Vec<TacticalCluster> {
    let n = clusters.len();
    if n <= 1 {
        return clusters;
    }
    let effective_zoom = if zoom >= 1.0 { zoom } else { 1.0 };
    let min_world_dist = icon_screen_size / effective_zoom;
    let mut positions: Vec<Vec2> = clusters.iter().map(|c| c.position).collect();
    for _ in 0..DEOVERLAP_ITERATIONS {
        for i in 0..n {
            for j in (i + 1)..n {
                let delta = positions[j] - positions[i];
                let dist = delta.length();
                if dist >= min_world_dist {
                    continue;
                }
                let (dir, push) = if dist > 1e-6 {
                    (delta / dist, (min_world_dist - dist) * 0.5)
                } else {
                    // Co-located: pick a fixed axis
                    // derived from the pair index so the
                    // relaxation is reproducible.
                    let axis = if (i + j) % 2 == 0 { Vec2::X } else { Vec2::Y };
                    (axis, min_world_dist * 0.5)
                };
                positions[i] -= dir * push;
                positions[j] += dir * push;
            }
        }
    }
    clusters
        .into_iter()
        .enumerate()
        .map(|(i, mut c)| {
            c.position = positions[i];
            c.slot = cluster_spatial_slot(c.position);
            c
        })
        .collect()
}

/// Background color of the marker panel for a cluster.
/// The RGB channels are shared with the structure overlay
/// so a marker overlaid on its target reads as the same
/// hue at two zoom levels; the alpha is fixed at
/// [`TACTICAL_MARKER_ALPHA`] so the marker is
/// semi-transparent and the player can still see the
/// underlying map through it.
pub fn cluster_color(kind: TacticalMarkerKind) -> Color {
    let (r, g, b) = match kind {
        TacticalMarkerKind::PlayerBase => (0.10, 0.55, 0.90),
        TacticalMarkerKind::OpponentBase => (0.85, 0.20, 0.20),
        TacticalMarkerKind::Deposit => (0.65, 0.45, 0.10),
        TacticalMarkerKind::Facility => (0.20, 0.40, 0.70),
        TacticalMarkerKind::Stockpile => (0.20, 0.50, 0.20),
        TacticalMarkerKind::Planned => (0.45, 0.45, 0.45),
        TacticalMarkerKind::Charger => (0.55, 0.30, 0.70),
    };
    Color::srgba(r, g, b, TACTICAL_MARKER_ALPHA)
}

/// Decide the visibility for the overlay given the current
/// camera zoom and the configured threshold. Mirrors
/// [`crate::structure_overlay::overlay_visibility_for_zoom`]
/// so the two layers share the same boundary behaviour.
///
/// - `threshold == f32::INFINITY` always hides.
/// - `threshold <= 0.0` always shows.
pub fn tactical_visibility_for_zoom(zoom: f32, threshold: f32) -> Visibility {
    if threshold == f32::INFINITY {
        return Visibility::Hidden;
    }
    if threshold <= 0.0 {
        return Visibility::Inherited;
    }
    if zoom >= threshold {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    }
}

/// Compute the world-size of a marker body for the
/// current camera zoom. The marker body is a
/// unit-rectangle `Sprite` (custom size 1.0 in each
/// axis) scaled by this value, so its on-screen pixel
/// footprint is `icon_screen_size` regardless of zoom:
/// `world_size * zoom = icon_screen_size`.
///
/// `zoom < 1.0` clamps to `1.0` so a misconfigured
/// settings resource cannot produce an infinitely large
/// marker.
pub fn marker_world_size_for_zoom(zoom: f32, screen_size: f32) -> f32 {
    let effective_zoom = if zoom >= 1.0 { zoom } else { 1.0 };
    screen_size / effective_zoom
}

// ---------------------------------------------------------------------------
// Bevy systems
// ---------------------------------------------------------------------------

/// Plugin that wires the tactical overlay update system
/// into the `Update` schedule. Each tick produces a fresh
/// set of markers from the current world state and
/// despawns any marker that lost its source.
pub struct TacticalOverlayPlugin;

impl Plugin for TacticalOverlayPlugin {
    fn build(&self, app: &mut App) {
        if !app.world().contains_resource::<TacticalOverlaySettings>() {
            app.init_resource::<TacticalOverlaySettings>();
        }
        app.add_systems(Update, tactical_overlay_update_system);
    }
}

/// Reconcile the current cluster list with the spawned
/// marker entities. Existing markers that survive across
/// ticks keep their entity (and therefore their slot);
/// a new cluster spawns a fresh body with the correct
/// initial visibility, and an unmatched existing marker
/// is despawned.
///
/// Each marker is a single `Sprite` entity with a
/// `TacticalMarker` and a `TacticalClusterKey`. There is
/// no child label entity: issue #36 drops text labels in
/// favour of a semi-transparent icon body whose shape and
/// color encode the cluster kind.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn tactical_overlay_update_system(
    mut commands: Commands,
    settings: Res<TacticalOverlaySettings>,
    zoom_query: Query<&CameraZoom2d>,
    swarms: Query<
        (Entity, &Transform, Option<&OpponentSwarm>, Option<&SwarmId>),
        (With<Swarm>, Without<TacticalMarker>),
    >,
    deposits: Query<&Transform, (With<ResourceDeposit>, Without<TacticalMarker>)>,
    facilities: Query<&Transform, (With<ProductionFacility>, Without<TacticalMarker>)>,
    stockpiles: Query<&Transform, (With<Stockpile>, Without<TacticalMarker>)>,
    planned: Query<&Transform, (With<PlannedStructure>, Without<TacticalMarker>)>,
    chargers: Query<&Transform, (With<Charger>, Without<TacticalMarker>)>,
    mut existing: Query<
        (
            Entity,
            &TacticalClusterKey,
            &TacticalMarker,
            &mut Transform,
            &mut Visibility,
        ),
        With<TacticalMarker>,
    >,
    mut source_cache: Local<Vec<TacticalSource>>,
) {
    let zoom = zoom_query.iter().next().map(|z| z.zoom).unwrap_or(1.0);
    let visibility = tactical_visibility_for_zoom(zoom, settings.show_zoom_threshold);
    let marker_size = marker_world_size_for_zoom(zoom, settings.marker_screen_size);

    source_cache.clear();
    for (_entity, transform, opponent, swarm_id) in &swarms {
        let owner = swarm_id.copied().unwrap_or(SwarmId::PLAYER);
        let kind = if opponent.is_some() {
            TacticalMarkerKind::OpponentBase
        } else {
            TacticalMarkerKind::PlayerBase
        };
        source_cache.push(TacticalSource {
            position: transform.translation.truncate(),
            kind,
            owner,
        });
    }
    for transform in &deposits {
        source_cache.push(TacticalSource {
            position: transform.translation.truncate(),
            kind: TacticalMarkerKind::Deposit,
            owner: UNOWNED_SWARM_ID,
        });
    }
    for transform in &facilities {
        source_cache.push(TacticalSource {
            position: transform.translation.truncate(),
            kind: TacticalMarkerKind::Facility,
            owner: UNOWNED_SWARM_ID,
        });
    }
    for transform in &stockpiles {
        source_cache.push(TacticalSource {
            position: transform.translation.truncate(),
            kind: TacticalMarkerKind::Stockpile,
            owner: UNOWNED_SWARM_ID,
        });
    }
    for transform in &planned {
        source_cache.push(TacticalSource {
            position: transform.translation.truncate(),
            kind: TacticalMarkerKind::Planned,
            owner: UNOWNED_SWARM_ID,
        });
    }
    for transform in &chargers {
        source_cache.push(TacticalSource {
            position: transform.translation.truncate(),
            kind: TacticalMarkerKind::Charger,
            owner: UNOWNED_SWARM_ID,
        });
    }
    let mut clusters = cluster_tactical_markers(
        source_cache.as_slice(),
        cluster_radius_for_zoom(zoom, &settings),
    );
    clusters = deoverlap_clusters(clusters, zoom, settings.marker_screen_size);

    // Index existing markers by their cluster key for
    // O(1) lookup in the patch loop.
    let mut by_key: HashMap<TacticalClusterKey, Entity> = HashMap::new();
    for (entity, key, _, _, _) in existing.iter() {
        by_key.insert(*key, entity);
    }
    let mut matched: HashSet<Entity> = HashSet::new();
    for cluster in &clusters {
        let key = TacticalClusterKey {
            kind: cluster.kind,
            owner: cluster.owner,
            slot: cluster.slot,
        };
        if let Some(&entity) = by_key.get(&key) {
            // Patch: update position, scale, and
            // visibility on the survivor.
            if let Ok((_, _, _, mut transform, mut vis)) = existing.get_mut(entity) {
                transform.translation = cluster.position.extend(TACTICAL_MARKER_Z);
                transform.scale = Vec3::splat(marker_size);
                if *vis != visibility {
                    *vis = visibility;
                }
            }
            matched.insert(entity);
        } else {
            // Spawn a fresh marker body. The body is a
            // unit-rectangle sprite scaled by marker_size
            // so its on-screen footprint stays
            // `marker_screen_size` pixels regardless of
            // zoom. The sprite color is the per-kind
            // 50%-alpha panel color from `cluster_color`.
            let body = commands
                .spawn((
                    Sprite {
                        color: cluster_color(cluster.kind),
                        custom_size: Some(Vec2::splat(1.0)),
                        ..default()
                    },
                    Transform::from_translation(cluster.position.extend(TACTICAL_MARKER_Z))
                        .with_scale(Vec3::splat(marker_size)),
                    TacticalMarker,
                    key,
                    visibility,
                ))
                .id();
            matched.insert(body);
        }
    }
    // Despawn unmatched existing markers.
    for (entity, _, _, _, _) in existing.iter() {
        if !matched.contains(&entity) {
            commands.entity(entity).despawn();
        }
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    //! Pure-helper unit tests. End-to-end behavior (spawn,
    //! visibility, clustering at the configured zoom,
    //! de-overlap, multi-cluster coexistence) lives in
    //! `tests/behavior/tactical_overlay.rs`.

    use super::*;

    fn src(pos: Vec2, kind: TacticalMarkerKind, owner: SwarmId) -> TacticalSource {
        TacticalSource {
            position: pos,
            kind,
            owner,
        }
    }

    fn cluster(pos: Vec2, kind: TacticalMarkerKind, owner: SwarmId) -> TacticalCluster {
        TacticalCluster {
            position: pos,
            kind,
            owner,
            count: 1,
            slot: cluster_spatial_slot(pos),
        }
    }

    #[test]
    fn default_settings_match_module_constants() {
        let s = TacticalOverlaySettings::default();
        assert_eq!(s.show_zoom_threshold, DEFAULT_TACTICAL_SHOW_ZOOM_THRESHOLD);
        assert_eq!(s.merge_radius_world, DEFAULT_TACTICAL_MERGE_RADIUS_WORLD);
        assert_eq!(
            s.far_merge_radius_world,
            DEFAULT_TACTICAL_FAR_MERGE_RADIUS_WORLD
        );
        assert_eq!(s.far_merge_zoom, DEFAULT_TACTICAL_FAR_MERGE_ZOOM);
        assert_eq!(s.marker_screen_size, DEFAULT_TACTICAL_MARKER_SCREEN_SIZE);
    }

    #[test]
    fn cluster_radius_at_or_below_threshold_uses_moderate_value() {
        let s = TacticalOverlaySettings::default();
        assert_eq!(
            cluster_radius_for_zoom(1.0, &s),
            s.merge_radius_world,
            "default play zoom is below the show threshold"
        );
        assert_eq!(
            cluster_radius_for_zoom(s.show_zoom_threshold, &s),
            s.merge_radius_world
        );
    }

    #[test]
    fn cluster_radius_at_or_above_far_zoom_uses_far_value() {
        let s = TacticalOverlaySettings::default();
        assert_eq!(
            cluster_radius_for_zoom(s.far_merge_zoom, &s),
            s.far_merge_radius_world
        );
        assert_eq!(
            cluster_radius_for_zoom(s.far_merge_zoom + 50.0, &s),
            s.far_merge_radius_world,
            "beyond the far zoom the radius must not grow further"
        );
    }

    #[test]
    fn cluster_radius_grows_linearly_between_thresholds() {
        let s = TacticalOverlaySettings::default();
        // Midpoint between show_zoom_threshold (8) and
        // far_merge_zoom (16) is zoom 12, which sits at
        // t=0.5 along the ramp and produces the
        // arithmetic midpoint of the two radii.
        let mid_zoom = (s.show_zoom_threshold + s.far_merge_zoom) * 0.5;
        let expected = (s.merge_radius_world + s.far_merge_radius_world) * 0.5;
        let got = cluster_radius_for_zoom(mid_zoom, &s);
        assert!(
            (got - expected).abs() < 1.0,
            "midpoint zoom must produce midpoint radius; got {got}, expected {expected}"
        );
    }

    #[test]
    fn cluster_radius_zero_settings_returns_zero() {
        let s = TacticalOverlaySettings {
            merge_radius_world: 0.0,
            ..Default::default()
        };
        assert_eq!(cluster_radius_for_zoom(10.0, &s), 0.0);
    }

    #[test]
    fn cluster_radius_far_not_above_moderate_returns_moderate() {
        // A misconfigured settings resource that puts
        // the "far" value below the moderate value must
        // not produce a negative slope; the function
        // collapses to a constant.
        let moderate = 2048.0;
        let s = TacticalOverlaySettings {
            merge_radius_world: moderate,
            far_merge_radius_world: moderate - 100.0,
            ..Default::default()
        };
        assert_eq!(cluster_radius_for_zoom(10.0, &s), moderate);
    }

    // -----------------------------------------------------------------------
    // Spatial slot
    // -----------------------------------------------------------------------

    #[test]
    fn spatial_slot_is_origin_for_origin_position() {
        assert_eq!(cluster_spatial_slot(Vec2::ZERO), (0, 0));
    }

    #[test]
    fn spatial_slot_quantizes_to_slot_size_grid() {
        // Two positions inside the same cell share a
        // slot. The slot grid is half-open
        // `[n*size, (n+1)*size)`.
        let a = Vec2::new(1.0, 1.0);
        let b = Vec2::new(CLUSTER_SPATIAL_SLOT_SIZE - 1.0, 0.0);
        assert_eq!(cluster_spatial_slot(a), cluster_spatial_slot(b));
    }

    #[test]
    fn spatial_slot_steps_at_cell_boundary() {
        // The boundary is owned by the next cell, so
        // `slot_size` exactly is the first member of the
        // next cell.
        let inside = cluster_spatial_slot(Vec2::new(CLUSTER_SPATIAL_SLOT_SIZE - 1.0, 0.0));
        let on_boundary = cluster_spatial_slot(Vec2::new(CLUSTER_SPATIAL_SLOT_SIZE, 0.0));
        let next_cell = cluster_spatial_slot(Vec2::new(CLUSTER_SPATIAL_SLOT_SIZE + 1.0, 0.0));
        assert_eq!(inside, (0, 0));
        assert_eq!(on_boundary, (1, 0));
        assert_eq!(next_cell, (1, 0));
    }

    #[test]
    fn spatial_slot_handles_negative_coordinates() {
        // A position one slot south and west of the
        // origin is in slot (-1, -1), and a position
        // exactly on the negative boundary is in (-1, 0)
        // (the boundary itself is the first member of
        // the next cell).
        assert_eq!(cluster_spatial_slot(Vec2::new(-1.0, -1.0)), (-1, -1));
        assert_eq!(
            cluster_spatial_slot(Vec2::new(-CLUSTER_SPATIAL_SLOT_SIZE, 0.0)),
            (-1, 0)
        );
    }

    #[test]
    fn spatial_slot_size_matches_max_merge_radius() {
        // The slot grid is sized to the max merge radius
        // so two clusters that don't merge always end
        // up in different slots (or the same one only
        // when they would have merged anyway).
        assert_eq!(
            CLUSTER_SPATIAL_SLOT_SIZE,
            DEFAULT_TACTICAL_FAR_MERGE_RADIUS_WORLD
        );
    }

    // -----------------------------------------------------------------------
    // cluster_tactical_markers
    // -----------------------------------------------------------------------

    #[test]
    fn cluster_merges_two_close_same_key_sources() {
        let owner = SwarmId(1);
        let s1 = src(Vec2::new(0.0, 0.0), TacticalMarkerKind::Deposit, owner);
        let s2 = src(Vec2::new(10.0, 0.0), TacticalMarkerKind::Deposit, owner);
        let clusters = cluster_tactical_markers(&[s1, s2], 100.0);
        assert_eq!(clusters.len(), 1);
        assert_eq!(clusters[0].count, 2);
        // Centroid of (0,0) and (10,0) is (5,0).
        assert!((clusters[0].position.x - 5.0).abs() < 1e-3);
        assert!(clusters[0].position.y.abs() < 1e-3);
        // Slot for the centroid (5,0) is the origin slot.
        assert_eq!(clusters[0].slot, (0, 0));
    }

    #[test]
    fn cluster_keeps_far_apart_sources_separate() {
        let owner = SwarmId(1);
        let s1 = src(Vec2::new(0.0, 0.0), TacticalMarkerKind::Deposit, owner);
        let s2 = src(Vec2::new(200.0, 0.0), TacticalMarkerKind::Deposit, owner);
        let clusters = cluster_tactical_markers(&[s1, s2], 100.0);
        assert_eq!(clusters.len(), 2);
        assert!(clusters.iter().all(|c| c.count == 1));
    }

    #[test]
    fn cluster_separates_by_kind_even_when_close() {
        let owner = SwarmId(1);
        let s1 = src(Vec2::new(0.0, 0.0), TacticalMarkerKind::Deposit, owner);
        let s2 = src(Vec2::new(0.0, 0.0), TacticalMarkerKind::Facility, owner);
        let clusters = cluster_tactical_markers(&[s1, s2], 100.0);
        assert_eq!(
            clusters.len(),
            2,
            "different kinds must stay separate even at zero distance"
        );
    }

    #[test]
    fn cluster_separates_by_owner_even_when_close() {
        let s1 = src(Vec2::new(0.0, 0.0), TacticalMarkerKind::Deposit, SwarmId(1));
        let s2 = src(Vec2::new(0.0, 0.0), TacticalMarkerKind::Deposit, SwarmId(2));
        let clusters = cluster_tactical_markers(&[s1, s2], 100.0);
        assert_eq!(
            clusters.len(),
            2,
            "different owners must stay separate even at zero distance"
        );
    }

    #[test]
    fn cluster_radius_zero_means_no_merging() {
        let owner = SwarmId(1);
        let s1 = src(Vec2::new(0.0, 0.0), TacticalMarkerKind::Deposit, owner);
        let s2 = src(Vec2::new(0.001, 0.0), TacticalMarkerKind::Deposit, owner);
        let clusters = cluster_tactical_markers(&[s1, s2], 0.0);
        assert_eq!(clusters.len(), 2);
    }

    #[test]
    fn cluster_handles_empty_input() {
        let clusters = cluster_tactical_markers(&[], 100.0);
        assert!(clusters.is_empty());
    }

    #[test]
    fn cluster_negative_radius_treated_as_zero() {
        let owner = SwarmId(1);
        let s1 = src(Vec2::new(0.0, 0.0), TacticalMarkerKind::Deposit, owner);
        let s2 = src(Vec2::new(0.001, 0.0), TacticalMarkerKind::Deposit, owner);
        let clusters = cluster_tactical_markers(&[s1, s2], -10.0);
        assert_eq!(clusters.len(), 2);
    }

    #[test]
    fn cluster_stamps_slot_on_new_cluster() {
        // A fresh cluster at (1000, 0) with a
        // 6000-unit slot size lands in slot (0, 0). A
        // cluster at (7000, 0) lands in slot (1, 0).
        let owner = SwarmId(1);
        let s1 = src(Vec2::new(1000.0, 0.0), TacticalMarkerKind::Deposit, owner);
        let s2 = src(Vec2::new(7000.0, 0.0), TacticalMarkerKind::Deposit, owner);
        let clusters = cluster_tactical_markers(&[s1, s2], 0.0);
        assert_eq!(clusters.len(), 2);
        let slots: Vec<(i32, i32)> = clusters.iter().map(|c| c.slot).collect();
        assert!(slots.contains(&(0, 0)));
        assert!(slots.contains(&(1, 0)));
    }

    #[test]
    fn cluster_refreshes_slot_when_centroid_crosses_boundary() {
        // Source A is in slot 0; source B is in slot
        // 1. The centroid of (100, 0) and (15000, 0) is
        // (7550, 0), which sits in slot 1. With a
        // merge radius that covers the distance the
        // algorithm collapses the pair into one
        // cluster, and the merged cluster's `slot` is
        // (1, 0) -- not (0, 0) -- because the slot
        // refreshes on every merge.
        let owner = SwarmId(1);
        let s1 = src(Vec2::new(100.0, 0.0), TacticalMarkerKind::Deposit, owner);
        let s2 = src(Vec2::new(15000.0, 0.0), TacticalMarkerKind::Deposit, owner);
        let clusters = cluster_tactical_markers(&[s1, s2], 100_000.0);
        assert_eq!(clusters.len(), 1);
        assert_eq!(clusters[0].slot, (1, 0));
    }

    // -----------------------------------------------------------------------
    // deoverlap_clusters
    // -----------------------------------------------------------------------

    #[test]
    fn deoverlap_returns_empty_for_empty_input() {
        let out = deoverlap_clusters(vec![], 8.0, 32.0);
        assert!(out.is_empty());
    }

    #[test]
    fn deoverlap_returns_single_cluster_unchanged() {
        let c = cluster(Vec2::new(0.0, 0.0), TacticalMarkerKind::Deposit, SwarmId(1));
        let out = deoverlap_clusters(vec![c], 8.0, 32.0);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].position, Vec2::ZERO);
    }

    #[test]
    fn deoverlap_pushes_two_overlapping_clusters_apart() {
        // Two clusters 1 world unit apart at zoom 8.0
        // (icon size 32, so min world distance = 4).
        // After de-overlap they must sit at least
        // 4 world units apart along their connecting
        // line, and the midpoint of the new positions
        // is the same as the midpoint of the originals
        // (the pair relaxes without bias).
        let a = cluster(Vec2::new(0.0, 0.0), TacticalMarkerKind::Deposit, SwarmId(1));
        let b = cluster(Vec2::new(1.0, 0.0), TacticalMarkerKind::Deposit, SwarmId(2));
        let midpoint = (a.position + b.position) * 0.5;
        let out = deoverlap_clusters(vec![a, b], 8.0, 32.0);
        let dist = (out[0].position - out[1].position).length();
        assert!(
            dist >= 4.0 - 1e-4,
            "two overlapping icons must be pushed apart to at least the min world distance; got {dist}"
        );
        // The relaxation is mass-balanced: the midpoint
        // of the two relaxed positions is the midpoint
        // of the original positions (each icon moves by
        // half the overlap).
        let new_midpoint = (out[0].position + out[1].position) * 0.5;
        assert!(
            (new_midpoint - midpoint).length() < 1e-3,
            "midpoint of the relaxed pair must equal the original midpoint; got {new_midpoint:?} vs {midpoint:?}"
        );
    }

    #[test]
    fn deoverlap_leaves_far_apart_clusters_in_place() {
        // Two clusters 1000 world units apart at zoom
        // 8.0. Min world distance is 4, so 1000 is
        // well above the overlap threshold. The
        // de-overlap pass must not move them.
        let a = cluster(Vec2::new(0.0, 0.0), TacticalMarkerKind::Deposit, SwarmId(1));
        let b = cluster(
            Vec2::new(1000.0, 0.0),
            TacticalMarkerKind::Deposit,
            SwarmId(2),
        );
        let out = deoverlap_clusters(vec![a, b], 8.0, 32.0);
        assert!((out[0].position - Vec2::new(0.0, 0.0)).length() < 1e-3);
        assert!((out[1].position - Vec2::new(1000.0, 0.0)).length() < 1e-3);
    }

    #[test]
    fn deoverlap_spreads_three_co_loc_clusters() {
        // Three co-located clusters at the origin
        // (would overlap on screen). After the
        // de-overlap pass no two of them are within
        // the min world distance.
        let a = cluster(Vec2::new(0.0, 0.0), TacticalMarkerKind::Deposit, SwarmId(1));
        let b = cluster(Vec2::new(0.0, 0.0), TacticalMarkerKind::Deposit, SwarmId(2));
        let c = cluster(
            Vec2::new(0.0, 0.0),
            TacticalMarkerKind::Facility,
            SwarmId(1),
        );
        let out = deoverlap_clusters(vec![a, b, c], 8.0, 32.0);
        let min_dist = 32.0 / 8.0;
        for i in 0..out.len() {
            for j in (i + 1)..out.len() {
                let d = (out[i].position - out[j].position).length();
                assert!(
                    d >= min_dist - 1e-3,
                    "pair ({i}, {j}) still overlaps: distance {d} < {min_dist}"
                );
            }
        }
    }

    #[test]
    fn deoverlap_refreshes_slot_to_match_new_position() {
        // A cluster whose de-overlap moves it across a
        // slot boundary must have its `slot` field
        // updated to match the new position.
        let a = cluster(Vec2::new(0.0, 0.0), TacticalMarkerKind::Deposit, SwarmId(1));
        let b = cluster(
            Vec2::new(0.0, 0.0),
            TacticalMarkerKind::Facility,
            SwarmId(1),
        );
        let out = deoverlap_clusters(vec![a, b], 8.0, 32.0);
        for c in &out {
            assert_eq!(c.slot, cluster_spatial_slot(c.position));
        }
    }

    #[test]
    fn deoverlap_clamps_zero_zoom_to_one() {
        // A zoom of 0.0 (effectively a "huge" min world
        // distance) must not produce NaN or infinite
        // positions. The clamp mirrors
        // `marker_world_size_for_zoom`.
        let a = cluster(Vec2::new(0.0, 0.0), TacticalMarkerKind::Deposit, SwarmId(1));
        let b = cluster(Vec2::new(1.0, 0.0), TacticalMarkerKind::Deposit, SwarmId(2));
        let out = deoverlap_clusters(vec![a, b], 0.0, 32.0);
        for c in &out {
            assert!(c.position.is_finite());
        }
    }

    // -----------------------------------------------------------------------
    // cluster_color
    // -----------------------------------------------------------------------

    #[test]
    fn cluster_color_alpha_is_50_percent() {
        // Every tactical marker is semi-transparent
        // (issue #36 acceptance #2). The alpha is
        // shared across kinds, so all seven colors
        // share the same alpha component.
        for kind in [
            TacticalMarkerKind::PlayerBase,
            TacticalMarkerKind::OpponentBase,
            TacticalMarkerKind::Deposit,
            TacticalMarkerKind::Facility,
            TacticalMarkerKind::Stockpile,
            TacticalMarkerKind::Planned,
            TacticalMarkerKind::Charger,
        ] {
            let c = cluster_color(kind);
            let srgba = c.to_srgba();
            assert!(
                (srgba.alpha - TACTICAL_MARKER_ALPHA).abs() < 1e-4,
                "{kind:?} must have alpha {TACTICAL_MARKER_ALPHA}, got {}",
                srgba.alpha
            );
        }
    }

    #[test]
    fn cluster_color_pairwise_distinct() {
        let kinds = [
            TacticalMarkerKind::PlayerBase,
            TacticalMarkerKind::OpponentBase,
            TacticalMarkerKind::Deposit,
            TacticalMarkerKind::Facility,
            TacticalMarkerKind::Stockpile,
            TacticalMarkerKind::Planned,
            TacticalMarkerKind::Charger,
        ];
        let colors: Vec<Color> = kinds.iter().map(|k| cluster_color(*k)).collect();
        for i in 0..colors.len() {
            for j in (i + 1)..colors.len() {
                assert_ne!(
                    colors[i], colors[j],
                    "kinds {:?} and {:?} must have distinct colors",
                    kinds[i], kinds[j]
                );
            }
        }
    }

    // -----------------------------------------------------------------------
    // Visibility, size
    // -----------------------------------------------------------------------

    #[test]
    fn visibility_hidden_below_threshold() {
        assert_eq!(
            tactical_visibility_for_zoom(1.0, 4.0),
            Visibility::Hidden,
            "default play zoom must hide the overlay"
        );
        assert_eq!(tactical_visibility_for_zoom(3.99, 4.0), Visibility::Hidden);
    }

    #[test]
    fn visibility_visible_at_or_above_threshold() {
        assert_eq!(
            tactical_visibility_for_zoom(4.0, 4.0),
            Visibility::Inherited
        );
        assert_eq!(
            tactical_visibility_for_zoom(10.0, 4.0),
            Visibility::Inherited
        );
    }

    #[test]
    fn visibility_zero_threshold_always_visible() {
        assert_eq!(
            tactical_visibility_for_zoom(f32::INFINITY, 0.0),
            Visibility::Inherited
        );
    }

    #[test]
    fn visibility_infinite_threshold_always_hidden() {
        assert_eq!(
            tactical_visibility_for_zoom(1.0, f32::INFINITY),
            Visibility::Hidden
        );
    }

    #[test]
    fn marker_world_size_keeps_screen_constant() {
        // A camera zoom of 4.0 must produce a world
        // size of 32/4 = 8 world units so the on-screen
        // footprint is 32 pixels regardless of zoom.
        assert!((marker_world_size_for_zoom(4.0, 32.0) - 8.0).abs() < 1e-4);
        assert!((marker_world_size_for_zoom(8.0, 32.0) - 4.0).abs() < 1e-4);
        assert!((marker_world_size_for_zoom(16.0, 32.0) - 2.0).abs() < 1e-4);
    }

    #[test]
    fn marker_world_size_clamps_zero_zoom() {
        // A misconfigured camera must not produce an
        // infinite marker.
        let size = marker_world_size_for_zoom(0.0, 32.0);
        assert!(size.is_finite(), "size must be finite, got {size}");
        assert!(size > 0.0, "size must stay positive, got {size}");
    }
}
