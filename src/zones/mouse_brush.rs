use bevy::prelude::*;

use super::{
    BrushFeedback, BrushHover, PlayerIntentAction, PlayerIntentError, apply_player_intent,
    get_zone_pos_from_world,
};
use crate::{
    ZONE_BLOCK_SIZE,
    fly_camera::FlyCamera2d,
    intent::{BrushSelection, IntentGrid, IntentKind},
    nanobot::MatchOutcome,
    reachability::SwarmReachabilityMap,
    session::SessionRules,
    ui::{UiHandling, scenario_menu::ScenarioMenu},
};

pub struct BrushSample {
    position: Vec2,
    action: PlayerIntentAction,
    kind: IntentKind,
    view_revision: u64,
}

/// Applies the selected player intent to the cells crossed by the mouse stroke.
#[allow(clippy::too_many_arguments)]
pub fn zone_brush_system(
    windows: Query<&Window>,
    mouse: Res<ButtonInput<MouseButton>>,
    brush_selection: Res<BrushSelection>,
    ui_handling: Res<UiHandling>,
    outcome: Option<Res<MatchOutcome>>,
    cameras: Query<(&Transform, &Camera, Option<&FlyCamera2d>)>,
    mut grid: ResMut<IntentGrid>,
    reachability: Option<Res<SwarmReachabilityMap>>,
    menu: Option<Res<ScenarioMenu>>,
    rules: Res<SessionRules>,
    mut previous: Local<Option<BrushSample>>,
    mut feedback: Option<ResMut<BrushFeedback>>,
) {
    let previous_sample = previous.take();
    let outcome = outcome.as_deref().copied().unwrap_or_default();
    let enabled = rules.player_swarm.is_some()
        && outcome == MatchOutcome::InProgress
        && !menu.is_some_and(|menu| menu.blocks_world_input);
    if let Some(feedback) = feedback.as_deref_mut() {
        *feedback = BrushFeedback {
            enabled,
            ..default()
        };
    }
    if !enabled || ui_handling.is_pointer_over_ui {
        return;
    }
    let (Some(owner), Some(reachability)) = (rules.player_swarm, reachability) else {
        return;
    };
    let (Ok(window), Ok((transform, camera, movement))) = (windows.single(), cameras.single())
    else {
        return;
    };
    // The game camera has no parent. UI layout runs before transform propagation.
    let camera_pose = GlobalTransform::from(*transform);
    let Some(position) = window
        .cursor_position()
        .and_then(|cursor| camera.viewport_to_world_2d(&camera_pose, cursor).ok())
        .filter(|position| position.is_finite())
    else {
        return;
    };
    let cell = get_zone_pos_from_world(position);
    if !grid.in_bounds(cell) {
        return;
    }
    let gesture = [
        (MouseButton::Left, PlayerIntentAction::Paint),
        (MouseButton::Right, PlayerIntentAction::Erase),
    ]
    .into_iter()
    .find(|(button, _)| mouse.pressed(*button));
    if let Some(feedback) = feedback.as_deref_mut() {
        feedback.hover = Some(BrushHover {
            cell,
            accepts_action: gesture.is_some_and(|(_, action)| action == PlayerIntentAction::Erase)
                || reachability.contains(owner, cell, &grid),
        });
        feedback.active_gesture = gesture.is_some();
    }
    let Some((button, action)) = gesture else {
        return;
    };

    let kind = brush_selection.kind;
    let view_revision = movement.map_or(0, |movement| movement.view_revision);
    let start = previous_sample
        .filter(|sample| {
            sample.action == action
                && sample.kind == kind
                && sample.view_revision == view_revision
                && !mouse.just_pressed(button)
                && !reachability.is_changed()
        })
        .map_or(position, |sample| sample.position);
    *previous = Some(BrushSample {
        position,
        action,
        kind,
        view_revision,
    });

    let mut unreachable = false;
    for cell in crossed_cells(start, position) {
        unreachable |= apply_player_intent(
            &mut grid,
            &reachability,
            outcome,
            *rules,
            cell,
            kind,
            action,
        ) == Err(PlayerIntentError::Unreachable);
    }
    if unreachable && let Some(mut feedback) = feedback {
        feedback.unreachable = true;
    }
}

fn crossed_cells(start: Vec2, end: Vec2) -> impl Iterator<Item = IVec2> {
    let mut current = get_zone_pos_from_world(start);
    let target = get_zone_pos_from_world(end);
    let step = (target - current).signum();
    let delta = end - start;
    let mut finished = false;
    std::iter::from_fn(move || {
        if finished {
            return None;
        }
        let cell = current;
        finished = current == target;
        if !finished {
            // Compare where the pointer segment crosses the next edge on each axis.
            let next_edge = (current + step.max(IVec2::ZERO)).as_vec2() * ZONE_BLOCK_SIZE;
            let t = (next_edge - start) / delta;
            let x = if current.x == target.x {
                f32::INFINITY
            } else {
                t.x
            };
            let y = if current.y == target.y {
                f32::INFINITY
            } else {
                t.y
            };
            if x <= y {
                current.x += step.x;
            }
            if y <= x {
                current.y += step.y;
            }
        }
        Some(cell)
    })
}
