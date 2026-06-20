//! Scenario tests for issue #16: Production Collapse
//! win/loss detection.
//!
//! Each test isolates one recoverable / collapsed state and
//! asserts the corresponding
//! [`ProductionCollapseState`] flag flips. The tests build the
//! smallest Bevy `App` that proves the system wiring:
//! swarms, facilities, and the production + collapse systems
//! chained in order.

use bevy::{math::Vec2, prelude::*};
use top_down_2d_rts_prototype_nano_swarm::{
    game_settings::GameSettings,
    intent::IntentGrid,
    nanobot::{
        bot_debug_circle_system, move_velocity_system, separation_system, velocity_system,
        CollapsePlugin, Commitment, Health, Nanobot, NanobotBundle, NanobotType, OpponentSwarm,
        OwnerSwarm, ProductionCollapseState, ProductionFacility, ProductionPlugin, ProductionRatio,
        SoftWorkSlots, Swarm, SwarmProduction, VelocityComponent, PRODUCTION_COST_PER_BOT,
    },
    resources::{ResourceKind, ResourceLedger, Stockpile},
};

fn build_app() -> App {
    let mut app = App::new();
    app.add_plugins(bevy::time::TimePlugin);
    app.insert_resource(IntentGrid::new(8, 8));
    app.insert_resource(GameSettings {
        width: 1000.0,
        height: 1000.0,
        bot_speed: 5.0,
        debug_draw_circles: false,
    });
    app.init_resource::<SoftWorkSlots>();
    app.init_resource::<ResourceLedger>();
    // Empty global ratio by default; each test sets the
    // ratio(s) it needs.
    app.insert_resource(ProductionRatio::new());
    app.add_systems(
        Update,
        (
            separation_system,
            velocity_system,
            move_velocity_system,
            bot_debug_circle_system,
        )
            .chain(),
    );
    app.add_plugins(ProductionPlugin);
    app.add_plugins(CollapsePlugin);
    app
}

fn spawn_swarm_with_nanobots(
    app: &mut App,
    world_pos: Vec2,
    counts: &[(NanobotType, u32)],
) -> Entity {
    let swarm = app
        .world_mut()
        .spawn((Swarm {}, Transform::from_translation(world_pos.extend(0.0))))
        .id();
    app.world_mut().entity_mut(swarm).with_children(|p| {
        for (kind, n) in counts {
            for _ in 0..*n {
                p.spawn((
                    NanobotBundle {
                        nanobot: Nanobot {},
                        nanobot_type: *kind,
                        velocity: VelocityComponent::default(),
                        ai_state: Default::default(),
                        health: Health::default(),
                    },
                    Commitment::Idle,
                    Transform::from_translation(world_pos.extend(0.0)),
                ));
            }
        }
    });
    swarm
}

fn spawn_opponent_with_nanobots(
    app: &mut App,
    world_pos: Vec2,
    ratio: ProductionRatio,
    counts: &[(NanobotType, u32)],
) -> Entity {
    let swarm = app
        .world_mut()
        .spawn((
            Swarm {},
            OpponentSwarm {},
            SwarmProduction::new(ratio),
            Transform::from_translation(world_pos.extend(0.0)),
        ))
        .id();
    app.world_mut().entity_mut(swarm).with_children(|p| {
        for (kind, n) in counts {
            for _ in 0..*n {
                p.spawn((
                    NanobotBundle {
                        nanobot: Nanobot {},
                        nanobot_type: *kind,
                        velocity: VelocityComponent::default(),
                        ai_state: Default::default(),
                        health: Health::default(),
                    },
                    Commitment::Idle,
                    Transform::from_translation(world_pos.extend(0.0)),
                ));
            }
        }
    });
    swarm
}

fn spawn_stockpile(app: &mut App, world_pos: Vec2, amount: u32, capacity: u32) -> Entity {
    app.world_mut()
        .spawn((
            Stockpile {
                kind: ResourceKind::Minerals,
                amount,
                capacity,
                radius: 32.0,
            },
            Transform::from_translation(world_pos.extend(0.0)),
        ))
        .id()
}

fn spawn_facility(app: &mut App, owner: Entity, pos: Vec2) -> Entity {
    app.world_mut()
        .spawn((
            ProductionFacility::new(),
            OwnerSwarm(owner),
            Transform::from_translation(pos.extend(0.0)),
        ))
        .id()
}

#[test]
fn player_swarm_with_working_facility_is_not_collapsed() {
    // The player swarm has 1 facility that is currently
    // producing, plus a Worker + Hauler. The collapse
    // system must report no collapse.
    let mut app = build_app();
    {
        let mut ratio = app.world_mut().resource_mut::<ProductionRatio>();
        ratio.set_target(NanobotType::Worker, 5);
        ratio.set_target(NanobotType::Hauler, 2);
    }
    let player_pos = Vec2::new(0.0, 0.0);
    let player = spawn_swarm_with_nanobots(
        &mut app,
        player_pos,
        &[(NanobotType::Worker, 2), (NanobotType::Hauler, 1)],
    );
    let _pile = spawn_stockpile(&mut app, player_pos, PRODUCTION_COST_PER_BOT * 5, 1000);
    let facility = spawn_facility(&mut app, player, player_pos);

    app.update();

    let state = app.world().resource::<ProductionCollapseState>();
    assert!(
        !state.player_collapsed,
        "a working facility means the player swarm is not collapsed"
    );
    assert!(!state.opponent_collapsed);
    // The facility should have picked a target by now.
    let f = app
        .world()
        .entity(facility)
        .get::<ProductionFacility>()
        .unwrap();
    assert!(
        f.is_busy(),
        "facility should have started a production cycle"
    );
}

#[test]
fn player_swarm_with_no_facility_and_recoverable_crew_is_not_collapsed() {
    // The player swarm lost every facility but still has
    // 1 Worker and 1 Hauler. The collapse system must
    // report "can recover, not collapsed".
    let mut app = build_app();
    {
        let mut ratio = app.world_mut().resource_mut::<ProductionRatio>();
        ratio.set_target(NanobotType::Worker, 5);
        ratio.set_target(NanobotType::Hauler, 2);
    }
    let player_pos = Vec2::new(0.0, 0.0);
    let _player = spawn_swarm_with_nanobots(
        &mut app,
        player_pos,
        &[(NanobotType::Worker, 1), (NanobotType::Hauler, 1)],
    );

    app.update();

    let state = app.world().resource::<ProductionCollapseState>();
    assert!(
        !state.player_collapsed,
        "a recoverable crew means the player swarm is not collapsed"
    );
    assert!(!state.opponent_collapsed);
    assert!(!state.player_won());
}

#[test]
fn player_swarm_with_no_facility_and_no_haulers_is_collapsed() {
    // The player swarm lost its facility and has no
    // haulers. A lone worker cannot deliver minerals to a
    // stockpile, so the production chain is dead. The
    // collapse system must report a player loss.
    let mut app = build_app();
    {
        let mut ratio = app.world_mut().resource_mut::<ProductionRatio>();
        ratio.set_target(NanobotType::Worker, 5);
        ratio.set_target(NanobotType::Hauler, 2);
    }
    let player_pos = Vec2::new(0.0, 0.0);
    let _player = spawn_swarm_with_nanobots(
        &mut app,
        player_pos,
        &[(NanobotType::Worker, 2), (NanobotType::Defender, 1)],
    );

    app.update();

    let state = app.world().resource::<ProductionCollapseState>();
    assert!(
        state.player_collapsed,
        "no facility + no haulers means the player swarm is collapsed"
    );
    assert!(!state.opponent_collapsed);
    assert!(state.player_lost());
    assert!(!state.player_won());
}

#[test]
fn player_swarm_with_no_facility_and_no_workers_is_collapsed() {
    // Mirror of the previous test: only haulers remain.
    // They cannot extract from deposits, so the production
    // chain is dead.
    let mut app = build_app();
    {
        let mut ratio = app.world_mut().resource_mut::<ProductionRatio>();
        ratio.set_target(NanobotType::Worker, 5);
        ratio.set_target(NanobotType::Hauler, 2);
    }
    let player_pos = Vec2::new(0.0, 0.0);
    let _player = spawn_swarm_with_nanobots(
        &mut app,
        player_pos,
        &[(NanobotType::Hauler, 2), (NanobotType::Defender, 1)],
    );

    app.update();

    let state = app.world().resource::<ProductionCollapseState>();
    assert!(state.player_collapsed);
    assert!(state.player_lost());
}

#[test]
fn opponent_swarm_with_no_facility_and_no_haulers_means_player_wins() {
    // The opponent swarm is the one that lost its
    // production capacity; the player swarm is healthy.
    // The collapse system must report a player win.
    let mut app = build_app();
    {
        let mut ratio = app.world_mut().resource_mut::<ProductionRatio>();
        ratio.set_target(NanobotType::Worker, 5);
        ratio.set_target(NanobotType::Hauler, 2);
    }
    let player_pos = Vec2::new(0.0, 0.0);
    let _player = spawn_swarm_with_nanobots(
        &mut app,
        player_pos,
        &[(NanobotType::Worker, 2), (NanobotType::Hauler, 1)],
    );

    let opponent_pos = Vec2::new(2000.0, 0.0);
    let mut opponent_ratio = ProductionRatio::new();
    opponent_ratio.set_target(NanobotType::Worker, 5);
    opponent_ratio.set_target(NanobotType::Hauler, 2);
    let _opponent = spawn_opponent_with_nanobots(
        &mut app,
        opponent_pos,
        opponent_ratio,
        &[(NanobotType::Worker, 2), (NanobotType::Defender, 1)],
    );

    app.update();

    let state = app.world().resource::<ProductionCollapseState>();
    assert!(
        state.opponent_collapsed,
        "opponent with no facility + no haulers must be collapsed"
    );
    assert!(!state.player_collapsed);
    assert!(
        state.player_won(),
        "player wins when only the opponent collapses"
    );
    assert!(!state.player_lost());
}

#[test]
fn both_swarms_collapsed_is_a_loss_not_a_win() {
    // Degenerate scenario: both sides lose their
    // production capacity. The player_lost flag takes
    // priority over player_won so the UI shows the loss
    // state.
    let mut app = build_app();
    {
        let mut ratio = app.world_mut().resource_mut::<ProductionRatio>();
        ratio.set_target(NanobotType::Worker, 5);
    }
    let player_pos = Vec2::new(0.0, 0.0);
    let _player = spawn_swarm_with_nanobots(&mut app, player_pos, &[(NanobotType::Defender, 1)]);

    let opponent_pos = Vec2::new(2000.0, 0.0);
    let mut opponent_ratio = ProductionRatio::new();
    opponent_ratio.set_target(NanobotType::Worker, 5);
    let _opponent = spawn_opponent_with_nanobots(
        &mut app,
        opponent_pos,
        opponent_ratio,
        &[(NanobotType::Defender, 1)],
    );

    app.update();

    let state = app.world().resource::<ProductionCollapseState>();
    assert!(state.player_collapsed);
    assert!(state.opponent_collapsed);
    assert!(state.player_lost());
    assert!(!state.player_won(), "mutual collapse is a loss, not a win");
}

#[test]
fn swarm_at_production_target_is_not_collapsed_without_a_facility() {
    // A swarm that has reached its production ratio target
    // has no unmet demand. "No facility" is the success
    // state, not the collapse state.
    let mut app = build_app();
    {
        let mut ratio = app.world_mut().resource_mut::<ProductionRatio>();
        ratio.set_target(NanobotType::Worker, 2);
    }
    let player_pos = Vec2::new(0.0, 0.0);
    let _player = spawn_swarm_with_nanobots(&mut app, player_pos, &[(NanobotType::Worker, 2)]);

    app.update();

    let state = app.world().resource::<ProductionCollapseState>();
    assert!(
        !state.player_collapsed,
        "a swarm at target is at rest, not collapsed"
    );
}

#[test]
fn collapse_state_updates_after_facility_is_destroyed() {
    // Dynamic scenario: the player starts healthy with a
    // working facility, then the facility is despawned.
    // After the next tick, the system must report a
    // collapse (assuming the swarm cannot recover on its
    // own -- here the swarm has no crew at all).
    let mut app = build_app();
    {
        let mut ratio = app.world_mut().resource_mut::<ProductionRatio>();
        ratio.set_target(NanobotType::Worker, 5);
        ratio.set_target(NanobotType::Hauler, 2);
    }
    let player_pos = Vec2::new(0.0, 0.0);
    // No nanobots; the swarm is essentially empty.
    let player = app
        .world_mut()
        .spawn((
            Swarm {},
            Transform::from_translation(player_pos.extend(0.0)),
        ))
        .id();
    let facility = spawn_facility(&mut app, player, player_pos);
    let _pile = spawn_stockpile(&mut app, player_pos, PRODUCTION_COST_PER_BOT * 5, 1000);

    // Tick once to let the production system start the
    // facility.
    app.update();
    {
        let f = app
            .world()
            .entity(facility)
            .get::<ProductionFacility>()
            .unwrap();
        assert!(f.is_busy(), "facility must be busy after one tick");
    }
    let state = app.world().resource::<ProductionCollapseState>();
    assert!(!state.player_collapsed, "facility is busy, no collapse yet");

    // Destroy the facility. Production still has unmet
    // demand but the swarm has no nanobots to recover.
    app.world_mut().despawn(facility);
    app.update();

    let state = app.world().resource::<ProductionCollapseState>();
    assert!(
        state.player_collapsed,
        "no facility + no nanobots means a player collapse"
    );
    assert!(state.player_lost());
}

#[test]
fn opponent_with_recoverable_crew_does_not_trigger_player_win() {
    // Opponent lost its facility but still has a Worker
    // and a Hauler. The opponent is not collapsed, so the
    // player has not won yet.
    let mut app = build_app();
    {
        let mut ratio = app.world_mut().resource_mut::<ProductionRatio>();
        ratio.set_target(NanobotType::Worker, 5);
        ratio.set_target(NanobotType::Hauler, 2);
    }
    let player_pos = Vec2::new(0.0, 0.0);
    let _player = spawn_swarm_with_nanobots(
        &mut app,
        player_pos,
        &[(NanobotType::Worker, 2), (NanobotType::Hauler, 1)],
    );

    let opponent_pos = Vec2::new(2000.0, 0.0);
    let mut opponent_ratio = ProductionRatio::new();
    opponent_ratio.set_target(NanobotType::Worker, 5);
    opponent_ratio.set_target(NanobotType::Hauler, 2);
    let _opponent = spawn_opponent_with_nanobots(
        &mut app,
        opponent_pos,
        opponent_ratio,
        &[(NanobotType::Worker, 1), (NanobotType::Hauler, 1)],
    );

    app.update();

    let state = app.world().resource::<ProductionCollapseState>();
    assert!(!state.opponent_collapsed);
    assert!(!state.player_won());
    assert!(!state.player_lost());
}

#[test]
fn idle_facility_with_no_stockpile_is_not_working_for_collapse_check() {
    // A facility exists but cannot start a production
    // cycle (no stockpile, no material). It is idle and
    // must not count as "working production". The swarm
    // has only Defenders, so the collapse system must
    // report a player loss.
    let mut app = build_app();
    {
        let mut ratio = app.world_mut().resource_mut::<ProductionRatio>();
        ratio.set_target(NanobotType::Worker, 5);
        ratio.set_target(NanobotType::Hauler, 2);
    }
    let player_pos = Vec2::new(0.0, 0.0);
    let player = spawn_swarm_with_nanobots(&mut app, player_pos, &[(NanobotType::Defender, 2)]);
    let _facility = spawn_facility(&mut app, player, player_pos);
    // No stockpile; facility stays idle.

    app.update();

    let state = app.world().resource::<ProductionCollapseState>();
    assert!(
        state.player_collapsed,
        "idle facility + no recover crew must register as a player collapse"
    );
}
