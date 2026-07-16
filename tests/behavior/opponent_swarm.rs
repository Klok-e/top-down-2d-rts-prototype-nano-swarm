//! Integration tests for issue #15: prepainted opponent swarm
//! using the same systems as the player swarm.
//!
//! Each test isolates one behavior so a failure points at a
//! single contract: opponent can be initialized with prepainted
//! intent, opponent production uses its own fixed priority through
//! the same production systems, and opponent nanobots are
//! driven by the same scoring as player nanobots.
//!
//! The tests deliberately avoid testing "active dynamic enemy
//! AI" because the acceptance criteria explicitly say it is
//! not required for the first implementation.

use bevy::{math::Vec2, prelude::*};
use top_down_2d_rts_prototype_nano_swarm::{
    ZONE_BLOCK_SIZE,
    intent::{IntentGrid, IntentKind},
    nanobot::{
        Commitment, Health, Nanobot, NanobotBundle, NanobotType, OpponentSwarm, OwnerSwarm,
        PRODUCTION_COST_PER_BOT, PRODUCTION_TICKS_PER_BOT, PrepaintedIntent, ProductionFacility,
        ProductionPriority, SeedNanobots, SoftWorkSlots, Swarm, SwarmId, SwarmProduction,
        VelocityComponent, best_candidate, spawn_opponent_swarm,
    },
};

#[path = "../common/mod.rs"]
mod common;

fn build_app() -> App {
    // Use an empty global priority as a default; opponent tests
    // set their own priority via SwarmProduction.
    let mut app = common::sim_app_with_production();
    app.insert_resource(ProductionPriority::new());
    app
}

fn children_of(world: &World, parent: Entity) -> Vec<Entity> {
    world
        .get::<Children>(parent)
        .map(|c| c.iter().collect())
        .unwrap_or_default()
}

/// Bot position one cell away from `(cell_x, cell_y)` on the
/// same row, so the distance penalty in the scoring function
/// does not bury the prepainted cell's score.
fn world_to_bot_pos(cell_x: i32, cell_y: i32) -> Vec2 {
    Vec2::new(
        (cell_x as f32 - 0.5) * ZONE_BLOCK_SIZE,
        (cell_y as f32 + 0.5) * ZONE_BLOCK_SIZE,
    )
}

#[test]
fn opponent_swarm_can_be_initialized_with_prepainted_intent() {
    let mut app = build_app();
    let opponent_pos = Vec2::new(2000.0, 0.0);
    let gather_cell = IVec2::new(2, 0);
    let mut priority = ProductionPriority::new();
    priority.set_weight(NanobotType::Worker, 5);

    let opponent = spawn_opponent_swarm(
        app.world_mut(),
        opponent_pos,
        priority.clone(),
        &[PrepaintedIntent::new(gather_cell, IntentKind::Gather)],
        &[SeedNanobots::new(NanobotType::Worker, 2)],
    );

    let world = app.world();
    assert!(world.entity(opponent).get::<OpponentSwarm>().is_some());
    assert!(world.entity(opponent).get::<Swarm>().is_some());
    let swarm_priority = world
        .entity(opponent)
        .get::<SwarmProduction>()
        .expect("opponent must carry a SwarmProduction");
    assert_eq!(swarm_priority.priority.weight(NanobotType::Worker), 5);

    let grid = world.resource::<IntentGrid>();
    let cell = grid.cell(gather_cell).expect("cell must be in bounds");
    assert!(cell.has(IntentKind::Gather));

    let children = children_of(world, opponent);
    // Issue #38 / ADR-0004: the seed nanobots are
    // top-level entities, not children. The swarm is
    // a spawn-origin / ownership marker; the seed
    // bot's `SwarmMember(swarm_id)` is what the
    // per-swarm intent filter matches. Count Workers
    // owned by this swarm rather than children.
    let swarm_id = world
        .entity(opponent)
        .get::<SwarmId>()
        .copied()
        .expect("opponent swarm must carry a SwarmId");
    let mut worker_seeded = 0;
    {
        let world_mut = app.world_mut();
        let mut query = world_mut.query::<(
            &top_down_2d_rts_prototype_nano_swarm::nanobot::SwarmMember,
            &NanobotType,
        )>();
        for (member, nanobot_type) in query.iter(world_mut) {
            if member.0 != swarm_id {
                continue;
            }
            if *nanobot_type == NanobotType::Worker {
                worker_seeded += 1;
            }
        }
    }
    assert_eq!(
        worker_seeded, 2,
        "opponent must seed 2 Workers owned by SwarmId {swarm_id:?}"
    );
    // `children` is now expected to be empty:
    // nanobots are top-level, not children.
    assert!(
        children.is_empty(),
        "issue #38 / ADR-0004: seed nanobots are top-level, not children"
    );
}

#[test]
fn opponent_nanobot_picks_prepainted_intent_via_same_scoring() {
    // The load-bearing assertion: prepainted intent flows
    // through the same `best_candidate` the player autonomy
    // system uses.
    let mut app = build_app();
    let opponent_pos = Vec2::new(2000.0, 0.0);
    let gather_cell = IVec2::new(2, 0);
    let mut priority = ProductionPriority::new();
    priority.set_weight(NanobotType::Worker, 1);
    let _opponent = spawn_opponent_swarm(
        app.world_mut(),
        opponent_pos,
        priority,
        &[PrepaintedIntent::new(gather_cell, IntentKind::Gather)],
        &[SeedNanobots::new(NanobotType::Worker, 1)],
    );
    app.update();

    let grid = app.world().resource::<IntentGrid>();
    let slots = app.world().resource::<SoftWorkSlots>();
    // The opponent's `SwarmId` is the owner stamped on the
    // prepainted cell; the scoring filter requires the
    // candidate nanobot's swarm to match. Reading the id off
    // the opponent entity keeps the test in lock-step with
    // `spawn_opponent_swarm`'s allocation rather than
    // hard-coding a value.
    let opponent_id = app
        .world()
        .entity(_opponent)
        .get::<SwarmId>()
        .copied()
        .expect("opponent swarm must carry a SwarmId");
    let bot_pos = world_to_bot_pos(2, 0);
    let picked = best_candidate(
        grid,
        NanobotType::Worker,
        Commitment::Idle,
        bot_pos,
        slots,
        ZONE_BLOCK_SIZE,
        &IntentKind::ALL,
        opponent_id,
    )
    .expect("opponent nanobot must find a candidate via the same scoring");
    assert_eq!(picked.cell, gather_cell);
    assert_eq!(picked.kind, IntentKind::Gather);
    assert!(picked.score > 0.0);
}

#[test]
fn opponent_swarm_uses_own_fixed_priority_through_same_production_systems() {
    // Opponent targets 4 Haulers; player uses the global
    // 10/3/1 mix. Each facility has an OwnerSwarm, so the
    // pick path uses the owner's priority. Opponent picks
    // Hauler (its only large deficit), player picks Worker
    // (its largest deficit under the global priority).
    let mut app = build_app();
    {
        let mut priority = app.world_mut().resource_mut::<ProductionPriority>();
        priority.set_weight(NanobotType::Worker, 10);
        priority.set_weight(NanobotType::Hauler, 3);
        priority.set_weight(NanobotType::Defender, 1);
    }
    let player_pos = Vec2::new(0.0, 0.0);
    let player_swarm = app
        .world_mut()
        .spawn((
            Swarm {},
            Transform::from_translation(player_pos.extend(0.0)),
        ))
        .id();
    app.world_mut().entity_mut(player_swarm).with_children(|p| {
        p.spawn((
            NanobotBundle {
                nanobot: Nanobot {},
                nanobot_type: NanobotType::Worker,
                velocity: VelocityComponent::default(),
                ai_state: Default::default(),
                health: Health::default(),
                swarm_member: top_down_2d_rts_prototype_nano_swarm::nanobot::SwarmMember::new(
                    top_down_2d_rts_prototype_nano_swarm::nanobot::SwarmId::PLAYER,
                ),
            },
            Commitment::Idle,
            Transform::from_translation(player_pos.extend(0.0)),
        ));
    });

    let opponent_pos = Vec2::new(2000.0, 0.0);
    let mut opponent_priority = ProductionPriority::new();
    opponent_priority.set_weight(NanobotType::Hauler, 4);
    let opponent = spawn_opponent_swarm(app.world_mut(), opponent_pos, opponent_priority, &[], &[]);

    let _player_stock =
        common::spawn_stockpile(&mut app, player_pos, PRODUCTION_COST_PER_BOT * 5, 1000);
    let _opponent_stock =
        common::spawn_stockpile(&mut app, opponent_pos, PRODUCTION_COST_PER_BOT * 5, 1000);

    let player_facility = app
        .world_mut()
        .spawn((
            ProductionFacility::new(),
            OwnerSwarm(player_swarm),
            Transform::from_translation(player_pos.extend(0.0)),
        ))
        .id();
    common::fill_facility_input(&mut app, player_facility);
    let opponent_facility = app
        .world_mut()
        .spawn((
            ProductionFacility::new(),
            OwnerSwarm(opponent),
            Transform::from_translation(opponent_pos.extend(0.0)),
        ))
        .id();
    common::fill_facility_input(&mut app, opponent_facility);

    app.update();

    let world = app.world();
    let opp_state = world
        .entity(opponent_facility)
        .get::<ProductionFacility>()
        .unwrap();
    assert_eq!(
        opp_state.current_target,
        Some(NanobotType::Hauler),
        "opponent facility must pick from the opponent's own fixed priority"
    );
    let player_state = world
        .entity(player_facility)
        .get::<ProductionFacility>()
        .unwrap();
    // Issue #32: production now picks the type with the
    // largest **proportional** deficit, not the largest
    // count gap. With the global priority W10/H3/D1
    // (normalized 71.4% / 21.4% / 7.1%) and one Worker
    // in the player swarm (current share 100% / 0% / 0%),
    // the largest positive share deficit is on Hauler
    // (target 21.4%, current 0%). The exact *type* picked
    // is less important than the assertion that the
    // global priority still drives the player's facility
    // and the opponent's `SwarmProduction` override still
    // drives the opponent's facility, even when they
    // happen to agree (they agree here because both
    // priorities call for Hauler first).
    assert_eq!(
        player_state.current_target,
        Some(NanobotType::Hauler),
        "player facility must keep using the global priority (proportional picker)"
    );
}

#[test]
fn opponent_production_spawns_nanobots_as_children_of_opponent_swarm() {
    // A full production cycle for the opponent must end
    // with a new nanobot parented to the opponent swarm.
    let mut app = build_app();
    let opponent_pos = Vec2::new(2000.0, 0.0);
    let mut opponent_priority = ProductionPriority::new();
    opponent_priority.set_weight(NanobotType::Worker, 1);
    let opponent = spawn_opponent_swarm(app.world_mut(), opponent_pos, opponent_priority, &[], &[]);
    let _stockpile =
        common::spawn_stockpile(&mut app, opponent_pos, PRODUCTION_COST_PER_BOT * 5, 1000);
    let _facility = app
        .world_mut()
        .spawn((
            ProductionFacility::new(),
            OwnerSwarm(opponent),
            Transform::from_translation(opponent_pos.extend(0.0)),
        ))
        .id();
    common::fill_facility_input(&mut app, _facility);

    for _ in 0..(1 + PRODUCTION_TICKS_PER_BOT as usize) {
        app.update();
    }

    // Issue #38 / ADR-0004: production-spawned nanobots
    // are top-level entities whose `SwarmMember` matches
    // the opponent's `SwarmId`. The swarm no longer
    // parents the produced bots. Count the new bot by
    // matching `SwarmMember == opponent.SwarmId`.
    let opponent_swarm_id = app
        .world()
        .entity(opponent)
        .get::<SwarmId>()
        .copied()
        .expect("opponent swarm must carry a SwarmId");
    let mut produced_bots = 0;
    {
        let world = app.world_mut();
        let mut query = world.query::<(
            Entity,
            &top_down_2d_rts_prototype_nano_swarm::nanobot::SwarmMember,
        )>();
        for (_entity, member) in query.iter(world) {
            if member.0 == opponent_swarm_id {
                produced_bots += 1;
            }
        }
    }
    assert!(
        produced_bots >= 1,
        "opponent swarm must receive the production-spawned nanobot owned by SwarmId {opponent_swarm_id:?}"
    );
    // No non-opponent swarm exists in this test, so the
    // production chain did not invent a player swarm.
    let mut player_swarm_query = app
        .world_mut()
        .query_filtered::<Entity, (With<Swarm>, Without<OpponentSwarm>)>();
    assert_eq!(
        player_swarm_query.iter(app.world()).count(),
        0,
        "no player swarm must exist when only the opponent is initialized"
    );
}

#[test]
fn opponent_uses_default_priority_when_swarm_production_absent() {
    // Backward compatibility: a swarm without
    // SwarmProduction falls back to the global
    // ProductionPriority. Pre-existing tests depend on this.
    let mut app = build_app();
    {
        let mut priority = app.world_mut().resource_mut::<ProductionPriority>();
        priority.set_weight(NanobotType::Defender, 5);
    }
    let swarm_pos = Vec2::new(0.0, 0.0);
    let swarm = app
        .world_mut()
        .spawn((Swarm {}, Transform::from_translation(swarm_pos.extend(0.0))))
        .id();
    let _stockpile =
        common::spawn_stockpile(&mut app, swarm_pos, PRODUCTION_COST_PER_BOT * 5, 1000);
    let facility = app
        .world_mut()
        .spawn((
            ProductionFacility::new(),
            OwnerSwarm(swarm),
            Transform::from_translation(swarm_pos.extend(0.0)),
        ))
        .id();
    common::fill_facility_input(&mut app, facility);

    app.update();

    // Global priority wants Defender; no SwarmProduction so the
    // facility must use the global. (Worker and Hauler
    // targets are 0; Defender is 5.)
    let state = app
        .world()
        .entity(facility)
        .get::<ProductionFacility>()
        .unwrap();
    assert_eq!(
        state.current_target,
        Some(NanobotType::Defender),
        "facility without a swarm-specific priority must fall back to the global ProductionPriority"
    );
}
