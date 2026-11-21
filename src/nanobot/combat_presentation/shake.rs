use std::{
    collections::{HashMap, VecDeque},
    f32::consts::TAU,
    time::Duration,
};

use bevy::{
    prelude::*,
    render::{Extract, ExtractSchedule, RenderApp, camera::extract_cameras, view::ExtractedView},
};

use crate::{
    nanobot::{
        CombatAppearance, NonCombatDestruction, ResolvedCombatFact,
        motion_presentation::CombatTickSnapshots,
    },
    tuning::Tuning,
};

use super::{CombatView, render_anchor::CombatCameras, resolve_combat, update_combat_view};

const CASUALTY_CELL_SIZE: f32 = 256.0;
const CASUALTY_THRESHOLD: usize = 10;
const CASUALTY_WINDOW: Duration = Duration::from_millis(300);
const SHAKE_DURATION: Duration = Duration::from_millis(150);
const SHAKE_PIXELS: f32 = 4.0;

#[derive(Default, Resource)]
struct CameraShake {
    started: Option<Duration>,
    recent_deaths: HashMap<IVec2, VecDeque<Duration>>,
    offset_pixels: Vec2,
}

pub(super) struct CombatShakePlugin;

impl Plugin for CombatShakePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CameraShake>().add_systems(
            Update,
            update_shake
                .after(update_combat_view)
                .before(resolve_combat),
        );

        if let Some(render) = app.get_sub_app_mut(RenderApp) {
            render.add_systems(ExtractSchedule, extract_shake.after(extract_cameras));
        }
    }
}

pub(super) fn reset_session(world: &mut World) {
    crate::nanobot::reset_resource::<CameraShake>(world);
}

fn update_shake(
    time: Option<Res<Time>>,
    tuning: Res<Tuning>,
    view: Res<CombatView>,
    snapshots: Res<CombatTickSnapshots>,
    (mut facts, mut other_losses): (
        MessageReader<ResolvedCombatFact>,
        MessageReader<NonCombatDestruction>,
    ),
    mut shake: ResMut<CameraShake>,
    mut casualties: Local<Vec<(Duration, Vec2)>>,
) {
    let intensity = tuning.presentation.camera_shake_intensity;
    let Some(bounds) = view.bounds.filter(|_| intensity > 0.0) else {
        facts.clear();
        other_losses.clear();
        if shake.started.is_some() || !shake.recent_deaths.is_empty() {
            *shake = CameraShake::default();
        }
        return;
    };
    let now = time.map_or(Duration::ZERO, |time| time.elapsed());
    let mut triggered = false;
    for fact in facts.read() {
        let ResolvedCombatFact::Death(death) = fact else {
            continue;
        };
        let victim = &death.victim;
        if !bounds.contains(victim.position) {
            continue;
        }
        match victim.appearance {
            CombatAppearance::Structure(_) => triggered = true,
            CombatAppearance::Nanobot(_) => {
                let occurred = snapshots.death_time(victim.entity).unwrap_or(now);
                casualties.push((occurred, victim.position));
            }
        }
    }
    for loss in other_losses.read() {
        match *loss {
            NonCombatDestruction::Structure { position } => triggered |= bounds.contains(position),
            NonCombatDestruction::Nanobot {
                victim,
                occurred_at,
            } if bounds.contains(victim.position) => {
                casualties.push((occurred_at, victim.position));
            }
            _ => {}
        }
    }
    casualties.sort_unstable_by_key(|&(occurred, _)| occurred);
    for (occurred, position) in casualties.drain(..) {
        let cell = (position / CASUALTY_CELL_SIZE).floor().as_ivec2();
        let deaths = shake.recent_deaths.entry(cell).or_default();
        expire_deaths(deaths, occurred);
        let below_threshold = deaths.len() < CASUALTY_THRESHOLD;
        if !below_threshold {
            deaths.pop_front();
        }
        deaths.push_back(occurred);
        triggered |= below_threshold && deaths.len() == CASUALTY_THRESHOLD;
    }
    shake.recent_deaths.retain(|_, deaths| {
        expire_deaths(deaths, now);
        !deaths.is_empty()
    });

    if triggered {
        shake.started = Some(now);
    }
    shake.started = shake
        .started
        .filter(|&started| now - started < SHAKE_DURATION);
    shake.offset_pixels = shake.started.map_or(Vec2::ZERO, |started| {
        let progress = (now - started).as_secs_f32() / SHAKE_DURATION.as_secs_f32();
        Vec2::from_angle(progress * TAU * 2.5) * (intensity * SHAKE_PIXELS * (1.0 - progress))
    });
}

fn expire_deaths(deaths: &mut VecDeque<Duration>, now: Duration) {
    let oldest = now.saturating_sub(CASUALTY_WINDOW);
    while deaths.front().is_some_and(|&death| death < oldest) {
        deaths.pop_front();
    }
}

fn extract_shake(mut commands: Commands, cameras: CombatCameras, shake: Extract<Res<CameraShake>>) {
    let offset_pixels = shake.offset_pixels;
    if offset_pixels == Vec2::ZERO {
        return;
    }

    for (_, render) in &cameras {
        let entity = render.id();
        // Offset the extracted world camera after Bevy queues its unshaken view.
        commands.queue(move |world: &mut World| {
            let Some(mut view) = world.get_mut::<ExtractedView>(entity) else {
                return;
            };
            let clip_scale = Vec2::new(view.clip_from_view.x_axis.x, view.clip_from_view.y_axis.y);
            let viewport_size = Vec2::new(view.viewport.z as f32, view.viewport.w as f32);
            let offset = (2.0 * offset_pixels / (viewport_size * clip_scale)).extend(0.0);
            view.world_from_view = view
                .world_from_view
                .mul_transform(Transform::from_translation(offset));
        });
    }
}
