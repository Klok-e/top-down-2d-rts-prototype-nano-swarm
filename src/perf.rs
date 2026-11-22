mod fixed_phases;
mod recorder;

#[cfg(feature = "perf-trace")]
mod trace;
#[cfg(feature = "perf-trace")]
pub(crate) use trace::trace_main_world_frames;

use std::time::{Duration, Instant};

use bevy::{
    app::{MainScheduleOrder, SpawnScene},
    ecs::schedule::{InternedScheduleLabel, ScheduleLabel},
    prelude::*,
    state::state::StateTransition,
};

use crate::{
    agent_control::{AgentControlClock, AgentControlClockPlugin},
    battle_statistics::BattleRun,
    nanobot::{Nanobot, Structure},
    runtime::HeadlessPacing,
    session_lifecycle::SessionGeneration,
};

pub use recorder::{
    FixedPhaseTimes, FrameRecord, FrameRecorder, HitchDetection, HitchEpisode, HitchEpisodeSummary,
    ScheduleTimes,
};

const SESSION_WARMUP_FRAMES: u64 = 60;

#[derive(Resource)]
pub struct RecorderClock(ClockSource);

enum ClockSource {
    Monotonic(Instant),
    Manual(Duration),
}

impl Default for RecorderClock {
    fn default() -> Self {
        Self(ClockSource::Monotonic(Instant::now()))
    }
}

impl RecorderClock {
    pub fn manual() -> Self {
        Self(ClockSource::Manual(Duration::ZERO))
    }

    pub fn advance(&mut self, duration: Duration) {
        let ClockSource::Manual(elapsed) = &mut self.0 else {
            panic!("only a manual recorder clock can be advanced");
        };
        *elapsed += duration;
    }

    pub fn now(&self) -> Duration {
        match self.0 {
            ClockSource::Monotonic(started) => started.elapsed(),
            ClockSource::Manual(elapsed) => elapsed,
        }
    }
}

pub struct HitchRecorderPlugin;

#[derive(Resource, Default)]
pub(crate) struct RecordedHitches(pub FrameRecorder);

#[derive(Resource, Default)]
struct FrameMeasurements {
    frame_start: Option<Duration>,
    schedule_start: Duration,
    schedules: ScheduleTimes,
    previous_fixed_tick: u64,
    session_generation: u64,
    session_started_after_frame: u64,
}

#[derive(ScheduleLabel, Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct StartRecordingFrame;

#[derive(ScheduleLabel, Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct ScheduleBoundary(usize);

impl Plugin for HitchRecorderPlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<AgentControlClockPlugin>() {
            app.add_plugins(AgentControlClockPlugin);
        }
        app.init_resource::<RecorderClock>()
            .init_resource::<FrameMeasurements>()
            .init_resource::<RecordedHitches>()
            .add_systems(StartRecordingFrame, start_recording_frame);
        fixed_phases::install(app);
        app.world_mut()
            .resource_mut::<MainScheduleOrder>()
            .insert_before(First, StartRecordingFrame);

        type ScheduleTiming = fn(&mut ScheduleTimes) -> &mut u64;
        let measured: [(InternedScheduleLabel, ScheduleTiming); 8] = [
            (First.intern(), |times| &mut times.first),
            (PreUpdate.intern(), |times| &mut times.pre_update),
            (StateTransition.intern(), |times| {
                &mut times.state_transition
            }),
            (RunFixedMainLoop.intern(), |times| {
                &mut times.run_fixed_main_loop
            }),
            (Update.intern(), |times| &mut times.update),
            (SpawnScene.intern(), |times| &mut times.spawn_scene),
            (PostUpdate.intern(), |times| &mut times.post_update),
            (Last.intern(), |times| &mut times.last),
        ];
        let timing = |label: &InternedScheduleLabel| {
            measured
                .iter()
                .find_map(|(schedule, time)| (schedule == label).then_some(*time))
        };
        let labels =
            std::mem::take(&mut app.world_mut().resource_mut::<MainScheduleOrder>().labels);
        let mut order = Vec::new();
        let mut end: Option<ScheduleTiming> = None;
        for (index, schedule) in labels.into_iter().map(Some).chain([None]).enumerate() {
            let start = schedule.as_ref().and_then(timing);
            let starts_next = start.is_some();
            if starts_next || end.is_some() {
                let boundary = ScheduleBoundary(index);
                let ends_previous = end;
                order.push(boundary.intern());
                app.add_systems(
                    boundary,
                    move |clock: Res<RecorderClock>,
                          mut measurements: ResMut<FrameMeasurements>| {
                        let now = clock.now();
                        if let Some(time) = ends_previous {
                            let duration = elapsed_ns(measurements.schedule_start, now);
                            *time(&mut measurements.schedules) = duration;
                        }
                        if starts_next {
                            measurements.schedule_start = now;
                        }
                    },
                );
            }
            order.extend(schedule);
            end = start;
        }
        app.world_mut().resource_mut::<MainScheduleOrder>().labels = order;
    }
}

#[allow(clippy::too_many_arguments)]
fn start_recording_frame(
    clock: Res<RecorderClock>,
    counters: Res<AgentControlClock>,
    time: Option<Res<Time<Virtual>>>,
    generation: Option<Res<SessionGeneration>>,
    pacing: Option<Res<HeadlessPacing>>,
    mut measurements: ResMut<FrameMeasurements>,
    mut fixed_phases: ResMut<fixed_phases::FixedMeasurements>,
    mut hitches: ResMut<RecordedHitches>,
    nanobots: Query<(), With<Nanobot>>,
    structures: Query<(), With<Structure>>,
    battle: Option<ResMut<BattleRun>>,
    mut exit: MessageWriter<AppExit>,
) {
    // Bevy updates the render sub-app after the main schedule. The next frame
    // start is the first common boundary after that work in both runtime modes.
    let now = clock.now();
    if let Some(start) = measurements.frame_start.replace(now) {
        let frame_ns = elapsed_ns(start, now);
        let schedule_ns = std::mem::take(&mut measurements.schedules);
        let (fixed_phase_ns, longest_tick_ns) = fixed_phases.take_frame();
        let generation = generation.map_or(0, |generation| generation.0);
        let session_replaced = generation != measurements.session_generation;
        if session_replaced {
            // A replacement runs in PreUpdate, within the completed frame.
            measurements.session_generation = generation;
            measurements.session_started_after_frame = counters.frame - 1;
            hitches.0.reset_session();
        }
        let fixed_ticks_run = counters.fixed_tick - measurements.previous_fixed_tick;
        let session_frames = counters.frame - measurements.session_started_after_frame;
        let paused = time.is_some_and(|time| time.is_paused());
        let detection = if paused || session_frames <= SESSION_WARMUP_FRAMES {
            HitchDetection::Excluded
        } else if pacing.is_some_and(|pacing| pacing.unbounded) {
            HitchDetection::Unbounded {
                fixed_tick_ns: longest_tick_ns,
            }
        } else {
            HitchDetection::Paced
        };
        let closed = hitches.0.record(
            FrameRecord {
                frame: counters.frame,
                fixed_tick: counters.fixed_tick,
                frame_ns,
                fixed_ticks_run,
                unmeasured_ns: frame_ns - schedule_ns.total(),
                schedule_ns,
                fixed_phase_ns,
                nanobots: nanobots.iter().len(),
                structures: structures.iter().len(),
            },
            detection,
        );
        if !session_replaced
            && let Some(mut battle) = battle
            && let Err(error) =
                battle.record_frame(frame_ns, paused, closed.map(HitchEpisode::summary))
        {
            error!("Cannot append to the battle hitch log: {error}");
            battle.error = Some(error.to_string());
            exit.write(AppExit::error());
        }
    }
    measurements.previous_fixed_tick = counters.fixed_tick;
}

fn elapsed_ns(start: Duration, end: Duration) -> u64 {
    u64::try_from((end - start).as_nanos()).unwrap_or(u64::MAX)
}
