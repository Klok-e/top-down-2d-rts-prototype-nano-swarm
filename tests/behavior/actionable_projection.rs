use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    intent::{IntentGrid, IntentKind},
    nanobot::{
        project_actionable_opportunities_system, ActionableOpportunity, ActionableProjection,
        AllocationRegion, OpportunityCategory, OwnerSwarm, PlannedKind, PlannedStructure,
        Structure, StructureKind, SwarmId, MAINTENANCE_NEEDS_THRESHOLD,
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
        .add(IVec2::ZERO, IntentKind::Gather);
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
        grid.add(IVec2::ZERO, IntentKind::Build);
        grid.add(IVec2::ZERO, IntentKind::Defend);
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
    assert_eq!(opportunities[1].available_work, 1);
}

#[test]
fn haul_opportunity_is_indexed_by_source_region() {
    let mut app = projection_app();
    let swarm = app.world_mut().spawn(SwarmId::PLAYER).id();
    let source_cell = IVec2::new(9, 0);
    app.world_mut().spawn((
        Stockpile {
            kind: ResourceKind::Minerals,
            amount: 20,
            capacity: 20,
            radius: 16.0,
        },
        StockpileRole::Source,
        OwnerSwarm(swarm),
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
        OwnerSwarm(swarm),
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
fn haul_projection_rejects_unowned_and_cross_swarm_pairs() {
    let mut app = projection_app();
    let player = app.world_mut().spawn(SwarmId::PLAYER).id();
    let enemy = app.world_mut().spawn(SwarmId(9)).id();
    let source = Stockpile {
        kind: ResourceKind::Minerals,
        amount: 20,
        capacity: 20,
        radius: 16.0,
    };
    let sink = Stockpile {
        kind: ResourceKind::Minerals,
        amount: 0,
        capacity: 50,
        radius: 16.0,
    };
    app.world_mut().spawn((
        source,
        StockpileRole::Source,
        OwnerSwarm(player),
        Transform::from_xyz(32.0, 32.0, 0.0),
    ));
    app.world_mut().spawn((
        sink,
        StockpileRole::Sink,
        OwnerSwarm(enemy),
        Transform::from_xyz(64.0, 32.0, 0.0),
    ));
    app.world_mut().spawn((
        sink,
        StockpileRole::Sink,
        Transform::from_xyz(96.0, 32.0, 0.0),
    ));

    app.update();

    assert!(app
        .world()
        .resource::<ActionableProjection>()
        .opportunities(AllocationRegion::for_cell(IVec2::ZERO))
        .is_empty());
}

#[test]
fn projection_replaces_only_regions_dirtied_by_intent_changes() {
    let mut app = projection_app();
    let first = IVec2::ZERO;
    let second = IVec2::new(9, 0);
    {
        let mut grid = app.world_mut().resource_mut::<IntentGrid>();
        grid.add(first, IntentKind::Defend);
        grid.add(second, IntentKind::Defend);
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
    assert_eq!(
        app.world().resource::<IntentGrid>().render_dirty_count(),
        2,
        "projection consumption must not drain render changes"
    );
}

fn gather_opportunities(projection: &ActionableProjection) -> Vec<ActionableOpportunity> {
    projection
        .iter_regions()
        .flat_map(|(_, opportunities)| opportunities.iter().copied())
        .filter(|opportunity| opportunity.category == OpportunityCategory::Gather)
        .collect()
}

#[test]
fn deposit_projects_once_across_overlapping_allocation_regions() {
    let mut app = projection_app();
    let left = IVec2::new(7, 0);
    let right = IVec2::new(8, 0);
    {
        let mut grid = app.world_mut().resource_mut::<IntentGrid>();
        grid.paint(left, IntentKind::Gather);
        grid.paint(right, IntentKind::Gather);
    }
    let deposit = app
        .world_mut()
        .spawn((
            ResourceDeposit {
                kind: ResourceKind::Minerals,
                amount: 20,
                capacity: 20,
                radius: 64.0,
            },
            Transform::from_xyz(8.0 * 512.0, 256.0, 0.0),
        ))
        .id();

    app.update();
    let opportunities = gather_opportunities(app.world().resource::<ActionableProjection>());
    assert_eq!(opportunities.len(), 1);
    assert_eq!(opportunities[0].cell, left);

    app.world_mut()
        .resource_mut::<IntentGrid>()
        .erase(left, IntentKind::Gather);
    app.update();
    let opportunities = gather_opportunities(app.world().resource::<ActionableProjection>());
    assert_eq!(opportunities.len(), 1);
    assert_eq!(opportunities[0].cell, right);

    app.world_mut()
        .entity_mut(deposit)
        .get_mut::<ResourceDeposit>()
        .unwrap()
        .amount = 0;
    app.update();
    assert!(gather_opportunities(app.world().resource::<ActionableProjection>()).is_empty());
}
