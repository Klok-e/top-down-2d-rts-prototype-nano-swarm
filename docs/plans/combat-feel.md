# Combat feel: proposals

Status: accepted direction, 2026-11-21. Nothing here is implemented yet. The user approved every open decision (see [Decisions](#decisions)). Items marked **[ADR]** need ADR 0015 amended, and the bolt wording in ADR 0014 refined, as part of their implementation.

## Goal

Both a 4-vs-4 skirmish and a 300-vs-300 battle should look fluid and exciting at every zoom the player uses.

"Fluid" here means four things:

1. Motion is continuous on any display refresh rate.
2. Units turn instead of snapping.
3. Effects never strobe.
4. The frame rate holds during the largest fights.

"Exciting" also means four things:

1. You can see who is shooting whom.
2. Every kill reads as an event.
3. A battle has a shape: a front, a surge, a collapse.
4. A huge battle is legible from the strategic zoom.

## How I evaluated it

- I read `src/nanobot/combat.rs`, `combat_presentation.rs`, `move_system.rs`, `allocation/defender_response.rs`, `charge.rs`, `config/balance.ron`, ADR 0014 and ADR 0015.
- I played AI Battle headless through the agent socket until first contact (about tick 9,900) and captured bursts at zoom 1.
- I ran a throwaway offscreen fixture: 300 vs 300 full-charge Defenders on mutually painted Defend cells, plus a separate 4-vs-4 skirmish. It captured frames from zoom 0.9 to 10 and logged statistics every 30 ticks. The fixture is in `.tmp/massbattle/repo/screenshots/mass_battle.rs`; that directory is git-ignored, so the file exists only on this machine. The frames are in `.tmp/massbattle/repo/target/playtest-screenshots/mass_*.png`.

## What combat is today

### Simulation

| Fact | Value | Source |
|---|---|---|
| Who can attack | Only Defenders that currently hold a `DefenderResponse` | `combat.rs`, `defender_combat_system` |
| Range | 96 world units (a bot is 32 wide) | `balance.ron` `attack_range` |
| Cadence | One hit every 30 ticks (0.5 s); the first hit lands immediately | `attack_interval_ticks` |
| Damage | 10 attack / (1 + 10 defense / 10) = 5 HP against a full-charge Defender | `damage_after_defense` |
| Equal duel | 20 hits, about 9.5–10 s | derived |
| Hit model | Instant hitscan against the nearest hostile in range | `defender_combat_system` |
| Response pairing | Each Threat gets **at most one** responder, assigned greedily (most dangerous, then nearest unclaimed) | ADR 0015, `choose_response_target` |
| Engagement movement | Move toward the assigned Threat and stop at `attack_range`, then stand still | `stop_radius = attack_range` |
| Facing | The root rotation snaps to the velocity direction every tick, with no turn rate | `move_system.rs:844` |
| Separation | A constant force of 1.5 per overlapping pair, then a speed clamp | `separation_deltas` |
| Non-combatants | Workers and Haulers ignore danger and keep working until they die | no flee logic exists |
| Charge | Below 0.3 charge, attack and defense scale down linearly, so a weakened Defender barely hurts and barely resists | `charge_strength_multiplier` |

### Presentation

| Fact | Value |
|---|---|
| Hit visual | A gizmo line from attacker to target lasting **0.06 s** (about 4 frames), 3 px, flat color, no glow |
| Impact mark | A 10-unit perpendicular tick. A **single global pool capped at 24** drops the oldest mark first |
| Pose | The sprite child jabs 12 units toward the target and the target recoils 10 units; both decay linearly over 0.18 s, then snap back to identity |
| Death | A 0.32 s squash-and-fade ghost. Nothing remains afterwards |
| Structure death | A flash plus an expanding ring |
| Colors | Hit lines and death flashes use a hard-coded **blue / red** `faction_color`, but bots use **cyan / pink** (`swarm_presentation_color`) |
| Zoom | **Every combat effect is switched off at zoom ≥ 8** (`combat_zoom_cutoff`) |
| Render smoothing | **None.** The simulation runs at 60 Hz and drives the transforms the renderer draws, with no interpolation |
| Unit state | Defenders show **no health and no charge indicator** |
| Audio, particles, bloom, camera shake | None in the codebase |

### What I observed

**Small skirmish (4 v 4), zoom 0.9, twelve captures:**
- About half the captures show no line at all. When lines do appear, they are thin hairlines that vanish by the next capture.
- Units stand about 100 units apart, idle, and face arbitrary directions. The jab turns the sprite toward the target for 0.18 s, then it snaps back to the movement facing.
- It reads as "bots loitering near each other", not as a fight.

**AI Battle first contact:**
- Two player Defenders stood inside 96 units of an enemy and did not fire, because they held no response.
- One pink Defender sat among eight cyan Defenders and only one of them engaged it.
- The death flash is red, while the enemy swarm is pink.

**Mass battle (300 v 300):**

| Time | Alive (player, opponent) | Active hit lines | Impact marks |
|---|---|---|---|
| Contact, about 2 s | 304, 304 | 11 | 11 |
| 3 s | 302, 301 | 29 | 24 (cap) |
| 10 s | 236, 264 | 23 | 24 (cap) |
| 25 s | 102, 116 | 11 | 11 |

- **Thin static front.** At contact only the touching rows fight. The other roughly 90% stand behind the front, out of range and motionless (frames f120, f180).
- **Mosh pit.** Greedy one-to-one pairing sends late responders to deep, unclaimed targets inside the enemy blob. By 10 s the two armies have interpenetrated into one mixed cloud with no front, so you can't tell who is winning (frames f600, f1200, f1500).
- **Strobing.** Consecutive captures show completely different sets of lines. The battle twinkles rather than burns.
- **Impact budget exhausted.** The 24-mark global pool was pinned at its cap for most of the fight, so most hits produce no impact mark, and an off-screen fight would steal marks from an on-screen one.
- **Invisible from the strategic zoom.** At zoom 10 (f182) the battle is two clouds of 2 px dots with no effects, because effects are culled at zoom ≥ 8. At zoom 6 (f181) the 3 px screen-space lines merge into a red smear along the contact seam.
- **Overlap.** Units at the seam pile on top of each other (f121–f126).
- **Asymmetry worth checking.** The player lost 202 units and the opponent 188 from geometrically equivalent (translated, not mirrored) blobs. That may be geometry, or an ordering bias in pairing or separation. The next balance pass should check it.

## Root causes, ranked by impact on feel

1. **Engagement rules produce the wrong battle shape.** The one-to-one response pairing, "only responders fire", and "stop at range and freeze" together give a thin static seam that later dissolves into a mixed mosh pit. No amount of visual effects fixes this.
2. **No render interpolation and no turn rate.** Motion and rotation step at 60 Hz with snaps, which feels choppy on 120–165 Hz displays and whenever frames drop.
3. **Hit visuals are instantaneous and tiny.** A 0.06 s hairline every 0.5 s per attacker strobes at small scale and turns to noise at large scale.
4. **No zoom-appropriate representation.** There is one representation that is switched off at zoom 8, plus a global effect budget that isn't based on what's on screen.
5. **Low per-hit significance.** It takes 20 hits to kill, with no visible health, so individual hits don't matter and kills arrive unannounced.
6. **Missing juice.** There are no particles, glow, aftermath, sound or camera feedback.

## Proposals

Proposals are grouped by layer. Each says whether it changes **simulation** (deterministic, gameplay, may need an ADR) or only **presentation** (render-only, under ADR 0014's rule that presentation never feeds back into gameplay).

### 1. Fluidity foundation (presentation; do this first)

**1.1 Fixed-step render interpolation.**
- Record each nanobot's previous and current simulation position and rotation in `FixedPostUpdate`.
- In `Update`, set the presentation child's local offset to `lerp(prev, curr, overstep_fraction) − curr`, rotated into root space. Use `Time<Fixed>::overstep_fraction()`.
- ADR 0014 already moved every sprite to a presentation child, so this fits without touching simulation transforms.
- Apply the same to structures' hit flash anchors, death-ghost spawn points and the camera follow.
- *Acceptance:* at 144 Hz, consecutive frames never show a bot moving 0 px and then 2× px. Measure it in an offscreen fixture by rendering at a fixed delta that isn't a multiple of 60 Hz.

**1.2 Turn rate and persistent aim.**
- Presentation facing slerps toward the desired heading at a capped angular speed, for example 720°/s for Defenders and 540°/s for others.
- An engaged Defender faces its current target rather than its velocity, and keeps that aim for about 0.6 s after its last hit. It then turns smoothly back to its movement heading.
- The aim target can come from the latest `ResolvedCombatHit`, or from `DefenderResponse.target` if it's exposed read-only.
- This replaces the jab's rotate-then-snap-back twitch.

**1.3 Eased transients.**
- Replace linear `1 − progress` decays with ease-out curves: cubic for the jab, back-out for the recoil, so they read as a punch and a stagger.
- Never snap a transform to identity mid-motion. Blend the last pose out over 2–3 frames when a new pose interrupts it.

**1.4 Presentation hot-path cost.** Fix these before adding more effects, so effects never cost frame rate.
- `visual_for` scans every `NanobotVisual` for each hit, which is O(hits × bots). Resolve the child through `Children` on the root, or keep a root-to-visual map.
- Move hit lines, impact marks and particles from gizmos to **one instanced mesh per effect type**, with a GPU-side lifetime fade, so 5,000+ simultaneous effects cost one draw call each.
- *Acceptance:* the `swarm_acceptance` benchmark gains a combat case (1,000 v 1,000 engaged). Its p95 frame time must stay under the 16.7 ms budget with every effect enabled.

### 2. Engagement behavior (simulation; changes ADR 0015) **[ADR]**

The aim is battles with a readable shape: armies close, form a front, the front bends, the losing side breaks, and survivors fall back.

**2.1 Every armed Defender fights what is in range.**
- A Defender with any hostile inside `attack_range` fires at it whether or not it holds a response. This is "self-defense fire".
- Responses keep their role of *moving* Defenders toward Threats.
- This removes "standing next to an enemy, not shooting", which I observed in AI Battle.

**2.2 Split strategic response from local engagement.**
- **Strategic response** (existing allocation): decides which Defenders head toward which battle region. It should target a **battle cluster**, meaning a spatial grouping of Threats, not a single entity. It can also assign more responders than the cluster has Threats, up to its strength. Today the one-to-one rule leaves surplus Defenders idle while a fight is lost.
- **Local engagement** (new, per tick, bounded spatial query): once a Defender is within about 2× `attack_range` of hostiles, it picks the nearest reachable hostile on **its side of the front** and engages.
- Responders should never path through the enemy formation to reach a deep target. That deep-target routing is exactly what creates the mosh pit.

**2.3 Engagement spacing instead of freezing.**
- An engaged Defender holds a band around its target, for example 70–96 units, instead of a stop radius.
- Within that band it **strafes**: it moves tangentially with a deterministic per-entity phase, derived from a hash of the entity bits as `coincident_pair_direction` already does, and reverses every 1–2 s.
- The result is a living, shimmering front line instead of a parked one. The behavior stays deterministic, and speed stays within `bot_speed`.

**2.4 Fill the line.**
- Defenders behind the front with no hostile in range advance until they are about 1.5 body diameters behind the nearest friendly engaged unit. They don't stop at the rear.
- Back ranks then visibly push forward as the front thins. That gives the battle a surge, and damage output scales with army size the way a player expects. Today, damage output is limited to the width of the front.

**2.5 Break and fall back.**
- A Defender whose charge drops below the weakened threshold (0.3) disengages: it backs off while still facing the enemy, and then rotates to a Charger as ADR 0015 already allows.
- A cohort that has lost more than about 60% of its local strength inside an engagement region within a short window retreats toward its nearest friendly Swarm Tile. This makes collapses and routs visible events rather than a slow grind. It's a gameplay change, so it needs a design decision.

**2.6 Non-combatants react.**
- Workers and Haulers within about 200 units of a hostile Defender pause their task and flee toward the nearest friendly Defender or Production Facility, then resume.
- Scattering civilians are the single biggest "this place is under attack" cue in classic RTS games, and they also add gameplay counterplay.

**2.7 Desynchronized cadence.**
- Seed each Defender's first cooldown with a deterministic per-entity offset in `[0, attack_interval)`. This stops spawn batches or recharge returns from firing volleys in lockstep.
- Optionally add ±10% per-shot cadence jitter from a deterministic hash. Synchronized hits read as strobing.

**2.8 Separation that doesn't jitter.**
- Make the separation force scale with overlap depth instead of a constant 1.5 per pair, and smooth it across ticks.
- At the front, also let engaged units yield sideways along the front tangent rather than straight back. This removes the pile-ups at the seam (f121–f126).

### 3. Hit, damage and kill presentation (presentation)

**3.1 Replace the hairline with a bolt.**
- Each resolved hit spawns an energy bolt: a short bright head with a fading tail, about 30–40 units long.
- The bolt travels from attacker to target in about 60–90 ms at a constant speed, so a 96-unit shot takes about 70 ms.
- Additive blending in the swarm's own `swarm_presentation_color`. Drop the red/blue `faction_color` so the hit color matches the unit color.
- ADR 0014 says the pulse "is not a projectile". The bolt stays presentation-only: damage has already resolved, and only the visual travels. The ADR wording should be refined, not reversed. **[ADR wording]**
- Small budget-limited spawns go with it:
  - a **muzzle flash** (one bright sprite, about 50 ms) at the attacker;
  - an **impact spark** of 4–8 instanced particles in the attacker's color, spraying along the reflected direction;
  - a target **hit flash and recoil** delayed until the bolt arrives.

**3.2 Synchronize death with the killing bolt.**
- When the killing hit's bolt arrives, play the death:
  - a bright pop (white core, then the swarm color);
  - 6–12 fragment particles with drag and spin;
  - a brief expanding shock ring;
  - a **scorch decal** that stays 20–40 s and fades.
- The gameplay entity is already gone; ADR 0014 explicitly allows death presentation to outlive it.

**3.3 Make health and charge visible without clutter.**
- **Health:** a thin arc around the bot's base that appears only after the bot takes damage, fades about 2 s after the last hit, and is colored green → amber → red.
- **Charge:** the existing white core dims as charge drops. Below the weakened threshold it flickers slowly, so a weakened Defender is recognizable at a glance.

**3.4 Glow.**
- Enable HDR and bloom on the main 2D camera and author combat effects with emissive intensity above 1.0. Units, zones and UI stay at ≤ 1.0 so only energy glows.
- Make bloom and particle intensity a presentation tuning setting, with a "reduced effects" preset.

**3.5 Structure damage.**
- While a structure is below 50% health, emit sparks and smoke wisps from it.
- On destruction, play a larger multi-stage pop with debris and leave a lasting rubble decal.
- This keeps the ring.

### 4. Large-scale and far-zoom representation (presentation)

**4.1 Zoom tiers instead of one cutoff.**

| Tier | Zoom | Representation |
|---|---|---|
| Close | < 2.5 | Everything in section 3 |
| Mid | 2.5–8 | Bolts become short streak sprites with no tail. Impact sparks become one flash sprite. Death pop and decal stay. Particle counts halve |
| Far | ≥ 8 | Individual effects off, replaced by the **battle heat layer** (4.2) and **casualty flashes** (4.3) |

At every tier, line widths and particle sizes are authored in world units with a screen-space minimum and maximum, so effects don't swell into smears as you zoom out (see f181).

**4.2 Battle heat layer.**
- Accumulate resolved hits per coarse bucket (128–256 world units) into a small texture.
- Store intensity and swarm mix in each texel. Decay it with a half-life of about 0.7 s.
- Render it as a soft additive glow, tinted by the mix of the two swarm colors and shimmering slightly.
- From far away you then see a glowing front line that thickens, moves and fades. This is the main way a huge battle stays legible strategically, and it is cheap: one texture update and one quad.

**4.3 Casualty flashes.** At the far tier, every death is a single bright dot in the victim's swarm color that pops and fades over about 0.4 s. You can watch an army melt.

**4.4 Battle marker in the tactical overlay.**
- At zoom ≥ 16 (`tactical_far_merge_zoom`), add a pulsing crossed-swords pictogram where heat exceeds a threshold.
- Its size reflects intensity and its ring shows the local casualty ratio, so the player can see at a glance who is winning each fight.
- Clicking it, or pressing a key, jumps the camera there. This also gives a natural place for later minimap pings.

**4.5 Screen-relative effect budget.**
- Replace the global cap of 24 impact marks with a budget per frame for effects **on screen**: 300 bolts, 600 particles and 40 death pops at the close tier.
- Prioritize, in order: deaths, effects near the screen center, hits against the player swarm, then the rest.
- Off-screen hits only feed the heat layer.

### 5. Juice and feedback (presentation)

**5.1 Camera shake.** Only for structure destruction on screen and for a big death burst (more than about 10 deaths in one heat bucket within 0.3 s). Keep it small (≤ 4 px), quick (≤ 150 ms), and toggleable in the tuning panel.

**5.2 Audio.** Bevy's default features include audio, and the game has none. Layer it by intensity rather than playing per-hit sounds, which would become noise:
- an ambient **battle bed** whose volume and filter follow on-screen heat;
- sparse **kill pops** for deaths near the camera, limited to about 8 per second with pitch variation;
- **structure destruction booms**;
- an **under-attack sting** that is rate-limited and tied to the existing HUD warning.

**5.3 Aftermath.** Scorch and rubble decals (3.2, 3.5) stay for 20–40 s and are merged per bucket to cap their count. After a battle, the ground shows where it happened and how big it was. That gives the map history at almost no cost.

**5.4 Small skirmish punctuation.** When the last hostile in an engagement region dies, play a brief soft shockwave ring in the winner's color and a tiny UI tick in the HUD, such as "Skirmish won (−3 / +1)". It marks the end of the event.

### 6. Pacing and tuning (simulation)

These are numbers to try in an isolated tuning file before committing to them. They aim for fewer, more meaningful hits.

| Knob | Now | Proposed | Why |
|---|---|---|---|
| `attack_interval_ticks` | 30 (0.5 s) | 40 (0.67 s) ± jitter | Each bolt is readable, and there's less overlap noise |
| Damage against a full-charge Defender | 5 | 12–15 (raise `base_attack`, or reduce how much defense halves it) | 7–9 hits to kill instead of 20 |
| Equal duel | about 10 s | about 5 s | A small fight resolves while the player is still watching |
| `attack_range` | 96 | 110–130 | With strafing (2.3), adds a clear gap between the lines so bolts are visible |
| 300 v 300 | about 40+ s grind | about 20–30 s with a visible surge and collapse | Lethality scales with the engaged count once the line fills (2.4) |
| Weakened scaling | linear to 0 below 0.3 | floor at 40% attack | Weakened units still matter; the retreat in 2.5 is a choice, not a death sentence |

Recheck Charge economics (drain, pulse) after changing cadence. ADR 0015 says combat cadence and damage are unchanged, so a tuning pass needs an amendment or a separate balance decision.

## Suggested order

1. **Foundation:** 1.1 interpolation, 1.2 turn rate and aim, 1.3 easing, 1.4 hot-path fixes. Pure presentation, low risk, and the largest fluidity gain.
2. **Engagement shape:** 2.1 self-defense fire, 2.7 cadence offsets, 2.8 separation, then 2.2 and 2.4 (cluster response and line fill). Then 2.3 strafing. Update ADR 0015.
3. **Hit and kill visuals:** 3.1, 3.2, 3.3, then 3.4 bloom, behind the screen budget (4.5).
4. **Scale:** 4.1 tiers, 4.2 heat layer, 4.3 casualty flashes, 4.4 battle marker.
5. **Pacing pass:** section 6, tuned against fixtures at both scales.
6. **Juice:** 5.1–5.4, plus 2.5 retreat and 2.6 civilian flee as their own gameplay decisions.

## Verification to add

- **Promote the mass-battle fixture** into `screenshots/`, with fixed seeds. It should cover 4 v 4, 40 v 40 and 300 v 300 at zoom 1, 3, 6 and 12. It should assert:
  - every capture taken after first contact (hits are flowing) shows at least one visible bolt or heat pixel;
  - no Defender within range of a hostile goes more than one attack interval without firing (2.1);
  - the front stays coherent: the count of units behind enemy lines stays below a small fraction of each army until one side drops below 30%.
- **Motion smoothness check:** render at a 144 Hz manual delta, and assert per-frame displacement variance and rotation step size for moving bots.
- **Combat benchmark case:** 1,000 v 1,000 engaged with all effects on, with a p95 frame-time budget.
- **Symmetry test:** two mirror-image armies must finish with equal survivors, to rule out the ordering bias suspected above.

## Decisions

The user decided these on 2026-11-21:

1. **Engagement rules: approved.** Any Defender fires at a hostile in range whether or not it holds a response (2.1). Strategic responses target battle clusters and may send more than one Defender per Threat (2.2). Both supersede the matching parts of ADR 0015.
2. **Retreat and flee: approved, automatic.** Weakened Defenders and broken cohorts fall back on their own (2.5), and Workers and Haulers flee from hostile Defenders (2.6). The user answered "yes" without choosing between automatic and player-controlled, so this plan assumes automatic, as proposed. A player-controlled override would be separate later work.
3. **Pacing: approved.** The target is an equal duel of about 5 s, with the section 6 table as the starting point.
4. **Glow and audio: in scope.** HDR bloom (3.4) and layered audio (5.2) are part of this work, with the "reduced effects" preset.
