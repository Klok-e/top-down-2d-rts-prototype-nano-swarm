use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

use bevy::{
    ecs::message::MessageCursor,
    post_process::bloom::Bloom,
    prelude::*,
    render::{
        render_resource::PollType,
        renderer::{RenderAdapterInfo, RenderDevice},
    },
    time::TimeUpdateStrategy,
};
use top_down_2d_rts_prototype_nano_swarm::{
    GAMEPLAY_SPRITE_Z,
    fly_camera::CameraZoom2d,
    nanobot::{
        ActiveCombatBolts, ActiveCombatEffects, ActiveSkirmishRings, Cargo, CombatHeat,
        CombatParticleKind, Health, Nanobot, NanobotSprites, NanobotType, NanobotVisual,
        OpponentSwarm, PlannedStructure, ResolvedCombatFact, StrategicController, Structure,
        SwarmId, SwarmMember,
    },
    resources::ResourceKind,
    terrain::RockFormation,
    tuning::Tuning,
    ui::match_banner::MatchBannerRoot,
};

#[path = "../tests/common/combat_scenes.rs"]
mod scenes;

use crate::harness::{TestContext, TestFlow};
use scenes::{Scene, Variant, spawn_scene};

const TIMING_ZOOM: u32 = 3;
const TIMING_START: u64 = 100;
const TIMING_END: u64 = 590;
const BOLT_COUNT_TICKS: u64 = 60;
const CAPTURE_WAIT_TICKS: u64 = 4;
const MOTION_FRAME_DELTA: Duration = Duration::from_nanos(6_944_444);
const BATTLE_CAPTURES: [(u64, u32); 9] = [
    (90, 1),
    (90, 3),
    (90, 6),
    (90, 12),
    (600, 1),
    (600, 3),
    (600, 6),
    (600, 12),
    (600, 20),
];

#[derive(Resource, Default)]
struct SceneCapture {
    hits: MessageCursor<ResolvedCombatFact>,
    contact: Option<u64>,
    next_capture: usize,
    capture_prepared: bool,
    captured_casualties: bool,
    captured_skirmish_end: bool,
    timing: Option<(u32, Instant)>,
    bolt_spawns: VecDeque<(u64, usize)>,
    peaks: SpawnPeaks,
}

#[derive(Default)]
struct SpawnPeaks {
    bolts: usize,
    particles: usize,
    death_pops: usize,
    decals: usize,
}

impl SceneCapture {
    fn record_spawns(&mut self, world: &World, tick: u64) {
        let bolts = world.resource::<ActiveCombatBolts>();
        let effects = world.resource::<ActiveCombatEffects>();
        let bolt_spawns = bolts.spawned_this_frame();
        let spawns = effects.frame_spawns();
        let decal_count = effects.decal_count();
        assert!(
            bolt_spawns <= 300,
            "{bolt_spawns} bolts spawned in one frame"
        );
        assert!(
            spawns.particles <= 600,
            "{} particles spawned in one frame",
            spawns.particles
        );
        assert!(
            spawns.death_pops <= 40,
            "{} death pops spawned in one frame",
            spawns.death_pops
        );
        assert!(decal_count <= 2_000, "{decal_count} active decals");
        self.peaks.bolts = self.peaks.bolts.max(bolt_spawns);
        self.peaks.particles = self.peaks.particles.max(spawns.particles);
        self.peaks.death_pops = self.peaks.death_pops.max(spawns.death_pops);
        self.peaks.decals = self.peaks.decals.max(decal_count);
        if bolt_spawns > 0 {
            self.bolt_spawns.push_back((tick, bolt_spawns));
        }
        while self
            .bolt_spawns
            .front()
            .is_some_and(|&(spawn_tick, _)| tick - spawn_tick >= BOLT_COUNT_TICKS)
        {
            self.bolt_spawns.pop_front();
        }
    }
}

pub fn battle(ctx: &mut TestContext) -> TestFlow {
    capture_scene(ctx, Scene::Battle, &BATTLE_CAPTURES, false)
}

pub fn battle_reduced(ctx: &mut TestContext) -> TestFlow {
    capture_scene(ctx, Scene::Battle, &[(90, 1)], true)
}

pub fn skirmish(ctx: &mut TestContext) -> TestFlow {
    if ctx.frame == 0 {
        ctx.world
            .insert_resource(TimeUpdateStrategy::ManualDuration(MOTION_FRAME_DELTA));
        ctx.world.resource_mut::<Tuning>().combat.rout_ratio = 0.0;
    }
    capture_scene(ctx, Scene::Skirmish, &[(30, 1)], false)
}

#[derive(Component)]
struct ScriptedWorker;

#[derive(Resource, Default)]
struct MotionCapture {
    last_x: Option<f32>,
    steps: Vec<f32>,
}

pub fn rendered_motion(ctx: &mut TestContext) -> TestFlow {
    let world = &mut *ctx.world;
    let center = Scene::Skirmish.center();
    match ctx.frame {
        0 => {
            setup(world, Scene::Skirmish);
            world.insert_resource(TimeUpdateStrategy::ManualDuration(MOTION_FRAME_DELTA));
            world.init_resource::<MotionCapture>();
            world.schedule_scope(FixedUpdate, |_, schedule| {
                schedule.add_systems(|mut worker: Single<&mut Transform, With<ScriptedWorker>>| {
                    worker.translation.x += 3.0;
                });
            });
            world.spawn((
                ScriptedWorker,
                Nanobot {},
                NanobotType::Worker,
                SwarmMember::new(SwarmId::PLAYER),
                Cargo {
                    kind: ResourceKind::Minerals,
                    amount: 2,
                },
                Transform::from_translation(
                    (center + Vec2::new(-100.0, -115.0)).extend(GAMEPLAY_SPRITE_Z),
                ),
            ));
            set_camera(world, center, 1);
        }
        1..=7 => {}
        8..=56 => {
            let worker = world
                .query_filtered::<&Children, With<ScriptedWorker>>()
                .single(world)
                .unwrap();
            let sprite = worker
                .iter()
                .find(|&entity| world.get::<NanobotVisual>(entity).is_some())
                .unwrap();
            let x = world
                .get::<GlobalTransform>(sprite)
                .unwrap()
                .translation()
                .x;
            let mut sample = world.resource_mut::<MotionCapture>();
            if let Some(last_x) = sample.last_x.replace(x) {
                sample.steps.push(x - last_x);
            }
        }
        57 => {
            let steps = &world.resource::<MotionCapture>().steps;
            let mean = steps.iter().sum::<f32>() / steps.len() as f32;
            assert!((mean - 1.25).abs() < 0.01);
            assert!(
                steps
                    .iter()
                    .all(|&step| step > 0.0 && (step - mean).abs() <= 0.1 * mean),
                "the rendered worker stalled or jumped between 144 Hz frames: {steps:?}",
            );
            let max_deviation = steps
                .iter()
                .map(|step| (step - mean).abs() / mean)
                .fold(0.0, f32::max);
            println!(
                "144 Hz offscreen visual translation: mean {mean:.4} units, max deviation {:.4}%",
                100.0 * max_deviation
            );
            return TestFlow::Screenshot("combat_motion_144_hz".into());
        }
        _ => return TestFlow::Exit,
    }
    TestFlow::Continue
}

fn capture_scene(
    ctx: &mut TestContext,
    scene: Scene,
    captures: &[(u64, u32)],
    reduced: bool,
) -> TestFlow {
    let world = &mut *ctx.world;
    if ctx.frame == 0 {
        if reduced {
            let mut tuning = world.resource_mut::<Tuning>();
            tuning.presentation.effects_intensity = 0.3;
            tuning.presentation.bloom_intensity = 0.0;
        }
        setup(world, scene);
        return TestFlow::Continue;
    }

    let tick = fixed_tick(world);
    world.resource_scope(|world, mut state: Mut<SceneCapture>| {
        state.record_spawns(world, tick);
        if state.contact.is_none() {
            if state.hits.read(world.resource::<Messages<ResolvedCombatFact>>())
                .any(|fact| matches!(fact, ResolvedCombatFact::Hit(_)))
            {
                state.contact = Some(tick);
            } else {
                return TestFlow::Continue;
            }
        }
        let elapsed_ticks = tick - state.contact.unwrap();
        let Some(&(capture_tick, zoom)) = captures.get(state.next_capture) else {
            match scene {
                Scene::Battle if !reduced && !state.captured_casualties => {
                    return capture_casualties(world, &mut state, elapsed_ticks);
                }
                Scene::Skirmish if !state.captured_skirmish_end => {
                    return capture_skirmish_end(world, &mut state, elapsed_ticks);
                }
                _ => {}
            }
            let SpawnPeaks { bolts, particles, death_pops, decals } = state.peaks;
            println!(
                "{}{} peaks: bolts/frame={bolts} particles/frame={particles} death_pops/frame={death_pops} decals={decals}",
                scene.name(),
                if reduced { " reduced" } else { "" },
            );
            return TestFlow::Exit;
        };
        if elapsed_ticks < capture_tick {
            if world.resource::<Time<Virtual>>().is_paused() {
                world.resource_mut::<Time<Virtual>>().unpause();
                set_camera(world, scene.center(), TIMING_ZOOM);
            }
            if matches!(scene, Scene::Battle) && !reduced {
                measure_updates(world, &mut state, ctx.frame, elapsed_ticks);
            }
            return TestFlow::Continue;
        }

        // Readback pumps full app updates. Freeze both simulation and animation
        // so every zoom sees the same battle state and the effects counted here.
        if !world.resource::<Time<Virtual>>().is_paused() {
            if elapsed_ticks < capture_tick + CAPTURE_WAIT_TICKS
                && !has_visible_combat(world, scene)
            {
                return TestFlow::Continue;
            }
            world.resource_mut::<Time<Virtual>>().pause();
            return TestFlow::Continue;
        }

        let suffix = if reduced { "_reduced" } else { "" };
        let name = format!("combat_{}_{capture_tick}_z{zoom}{suffix}", scene.name());
        assert!(
            elapsed_ticks <= capture_tick + CAPTURE_WAIT_TICKS + 1,
            "{name} was requested {elapsed_ticks} ticks after contact"
        );
        if !state.capture_prepared {
            set_camera(world, scene.center(), zoom);
            state.capture_prepared = true;
            return TestFlow::Continue;
        }
        let half_view = assert_both_sides_in_view(world, scene.center(), zoom, &name);
        assert_capture_setup(world, reduced);
        let bolts = world.resource::<ActiveCombatBolts>();
        let effects = world.resource::<ActiveCombatEffects>();
        let in_view = |position: Vec2| (position - scene.center()).abs().cmplt(half_view).all();
        let heat = world.resource::<CombatHeat>();
        let visible_markers = heat.markers().iter().filter(|marker| in_view(marker.position)).count();
        let visible_casualties = effects.casualty_flashes().iter().filter(|flash| in_view(flash.position)).count();
        let visible_bolts = bolts
            .iter()
            .filter(|bolt| in_view(bolt.position))
            .count();
        if matches!(scene, Scene::Battle) && !reduced {
            match zoom {
                6 => {
                    assert!(visible_bolts > 0, "{name} must capture bolts");
                    println!(
                        "{name} bolt positions: {:?}",
                        bolts.iter().filter_map(|bolt| in_view(bolt.position).then_some(bolt.position)).collect::<Vec<_>>()
                    );
                }
                12 => {
                    assert!(heat.total() > 0.0, "{name} must capture battle heat");
                    state.captured_casualties |= effects.casualty_flashes().iter().any(|flash| {
                        in_view(flash.position) && flash.opacity() > 0.6
                    });
                }
                20 => assert!(visible_markers > 0, "{name} must capture a battle marker"),
                _ => {}
            }
        }
        let visible_sparks = effects
            .particles()
            .filter(|particle| {
                particle.kind == CombatParticleKind::ImpactSpark && in_view(particle.position)
            })
            .count();
        let recent_bolts: usize = state.bolt_spawns.iter().map(|&(_, count)| count).sum();
        println!(
            "{name}.png at contact +{elapsed_ticks}: bolts={} bolts_in_view={visible_bolts} bolts_last_60_ticks={recent_bolts} particles={} impact_sparks_in_view={visible_sparks} death_pops={} decals={} casualty_flashes_in_view={visible_casualties} heat={:.1} markers_in_view={visible_markers}",
            bolts.len(),
            effects.particles().len(),
            effects.death_pops().len(),
            effects.decal_count(),
            heat.total(),
        );
        state.next_capture += 1;
        state.capture_prepared = false;
        TestFlow::Screenshot(name)
    })
}

fn has_visible_combat(world: &mut World, scene: Scene) -> bool {
    let half_view = viewport_half_size(world, 1);
    let in_view = |position: Vec2| (position - scene.center()).abs().cmplt(half_view).all();
    world
        .resource::<ActiveCombatBolts>()
        .iter()
        .any(|bolt| in_view(bolt.position))
        || (matches!(scene, Scene::Skirmish)
            && world
                .resource::<ActiveCombatEffects>()
                .particles()
                .any(|particle| in_view(particle.position)))
}

fn capture_casualties(world: &mut World, state: &mut SceneCapture, elapsed_ticks: u64) -> TestFlow {
    let center = Scene::Battle.center();
    set_camera(world, center, 12);
    world.resource_mut::<Time<Virtual>>().unpause();
    assert!(
        elapsed_ticks < 780,
        "the battle did not show a casualty flash"
    );
    let half_view = assert_both_sides_in_view(world, center, 12, "battle casualties");
    let casualties = world.resource::<ActiveCombatEffects>().casualty_flashes();
    let visible_flash = casualties.iter().find(|flash| {
        (flash.position - center).abs().cmplt(half_view).all() && flash.opacity() > 0.6
    });
    let Some(flash) = visible_flash else {
        return TestFlow::Continue;
    };
    println!(
        "battle casualty flash at contact +{elapsed_ticks} on zoom 12: center={} diameter={:.2}px",
        flash.position,
        flash.diameter_px(),
    );
    state.captured_casualties = true;
    TestFlow::Screenshot("combat_battle_casualty_z12".into())
}

fn capture_skirmish_end(
    world: &mut World,
    state: &mut SceneCapture,
    elapsed_ticks: u64,
) -> TestFlow {
    world.resource_mut::<Time<Virtual>>().unpause();
    assert!(
        elapsed_ticks < 1_200,
        "the skirmish did not show an end ring"
    );
    let rings = world.resource::<ActiveSkirmishRings>();
    let Some(ring) = rings
        .iter()
        .find(|ring| (80.0..120.0).contains(&ring.radius()))
    else {
        return TestFlow::Continue;
    };
    println!(
        "skirmish end ring at contact +{elapsed_ticks}: center={} radius={:.1}, color={:?}",
        ring.center,
        ring.radius(),
        ring.color,
    );
    let center = ring.center;
    set_camera(world, center, 1);
    state.captured_skirmish_end = true;
    TestFlow::Screenshot("combat_skirmish_end_z1".into())
}

fn setup(world: &mut World, scene: Scene) {
    let removed = world
        .query_filtered::<Entity, Or<(
            With<RockFormation>,
            With<Nanobot>,
            With<Structure>,
            With<PlannedStructure>,
            With<MatchBannerRoot>,
        )>>()
        .iter(world)
        .collect::<Vec<_>>();
    for entity in removed {
        world.despawn(entity);
    }
    let opponent = world
        .query_filtered::<Entity, With<OpponentSwarm>>()
        .single(world)
        .expect("full app should have one opponent swarm");
    world.entity_mut(opponent).remove::<StrategicController>();

    spawn_scene(world, scene, Variant::Standard);
    let hits = world
        .resource::<Messages<ResolvedCombatFact>>()
        .get_cursor_current();
    world.insert_resource(SceneCapture { hits, ..default() });
    set_camera(world, scene.center(), TIMING_ZOOM);
}

fn fixed_tick(world: &World) -> u64 {
    let time = world.resource::<Time<Fixed>>();
    (time.elapsed().as_nanos() / time.timestep().as_nanos()) as u64
}

fn set_camera(world: &mut World, center: Vec2, zoom: u32) {
    let (mut transform, mut projection, mut camera_zoom) = world
        .query::<(&mut Transform, &mut Projection, &mut CameraZoom2d)>()
        .single_mut(world)
        .expect("full app should have one fly camera");
    let Projection::Orthographic(orthographic) = &mut *projection else {
        panic!("combat scene expects an orthographic camera");
    };
    transform.translation = center.extend(transform.translation.z);
    orthographic.scale = zoom as f32;
    camera_zoom.zoom = zoom as f32;
}

fn viewport_half_size(world: &mut World, zoom: u32) -> Vec2 {
    let size = world
        .query_filtered::<&Camera, With<CameraZoom2d>>()
        .single(world)
        .expect("full app should have one fly camera")
        .logical_viewport_size()
        .expect("camera viewport should be available after startup");
    size * (zoom as f32 / 2.0)
}

fn assert_both_sides_in_view(world: &mut World, center: Vec2, zoom: u32, name: &str) -> Vec2 {
    let half_view = viewport_half_size(world, zoom);
    let mut visible = [0; 2];
    for (transform, member, health, kind) in world
        .query_filtered::<(&Transform, &SwarmMember, &Health, &NanobotType), With<Nanobot>>()
        .iter(world)
    {
        let offset = (transform.translation.truncate() - center).abs();
        if *kind == NanobotType::Defender && health.current > 0 && offset.cmplt(half_view).all() {
            visible[usize::from(member.0 != SwarmId::PLAYER)] += 1;
        }
    }
    assert!(
        visible.iter().all(|&count| count > 0),
        "{name} must show both armies, visible Defenders: {visible:?}"
    );
    half_view
}

fn assert_capture_setup(world: &mut World, reduced: bool) {
    let sprites = world.resource::<NanobotSprites>();
    let images = world.resource::<Assets<Image>>();
    for handle in [&sprites.player_defender, &sprites.opponent_defender] {
        assert!(
            images.get(handle).is_some(),
            "Defender art must load for combat captures"
        );
    }
    let bloom = world
        .query_filtered::<Option<&Bloom>, With<CameraZoom2d>>()
        .single(world)
        .expect("full app should have one fly camera");
    if reduced {
        assert!(bloom.is_none_or(|bloom| bloom.intensity == 0.0));
    } else {
        let bloom = bloom.expect("the standard battle camera should have Bloom");
        assert_eq!(bloom.prefilter.threshold, 1.0);
        assert_eq!(bloom.intensity, 0.3);
    }
}

fn measure_updates(world: &World, state: &mut SceneCapture, frame: u32, tick: u64) {
    match tick {
        TIMING_START => {
            finish_rendering(world);
            state.timing = Some((frame, Instant::now()));
        }
        TIMING_END => {
            let (start_frame, start) = state.timing.expect("battle timing start was skipped");
            finish_rendering(world);
            let cost = start.elapsed() / (frame - start_frame);
            let adapter = world.resource::<RenderAdapterInfo>();
            println!(
                "combat_battle mean app update: {:.3} ms ({} updates between contact +{TIMING_START} and +{TIMING_END}, zoom {TIMING_ZOOM}, GPU {}, {:?})",
                cost.as_secs_f64() * 1000.0,
                frame - start_frame,
                adapter.name,
                adapter.backend,
            );
        }
        _ => {}
    }
}

fn finish_rendering(world: &World) {
    world
        .resource::<RenderDevice>()
        .poll(PollType::Wait {
            submission_index: None,
            timeout: Some(Duration::from_secs(30)),
        })
        .expect("battle timing GPU queue should complete");
}
