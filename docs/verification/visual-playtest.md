# Visual playtest observations

| Scene | What to look for |
| --- | --- |
| A Production Facility built from a plan beside the starting facility | Both use the same artwork. The built facility appears about one third as wide as the starting one. Compare them at the same zoom when assessing whether players can recognize the building. |
| A cell painted by both swarms | Each owner has a half-height row. The divider can look like a grid boundary. Compare a shared cell with its single-owner neighbor and check whether both rows read as one cell. |
| A map corner at maximum zoom | Panning stops when the camera center reaches the map edge. At a corner, roughly three quarters of the viewport can show empty space. Compare with the view at Home when assessing camera bounds. |

In the live 800×600 Sandbox replay, the status panel cleared the home Gather Zone and its Resource Deposit. Opening tuning covered the right edge of the zone while the deposit and all four intent buttons stayed visible. To repeat the comparison, launch `cargo run -- --headless --agent-socket --scenario sandbox --width 800 --height 600` and capture the opening view before and after `tuning-toggle`. The automated Standard fixture checks button clearance at the same size. [Control client](../agents/agent-control.md#client) · [Tuning fixture](../../screenshots/tuning_panel.rs)

In Sandbox, use the control client’s `camera -512 3328 2` and `paint defend -2 6`. A blue Defend row appears at the western rock edge because a player nanobot can reach ground inside the tile even though its center is blocked. Compare `state` before and after `paint defend -3 6`. That exterior tile should stay clear and the map revision should stay the same. [Reachable ground for intent](../adr/0028-reachable-ground-for-intent.md)
