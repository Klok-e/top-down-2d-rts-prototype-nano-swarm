//! Fixed-tick comparison of stocked deliveries through two single-file lanes.
#[path = "../tests/common/mod.rs"]
mod common;

use bevy::{
    ecs::{entity::EntityHashMap, schedule::ExecutorKind},
    prelude::*,
};
use rand::{RngExt, SeedableRng, rngs::StdRng};
use serde_json::{Value, json};
use std::time::Instant;
use top_down_2d_rts_prototype_nano_swarm::{
    battle_statistics::BattleCounters,
    intent::{IntentGrid, IntentKind},
    nanobot::{
        Cargo, NanobotType, OwnerSwarm, PopulationDemandPlugin, ProductionFacility,
        ProductionPlugin, SwarmId,
    },
    resources::{ResourceKind, ResourceLedger, Stockpile},
    tuning::{Tuning, TuningStore},
};

const MAP_SIZE: i32 = 48;
const PICKUP_STOCK: u32 = 2_000;
const HAULERS: usize = 24;
const TICKS: usize = 7_200;
const SEEDS: [u64; 5] = [1, 11, 42, 73, 117];
const CROSSING_X: f32 = 180.0;
const GAP_RANGES: [(f32, f32); 2] = [(72.0, 144.0), (216.0, 288.0)];

fn set_up(tuning: &Tuning, seed: u64) -> (App, Entity, [Entity; 3]) {
    let mut app = common::sim_app_with_gather_haul();
    app.insert_resource(tuning.clone())
        .init_resource::<BattleCounters>()
        .add_plugins((PopulationDemandPlugin, ProductionPlugin));
    app.edit_schedule(FixedUpdate, |schedule| {
        schedule.set_executor_kind(ExecutorKind::SingleThreaded);
    });

    let mut grid = IntentGrid::new(MAP_SIZE, MAP_SIZE);
    for y in -12..12 {
        for x in 2..19 {
            // Keep typed demand above the three facilities' 120-second production capacity.
            grid.paint(IVec2::new(x, y), IntentKind::Defend, SwarmId::PLAYER);
        }
    }
    app.insert_resource(grid);
    let owner = common::spawn_swarm_at(&mut app, Vec2::new(-324.0, 180.0));
    let pickup = common::spawn_sink_stockpile(
        &mut app,
        Vec2::new(-324.0, 180.0),
        PICKUP_STOCK,
        PICKUP_STOCK,
    );
    app.world_mut().entity_mut(pickup).insert(OwnerSwarm(owner));
    app.world_mut().get_mut::<Transform>(pickup).unwrap().scale = Vec3::new(3.375, 3.375, 1.0);
    app.world_mut()
        .resource_mut::<ResourceLedger>()
        .add(ResourceKind::Minerals, PICKUP_STOCK);

    let facilities = [-108.0, 180.0, 468.0].map(|y| {
        app.world_mut()
            .spawn((
                ProductionFacility::with_tuning(tuning),
                OwnerSwarm(owner),
                Transform::from_xyz(900.0, y, 0.0).with_scale(Vec3::new(3.375, 3.375, 1.0)),
            ))
            .id()
    });
    let bound = MAP_SIZE as f32 * 256.0;
    for (low, high) in [
        (-bound, GAP_RANGES[0].0),
        (GAP_RANGES[0].1, GAP_RANGES[1].0),
        (GAP_RANGES[1].1, bound),
    ] {
        let wall = common::spawn_structure_at(&mut app, Vec2::new(CROSSING_X, (low + high) * 0.5));
        app.world_mut().get_mut::<Transform>(wall).unwrap().scale =
            Vec3::new(CROSSING_X / 32.0, (high - low) / 64.0, 1.0);
    }

    let mut rng = StdRng::seed_from_u64(seed);
    for index in 0..HAULERS {
        let x = [-158.0, -72.0, 432.0, 518.0][index / 6];
        let y = -45.0 + (index % 6) as f32 * 90.0;
        common::spawn_hauler_at(
            &mut app,
            Vec2::new(
                x + rng.random_range(-4.0..4.0),
                y + rng.random_range(-4.0..4.0),
            ),
        );
    }
    (app, pickup, facilities)
}

fn run(tuning: &Tuning, seed: u64) -> Value {
    let (mut app, pickup, facilities) = set_up(tuning, seed);
    let mut bots = app
        .world_mut()
        .query::<(Entity, &NanobotType, &Transform)>();
    let mut previous: EntityHashMap<_> = bots
        .iter(app.world())
        .filter(|(_, kind, _)| **kind == NanobotType::Hauler)
        .map(|(entity, _, transform)| (entity, transform.translation.truncate()))
        .collect();
    let mut tick_ms = Vec::with_capacity(TICKS);
    let mut crossings = [[0; 2]; 2];
    for _ in 0..TICKS {
        let start = Instant::now();
        app.update();
        tick_ms.push(start.elapsed().as_secs_f64() * 1_000.0);
        for (bot, kind, transform) in bots.iter(app.world()) {
            if *kind != NanobotType::Hauler {
                continue;
            }
            let after = transform.translation.truncate();
            let Some(before) = previous.insert(bot, after) else {
                continue;
            };
            if (before.x < CROSSING_X) != (after.x < CROSSING_X) {
                let fraction = (CROSSING_X - before.x) / (after.x - before.x);
                let y = before.lerp(after, fraction).y;
                let lane = GAP_RANGES
                    .iter()
                    .position(|&(low, high)| (low..high).contains(&y))
                    .expect("crossing used a marked lane");
                let direction = usize::from(after.x < CROSSING_X);
                crossings[lane][direction] += 1;
            }
        }
    }

    let world = app.world_mut();
    let fixed_time = world.resource::<Time<Fixed>>();
    assert_eq!(fixed_time.elapsed(), fixed_time.timestep() * TICKS as u32);
    let totals = world
        .resource::<BattleCounters>()
        .totals_for(SwarmId::PLAYER);
    let input: u64 = facilities
        .iter()
        .map(|&facility| {
            u64::from(
                world
                    .get::<ProductionFacility>(facility)
                    .unwrap()
                    .input_amount,
            )
        })
        .sum();
    let held: u64 = world
        .query::<&Cargo>()
        .iter(world)
        .map(|cargo| u64::from(cargo.amount))
        .sum();
    // The three hoppers started empty, and only their production cycles consume minerals.
    let delivered = input + totals.minerals_consumed;
    assert_eq!(
        delivered + held + u64::from(world.get::<Stockpile>(pickup).unwrap().amount),
        u64::from(PICKUP_STOCK),
        "seed {seed} conserves stock, cargo, hopper contents, and paid mineral cost",
    );
    tick_ms.sort_unstable_by(f64::total_cmp);
    let percentile = |p: usize| tick_ms[(p * TICKS).div_ceil(100) - 1];
    json!({
        "seed": seed,
        "delivered_minerals": delivered,
        "released_births": totals.births,
        "tick_ms": { "p50": percentile(50), "p95": percentile(95) },
        "crossings": {
            "lower": { "east": crossings[0][0], "west": crossings[0][1] },
            "upper": { "east": crossings[1][0], "west": crossings[1][1] },
        },
    })
}

fn main() {
    let (_, tuning) = TuningStore::load(TuningStore::default_path()).expect("shipped balance");
    let runs: Vec<_> = SEEDS.into_iter().map(|seed| run(&tuning, seed)).collect();
    println!(
        "{}",
        json!({
            "map": [MAP_SIZE, MAP_SIZE],
            "initial_haulers": HAULERS,
            "initial_stock": PICKUP_STOCK,
            "ticks": TICKS,
            "runs": runs,
        })
    );
}
