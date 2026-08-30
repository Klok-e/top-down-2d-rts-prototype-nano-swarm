use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    intent::{IntentGrid, IntentKind},
    nanobot::{
        ActionableOpportunity, ActionableProjection, AllocationRegion, ChargerAssignment,
        ChargerProgress, Health, MAINTENANCE_NEEDS_THRESHOLD, OpportunityCategory,
        OpportunityTarget, OwnerSwarm, PlannedKind, PlannedStructure,
        SUPPORT_OPERATIONAL_HEALTH_THRESHOLD, Structure, StructureKind, SwarmId, SwarmMember,
    },
    resources::{ResourceDeposit, ResourceKind, Stockpile, StockpileRole},
};

#[path = "../common/mod.rs"]
mod common;

#[test]
fn gather_paint_projects_only_live_overlapping_deposit_work() {
    let mut app = common::minimal_app_with_actionable_projection();
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
    let mut app = common::minimal_app_with_actionable_projection();
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
fn stale_structure_projects_maintenance_without_defend_work() {
    let mut app = common::minimal_app_with_actionable_projection();
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
        vec![OpportunityCategory::Maintenance]
    );
    assert_eq!(opportunities[0].available_work, 1);
}

#[test]
fn unattended_valid_charger_does_not_project_maintenance() {
    let mut app = common::minimal_app_with_actionable_projection();
    let charger = common::spawn_projected_charger(
        &mut app,
        common::ProjectedChargerFixture {
            cell: IVec2::ZERO,
            amount: 10,
            ticks_since_maintained: MAINTENANCE_NEEDS_THRESHOLD,
            owner: SwarmId::PLAYER,
            defend_paint_owner: SwarmId::PLAYER,
        },
    );

    app.update();

    let maintenance = app
        .world()
        .resource::<ActionableProjection>()
        .opportunities(AllocationRegion::for_cell(IVec2::ZERO))
        .iter()
        .filter(|opportunity| {
            opportunity.category == OpportunityCategory::Maintenance
                && opportunity.target == OpportunityTarget::Maintenance { structure: charger }
        })
        .count();
    assert_eq!(
        maintenance, 0,
        "unattended Charger capacity must be allowed to degrade",
    );
}

#[test]
fn friendly_defender_in_charger_or_adjacent_cell_projects_maintenance() {
    let offsets = [
        IVec2::new(-1, -1),
        IVec2::new(0, -1),
        IVec2::new(1, -1),
        IVec2::new(-1, 0),
        IVec2::ZERO,
        IVec2::new(1, 0),
        IVec2::new(-1, 1),
        IVec2::new(0, 1),
        IVec2::new(1, 1),
    ];

    for offset in offsets {
        let mut app = common::minimal_app_with_actionable_projection();
        let charger = common::spawn_projected_charger(
            &mut app,
            common::ProjectedChargerFixture {
                cell: IVec2::ZERO,
                amount: 10,
                ticks_since_maintained: MAINTENANCE_NEEDS_THRESHOLD,
                owner: SwarmId::PLAYER,
                defend_paint_owner: SwarmId::PLAYER,
            },
        );
        let defender = common::spawn_defender_at(&mut app, common::cell_world_center(offset));
        app.world_mut()
            .entity_mut(defender)
            .insert(SwarmMember::new(SwarmId::PLAYER));

        app.update();

        let projected = app
            .world()
            .resource::<ActionableProjection>()
            .opportunities(AllocationRegion::for_cell(IVec2::ZERO))
            .iter()
            .any(|opportunity| {
                opportunity.target == OpportunityTarget::Maintenance { structure: charger }
            });
        assert!(
            projected,
            "friendly Defender offset {offset:?} must keep nearby Charger capacity maintained",
        );
    }
}

#[test]
fn despawned_nearby_defender_removes_cached_charger_maintenance() {
    let mut app = common::minimal_app_with_actionable_projection();
    let charger = common::spawn_projected_charger(
        &mut app,
        common::ProjectedChargerFixture {
            cell: IVec2::ZERO,
            amount: 10,
            ticks_since_maintained: MAINTENANCE_NEEDS_THRESHOLD,
            owner: SwarmId::PLAYER,
            defend_paint_owner: SwarmId::PLAYER,
        },
    );
    let defender = common::spawn_defender_at(&mut app, common::cell_world_center(IVec2::new(1, 0)));
    app.world_mut()
        .entity_mut(defender)
        .insert(SwarmMember::new(SwarmId::PLAYER));
    app.update();
    assert!(
        app.world()
            .resource::<ActionableProjection>()
            .opportunities(AllocationRegion::for_cell(IVec2::ZERO))
            .iter()
            .any(|opportunity| {
                opportunity.target == OpportunityTarget::Maintenance { structure: charger }
            }),
        "nearby live Defender initially requests Charger Maintenance",
    );

    app.world_mut().despawn(defender);
    app.update();

    assert!(
        app.world()
            .resource::<ActionableProjection>()
            .opportunities(AllocationRegion::for_cell(IVec2::ZERO))
            .iter()
            .all(|opportunity| {
                opportunity.target != OpportunityTarget::Maintenance { structure: charger }
            }),
        "despawned Defender must stop requesting cached Charger Maintenance",
    );
}

#[test]
fn distant_foreign_or_dead_defender_does_not_project_charger_maintenance() {
    let cases = [
        ("two-cell distance", SwarmId::PLAYER, IVec2::new(2, 0), true),
        ("foreign Defender", SwarmId(11), IVec2::new(1, 0), true),
        ("dead Defender", SwarmId::PLAYER, IVec2::new(1, 0), false),
    ];

    for (label, swarm, defender_cell, alive) in cases {
        let mut app = common::minimal_app_with_actionable_projection();
        let charger = common::spawn_projected_charger(
            &mut app,
            common::ProjectedChargerFixture {
                cell: IVec2::ZERO,
                amount: 10,
                ticks_since_maintained: MAINTENANCE_NEEDS_THRESHOLD,
                owner: SwarmId::PLAYER,
                defend_paint_owner: SwarmId::PLAYER,
            },
        );
        let defender =
            common::spawn_defender_at(&mut app, common::cell_world_center(defender_cell));
        app.world_mut()
            .entity_mut(defender)
            .insert(SwarmMember::new(swarm));
        if !alive {
            app.world_mut()
                .entity_mut(defender)
                .get_mut::<Health>()
                .expect("Defender has Health")
                .current = 0;
        }

        app.update();

        let projected = app
            .world()
            .resource::<ActionableProjection>()
            .opportunities(AllocationRegion::for_cell(IVec2::ZERO))
            .iter()
            .any(|opportunity| {
                opportunity.target == OpportunityTarget::Maintenance { structure: charger }
            });
        assert!(!projected, "{label} must not maintain the Charger");
    }
}

#[test]
fn assigned_en_route_or_charging_defender_projects_charger_maintenance() {
    for service_state in ["assigned", "en-route", "charging"] {
        let mut app = common::minimal_app_with_actionable_projection();
        let charger = common::spawn_projected_charger(
            &mut app,
            common::ProjectedChargerFixture {
                cell: IVec2::ZERO,
                amount: 10,
                ticks_since_maintained: MAINTENANCE_NEEDS_THRESHOLD,
                owner: SwarmId::PLAYER,
                defend_paint_owner: SwarmId::PLAYER,
            },
        );
        let defender =
            common::spawn_defender_at(&mut app, common::cell_world_center(IVec2::new(3, 0)));
        app.world_mut()
            .entity_mut(defender)
            .insert(SwarmMember::new(SwarmId::PLAYER));
        match service_state {
            "assigned" => {
                app.world_mut()
                    .entity_mut(defender)
                    .insert(ChargerAssignment { charger });
            }
            "en-route" => {
                app.world_mut().entity_mut(defender).insert((
                    ChargerAssignment { charger },
                    top_down_2d_rts_prototype_nano_swarm::nanobot::DirectMovementComponent {
                        xy: Vec2::ZERO,
                        stop_radius: 1.0,
                    },
                ));
            }
            "charging" => {
                app.world_mut()
                    .entity_mut(defender)
                    .insert((ChargerAssignment { charger }, ChargerProgress { charger }));
            }
            _ => unreachable!(),
        }

        app.update();

        let projected = app
            .world()
            .resource::<ActionableProjection>()
            .opportunities(AllocationRegion::for_cell(IVec2::ZERO))
            .iter()
            .any(|opportunity| {
                opportunity.target == OpportunityTarget::Maintenance { structure: charger }
            });
        assert!(
            projected,
            "{service_state} Defender service must maintain the Charger",
        );
    }
}

#[test]
fn haul_opportunity_is_indexed_by_source_region() {
    let mut app = common::minimal_app_with_actionable_projection();
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
fn degraded_stockpile_does_not_project_haul_work() {
    let mut app = common::minimal_app_with_actionable_projection();
    let swarm = app.world_mut().spawn(SwarmId::PLAYER).id();
    let mut condition = Structure::new(StructureKind::Basic);
    condition.health = SUPPORT_OPERATIONAL_HEALTH_THRESHOLD - 1;
    app.world_mut().spawn((
        Stockpile {
            kind: ResourceKind::Minerals,
            amount: 20,
            capacity: 20,
            radius: 16.0,
        },
        StockpileRole::Source,
        OwnerSwarm(swarm),
        condition,
        Transform::from_xyz(32.0, 32.0, 0.0),
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
        Transform::from_xyz(544.0, 32.0, 0.0),
    ));

    app.update();

    assert!(
        app.world()
            .resource::<ActionableProjection>()
            .opportunities(AllocationRegion::for_cell(IVec2::ZERO))
            .iter()
            .all(|opportunity| opportunity.category != OpportunityCategory::Haul),
        "degraded Stockpile must stop participating in logistics",
    );
}

#[test]
fn haul_projection_rejects_unowned_and_cross_swarm_pairs() {
    let mut app = common::minimal_app_with_actionable_projection();
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

    assert!(
        app.world()
            .resource::<ActionableProjection>()
            .opportunities(AllocationRegion::for_cell(IVec2::ZERO))
            .is_empty()
    );
}

#[test]
fn projection_replaces_only_regions_dirtied_by_intent_changes() {
    let mut app = common::minimal_app_with_actionable_projection();
    let first = IVec2::ZERO;
    let second = IVec2::new(9, 0);
    {
        let mut grid = app.world_mut().resource_mut::<IntentGrid>();
        grid.add(first, IntentKind::Gather);
        grid.add(second, IntentKind::Gather);
    }
    for cell in [first, second] {
        app.world_mut().spawn((
            ResourceDeposit {
                kind: ResourceKind::Minerals,
                amount: 20,
                capacity: 20,
                radius: 16.0,
            },
            Transform::from_translation(
                top_down_2d_rts_prototype_nano_swarm::ai::get_world_from_zone(cell).extend(0.0),
            ),
        ));
    }
    app.update();

    app.world_mut()
        .resource_mut::<IntentGrid>()
        .remove(first, IntentKind::Gather);
    app.update();

    let projection = app.world().resource::<ActionableProjection>();
    assert!(
        projection
            .opportunities(AllocationRegion::for_cell(first))
            .is_empty()
    );
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
    let mut app = common::minimal_app_with_actionable_projection();
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

#[test]
fn unowned_deposit_projects_once_for_each_distinct_painted_owner() {
    let mut app = common::minimal_app_with_actionable_projection();
    let first_owner = SwarmId::PLAYER;
    let second_owner = SwarmId(7);
    let left = IVec2::ZERO;
    let right = IVec2::X;
    {
        let mut grid = app.world_mut().resource_mut::<IntentGrid>();
        assert!(grid.paint_owned(left, IntentKind::Gather, Some(first_owner)));
        assert!(grid.paint_owned(right, IntentKind::Gather, Some(second_owner)));
    }
    app.world_mut().spawn((
        ResourceDeposit {
            kind: ResourceKind::Minerals,
            amount: 20,
            capacity: 20,
            radius: 64.0,
        },
        Transform::from_xyz(512.0, 256.0, 0.0),
    ));

    app.update();

    let opportunities = gather_opportunities(app.world().resource::<ActionableProjection>());
    assert_eq!(opportunities.len(), 2);
    assert_eq!(
        opportunities
            .iter()
            .filter(|opportunity| opportunity.owner == Some(first_owner))
            .count(),
        1
    );
    assert_eq!(
        opportunities
            .iter()
            .filter(|opportunity| opportunity.owner == Some(second_owner))
            .count(),
        1
    );
}

#[test]
fn unowned_gather_paint_suppresses_owned_groups_for_unowned_deposit() {
    let mut app = common::minimal_app_with_actionable_projection();
    let shared = IVec2::ZERO;
    let owned = IVec2::X;
    {
        let mut grid = app.world_mut().resource_mut::<IntentGrid>();
        assert!(grid.paint(shared, IntentKind::Gather));
        assert!(grid.paint_owned(owned, IntentKind::Gather, Some(SwarmId::PLAYER)));
    }
    app.world_mut().spawn((
        ResourceDeposit {
            kind: ResourceKind::Minerals,
            amount: 20,
            capacity: 20,
            radius: 64.0,
        },
        Transform::from_xyz(512.0, 256.0, 0.0),
    ));

    app.update();

    let opportunities = gather_opportunities(app.world().resource::<ActionableProjection>());
    assert_eq!(opportunities.len(), 1);
    assert_eq!(opportunities[0].owner, None);
    assert_eq!(opportunities[0].cell, shared);
}

#[test]
fn deposit_owner_changes_and_removal_reproject_gather_opportunity() {
    let mut app = common::minimal_app_with_actionable_projection();
    let first_swarm = app.world_mut().spawn(SwarmId::PLAYER).id();
    let second_id = SwarmId(7);
    let second_swarm = app.world_mut().spawn(second_id).id();
    let shared = IVec2::ZERO;
    let first_owned = IVec2::X;
    {
        let mut grid = app.world_mut().resource_mut::<IntentGrid>();
        assert!(grid.paint(shared, IntentKind::Gather));
        assert!(grid.paint_owned(first_owned, IntentKind::Gather, Some(SwarmId::PLAYER)));
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
            OwnerSwarm(first_swarm),
            Transform::from_xyz(512.0, 256.0, 0.0),
        ))
        .id();

    app.update();
    let opportunities = gather_opportunities(app.world().resource::<ActionableProjection>());
    assert_eq!(opportunities.len(), 1);
    assert_eq!(opportunities[0].owner, Some(SwarmId::PLAYER));
    assert_eq!(opportunities[0].cell, shared);

    app.world_mut()
        .entity_mut(deposit)
        .insert(OwnerSwarm(second_swarm));
    app.update();
    let opportunities = gather_opportunities(app.world().resource::<ActionableProjection>());
    assert_eq!(opportunities.len(), 1);
    assert_eq!(opportunities[0].owner, Some(second_id));
    assert_eq!(opportunities[0].cell, shared);

    app.world_mut().entity_mut(deposit).remove::<OwnerSwarm>();
    app.update();
    let opportunities = gather_opportunities(app.world().resource::<ActionableProjection>());
    assert_eq!(opportunities.len(), 1);
    assert_eq!(opportunities[0].owner, None);
    assert_eq!(opportunities[0].cell, shared);
}
