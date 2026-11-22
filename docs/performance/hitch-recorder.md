# Hitch recorder

The recorder runs in the windowed and paced headless game, including runs launched without the agent socket. Fetch closed episodes with [`perf.hitches`](../agents/agent-control.md#hitch-episodes).

## Detection and retention

- A frame is a hitch when it takes **more than 33.3 ms** or runs **more than one fixed tick**.
- A hitch opens an episode with up to 120 preceding frames. A later hitch extends it until 30 consecutive hitch-free frames have passed. The episode includes those 30 recovery frames.
- Long episodes retain the first and last 300 frames, for at most 600 detailed records. A gap in the frame numbers marks omitted detail.
- The process keeps the latest 32 closed episodes. Episode IDs increase for the life of the process.

## Frame detail

Each episode contains its `id`, `first_hitch_frame`, `last_hitch_frame`, and `frames` ordered by frame number. Frame records carry these fields.

| Field | Meaning |
| --- | --- |
| `frame`, `fixed_tick` | The same completed-frame and tick counters used by agent control |
| `fixed_ticks_run` | Ticks completed during this frame |
| `frame_ns` | Nanoseconds from the start of this main-world frame to the start of the next |
| `schedule_ns` | Nanoseconds in each main schedule, including the complete fixed loop in `RunFixedMainLoop` |
| `unmeasured_ns` | Frame duration minus the sum of the main-schedule times |
| `nanobots`, `structures` | Nanobot and completed structure entity counts at the end of the frame |

The next frame publishes the previous sample before agent requests run. Unmeasured time includes render work on the frame's critical path, frame pacing, and scheduling overhead. Pipelined windowed rendering can overlap the main world, so this remainder cannot isolate GPU time.

The overhead target is about 30 monotonic clock reads per frame. Main-schedule measurement uses 17 in the real runtime, plus bounded frame storage and entity counts. Opening an episode copies the preceding detail. Closing it joins the retained start and end.
