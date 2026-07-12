use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    intent::{IntentGrid, IntentKind},
    nanobot::{
        project_actionable_opportunities_system, ActionableProjection, AllocationRegion,
        OpportunityCategory, PlannedKind, PlannedStructure, Structure, StructureKind,
        MAINTENANCE_NEEDS_THRESHOLD,
    },
    resources::{ResourceDeposit, ResourceKind, Stockpile, StockpileRole},
};

fn projection_app() -> App {
    let mut app = App::new();
    app.insert_resource(IntentGrid::new(32, 32))
        .init_resource::<ActionableProjection>()
        .add_systems(Update, project_actionable_opportunities_system);
    app
}

#[test]
fn gather_paint_projects_only_live_overlapping_deposit_work() {
    let mut app = projection_app();
    app.world_mut()
        .resource_mut::<IntentGrid>()
        .add(IVec2::ZERO, IntentKind::Gather, 6);
    app.world_mut().spawn((
        ResourceDeposit {
            kind: ResourceKind::Minerals,
            amount: 12,
            capacity: 20,
            radius: 16.0,
        },
        Transform::from_xyz(32.0, 32.0, 0.0),
    ));

    app.update();

    let projection = app.world().resource::<ActionableProjection>();
    let opportunities = projection.opportunities(AllocationRegion::for_cell(IVec2::ZERO));
    assert_eq!(opportunities.len(), 1);
    assert_eq!(opportunities[0].category, OpportunityCategory::Gather);
    assert_eq!(opportunities[0].paint_strength, 6);
    assert_eq!(opportunities[0].available_work, 12);
}

#[test]
fn unclaimed_planned_structure_projects_remaining_build_work() {
    let mut app = projection_app();
    app.world_mut().spawn(PlannedStructure::new(
        PlannedKind::ProductionFacility,
        IVec2::new(9, 1),
    ));

    app.update();

    let projection = app.world().resource::<ActionableProjection>();
    let opportunities = projection.opportunities(AllocationRegion::for_cell(IVec2::new(9, 1)));
    assert_eq!(opportunities.len(), 1);
    assert_eq!(opportunities[0].category, OpportunityCategory::PlannedBuild);
    assert_eq!(opportunities[0].available_work, 5);
}

#[test]
fn build_and_defend_intent_project_maintenance_and_defend_work() {
    let mut app = projection_app();
    {
        let mut grid = app.world_mut().resource_mut::<IntentGrid>();
        grid.add(IVec2::ZERO, IntentKind::Build, 4);
        grid.add(IVec2::ZERO, IntentKind::Defend, 7);
    }
    let mut structure = Structure::new(StructureKind::Basic);
    structure.ticks_since_maintained = MAINTENANCE_NEEDS_THRESHOLD;
    app.world_mut()
        .spawn((structure, Transform::from_xyz(32.0, 32.0, 0.0)));

    app.update();

    let projection = app.world().resource::<ActionableProjection>();
    let opportunities = projection.opportunities(AllocationRegion::for_cell(IVec2::ZERO));
    assert_eq!(
        opportunities
            .iter()
            .map(|opportunity| opportunity.category)
            .collect::<Vec<_>>(),
        vec![
            OpportunityCategory::Maintenance,
            OpportunityCategory::Defend
        ]
    );
    assert_eq!(opportunities[1].available_work, 7);
}

#[test]
fn haul_opportunity_is_indexed_by_source_region() {
    let mut app = projection_app();
    let source_cell = IVec2::new(9, 0);
    app.world_mut().spawn((
        Stockpile {
            kind: ResourceKind::Minerals,
            amount: 20,
            capacity: 20,
            radius: 16.0,
        },
        StockpileRole::Source,
        Transform::from_xyz(9.5 * 512.0, 32.0, 0.0),
    ));
    app.world_mut().spawn((
        Stockpile {
            kind: ResourceKind::Minerals,
            amount: 0,
            capacity: 50,
            radius: 16.0,
        },
        StockpileRole::Sink,
        Transform::from_xyz(32.0, 32.0, 0.0),
    ));

    app.update();

    let projection = app.world().resource::<ActionableProjection>();
    let source_work = projection.opportunities(AllocationRegion::for_cell(source_cell));
    assert_eq!(source_work.len(), 1);
    assert_eq!(source_work[0].category, OpportunityCategory::Haul);
    assert_eq!(source_work[0].cell, source_cell);
    assert_eq!(source_work[0].available_work, 20);
    assert!(
        projection
            .opportunities(AllocationRegion::for_cell(IVec2::ZERO))
            .is_empty(),
        "sink region must not own source-anchored haul work"
    );
}

#[test]
fn projection_replaces_only_regions_dirtied_by_intent_changes() {
    let mut app = projection_app();
    let first = IVec2::ZERO;
    let second = IVec2::new(9, 0);
    {
        let mut grid = app.world_mut().resource_mut::<IntentGrid>();
        grid.add(first, IntentKind::Defend, 2);
        grid.add(second, IntentKind::Defend, 3);
    }
    app.update();

    app.world_mut()
        .resource_mut::<IntentGrid>()
        .remove(first, IntentKind::Defend);
    app.update();

    let projection = app.world().resource::<ActionableProjection>();
    assert!(projection
        .opportunities(AllocationRegion::for_cell(first))
        .is_empty());
    let untouched = projection.opportunities(AllocationRegion::for_cell(second));
    assert_eq!(untouched.len(), 1);
    assert_eq!(untouched[0].paint_strength, 3);
    assert_eq!(
        app.world().resource::<IntentGrid>().render_dirty_count(),
        2,
        "projection consumption must not drain render changes"
    );
}
