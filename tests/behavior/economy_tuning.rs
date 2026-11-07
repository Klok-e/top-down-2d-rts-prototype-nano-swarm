use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    nanobot::{
        MaintenanceProgress, NanobotType, OwnerSwarm, PlannedKind, PlannedStructure,
        ProductionFacility, Structure, StructureClearing, StructureKind, reconcile_economy_tuning,
    },
    tuning::Tuning,
};

#[test]
fn economy_edits_preserve_funded_cycles_materials_and_completed_clearing() {
    let mut world = World::new();
    let old = Tuning::default();
    let mut new = old.clone();
    new.production.ticks_per_bot = 240;
    new.production.cost_per_bot = 7;
    new.production.input_capacity = 10;
    new.construction.work_ticks = 300;
    let mut active = ProductionFacility::with_tuning(&old);
    active.current_target = Some(NanobotType::Hauler);
    active.progress = 150;
    active.input_amount = 65;
    let player = world.spawn_empty().id();
    let opponent = world.spawn_empty().id();
    let producing = world.spawn((active.clone(), OwnerSwarm(player))).id();
    active.progress = old.production.ticks_per_bot;
    active.finished_at = Some(42);
    let waiting = world.spawn((active, OwnerSwarm(opponent))).id();
    let partial = world
        .spawn(
            PlannedStructure::new(PlannedKind::SinkStockpile, IVec2::ZERO)
                .with_work_budget(1500)
                .with_work_remaining(1125),
        )
        .id();
    let clearing = world
        .spawn((
            PlannedStructure::new(PlannedKind::ProductionFacility, IVec2::ONE)
                .with_work_remaining(0),
            StructureClearing::awaiting_validation(Vec2::ZERO),
        ))
        .id();

    reconcile_economy_tuning(&mut world, &old, &new);

    let active = world.get::<ProductionFacility>(producing).unwrap();
    assert_eq!(active.progress, 60, "funded cycle retains 25% completion");
    assert_eq!(active.current_target, Some(NanobotType::Hauler));
    assert_eq!(
        active.input_amount, 65,
        "edits neither repay nor refund the cycle"
    );
    assert_eq!(
        active.input_free_space(),
        0,
        "overfull hopper rejects more material"
    );
    let waiting = world.get::<ProductionFacility>(waiting).unwrap();
    assert_eq!(waiting.progress, 240);
    assert_eq!(
        waiting.finished_at,
        Some(42),
        "exit ordering survives duration edits"
    );
    assert!(!waiting.is_busy(240));
    assert_eq!(
        world
            .get::<PlannedStructure>(partial)
            .unwrap()
            .construction_progress(),
        (75, 300)
    );
    assert!(
        world
            .get::<PlannedStructure>(clearing)
            .unwrap()
            .is_complete()
    );
    assert!(world.get::<StructureClearing>(clearing).is_some());
}

#[test]
fn economy_edits_preserve_condition_and_maintenance_phase() {
    let mut world = World::new();
    let old = Tuning::default();
    let mut new = old.clone();
    new.construction.max_health = 200;
    new.maintenance.buffer_ticks = 1800;
    new.maintenance.degradation_interval_ticks = 12;
    new.maintenance.work_ticks = 12;
    let mut condition = Structure::new(StructureKind::Basic);
    condition.health = 37;
    condition.ticks_since_maintained = 1800;
    let buffered = world.spawn(condition).id();
    condition.health = 1;
    condition.ticks_since_maintained = 3600 + 2 * 6 + 3;
    let overdue = world.spawn(condition).id();
    condition.health = 0;
    let collapsed = world.spawn(condition).id();
    let worker = world
        .spawn(MaintenanceProgress {
            cell: IVec2::ZERO,
            target: buffered,
            ticks_worked: 15,
        })
        .id();

    reconcile_economy_tuning(&mut world, &old, &new);

    let buffered = world.get::<Structure>(buffered).unwrap();
    assert_eq!(buffered.health, 74);
    assert_eq!(buffered.ticks_since_maintained, 900);
    let overdue = world.get::<Structure>(overdue).unwrap();
    assert_eq!(overdue.health, 2);
    assert_eq!(
        overdue.ticks_since_maintained, 1830,
        "two full degradation steps and a half-step remain"
    );
    assert_eq!(world.get::<Structure>(collapsed).unwrap().health, 0);
    assert_eq!(
        world
            .get::<MaintenanceProgress>(worker)
            .unwrap()
            .ticks_worked,
        6
    );
}

#[test]
fn shorter_work_budgets_do_not_complete_unfinished_work_by_rounding() {
    let mut world = World::new();
    let old = Tuning::default();
    let mut new = old.clone();
    new.construction.max_health = 1;
    new.construction.work_ticks = 1;
    new.production.ticks_per_bot = 1;
    new.maintenance.work_ticks = 1;
    let plan = world
        .spawn(
            PlannedStructure::new(PlannedKind::SinkStockpile, IVec2::ZERO).with_work_remaining(1),
        )
        .id();
    let mut facility = ProductionFacility::new();
    facility.current_target = Some(NanobotType::Worker);
    facility.progress = 599;
    let facility = world.spawn(facility).id();
    let worker = world
        .spawn(MaintenanceProgress {
            cell: IVec2::ZERO,
            target: plan,
            ticks_worked: 29,
        })
        .id();
    let mut health = Structure::new(StructureKind::Basic);
    health.health = 1;
    let structure = world.spawn(health).id();

    reconcile_economy_tuning(&mut world, &old, &new);

    assert_eq!(
        world
            .get::<PlannedStructure>(plan)
            .unwrap()
            .available_work(),
        1
    );
    assert_eq!(
        world.get::<ProductionFacility>(facility).unwrap().progress,
        0
    );
    assert_eq!(
        world
            .get::<MaintenanceProgress>(worker)
            .unwrap()
            .ticks_worked,
        0
    );
    assert_eq!(
        world.get::<Structure>(structure).unwrap().health,
        1,
        "positive structures survive a lower health budget"
    );
}
