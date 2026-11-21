# Playtest: visual and control issues (2026-11-21)

All 36 findings are addressed in the [fixes and regression coverage](#fixes-and-regression-coverage) below.

Played from commit `ad10fa9`. I drove a headless build (`--headless --agent-socket`, 1280×720, plus one run at 800×600) through Standard, Sandbox, and AI Battle. I used socket painting, menu and tuning commands, camera moves, and screenshots. I did not drive mouse or keyboard input in a real window, so findings marked **[code]** come from reading the input code, not from a live repro.

Screenshots are in `.tmp/playtest-2026-11-21/sNN_*.png` (git-ignored, so they exist only on this machine). The `sNN` IDs below refer to them.

Severity: **H** = misleads the player or hides important state; **M** = noticeably hurts readability or control; **L** = polish.

## A. Intent zone rendering (`assets/shaders/zone_shader.wgsl`)

| # | Sev | Issue | Evidence |
|---|-----|-------|----------|
| A1 | H | **Overlapping player intents blend into ambiguous colors.** `intent_color` averages the RGB of every present layer. Defend+Corridor gives neutral **gray** (0.5,0.5,0.5), which looks like rock or a planned-structure marker. Gather+Build gives pink, close to plain Build. Gather+Defend gives purple, close to Build. Gather+Build+Defend gives a muddy mauve. The player cannot tell which layers a cell holds. | s11 (left gray cell = defend+corridor, orange = gather+corridor, pink = gather+build), s25 (mauve AI Battle home) |
| A2 | H | **Contested cells use pure red stripes, which match Gather red.** When player and opponent share a cell, the hatch color is (1.0,0.12,0.08). On a Gather cell, contested and uncontested look almost the same. On other cells, the red stripe suggests "gather". | s11 cell (5,5) has player gather plus 3 enemy layers but looks plain red; s09, s15 |
| A3 | M | **Opponent-only cells lose their intent identity.** The shader mixes 40% toward red and darkens by half, so enemy Defend, Build, and Gather all look like similar dark purple or brown hatching. The player can't read what the enemy is doing (for example, attack paint over their base). | s09, s15, s17 |
| A4 | M | **Zone fill is too opaque and saturated.** Large zones (AI Battle Defend, Standard enemy Defend) flood the whole screen with strong blue or purple. Bots, structures, health bars, and the planned-structure ghost lose contrast. FPS also dropped to about 39 in s09, which may be unrelated. | s09, s25, s26 |
| A5 | M | **No legend or ownership key.** Nothing on screen explains solid vs hatched, what the colors mean when blended, or which swarm owns which hatch. | all |
| A6 | L | Shader colors are pure primaries, but the button colors are muted (`intent_layer_panel.rs:50`). The doc comment there says they "match the zone shader palette", and they don't quite match. | s04 vs zones |

## B. Unit, structure, and overlay readability

| # | Sev | Issue | Evidence |
|---|-----|-------|----------|
| B1 | H | **Enemy structures look identical to the player's.** Completed structures get one shared green tint (`completed_visual_color()`, `src/nanobot/structure_lifecycle.rs:696`) whatever the owner. Two opponent chargers inside the player's base looked like the player's own buildings. Only the bots are tinted (pink vs native). | s15, s16 (enemy chargers, confirmed via `state --details`), s17 |
| B2 | H | **Built Production Facilities look nothing like the starting one.** The scenario facility is a large gray-and-cyan sprite at scale 3 (`src/scenario.rs:351-374`). A facility built from a plan is a small green square. A player can't tell that the new buildings are facilities. | s20, s22 |
| B3 | M | **Stacked bars above structures have no meaning cue.** Up to three thin bars sit above a footprint: health, maintenance, and worker progress. Health and maintenance use the same green, orange, and red ramp (`src/structure_overlay.rs:165-182`), so they can't be told apart. The scenario facility also draws a bar inside its own sprite and a dark empty bar above the selection box (nearly invisible on magenta). | s03, s08, s21, s22 |
| B3a | M | **Stockpile bars overlap each other.** The 6 px resource-fill bar (cyan, or cyan with orange) is drawn over the lower 3 px condition bar (green), hiding about 2 px of it. The bars should sit in separate rows. The code computes the resource bar's position in `overlay_label_offset_y`, plus a target-scale correction (`src/structure_overlay.rs:686-694`), and the condition bars' positions separately in `condition_overlay_offset_y` (`:566`), which has no scale correction. Measured on screen, the resource bar sits about 7 px higher than that code predicts, so the cause isn't confirmed yet. The suspect is the scale correction applied to the resource bar but not the condition bars. | 8× crops in `.tmp/playtest-2026-11-21/crop_*.png` (from s08, s21, s22) |
| B4 | M | **Completed structure kinds are hard to tell apart.** Source stockpile, sink stockpile, charger, and new facility are all the same size, the same dark green, and sit in the same white frame. They differ only by a tiny glyph. | s08, s20, s22 |
| B5 | M | **Planned structures are nearly invisible at normal zoom.** At zoom ≥2 the planned ghost is a faint outline with a thin bar, and it disappears on blue or magenta zones. | s20 (planned charger at about 822,138 not visible), s21 at zoom 1 is barely visible |
| B6 | M | **Far-zoom tactical markers reuse intent colors.** `cluster_color` (`src/tactical_overlay.rs:321`) uses red for the opponent base (the same as the Gather button), blue for the player base (≈ Defend), purple for chargers (≈ Build), and gray for planned structures (≈ the A1 gray). The markers are unlabeled translucent squares that sit on top of zones, so at zoom ≥8 the player can't tell a structure marker from a painted cell. | s02, s06, s07, s28 |
| B7 | L | Tactical deposit markers have a fixed screen size, so at high zoom they extend past the map's outer wall. | s07, s24 |
| B8 | L | The deposit's remaining-amount bar uses the same cyan as bot charge bars and looks like a health bar. | s01, s08 |
| B9 | L | Nothing distinguishes worker, hauler, and defender roles except shape. The HUD abbreviations W, H, and D are never explained on screen. | s01 |

## C. HUD and feedback

| # | Sev | Issue | Evidence |
|---|-----|-------|----------|
| C1 | H | **Nothing warns the player that their swarm is under attack or wiped out.** In Standard with default starting intents and no input, the opponent painted Defend over the player base at about 1 minute. All player bots were dead by about 3 minutes (`Population: W0 H0 D0`). The defeat banner appeared only at about 5 minutes, once the last structure died. Between those points the HUD gave no alert, and the player sat with 249 minerals and "Production: waiting for delivery". Production needs haulers that no longer exist, and the HUD doesn't say so. | s12, s15, s16 |
| C2 | M | **Painting an unreachable cell gives no feedback.** `zone_brush_system` discards the `apply_player_intent` result (`src/zones/zone_brush.rs:265`, `let _ =`). Cells partly covered by rock (for example cell (1,5), which is mostly open floor) are silently refused. **[code]** | socket returned `unreachable_intent_cell` for (1,5), (2,5), (-2,-1) |
| C3 | M | **No hover preview of the brush cell.** The cell grid appears only while a button is held (`src/materials.rs:20`), so the player can't see which 512-unit cell they will paint before clicking. **[code]** | — |
| C4 | M | **The Defeat/Victory banner blocks the center of the screen for good.** It can't be dismissed, and it doesn't say how to continue ("Esc for menu"). After the match ends, the intent buttons still look active even though every paint is rejected. | s16, s17 |
| C5 | L | **The HUD panel changes width with its text** ("demand met" vs "waiting for delivery"), so it jumps while playing. "Production: H x2" is cryptic. "Deposits: 72000" totals every deposit on the map, including the enemy's. | s01, s04, s08 |
| C6 | L | The FPS counter is always visible and can't be toggled. | all |
| C7 | L | AI Battle HUD: nothing links "Swarm 0" and "Swarm 1" to their on-screen colors (blue vs pink bots). Swarm 0's zones render solid and Swarm 1's render hatched, which is asymmetric for a neutral spectator view. | s25, s26 |
| C8 | L | Fonts are mixed: the HUD and buttons use Fira Sans, while the scenario menu and tuning panel use a monospace font. | s12, s13 |
| C9 | L | A doc comment says the intent panel has a "Swarm Intent" label (`intent_layer_panel.rs:94`), but no label is rendered. | s01 |

## D. Camera controls (`src/fly_camera/`)

| # | Sev | Issue | Evidence |
|---|-----|-------|----------|
| D1 | M | **The camera is unbounded.** It can pan to (-50000,-50000), where only empty floor shows, and there is no "return home" key. The textured void outside the map looks exactly like playable floor, and only a thin wall marks the edge. | s23, s07 |
| D2 | M | **Max zoom of 100 is far too wide.** The whole playable map is a small box in the middle of the screen. Useful max is about 30. Min zoom 1 is fine. | s24 |
| D3 | M | **The wheel zooms around the screen center, not the cursor** (`camera_zoom.rs:28-41`). **[code]** | — |
| D4 | M | **Panning is keyboard-only (WASD).** There are no arrow keys, no middle or right drag, and no screen-edge pan. Panning has acceleration (1800) and friction (1050), so it accelerates slowly and keeps drifting after the key is released (`config/balance.ron:116-118`). Crossing the map at zoom 2 takes about 12 s. **[code]** | — |
| D5 | L | The opening camera (256,256 @2) cuts off the home Gather zone and deposit at the bottom of the screen, so the starting base isn't framed. | s01, s19 |

## E. Painting controls (`src/zones/zone_brush.rs`)

| # | Sev | Issue | Evidence |
|---|-----|-------|----------|
| E1 | M | **A fast drag skips cells.** Each frame paints only the cell under the cursor. Nothing interpolates between the previous and current cursor position, so a quick stroke leaves gaps. **[code]** | — |
| E2 | L | Right-click erases only the *selected* kind. Nothing on screen says so, so a right-click on a Defend cell with the Gather brush selected appears to do nothing. **[code]** | — |
| E3 | L | While a tuning field has focus, `is_pointer_over_ui` is true everywhere (`ui_interaction_system.rs:19-20`). The first world click only clears focus and paints nothing. **[code]** | — |

## F. Menus and panels

| # | Sev | Issue | Evidence |
|---|-----|-------|----------|
| F1 | M | **The tuning panel covers the intent buttons in narrow windows.** At 800×600 the panel hides "3: Defend" and "4: Corridor" and clips "2: Build". The panel has a fixed width of about 430 px, more than half of an 800 px window. | s29 |
| F2 | L | The tuning panel's dark background fills the full window height even when its content is short. Unit labels ("world") touch the right edge. The panel has no title and no "F2 to close" hint. | s13, s14 |
| F3 | L | The scenario menu marks the selected scenario only with a thin teal border, which is easy to miss next to the plain borders. The title says "Paused" even after the match has ended. | s12, s18 |

## Suggested order

Fix in this order: A1, A2, and B1 (they actively mislead the player), then C1, B2, B6, and A3. After that, D1–D3, E1, F1, and C2–C4. The rest is polish.

## Fixes and regression coverage

| Findings | What changed | Checks |
|---|---|---|
| A1–A6, C7, C9, E2 | Each owner gets a row of intent swatches. The HUD keys the palette and ownership patterns, identifies both spectator swarms, and explains right-click erase. | The [zone renderer test](../../screenshots/zone_binary_overlay.rs) checks overlap colors and both spectator hatch directions. The [session restart playtest](../../tests/playtest/session_restart.rs) exercises `intent_panel_tracks_the_scenario_and_disables_controls_when_the_match_ends`. |
| B1–B5, including B3a | Ownership frames distinguish structures. Seeded and constructed facilities share artwork. Planned structures use bright brackets. The HUD identifies the bar rows, which now share a scale-aware layout. | [Facility construction](../../tests/playtest/defender_feel.rs) exercises `completed_facility_uses_the_same_art_as_the_starting_facility`. [Overlay geometry](../../tests/behavior/structure_overlay.rs) exercises `completed_support_structure_gets_stacked_condition_bars`. The [fill indicator scene](../../screenshots/fill_indicators.rs) renders adjacent structures at normal zoom. |
| B6–B8 | Map landmarks have pictograms, tactical deposit markers fit inside the wall, and deposit amounts use amber. | [Offscreen presentation](../../tests/playtest/offscreen_presentation.rs) exercises `offscreen_presentation_starts_full_scene_without_a_window`, including ownership and marker bounds. |
| C1, C4–C6 | The fixed-width HUD names types and the home deposit, warns of attacks and population loss, and explains a missing delivery crew. F3 toggles FPS. The terminal banner offers Dismiss and a menu hint. | [Opponent gameplay playtests](../../tests/playtest/opponent_gameplay_loop.rs) cover `swarm_hud_warns_of_combat_and_a_missing_delivery_crew`, `mutual_final_combat_deaths_show_a_dismissible_draw_in_the_same_tick`, and `f3_shows_and_hides_the_fps_counter`. |
| D1–D5 | Camera movement stops at the map boundary and zoom stops at 30. The wheel anchors to the pointer. Arrows, WASD, middle drag and screen edges pan. Home restores the base and deposit framing. | The [camera control playtests](../../tests/playtest/camera_controls.rs) exercise camera input, including `wheel_preserves_the_world_point_under_the_cursor` and `arrows_pan_and_home_recovers_the_opening_view`. [Session restart](../../tests/playtest/session_restart.rs) checks home framing in Standard and Flanks at 800×600. |
| C2–C3, E1, E3 | The brush previews the hovered cell, explains an unreachable paint attempt, and fills cells crossed during a drag. The first world click after editing tuning also paints. A camera jump breaks a held stroke. | [Mouse painting](../../tests/playtest/mouse_zone_painting.rs) covers `fast_mouse_strokes_paint_and_erase_every_crossed_cell`, `home_does_not_erase_the_cells_between_camera_views`, and `scripted_cell_preview_and_guides_follow_pointer_actions`. The [tuning playtest](../../tests/playtest/tuning_panel.rs) checks focus release and the [brush screenshot test](../../screenshots/brush_feedback.rs) checks the visible outline and refusal notice. |
| C8, F1–F3 | Tuning fits below the intent controls and scrolls within the viewport. The panels share the HUD font. The scenario menu labels its selection and shows “Match complete” after play ends. | The [tuning captures](../../screenshots/tuning_panel.rs) check 800×600 layout, scroll, and error text. The [scenario menu playtest](../../tests/playtest/scenario_menu.rs) exercises `scenario_selection_updates_the_selected_label_and_start_requests_it`, and the [menu captures](../../screenshots/scenario_menu.rs) show both titles. |

For B9, the HUD names the three bot roles. On the map, each role uses its own sprite artwork without an overlaid icon.

The gameplay HUD tests replace three formatter-only checks and retain coverage for deposit ownership, facility ownership, and carried minerals. Tactical presentation tests exercise the replacement pictograms and ownership tints.

The 72,000 shown in the original HUD was the starting amount in the player's home deposit. The HUD now makes that scope explicit. The reported 39 FPS sample did not establish a cause or a performance target.

Verification passed 982 default Rust tests, three additional GPU playtests, and 48 offscreen screenshot tests. A separate headless process exercised Standard, Sandbox, AI Battle, and 800×600 tuning. An isolated combat configuration drove Standard to Defeat and verified that dismissing the banner preserved the outcome.

## Independent verification (2026-11-21)

I replayed the original scenarios against the uncommitted working tree on top of `ad10fa9`. I used the headless build with the agent socket at 1280×720 (Standard to Defeat, then AI Battle) and again at 800×600 (Sandbox with tuning open). Screenshots are in `.tmp/playtest-2026-11-21/verify/vNN_*.png`.

Test runs:
- `cargo test`: 359, 527, and 96 passed; no failures.
- `cargo test --test playtest -- --ignored`: 3 GPU playtests passed.
- `cargo test --test screenshots -- --ignored`: 48 passed. The one panic in the output is the harness's intentional self-test (`callback_panic_fails`).

**Confirmed fixed in the live game:** A1–A6, B1, B3, B3a, B4–B9, C1, C4–C9, D1, D2, D5, E2, F1–F3.

**Confirmed only through tests and captures** (mouse and keyboard can't be driven over the socket): C2, C3, D3, D4, E1, E3. The tests pass and `brush_feedback_*.png` shows the hover outline and the "Cannot paint an unreachable cell" notice.

**Residual issues** (all minor):
- **R1 (B2, partial).** A facility built from a plan uses the same art as the starting facility but renders at about one third of its size. Compare 832,360 in v04 with the starting facility in v05.
- **R2.** A contested cell is split into two half-height rows: player swatches on top, opponent hatch below. The dividing line looks like a cell border, so one cell reads as two. The legend text "Two rows · Both swarms" doesn't explain this well. See v04 and v11_t2.
- **R3.** The camera clamps only its center to the map. At zoom 30 near a corner, about three quarters of the view is black void (v10). Clamping the visible rectangle where the map is larger than the view would fix this.
- **R4.** The new HUD is about 270×265 px, roughly twice the old panel, and the legend strip spans most of the top. At 800×600 the HUD covers part of the home Gather zone (v17).
- **R5.** Cells that are mostly open floor but marked unreachable, such as (1,5), are still refused. The player now gets feedback, but the rule may be stricter than players expect. This is a gameplay rule, not a rendering bug.
