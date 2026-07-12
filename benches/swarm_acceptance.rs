use std::time::Duration;

use bevy::{prelude::*, time::TimeUpdateStrategy};
use criterion::{criterion_group, criterion_main, Criterion, Throughput};
use top_down_2d_rts_prototype_nano_swarm::{
    game_settings::GameSettings,
    intent::{IntentGrid, IntentKind, PAINT_STRENGTH_CAP},
    nanobot::{
        idle_spread_system, move_velocity_system, separation_system, velocity_system, Commitment,
        NanobotBundle, NanobotType, RegionalAllocationPlugin, SwarmId,
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
        .insert_resource(IntentGrid::new(1000, 1000))
        .insert_resource(GameSettings {
            width: 512_000.0,
            height: 512_000.0,
            bot_speed: 5.0,
            debug_draw_circles: false,
        })
        .init_resource::<ResourceLedger>()
        .add_plugins(RegionalAllocationPlugin)
        .add_systems(
            Update,
            (
                separation_system,
                idle_spread_system,
                velocity_system,
                move_velocity_system,
            )
                .chain(),
        );

    if defend_work {
        let mut grid = app.world_mut().resource_mut::<IntentGrid>();
        for y in -8..8 {
            for x in -8..8 {
                grid.add_owned(
                    IVec2::new(x, y),
                    IntentKind::Defend,
                    PAINT_STRENGTH_CAP,
                    Some(SwarmId::PLAYER),
                );
            }
        }
    }

    for i in 0..BOT_COUNT {
        let x = (i % 100) as f32 * 40.0;
        let y = (i / 100) as f32 * 40.0;
        let mut bundle = NanobotBundle::default();
        bundle.nanobot_type = if defend_work {
            NanobotType::Defender
        } else {
            NanobotType::Worker
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
    app
}

fn swarm_acceptance(c: &mut Criterion) {
    let mut group = c.benchmark_group("swarm_acceptance_5000_bots");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(10));
    group.warm_up_time(Duration::from_secs(2));
    group.throughput(Throughput::Elements(BOT_COUNT as u64));

    let mut steady = warmed_app(true);
    group.bench_function("steady_defend_frame", |b| b.iter(|| steady.update()));

    let mut exhausted = warmed_app(false);
    group.bench_function("exhausted_gather_frame", |b| b.iter(|| exhausted.update()));

    group.finish();
}

criterion_group!(benches, swarm_acceptance);
criterion_main!(benches);
