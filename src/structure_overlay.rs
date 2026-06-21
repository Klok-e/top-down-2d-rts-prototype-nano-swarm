//! Zoom-aware structure status overlays (issue #30).
//!
//! Every support structure carries an always-visible,
//! world-attached status label that summarises its current
//! state. Deposits show remaining resources, stockpiles
//! show load, production facilities show idle/working
//! state with progress and target, planned structures
//! show build progress, and chargers show current/total
//! supply. Labels sit in world space (so they respect
//! camera zoom automatically) and disappear once the
//! camera zoom value meets or exceeds
//! [`StructureOverlaySettings::hide_zoom_threshold`] so the
//! player can declutter the view when zoomed out far.

use bevy::prelude::*;

use crate::fly_camera::CameraZoom2d;
use crate::nanobot::{
    Charger, PlannedKind, PlannedStructure, ProductionFacility, DEFAULT_PLANNED_WORK_TICKS,
};
use crate::resources::{ResourceDeposit, Stockpile};

/// Camera zoom value at or above which overlays hide. The
/// camera's default `zoom` is `1.0`; this default sits
/// well above that so labels stay visible at the typical
/// play zoom and disappear only when the player zooms out
/// deliberately. Override at runtime through
/// [`StructureOverlaySettings`].
pub const DEFAULT_OVERLAY_HIDE_ZOOM_THRESHOLD: f32 = 4.0;

/// World-units Y offset for the overlay label relative to
/// the structure's world position. Negative so the label
/// sits below the structure's centre and the structure's
/// own sprite stays visible above it.
pub const OVERLAY_LABEL_OFFSET_Y: f32 = -48.0;

/// Runtime configuration for the overlay system. Inserted
/// as a Bevy [`Resource`] so it can be mutated by the
/// player (e.g. a future "labels off" toggle) or by tests.
///
/// `hide_zoom_threshold` is the camera zoom value at or
/// above which overlays hide. The default
/// ([`DEFAULT_OVERLAY_HIDE_ZOOM_THRESHOLD`]) keeps labels
/// visible at the default play zoom. Setting it to `0.0`
/// or below forces every overlay visible; setting it to
/// `f32::INFINITY` hides every overlay.
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

/// The five structure kinds an overlay can summarise. The
/// spawn / update / cleanup systems switch on this to
/// pick the right query and the right formatter. No
/// structure-side marker is required: the overlay is its
/// own entity and stores its target, so the gameplay
/// systems never need to know about the overlay layer.
#[derive(Debug, Component, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StructureOverlayKind {
    Deposit,
    Stockpile,
    Facility,
    Planned,
    Charger,
}

impl StructureOverlayKind {
    /// Every kind in stable declaration order. Used by the
    /// spawn system to iterate kinds and by tests that
    /// pin the full set.
    pub const ALL: [StructureOverlayKind; 5] = [
        StructureOverlayKind::Deposit,
        StructureOverlayKind::Stockpile,
        StructureOverlayKind::Facility,
        StructureOverlayKind::Planned,
        StructureOverlayKind::Charger,
    ];
}

/// Marker on the overlay entity. `target` is the structure
/// the overlay tracks. The cleanup system despawns the
/// overlay when `target` is gone; the update system reads
/// from `target` every tick.
#[derive(Debug, Component, Clone, Copy)]
pub struct StructureOverlay {
    pub target: Entity,
    pub kind: StructureOverlayKind,
}

/// Background color of the overlay panel for `kind`. Each
/// kind gets a distinct hue so the player can tell kinds
/// apart at a glance.
pub fn overlay_background_color(kind: StructureOverlayKind) -> Color {
    match kind {
        StructureOverlayKind::Deposit => Color::srgba(0.65, 0.45, 0.10, 0.85),
        StructureOverlayKind::Stockpile => Color::srgba(0.20, 0.50, 0.20, 0.85),
        StructureOverlayKind::Facility => Color::srgba(0.20, 0.40, 0.70, 0.85),
        StructureOverlayKind::Planned => Color::srgba(0.45, 0.45, 0.45, 0.85),
        StructureOverlayKind::Charger => Color::srgba(0.55, 0.30, 0.70, 0.85),
    }
}

/// "Deposit 840"-style label for a [`ResourceDeposit`].
/// The amount is the only state the deposit carries that
/// belongs in the label; capacity and radius are
/// simulation-internal and would only add noise.
pub fn format_deposit_label(amount: u32) -> String {
    format!("Deposit {amount}")
}

/// "Stockpile 120/1000"-style label for a [`Stockpile`].
/// Both `amount` and `capacity` belong in the label so the
/// player can see how full the buffer is at a glance.
pub fn format_stockpile_label(amount: u32, capacity: u32) -> String {
    format!("Stockpile {amount}/{capacity}")
}

/// "Facility: idle" / "Facility: Worker 40%" label for a
/// [`ProductionFacility`]. Idle facilities show
/// "Facility: idle" with no target; working facilities
/// show the type and the current cycle's progress as a
/// percent. The percent is rounded down to keep the label
/// stable across ticks.
pub fn format_facility_label(facility: &ProductionFacility) -> String {
    match facility.current_target {
        None => "Facility: idle".to_string(),
        Some(kind) => {
            let percent = crate::nanobot::production_progress_percent(facility);
            format!("Facility: {kind:?} {percent}%")
        }
    }
}

/// "Building Stockpile 40%"-style label for a
/// [`PlannedStructure`]. The "kind" string is the
/// short tag the issue's acceptance list uses (e.g.
/// "Stockpile", "Facility", "Charger"). The percent is
/// computed from `work_remaining` and the default work
/// budget so the player sees the build advancing.
pub fn format_planned_label(planned: &PlannedStructure) -> String {
    let kind_str = match planned.kind {
        PlannedKind::SourceStockpile | PlannedKind::SinkStockpile => "Stockpile",
        PlannedKind::ProductionFacility => "Facility",
        PlannedKind::Charger => "Charger",
    };
    let percent = planned_build_percent(planned);
    format!("Building {kind_str} {percent}%")
}

/// "Charger 60/100"-style label for a [`Charger`]. Both
/// `amount` and `capacity` belong in the label so the
/// player can see the buffer's fill level.
pub fn format_charger_label(amount: u32, capacity: u32) -> String {
    format!("Charger {amount}/{capacity}")
}

/// Build progress for a [`PlannedStructure`] as a
/// `0..=100` integer percent. The percent is computed
/// from `work_remaining` and the total work budget
/// ([`DEFAULT_PLANNED_WORK_TICKS`]) so a freshly planned
/// structure with the full budget shows 0% and a plan
/// that has spent all but the last tick shows 80%
/// (assuming a 5-tick budget). Floors at 0 and caps at
/// 100 so an over-spend or over-budget plan does not
/// produce a confusing label.
pub fn planned_build_percent(planned: &PlannedStructure) -> u32 {
    let work_budget = DEFAULT_PLANNED_WORK_TICKS;
    if work_budget == 0 {
        return 100;
    }
    let spent = work_budget.saturating_sub(planned.work_remaining);
    let percent = (spent as u64 * 100 / work_budget as u64) as u32;
    percent.min(100)
}

/// Decide the visibility for an overlay given the current
/// camera zoom and the configured threshold. The contract
/// is "hide when zoomed out beyond the threshold" -- the
/// math is `zoom >= threshold`. Using `>=` rather than `>`
/// so the boundary value itself is hidden (one zoom unit
/// of hysteresis would be over-engineering for a visual
/// declutter toggle).
///
/// Two threshold extremes are special-cased so tests and
/// "always on / always off" toggles have a clean knob:
///
/// - `threshold == f32::INFINITY` always hides, even at
///   `zoom = 0.0`. A future "labels off" UI toggle can
///   set the threshold to `f32::INFINITY` to disable the
///   layer without removing the system.
/// - `threshold <= 0.0` always shows, even at the largest
///   possible zoom. Useful for tests that want the
///   overlays visible no matter what the camera reports.
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

/// Effective zoom for the visibility check. Returns the
/// first camera's zoom, or `fallback_zoom` if the world
/// has no camera. The fallback is `1.0` so a test app
/// without a camera treats the world as the default
/// play zoom and the overlays stay visible.
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

/// Plugin that wires the structure overlay systems into
/// the `Update` schedule. The chain runs spawn -> update
/// -> visibility -> cleanup, so a structure spawned and
/// then despawned in the same tick leaves no orphan
/// overlay alive.
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

/// Spawn one [`StructureOverlay`] for every structure
/// entity that does not yet have one. The set of
/// "already has an overlay" is built once per tick from
/// the overlay query, then the structure queries run a
/// "not in set" check. The text starts as an empty
/// string; the update system overwrites it on the same
/// tick's next system run.
#[allow(clippy::type_complexity)]
pub fn structure_overlay_spawn_system(
    mut commands: Commands,
    deposits: Query<Entity, With<ResourceDeposit>>,
    stockpiles: Query<Entity, With<Stockpile>>,
    facilities: Query<Entity, With<ProductionFacility>>,
    planned: Query<Entity, With<PlannedStructure>>,
    chargers: Query<Entity, With<Charger>>,
    existing: Query<&StructureOverlay>,
) {
    let covered: std::collections::HashSet<Entity> = existing.iter().map(|o| o.target).collect();
    for entity in &deposits {
        if covered.contains(&entity) {
            continue;
        }
        spawn_overlay_for(&mut commands, entity, StructureOverlayKind::Deposit);
    }
    for entity in &stockpiles {
        if covered.contains(&entity) {
            continue;
        }
        spawn_overlay_for(&mut commands, entity, StructureOverlayKind::Stockpile);
    }
    for entity in &facilities {
        if covered.contains(&entity) {
            continue;
        }
        spawn_overlay_for(&mut commands, entity, StructureOverlayKind::Facility);
    }
    for entity in &planned {
        if covered.contains(&entity) {
            continue;
        }
        spawn_overlay_for(&mut commands, entity, StructureOverlayKind::Planned);
    }
    for entity in &chargers {
        if covered.contains(&entity) {
            continue;
        }
        spawn_overlay_for(&mut commands, entity, StructureOverlayKind::Charger);
    }
}

fn spawn_overlay_for(commands: &mut Commands, target: Entity, kind: StructureOverlayKind) {
    commands.spawn((
        StructureOverlay { target, kind },
        Text2d::new(""),
        TextColor(Color::WHITE),
        TextBackgroundColor(overlay_background_color(kind)),
        Transform::from_translation(Vec3::new(0.0, OVERLAY_LABEL_OFFSET_Y, 0.5)),
        Visibility::Inherited,
    ));
}

/// Update the label text and world position of every
/// existing overlay. The text is recomputed from the
/// target's current state every tick, so a deposit that
/// loses material or a facility that finishes a cycle
/// sees the new label on the next tick without any extra
/// signal. The position is the target's translation
/// plus the [`OVERLAY_LABEL_OFFSET_Y`] Y offset.
#[allow(clippy::type_complexity)]
pub fn structure_overlay_update_system(
    mut overlays: Query<(&StructureOverlay, &mut Text2d, &mut Transform)>,
    deposits: Query<&ResourceDeposit, Without<StructureOverlay>>,
    stockpiles: Query<&Stockpile, Without<StructureOverlay>>,
    facilities: Query<&ProductionFacility, Without<StructureOverlay>>,
    planned: Query<&PlannedStructure, Without<StructureOverlay>>,
    chargers: Query<&Charger, Without<StructureOverlay>>,
    target_transforms: Query<&Transform, Without<StructureOverlay>>,
) {
    for (overlay, mut text, mut transform) in &mut overlays {
        // If the target is gone the cleanup system will
        // despawn the overlay on this tick's last pass;
        // skip the update rather than crash.
        let Ok(target_pos) = target_transforms
            .get(overlay.target)
            .map(|t| t.translation.truncate())
        else {
            continue;
        };
        transform.translation = (target_pos + Vec2::new(0.0, OVERLAY_LABEL_OFFSET_Y)).extend(0.5);
        text.0 = compute_label_text(
            overlay.kind,
            &deposits,
            &stockpiles,
            &facilities,
            &planned,
            &chargers,
            overlay.target,
        );
    }
}

#[allow(clippy::type_complexity)]
fn compute_label_text(
    kind: StructureOverlayKind,
    deposits: &Query<&ResourceDeposit, Without<StructureOverlay>>,
    stockpiles: &Query<&Stockpile, Without<StructureOverlay>>,
    facilities: &Query<&ProductionFacility, Without<StructureOverlay>>,
    planned: &Query<&PlannedStructure, Without<StructureOverlay>>,
    chargers: &Query<&Charger, Without<StructureOverlay>>,
    target: Entity,
) -> String {
    match kind {
        StructureOverlayKind::Deposit => deposits
            .get(target)
            .map(|d| format_deposit_label(d.amount))
            .unwrap_or_default(),
        StructureOverlayKind::Stockpile => stockpiles
            .get(target)
            .map(|s| format_stockpile_label(s.amount, s.capacity))
            .unwrap_or_default(),
        StructureOverlayKind::Facility => facilities
            .get(target)
            .map(format_facility_label)
            .unwrap_or_default(),
        StructureOverlayKind::Planned => planned
            .get(target)
            .map(format_planned_label)
            .unwrap_or_default(),
        StructureOverlayKind::Charger => chargers
            .get(target)
            .map(|c| format_charger_label(c.amount, c.capacity))
            .unwrap_or_default(),
    }
}

/// Toggle each overlay's [`Visibility`] based on the
/// current camera zoom and the configured threshold.
/// The visibility contract is in
/// [`overlay_visibility_for_zoom`]: hidden when
/// `zoom >= threshold`, visible (inherited) otherwise.
/// A test app with no camera sees a zoom of `1.0`,
/// which is below the default threshold of `4.0`, so
/// the overlays stay visible.
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

/// Despawn every overlay whose target entity is gone.
/// Runs last in the chain so a freshly despawned
/// structure (e.g. a planned structure that just
/// promoted to a stockpile) does not leave an
/// orphan alive in the same tick. The new entity
/// (the completed stockpile) gets its own fresh
/// overlay from the spawn system on the same tick.
pub fn structure_overlay_cleanup_system(
    mut commands: Commands,
    overlays: Query<(Entity, &StructureOverlay)>,
    targets: Query<Entity>,
) {
    for (overlay_entity, overlay) in &overlays {
        if targets.get(overlay.target).is_err() {
            commands.entity(overlay_entity).despawn();
        }
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    //! Pure-helper unit tests. The end-to-end contracts
    //! (spawn, update, visibility, cleanup) live in
    //! `tests/behavior/structure_overlay.rs`.

    use super::*;

    #[test]
    fn format_deposit_label_includes_remaining_amount() {
        assert_eq!(format_deposit_label(0), "Deposit 0");
        assert_eq!(format_deposit_label(840), "Deposit 840");
        assert_eq!(format_deposit_label(u32::MAX), "Deposit 4294967295");
    }

    #[test]
    fn format_stockpile_label_includes_amount_and_capacity() {
        assert_eq!(format_stockpile_label(0, 1000), "Stockpile 0/1000");
        assert_eq!(format_stockpile_label(120, 1000), "Stockpile 120/1000");
        assert_eq!(format_stockpile_label(1000, 1000), "Stockpile 1000/1000");
    }

    #[test]
    fn format_charger_label_includes_amount_and_capacity() {
        assert_eq!(format_charger_label(0, 100), "Charger 0/100");
        assert_eq!(format_charger_label(60, 100), "Charger 60/100");
        assert_eq!(format_charger_label(100, 100), "Charger 100/100");
    }

    #[test]
    fn format_facility_label_idle_when_no_target() {
        let f = ProductionFacility::new();
        assert_eq!(format_facility_label(&f), "Facility: idle");
    }

    #[test]
    fn format_facility_label_working_shows_type_and_progress() {
        let mut f = ProductionFacility::new();
        f.current_target = Some(crate::nanobot::NanobotType::Worker);
        f.progress = 2;
        let label = format_facility_label(&f);
        assert!(
            label.starts_with("Facility: Worker "),
            "working facility label must show the type; got {label}"
        );
        assert!(
            label.ends_with('%'),
            "working facility label must end with a percent sign; got {label}"
        );
    }

    #[test]
    fn format_planned_label_uses_kind_name_and_percent() {
        let cell = IVec2::new(0, 0);
        // work_remaining / DEFAULT_PLANNED_WORK_TICKS
        // (5) drives the percent. 3/5 remaining =
        // 40% spent; 1/5 = 80%; 5/5 = 0%; 0/5 = 100%.
        let mut sink = PlannedStructure::new(PlannedKind::SinkStockpile, cell);
        sink.work_remaining = 3;
        assert_eq!(format_planned_label(&sink), "Building Stockpile 40%");

        let mut fac = PlannedStructure::new(PlannedKind::ProductionFacility, cell);
        fac.work_remaining = 1;
        assert_eq!(format_planned_label(&fac), "Building Facility 80%");

        let mut chg = PlannedStructure::new(PlannedKind::Charger, cell);
        chg.work_remaining = 5;
        assert_eq!(format_planned_label(&chg), "Building Charger 0%");
        chg.work_remaining = 0;
        assert_eq!(format_planned_label(&chg), "Building Charger 100%");

        // Source and Sink stockpiles share the short
        // label so the player only learns one name.
        let mut source = PlannedStructure::new(PlannedKind::SourceStockpile, cell);
        source.work_remaining = sink.work_remaining;
        assert_eq!(
            format_planned_label(&source),
            format_planned_label(&sink),
            "source and sink stockpiles must share the label"
        );
    }

    #[test]
    fn planned_build_percent_floors_at_zero_and_caps_at_hundred() {
        let cell = IVec2::new(0, 0);
        let mut p = PlannedStructure::new(PlannedKind::SourceStockpile, cell);
        p.work_remaining = DEFAULT_PLANNED_WORK_TICKS;
        assert_eq!(planned_build_percent(&p), 0);
        p.work_remaining = 0;
        assert_eq!(planned_build_percent(&p), 100);
    }

    #[test]
    fn overlay_background_colors_are_distinct_per_kind() {
        let colors: Vec<Color> = StructureOverlayKind::ALL
            .iter()
            .map(|k| overlay_background_color(*k))
            .collect();
        for i in 0..colors.len() {
            for j in (i + 1)..colors.len() {
                assert_ne!(
                    colors[i],
                    colors[j],
                    "overlay background color for {:?} must differ from {:?}",
                    StructureOverlayKind::ALL[i],
                    StructureOverlayKind::ALL[j]
                );
            }
        }
    }

    #[test]
    fn overlay_visibility_hides_only_at_or_above_threshold() {
        // Just below the threshold still shows; at or
        // above hides. A threshold of zero keeps the
        // labels always visible; `f32::INFINITY` keeps
        // them always hidden -- the two extremes a
        // player toggle and a test seam both need.
        assert_eq!(overlay_visibility_for_zoom(1.0, 4.0), Visibility::Inherited);
        assert_eq!(
            overlay_visibility_for_zoom(3.99, 4.0),
            Visibility::Inherited
        );
        assert_eq!(overlay_visibility_for_zoom(4.0, 4.0), Visibility::Hidden);
        assert_eq!(overlay_visibility_for_zoom(10.0, 4.0), Visibility::Hidden);
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
    fn effective_zoom_falls_back_when_no_camera() {
        // No camera in the iterator => the supplied
        // fallback wins. Test apps that do not spawn a
        // camera rely on this so the visibility system
        // still works.
        let zoom = effective_zoom(std::iter::empty::<&CameraZoom2d>(), 1.0);
        assert_eq!(zoom, 1.0);
    }

    #[test]
    fn effective_zoom_reads_first_camera_zoom() {
        // The first camera's zoom wins so multi-camera
        // setups are deterministic.
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
        let zoom = effective_zoom(cameras.iter(), 1.0);
        assert_eq!(zoom, 2.5);
    }

    #[test]
    fn default_settings_use_default_threshold() {
        let s = StructureOverlaySettings::default();
        assert_eq!(s.hide_zoom_threshold, DEFAULT_OVERLAY_HIDE_ZOOM_THRESHOLD);
    }
}
