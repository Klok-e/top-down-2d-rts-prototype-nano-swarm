use bevy::{app::Main, ecs::schedule::ScheduleLabel, log::info_span, prelude::*};

use crate::agent_control::{AgentControlClock, AgentControlClockPlugin};

#[derive(ScheduleLabel, Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct TraceMainWorldFrame;

pub(crate) fn trace_main_world_frames(app: &mut App) {
    if !app.is_plugin_added::<AgentControlClockPlugin>() {
        app.add_plugins(AgentControlClockPlugin);
    }
    app.add_systems(TraceMainWorldFrame, trace_frame);
    app.main_mut().update_schedule = Some(TraceMainWorldFrame.intern());
}

fn trace_frame(world: &mut World) {
    // The clock advances in Last, so the frame entering Main is its successor.
    let frame = world
        .resource::<AgentControlClock>()
        .frame
        .saturating_add(1);
    let _span = info_span!("perf_frame", frame).entered();
    world.run_schedule(Main);
}
