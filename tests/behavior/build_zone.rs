//! Integration tests for issue #10: Build Zones and automatic
//! stockpile-backed construction.
//!
//! Each test isolates one behaviour so a failure points at a single
//! contract: BuildSite auto-creation, worker choice via autonomy
//! scoring, material consumption from local stockpiles, repair of
//! damaged structures, and placement near intent paint.
//!
//! The "extra workers may wait/crowd/choose other work" half of
//! the contract is covered by the soft-slot scoring tests in
//! `tests/nanobot_autonomy_behavior.rs`; the build chain just
//! consumes the same `best_candidate` output, so the contract is
//! the same here as it is for Gather and Haul.

use bevy::{math::Vec2, prelude::*};
use top_down_2d_rts_prototype_nano_swarm::{
    intent::{IntentGrid, IntentKind, PAINT_STRENGTH_CAP},
    nanobot::{
        world_to_cell, BuildAssignment, BuildProgress, BuildSite, Structure, StructureKind,
        BUILD_REQUIRED_MATERIALS, STRUCTURE_MAX_HEALTH,
    },
    resources::Stockpile,
    ZONE_BLOCK_SIZE,
};

#[path = "../common/mod.rs"]
mod common;

fn build_app() -> App {
    common::sim_app_with_build()
}

#[test]
fn build_site_auto_emerges_in_build_cell_with_demand() {
    // Acceptance: "Automatic support construction happens inside
    // or near matching intent paint." A Build-painted cell with
    // no existing structure must gain a BuildSite as soon as the
    // auto-creation system runs. The site lives in the cell so
    // the test pins the "inside" half of the placement contract.
    let mut app = build_app();
    let cell = IVec2::new(0, 0);
    app.world_mut()
        .resource_mut::<IntentGrid>()
        .paint(cell, IntentKind::Build, PAINT_STRENGTH_CAP);

    app.update();

    let world = app.world_mut();
    let mut q = world.query::<(&BuildSite, &Transform)>();
    let (site, transform) = q
        .iter(world)
        .next()
        .expect("BuildSite must spawn in a Build-painted cell");
    let center = common::cell_world_center(cell);
    assert!(
        (transform.translation.truncate() - center).length() < 1.0,
        "BuildSite must be created at the cell's world center; got {:?}",
        transform.translation
    );
    assert_eq!(site.cell, cell);
    assert_eq!(site.required_materials, BUILD_REQUIRED_MATERIALS);
    assert_eq!(site.consumed_materials, 0);
}

#[test]
fn build_site_not_duplicated_when_one_already_exists() {
    // Once a cell has a BuildSite, repeated ticks must not
    // spawn another one. The acceptance bullet says structures
    // "emerge automatically", not "multiply indefinitely".
    let mut app = build_app();
    let cell = IVec2::new(0, 0);
    app.world_mut()
        .resource_mut::<IntentGrid>()
        .paint(cell, IntentKind::Build, PAINT_STRENGTH_CAP);
    let center = common::cell_world_center(cell);

    // Pre-place a BuildSite manually. Auto-creation must not
    // add a second one.
    app.world_mut().spawn((
        BuildSite::new(cell, StructureKind::Basic),
        Transform::from_translation(center.extend(0.0)),
    ));

    for _ in 0..5 {
        app.update();
    }

    let world = app.world_mut();
    let mut q = world.query::<&BuildSite>();
    let count = q.iter(world).count();
    assert_eq!(count, 1, "auto-creation must not duplicate BuildSites");
}

#[test]
fn build_site_not_emerged_for_gather_only_cell() {
    // Build intent and Gather intent are distinct layers. A
    // Gather-only cell does not express construction demand, so
    // no BuildSite emerges.
    let mut app = build_app();
    let cell = IVec2::new(0, 0);
    app.world_mut().resource_mut::<IntentGrid>().paint(
        cell,
        IntentKind::Gather,
        PAINT_STRENGTH_CAP,
    );

    for _ in 0..3 {
        app.update();
    }

    let world = app.world_mut();
    let mut q = world.query::<&BuildSite>();
    let count = q.iter(world).count();
    assert_eq!(count, 0, "Gather-only cell must not spawn a BuildSite");
}

#[test]
fn idle_worker_chooses_build_via_autonomy_scoring() {
    // Acceptance: "Workers choose Build Zone construction/repair
    // work from autonomy scoring." An idle worker on a
    // Build-painted cell with a BuildSite must be assigned to
    // that site and start moving toward it.
    let mut app = build_app();
    let cell = IVec2::new(0, 0);
    app.world_mut()
        .resource_mut::<IntentGrid>()
        .paint(cell, IntentKind::Build, PAINT_STRENGTH_CAP);
    // Auto-creation will spawn a BuildSite in the cell on the
    // first tick. Give the worker a head start so the test is
    // not sensitive to the spawn tick ordering.
    let center = common::cell_world_center(cell);
    let worker = common::spawn_worker_at(&mut app, center);

    for _ in 0..3 {
        app.update();
    }

    let assignment = app
        .world()
        .entity(worker)
        .get::<BuildAssignment>()
        .expect("idle worker in a Build-painted cell must receive a BuildAssignment");
    assert_eq!(assignment.cell, cell);
}

#[test]
fn worker_builds_structure_consuming_local_stockpile() {
    // Acceptance: "Build work consumes resources delivered
    // physically to local stockpiles or needs." A worker at a
    // BuildSite pulls material from a local stockpile, the
    // site consumes the material, and the site becomes a
    // completed Structure once the material budget is exhausted.
    let mut app = build_app();
    let cell = IVec2::new(0, 0);
    app.world_mut()
        .resource_mut::<IntentGrid>()
        .paint(cell, IntentKind::Build, PAINT_STRENGTH_CAP);
    let center = common::cell_world_center(cell);
    // Pre-seed a stockpile with enough material to finish the
    // site. BuildSite auto-spawns in the cell on the first tick.
    let stockpile = common::spawn_stockpile(&mut app, center, BUILD_REQUIRED_MATERIALS * 2, 1000);
    let worker = common::spawn_worker_at(&mut app, center);

    // 1 tick to auto-create, 1 to assign, 1 to arrive,
    // BUILD_REQUIRED_MATERIALS work ticks, +5 buffer.
    let total_ticks = 3 + BUILD_REQUIRED_MATERIALS as usize + 5;
    for _ in 0..total_ticks {
        app.update();
    }

    let world = app.world_mut();
    let stockpile_state = world.entity(stockpile).get::<Stockpile>().unwrap();
    assert!(
        stockpile_state.amount < BUILD_REQUIRED_MATERIALS * 2,
        "stockpile must be drained by the build work; got {}",
        stockpile_state.amount
    );

    let remaining_sites_count = {
        let mut q = world.query::<&BuildSite>();
        q.iter(world).count()
    };
    let structure_count = {
        let mut q = world.query::<&Structure>();
        q.iter(world).count()
    };
    assert_eq!(
        remaining_sites_count, 0,
        "BuildSite must be removed when construction completes"
    );
    assert!(
        structure_count >= 1,
        "a completed Structure must exist after the worker finishes construction; got {}",
        structure_count
    );
    // The completed Structure starts at full health.
    let healths: Vec<u32> = {
        let mut q = world.query::<&Structure>();
        q.iter(world).map(|s| s.health).collect()
    };
    for health in healths {
        assert_eq!(
            health, STRUCTURE_MAX_HEALTH,
            "newly completed Structure must start at full health"
        );
    }
    // The worker is idle again.
    assert!(
        world.entity(worker).get::<BuildAssignment>().is_none(),
        "worker assignment must be released after the site completes"
    );
    assert!(
        world.entity(worker).get::<BuildProgress>().is_none(),
        "worker progress must be cleared after the site completes"
    );
}

#[test]
fn worker_repairs_damaged_structure_consuming_materials() {
    // Acceptance: "Build work consumes resources delivered
    // physically to local stockpiles or needs." A damaged
    // Structure in a Build cell can be repaired: the worker at
    // the site consumes material from a local stockpile and the
    // structure's health rises back to the max. This pins the
    // "repair" half of the "construct or repair" contract.
    let mut app = build_app();
    let cell = IVec2::new(0, 0);
    app.world_mut()
        .resource_mut::<IntentGrid>()
        .paint(cell, IntentKind::Build, PAINT_STRENGTH_CAP);
    let center = common::cell_world_center(cell);
    // Pre-place a damaged Structure in the cell. BuildSite
    // auto-spawn is suppressed for cells that already hold a
    // Structure, so no BuildSite is created.
    let mut damaged = Structure::new(StructureKind::Basic);
    damaged.health = STRUCTURE_MAX_HEALTH / 2;
    let structure = app
        .world_mut()
        .spawn((damaged, Transform::from_translation(center.extend(0.0))))
        .id();
    // Local stockpile in the same cell so the repair work has a
    // source of material.
    let _stockpile = common::spawn_stockpile(&mut app, center, BUILD_REQUIRED_MATERIALS * 2, 1000);
    let worker = common::spawn_worker_at(&mut app, center);

    // 3 overhead ticks + 5 repair ticks (50 damage / 10 health
    // per material) + 5 buffer.
    let total_ticks = 3 + 5 + 5;
    for _ in 0..total_ticks {
        app.update();
    }

    let world = app.world_mut();
    let s = world.entity(structure).get::<Structure>().unwrap();
    assert_eq!(
        s.health, STRUCTURE_MAX_HEALTH,
        "repair should bring the structure back to full health; got {}",
        s.health
    );
    // Worker is idle again.
    assert!(
        world.entity(worker).get::<BuildAssignment>().is_none(),
        "worker assignment must be released after the repair completes"
    );
}

#[test]
fn build_site_placed_inside_build_intent_cell() {
    // Acceptance: "Automatic support construction happens inside
    // or near matching intent paint." The BuildSite's world
    // position must fall inside the painted cell.
    let mut app = build_app();
    let cell = IVec2::new(1, -1);
    app.world_mut()
        .resource_mut::<IntentGrid>()
        .paint(cell, IntentKind::Build, PAINT_STRENGTH_CAP);

    app.update();

    let world = app.world_mut();
    let mut q = world.query::<(&BuildSite, &Transform)>();
    let (site, transform) = q
        .iter(world)
        .next()
        .expect("BuildSite must exist in the painted cell");
    assert_eq!(site.cell, cell);
    let site_cell = world_to_cell(transform.translation.truncate());
    assert_eq!(
        site_cell, cell,
        "BuildSite must be placed inside the painted cell"
    );
}

#[test]
fn build_work_consumes_minerals_from_local_stockpile_only() {
    // The build work's material source is the local stockpile in
    // the build cell. A distant stockpile must not be drained.
    // This pins the "local" half of "local stockpile-backed
    // material flow".
    let mut app = build_app();
    let cell = IVec2::new(0, 0);
    app.world_mut()
        .resource_mut::<IntentGrid>()
        .paint(cell, IntentKind::Build, PAINT_STRENGTH_CAP);
    let center = common::cell_world_center(cell);
    // A small local stockpile plus a large distant stockpile.
    let local = common::spawn_stockpile(&mut app, center, BUILD_REQUIRED_MATERIALS * 2, 1000);
    let distant_pos = center + Vec2::new(ZONE_BLOCK_SIZE * 4.0, 0.0);
    let distant = common::spawn_stockpile(&mut app, distant_pos, 10_000, 20_000);
    let worker = common::spawn_worker_at(&mut app, center);

    // Drive enough ticks for the build to make significant
    // progress but not so many that the build finishes (we want
    // to observe the local stock draining and the distant stock
    // intact).
    let ticks = (BUILD_REQUIRED_MATERIALS / 2) as usize + 5;
    for _ in 0..ticks {
        app.update();
    }

    let world = app.world_mut();
    let local_state = world.entity(local).get::<Stockpile>().unwrap();
    let distant_state = world.entity(distant).get::<Stockpile>().unwrap();
    assert!(
        local_state.amount < BUILD_REQUIRED_MATERIALS * 2,
        "local stockpile must be drained by build work; got {}",
        local_state.amount
    );
    assert_eq!(
        distant_state.amount, 10_000,
        "distant stockpile must NOT be drained by local build work"
    );
    // The worker has done useful work (consumed local material).
    // We do not pin a specific amount -- the test's contract is
    // "local drains, distant doesn't".
    let _ = world.entity(worker).get::<Transform>();
}

#[test]
fn idle_worker_in_build_cell_idles_when_no_stockpile_has_materials() {
    // The worker arrives at a BuildSite but no local stockpile
    // has material. The site must not consume anything and the
    // empty stockpile must stay empty.
    let mut app = build_app();
    let cell = IVec2::new(0, 0);
    app.world_mut()
        .resource_mut::<IntentGrid>()
        .paint(cell, IntentKind::Build, PAINT_STRENGTH_CAP);
    let center = common::cell_world_center(cell);
    let stockpile = common::spawn_stockpile(&mut app, center, 0, 1000);
    common::spawn_worker_at(&mut app, center);

    for _ in 0..5 {
        app.update();
    }

    let world = app.world_mut();
    let mut consumed = 0;
    {
        let mut q = world.query::<&BuildSite>();
        for site in q.iter(world) {
            consumed = site.consumed_materials;
        }
    }
    assert_eq!(
        consumed, 0,
        "BuildSite must not consume material when no local stockpile has it"
    );
    let s = world.entity(stockpile).get::<Stockpile>().unwrap();
    assert_eq!(s.amount, 0, "empty stockpile must stay empty");
}
