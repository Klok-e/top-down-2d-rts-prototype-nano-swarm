//! Zoomed-out tactical overlay (issue #31).
//!
//! When the camera zooms past the structure overlay's hide
//! threshold the always-visible status labels disappear.
//! The tactical overlay takes their place: cluster markers
//! for the player base, opponent base, deposits,
//! facilities, stockpiles, planned structures, and
//! chargers, with progressive merging as the player zooms
//! farther out.
//!
//! Markers stay screen-constant by setting the body's
//! `Transform::scale` to `screen_size / zoom`. Cluster
//! radius grows linearly with zoom so a single
//! [`cluster_tactical_markers`] call covers every level.

use bevy::prelude::*;

use crate::fly_camera::CameraZoom2d;
use crate::nanobot::{
    Charger, OpponentSwarm, PlannedStructure, ProductionFacility, Swarm, SwarmId,
};
use crate::resources::{ResourceDeposit, Stockpile};

/// Camera zoom at or above which the tactical overlay
/// appears. Matches
/// [`crate::structure_overlay::DEFAULT_OVERLAY_HIDE_ZOOM_THRESHOLD`]
/// so the two layers fade at the same boundary.
pub const DEFAULT_TACTICAL_SHOW_ZOOM_THRESHOLD: f32 = 4.0;

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

/// On-screen pixel size for the marker label text.
pub const DEFAULT_TACTICAL_LABEL_SCREEN_SIZE: f32 = 16.0;

/// Z-translation for the marker body sprites. Sits between
/// the zone overlay and the gameplay sprites so a marker
/// never eclipses a real structure.
pub const TACTICAL_MARKER_Z: f32 = 0.5;

/// Z-translation for the marker label text. Renders above
/// the marker body so the text reads cleanly.
pub const TACTICAL_LABEL_Z: f32 = 0.75;

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
    pub label_screen_size: f32,
}

impl Default for TacticalOverlaySettings {
    fn default() -> Self {
        Self {
            show_zoom_threshold: DEFAULT_TACTICAL_SHOW_ZOOM_THRESHOLD,
            merge_radius_world: DEFAULT_TACTICAL_MERGE_RADIUS_WORLD,
            far_merge_radius_world: DEFAULT_TACTICAL_FAR_MERGE_RADIUS_WORLD,
            far_merge_zoom: DEFAULT_TACTICAL_FAR_MERGE_ZOOM,
            marker_screen_size: DEFAULT_TACTICAL_MARKER_SCREEN_SIZE,
            label_screen_size: DEFAULT_TACTICAL_LABEL_SCREEN_SIZE,
        }
    }
}

/// Marker category. The cluster key is
/// `(kind, owner_swarm_id)` so two deposits belonging to
/// different swarms stay separate but two player-owned
/// facilities next to each other collapse.
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
    /// marker kind; the cluster label includes a count.
    Stockpile,
    /// A planned structure.
    Planned,
    /// A charger.
    Charger,
}

/// The cluster key for a marker. Two markers with the same
/// `(kind, owner_id)` and a world distance below the
/// current merge radius collapse to a single cluster.
#[derive(Debug, Component, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TacticalClusterKey {
    pub kind: TacticalMarkerKind,
    pub owner: SwarmId,
}

/// Single spawned tactical marker entity. `count` is what
/// the cluster system writes when it merges several source
/// points together; a marker that stands alone has
/// `count == 1`.
#[derive(Debug, Component, Clone, Copy)]
pub struct TacticalMarker {
    pub count: u32,
}

/// Source point for the cluster algorithm.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TacticalSource {
    pub position: Vec2,
    pub kind: TacticalMarkerKind,
    pub owner: SwarmId,
}

/// One merged cluster. `position` is the running average
/// of the merged source positions; `count` is the number
/// of sources that collapsed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TacticalCluster {
    pub position: Vec2,
    pub kind: TacticalMarkerKind,
    pub owner: SwarmId,
    pub count: u32,
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

/// Merge a slice of [`TacticalSource`] entries into
/// [`TacticalCluster`]s using a greedy single-link
/// algorithm: scan the inputs in order, and any source
/// within `merge_radius` of an existing cluster's
/// `position` joins that cluster; otherwise it seeds a new
/// cluster.
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
        } else {
            clusters.push(TacticalCluster {
                position: src.position,
                kind: src.kind,
                owner: src.owner,
                count: 1,
            });
        }
    }
    clusters
}

/// Short label for a cluster. Compact so it fits inside
/// the marker body at the screen-constant label size.
pub fn cluster_label(cluster: &TacticalCluster) -> String {
    match cluster.kind {
        TacticalMarkerKind::PlayerBase => "You".to_string(),
        TacticalMarkerKind::OpponentBase => "Enemy".to_string(),
        TacticalMarkerKind::Deposit => plural_label("Deposit", cluster.count),
        TacticalMarkerKind::Facility => plural_label("Facility", cluster.count),
        TacticalMarkerKind::Stockpile => plural_label("Stockpile", cluster.count),
        TacticalMarkerKind::Planned => plural_label("Building", cluster.count),
        TacticalMarkerKind::Charger => plural_label("Charger", cluster.count),
    }
}

fn plural_label(base: &str, count: u32) -> String {
    if count <= 1 {
        base.to_string()
    } else {
        format!("{base} x{count}")
    }
}

/// Background color of the marker panel for a cluster.
/// Picked to be distinct from the structure overlay panel
/// colors so a player reading the map can tell the two
/// layers apart at a glance.
pub fn cluster_color(kind: TacticalMarkerKind) -> Color {
    match kind {
        TacticalMarkerKind::PlayerBase => Color::srgba(0.10, 0.55, 0.90, 0.90),
        TacticalMarkerKind::OpponentBase => Color::srgba(0.85, 0.20, 0.20, 0.90),
        TacticalMarkerKind::Deposit => Color::srgba(0.65, 0.45, 0.10, 0.85),
        TacticalMarkerKind::Facility => Color::srgba(0.20, 0.40, 0.70, 0.85),
        TacticalMarkerKind::Stockpile => Color::srgba(0.20, 0.50, 0.20, 0.85),
        TacticalMarkerKind::Planned => Color::srgba(0.45, 0.45, 0.45, 0.85),
        TacticalMarkerKind::Charger => Color::srgba(0.55, 0.30, 0.70, 0.85),
    }
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

/// Compute the on-screen pixel size of a marker body for
/// the current camera zoom. The marker's
/// `Transform::scale` is set to this value so the on-screen
/// footprint stays constant regardless of the
/// orthographic projection's scale.
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
/// ticks keep their entity (and therefore their label
/// child); a new cluster spawns a fresh body with the
/// correct initial visibility, and an unmatched existing
/// marker is despawned.
///
/// The marker body and the label text are two separate
/// entities: the body carries the `TacticalMarker` and
/// `TacticalClusterKey` components, the label is a child
/// `Text2d`. The split keeps the body cheap (just a
/// sprite) and lets the label be moved independently.
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
            &mut TacticalMarker,
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
    let label_size = marker_world_size_for_zoom(zoom, settings.label_screen_size);

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
    let clusters = cluster_tactical_markers(
        source_cache.as_slice(),
        cluster_radius_for_zoom(zoom, &settings),
    );

    // Index existing markers by their cluster key for
    // O(1) lookup in the patch loop.
    let mut by_key: std::collections::HashMap<TacticalClusterKey, Entity> =
        std::collections::HashMap::new();
    for (entity, key, _, _, _) in existing.iter() {
        by_key.insert(*key, entity);
    }
    let mut matched: std::collections::HashSet<Entity> = std::collections::HashSet::new();
    for cluster in &clusters {
        let key = TacticalClusterKey {
            kind: cluster.kind,
            owner: cluster.owner,
        };
        if let Some(&entity) = by_key.get(&key) {
            // Patch: update count, position, scale, and
            // visibility on the survivor.
            if let Ok((_, _, mut marker, mut transform, mut vis)) = existing.get_mut(entity) {
                marker.count = cluster.count;
                transform.translation = cluster.position.extend(TACTICAL_MARKER_Z);
                transform.scale = Vec3::splat(marker_size);
                if *vis != visibility {
                    *vis = visibility;
                }
            }
            matched.insert(entity);
        } else {
            // Spawn a fresh marker body. The label is
            // spawned as a child so it inherits the
            // body's transform.
            let label = cluster_label(cluster);
            let color = cluster_color(cluster.kind);
            let body = commands
                .spawn((
                    Sprite {
                        color,
                        custom_size: Some(Vec2::splat(marker_size)),
                        ..default()
                    },
                    Transform::from_translation(cluster.position.extend(TACTICAL_MARKER_Z))
                        .with_scale(Vec3::splat(marker_size)),
                    TacticalMarker {
                        count: cluster.count,
                    },
                    key,
                    visibility,
                ))
                .with_children(|p| {
                    p.spawn((
                        Text2d::new(label),
                        TextColor(Color::WHITE),
                        Transform::from_translation(Vec3::new(0.0, 0.0, TACTICAL_LABEL_Z))
                            .with_scale(Vec3::splat(label_size)),
                    ));
                })
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
    //! visibility, clustering at the configured zoom) lives
    //! in `tests/behavior/tactical_overlay.rs`.

    use super::*;

    fn src(pos: Vec2, kind: TacticalMarkerKind, owner: SwarmId) -> TacticalSource {
        TacticalSource {
            position: pos,
            kind,
            owner,
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
        assert_eq!(s.label_screen_size, DEFAULT_TACTICAL_LABEL_SCREEN_SIZE);
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
        // Midpoint between show_zoom_threshold (4) and
        // far_merge_zoom (16) is zoom 10, which sits at
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
    fn cluster_label_singular_for_count_one() {
        let c = TacticalCluster {
            position: Vec2::ZERO,
            kind: TacticalMarkerKind::Deposit,
            owner: SwarmId(1),
            count: 1,
        };
        assert_eq!(cluster_label(&c), "Deposit");
    }

    #[test]
    fn cluster_label_plural_with_count() {
        let c = TacticalCluster {
            position: Vec2::ZERO,
            kind: TacticalMarkerKind::Deposit,
            owner: SwarmId(1),
            count: 3,
        };
        assert_eq!(cluster_label(&c), "Deposit x3");
    }

    #[test]
    fn cluster_label_player_base_uses_you() {
        let c = TacticalCluster {
            position: Vec2::ZERO,
            kind: TacticalMarkerKind::PlayerBase,
            owner: SwarmId::PLAYER,
            count: 1,
        };
        assert_eq!(cluster_label(&c), "You");
    }

    #[test]
    fn cluster_label_opponent_base_uses_enemy() {
        let c = TacticalCluster {
            position: Vec2::ZERO,
            kind: TacticalMarkerKind::OpponentBase,
            owner: SwarmId(7),
            count: 1,
        };
        assert_eq!(cluster_label(&c), "Enemy");
    }

    #[test]
    fn cluster_label_covers_every_kind() {
        // Sanity check so a new variant added to
        // TacticalMarkerKind triggers a compile error
        // and an explicit label decision.
        for kind in [
            TacticalMarkerKind::Deposit,
            TacticalMarkerKind::Facility,
            TacticalMarkerKind::Stockpile,
            TacticalMarkerKind::Planned,
            TacticalMarkerKind::Charger,
        ] {
            let c = TacticalCluster {
                position: Vec2::ZERO,
                kind,
                owner: SwarmId(1),
                count: 2,
            };
            let label = cluster_label(&c);
            assert!(!label.is_empty(), "label for {kind:?} must not be empty");
            assert!(
                label.contains('x'),
                "label for {kind:?} must show the count"
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
