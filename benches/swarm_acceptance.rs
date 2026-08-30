use std::time::Duration;

use bevy::{prelude::*, time::TimeUpdateStrategy};
use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use top_down_2d_rts_prototype_nano_swarm::{
    game_settings::GameSettings,
    intent::{IntentGrid, IntentKind},
    nanobot::{
        CombatPlugin, Commitment, DefenderResponse, Health, Nanobot, NanobotBundle, NanobotPlugin,
        NanobotType, RegionalAllocationPlugin, SwarmId, SwarmMember,
    },
    resources::ResourceLedger,
};

const BOT_COUNT: usize = 5_000;
const WARMUP_FRAMES: usize = 60;

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

fn warmed_app(defend_work: bool) -> App {
    let mut app = app_with_bots(defend_work);
    for _ in 0..WARMUP_FRAMES {
        app.update();
    }
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

criterion_group!(benches, swarm_acceptance);
criterion_main!(benches);
