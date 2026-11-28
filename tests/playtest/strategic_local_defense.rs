#[path = "../common/mod.rs"]
mod common;

use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    intent::{IntentGrid, IntentKind},
    nanobot::{CombatPlugin, DefenderResponse, PlannedKind, SwarmId, SwarmMember},
    reachability::SwarmReachabilityMap,
    strategic_runtime::{StrategicController, StrategicControllerPlugin},
    tuning::Tuning,
};

#[test]
fn crossing_a_mine_draws_a_local_response_without_moving_the_offensive_front() {
    let mut app = common::sim_app();
    app.insert_resource(IntentGrid::new(24, 24))
        .add_plugins((StrategicControllerPlugin, CombatPlugin));
    let owner = SwarmId::PLAYER;
    let home = common::cell_world_center(IVec2::ZERO);
    let own = common::spawn_swarm_at(&mut app, home);
    app.world_mut()
        .entity_mut(own)
        .insert(StrategicController::new(owner));
    let front = IVec2::new(8, 0);
    let enemy_home = common::cell_world_center(front);
    let enemy = common::spawn_opponent_swarm_with_nanobots(&mut app, enemy_home, &[]);
    let enemy_id = *app.world().get::<SwarmId>(enemy).unwrap();
    let target =
        common::spawn_empty_completed_structure(&mut app, enemy, PlannedKind::ProductionFacility);
    app.world_mut()
        .entity_mut(target)
        .insert(Transform::from_translation(enemy_home.extend(0.0)));
    let mine = IVec2::X;
    let mine_position = common::cell_world_center(mine);
    common::spawn_deposit(
        &mut app,
        common::DepositFixture {
            world_pos: mine_position,
            amount: 72_000,
            capacity: 72_000,
            radius: 64.0,
        },
    );
    app.world_mut()
        .resource_mut::<IntentGrid>()
        .paint(mine, IntentKind::Gather, owner);
    let reachability = SwarmReachabilityMap::new(
        app.world().resource::<IntentGrid>(),
        [],
        [(owner, home), (enemy_id, enemy_home)],
    )
    .unwrap();
    app.insert_resource(reachability);
    let mut tuning = app.world_mut().resource_mut::<Tuning>();
    tuning.autonomy.strategy_staging_spacing_cells = 64;
    tuning.autonomy.controller_review_period_ticks = 30;
    let defenders = [-90.0, -30.0, 30.0, 90.0]
        .map(|offset| common::spawn_defender_at(&mut app, home + Vec2::Y * offset));
    let at_front = |app: &App| {
        app.world()
            .resource::<IntentGrid>()
            .cell(front + IVec2::Y)
            .is_some_and(|cell| cell.has_owned(IntentKind::Defend, owner))
    };
    for _ in 0..31 {
        app.update();
    }
    assert!(at_front(&app), "the offensive front must already be active");

    let worker = common::spawn_worker_at(&mut app, mine_position + Vec2::Y * 180.0);
    app.world_mut()
        .entity_mut(worker)
        .insert(SwarmMember(enemy_id));
    let mut saw_local_response = false;
    for _ in 0..30 {
        app.update();
        saw_local_response |= defenders.iter().any(|&defender| {
            app.world()
                .get::<DefenderResponse>(defender)
                .is_some_and(|response| response.target == worker)
        });
        assert!(
            at_front(&app),
            "the passing Worker moved the offensive front"
        );
    }
    assert!(
        saw_local_response,
        "shared allocation must respond to the intruder"
    );
}
