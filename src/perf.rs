mod recorder;

use std::time::{Duration, Instant};

use bevy::{
    app::{MainScheduleOrder, SpawnScene},
    ecs::schedule::{InternedScheduleLabel, ScheduleLabel},
    prelude::*,
    state::state::StateTransition,
};

use crate::{
    agent_control::{AgentControlClock, AgentControlClockPlugin},
    nanobot::{Nanobot, Structure},
    runtime::HeadlessPacing,
    session_lifecycle::SessionGeneration,
};

pub use recorder::{FrameRecord, FrameRecorder, HitchDetection, HitchEpisode, ScheduleTimes};

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
struct ScheduleBoundary {
    measured: InternedScheduleLabel,
    end: bool,
}

impl Plugin for HitchRecorderPlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<AgentControlClockPlugin>() {
            app.add_plugins(AgentControlClockPlugin);
        }
        app.init_resource::<RecorderClock>()
            .init_resource::<FrameMeasurements>()
            .init_resource::<RecordedHitches>()
            .add_systems(StartRecordingFrame, start_recording_frame);
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
        for (schedule, time) in measured {
            let mut order = app.world_mut().resource_mut::<MainScheduleOrder>();
            let Some(index) = order.labels.iter().position(|label| *label == schedule) else {
                continue;
            };
            let start = ScheduleBoundary {
                measured: schedule,
                end: false,
            };
            let end = ScheduleBoundary {
                measured: schedule,
                end: true,
            };
            order
                .labels
                .splice(index..=index, [start.intern(), schedule, end.intern()]);
            app.add_systems(start, start_schedule).add_systems(
                end,
                move |clock: Res<RecorderClock>, mut measurements: ResMut<FrameMeasurements>| {
                    let duration = elapsed_ns(measurements.schedule_start, clock.now());
                    *time(&mut measurements.schedules) = duration;
                },
            );
        }
    }
}

fn start_recording_frame(
    clock: Res<RecorderClock>,
    counters: Res<AgentControlClock>,
    time: Res<Time<Virtual>>,
    generation: Option<Res<SessionGeneration>>,
    pacing: Option<Res<HeadlessPacing>>,
    mut measurements: ResMut<FrameMeasurements>,
    mut hitches: ResMut<RecordedHitches>,
    nanobots: Query<(), With<Nanobot>>,
    structures: Query<(), With<Structure>>,
) {
    // Bevy updates the render sub-app after the main schedule. The next frame
    // start is the first common boundary after that work in both runtime modes.
    let now = clock.now();
    if let Some(start) = measurements.frame_start.replace(now) {
        let frame_ns = elapsed_ns(start, now);
        let schedule_ns = std::mem::take(&mut measurements.schedules);
        let generation = generation.map_or(0, |generation| generation.0);
        if generation != measurements.session_generation {
            // A replacement runs in PreUpdate, within the completed frame.
            measurements.session_generation = generation;
            measurements.session_started_after_frame = counters.frame - 1;
            hitches.0.reset_session();
        }
        let fixed_ticks_run = counters.fixed_tick - measurements.previous_fixed_tick;
        let session_frames = counters.frame - measurements.session_started_after_frame;
        let detection = if time.is_paused() || session_frames <= SESSION_WARMUP_FRAMES {
            HitchDetection::Excluded
        } else if pacing.is_some_and(|pacing| pacing.unbounded) {
            HitchDetection::Unbounded {
                fixed_tick_ns: if fixed_ticks_run > 0 {
                    schedule_ns.run_fixed_main_loop
                } else {
                    0
                },
            }
        } else {
            HitchDetection::Paced
        };
        hitches.0.record(
            FrameRecord {
                frame: counters.frame,
                fixed_tick: counters.fixed_tick,
                frame_ns,
                fixed_ticks_run,
                unmeasured_ns: frame_ns - schedule_ns.total(),
                schedule_ns,
                nanobots: nanobots.iter().len(),
                structures: structures.iter().len(),
            },
            detection,
        );
    }
    measurements.previous_fixed_tick = counters.fixed_tick;
}

fn start_schedule(clock: Res<RecorderClock>, mut measurements: ResMut<FrameMeasurements>) {
    measurements.schedule_start = clock.now();
}

fn elapsed_ns(start: Duration, end: Duration) -> u64 {
    u64::try_from((end - start).as_nanos()).unwrap_or(u64::MAX)
}
