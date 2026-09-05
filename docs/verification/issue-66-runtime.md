# Issue #66 real-process acceptance

Verified 2026-09-05 after the final scheduler/cache build using the normal game binary and the existing `local_avoidance` example. Both create only offscreen render targets and expose the real agent-control socket. All player actions, frame synchronization, state reads, screenshots, and successful shutdown requests went through `scripts/nano_swarm_control.py` as a separate process. No direct ECS mutation or compositor input was used by the driver.

## Reproduction

From the repository root:

```bash
CARGO_INCREMENTAL=0 cargo build --bin top-down-2d-rts-prototype-nano-swarm --example local_avoidance
python3 scripts/navigation_runtime_acceptance.py --output target/issue-66/runtime/final
```

Choose a new output directory for a repeated run; the driver refuses to overwrite prior evidence. The recorded acceptance invoked the already-built debug binaries directly, with `target/debug/deps` and `rustc --print target-libdir` on `LD_LIBRARY_PATH`, avoiding concurrent Cargo builds. The normal binary arguments were `--headless --agent-socket --width 1280 --height 720`. The example configures the same presentation and socket internally. `BEVY_ASSET_ROOT` was the repository root. Each process received its own private mode-0700 temporary `XDG_RUNTIME_DIR`; short temporary paths avoid Unix socket path limits.

Each scene directory includes `launch.json`, the exact external commands and responses in `commands.json`, `process.log`, PNGs with matching state JSON, and `cleanup.json`. The driver does not judge screenshot contents; visual inspection is a separate required step. State requests occur after screenshot readback, so their ticks and fast-changing counters can be slightly later than the image.

## Normal economy and intent flow

The driver sets camera `(256, 256)` at zoom `1.5`, presses the Corridor intent button, paints Corridor at `(-1,0)`, `(0,0)`, and `(0,1)`, adds Build at `(-1,1)`, and sets production priority `(worker=0, hauler=0, defender=100)`. It captures the start, then three additional snapshots separated by explicit 600-fixed-tick waits. Corridor layers coexist with the existing Gather/Build layers in returned map state, and the selected intent remains Corridor.

All four `target/issue-66/runtime/final/normal/*.png` files were opened individually and inspected:

| Capture | State tick | Player minerals | Population W/H/D | Observed visual facts |
| --- | ---: | ---: | --- | --- |
| `start` | 46 | 0 | 4/2/3 | Workers move toward the cyan deposit; two Haulers and three Defenders stand outside the production structure. The support structure is an unfinished translucent outline. Corridor is selected and the priority UI reads 100% Defender. |
| `working` | 664 | 115 | 4/2/3 | Workers surround the deposit; the Stockpile and Gather support are solid completed shapes with bars. Two Haulers occupy the open space around them. Friendly and opposing Defenders remain visible to the right. The HUD reads `waiting for delivery`. |
| `delivery` | 1280 | 220 | 4/3/1 | Three Haulers are visible outside structures. A second Gather support and an additional production facility are solid completed shapes. The facility count rose from one to two and the Hauler count rose from two to three. |
| `later` | 1898 | 140 | 4/5/3 | Workers remain around the still-present deposit; five Haulers occupy exterior positions around support and production. The HUD confirms five Haulers and three Defenders. The player has not collapsed. |

The deposit HUD falls from 72,000 to 71,760. Minerals rise from 0 to 220 by the delivery snapshot and are 140 in the later snapshot. Completed support structures, the additional production facility, Haulers increasing from two to five, and Defenders recovering from one to three establish the combined runtime economy, construction, transport, and production wiring. Snapshots show bodies outside rendered structures; continuous swept clearance and exact service distances remain the responsibility of deterministic navigation tests. Screenshot HUD minerals at `working` are 111 while the subsequent state read reports 115, illustrating the readback timing distinction above.

Earlier exploratory and scripted runs are retained under `target/issue-66/runtime/{normal,bottleneck,replay}/`. All twelve of their PNGs were inspected. The exploratory normal run used priorities 60/30/10 and reached production-collapse defeat by state tick 3922; it is not evidence of long-term economy survival. The final evidence above supersedes those runs for the scheduler/cache changes.

## Opposing bottleneck

The unmodified `examples/local_avoidance.rs` provides the actual runtime movement/navigation stack, four bodies, two 288-by-72 structures, and independently checked swept body/structure clearance. As documented for #62, this isolated fixture removes the authored economy and Swarms, supplies original movement goals, and pauses virtual time at each capture phase. Remote Corridor paint at `(-3,-3)` and `(-2,-3)` releases the two phases; these distant cells do not guide the Worker routes.

All three `target/issue-66/runtime/final/bottleneck/*.png` files were opened individually and inspected:

- `before`: four distinct green bodies line the one-cell passage, with two destination markers on either side.
- `retreat`: at fixture tick 23, the rightmost leading body has moved up around the right wall end; three separated bodies remain in the passage. Logged coordinates are `(415,324)`, `(487,324)`, `(564,324)`, `(611,379)`.
- `arrived`: at fixture tick 244, two bodies occupy each side's destination markers and the passage is empty. Logged final coordinates are `(828,324)`, `(900,324)`, `(180,324)`, `(108,324)`.

The example's swept assertions passed every fixed tick through arrival. The final replay reproduced the same fixture ticks and coordinates as the earlier bottleneck runs. All seven final PNGs were opened individually; together with the twelve earlier captures, all nineteen runtime PNGs were inspected.

## Cleanup and limits

Both final scripted processes returned exit code 0 after client `shutdown`. The driver verified the socket was removed and obtained an exclusive nonblocking lock on the retained `control.sock.lock` inode, proving the advisory lock was released. Temporary runtime directories were removed after screenshots and evidence were copied. The four earlier successful process runs also exited 0 and passed the same socket/lock checks. Audio-device warnings in logs did not prevent offscreen rendering or orderly shutdown.

These captures establish representative real-process integration and inspected rendered behavior, not a frame-time or 5,000-Nanobot performance claim. Normal gameplay is wall-clock driven, so repeated snapshot ticks and combat outcomes can differ. The paused bottleneck's fixture ticks are deterministic. The acceptance matrix and benchmarks for #66 supply the separate quantitative navigation/performance evidence.
