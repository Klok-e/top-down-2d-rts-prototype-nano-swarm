//! Integration tests for issue #8: Stockpiles and basic physical hauling.
//!
//! Each test isolates one behaviour so a failure points at a single
//! contract: stockpile bookkeeping, auto-creation, source/sink pair
//! selection, bulk load, physical delivery, and ledger conservation.

use bevy::{math::Vec2, prelude::*};
use top_down_2d_rts_prototype_nano_swarm::{
    intent::{IntentGrid, IntentKind, PAINT_STRENGTH_CAP},
    nanobot::{
        DirectMovementComponent, HaulerAssignment, HaulerLoad, OwnerSwarm, ProductionFacility,
        Swarm, SwarmId, DEFAULT_STOCKPILE_CAPACITY, HAULER_CARRY_CAPACITY, WORKER_CARRY_CAPACITY,
    },
    resources::{ResourceDeposit, ResourceKind, ResourceLedger, Stockpile},
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
fn stockpile_can_hold_local_resource_amounts() {
    // The Stockpile component already exists from issue #7; this
    // test pins the "stockpile is a local buffer" contract for
    // issue #8: a stockpile tracks an amount, has a capacity, and
    // exposes free_space so delivery systems can reason about it.
    let mut app = build_app();
    let stockpile_pos = Vec2::new(200.0, 0.0);
    let stockpile = common::spawn_stockpile(&mut app, stockpile_pos, 0, DEFAULT_STOCKPILE_CAPACITY);

    // Empty stockpile: free_space == capacity.
    {
        let s = app.world().entity(stockpile).get::<Stockpile>().unwrap();
        assert_eq!(s.amount, 0);
        assert_eq!(s.capacity, DEFAULT_STOCKPILE_CAPACITY);
        assert_eq!(s.free_space(), DEFAULT_STOCKPILE_CAPACITY);
    }

    // Adding resources changes the buffer amount.
    app.world_mut()
        .entity_mut(stockpile)
        .get_mut::<Stockpile>()
        .unwrap()
        .amount = 37;
    {
        let s = app.world().entity(stockpile).get::<Stockpile>().unwrap();
        assert_eq!(s.amount, 37);
        assert_eq!(
            s.free_space(),
            DEFAULT_STOCKPILE_CAPACITY - 37,
            "free space shrinks as the buffer fills"
        );
    }
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
    app.world_mut().resource_mut::<IntentGrid>().paint(
        cell,
        IntentKind::Gather,
        PAINT_STRENGTH_CAP,
    );
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
    app.world_mut().resource_mut::<IntentGrid>().paint(
        cell,
        IntentKind::Gather,
        PAINT_STRENGTH_CAP,
    );
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
    app.world_mut().resource_mut::<IntentGrid>().paint(
        cell,
        IntentKind::Corridor,
        PAINT_STRENGTH_CAP,
    );

    for _ in 0..3 {
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
    let enemy_sink_pos = Vec2::new(50.0, 0.0);
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
    let hauler = common::spawn_hauler_at(&mut app, source_pos);

    for _ in 0..3 {
        app.update();
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
    let source_pos = Vec2::new(100.0, 0.0);
    let sink_pos = Vec2::new(400.0, 0.0);
    let source = common::spawn_stockpile(&mut app, source_pos, 1000, 1000);
    let sink = common::spawn_sink_stockpile(&mut app, sink_pos, 0, 1000);
    let hauler = common::spawn_hauler_at(&mut app, source_pos);

    for _ in 0..3 {
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

    app.world_mut()
        .entity_mut(hauler)
        .insert(HaulerAssignment { source, sink });

    app.update();

    let dmc = app
        .world()
        .entity(hauler)
        .get::<DirectMovementComponent>()
        .expect("hauler still outside source should resume movement after DMC timeout");
    assert_eq!(dmc.xy, source_pos);
    assert!(
        app.world()
            .entity(hauler)
            .get::<HaulerAssignment>()
            .is_some(),
        "hauler keeps source/sink assignment while resuming source leg"
    );
}

#[test]
fn hauler_fills_load_up_to_carry_capacity() {
    // The hauler's load fills up to HAULER_CARRY_CAPACITY by
    // pulling HAULER_EXTRACT_PER_TICK units per tick from the
    // deposit. The load is removed on full transition and a
    // HaulerLoad component is inserted with the full amount.
    let mut app = build_app();
    let deposit_pos = Vec2::new(100.0, 0.0);
    let stockpile_pos = Vec2::new(400.0, 0.0);
    let deposit = common::spawn_deposit(&mut app, deposit_pos, 1000);
    let _stockpile = common::spawn_stockpile(&mut app, stockpile_pos, 0, 1000);
    let hauler = common::spawn_hauler_at(&mut app, deposit_pos);

    // Pre-seed an assignment to the source so the test isolates
    // the loading chain from the source/sink selection logic.
    app.world_mut().entity_mut(hauler).insert(HaulerAssignment {
        source: deposit,
        sink: _stockpile,
    });

    // Drive enough ticks for the load to fill:
    //   1 arrive tick + 5 load ticks + 1 buffer = 7 ticks.
    for _ in 0..7 {
        app.update();
    }

    let load = app
        .world()
        .entity(hauler)
        .get::<HaulerLoad>()
        .expect("hauler should carry a HaulerLoad after filling");
    assert_eq!(
        load.amount, HAULER_CARRY_CAPACITY,
        "hauler load caps at HAULER_CARRY_CAPACITY"
    );
    let deposit_state = app
        .world()
        .entity(deposit)
        .get::<ResourceDeposit>()
        .unwrap();
    assert_eq!(
        deposit_state.amount,
        1000 - HAULER_CARRY_CAPACITY,
        "deposit lost exactly HAULER_CARRY_CAPACITY units across the load phase"
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
    const { assert!(HAULER_CARRY_CAPACITY > WORKER_CARRY_CAPACITY) };
    const { assert!(HAULER_CARRY_CAPACITY >= 5 * WORKER_CARRY_CAPACITY) };
}

#[test]
fn hauler_delivers_full_load_to_sink() {
    // When the hauler reaches the sink, the load is dropped into
    // it and the hauler becomes idle. The dropped amount matches
    // what the hauler was carrying.
    let mut app = build_app();
    let deposit_pos = Vec2::new(100.0, 0.0);
    let stockpile_pos = Vec2::new(150.0, 0.0); // very close: within radius
    let deposit = common::spawn_deposit(&mut app, deposit_pos, 1000);
    let stockpile = common::spawn_stockpile(&mut app, stockpile_pos, 0, 1000);
    let hauler = common::spawn_hauler_at(&mut app, deposit_pos);

    // Pre-seed assignment pointing at the (very close) stockpile
    // so the hauler fills its load and walks to the sink in a
    // handful of ticks.
    app.world_mut().entity_mut(hauler).insert(HaulerAssignment {
        source: deposit,
        sink: stockpile,
    });

    // Fill the load (5 ticks of extraction at the deposit) plus
    // travel + delivery. Travel from (100, 0) to (150, 0) at
    // bot_speed 5.0 = 10 ticks, plus one tick for the delivery
    // itself. Issue #38 / ADR-0004: the hauler stops at the
    // sink's physical extent (radius 32) instead of the
    // legacy `STOP_THRESHOLD` (2). Travel is 4 ticks instead
    // of 10; the test only needs to run one cycle, so 12
    // ticks is enough. The original 30-tick budget would
    // let the hauler start a second cycle and the
    // assertions would fail (HaulerAssignment is re-issued
    // by the assignment system once the hauler idles).
    for _ in 0..12 {
        app.update();
    }

    let sink = app.world().entity(stockpile).get::<Stockpile>().unwrap();
    assert!(
        sink.amount >= HAULER_CARRY_CAPACITY,
        "sink should receive the hauler's full load; got {}",
        sink.amount
    );
    assert!(
        app.world().entity(hauler).get::<HaulerLoad>().is_none(),
        "HaulerLoad is removed on successful delivery"
    );
    // Issue #38 / ADR-0004: the hauler is re-assigned on the
    // same tick the delivery clears its markers (the
    // assignment system runs in the same chain as the
    // delivery system). The HaulerAssignment assertion was
    // removed because it is timing-dependent on the
    // chain order: with the new stop_radius, the hauler
    // reaches the sink faster, so the post-delivery idle
    // window is too short to observe "no HaulerAssignment"
    // from outside the chain. The contract ("delivery
    // happens") is pinned by the sink.amount and
    // HaulerLoad assertions above.
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
    let source_pos = Vec2::new(100.0, 0.0);
    let sink_pos = Vec2::new(200.0, 0.0);
    let source = common::spawn_stockpile(&mut app, source_pos, 1000, 1000);
    let sink = common::spawn_sink_stockpile(&mut app, sink_pos, 0, 1000);
    let hauler = common::spawn_hauler_at(&mut app, source_pos);
    // Initial total physical resources in the world: the source
    // stockpile holds 1000 minerals. The ResourceLedger is
    // updated on every pickup and delivery, so the conservation
    // check pins "source + sink = initial".
    let initial_total = 1000u32;

    for _ in 0..40 {
        app.update();
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
    // The ledger total must equal the sum of physical resources
    // across deposits and stockpiles after a chain of hauler
    // trips. The ledger is updated on every pickup and delivery,
    // so it stays a real-time view of swarm resources.
    let mut app = build_app();
    let deposit_pos = Vec2::new(100.0, 0.0);
    let stockpile_pos = Vec2::new(200.0, 0.0);
    let deposit = common::spawn_deposit(&mut app, deposit_pos, 200);
    let stockpile = common::spawn_stockpile(&mut app, stockpile_pos, 0, 1000);
    let _hauler = common::spawn_hauler_at(&mut app, deposit_pos);

    // The ResourceLedger starts at 0 because pre-existing
    // deposits are not yet in the ledger (only physical
    // movements are). The first delivery is what populates it.
    {
        let initial_ledger = app
            .world()
            .resource::<ResourceLedger>()
            .total(ResourceKind::Minerals);
        assert_eq!(
            initial_ledger, 0,
            "deposit pre-load: nothing in the ledger yet"
        );
    }

    // Drive enough ticks for the hauler to load, travel to the
    // sink, deliver, and complete at least one round trip.
    //   1 arrive + 5 load + 20 travel + 1 delivery = 27 ticks.
    //   Plus a margin for the return trip's start = 50 ticks.
    for _ in 0..50 {
        app.update();
    }

    let world = app.world();
    let deposit_amount = world
        .entity(deposit)
        .get::<ResourceDeposit>()
        .unwrap()
        .amount;
    let stockpile_amount = world.entity(stockpile).get::<Stockpile>().unwrap().amount;
    let ledger = world
        .resource::<ResourceLedger>()
        .total(ResourceKind::Minerals);

    // The ledger is updated on every pickup (decrement) and
    // delivery (increment). With a starting ledger of 0 and the
    // initial deposit not yet in the ledger, the ledger equals
    // the net amount currently in the swarm -- which is the
    // amount that has been delivered to the sink so far.
    assert_eq!(
        ledger, stockpile_amount,
        "ledger tracks deliveries: ledger == sink amount"
    );
    // The deposit and sink together hold at most the initial
    // total; the hauler load is in flight and is also tracked
    // by the ledger.
    assert!(
        deposit_amount + stockpile_amount <= 200,
        "physical resources never exceed the initial total"
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
    use top_down_2d_rts_prototype_nano_swarm::nanobot::ProductionRatio;
    let mut app = build_app();
    // Empty ratio so production never fires and the hauler is the
    // only actor touching the facility's hopper.
    app.insert_resource(ProductionRatio::new());
    let hauler_pos = Vec2::new(0.0, 0.0);
    let source_pos = Vec2::new(50.0, 0.0); // source-role, closer to hauler
    let sink_pos = Vec2::new(100.0, 0.0); // sink-role
    let facility_pos = Vec2::new(140.0, 0.0);
    let source = common::spawn_stockpile(&mut app, source_pos, 1000, 1000);
    let sink = common::spawn_sink_stockpile(&mut app, sink_pos, 1000, 1000);
    // Facility with an EMPTY input hopper: it has demand, so it is a
    // tier-0 sink.
    let facility = app
        .world_mut()
        .spawn((
            ProductionFacility::new(),
            Transform::from_translation(facility_pos.extend(0.0)),
        ))
        .id();
    let hauler = common::spawn_hauler_at(&mut app, hauler_pos);

    app.update();

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
    for _ in 0..60 {
        app.update();
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
fn hauler_does_not_start_facility_leg_when_hopper_has_less_space_than_load() {
    // Regression for a later-trip stuck hauler: facility free
    // space was checked as `> 0`, so a hauler could pick a
    // sink-stockpile -> facility leg, load `HAULER_CARRY_CAPACITY`
    // minerals, arrive at a hopper with only a few free slots, then wait forever if
    // production did not drain enough space. The leg picker should
    // skip terminal legs that cannot accept the load it is about to
    // pull from the source.
    let mut app = build_app();
    let sink_pos = Vec2::new(100.0, 0.0);
    let facility_pos = Vec2::new(140.0, 0.0);
    let _sink = common::spawn_sink_stockpile(&mut app, sink_pos, 1000, 1000);
    let mut facility = ProductionFacility::new();
    facility.input_amount = facility.input_capacity - 10;
    let _facility = app
        .world_mut()
        .spawn((
            facility,
            Transform::from_translation(facility_pos.extend(0.0)),
        ))
        .id();
    let hauler = common::spawn_hauler_at(&mut app, sink_pos);

    app.update();

    assert!(
        app.world()
            .entity(hauler)
            .get::<HaulerAssignment>()
            .is_none(),
        "hauler must not start a facility leg when hopper free space cannot accept its load"
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
    let source_pos = Vec2::new(100.0, 0.0);
    let sink_pos = Vec2::new(400.0, 0.0);
    let source = common::spawn_stockpile(&mut app, source_pos, 1000, 1000);
    let sink = common::spawn_sink_stockpile(&mut app, sink_pos, 0, 1000);
    let hauler = common::spawn_hauler_at(&mut app, source_pos);

    app.update();

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
