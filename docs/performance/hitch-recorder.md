# Hitch recorder

The recorder runs in the windowed and headless game, including runs launched without the agent socket. Fetch closed episodes with [`perf.hitches`](../agents/agent-control.md#hitch-episodes).

## Detection and retention

- In paced runs, a frame is a hitch when it takes **more than 33.3 ms** or runs **more than one fixed tick**.
- With `--headless --unbounded`, a fixed tick taking **more than 33.3 ms** is a hitch.
- Frames while virtual time is paused and the first 60 frames of each session are excluded from hitch detection. The exclusions apply to the start of the process, same-scenario restarts, and scenario switches. These frames can still provide context and count toward recovery.
- A hitch opens an episode with up to 120 preceding frames. A later hitch extends it until 30 consecutive hitch-free frames have passed. The episode includes those 30 recovery frames.
- Long episodes retain the first and last 300 frames, for at most 600 detailed records. A gap in the frame numbers marks omitted detail.
- Each new session starts with empty preceding-frame history and discards any still-open episode. The process keeps the latest 32 closed episodes across sessions. Episode IDs increase for the life of the process.

## Frame detail

Each episode contains its `id`, `first_hitch_frame`, `last_hitch_frame`, and `frames` ordered by frame number. Frame records carry these fields.

| Field | Meaning |
| --- | --- |
| `frame`, `fixed_tick` | The same completed-frame and tick counters used by agent control |
| `fixed_ticks_run` | Ticks completed during this frame |
| `frame_ns` | Nanoseconds from the start of this main-world frame to the start of the next |
| `schedule_ns` | Nanoseconds in each main schedule, including the complete fixed loop in `RunFixedMainLoop` |
| `fixed_phase_ns` | Nanoseconds in each fixed-simulation phase, summed across all fixed ticks in the frame |
| `unmeasured_ns` | Frame duration minus the sum of the main-schedule times, which already cover the fixed phases |
| `nanobots`, `structures` | Nanobot and completed structure entity counts at the end of the frame |

The next frame publishes the previous sample before agent requests run. Unmeasured time includes render work on the frame's critical path, frame pacing, and scheduling overhead. Pipelined windowed rendering can overlap the main world, so this remainder cannot isolate GPU time.

## Fixed simulation phases

| Phase | System set or work |
| --- | --- |
| `movement` | `NanobotSimulationSet::Movement` |
| `threat` | `NanobotSimulationSet::Threat` |
| `combat` | `NanobotSimulationSet::Combat` |
| `maintenance` | `NanobotSimulationSet::Maintenance` |
| `allocation_project` | `RegionalAllocationSet::Project` |
| `allocation_invalidate` | `RegionalAllocationSet::Invalidate` |
| `allocation_acquire` | `RegionalAllocationSet::Acquire` |
| `strategic_controller` | `StrategicControllerSet` |
| `fixed_other` | Other fixed systems and remaining time inside each fixed tick, including the other fixed schedules |

The overhead target is about 30 monotonic clock reads per frame. The runtime uses 17 for the outer frame and main schedules, two per fixed tick, and one per phase transition during `FixedUpdate`. Catch-up frames repeat the fixed measurements for each tick. Opening an episode copies the preceding detail. Closing it joins the retained start and end.

## Per-system trace

Enable `perf-trace` for a short profiling run when the recorder's phase timings need a closer look. Normal builds leave Chrome tracing disabled. The INFO filter keeps the frame and system spans that Bevy records.

```bash
mkdir -p .tmp
trace_run=$(mktemp -d .tmp/perf-trace.XXXXXX)
RUST_LOG=info,wgpu=error,naga=warn TRACE_CHROME="$PWD/$trace_run/trace.json" \
  cargo run --features perf-trace -- --headless --agent-socket --scenario ai-battle \
  --output-root "$trace_run/battle"
```

While the game is running, fetch a closed episode with `python3 scripts/nano_swarm_control.py perf-hitches`. Use `python3 scripts/nano_swarm_control.py shutdown` to stop cleanly and flush the trace. Traces can grow quickly, and tracing overhead can affect frame times.

```bash
python3 scripts/summarize_perf_trace.py "$trace_run/trace.json"
python3 scripts/summarize_perf_trace.py "$trace_run/trace.json" --frames 101 230
```

Use the first and last frame numbers in the episode's `frames` list as the inclusive range. Bevy writes the recorder's frame number in the event name, such as `perf_frame: frame=101`. The slowest-frame ranking measures the main-world span. The recorder measures the start-to-start interval, which also includes pacing and work outside that span. The range report shows how many of the requested frames were captured and ranks system invocations contained in those frame spans across all threads. Total sums their durations and peak is the longest invocation. Parallel and nested system durations can overlap.
