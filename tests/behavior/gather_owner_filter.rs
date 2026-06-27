//! Regression: a Worker carrying a load must not deliver into a
//! Source Stockpile owned by an opposing swarm. Before the
//! per-swarm ownership filter was added to the gather delivery
//! path, `find_nearest_stockpile` and
//! `has_usable_built_source_stockpile` matched on kind + free
//! space + role only, so a player Worker would happily route to
//! (and fill) an enemy-owned Source Stockpile sitting next to a
//! contested deposit.

use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    nanobot::{
        OwnerSwarm, ReturningToStockpile, Swarm, SwarmId, SwarmMember, WorkerLoad,
        WORKER_CARRY_CAPACITY,
    },
    resources::{ResourceKind, Stockpile, StockpileRole},
};

#[path = "../common/mod.rs"]
mod common;

fn build_app() -> App {
    common::sim_app_with_gather()
}

/// Spawn a `Swarm` carrying `id` and return its entity. Used as
/// the owner reference stamped onto an enemy-owned stockpile.
fn spawn_swarm_with_id(app: &mut App, id: SwarmId) -> Entity {
    app.world_mut()
        .spawn((Swarm {}, id, Transform::from_translation(Vec3::ZERO)))
        .id()
}

/// Spawn a Source Stockpile at `pos` owned by `owner_swarm`.
fn spawn_owned_source_stockpile(app: &mut App, pos: Vec2, owner_swarm: Entity) -> Entity {
    app.world_mut()
        .spawn((
            Stockpile {
                kind: ResourceKind::Minerals,
                amount: 0,
                capacity: 100,
                radius: 32.0,
            },
            StockpileRole::Source,
            OwnerSwarm(owner_swarm),
            Transform::from_translation(pos.extend(0.0)),
        ))
        .id()
}

#[test]
fn worker_carry_assign_skips_enemy_owned_stockpile() {
    let mut app = build_app();
    let worker_pos = Vec2::new(0.0, 0.0);
    let worker = common::spawn_worker_at(&mut app, worker_pos);
    // Stamp a load so the carry-assign system processes the worker.
    app.world_mut().entity_mut(worker).insert(WorkerLoad {
        kind: ResourceKind::Minerals,
        amount: WORKER_CARRY_CAPACITY,
    });

    // An enemy-owned Source Stockpile right next to the worker.
    // Without the owner filter this is the nearest (only)
    // candidate, so the worker would route to it.
    let enemy_swarm = spawn_swarm_with_id(&mut app, SwarmId(7));
    let _enemy_stockpile =
        spawn_owned_source_stockpile(&mut app, worker_pos + Vec2::new(50.0, 0.0), enemy_swarm);

    for _ in 0..5 {
        app.update();
    }

    let returning = app.world().entity(worker).get::<ReturningToStockpile>();
    assert!(
        returning.is_none(),
        "player Worker must not deliver into an enemy-owned Source Stockpile"
    );
}

#[test]
fn worker_carry_assign_prefers_own_stockpile_over_enemy() {
    // Same setup as above but with a second, allied stockpile
    // farther away. The worker must pick the allied one even
    // though the enemy one is nearer.
    let mut app = build_app();
    let worker_pos = Vec2::new(0.0, 0.0);
    let worker = common::spawn_worker_at(&mut app, worker_pos);
    app.world_mut().entity_mut(worker).insert(WorkerLoad {
        kind: ResourceKind::Minerals,
        amount: WORKER_CARRY_CAPACITY,
    });

    let enemy_swarm = spawn_swarm_with_id(&mut app, SwarmId(7));
    let _enemy_stockpile =
        spawn_owned_source_stockpile(&mut app, worker_pos + Vec2::new(40.0, 0.0), enemy_swarm);

    let player_swarm = spawn_swarm_with_id(&mut app, SwarmId::PLAYER);
    let allied_stockpile =
        spawn_owned_source_stockpile(&mut app, worker_pos + Vec2::new(120.0, 0.0), player_swarm);

    for _ in 0..5 {
        app.update();
    }

    let returning = app
        .world()
        .entity(worker)
        .get::<ReturningToStockpile>()
        .expect("worker must route to its own stockpile");
    assert_eq!(
        returning.stockpile, allied_stockpile,
        "worker must prefer the allied stockpile over the nearer enemy one"
    );
}

#[test]
fn unowned_stockpile_still_usable_by_any_worker() {
    // Backwards-compat: a bare Stockpile without an OwnerSwarm
    // marker (the legacy default used by hand-spawned fixtures)
    // stays usable by any swarm, so the ownership filter does
    // not break the pre-existing test seam contract.
    let mut app = build_app();
    let worker_pos = Vec2::new(0.0, 0.0);
    let worker = common::spawn_worker_at(&mut app, worker_pos);
    app.world_mut().entity_mut(worker).insert(WorkerLoad {
        kind: ResourceKind::Minerals,
        amount: WORKER_CARRY_CAPACITY,
    });

    let _unowned = app
        .world_mut()
        .spawn((
            Stockpile {
                kind: ResourceKind::Minerals,
                amount: 0,
                capacity: 100,
                radius: 32.0,
            },
            Transform::from_translation((worker_pos + Vec2::new(50.0, 0.0)).extend(0.0)),
        ))
        .id();

    app.update();

    let returning = app.world().entity(worker).get::<ReturningToStockpile>();
    assert!(
        returning.is_some(),
        "unowned stockpile must remain usable by any worker (legacy default)"
    );
}

#[test]
fn worker_swarm_member_is_player_by_default() {
    // Sanity: the common spawn_worker_at helper stamps the
    // player swarm on the worker, which is the precondition the
    // two filter tests above rely on.
    let mut app = build_app();
    let worker = common::spawn_worker_at(&mut app, Vec2::ZERO);
    let member = app.world().entity(worker).get::<SwarmMember>();
    assert_eq!(member.map(|m| m.0), Some(SwarmId::PLAYER));
}
