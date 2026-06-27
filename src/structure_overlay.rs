//! Zoom-aware world-space fill bars for structures and loaded haulers.
//!
//! Structures always get a small horizontal bar above their sprite. The bar
//! shows buffer fullness for deposits, stockpiles, facilities, chargers, and
//! build progress for planned structures. Haulers get the same treatment only
//! while carrying a [`HaulerLoad`]. Bars live in world space and use the same
//! zoom threshold that the previous structure labels used.

use bevy::{ecs::query::QueryFilter, prelude::*};

use crate::fly_camera::CameraZoom2d;
use crate::nanobot::{
    Charger, HaulerLoad, Nanobot, PlannedStructure, ProductionFacility, BOT_RADIUS,
    DEFAULT_PLANNED_WORK_TICKS, HAULER_CARRY_CAPACITY, PLANNED_STRUCTURE_FOOTPRINT,
};
use crate::resources::{ResourceDeposit, Stockpile};
use crate::GAMEPLAY_SPRITE_Z;

/// Camera zoom value at or above which overlays hide.
pub const DEFAULT_OVERLAY_HIDE_ZOOM_THRESHOLD: f32 = 8.0;

/// Z-translation for overlay backgrounds. Fill children sit slightly above it.
pub const STRUCTURE_OVERLAY_Z: f32 = GAMEPLAY_SPRITE_Z + 1.0;

/// Default deposit radius used by pure offset tests when no component is known.
pub const DEFAULT_DEPOSIT_OVERLAY_RADIUS: f32 = 32.0;

/// Vertical gap between target footprint top and bar centre.
pub const STRUCTURE_FOOTPRINT_LABEL_GAP: f32 = 12.0;

const STRUCTURE_BAR_SIZE: Vec2 = Vec2::new(48.0, 6.0);
const HAULER_BAR_SIZE: Vec2 = Vec2::new(32.0, 4.0);
const HAULER_OVERLAY_GAP: f32 = 8.0;
const FILL_CHILD_Z: f32 = 0.01;

/// Runtime configuration for the overlay system.
#[derive(Debug, Resource, Clone, Copy, PartialEq)]
pub struct StructureOverlaySettings {
    pub hide_zoom_threshold: f32,
}

impl Default for StructureOverlaySettings {
    fn default() -> Self {
        Self {
            hide_zoom_threshold: DEFAULT_OVERLAY_HIDE_ZOOM_THRESHOLD,
        }
    }
}

/// Every fill bar kind the overlay layer knows how to render.
#[derive(Debug, Component, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StructureOverlayKind {
    Deposit,
    Stockpile,
    Facility,
    Planned,
    Charger,
    Hauler,
}

impl StructureOverlayKind {
    /// All kinds in stable declaration order.
    pub const ALL: [StructureOverlayKind; 6] = [
        StructureOverlayKind::Deposit,
        StructureOverlayKind::Stockpile,
        StructureOverlayKind::Facility,
        StructureOverlayKind::Planned,
        StructureOverlayKind::Charger,
        StructureOverlayKind::Hauler,
    ];

    pub const STRUCTURES: [StructureOverlayKind; 5] = [
        StructureOverlayKind::Deposit,
        StructureOverlayKind::Stockpile,
        StructureOverlayKind::Facility,
        StructureOverlayKind::Planned,
        StructureOverlayKind::Charger,
    ];
}

/// Marker on the bar background entity. `fill` is the child entity whose width
/// is updated to match the target's current fullness.
#[derive(Debug, Component, Clone, Copy)]
pub struct StructureOverlay {
    pub target: Entity,
    pub kind: StructureOverlayKind,
    pub fill: Entity,
}

/// Marker for overlay background sprites.
#[derive(Debug, Component, Clone, Copy)]
pub struct StructureOverlayBackground;

/// Marker for overlay fill sprites.
#[derive(Debug, Component, Clone, Copy)]
pub struct StructureOverlayFill;

/// Backwards-compatible alias for callers that name the generic fill overlay.
pub type FillOverlay = StructureOverlay;
pub type FillOverlayKind = StructureOverlayKind;
pub type FillOverlayBackground = StructureOverlayBackground;
pub type FillOverlayFill = StructureOverlayFill;

/// World-space Y offset (positive = above target centre) for a bar of `kind`.
pub fn overlay_label_offset_y(kind: StructureOverlayKind, deposit_radius: Option<f32>) -> f32 {
    let extent = match kind {
        StructureOverlayKind::Deposit => deposit_radius.unwrap_or(DEFAULT_DEPOSIT_OVERLAY_RADIUS),
        StructureOverlayKind::Stockpile
        | StructureOverlayKind::Facility
        | StructureOverlayKind::Planned
        | StructureOverlayKind::Charger => PLANNED_STRUCTURE_FOOTPRINT / 2.0,
        StructureOverlayKind::Hauler => BOT_RADIUS,
    };
    let gap = match kind {
        StructureOverlayKind::Hauler => HAULER_OVERLAY_GAP,
        _ => STRUCTURE_FOOTPRINT_LABEL_GAP,
    };
    extent + gap
}

/// Fraction helper for amount/capacity-style buffers.
pub fn fill_fraction(amount: u32, capacity: u32) -> f32 {
    if capacity == 0 {
        return 0.0;
    }
    (amount as f32 / capacity as f32).clamp(0.0, 1.0)
}

/// Planned-structure build progress as a `0.0..=1.0` fraction.
pub fn planned_fill_fraction(planned: &PlannedStructure) -> f32 {
    fill_fraction(
        DEFAULT_PLANNED_WORK_TICKS.saturating_sub(planned.work_remaining),
        DEFAULT_PLANNED_WORK_TICKS,
    )
}

/// World-space bar size for each overlay kind.
pub fn overlay_bar_size(kind: StructureOverlayKind) -> Vec2 {
    match kind {
        StructureOverlayKind::Hauler => HAULER_BAR_SIZE,
        _ => STRUCTURE_BAR_SIZE,
    }
}

/// Dark backing panel shared by every bar.
pub fn overlay_background_color(_kind: StructureOverlayKind) -> Color {
    Color::srgba(0.0, 0.0, 0.0, 0.65)
}

/// Fill color for each kind.
pub fn overlay_fill_color(kind: StructureOverlayKind) -> Color {
    match kind {
        StructureOverlayKind::Deposit => Color::srgb(1.0, 0.68, 0.20),
        StructureOverlayKind::Stockpile => Color::srgb(0.25, 0.85, 0.35),
        StructureOverlayKind::Facility => Color::srgb(0.25, 0.55, 1.0),
        StructureOverlayKind::Planned => Color::srgb(0.85, 0.85, 0.90),
        StructureOverlayKind::Charger => Color::srgb(0.75, 0.35, 1.0),
        StructureOverlayKind::Hauler => Color::srgb(0.25, 0.90, 1.0),
    }
}

/// Decide overlay visibility for a camera zoom and configured threshold.
pub fn overlay_visibility_for_zoom(zoom: f32, threshold: f32) -> Visibility {
    if threshold == f32::INFINITY {
        return Visibility::Hidden;
    }
    if threshold <= 0.0 {
        return Visibility::Inherited;
    }
    if zoom >= threshold {
        Visibility::Hidden
    } else {
        Visibility::Inherited
    }
}

/// First camera zoom, or fallback when no camera exists.
pub fn effective_zoom<'a, I>(zoom_iter: I, fallback_zoom: f32) -> f32
where
    I: IntoIterator<Item = &'a CameraZoom2d>,
{
    zoom_iter
        .into_iter()
        .next()
        .map(|z| z.zoom)
        .unwrap_or(fallback_zoom)
}

/// Plugin wiring for fill bars.
pub struct StructureOverlayPlugin;

impl Plugin for StructureOverlayPlugin {
    fn build(&self, app: &mut App) {
        if !app.world().contains_resource::<StructureOverlaySettings>() {
            app.init_resource::<StructureOverlaySettings>();
        }
        app.add_systems(
            Update,
            (
                structure_overlay_spawn_system,
                structure_overlay_update_system,
                structure_overlay_visibility_system,
                structure_overlay_cleanup_system,
            )
                .chain(),
        );
    }
}

#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub fn structure_overlay_spawn_system(
    mut commands: Commands,
    deposits: Query<Entity, With<ResourceDeposit>>,
    stockpiles: Query<Entity, With<Stockpile>>,
    facilities: Query<Entity, With<ProductionFacility>>,
    planned: Query<Entity, With<PlannedStructure>>,
    chargers: Query<Entity, With<Charger>>,
    loaded_haulers: Query<Entity, (With<Nanobot>, With<HaulerLoad>)>,
    existing: Query<&StructureOverlay>,
) {
    let covered: std::collections::HashSet<Entity> = existing.iter().map(|o| o.target).collect();
    spawn_missing(
        &mut commands,
        &covered,
        &deposits,
        StructureOverlayKind::Deposit,
    );
    spawn_missing(
        &mut commands,
        &covered,
        &stockpiles,
        StructureOverlayKind::Stockpile,
    );
    spawn_missing(
        &mut commands,
        &covered,
        &facilities,
        StructureOverlayKind::Facility,
    );
    spawn_missing(
        &mut commands,
        &covered,
        &planned,
        StructureOverlayKind::Planned,
    );
    spawn_missing(
        &mut commands,
        &covered,
        &chargers,
        StructureOverlayKind::Charger,
    );
    spawn_missing(
        &mut commands,
        &covered,
        &loaded_haulers,
        StructureOverlayKind::Hauler,
    );
}

fn spawn_missing(
    commands: &mut Commands,
    covered: &std::collections::HashSet<Entity>,
    targets: &Query<Entity, impl QueryFilter>,
    kind: StructureOverlayKind,
) {
    for entity in targets {
        if !covered.contains(&entity) {
            spawn_overlay_for(commands, entity, kind);
        }
    }
}

fn spawn_overlay_for(commands: &mut Commands, target: Entity, kind: StructureOverlayKind) {
    let size = overlay_bar_size(kind);
    let fill = commands
        .spawn((
            StructureOverlayFill,
            Sprite {
                color: overlay_fill_color(kind),
                custom_size: Some(Vec2::new(0.0, size.y)),
                ..default()
            },
            Transform::from_translation(Vec3::new(-size.x / 2.0, 0.0, FILL_CHILD_Z)),
            Visibility::Inherited,
        ))
        .id();

    let background = commands
        .spawn((
            StructureOverlay { target, kind, fill },
            StructureOverlayBackground,
            Sprite {
                color: overlay_background_color(kind),
                custom_size: Some(size),
                ..default()
            },
            Transform::from_translation(Vec3::new(0.0, 0.0, STRUCTURE_OVERLAY_Z)),
            Visibility::Inherited,
        ))
        .id();

    commands.entity(background).add_child(fill);
}

#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub fn structure_overlay_update_system(
    mut overlays: Query<(&StructureOverlay, &mut Transform), Without<StructureOverlayFill>>,
    mut fills: Query<
        (&mut Sprite, &mut Transform),
        (With<StructureOverlayFill>, Without<StructureOverlay>),
    >,
    deposits: Query<&ResourceDeposit, Without<StructureOverlay>>,
    stockpiles: Query<&Stockpile, Without<StructureOverlay>>,
    facilities: Query<&ProductionFacility, Without<StructureOverlay>>,
    planned: Query<&PlannedStructure, Without<StructureOverlay>>,
    chargers: Query<&Charger, Without<StructureOverlay>>,
    haulers: Query<&HaulerLoad, Without<StructureOverlay>>,
    target_transforms: Query<
        &Transform,
        (Without<StructureOverlay>, Without<StructureOverlayFill>),
    >,
) {
    for (overlay, mut transform) in &mut overlays {
        let Ok(target_pos) = target_transforms
            .get(overlay.target)
            .map(|t| t.translation.truncate())
        else {
            continue;
        };

        let deposit_radius = deposits.get(overlay.target).ok().map(|d| d.radius);
        let offset_y = overlay_label_offset_y(overlay.kind, deposit_radius);
        transform.translation = (target_pos + Vec2::new(0.0, offset_y)).extend(STRUCTURE_OVERLAY_Z);

        let fraction = compute_fill_fraction(
            overlay.kind,
            &deposits,
            &stockpiles,
            &facilities,
            &planned,
            &chargers,
            &haulers,
            overlay.target,
        );
        update_fill_sprite(overlay.kind, fraction, overlay.fill, &mut fills);
    }
}

#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn compute_fill_fraction(
    kind: StructureOverlayKind,
    deposits: &Query<&ResourceDeposit, Without<StructureOverlay>>,
    stockpiles: &Query<&Stockpile, Without<StructureOverlay>>,
    facilities: &Query<&ProductionFacility, Without<StructureOverlay>>,
    planned: &Query<&PlannedStructure, Without<StructureOverlay>>,
    chargers: &Query<&Charger, Without<StructureOverlay>>,
    haulers: &Query<&HaulerLoad, Without<StructureOverlay>>,
    target: Entity,
) -> f32 {
    match kind {
        StructureOverlayKind::Deposit => deposits
            .get(target)
            .map(|d| fill_fraction(d.amount, d.capacity))
            .unwrap_or(0.0),
        StructureOverlayKind::Stockpile => stockpiles
            .get(target)
            .map(|s| fill_fraction(s.amount, s.capacity))
            .unwrap_or(0.0),
        StructureOverlayKind::Facility => facilities
            .get(target)
            .map(|f| fill_fraction(f.input_amount, f.input_capacity))
            .unwrap_or(0.0),
        StructureOverlayKind::Planned => planned
            .get(target)
            .map(planned_fill_fraction)
            .unwrap_or(0.0),
        StructureOverlayKind::Charger => chargers
            .get(target)
            .map(|c| fill_fraction(c.amount, c.capacity))
            .unwrap_or(0.0),
        StructureOverlayKind::Hauler => haulers
            .get(target)
            .map(|l| fill_fraction(l.amount, HAULER_CARRY_CAPACITY))
            .unwrap_or(0.0),
    }
}

#[allow(clippy::type_complexity)]
fn update_fill_sprite(
    kind: StructureOverlayKind,
    fraction: f32,
    fill: Entity,
    fills: &mut Query<
        (&mut Sprite, &mut Transform),
        (With<StructureOverlayFill>, Without<StructureOverlay>),
    >,
) {
    let Ok((mut sprite, mut transform)) = fills.get_mut(fill) else {
        return;
    };
    let size = overlay_bar_size(kind);
    let fill_width = size.x * fraction.clamp(0.0, 1.0);
    sprite.custom_size = Some(Vec2::new(fill_width, size.y));
    sprite.color = overlay_fill_color(kind);
    transform.translation = Vec3::new(-(size.x - fill_width) / 2.0, 0.0, FILL_CHILD_Z);
}

pub fn structure_overlay_visibility_system(
    settings: Res<StructureOverlaySettings>,
    zoom_query: Query<&CameraZoom2d>,
    mut overlays: Query<&mut Visibility, With<StructureOverlay>>,
) {
    let zoom = effective_zoom(zoom_query.iter(), 1.0);
    let target = overlay_visibility_for_zoom(zoom, settings.hide_zoom_threshold);
    for mut visibility in &mut overlays {
        if *visibility != target {
            *visibility = target;
        }
    }
}

pub fn structure_overlay_cleanup_system(
    mut commands: Commands,
    overlays: Query<(Entity, &StructureOverlay)>,
    targets: Query<Entity>,
    loaded_haulers: Query<(), With<HaulerLoad>>,
) {
    for (overlay_entity, overlay) in &overlays {
        let target_gone = targets.get(overlay.target).is_err();
        let empty_hauler = overlay.kind == StructureOverlayKind::Hauler
            && loaded_haulers.get(overlay.target).is_err();
        if target_gone || empty_hauler {
            commands.entity(overlay_entity).despawn();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nanobot::{PlannedKind, DEFAULT_PLANNED_WORK_TICKS};

    #[test]
    fn fill_fraction_clamps_and_handles_zero_capacity() {
        assert_eq!(fill_fraction(0, 100), 0.0);
        assert_eq!(fill_fraction(50, 100), 0.5);
        assert_eq!(fill_fraction(150, 100), 1.0);
        assert_eq!(fill_fraction(10, 0), 0.0);
    }

    #[test]
    fn planned_fill_fraction_uses_spent_work_budget() {
        let mut planned = PlannedStructure::new(PlannedKind::SinkStockpile, IVec2::ZERO);
        planned.work_remaining = DEFAULT_PLANNED_WORK_TICKS;
        assert_eq!(planned_fill_fraction(&planned), 0.0);
        planned.work_remaining = DEFAULT_PLANNED_WORK_TICKS / 2;
        assert!(planned_fill_fraction(&planned) > 0.0);
        planned.work_remaining = 0;
        assert_eq!(planned_fill_fraction(&planned), 1.0);
    }

    #[test]
    fn overlay_bar_size_is_smaller_for_haulers() {
        assert_eq!(
            overlay_bar_size(StructureOverlayKind::Stockpile),
            STRUCTURE_BAR_SIZE
        );
        assert_eq!(
            overlay_bar_size(StructureOverlayKind::Hauler),
            HAULER_BAR_SIZE
        );
    }

    #[test]
    fn fill_colors_are_distinct_per_kind() {
        let colors: Vec<Color> = StructureOverlayKind::ALL
            .iter()
            .map(|k| overlay_fill_color(*k))
            .collect();
        for i in 0..colors.len() {
            for j in (i + 1)..colors.len() {
                assert_ne!(colors[i], colors[j]);
            }
        }
    }

    #[test]
    fn overlay_visibility_hides_only_at_or_above_threshold() {
        assert_eq!(overlay_visibility_for_zoom(1.0, 8.0), Visibility::Inherited);
        assert_eq!(
            overlay_visibility_for_zoom(7.99, 8.0),
            Visibility::Inherited
        );
        assert_eq!(overlay_visibility_for_zoom(8.0, 8.0), Visibility::Hidden);
        assert_eq!(overlay_visibility_for_zoom(10.0, 8.0), Visibility::Hidden);
        assert_eq!(
            overlay_visibility_for_zoom(f32::INFINITY, 0.0),
            Visibility::Inherited
        );
        assert_eq!(
            overlay_visibility_for_zoom(1.0, f32::INFINITY),
            Visibility::Hidden
        );
    }

    #[test]
    fn effective_zoom_falls_back_or_reads_first_camera_zoom() {
        assert_eq!(
            effective_zoom(std::iter::empty::<&CameraZoom2d>(), 1.0),
            1.0
        );
        let cameras = [
            CameraZoom2d {
                zoom: 2.5,
                ..default()
            },
            CameraZoom2d {
                zoom: 7.5,
                ..default()
            },
        ];
        assert_eq!(effective_zoom(cameras.iter(), 1.0), 2.5);
    }

    #[test]
    fn default_settings_use_default_threshold() {
        assert_eq!(
            StructureOverlaySettings::default().hide_zoom_threshold,
            DEFAULT_OVERLAY_HIDE_ZOOM_THRESHOLD
        );
    }

    #[test]
    fn overlay_offsets_are_above_targets() {
        assert_eq!(
            overlay_label_offset_y(StructureOverlayKind::Deposit, Some(64.0)),
            64.0 + STRUCTURE_FOOTPRINT_LABEL_GAP
        );
        assert_eq!(
            overlay_label_offset_y(StructureOverlayKind::Deposit, None),
            DEFAULT_DEPOSIT_OVERLAY_RADIUS + STRUCTURE_FOOTPRINT_LABEL_GAP
        );
        let structure_offset = PLANNED_STRUCTURE_FOOTPRINT / 2.0 + STRUCTURE_FOOTPRINT_LABEL_GAP;
        for kind in StructureOverlayKind::STRUCTURES {
            let offset = overlay_label_offset_y(kind, Some(9999.0));
            assert!(offset > 0.0);
            if kind != StructureOverlayKind::Deposit {
                assert_eq!(offset, structure_offset);
            }
        }
        assert_eq!(
            overlay_label_offset_y(StructureOverlayKind::Hauler, None),
            BOT_RADIUS + HAULER_OVERLAY_GAP
        );
    }
}
