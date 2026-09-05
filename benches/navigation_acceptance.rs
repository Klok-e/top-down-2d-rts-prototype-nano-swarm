//! Reproducible headless navigation workload: cargo bench --bench navigation_acceptance.
#[path = "../tests/common/mod.rs"]
mod common;
#[path = "navigation_acceptance/reference.rs"]
mod reference;
use bevy::{prelude::*, time::TimeUpdateStrategy};
use serde_json::json;
use std::{
    fs,
    time::{Duration, Instant},
};
use top_down_2d_rts_prototype_nano_swarm::{
    game_settings::GameSettings,
    intent::IntentGrid,
    nanobot::DirectMovementComponent,
    navigation::{BODY_RADIUS, CELL_WIDTH, Navigation, Obstacle},
};
const BOTS: usize = 5000;
const MAP: i32 = 64;
fn obstacles(name: &str) -> Vec<Obstacle> {
    match name {
        "obstacle_dense" => (0..8)
            .flat_map(|x| {
                (0..7).map(move |y| Obstacle::Rectangle {
                    center: Vec2::new(x as f32 * 720.0 + 36.0, y as f32 * 1152.0 - 3420.0),
                    half: Vec2::new(144.0, 360.0),
                })
            })
            .collect(),
        "bottleneck" => vec![
            Obstacle::Rectangle {
                center: Vec2::new(36.0, -8192.0),
                half: Vec2::new(36.0, 8192.0),
            },
            Obstacle::Rectangle {
                center: Vec2::new(36.0, 8228.0),
                half: Vec2::new(36.0, 8156.0),
            },
        ],
        "simultaneous_replanning" => vec![Obstacle::Rectangle {
            center: Vec2::new(1008.0, 36.0),
            half: Vec2::new(36.0, 4608.0),
        }],
        _ => vec![],
    }
}
fn install_obstacles(app: &mut App, shapes: &[Obstacle]) {
    for &shape in shapes {
        let Obstacle::Rectangle { center, half } = shape else {
            unreachable!()
        };
        let e = common::spawn_structure_at(app, center);
        app.world_mut().get_mut::<Transform>(e).unwrap().scale = (half / 32.0).extend(1.0);
    }
}
fn percentile(values: &[f64], percent: usize) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    sorted[(sorted.len() * percent).div_ceil(100) - 1]
}
fn main() {
    let ticks: usize = std::env::var("NAV_BENCH_TICKS")
        .ok()
        .map(|v| v.parse().unwrap())
        .unwrap_or(600);
    let directory =
        std::env::var("NAV_BENCH_OUTPUT").unwrap_or_else(|_| "target/issue-66/scale".into());
    fs::create_dir_all(&directory).unwrap();
    let cpu = fs::read_to_string("/proc/cpuinfo")
        .unwrap_or_default()
        .lines()
        .find(|l| l.starts_with("model name"))
        .unwrap_or("unknown")
        .to_owned();
    let mut results = vec![];
    for name in [
        "open",
        "obstacle_dense",
        "bottleneck",
        "simultaneous_replanning",
    ] {
        let mut app = common::sim_app_with_movement();
        app.insert_resource(IntentGrid::new(MAP, MAP));
        app.insert_resource(Time::<Fixed>::from_duration(Duration::from_nanos(
            16_666_667,
        )));
        app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_nanos(
            16_666_667,
        )));
        app.world_mut().resource_mut::<GameSettings>().bot_speed = 5.0;
        let shapes = obstacles(name);
        if name != "simultaneous_replanning" {
            install_obstacles(&mut app, &shapes);
        }
        let mut bots = vec![];
        for i in 0..BOTS {
            let start = Vec2::new(
                -252.0 - (i % 100) as f32 * 72.0,
                36.0 + if (i / 100) % 2 == 0 {
                    (i / 200) as f32 * 144.0
                } else {
                    -((i / 200 + 1) as f32) * 144.0
                },
            );
            let e = common::spawn_worker_at(&mut app, start);
            app.world_mut()
                .entity_mut(e)
                .insert(DirectMovementComponent {
                    xy: start + Vec2::X * 15000.0,
                    stop_radius: 0.0,
                    interaction: None,
                    speed: Some(5.0),
                });
            bots.push((e, start));
        }
        let mut previous: Vec<_> = bots.iter().map(|(_, p)| *p).collect();
        let mut records = vec![];
        let mut times = vec![];
        let mut moved = std::collections::HashSet::new();
        let mut crossed = std::collections::HashSet::new();
        for tick in 0..ticks {
            if name == "simultaneous_replanning" && tick == 30 {
                assert_eq!(
                    moved.len(),
                    BOTS,
                    "all bots must have active moving routes before invalidation"
                );
                install_obstacles(&mut app, &shapes);
            }
            let now = Instant::now();
            app.update();
            let ms = now.elapsed().as_secs_f64() * 1000.0;
            times.push(ms);
            let work = app.world().resource::<Navigation>().work();
            assert!(work.work <= 32768);
            let mut moving = 0;
            for (index, &(e, start)) in bots.iter().enumerate() {
                let p = app
                    .world()
                    .get::<Transform>(e)
                    .unwrap()
                    .translation
                    .truncate();
                if p.x > 72.0 {
                    crossed.insert(e);
                }
                if p.distance_squared(start) > 0.01 {
                    moved.insert(e);
                }
                if p.distance_squared(previous[index]) > 0.01 {
                    moving += 1;
                }
                assert!(
                    app.world()
                        .resource::<Navigation>()
                        .segment_clear(previous[index], p),
                    "{name} movement crossed blocker"
                );
                previous[index] = p;
                assert!(
                    app.world().resource::<Navigation>().point_clear(p),
                    "{name} bot entered blocker at {p}"
                );
            }
            records.push(json!({"tick":tick,"tick_ms":ms,"pending":work.pending,"completed":work.completed,"completion_max_latency_ticks":work.max_latency_ticks,"cooperative_work":work.work,"hierarchy_cells":work.hierarchy_cells,"hierarchy_chunks":work.hierarchy_chunks,"coarse_expansions":work.coarse_expansions,"fine_expansions":work.fine_expansions,"moving":moving}));
        }
        let summary = json!({"scenario":name,"spawned":BOTS,"moved_at_least_once":moved.len(),"crossed_x72":crossed.len(),"ticks":ticks,"tick_ms":{"p50":percentile(&times,50),"p95":percentile(&times,95),"p99":percentile(&times,99)},"obstacles":shapes.len(),"final":records.last()});
        println!("{summary}");
        fs::write(
            format!("{directory}/{name}.json"),
            serde_json::to_vec_pretty(&json!({"summary":summary,"ticks":records})).unwrap(),
        )
        .unwrap();
        results.push(summary);
    }
    let comparison = reference::run();
    let report = json!({"cpu":cpu,"logical_cpus":std::thread::available_parallelism().unwrap().get(),"map_intent_cells":[MAP,MAP],"map_world_units":[MAP*512,MAP*512],"body_radius":BODY_RADIUS,"cell_width":CELL_WIDTH,"chunk_width_cells":8,"budget":32768,"simulation_hz":60,"profile":"bench","scope":"minimal simulation movement and navigation, excludes allocation/economy/rendering","queue_latency_note":"completion latency is observed only for completed requests; pending requests are right censored at final tick","scenarios":results,"route_comparison":comparison});
    fs::write(
        format!("{directory}/report.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
}
