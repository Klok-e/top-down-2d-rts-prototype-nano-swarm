//! Zoom-aware world-space logistics, condition, and work indicators.
//!
//! Structures show physical buffer amounts plus reservation state. Cargo bars
//! appear above Workers and Haulers only while cargo exists or a source transfer
//! is active. Completed support structures also show maintenance reserve and
//! health, while active maintenance Workers show shift progress. Every segment
//! reads live ECS state each update.

use bevy::{ecs::query::QueryFilter, prelude::*};

use crate::GAMEPLAY_SPRITE_Z;
use crate::fly_camera::CameraZoom2d;
use crate::nanobot::{
    BOT_RADIUS, Cargo, Charger, ExtractProgress, HaulerLoading, LogisticsReservation,
    MaintenanceProgress, Nanobot, NanobotType, PLANNED_STRUCTURE_FOOTPRINT, PlannedStructure,
    ProductionFacility, SUPPORT_OPERATIONAL_HEALTH_THRESHOLD, Structure,
};
use crate::resources::{ResourceDeposit, Stockpile};
use crate::tuning::{PresentationTuning, Tuning};

/// Z-translation for overlay backgrounds. Fill children sit slightly above it.
pub const STRUCTURE_OVERLAY_Z: f32 = GAMEPLAY_SPRITE_Z + 1.0;

/// Default deposit radius used by pure offset tests when no component is known.
pub const DEFAULT_DEPOSIT_OVERLAY_RADIUS: f32 = 32.0;

const FILL_CHILD_Z: f32 = 0.01;

/// Every fill bar kind the overlay layer knows how to render.
#[derive(Debug, Component, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StructureOverlayKind {
    Deposit,
    Stockpile,
    Facility,
    Planned,
    Charger,
    Worker,
    Hauler,
}

impl StructureOverlayKind {
    /// All kinds in stable declaration order.
    pub const ALL: [StructureOverlayKind; 7] = [
        StructureOverlayKind::Deposit,
        StructureOverlayKind::Stockpile,
        StructureOverlayKind::Facility,
        StructureOverlayKind::Planned,
        StructureOverlayKind::Charger,
        StructureOverlayKind::Worker,
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
    /// Physical-amount segment; retained as `fill` for API compatibility.
    pub fill: Entity,
    pub outgoing_reserved: Entity,
    pub incoming_reserved: Entity,
}

/// Marker for overlay background sprites.
#[derive(Debug, Component, Clone, Copy)]
pub struct StructureOverlayBackground;

/// Marker for overlay fill sprites.
#[derive(Debug, Component, Clone, Copy)]
pub struct StructureOverlayFill;

/// Maintenance, health, or active Worker progress bar.
#[derive(Debug, Component, Clone, Copy)]
pub struct ConditionOverlay {
    pub target: Entity,
    pub kind: ConditionOverlayKind,
    pub fill: Entity,
}

#[derive(Debug, Component, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConditionOverlayKind {
    Maintenance,
    Health,
    WorkerProgress,
}

#[derive(Debug, Component, Clone, Copy)]
pub struct ConditionOverlayBackground;

#[derive(Debug, Component, Clone, Copy)]
pub struct ConditionOverlayFill;

/// Semantic segment type used by deterministic rendering assertions.
#[derive(Debug, Component, Clone, Copy, PartialEq, Eq)]
pub struct StructureOverlaySegment {
    pub kind: StructureOverlaySegmentKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StructureOverlaySegmentKind {
    Physical,
    OutgoingReserved,
    IncomingReserved,
}

/// Backwards-compatible alias for callers that name the generic fill overlay.
pub type FillOverlay = StructureOverlay;
pub type FillOverlayKind = StructureOverlayKind;
pub type FillOverlayBackground = StructureOverlayBackground;
pub type FillOverlayFill = StructureOverlayFill;

/// World-space Y offset (positive = above target centre) for a bar of `kind`.
pub fn overlay_label_offset_y(
    kind: StructureOverlayKind,
    deposit_radius: Option<f32>,
    settings: &PresentationTuning,
) -> f32 {
    let extent = match kind {
        StructureOverlayKind::Deposit => deposit_radius.unwrap_or(DEFAULT_DEPOSIT_OVERLAY_RADIUS),
        StructureOverlayKind::Stockpile
        | StructureOverlayKind::Facility
        | StructureOverlayKind::Planned
        | StructureOverlayKind::Charger => PLANNED_STRUCTURE_FOOTPRINT / 2.0,
        StructureOverlayKind::Worker | StructureOverlayKind::Hauler => BOT_RADIUS,
    };
    let gap = match kind {
        StructureOverlayKind::Worker | StructureOverlayKind::Hauler => settings.cargo_label_gap,
        _ => settings.structure_label_gap,
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
    let (completed, total) = planned.construction_progress();
    fill_fraction(completed, total)
}

pub fn maintenance_fill_fraction(ticks_since_maintained: u32, tuning: &Tuning) -> f32 {
    1.0 - fill_fraction(ticks_since_maintained, tuning.maintenance.buffer_ticks)
}

pub fn health_fill_fraction(health: u32, tuning: &Tuning) -> f32 {
    fill_fraction(health, tuning.construction.max_health)
}

pub fn condition_fill_color(kind: ConditionOverlayKind, value: u32, tuning: &Tuning) -> Color {
    match kind {
        ConditionOverlayKind::Maintenance if value >= tuning.maintenance.buffer_ticks => {
            Color::srgb(1.0, 0.22, 0.18)
        }
        ConditionOverlayKind::Maintenance if value >= tuning.maintenance.needs_threshold => {
            Color::srgb(1.0, 0.68, 0.20)
        }
        ConditionOverlayKind::Maintenance => Color::srgb(0.25, 0.85, 0.35),
        ConditionOverlayKind::Health if value < SUPPORT_OPERATIONAL_HEALTH_THRESHOLD => {
            Color::srgb(1.0, 0.22, 0.18)
        }
        ConditionOverlayKind::Health if value < tuning.construction.max_health => {
            Color::srgb(1.0, 0.68, 0.20)
        }
        ConditionOverlayKind::Health => Color::srgb(0.25, 0.85, 0.35),
        ConditionOverlayKind::WorkerProgress => Color::WHITE,
    }
}

/// World-space bar size for each overlay kind.
pub fn overlay_bar_size(kind: StructureOverlayKind, settings: &PresentationTuning) -> Vec2 {
    match kind {
        StructureOverlayKind::Worker | StructureOverlayKind::Hauler => {
            Vec2::new(settings.cargo_bar_width, settings.cargo_bar_height)
        }
        _ => Vec2::new(settings.structure_bar_width, settings.structure_bar_height),
    }
}

/// Dark backing panel shared by every bar.
pub fn overlay_background_color(_kind: StructureOverlayKind) -> Color {
    Color::srgba(0.0, 0.0, 0.0, 0.65)
}

/// Fill color for each kind.
pub fn overlay_fill_color(kind: StructureOverlayKind) -> Color {
    match kind {
        StructureOverlayKind::Deposit => Color::srgb(0.18, 0.88, 0.96),
        StructureOverlayKind::Stockpile => Color::srgb(0.25, 0.85, 0.35),
        StructureOverlayKind::Facility => Color::srgb(0.25, 0.55, 1.0),
        StructureOverlayKind::Planned => Color::srgb(0.85, 0.85, 0.90),
        StructureOverlayKind::Charger => Color::srgb(0.75, 0.35, 1.0),
        StructureOverlayKind::Worker => Color::srgb(0.35, 1.0, 0.65),
        StructureOverlayKind::Hauler => Color::srgb(0.25, 0.90, 1.0),
    }
}

/// Stable colors make reservation direction legible across structure kinds.
pub fn reservation_segment_color(kind: StructureOverlaySegmentKind) -> Color {
    match kind {
        StructureOverlaySegmentKind::Physical => Color::WHITE,
        StructureOverlaySegmentKind::OutgoingReserved => Color::srgb(1.0, 0.42, 0.12),
        StructureOverlaySegmentKind::IncomingReserved => Color::srgb(0.20, 0.95, 1.0),
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
        app.init_resource::<Tuning>();
        app.add_systems(
            Update,
            (
                structure_overlay_spawn_system,
                condition_overlay_spawn_system,
                structure_overlay_update_system,
                condition_overlay_update_system,
                structure_overlay_visibility_system,
                structure_overlay_cleanup_system,
                condition_overlay_cleanup_system,
            )
                .chain(),
        );
    }
}

#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub fn structure_overlay_spawn_system(
    tuning: Res<Tuning>,
    mut commands: Commands,
    deposits: Query<Entity, With<ResourceDeposit>>,
    stockpiles: Query<Entity, With<Stockpile>>,
    facilities: Query<Entity, With<ProductionFacility>>,
    planned: Query<Entity, With<PlannedStructure>>,
    chargers: Query<Entity, With<Charger>>,
    cargo_bots: Query<
        (
            Entity,
            &Cargo,
            &NanobotType,
            Option<&ExtractProgress>,
            Option<&HaulerLoading>,
        ),
        With<Nanobot>,
    >,
    existing: Query<(Entity, &StructureOverlay)>,
) {
    let settings = &tuning.presentation;
    let mut covered = std::collections::HashSet::new();
    for (overlay_entity, overlay) in &existing {
        let actual = actual_overlay_kind(
            overlay.target,
            &deposits,
            &stockpiles,
            &facilities,
            &planned,
            &chargers,
            &cargo_bots,
        );
        if actual == Some(overlay.kind) {
            covered.insert(overlay.target);
        } else {
            commands.entity(overlay_entity).despawn();
        }
    }
    spawn_missing(
        &mut commands,
        &covered,
        &deposits,
        StructureOverlayKind::Deposit,
        settings,
    );
    spawn_missing(
        &mut commands,
        &covered,
        &stockpiles,
        StructureOverlayKind::Stockpile,
        settings,
    );
    spawn_missing(
        &mut commands,
        &covered,
        &facilities,
        StructureOverlayKind::Facility,
        settings,
    );
    spawn_missing(
        &mut commands,
        &covered,
        &planned,
        StructureOverlayKind::Planned,
        settings,
    );
    spawn_missing(
        &mut commands,
        &covered,
        &chargers,
        StructureOverlayKind::Charger,
        settings,
    );
    for (entity, cargo, bot_type, extracting, loading) in &cargo_bots {
        if covered.contains(&entity)
            || (cargo.amount == 0 && extracting.is_none() && loading.is_none())
        {
            continue;
        }
        let kind = match bot_type {
            NanobotType::Worker => StructureOverlayKind::Worker,
            NanobotType::Hauler => StructureOverlayKind::Hauler,
            _ => continue,
        };
        spawn_overlay_for(&mut commands, entity, kind, settings);
    }
}

/// Resolve an entity's current overlay kind by probing each
/// structure component in turn. Returns `None` when the entity
/// carries none of the tracked structures (it has been promoted
/// to something the overlay layer does not render, or despawned).
#[allow(clippy::type_complexity)]
fn actual_overlay_kind(
    target: Entity,
    deposits: &Query<Entity, With<ResourceDeposit>>,
    stockpiles: &Query<Entity, With<Stockpile>>,
    facilities: &Query<Entity, With<ProductionFacility>>,
    planned: &Query<Entity, With<PlannedStructure>>,
    chargers: &Query<Entity, With<Charger>>,
    cargo_bots: &Query<
        (
            Entity,
            &Cargo,
            &NanobotType,
            Option<&ExtractProgress>,
            Option<&HaulerLoading>,
        ),
        With<Nanobot>,
    >,
) -> Option<StructureOverlayKind> {
    if deposits.get(target).is_ok() {
        Some(StructureOverlayKind::Deposit)
    } else if stockpiles.get(target).is_ok() {
        Some(StructureOverlayKind::Stockpile)
    } else if facilities.get(target).is_ok() {
        Some(StructureOverlayKind::Facility)
    } else if planned.get(target).is_ok() {
        Some(StructureOverlayKind::Planned)
    } else if chargers.get(target).is_ok() {
        Some(StructureOverlayKind::Charger)
    } else if let Ok((_, cargo, bot_type, extracting, loading)) = cargo_bots.get(target) {
        if cargo.amount == 0 && extracting.is_none() && loading.is_none() {
            None
        } else {
            match bot_type {
                NanobotType::Worker => Some(StructureOverlayKind::Worker),
                NanobotType::Hauler => Some(StructureOverlayKind::Hauler),
                _ => None,
            }
        }
    } else {
        None
    }
}

fn spawn_missing(
    commands: &mut Commands,
    covered: &std::collections::HashSet<Entity>,
    targets: &Query<Entity, impl QueryFilter>,
    kind: StructureOverlayKind,
    settings: &PresentationTuning,
) {
    for entity in targets {
        if !covered.contains(&entity) {
            spawn_overlay_for(commands, entity, kind, settings);
        }
    }
}

fn spawn_overlay_for(
    commands: &mut Commands,
    target: Entity,
    kind: StructureOverlayKind,
    settings: &PresentationTuning,
) {
    let size = overlay_bar_size(kind, settings);
    let spawn_segment = |commands: &mut Commands, segment_kind| {
        commands
            .spawn((
                StructureOverlayFill,
                StructureOverlaySegment { kind: segment_kind },
                Sprite {
                    color: match segment_kind {
                        StructureOverlaySegmentKind::Physical => overlay_fill_color(kind),
                        _ => reservation_segment_color(segment_kind),
                    },
                    custom_size: Some(Vec2::new(0.0, size.y)),
                    ..default()
                },
                Transform::from_translation(Vec3::new(-size.x / 2.0, 0.0, FILL_CHILD_Z)),
                Visibility::Inherited,
            ))
            .id()
    };
    let fill = spawn_segment(commands, StructureOverlaySegmentKind::Physical);
    let outgoing_reserved = spawn_segment(commands, StructureOverlaySegmentKind::OutgoingReserved);
    let incoming_reserved = spawn_segment(commands, StructureOverlaySegmentKind::IncomingReserved);

    let background = commands
        .spawn((
            StructureOverlay {
                target,
                kind,
                fill,
                outgoing_reserved,
                incoming_reserved,
            },
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

    commands
        .entity(background)
        .add_children(&[fill, outgoing_reserved, incoming_reserved]);
}

#[allow(clippy::type_complexity)]
pub fn condition_overlay_spawn_system(
    tuning: Res<Tuning>,
    mut commands: Commands,
    support_targets: Query<
        Entity,
        (
            With<Structure>,
            Or<(With<Stockpile>, With<ProductionFacility>, With<Charger>)>,
        ),
    >,
    maintenance_workers: Query<
        Entity,
        (
            With<Nanobot>,
            With<MaintenanceProgress>,
            Without<ConditionOverlay>,
        ),
    >,
    existing: Query<&ConditionOverlay>,
) {
    let covered: std::collections::HashSet<(Entity, ConditionOverlayKind)> = existing
        .iter()
        .map(|overlay| (overlay.target, overlay.kind))
        .collect();
    for target in &support_targets {
        for kind in [
            ConditionOverlayKind::Maintenance,
            ConditionOverlayKind::Health,
        ] {
            if !covered.contains(&(target, kind)) {
                spawn_condition_overlay(&mut commands, target, kind, &tuning);
            }
        }
    }
    for target in &maintenance_workers {
        let kind = ConditionOverlayKind::WorkerProgress;
        if !covered.contains(&(target, kind)) {
            spawn_condition_overlay(&mut commands, target, kind, &tuning);
        }
    }
}

fn spawn_condition_overlay(
    commands: &mut Commands,
    target: Entity,
    kind: ConditionOverlayKind,
    tuning: &Tuning,
) {
    let settings = &tuning.presentation;
    let size = condition_bar_size(kind, settings);
    let fill = commands
        .spawn((
            ConditionOverlayFill,
            Sprite {
                color: condition_fill_color(kind, 0, tuning),
                custom_size: Some(Vec2::new(0.0, size.y)),
                ..default()
            },
            Transform::from_translation(Vec3::new(-size.x / 2.0, 0.0, FILL_CHILD_Z)),
            Visibility::Inherited,
        ))
        .id();
    let background = commands
        .spawn((
            ConditionOverlay { target, kind, fill },
            ConditionOverlayBackground,
            Sprite {
                color: overlay_background_color(StructureOverlayKind::Facility),
                custom_size: Some(size),
                ..default()
            },
            Transform::from_translation(Vec3::new(0.0, 0.0, STRUCTURE_OVERLAY_Z)),
            Visibility::Inherited,
        ))
        .id();
    commands.entity(background).add_children(&[fill]);
}

fn condition_bar_size(kind: ConditionOverlayKind, settings: &PresentationTuning) -> Vec2 {
    match kind {
        ConditionOverlayKind::Maintenance | ConditionOverlayKind::Health => {
            Vec2::new(settings.condition_bar_width, settings.condition_bar_height)
        }
        ConditionOverlayKind::WorkerProgress => {
            Vec2::new(settings.cargo_bar_width, settings.cargo_bar_height)
        }
    }
}

fn condition_overlay_offset_y(kind: ConditionOverlayKind, settings: &PresentationTuning) -> f32 {
    let resource_y = PLANNED_STRUCTURE_FOOTPRINT / 2.0 + settings.structure_label_gap;
    let maintenance_y = resource_y
        + settings.structure_bar_height / 2.0
        + settings.condition_bar_gap
        + settings.condition_bar_height / 2.0;
    match kind {
        ConditionOverlayKind::Maintenance => maintenance_y,
        ConditionOverlayKind::Health => {
            maintenance_y + settings.condition_bar_height + settings.condition_bar_gap
        }
        ConditionOverlayKind::WorkerProgress => BOT_RADIUS + settings.cargo_label_gap,
    }
}

#[allow(clippy::type_complexity)]
pub fn condition_overlay_update_system(
    tuning: Res<Tuning>,
    mut overlays: Query<
        (&ConditionOverlay, &mut Transform, &mut Sprite),
        Without<ConditionOverlayFill>,
    >,
    mut fills: Query<
        (&mut Sprite, &mut Transform),
        (With<ConditionOverlayFill>, Without<ConditionOverlay>),
    >,
    conditions: Query<&Structure, Without<ConditionOverlay>>,
    maintenance_workers: Query<&MaintenanceProgress, Without<ConditionOverlay>>,
    target_transforms: Query<
        &Transform,
        (Without<ConditionOverlay>, Without<ConditionOverlayFill>),
    >,
) {
    let settings = &tuning.presentation;
    for (overlay, mut transform, mut background) in &mut overlays {
        background.custom_size = Some(condition_bar_size(overlay.kind, settings));
        let Ok(target_transform) = target_transforms.get(overlay.target) else {
            continue;
        };
        transform.translation = (target_transform.translation.truncate()
            + Vec2::Y * condition_overlay_offset_y(overlay.kind, settings))
        .extend(STRUCTURE_OVERLAY_Z);
        let (fraction, value) = match overlay.kind {
            ConditionOverlayKind::Maintenance => conditions
                .get(overlay.target)
                .map(|condition| {
                    (
                        maintenance_fill_fraction(condition.ticks_since_maintained, &tuning),
                        condition.ticks_since_maintained,
                    )
                })
                .unwrap_or_default(),
            ConditionOverlayKind::Health => conditions
                .get(overlay.target)
                .map(|condition| {
                    (
                        health_fill_fraction(condition.health, &tuning),
                        condition.health,
                    )
                })
                .unwrap_or_default(),
            ConditionOverlayKind::WorkerProgress => maintenance_workers
                .get(overlay.target)
                .map(|progress| {
                    (
                        fill_fraction(progress.ticks_worked, tuning.maintenance.work_ticks),
                        progress.ticks_worked,
                    )
                })
                .unwrap_or_default(),
        };
        let Ok((mut sprite, mut fill_transform)) = fills.get_mut(overlay.fill) else {
            continue;
        };
        let size = condition_bar_size(overlay.kind, settings);
        let width = size.x * fraction;
        sprite.custom_size = Some(Vec2::new(width, size.y));
        sprite.color = condition_fill_color(overlay.kind, value, &tuning);
        fill_transform.translation = Vec3::new(-size.x / 2.0 + width / 2.0, 0.0, FILL_CHILD_Z);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct OverlayAmounts {
    physical: u32,
    capacity: u32,
    outgoing_reserved: u32,
    incoming_reserved: u32,
}

#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub fn structure_overlay_update_system(
    tuning: Res<Tuning>,
    mut overlays: Query<
        (&StructureOverlay, &mut Transform, &mut Sprite),
        Without<StructureOverlayFill>,
    >,
    mut fills: Query<
        (&StructureOverlaySegment, &mut Sprite, &mut Transform),
        (With<StructureOverlayFill>, Without<StructureOverlay>),
    >,
    deposits: Query<&ResourceDeposit, Without<StructureOverlay>>,
    stockpiles: Query<&Stockpile, Without<StructureOverlay>>,
    facilities: Query<&ProductionFacility, Without<StructureOverlay>>,
    planned: Query<&PlannedStructure, Without<StructureOverlay>>,
    chargers: Query<&Charger, Without<StructureOverlay>>,
    cargo: Query<&Cargo, Without<StructureOverlay>>,
    reservations: Query<&LogisticsReservation>,
    target_transforms: Query<
        &Transform,
        (Without<StructureOverlay>, Without<StructureOverlayFill>),
    >,
) {
    let settings = &tuning.presentation;
    for (overlay, mut transform, mut background) in &mut overlays {
        background.custom_size = Some(overlay_bar_size(overlay.kind, settings));
        let Ok(target_transform) = target_transforms.get(overlay.target) else {
            continue;
        };
        let deposit_radius = deposits.get(overlay.target).ok().map(|d| d.radius);
        let mut offset_y = overlay_label_offset_y(overlay.kind, deposit_radius, settings);
        if matches!(
            overlay.kind,
            StructureOverlayKind::Stockpile
                | StructureOverlayKind::Facility
                | StructureOverlayKind::Planned
                | StructureOverlayKind::Charger
        ) {
            offset_y += PLANNED_STRUCTURE_FOOTPRINT / 2.0 * (target_transform.scale.y.abs() - 1.0);
        }
        transform.translation = (target_transform.translation.truncate()
            + Vec2::new(0.0, offset_y))
        .extend(STRUCTURE_OVERLAY_Z);

        let amounts = compute_overlay_amounts(
            overlay.kind,
            &deposits,
            &stockpiles,
            &facilities,
            &planned,
            &chargers,
            &cargo,
            &reservations,
            &tuning,
            overlay.target,
        );
        update_segment_sprite(overlay.kind, amounts, settings, overlay.fill, &mut fills);
        update_segment_sprite(
            overlay.kind,
            amounts,
            settings,
            overlay.outgoing_reserved,
            &mut fills,
        );
        update_segment_sprite(
            overlay.kind,
            amounts,
            settings,
            overlay.incoming_reserved,
            &mut fills,
        );
    }
}

#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn compute_overlay_amounts(
    kind: StructureOverlayKind,
    deposits: &Query<&ResourceDeposit, Without<StructureOverlay>>,
    stockpiles: &Query<&Stockpile, Without<StructureOverlay>>,
    facilities: &Query<&ProductionFacility, Without<StructureOverlay>>,
    planned: &Query<&PlannedStructure, Without<StructureOverlay>>,
    chargers: &Query<&Charger, Without<StructureOverlay>>,
    cargo: &Query<&Cargo, Without<StructureOverlay>>,
    reservations: &Query<&LogisticsReservation>,
    tuning: &Tuning,
    target: Entity,
) -> OverlayAmounts {
    let (physical, capacity) = match kind {
        StructureOverlayKind::Deposit => deposits
            .get(target)
            .map(|value| (value.amount, value.capacity))
            .unwrap_or_default(),
        StructureOverlayKind::Stockpile => stockpiles
            .get(target)
            .map(|value| (value.amount, value.capacity))
            .unwrap_or_default(),
        StructureOverlayKind::Facility => facilities
            .get(target)
            .map(|value| (value.input_amount, value.input_capacity))
            .unwrap_or_default(),
        StructureOverlayKind::Planned => planned
            .get(target)
            .map(PlannedStructure::construction_progress)
            .unwrap_or_default(),
        StructureOverlayKind::Charger => chargers
            .get(target)
            .map(|value| (value.amount, value.capacity))
            .unwrap_or_default(),
        StructureOverlayKind::Worker => cargo
            .get(target)
            .map(|value| (value.amount, tuning.logistics.worker_carry_capacity))
            .unwrap_or_default(),
        StructureOverlayKind::Hauler => cargo
            .get(target)
            .map(|value| (value.amount, tuning.logistics.hauler_carry_capacity))
            .unwrap_or_default(),
    };
    let outgoing_reserved = if matches!(
        kind,
        StructureOverlayKind::Deposit | StructureOverlayKind::Stockpile
    ) {
        reservations
            .iter()
            .filter(|reservation| reservation.source == target)
            .map(|reservation| reservation.source_remaining)
            .sum()
    } else {
        0
    };
    let incoming_reserved = if matches!(
        kind,
        StructureOverlayKind::Stockpile
            | StructureOverlayKind::Facility
            | StructureOverlayKind::Charger
    ) {
        reservations
            .iter()
            .filter(|reservation| reservation.destination == target)
            .map(|reservation| reservation.destination_remaining)
            .sum()
    } else {
        0
    };
    OverlayAmounts {
        physical,
        capacity,
        outgoing_reserved,
        incoming_reserved,
    }
}

#[allow(clippy::type_complexity)]
fn update_segment_sprite(
    kind: StructureOverlayKind,
    amounts: OverlayAmounts,
    settings: &PresentationTuning,
    entity: Entity,
    fills: &mut Query<
        (&StructureOverlaySegment, &mut Sprite, &mut Transform),
        (With<StructureOverlayFill>, Without<StructureOverlay>),
    >,
) {
    let Ok((segment, mut sprite, mut transform)) = fills.get_mut(entity) else {
        return;
    };
    let size = overlay_bar_size(kind, settings);
    let physical_fraction = fill_fraction(amounts.physical, amounts.capacity);
    let (fraction, start_fraction, color) = match segment.kind {
        StructureOverlaySegmentKind::Physical => (physical_fraction, 0.0, overlay_fill_color(kind)),
        StructureOverlaySegmentKind::OutgoingReserved => {
            let reserved = amounts.outgoing_reserved.min(amounts.physical);
            let fraction = fill_fraction(reserved, amounts.capacity);
            (
                fraction,
                (physical_fraction - fraction).max(0.0),
                reservation_segment_color(segment.kind),
            )
        }
        StructureOverlaySegmentKind::IncomingReserved => (
            fill_fraction(amounts.incoming_reserved, amounts.capacity).min(1.0 - physical_fraction),
            physical_fraction,
            reservation_segment_color(segment.kind),
        ),
    };
    let fill_width = size.x * fraction;
    sprite.custom_size = Some(Vec2::new(fill_width, size.y));
    sprite.color = color;
    transform.translation = Vec3::new(
        -size.x / 2.0 + size.x * start_fraction + fill_width / 2.0,
        0.0,
        FILL_CHILD_Z
            + if segment.kind == StructureOverlaySegmentKind::Physical {
                0.0
            } else {
                0.01
            },
    );
}

pub fn structure_overlay_visibility_system(
    tuning: Res<Tuning>,
    zoom_query: Query<&CameraZoom2d>,
    mut overlays: Query<&mut Visibility, (With<StructureOverlay>, Without<ConditionOverlay>)>,
    mut condition_overlays: Query<
        &mut Visibility,
        (With<ConditionOverlay>, Without<StructureOverlay>),
    >,
) {
    let zoom = effective_zoom(zoom_query.iter(), 1.0);
    let target = overlay_visibility_for_zoom(zoom, tuning.presentation.overlay_zoom_threshold);
    for mut visibility in &mut overlays {
        if *visibility != target {
            *visibility = target;
        }
    }
    for mut visibility in &mut condition_overlays {
        if *visibility != target {
            *visibility = target;
        }
    }
}

#[allow(clippy::type_complexity)]
pub fn structure_overlay_cleanup_system(
    mut commands: Commands,
    overlays: Query<(Entity, &StructureOverlay)>,
    targets: Query<Entity>,
    cargo_bots: Query<(&Cargo, Option<&ExtractProgress>, Option<&HaulerLoading>), With<Nanobot>>,
) {
    for (overlay_entity, overlay) in &overlays {
        let target_gone = targets.get(overlay.target).is_err();
        let cargo_hidden = matches!(
            overlay.kind,
            StructureOverlayKind::Worker | StructureOverlayKind::Hauler
        ) && cargo_bots.get(overlay.target).map_or(
            true,
            |(cargo, extracting, loading)| {
                cargo.amount == 0 && extracting.is_none() && loading.is_none()
            },
        );
        if target_gone || cargo_hidden {
            commands.entity(overlay_entity).despawn();
        }
    }
}

#[allow(clippy::type_complexity)]
pub fn condition_overlay_cleanup_system(
    mut commands: Commands,
    overlays: Query<(Entity, &ConditionOverlay)>,
    support_targets: Query<
        (),
        (
            With<Structure>,
            Or<(With<Stockpile>, With<ProductionFacility>, With<Charger>)>,
        ),
    >,
    maintenance_workers: Query<(), (With<Nanobot>, With<MaintenanceProgress>)>,
) {
    for (entity, overlay) in &overlays {
        let valid = match overlay.kind {
            ConditionOverlayKind::Maintenance | ConditionOverlayKind::Health => {
                support_targets.get(overlay.target).is_ok()
            }
            ConditionOverlayKind::WorkerProgress => maintenance_workers.get(overlay.target).is_ok(),
        };
        if !valid {
            commands.entity(entity).despawn();
        }
    }
}

/// Visual residue has no structure, reservation, or collision identity.
#[derive(Component)]
pub struct CancelledPlanVisual {
    elapsed: u32,
    original_scale: Vec3,
}

pub(crate) fn spawn_cancelled_plan_visual(
    commands: &mut Commands,
    sprites: &crate::structure_sprites::StructureSprites,
    kind: crate::nanobot::PlannedKind,
    transform: Transform,
) {
    let mut sprite = sprites.sprite(
        kind,
        crate::structure_sprites::StructureVisualState::Planned,
    );
    sprite.color = Color::srgba(1.0, 0.05, 0.05, 1.0);
    sprite.custom_size = Some(Vec2::splat(crate::navigation::STRUCTURE_SPRITE_SIZE));
    commands.spawn((
        sprite,
        transform,
        CancelledPlanVisual {
            elapsed: 0,
            original_scale: transform.scale,
        },
    ));
}

pub(crate) fn animate_cancelled_plans_system(
    mut commands: Commands,
    mut effects: Query<(
        Entity,
        &mut CancelledPlanVisual,
        &mut Transform,
        &mut Sprite,
    )>,
) {
    for (entity, mut effect, mut transform, mut sprite) in &mut effects {
        effect.elapsed += 1;
        if effect.elapsed >= 30 {
            commands.entity(entity).despawn();
            continue;
        }
        let fade = ((effect.elapsed.saturating_sub(6)) as f32 / 24.0).clamp(0.0, 1.0);
        transform.scale = effect.original_scale * (1.0 - fade);
        sprite.color = Color::srgba(1.0, 0.05, 0.05, 1.0 - fade);
    }
}

#[cfg(test)]
mod tests {
    use approx::assert_abs_diff_eq;

    use super::*;
    use crate::nanobot::PlannedKind;

    #[test]
    fn fill_fraction_clamps_and_handles_zero_capacity() {
        assert_abs_diff_eq!(fill_fraction(0, 100), 0.0, epsilon = 1e-5);
        assert_abs_diff_eq!(fill_fraction(50, 100), 0.5, epsilon = 1e-5);
        assert_abs_diff_eq!(fill_fraction(150, 100), 1.0, epsilon = 1e-5);
        assert_abs_diff_eq!(fill_fraction(10, 0), 0.0, epsilon = 1e-5);
    }

    #[test]
    fn planned_fill_fraction_reports_spent_work_budget() {
        let mut planned = PlannedStructure::new(PlannedKind::SinkStockpile, IVec2::ZERO);
        planned = planned.with_work_remaining(Tuning::default().construction.work_ticks);
        assert_abs_diff_eq!(planned_fill_fraction(&planned), 0.0, epsilon = 1e-5);
        planned = planned.with_work_remaining(Tuning::default().construction.work_ticks / 2);
        assert_abs_diff_eq!(planned_fill_fraction(&planned), 0.5, epsilon = 1e-5);
        planned = planned.with_work_remaining(0);
        assert_abs_diff_eq!(planned_fill_fraction(&planned), 1.0, epsilon = 1e-5);
    }

    #[test]
    fn overlay_bar_size_is_smaller_for_haulers() {
        let settings = PresentationTuning::default();
        assert_eq!(
            overlay_bar_size(StructureOverlayKind::Stockpile, &settings),
            Vec2::new(settings.structure_bar_width, settings.structure_bar_height)
        );
        assert_eq!(
            overlay_bar_size(StructureOverlayKind::Hauler, &settings),
            Vec2::new(settings.cargo_bar_width, settings.cargo_bar_height)
        );
        assert_eq!(
            overlay_bar_size(StructureOverlayKind::Worker, &settings),
            Vec2::new(settings.cargo_bar_width, settings.cargo_bar_height)
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
        assert_abs_diff_eq!(
            effective_zoom(std::iter::empty::<&CameraZoom2d>(), 1.0),
            1.0,
            epsilon = 1e-5
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
        assert_abs_diff_eq!(effective_zoom(cameras.iter(), 1.0), 2.5, epsilon = 1e-5);
    }

    #[test]
    fn overlay_offsets_are_above_targets() {
        let settings = PresentationTuning::default();
        assert_abs_diff_eq!(
            overlay_label_offset_y(StructureOverlayKind::Deposit, Some(64.0), &settings),
            64.0 + settings.structure_label_gap,
            epsilon = 1e-5
        );
        assert_abs_diff_eq!(
            overlay_label_offset_y(StructureOverlayKind::Deposit, None, &settings),
            DEFAULT_DEPOSIT_OVERLAY_RADIUS + settings.structure_label_gap,
            epsilon = 1e-5
        );
        let structure_offset = PLANNED_STRUCTURE_FOOTPRINT / 2.0 + settings.structure_label_gap;
        for kind in StructureOverlayKind::STRUCTURES {
            let offset = overlay_label_offset_y(kind, Some(9999.0), &settings);
            assert!(offset > 0.0);
            if kind != StructureOverlayKind::Deposit {
                assert_abs_diff_eq!(offset, structure_offset, epsilon = 1e-5);
            }
        }
        for kind in [StructureOverlayKind::Worker, StructureOverlayKind::Hauler] {
            assert_abs_diff_eq!(
                overlay_label_offset_y(kind, None, &settings),
                BOT_RADIUS + settings.cargo_label_gap,
                epsilon = 1e-5
            );
        }
    }
}
