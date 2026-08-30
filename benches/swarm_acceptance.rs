use std::time::{Duration, Instant};

use bevy::{prelude::*, time::TimeUpdateStrategy};
use criterion::{Criterion, Throughput};
use top_down_2d_rts_prototype_nano_swarm::{
    game_settings::GameSettings,
    intent::{IntentGrid, IntentKind},
    nanobot::{
        CombatPlugin, Commitment, DefenderResponse, Health, Nanobot, NanobotBundle, NanobotPlugin,
        NanobotSimulationSet, NanobotType, RegionalAllocationPlugin, RegionalAllocationSet,
        SwarmId, SwarmMember, idle_spread_system, move_velocity_system, separation_system,
    },
    resources::ResourceLedger,
};

const BOT_COUNT: usize = 5_000;
const WARMUP_FRAMES: usize = 60;
const ACCEPTANCE_SAMPLE_FRAMES: usize = 600;
const FRAME_P95_BUDGET: Duration = Duration::from_micros(16_700);
const ALLOCATION_P95_BUDGET: Duration = Duration::from_millis(2);
const SEPARATION_P95_BUDGET: Duration = Duration::from_millis(3);
const PROOF_ONLY_ENV: &str = "NANO_SWARM_ACCEPTANCE_PROOF_ONLY";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SystemSet)]
enum AcceptanceTimingSet {
    AllocationStart,
    ProjectEnd,
    InvalidateEnd,
    AllocationEnd,
}

#[derive(Debug, Default, Resource)]
struct AcceptanceTimings {
    allocation_started: Option<Instant>,
    allocation_stage_started: Option<Instant>,
    separation_started: Option<Instant>,
    allocation: Vec<Duration>,
    project: Vec<Duration>,
    invalidate: Vec<Duration>,
    acquire: Vec<Duration>,
    separation: Vec<Duration>,
}

fn start_allocation_timing(mut timings: ResMut<AcceptanceTimings>) {
    let started = Instant::now();
    timings.allocation_started = Some(started);
    timings.allocation_stage_started = Some(started);
}

fn finish_project_timing(mut timings: ResMut<AcceptanceTimings>) {
    let finished = Instant::now();
    let started = timings
        .allocation_stage_started
        .replace(finished)
        .expect("Project timing must start before it finishes");
    timings.project.push(finished.duration_since(started));
}

fn finish_invalidate_timing(mut timings: ResMut<AcceptanceTimings>) {
    let finished = Instant::now();
    let started = timings
        .allocation_stage_started
        .replace(finished)
        .expect("Invalidate timing must start before it finishes");
    timings.invalidate.push(finished.duration_since(started));
}

fn finish_allocation_timing(mut timings: ResMut<AcceptanceTimings>) {
    let finished = Instant::now();
    let stage_started = timings
        .allocation_stage_started
        .take()
        .expect("Acquire timing must start before it finishes");
    let allocation_started = timings
        .allocation_started
        .take()
        .expect("allocation timing must start before it finishes");
    timings.acquire.push(finished.duration_since(stage_started));
    timings
        .allocation
        .push(finished.duration_since(allocation_started));
}

fn start_separation_timing(mut timings: ResMut<AcceptanceTimings>) {
    timings.separation_started = Some(Instant::now());
}

fn finish_separation_timing(mut timings: ResMut<AcceptanceTimings>) {
    let started = timings
        .separation_started
        .take()
        .expect("separation timing must start before it finishes");
    timings.separation.push(started.elapsed());
}

fn add_acceptance_timing(app: &mut App) {
    app.init_resource::<AcceptanceTimings>()
        .configure_sets(
            FixedUpdate,
            AcceptanceTimingSet::AllocationStart
                .after(NanobotSimulationSet::Threat)
                .before(RegionalAllocationSet::Project),
        )
        .configure_sets(
            FixedUpdate,
            AcceptanceTimingSet::ProjectEnd
                .after(RegionalAllocationSet::Project)
                .before(RegionalAllocationSet::Invalidate),
        )
        .configure_sets(
            FixedUpdate,
            AcceptanceTimingSet::InvalidateEnd
                .after(RegionalAllocationSet::Invalidate)
                .before(RegionalAllocationSet::Acquire),
        )
        .configure_sets(
            FixedUpdate,
            AcceptanceTimingSet::AllocationEnd
                .after(RegionalAllocationSet::Acquire)
                .before(NanobotSimulationSet::Combat),
        )
        .add_systems(
            FixedUpdate,
            start_allocation_timing.in_set(AcceptanceTimingSet::AllocationStart),
        )
        .add_systems(
            FixedUpdate,
            finish_project_timing.in_set(AcceptanceTimingSet::ProjectEnd),
        )
        .add_systems(
            FixedUpdate,
            finish_invalidate_timing.in_set(AcceptanceTimingSet::InvalidateEnd),
        )
        .add_systems(
            FixedUpdate,
            finish_allocation_timing.in_set(AcceptanceTimingSet::AllocationEnd),
        )
        .add_systems(
            FixedUpdate,
            start_separation_timing
                .in_set(NanobotSimulationSet::Movement)
                .after(move_velocity_system)
                .before(separation_system),
        )
        .add_systems(
            FixedUpdate,
            finish_separation_timing
                .in_set(NanobotSimulationSet::Movement)
                .after(separation_system)
                .before(idle_spread_system),
        );
}

fn app_with_bots(defend_work: bool) -> App {
    let mut app = App::new();
    app.add_plugins(bevy::time::TimePlugin)
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_micros(
            16_667,
        )))
        .insert_resource(Time::<Fixed>::from_hz(60.0))
        .insert_resource(IntentGrid::new(1000, 1000))
        .insert_resource(GameSettings {
            width: 512_000.0,
            height: 512_000.0,
            bot_speed: 5.0,
            debug_draw_circles: false,
        })
        .init_resource::<ResourceLedger>()
        .add_plugins(NanobotPlugin::default())
        .add_plugins(CombatPlugin)
        .add_plugins(RegionalAllocationPlugin);

    {
        let mut grid = app.world_mut().resource_mut::<IntentGrid>();
        for y in -8..8 {
            for x in -8..8 {
                if defend_work {
                    grid.add_owned(IVec2::new(x, y), IntentKind::Defend, Some(SwarmId::PLAYER));
                } else {
                    grid.add_owned(IVec2::new(x, y), IntentKind::Gather, Some(SwarmId::PLAYER));
                }
            }
        }
    }

    for i in 0..BOT_COUNT {
        let x = (i % 100) as f32 * 40.0;
        let y = (i / 100) as f32 * 40.0;
        let bundle = NanobotBundle {
            nanobot_type: if defend_work {
                NanobotType::Defender
            } else {
                NanobotType::Worker
            },
            swarm_member: SwarmMember::new(if defend_work && i % 2 == 1 {
                SwarmId(11)
            } else {
                SwarmId::PLAYER
            }),
            health: Health::full(u32::MAX / 2),
            ..Default::default()
        };
        app.world_mut()
            .spawn((bundle, Commitment::Idle, Transform::from_xyz(x, y, 0.0)));
    }
    app
}

fn assert_warmed_load(app: &mut App, defend_work: bool) {
    let population = app
        .world_mut()
        .query_filtered::<Entity, With<Nanobot>>()
        .iter(app.world())
        .count();
    assert_eq!(population, BOT_COUNT, "benchmark warmup must preserve load");
    if defend_work {
        let responses = app
            .world_mut()
            .query::<&DefenderResponse>()
            .iter(app.world())
            .map(|response| response.target)
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            responses.len(),
            BOT_COUNT / 2,
            "benchmark warmup must cover every hostile Defender exactly once",
        );
    }
}

fn warmed_app(defend_work: bool) -> App {
    let mut app = app_with_bots(defend_work);
    for _ in 0..WARMUP_FRAMES {
        app.update();
    }
    assert_warmed_load(&mut app, defend_work);
    app
}

fn warmed_sparse_stranded_app() -> App {
    let mut app = app_with_bots(false);
    let mut grid = IntentGrid::new(1000, 1000);
    grid.add_owned(
        IVec2::new(400, 400),
        IntentKind::Gather,
        Some(SwarmId::PLAYER),
    );
    app.insert_resource(grid);
    for _ in 0..WARMUP_FRAMES {
        app.update();
    }
    app
}

fn p95(samples: &[Duration]) -> Duration {
    assert!(!samples.is_empty(), "p95 requires at least one sample");
    let mut ordered = samples.to_vec();
    ordered.sort_unstable();
    let nearest_rank = (ordered.len() * 95).div_ceil(100);
    ordered[nearest_rank - 1]
}

fn duration_ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000.0
}

fn assert_within_budget(metric: &str, actual: Duration, budget: Duration) {
    assert!(
        actual <= budget,
        "{metric} p95 {:.4} ms exceeded ADR 0009 budget {:.4} ms",
        duration_ms(actual),
        duration_ms(budget),
    );
}

fn defender_acceptance_p95_proof() {
    let mut app = app_with_bots(true);
    add_acceptance_timing(&mut app);
    for _ in 0..WARMUP_FRAMES {
        app.update();
    }
    assert_warmed_load(&mut app, true);

    {
        let mut timings = app.world_mut().resource_mut::<AcceptanceTimings>();
        timings.allocation.clear();
        timings.project.clear();
        timings.invalidate.clear();
        timings.acquire.clear();
        timings.separation.clear();
        timings.allocation.reserve(ACCEPTANCE_SAMPLE_FRAMES);
        timings.project.reserve(ACCEPTANCE_SAMPLE_FRAMES);
        timings.invalidate.reserve(ACCEPTANCE_SAMPLE_FRAMES);
        timings.acquire.reserve(ACCEPTANCE_SAMPLE_FRAMES);
        timings.separation.reserve(ACCEPTANCE_SAMPLE_FRAMES);
    }

    let mut frames = Vec::with_capacity(ACCEPTANCE_SAMPLE_FRAMES);
    for _ in 0..ACCEPTANCE_SAMPLE_FRAMES {
        let started = Instant::now();
        app.update();
        frames.push(started.elapsed());
    }

    let timings = app.world().resource::<AcceptanceTimings>();
    assert_eq!(
        timings.allocation.len(),
        ACCEPTANCE_SAMPLE_FRAMES,
        "each measured frame must execute one RegionalAllocation Project/Invalidate/Acquire pass",
    );
    assert_eq!(
        timings.separation.len(),
        ACCEPTANCE_SAMPLE_FRAMES,
        "each measured frame must execute one local-separation pass",
    );
    assert_eq!(
        (
            timings.project.len(),
            timings.invalidate.len(),
            timings.acquire.len(),
        ),
        (
            ACCEPTANCE_SAMPLE_FRAMES,
            ACCEPTANCE_SAMPLE_FRAMES,
            ACCEPTANCE_SAMPLE_FRAMES,
        ),
        "each measured allocation pass must include Project, Invalidate, and Acquire",
    );

    let frame_p95 = p95(&frames);
    let allocation_p95 = p95(&timings.allocation);
    let project_p95 = p95(&timings.project);
    let invalidate_p95 = p95(&timings.invalidate);
    let acquire_p95 = p95(&timings.acquire);
    let separation_p95 = p95(&timings.separation);
    println!(
        "acceptance_p95 steady_threat_response_frame samples={} frame={:.4}ms/{:.4}ms regional_allocation_project_invalidate_acquire={:.4}ms/{:.4}ms project={:.4}ms invalidate={:.4}ms acquire={:.4}ms local_separation={:.4}ms/{:.4}ms",
        ACCEPTANCE_SAMPLE_FRAMES,
        duration_ms(frame_p95),
        duration_ms(FRAME_P95_BUDGET),
        duration_ms(allocation_p95),
        duration_ms(ALLOCATION_P95_BUDGET),
        duration_ms(project_p95),
        duration_ms(invalidate_p95),
        duration_ms(acquire_p95),
        duration_ms(separation_p95),
        duration_ms(SEPARATION_P95_BUDGET),
    );

    assert_within_budget("whole frame", frame_p95, FRAME_P95_BUDGET);
    assert_within_budget(
        "RegionalAllocation Project/Invalidate/Acquire fast path",
        allocation_p95,
        ALLOCATION_P95_BUDGET,
    );
    assert_within_budget("local separation", separation_p95, SEPARATION_P95_BUDGET);
}

fn swarm_acceptance(c: &mut Criterion) {
    let mut group = c.benchmark_group("swarm_acceptance_5000_bots");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(10));
    group.warm_up_time(Duration::from_secs(2));
    group.throughput(Throughput::Elements(BOT_COUNT as u64));

    let mut steady = warmed_app(true);
    group.bench_function("steady_threat_response_frame", |b| {
        b.iter(|| steady.update())
    });

    let mut exhausted = warmed_app(false);
    group.bench_function("exhausted_gather_frame", |b| b.iter(|| exhausted.update()));

    let mut sparse_stranded = warmed_sparse_stranded_app();
    group.bench_function("sparse_distant_gather_frame", |b| {
        b.iter(|| sparse_stranded.update())
    });

    group.finish();
}

fn main() {
    defender_acceptance_p95_proof();
    if std::env::var_os(PROOF_ONLY_ENV).is_some() {
        return;
    }

    let mut criterion = Criterion::default().configure_from_args();
    swarm_acceptance(&mut criterion);
    criterion.final_summary();
}
