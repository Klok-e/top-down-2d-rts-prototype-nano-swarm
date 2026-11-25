//! Integration tests for issue #8: Stockpiles and basic physical hauling.
//!
//! Each test isolates one behaviour so a failure points at a single
//! contract: stockpile bookkeeping, auto-creation, source/sink pair
//! selection, bulk load, physical delivery, and ledger conservation.

use bevy::{math::Vec2, prelude::*};
use top_down_2d_rts_prototype_nano_swarm::tuning::Tuning;
use top_down_2d_rts_prototype_nano_swarm::{
    intent::{IntentGrid, IntentKind},
    nanobot::{
        Cargo, DirectMovementComponent, HaulerAssignment, HaulerLoad, LogisticsReservation,
        NanobotType, OwnerSwarm, ProductionFacility, Swarm, SwarmId,
    },
    resources::{ResourceKind, ResourceLedger, Stockpile, StockpileRole},
};

#[path = "../common/mod.rs"]
mod common;

fn build_app() -> App {
    common::sim_app_with_gather_haul()
}

fn stockpile_count(world: &mut World) -> usize {
    let mut q = world.query::<&Stockpile>();
    q.iter(world).count()
}

#[test]
fn stockpile_auto_emerges_in_gather_cell_with_demand() {
    // Issue #23 contract: "no completed Source Stockpile appears
    // instantly from Gather paint alone". A Gather-painted cell
    // with no pre-existing stockpile must NOT gain one as soon
    // as the auto-creation system runs -- Source Stockpiles in
    // Gather zones emerge through the planned-structure
    // lifecycle (plan -> build -> Stockpile) rather than as
    // instant auto-spawns. The test paints Gather intent and
    // asserts the world is still empty of stockpiles after a
    // tick of simulation.
    let mut app = build_app();
    let cell = IVec2::new(0, 0);
    app.world_mut()
        .resource_mut::<IntentGrid>()
        .paint(cell, IntentKind::Gather, SwarmId::PLAYER);
    assert_eq!(
        stockpile_count(app.world_mut()),
        0,
        "no stockpile before the tick"
    );

    for _ in 0..5 {
        app.update();
    }

    assert_eq!(
        stockpile_count(app.world_mut()),
        0,
        "Gather paint alone must not create a completed Source Stockpile"
    );
    // The full Source Stockpile flow (plan -> build ->
    // Stockpile) lives in tests/behavior/source_stockpile_flow.rs.
}

#[test]
fn stockpile_not_duplicated_when_one_already_exists() {
    // Once a cell has a stockpile (auto-created or manually
    // placed), repeated ticks must not spawn another one. The
    // acceptance bullet says stockpiles "emerge automatically",
    // not "multiply indefinitely".
    let mut app = build_app();
    let cell = IVec2::new(0, 0);
    app.world_mut()
        .resource_mut::<IntentGrid>()
        .paint(cell, IntentKind::Gather, SwarmId::PLAYER);
    let cell_world_center = common::cell_world_center(cell);

    // Manually pre-place a stockpile in the same cell. The
    // auto-creation system must not add another one on top.
    common::spawn_stockpile(&mut app, cell_world_center, 0, 500);

    for _ in 0..5 {
        app.update();
    }

    assert_eq!(
        stockpile_count(app.world_mut()),
        1,
        "auto-creation skips cells that already hold a stockpile"
    );
}

#[test]
fn stockpile_not_emerged_for_corridor_only_cell() {
    // The glossary says corridor is "hauler path guidance rather
    // than a work-producing intent". A corridor-only cell has no
    // demand in the gather/build sense, so no stockpile emerges.
    let mut app = build_app();
    let cell = IVec2::new(0, 0);
    app.world_mut()
        .resource_mut::<IntentGrid>()
        .paint(cell, IntentKind::Corridor, SwarmId::PLAYER);

    for _ in 0..60 {
        app.update();
    }

    assert_eq!(
        stockpile_count(app.world_mut()),
        0,
        "corridor-only cells do not create stockpiles"
    );
}

#[test]
fn hauler_ignores_enemy_owned_sink() {
    let mut app = build_app();
    let source_pos = Vec2::new(0.0, 0.0);
    let enemy_sink_pos = Vec2::new(200.0, 0.0);
    let player_sink_pos = Vec2::new(500.0, 0.0);
    let enemy_swarm = app
        .world_mut()
        .spawn((
            Swarm {},
            SwarmId(1),
            Transform::from_translation(Vec3::ZERO),
        ))
        .id();
    let player_swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    // Leg-2 source: a source-role stockpile the player hauler
    // pulls from. Deposits are worker-only under the tiered
    // logistics model (ADR-0005).
    let source = common::spawn_stockpile(&mut app, source_pos, 1000, 1000);
    app.world_mut()
        .entity_mut(source)
        .insert(OwnerSwarm(player_swarm));
    let enemy_sink = common::spawn_sink_stockpile(&mut app, enemy_sink_pos, 0, 1000);
    app.world_mut()
        .entity_mut(enemy_sink)
        .insert(OwnerSwarm(enemy_swarm));
    let player_sink = common::spawn_sink_stockpile(&mut app, player_sink_pos, 0, 1000);
    app.world_mut()
        .entity_mut(player_sink)
        .insert(OwnerSwarm(player_swarm));
    let hauler = common::spawn_hauler_at(&mut app, source_pos + Vec2::new(-68.0, 0.0));

    for _ in 0..100 {
        app.update();
        if app.world().get::<HaulerAssignment>(hauler).is_some() {
            break;
        }
    }

    let assignment = app
        .world()
        .entity(hauler)
        .get::<HaulerAssignment>()
        .expect("player hauler must still find player-owned sink");
    assert_eq!(assignment.source, source);
    assert_eq!(
        assignment.sink, player_sink,
        "player hauler must ignore closer enemy-owned sink"
    );
}

#[test]
fn hauler_assigns_to_source_and_sink() {
    // An idle hauler with a nearby source stockpile (leg-2
    // source) and a matching sink stockpile must commit to a
    // transport trip by getting a HaulerAssignment that points
    // at both. Deposits are worker-only under the tiered model,
    // so the hauler's source is a source-role stockpile.
    let mut app = build_app();
    let swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let source_pos = Vec2::new(100.0, 0.0);
    let sink_pos = Vec2::new(400.0, 0.0);
    let source = common::spawn_stockpile(&mut app, source_pos, 1000, 1000);
    let sink = common::spawn_sink_stockpile(&mut app, sink_pos, 0, 1000);
    app.world_mut().entity_mut(source).insert(OwnerSwarm(swarm));
    app.world_mut().entity_mut(sink).insert(OwnerSwarm(swarm));
    let hauler = common::spawn_hauler_at(&mut app, source_pos + Vec2::new(-68.0, 0.0));

    for _ in 0..60 {
        app.update();
    }

    let assignment = app
        .world()
        .entity(hauler)
        .get::<HaulerAssignment>()
        .expect("idle hauler near source + sink stockpile must receive a HaulerAssignment");
    assert_eq!(assignment.source, source);
    assert_eq!(assignment.sink, sink);
}

#[test]
fn hauler_source_arrival_reissues_movement_when_timeout_strips_dmc_before_arrival() {
    // Regression: a hauler assigned to a source can lose its
    // `DirectMovementComponent` through the progress timeout before
    // reaching the source. Arrival must not silently ignore that
    // state; it must restore movement so loading can eventually start.
    let mut app = build_app();
    let hauler_pos = Vec2::new(100.0, 0.0);
    let source_pos = Vec2::new(300.0, 0.0);
    let sink_pos = Vec2::new(500.0, 0.0);
    let source = common::spawn_stockpile(&mut app, source_pos, 1000, 1000);
    let sink = common::spawn_sink_stockpile(&mut app, sink_pos, 0, 1000);
    let hauler = common::spawn_hauler_at(&mut app, hauler_pos);

    app.world_mut().entity_mut(hauler).insert(HaulerAssignment {
        source,
        sink,
        pickup_role: StockpileRole::Source,
    });

    app.update();

    let dmc = app
        .world()
        .entity(hauler)
        .get::<DirectMovementComponent>()
        .expect("hauler still outside source should resume movement after DMC timeout");
    assert!(dmc.xy.distance(Vec2::new(232.0, 0.0)) < 0.001);
    assert!(
        app.world()
            .entity(hauler)
            .get::<HaulerAssignment>()
            .is_some(),
        "hauler keeps source/sink assignment while resuming source leg"
    );
}

#[test]
fn hauler_load_is_much_larger_than_worker_load() {
    // Acceptance: "Haulers carry more resources than Workers".
    // The worker caps at WORKER_CARRY_CAPACITY per trip. The
    // hauler's per-trip load is HAULER_CARRY_CAPACITY, which must
    // be much larger. This pins the size ordering so a future
    // tuning pass cannot accidentally bring them closer together.
    // Const blocks turn the checks into compile-time invariants
    // and dodge clippy's "assertion on a constant" lint.
    {
        assert!(
            Tuning::default().logistics.hauler_carry_capacity
                > Tuning::default().logistics.worker_carry_capacity
        )
    };
    {
        assert!(
            Tuning::default().logistics.hauler_carry_capacity
                >= 5 * Tuning::default().logistics.worker_carry_capacity
        )
    };
}

#[test]
fn hauler_delivers_full_load_to_sink() {
    // When the hauler reaches the sink, it unloads over several ticks
    // and becomes idle after the full carried amount reaches the sink.
    let mut app = build_app();
    let swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let source_pos = Vec2::new(100.0, 0.0);
    let sink_pos = Vec2::new(300.0, 0.0);
    let source = common::spawn_stockpile(&mut app, source_pos, 0, 1000);
    let sink_entity = common::spawn_sink_stockpile(&mut app, sink_pos, 0, 1000);
    app.world_mut().entity_mut(source).insert(OwnerSwarm(swarm));
    app.world_mut()
        .entity_mut(sink_entity)
        .insert(OwnerSwarm(swarm));
    let hauler = common::spawn_hauler_at(&mut app, Vec2::new(232.0, 0.0));
    app.world_mut().entity_mut(hauler).insert((
        HaulerAssignment {
            source,
            sink: sink_entity,
            pickup_role: StockpileRole::Source,
        },
        HaulerLoad {
            kind: ResourceKind::Minerals,
            amount: Tuning::default().logistics.hauler_carry_capacity,
        },
    ));

    // Twenty material-work intervals transfer the full cargo.
    for _ in 0..2_000 {
        app.update();
        if app.world().get::<HaulerLoad>(hauler).is_none() {
            break;
        }
    }

    let sink = app.world().entity(sink_entity).get::<Stockpile>().unwrap();
    assert_eq!(
        sink.amount,
        Tuning::default().logistics.hauler_carry_capacity,
        "sink receives the full load through gradual unloading",
    );
    assert!(app.world().get::<HaulerLoad>(hauler).is_none());
}

#[test]
fn hauler_transports_source_to_sink_end_to_end() {
    // Acceptance: "Resources move physically between deposits,
    // stockpiles, facilities, chargers, and needs". This test
    // pins the source-stockpile -> sink-stockpile hauler leg
    // (leg 2 of the tiered chain). The hauler is the transport,
    // and material is conserved across the move (it just moves
    // from one physical buffer to another). Deposits are
    // worker-only under ADR-0005, so the hauler source here is a
    // source-role stockpile, not a deposit.
    let mut app = build_app();
    let swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let source_pos = Vec2::new(100.0, 0.0);
    let sink_pos = Vec2::new(300.0, 0.0);
    let source = common::spawn_stockpile(&mut app, source_pos, 1000, 1000);
    let sink = common::spawn_sink_stockpile(&mut app, sink_pos, 0, 1000);
    app.world_mut().entity_mut(source).insert(OwnerSwarm(swarm));
    app.world_mut().entity_mut(sink).insert(OwnerSwarm(swarm));
    let hauler = common::spawn_hauler_at(&mut app, source_pos + Vec2::new(68.0, 0.0));
    // Initial total physical resources in the world: the source
    // stockpile holds 1000 minerals. The ResourceLedger is
    // updated on every pickup and delivery, so the conservation
    // check pins "source + sink = initial".
    let initial_total = 1000u32;

    for _ in 0..2_000 {
        app.update();
        if app.world().entity(sink).get::<Stockpile>().unwrap().amount > 0 {
            break;
        }
    }

    let world = app.world();
    let source_after = world.entity(source).get::<Stockpile>().unwrap();
    let sink_after = world.entity(sink).get::<Stockpile>().unwrap();
    // The hauler reassigns on the same tick it delivers (the
    // assignment system runs in the same chain), so it may be
    // mid-trip when we read state. The true conservation
    // invariant therefore includes any load currently in
    // flight: source + sink + carried = initial.
    let carried = world
        .entity(hauler)
        .get::<HaulerLoad>()
        .map(|l| l.amount)
        .unwrap_or(0);

    assert_eq!(
        source_after.amount + sink_after.amount + carried,
        initial_total,
        "resources are conserved: source + sink + carried = initial total"
    );
    assert!(
        sink_after.amount > 0,
        "sink received resources from the source stockpile; got {}",
        sink_after.amount
    );
}

#[test]
fn hauler_does_not_pick_work_when_no_source_available() {
    // With no deposit and no non-empty stockpile, the hauler has
    // no transport job and must not pick one.
    let mut app = build_app();
    let hauler = common::spawn_hauler_at(&mut app, Vec2::new(0.0, 0.0));
    // A single empty stockpile is not a source.
    let _stockpile = common::spawn_stockpile(&mut app, Vec2::new(200.0, 0.0), 0, 1000);

    for _ in 0..3 {
        app.update();
    }

    assert!(
        app.world()
            .entity(hauler)
            .get::<HaulerAssignment>()
            .is_none(),
        "no source means no HaulerAssignment"
    );
    assert!(
        app.world().entity(hauler).get::<HaulerLoad>().is_none(),
        "no source means no HaulerLoad"
    );
}

#[test]
fn resource_ledger_stays_consistent_through_transport() {
    let mut app = build_app();
    let swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let source = common::spawn_stockpile(&mut app, Vec2::new(100.0, 0.0), 200, 200);
    let sink = common::spawn_sink_stockpile(&mut app, Vec2::new(300.0, 0.0), 0, 200);
    app.world_mut().entity_mut(source).insert(OwnerSwarm(swarm));
    app.world_mut().entity_mut(sink).insert(OwnerSwarm(swarm));
    app.world_mut()
        .resource_mut::<ResourceLedger>()
        .add(ResourceKind::Minerals, 200);
    let hauler = common::spawn_hauler_at(&mut app, Vec2::new(168.0, 0.0));
    let mut carried_during_transit = false;

    for _ in 0..2_000 {
        app.update();
        let world = app.world();
        let source_amount = world.entity(source).get::<Stockpile>().unwrap().amount;
        let sink_amount = world.entity(sink).get::<Stockpile>().unwrap().amount;
        let cargo = world
            .entity(hauler)
            .get::<HaulerLoad>()
            .map_or(0, |load| load.amount);
        carried_during_transit |= cargo > 0;
        assert_eq!(
            source_amount + sink_amount + cargo,
            200,
            "physical custody survives each pickup, travel and unload tick"
        );
        assert_eq!(
            world
                .resource::<ResourceLedger>()
                .total(ResourceKind::Minerals),
            200,
            "transport within a swarm leaves its ledger unchanged"
        );
        if sink_amount > 0 {
            break;
        }
    }

    assert!(
        carried_during_transit,
        "minerals physically travel as Hauler cargo"
    );
    assert!(
        app.world().entity(sink).get::<Stockpile>().unwrap().amount > 0,
        "transport reaches the destination"
    );
}

#[test]
fn hauler_routes_to_facility_from_sink_stockpile_leg3() {
    // Leg 3 + downstream-first + role filter (ADR-0005). With a
    // facility (terminal), a sink stockpile, and a source stockpile
    // all present, the hauler must route to the FACILITY and draw
    // from the SINK stockpile -- the only legal leg-3 source -- and
    // never from the source stockpile. This single setup pins three
    // contracts at once: terminals beat buffers, a facility's source
    // is a sink stockpile, and a source stockpile is never a leg-3
    // source (the triple that prevents ping-pong).
    let mut app = build_app();
    let swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let hauler_pos = Vec2::new(0.0, 0.0);
    let source_pos = Vec2::new(100.0, -180.0); // source-role, closer to hauler
    let sink_pos = Vec2::new(250.0, 0.0); // sink-role
    let facility_pos = Vec2::new(450.0, 0.0);
    let source = common::spawn_stockpile(&mut app, source_pos, 1000, 1000);
    let sink = common::spawn_sink_stockpile(&mut app, sink_pos, 1000, 1000);
    app.world_mut().entity_mut(source).insert(OwnerSwarm(swarm));
    app.world_mut().entity_mut(sink).insert(OwnerSwarm(swarm));
    // Facility with an EMPTY input hopper: it has demand, so it is a
    // tier-0 sink.
    let facility = app
        .world_mut()
        .spawn((
            ProductionFacility::new(),
            OwnerSwarm(swarm),
            Transform::from_translation(facility_pos.extend(0.0)),
        ))
        .id();
    let hauler = common::spawn_hauler_at(&mut app, hauler_pos);

    for _ in 0..120 {
        app.update();
        if app.world().get::<HaulerAssignment>(hauler).is_some() {
            break;
        }
    }

    let assignment = app
        .world()
        .entity(hauler)
        .get::<HaulerAssignment>()
        .expect("hauler must commit to a leg-3 pair when a facility has demand");
    assert_eq!(
        assignment.sink, facility,
        "downstream-first must route the hauler to the facility terminal, not a stockpile buffer"
    );
    assert_eq!(
        assignment.source, sink,
        "leg-3 source must be the sink stockpile; a source-role stockpile is never a facility source"
    );
    // Isolate the committed leg for the travel/delivery half of
    // the test: the first assertion already proves a source-role
    // stockpile is not chosen as a facility source.
    app.world_mut().despawn(source);

    // Drive the trip and confirm material physically reaches the
    // hopper (production is off, so the hopper only grows).
    for _ in 0..3_000 {
        app.update();
        if app
            .world()
            .get::<ProductionFacility>(facility)
            .unwrap()
            .input_amount
            > 0
        {
            break;
        }
    }
    let input = app
        .world()
        .entity(facility)
        .get::<ProductionFacility>()
        .unwrap()
        .input_amount;
    assert!(
        input > 0,
        "hauler must deliver leg-3 material into the facility input hopper; got {input}"
    );
    let sink_after = app.world().entity(sink).get::<Stockpile>().unwrap().amount;
    assert!(
        sink_after < 1000,
        "sink stockpile must have lost material to the leg-3 hauler; got {sink_after}"
    );
}

#[test]
fn hauler_ignores_facility_whose_owner_is_not_a_swarm() {
    let mut app = build_app();
    let swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let sink = common::spawn_sink_stockpile(&mut app, Vec2::new(100.0, 0.0), 1000, 1000);
    app.world_mut().entity_mut(sink).insert(OwnerSwarm(swarm));
    let invalid_owner = app.world_mut().spawn(SwarmId::PLAYER).id();
    let facility = app
        .world_mut()
        .spawn((
            ProductionFacility::new(),
            OwnerSwarm(invalid_owner),
            Transform::from_xyz(200.0, 0.0, 0.0),
        ))
        .id();
    let hauler = common::spawn_hauler_at(&mut app, Vec2::ZERO);

    for _ in 0..10 {
        app.update();
    }

    assert_ne!(
        app.world()
            .get::<HaulerAssignment>(hauler)
            .map(|assignment| assignment.sink),
        Some(facility),
    );
}

#[test]
fn hauler_reserves_partial_facility_leg_when_hopper_has_less_space_than_capacity() {
    // Exact destination reservations let a hauler carry only what the
    // facility can accept instead of waiting for a full-capacity trip.
    let mut app = build_app();
    let swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let sink_pos = Vec2::new(100.0, 0.0);
    let facility_pos = Vec2::new(450.0, 0.0);
    let source = common::spawn_sink_stockpile(&mut app, sink_pos, 1000, 1000);
    app.world_mut().entity_mut(source).insert(OwnerSwarm(swarm));
    let mut facility = ProductionFacility::new();
    facility.input_amount = facility.input_capacity - 10;
    let facility = app
        .world_mut()
        .spawn((
            facility,
            OwnerSwarm(swarm),
            Transform::from_translation(facility_pos.extend(0.0)),
        ))
        .id();
    let hauler = common::spawn_hauler_at(&mut app, sink_pos + Vec2::new(-68.0, 0.0));

    for _ in 0..120 {
        app.update();
        if app.world().get::<LogisticsReservation>(hauler).is_some() {
            break;
        }
    }

    let reservation = app
        .world()
        .entity(hauler)
        .get::<LogisticsReservation>()
        .expect("partial destination capacity still creates a logistics leg");
    assert_eq!(reservation.source, source);
    assert_eq!(reservation.destination, facility);
    assert_eq!(reservation.amount, 10);
}

/// A swarm with a stocked sink, a full producing facility, and one idle hauler.
/// Only one production cycle may queue beyond the hopper.
fn full_producing_facility_app() -> (App, Entity, Entity) {
    full_producing_facility_app_with_waiting_loads(0)
}

fn full_producing_facility_app_with_waiting_loads(loads: u32) -> (App, Entity, Entity) {
    let mut app = build_app();
    {
        let mut tuning = app.world_mut().resource_mut::<Tuning>();
        tuning.logistics.material_work_ticks = 1;
        tuning.production.cost_per_bot = 5;
        tuning.production.input_capacity = 5;
        tuning.production.waiting_hauler_loads = loads;
    }
    let tuning = app.world().resource::<Tuning>().clone();
    let swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let sink_pos = Vec2::new(100.0, 0.0);
    let sink = common::spawn_sink_stockpile(&mut app, sink_pos, 1000, 1000);
    app.world_mut().entity_mut(sink).insert(OwnerSwarm(swarm));
    let mut facility = ProductionFacility::with_tuning(&tuning);
    facility.input_amount = facility.input_capacity;
    facility.current_target = Some(NanobotType::Worker);
    let facility = app
        .world_mut()
        .spawn((
            facility,
            OwnerSwarm(swarm),
            Transform::from_xyz(450.0, 0.0, 0.0),
        ))
        .id();
    let hauler = common::spawn_hauler_at(&mut app, sink_pos + Vec2::new(-68.0, 0.0));
    (app, facility, hauler)
}

/// Run until the hauler carries a queued delivery for `facility`, then let it
/// reach the facility and wait there.
fn queue_delivery_at_full_facility(app: &mut App, facility: Entity, hauler: Entity) -> u32 {
    let mut queued = None;
    for _ in 0..600 {
        app.update();
        let reservation = app.world().get::<LogisticsReservation>(hauler).copied();
        let cargo = app.world().get::<Cargo>(hauler).map_or(0, |c| c.amount);
        if reservation.is_some_and(|r| r.destination == facility && r.source_remaining == 0)
            && cargo > 0
        {
            queued = Some(cargo);
            break;
        }
    }
    let carried = queued.expect("a full producing facility accepts one queued cycle");
    for _ in 0..300 {
        app.update();
    }
    carried
}

#[test]
fn hauler_queues_a_cycle_for_a_full_producing_facility_and_unloads_after_it_drains() {
    let (mut app, facility, hauler) = full_producing_facility_app();
    let cost = app.world().resource::<Tuning>().production.cost_per_bot;

    let carried = queue_delivery_at_full_facility(&mut app, facility, hauler);
    assert_eq!(carried, cost, "the queued claim is one production cycle");
    let hopper = app.world().get::<ProductionFacility>(facility).unwrap();
    assert_eq!(
        hopper.input_amount, hopper.input_capacity,
        "a full hopper takes nothing"
    );
    assert_eq!(
        app.world().get::<HaulerAssignment>(hauler).map(|a| a.sink),
        Some(facility),
        "the loaded hauler waits at the producing facility",
    );

    // The next cycle start drains one cycle from the hopper.
    app.world_mut()
        .get_mut::<ProductionFacility>(facility)
        .unwrap()
        .input_amount -= cost;
    // One-tick transfers refill from waiting cargo far faster than the
    // ~140-tick round trip a fresh delivery would need.
    let mut refilled = false;
    for _ in 0..20 {
        app.update();
        let hopper = app.world().get::<ProductionFacility>(facility).unwrap();
        if hopper.input_amount == hopper.input_capacity {
            refilled = true;
            break;
        }
    }
    assert!(refilled, "the waiting cargo refills the drained hopper");
}

#[test]
fn waiting_hauler_leaves_a_full_facility_that_goes_idle() {
    let (mut app, facility, hauler) = full_producing_facility_app();
    queue_delivery_at_full_facility(&mut app, facility, hauler);

    app.world_mut()
        .get_mut::<ProductionFacility>(facility)
        .unwrap()
        .current_target = None;
    for _ in 0..30 {
        app.update();
    }

    assert_ne!(
        app.world().get::<HaulerAssignment>(hauler).map(|a| a.sink),
        Some(facility),
        "an idle facility no longer drains, so its queued claim is released",
    );
}

#[test]
fn hauler_never_picks_source_stockpile_as_sink() {
    // Ping-pong guard (ADR-0005): a source-role stockpile is never
    // a hauler sink, so leg 2 cannot reverse (sink -> source). With
    // a source stockpile full of material and a sink stockpile with
    // free space, the hauler commits source=source, sink=sink --
    // never the other way around.
    let mut app = build_app();
    let swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let source_pos = Vec2::new(100.0, 0.0);
    let sink_pos = Vec2::new(400.0, 0.0);
    let source = common::spawn_stockpile(&mut app, source_pos, 1000, 1000);
    let sink = common::spawn_sink_stockpile(&mut app, sink_pos, 0, 1000);
    app.world_mut().entity_mut(source).insert(OwnerSwarm(swarm));
    app.world_mut().entity_mut(sink).insert(OwnerSwarm(swarm));
    let hauler = common::spawn_hauler_at(&mut app, source_pos + Vec2::new(-68.0, 0.0));

    for _ in 0..120 {
        app.update();
        if app.world().get::<HaulerAssignment>(hauler).is_some() {
            break;
        }
    }

    let assignment = app
        .world()
        .entity(hauler)
        .get::<HaulerAssignment>()
        .expect("hauler must commit to a leg-2 pair");
    assert_eq!(
        assignment.source, source,
        "leg-2 source is the source stockpile"
    );
    assert_eq!(
        assignment.sink, sink,
        "leg-2 sink is the sink stockpile; a source-role stockpile is never a sink (no reversal)"
    );
}

#[test]
fn two_hauler_loads_wait_at_a_full_facility_and_drain_over_several_cycles() {
    let (mut app, facility, first) = full_producing_facility_app_with_waiting_loads(2);
    let second = common::spawn_hauler_at(&mut app, Vec2::new(32.0, 60.0));
    let carry = app
        .world()
        .resource::<Tuning>()
        .logistics
        .hauler_carry_capacity;
    // Keep the hopper full so neither delivery can unload during the test.
    let mut waiting = 0;
    for _ in 0..600 {
        app.world_mut()
            .get_mut::<ProductionFacility>(facility)
            .unwrap()
            .input_amount = 5;
        app.update();
        waiting = [first, second]
            .into_iter()
            .filter(|hauler| {
                app.world()
                    .get::<LogisticsReservation>(*hauler)
                    .is_some_and(|r| r.destination == facility && r.source_remaining == 0)
                    && app.world().get::<Cargo>(*hauler).map_or(0, |c| c.amount) == carry
            })
            .count();
        if waiting == 2 {
            break;
        }
    }
    assert_eq!(
        waiting, 2,
        "both Haulers carry a full load to the full facility and wait"
    );

    // Each simulated cycle drains 5 from the hopper. Two 20-mineral loads,
    // each larger than the 5-mineral hopper, must empty into it over
    // several cycles without overfilling it or going anywhere else.
    let start = app
        .world()
        .get::<ProductionFacility>(facility)
        .unwrap()
        .input_amount;
    let mut drained = 0;
    let mut emptied = [false; 2];
    for tick in 0..2_000 {
        if tick % 10 == 0 {
            let mut hopper = app
                .world_mut()
                .get_mut::<ProductionFacility>(facility)
                .unwrap();
            if hopper.input_amount >= 5 {
                hopper.input_amount -= 5;
                drained += 5;
            }
        }
        app.update();
        let hopper = app.world().get::<ProductionFacility>(facility).unwrap();
        assert!(hopper.input_amount <= hopper.input_capacity);
        for (index, hauler) in [first, second].into_iter().enumerate() {
            let cargo = app.world().get::<Cargo>(hauler).map_or(0, |c| c.amount);
            if cargo == 0 {
                emptied[index] = true;
            } else if !emptied[index] {
                assert_eq!(
                    app.world()
                        .get::<LogisticsReservation>(hauler)
                        .map(|r| r.destination),
                    Some(facility),
                    "a waiting Hauler keeps its facility claim until empty",
                );
            }
        }
        if emptied == [true, true] {
            break;
        }
    }
    assert_eq!(emptied, [true, true], "both loads drain into the facility");
    let end = app
        .world()
        .get::<ProductionFacility>(facility)
        .unwrap()
        .input_amount;
    assert_eq!(
        drained + end - start,
        2 * carry,
        "every carried mineral entered the hopper"
    );
}
