//! Observe loaded bots in the default-map headless runtime without changing the scene.
use std::collections::HashMap;

use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    nanobot::{
        ApproachPhase, Cargo, Commitment, CongestionRecovery, DirectMovementComponent,
        ExtractProgress, HaulerLoading, Nanobot, NanobotType, RemainingTravel,
        ReturningToStockpile, TrafficYield, WorkApproach, WorkBlocked,
    },
    navigation::Navigation,
    runtime::{RuntimeOptions, build_runtime_app},
};

#[derive(Default)]
struct Motion {
    position: Vec2,
    stationary_seconds: f32,
    reported: bool,
    direction: Vec2,
    pending_stop_seconds: f32,
}

#[derive(Resource, Default)]
struct Observations {
    ticks: u64,
    loaded: HashMap<Entity, Motion>,
    longest_stop: f32,
    direction_reversals: u64,
    longest_pending_stop: f32,
}

fn main() {
    let mut app = build_runtime_app(RuntimeOptions {
        headless: true,
        agent_socket: true,
        width: 1280,
        height: 720,
    })
    .expect("headless runtime");
    app.init_resource::<Observations>()
        .add_systems(FixedPostUpdate, observe);
    app.run();
}

#[allow(clippy::type_complexity)]
fn observe(
    time: Res<Time<Fixed>>,
    mut observations: ResMut<Observations>,
    navigation: Res<Navigation>,
    bots: Query<
        (
            Entity,
            &Transform,
            &NanobotType,
            &Cargo,
            Option<&WorkApproach>,
            Has<WorkBlocked>,
            Has<CongestionRecovery>,
            Option<&RemainingTravel>,
            Option<&DirectMovementComponent>,
            Option<&TrafficYield>,
            Option<&Commitment>,
            Has<HaulerLoading>,
            Has<ExtractProgress>,
            Option<&ReturningToStockpile>,
        ),
        With<Nanobot>,
    >,
) {
    observations.ticks += 1;
    observations.loaded.retain(|entity, _| {
        bots.get(*entity)
            .is_ok_and(|(_, _, _, cargo, _, _, _, _, _, _, _, _, _, _)| cargo.amount > 0)
    });
    let mut phases = [0_u32; 4];
    let mut cargo_total = 0_u32;
    let mut stalled = 0_u32;
    let mut stationary_causes = [0_u32; 8];
    let mut loaded_pending = 0_u32;
    let mut loaded_working = 0_u32;
    let summary_tick = observations.ticks.is_multiple_of(60);
    for (
        entity,
        transform,
        kind,
        cargo,
        approach,
        blocked,
        recovery,
        remaining,
        movement,
        yielding,
        commitment,
        loading,
        extracting,
        returning,
    ) in &bots
    {
        if cargo.amount == 0 {
            continue;
        }
        cargo_total += cargo.amount;
        phases[match approach.map(|approach| approach.phase) {
            Some(ApproachPhase::Travelling) => 0,
            Some(ApproachPhase::Searching) => 1,
            Some(ApproachPhase::Waiting) => 2,
            None => 3,
        }] += 1;
        let position = transform.translation.truncate();
        let pending_ticks = navigation.pending_movement_ticks(entity);
        loaded_pending += u32::from(pending_ticks.is_some());
        let working = movement.is_none()
            && (loading || extracting || commitment == Some(&Commitment::Working));
        loaded_working += u32::from(working);
        let (cause, cause_index) = if working {
            ("working", 0)
        } else if approach.is_some_and(|approach| approach.phase == ApproachPhase::Waiting) {
            ("destination_waiting", 1)
        } else if blocked {
            ("body_blocked", 2)
        } else if yielding.is_some() {
            ("yielding", 3)
        } else if recovery {
            ("recovery", 4)
        } else if pending_ticks.is_some() {
            ("pending_route", 5)
        } else if movement.is_some() {
            ("movement_without_pending_route", 6)
        } else {
            ("no_movement_goal", 7)
        };
        let motion = observations.loaded.entry(entity).or_insert(Motion {
            position,
            ..default()
        });
        if position.distance(motion.position) < 0.1 {
            stationary_causes[cause_index] += 1;
            motion.stationary_seconds += time.delta_secs();
            if cause_index == 5 {
                motion.pending_stop_seconds += time.delta_secs();
            } else {
                motion.pending_stop_seconds = 0.0;
            }
        } else {
            if motion.reported {
                info!(
                    "MOVEMENT resumed entity={entity:?} kind={kind:?} seconds={:.3} position={position:?} cargo={} approach={approach:?} blocked={blocked} recovery={recovery}",
                    motion.stationary_seconds, cargo.amount
                );
            }
            motion.stationary_seconds = 0.0;
            motion.pending_stop_seconds = 0.0;
            motion.reported = false;
        }
        let displacement = position - motion.position;
        let reversed = displacement.length() >= 0.1
            && motion.direction.dot(displacement.normalize_or_zero()) < -0.01;
        if displacement.length() >= 0.1 {
            motion.direction = displacement.normalize();
        }
        motion.position = position;
        if motion.stationary_seconds >= 1.0 {
            stalled += 1;
            if !motion.reported || summary_tick {
                info!(
                    "MOVEMENT stalled cause={cause} pending_ticks={pending_ticks:?} entity={entity:?} kind={kind:?} position={position:?} cargo={} approach={approach:?} blocked={blocked} recovery={recovery} remaining={remaining:?} movement={movement:?} yielding={yielding:?} commitment={commitment:?} loading={loading} extracting={extracting} returning={returning:?} goal_clear={} navigation={:?}",
                    cargo.amount,
                    movement
                        .is_some_and(|movement| navigation.segment_clear(position, movement.xy)),
                    navigation.work()
                );
                motion.reported = true;
            }
        }
        let stop = motion.stationary_seconds;
        let pending_stop = motion.pending_stop_seconds;
        observations.longest_pending_stop = observations.longest_pending_stop.max(pending_stop);
        observations.longest_stop = observations.longest_stop.max(stop);
        observations.direction_reversals += u64::from(reversed);
    }
    if observations.ticks.is_multiple_of(60) {
        info!(
            "MOVEMENT summary tick={} simulation_seconds={:.3} loaded={} cargo={cargo_total} travelling={} searching={} waiting={} other={} stationary_over_one_second={stalled} longest_stop={:.3} longest_pending_route_stop={:.3} direction_reversals={} loaded_pending={loaded_pending} loaded_working={loaded_working} stationary_working={} stationary_destination_waiting={} stationary_body_blocked={} stationary_yielding={} stationary_recovery={} stationary_pending_route={} stationary_without_pending_route={} stationary_without_goal={} navigation={:?}",
            observations.ticks,
            time.elapsed_secs_f64(),
            observations.loaded.len(),
            phases[0],
            phases[1],
            phases[2],
            phases[3],
            observations.longest_stop,
            observations.longest_pending_stop,
            observations.direction_reversals,
            stationary_causes[0],
            stationary_causes[1],
            stationary_causes[2],
            stationary_causes[3],
            stationary_causes[4],
            stationary_causes[5],
            stationary_causes[6],
            stationary_causes[7],
            navigation.work()
        );
    }
}
