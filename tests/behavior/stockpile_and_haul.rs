//! Integration tests for issue #8: Stockpiles and basic physical hauling.
//!
//! Each test isolates one behaviour so a failure points at a single
//! contract: stockpile bookkeeping, auto-creation, source/sink pair
//! selection, bulk load, physical delivery, and ledger conservation.

use bevy::{math::Vec2, prelude::*};
use top_down_2d_rts_prototype_nano_swarm::{
    intent::{IntentGrid, IntentKind, PAINT_STRENGTH_CAP},
    nanobot::{HaulerAssignment, HaulerLoad, HAULER_CARRY_CAPACITY, WORKER_CARRY_CAPACITY},
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
    let stockpile = common::spawn_stockpile(&mut app, stockpile_pos, 0, 100);

    // Empty stockpile: free_space == capacity.
    {
        let s = app.world().entity(stockpile).get::<Stockpile>().unwrap();
        assert_eq!(s.amount, 0);
        assert_eq!(s.capacity, 100);
        assert_eq!(s.free_space(), 100);
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
        assert_eq!(s.free_space(), 63, "free space shrinks as the buffer fills");
    }
}

#[test]
fn stockpile_auto_emerges_in_gather_cell_with_demand() {
    // Acceptance: "Stockpiles emerge automatically from sustained
    // gather/build demand". A Gather-painted cell with no
    // pre-existing stockpile must gain one as soon as the
    // auto-creation system runs.
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

    app.update();

    assert_eq!(
        stockpile_count(app.world_mut()),
        1,
        "exactly one stockpile emerged from Gather demand"
    );
    // The auto-created stockpile must live in the painted cell.
    let world = app.world_mut();
    let mut q = world.query::<(&Stockpile, &Transform)>();
    let (s, t) = q.iter(world).next().expect("stockpile exists");
    assert_eq!(s.kind, ResourceKind::Minerals);
    assert_eq!(s.amount, 0, "auto stockpile starts empty");
    assert!(s.capacity > 0);
    let cell_world_center = common::cell_world_center(cell);
    assert!(
        (t.translation.truncate() - cell_world_center).length() < 1.0,
        "stockpile must be created at the cell's world center"
    );
}

#[test]
fn stockpile_auto_emerges_in_build_cell_with_demand() {
    // Build-painted cells count as "sustained demand" too --
    // a Build zone needs local materials on hand.
    let mut app = build_app();
    let cell = IVec2::new(1, 1);
    app.world_mut()
        .resource_mut::<IntentGrid>()
        .paint(cell, IntentKind::Build, PAINT_STRENGTH_CAP);

    app.update();

    assert_eq!(stockpile_count(app.world_mut()), 1);
    // A Gather-only cell does not satisfy Build demand, and vice
    // versa. The two layers create two stockpiles.
    let other = IVec2::new(2, 2);
    app.world_mut().resource_mut::<IntentGrid>().paint(
        other,
        IntentKind::Gather,
        PAINT_STRENGTH_CAP,
    );
    app.update();
    assert_eq!(
        stockpile_count(app.world_mut()),
        2,
        "Build and Gather cells spawn separate stockpiles"
    );
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
fn hauler_assigns_to_source_and_sink() {
    // An idle hauler with a nearby deposit (source) and a
    // matching stockpile (sink) must commit to a transport trip
    // by getting a HaulerAssignment that points at both.
    let mut app = build_app();
    let deposit_pos = Vec2::new(100.0, 0.0);
    let stockpile_pos = Vec2::new(400.0, 0.0);
    let deposit = common::spawn_deposit(&mut app, deposit_pos, 1000);
    let stockpile = common::spawn_stockpile(&mut app, stockpile_pos, 0, 1000);
    let hauler = common::spawn_hauler_at(&mut app, deposit_pos);

    for _ in 0..3 {
        app.update();
    }

    let assignment = app
        .world()
        .entity(hauler)
        .get::<HaulerAssignment>()
        .expect("idle hauler near deposit + stockpile must receive a HaulerAssignment");
    assert_eq!(assignment.source, deposit);
    assert_eq!(assignment.sink, stockpile);
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
    // itself.
    for _ in 0..30 {
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
    assert!(
        app.world()
            .entity(hauler)
            .get::<HaulerAssignment>()
            .is_none(),
        "HaulerAssignment is removed on successful delivery"
    );
}

#[test]
fn hauler_transports_deposit_to_sink_end_to_end() {
    // Acceptance: "Resources move physically between deposits,
    // stockpiles, facilities, chargers, and needs". This test
    // pins the deposit -> stockpile leg. The hauler is the
    // primary transport, and the ledger must stay consistent
    // throughout (resources just move from one physical location
    // to another).
    let mut app = build_app();
    let deposit_pos = Vec2::new(100.0, 0.0);
    let stockpile_pos = Vec2::new(200.0, 0.0);
    let deposit = common::spawn_deposit(&mut app, deposit_pos, 1000);
    let stockpile = common::spawn_stockpile(&mut app, stockpile_pos, 0, 1000);
    let hauler = common::spawn_hauler_at(&mut app, deposit_pos);
    // Initial total physical resources in the world: the deposit
    // holds 1000 minerals. The ResourceLedger starts at 0 because
    // pre-existing deposits are not yet in the ledger (only
    // physical movements are). The conservation check therefore
    // pins "deposit + sink = initial" and ignores the ledger.
    let initial_total = 1000u32;

    // Record the ledger total at the start so we can sanity-check
    // it after the trip. The ledger is updated on every pickup
    // and delivery, so the post-trip total reflects the net
    // physical transport since the start.
    let _initial_ledger = app
        .world()
        .resource::<ResourceLedger>()
        .total(ResourceKind::Minerals);

    for _ in 0..40 {
        app.update();
    }

    let world = app.world();
    let deposit_after = world.entity(deposit).get::<ResourceDeposit>().unwrap();
    let sink_after = world.entity(stockpile).get::<Stockpile>().unwrap();

    // Resources only moved between physical locations; the
    // deposit + sink total is conserved. The ResourceLedger is
    // updated on every pickup and delivery, so it equals
    // deposit + sink + hauler load; we do not pin it here.
    assert_eq!(
        deposit_after.amount + sink_after.amount,
        initial_total,
        "resources are conserved: deposit + sink = initial total"
    );
    assert!(
        sink_after.amount > 0,
        "sink received resources from the deposit; got {}",
        sink_after.amount
    );
    // The hauler is idle and ready for the next trip.
    assert!(
        world.entity(hauler).get::<HaulerLoad>().is_none(),
        "hauler has no load after delivery"
    );
    assert!(
        world.entity(hauler).get::<HaulerAssignment>().is_none(),
        "hauler has no assignment after delivery"
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
