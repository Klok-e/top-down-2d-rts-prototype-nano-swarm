//! Right-side production-priority UI.
//!
//! Worker, Hauler, and Defender occupy one fixed 100% bar. Two handles edit
//! cumulative boundaries in 5% steps.

use bevy::prelude::*;
use bevy::ui::{
    AlignItems, BorderRadius, FlexDirection, JustifyContent, PositionType, RelativeCursorPosition,
    UiRect, Val,
};

use crate::nanobot::{NanobotType, ProductionPriority};

use super::ui_setup::FontsResource;

#[derive(Debug, Component)]
pub struct ProductionPriorityPanelRoot;

#[derive(Debug, Component)]
pub struct ProductionPriorityTrack;

#[derive(Debug, Component, Clone, Copy, PartialEq, Eq)]
pub struct ProductionPriorityHandle(pub HandleBoundary);

#[derive(Debug, Component, Clone, Copy)]
pub struct ProductionPrioritySegment(pub NanobotType);

#[derive(Debug, Component, Clone, Copy)]
pub struct ProductionPriorityValueText(pub NanobotType);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandleBoundary {
    WorkerEnd,
    HaulerEnd,
}

#[derive(Debug, Default, Resource)]
pub struct ProductionPriorityDragState {
    active: Option<HandleBoundary>,
}

pub const SNAP_STEP: u32 = 5;
pub const PANEL_TOP: f32 = 8.0;
pub const PANEL_RIGHT: f32 = 8.0;
pub const PANEL_WIDTH: f32 = 240.0;
pub const PANEL_PADDING: f32 = 10.0;
pub const PANEL_FONT_SIZE: f32 = 14.0;
pub const PANEL_TITLE_FONT_SIZE: f32 = 18.0;
pub const PANEL_GAP: f32 = 7.0;
pub const TRACK_HEIGHT: f32 = 22.0;
pub const HANDLE_WIDTH: f32 = 8.0;

fn type_color(kind: NanobotType) -> Color {
    match kind {
        NanobotType::Worker => Color::srgb(0.85, 0.65, 0.30),
        NanobotType::Hauler => Color::srgb(0.30, 0.75, 0.85),
        NanobotType::Defender => Color::srgb(0.40, 0.55, 0.95),
    }
}

fn type_label(kind: NanobotType) -> &'static str {
    match kind {
        NanobotType::Worker => "Worker",
        NanobotType::Hauler => "Hauler",
        NanobotType::Defender => "Defender",
    }
}

fn snap_percent(value: f32) -> u32 {
    ((value.clamp(0.0, 100.0) / SNAP_STEP as f32).round() as u32 * SNAP_STEP).min(100)
}

fn track_percent_from_normalized_x(normalized_x: f32) -> f32 {
    // Bevy reports node-relative coordinates from -0.5 at the left edge to
    // 0.5 at the right edge.
    (normalized_x + 0.5).clamp(0.0, 1.0) * 100.0
}

fn clamp_boundary(
    boundary: HandleBoundary,
    proposed: u32,
    worker_end: u32,
    hauler_end: u32,
) -> u32 {
    match boundary {
        HandleBoundary::WorkerEnd => proposed.min(hauler_end),
        HandleBoundary::HaulerEnd => proposed.max(worker_end).min(100),
    }
}

fn boundaries_from_priority(priority: &ProductionPriority) -> (u32, u32) {
    let total = priority.total_weight();
    if total == 0 {
        return (0, 0);
    }
    let worker = snap_percent(priority.weight(NanobotType::Worker) as f32 * 100.0 / total as f32);
    let hauler_end = snap_percent(
        (priority.weight(NanobotType::Worker) + priority.weight(NanobotType::Hauler)) as f32
            * 100.0
            / total as f32,
    );
    (worker.min(hauler_end), hauler_end)
}

fn write_boundaries(priority: &mut ProductionPriority, worker_end: u32, hauler_end: u32) {
    priority.set_weight(NanobotType::Worker, worker_end);
    priority.set_weight(NanobotType::Hauler, hauler_end - worker_end);
    priority.set_weight(NanobotType::Defender, 100 - hauler_end);
}

fn handle_offset(boundary: HandleBoundary, coincident: bool) -> f32 {
    if coincident {
        match boundary {
            HandleBoundary::WorkerEnd => -HANDLE_WIDTH,
            HandleBoundary::HaulerEnd => 0.0,
        }
    } else {
        -HANDLE_WIDTH / 2.0
    }
}

pub fn setup_production_priority_panel(
    mut commands: Commands,
    fonts: Res<FontsResource>,
    priority: Res<ProductionPriority>,
) {
    let font = fonts.font.clone();
    let (worker_end, hauler_end) = boundaries_from_priority(&priority);

    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(PANEL_TOP),
                right: Val::Px(PANEL_RIGHT),
                width: Val::Px(PANEL_WIDTH),
                padding: UiRect::all(Val::Px(PANEL_PADDING)),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(PANEL_GAP),
                align_items: AlignItems::Stretch,
                border_radius: BorderRadius::all(Val::Px(4.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.03, 0.04, 0.05, 0.78)),
            ProductionPriorityPanelRoot,
        ))
        .with_children(|panel| {
            panel.spawn((
                Text::new("Production Priority"),
                TextFont {
                    font: font.clone(),
                    font_size: PANEL_TITLE_FONT_SIZE,
                    ..default()
                },
                TextColor(Color::WHITE),
            ));

            panel
                .spawn((
                    ProductionPriorityTrack,
                    RelativeCursorPosition::default(),
                    Node {
                        position_type: PositionType::Relative,
                        width: Val::Percent(100.0),
                        height: Val::Px(TRACK_HEIGHT),
                        ..default()
                    },
                    BackgroundColor(Color::srgb(0.12, 0.12, 0.14)),
                ))
                .with_children(|track| {
                    let starts = [0, worker_end, hauler_end];
                    let ends = [worker_end, hauler_end, 100];
                    for ((kind, start), end) in NanobotType::ALL.into_iter().zip(starts).zip(ends) {
                        track.spawn((
                            ProductionPrioritySegment(kind),
                            Node {
                                position_type: PositionType::Absolute,
                                left: Val::Percent(start as f32),
                                width: Val::Percent((end - start) as f32),
                                height: Val::Percent(100.0),
                                ..default()
                            },
                            BackgroundColor(type_color(kind)),
                        ));
                    }
                    for (boundary, percent) in [
                        (HandleBoundary::WorkerEnd, worker_end),
                        (HandleBoundary::HaulerEnd, hauler_end),
                    ] {
                        track.spawn((
                            ProductionPriorityHandle(boundary),
                            RelativeCursorPosition::default(),
                            Node {
                                position_type: PositionType::Absolute,
                                left: Val::Percent(percent as f32),
                                width: Val::Px(HANDLE_WIDTH),
                                height: Val::Px(TRACK_HEIGHT),
                                margin: UiRect::left(Val::Px(handle_offset(
                                    boundary,
                                    worker_end == hauler_end,
                                ))),
                                border: UiRect::all(Val::Px(1.0)),
                                ..default()
                            },
                            BackgroundColor(Color::WHITE),
                            BorderColor::all(Color::BLACK),
                        ));
                    }
                });

            panel
                .spawn(Node {
                    flex_direction: FlexDirection::Row,
                    justify_content: JustifyContent::SpaceBetween,
                    align_items: AlignItems::Center,
                    ..default()
                })
                .with_children(|labels| {
                    for kind in NanobotType::ALL {
                        labels.spawn((
                            ProductionPriorityValueText(kind),
                            Text::new(format!(
                                "{} {}%",
                                type_label(kind),
                                priority.percentage(kind)
                            )),
                            TextFont {
                                font: font.clone(),
                                font_size: PANEL_FONT_SIZE,
                                ..default()
                            },
                            TextColor(type_color(kind)),
                        ));
                    }
                });
        });
}

pub fn production_priority_drag_system(
    mouse: Res<ButtonInput<MouseButton>>,
    mut drag: ResMut<ProductionPriorityDragState>,
    track: Single<&RelativeCursorPosition, With<ProductionPriorityTrack>>,
    handles: Query<(&ProductionPriorityHandle, &RelativeCursorPosition)>,
    mut priority: ResMut<ProductionPriority>,
) {
    if mouse.just_released(MouseButton::Left) || !mouse.pressed(MouseButton::Left) {
        drag.active = None;
        return;
    }
    if mouse.just_pressed(MouseButton::Left) {
        drag.active = handles
            .iter()
            .filter(|(_, cursor)| cursor.cursor_over())
            .map(|(handle, _)| handle.0)
            .next();
    }
    let Some(active) = drag.active else {
        return;
    };
    let Some(position) = track.normalized else {
        return;
    };
    let (worker_end, hauler_end) = boundaries_from_priority(&priority);
    let snapped = snap_percent(track_percent_from_normalized_x(position.x));
    let value = clamp_boundary(active, snapped, worker_end, hauler_end);
    let (new_worker, new_hauler) = match active {
        HandleBoundary::WorkerEnd => (value, hauler_end),
        HandleBoundary::HaulerEnd => (worker_end, value),
    };
    write_boundaries(&mut priority, new_worker, new_hauler);
}

#[allow(clippy::type_complexity)]
pub fn update_production_priority_panel(
    priority: Res<ProductionPriority>,
    mut segments: Query<(&ProductionPrioritySegment, &mut Node), Without<ProductionPriorityHandle>>,
    mut handles: Query<(&ProductionPriorityHandle, &mut Node), Without<ProductionPrioritySegment>>,
    mut labels: Query<(&ProductionPriorityValueText, &mut Text)>,
) {
    let (worker_end, hauler_end) = boundaries_from_priority(&priority);
    for (segment, mut node) in &mut segments {
        let (start, width) = match segment.0 {
            NanobotType::Worker => (0, worker_end),
            NanobotType::Hauler => (worker_end, hauler_end - worker_end),
            NanobotType::Defender => (hauler_end, 100 - hauler_end),
        };
        node.left = Val::Percent(start as f32);
        node.width = Val::Percent(width as f32);
    }
    for (handle, mut node) in &mut handles {
        node.left = Val::Percent(match handle.0 {
            HandleBoundary::WorkerEnd => worker_end,
            HandleBoundary::HaulerEnd => hauler_end,
        } as f32);
        node.margin.left = Val::Px(handle_offset(handle.0, worker_end == hauler_end));
    }
    for (label, mut text) in &mut labels {
        *text = Text::new(format!(
            "{} {}%",
            type_label(label.0),
            priority.percentage(label.0)
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snap_clamps_and_rounds_to_five_percent() {
        assert_eq!(snap_percent(-2.0), 0);
        assert_eq!(snap_percent(52.4), 50);
        assert_eq!(snap_percent(52.5), 55);
        assert_eq!(snap_percent(103.0), 100);
    }

    #[test]
    fn bevy_relative_cursor_coordinates_map_across_full_track() {
        assert_eq!(track_percent_from_normalized_x(-0.5), 0.0);
        assert_eq!(track_percent_from_normalized_x(0.0), 50.0);
        assert_eq!(track_percent_from_normalized_x(0.5), 100.0);
    }

    #[test]
    fn worker_boundary_cannot_cross_hauler_boundary() {
        assert_eq!(clamp_boundary(HandleBoundary::WorkerEnd, 80, 40, 65), 65);
        assert_eq!(clamp_boundary(HandleBoundary::WorkerEnd, 0, 40, 65), 0);
    }

    #[test]
    fn hauler_boundary_cannot_cross_worker_or_hundred() {
        assert_eq!(clamp_boundary(HandleBoundary::HaulerEnd, 20, 40, 65), 40);
        assert_eq!(clamp_boundary(HandleBoundary::HaulerEnd, 105, 40, 65), 100);
    }

    #[test]
    fn coincident_boundaries_allow_zero_middle_segment() {
        assert_eq!(clamp_boundary(HandleBoundary::WorkerEnd, 60, 40, 60), 60);
        assert_eq!(clamp_boundary(HandleBoundary::HaulerEnd, 40, 40, 60), 40);
    }
}
