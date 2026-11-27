use bevy::{ecs::system::RunSystemOnce, prelude::*, time::TimeUpdateStrategy};
use top_down_2d_rts_prototype_nano_swarm::{
    Presentation, build_app_with_presentation,
    intent::{IntentGrid, IntentKind},
    nanobot::{
        Cargo, ClearingEvacuation, DirectMovementComponent, HaulerAssignment, HaulerLoading,
        InteractionRegion, LogisticsReservation, MaintenanceAssignment, MaintenanceProgress,
        MatchOutcome, Nanobot, NanobotType, OwnerSwarm, PlannedKind, PlannedStructure,
        PlannedStructureClaim, PlannedStructureProgress, ProductionFacility, Structure,
        StructureClearing, Swarm, SwarmId, SwarmMember, construction_access::ConstructionAccess,
    },
    reachability::SwarmReachabilityMap,
    resources::{ResourceKind, ResourceLedger, Stockpile, StockpileRole},
    scenario_selection::{Scenario, ScenarioSelection},
    session::SessionRules,
    tuning::Tuning,
    zones::{PlayerIntentAction, apply_player_intent},
};

#[path = "../common/mod.rs"]
mod common;

const FACILITY_POSITION: Vec3 = Vec3::new(972., 252., 0.);
const FACILITY_CELL: IVec2 = IVec2::X;
const NEIGHBOR: IVec2 = IVec2::new(2, 0);
const OPPONENT: SwarmId = SwarmId(1);

fn sandbox() -> (App, Entity) {
    let mut app = build_app_with_presentation(Presentation::SimulationOnly);
    app.world_mut().resource_mut::<ScenarioSelection>().current = Scenario::Sandbox;
    app.insert_resource(TimeUpdateStrategy::FixedTimesteps(1));
    app.update();
    let world = app.world_mut();
    let (owner, id) = world
        .query_filtered::<(Entity, &SwarmId), With<Swarm>>()
        .single(world)
        .expect("Sandbox has one swarm");
    assert_eq!(*id, SwarmId::PLAYER);
    (app, owner)
}

fn apply_paint(app: &mut App, cell: IVec2, kind: IntentKind, action: PlayerIntentAction) {
    let ground = app.world().resource::<SwarmReachabilityMap>().clone();
    let outcome = *app.world().resource::<MatchOutcome>();
    let rules = *app.world().resource::<SessionRules>();
    assert_eq!(
        apply_player_intent(
            &mut app.world_mut().resource_mut::<IntentGrid>(),
            &ground,
            outcome,
            rules,
            cell,
            kind,
            action,
        ),
        Ok(true)
    );
}

fn build_paint_across_facility(app: &mut App) {
    for cell in [FACILITY_CELL, NEIGHBOR] {
        apply_paint(app, cell, IntentKind::Build, PlayerIntentAction::Paint);
    }
    app.world_mut()
        .resource_mut::<IntentGrid>()
        .paint(NEIGHBOR, IntentKind::Build, OPPONENT);
}

#[test]
fn erasing_owner_paint_cancels_all_plans_including_a_facility_that_is_clearing() {
    let (mut app, owner) = sandbox();
    build_paint_across_facility(&mut app);
    let facility = app
        .world_mut()
        .spawn((
            PlannedStructure::new(PlannedKind::ProductionFacility, FACILITY_CELL),
            OwnerSwarm(owner),
            PlannedKind::ProductionFacility.footprint_transform(FACILITY_POSITION),
        ))
        .id();

    let mut plans = Vec::new();
    for (kind, paint, cell) in [
        (PlannedKind::SourceStockpile, IntentKind::Gather, IVec2::Y),
        (
            PlannedKind::SinkStockpile,
            IntentKind::Build,
            IVec2::new(2, 1),
        ),
        (PlannedKind::Charger, IntentKind::Defend, IVec2::new(2, 2)),
    ] {
        apply_paint(&mut app, cell, paint, PlayerIntentAction::Paint);
        let transform = kind.footprint_transform(common::cell_world_center(cell).extend(0.));
        let plan = app
            .world_mut()
            .spawn((
                PlannedStructure::new(kind, cell),
                OwnerSwarm(owner),
                transform,
            ))
            .id();
        plans.push((kind, paint, cell, plan));
    }

    let (_, _, source_cell, source) = plans[0];
    let region = InteractionRegion::structure(app.world().get::<Transform>(source).unwrap());
    let worker = common::spawn_worker_at(
        &mut app,
        region.approach(common::cell_world_center(source_cell) + Vec2::Y * 200.),
    );
    assert!(
        app.world_mut()
            .get_mut::<PlannedStructure>(source)
            .unwrap()
            .try_claim(worker)
    );
    app.world_mut().entity_mut(worker).insert((
        PlannedStructureClaim {
            cell: source_cell,
            target: source,
        },
        PlannedStructureProgress {
            cell: source_cell,
            target: source,
        },
    ));

    app.update();
    for entity in std::iter::once(facility).chain(plans.iter().map(|(_, _, _, entity)| *entity)) {
        assert!(app.world().get::<PlannedStructure>(entity).is_some());
    }

    let builder = Vec2::new(828., 252.);
    let validated_layout = app
        .world_mut()
        .run_system_once(|access: ConstructionAccess, grid: Res<IntentGrid>| {
            access.snapshot().validation_key(&grid)
        })
        .unwrap();
    app.world_mut().entity_mut(facility).insert((
        PlannedStructure::new(PlannedKind::ProductionFacility, FACILITY_CELL)
            .with_work_remaining(0),
        StructureClearing::validated(builder, validated_layout),
    ));
    let occupant = common::spawn_hauler_at(&mut app, FACILITY_POSITION.truncate());
    let exit = FACILITY_POSITION.truncate() + Vec2::X * 144.;
    app.world_mut().entity_mut(occupant).insert((
        ClearingEvacuation {
            structure: facility,
            goal: exit,
        },
        DirectMovementComponent {
            xy: exit,
            stop_radius: 0.,
            interaction: None,
            speed: None,
        },
    ));

    apply_paint(
        &mut app,
        NEIGHBOR,
        IntentKind::Build,
        PlayerIntentAction::Erase,
    );
    for &(_, paint, cell, _) in &plans {
        apply_paint(&mut app, cell, paint, PlayerIntentAction::Erase);
    }
    app.update();

    assert!(
        app.world().get_entity(facility).is_err(),
        "the facility survived erasing one of the cells under its clearing footprint"
    );
    for (kind, _, _, entity) in plans {
        assert!(
            app.world().get_entity(entity).is_err(),
            "{kind:?} plan survived erasing its paint"
        );
    }
    assert!(app.world().get::<PlannedStructureClaim>(worker).is_none());
    assert!(
        app.world()
            .get::<PlannedStructureProgress>(worker)
            .is_none()
    );
    assert!(app.world().get::<ClearingEvacuation>(occupant).is_none());
    assert!(
        app.world()
            .get::<DirectMovementComponent>(occupant)
            .is_none_or(|movement| movement.xy != exit || movement.interaction.is_some()),
        "the occupant still has the cancelled evacuation goal"
    );
    let grid = app.world().resource::<IntentGrid>();
    assert!(
        grid.cell(FACILITY_CELL)
            .unwrap()
            .has_owned(IntentKind::Build, SwarmId::PLAYER)
    );
    assert!(
        grid.cell(NEIGHBOR)
            .unwrap()
            .has_owned(IntentKind::Build, OPPONENT)
    );
}

#[test]
fn erasing_a_footprint_edge_pauses_funded_production_and_maintenance_until_repainted() {
    let (mut app, owner) = sandbox();
    build_paint_across_facility(&mut app);

    let ticks_per_bot = {
        let mut tuning = app.world_mut().resource_mut::<Tuning>();
        tuning.maintenance.buffer_ticks = 3;
        tuning.maintenance.degradation_interval_ticks = 1;
        tuning.production.ticks_per_bot
    };
    let transform = PlannedKind::ProductionFacility.footprint_transform(FACILITY_POSITION);
    let mut state = ProductionFacility::new();
    let hopper = state.input_capacity.min(3);
    state.input_amount = hopper;
    state.current_target = Some(NanobotType::Defender);
    state.progress = ticks_per_bot - 2;
    let facility = app
        .world_mut()
        .spawn((state, OwnerSwarm(owner), transform))
        .id();

    let region = InteractionRegion::structure(&transform);
    let worker = common::spawn_worker_at(
        &mut app,
        region.approach(FACILITY_POSITION.truncate() - Vec2::X * 200.),
    );
    app.world_mut().entity_mut(worker).insert((
        MaintenanceAssignment {
            cell: FACILITY_CELL,
            target: facility,
        },
        MaintenanceProgress {
            cell: FACILITY_CELL,
            target: facility,
            ticks_worked: 0,
        },
    ));
    app.update();

    let defenders_before_erase = player_defenders(app.world_mut());
    let state = app.world().get::<ProductionFacility>(facility).unwrap();
    assert_eq!(state.progress, ticks_per_bot - 1);
    let maintained = *app.world().get::<Structure>(facility).unwrap();
    assert_eq!(
        app.world()
            .get::<MaintenanceProgress>(worker)
            .unwrap()
            .ticks_worked,
        1
    );

    apply_paint(
        &mut app,
        NEIGHBOR,
        IntentKind::Build,
        PlayerIntentAction::Erase,
    );
    app.update();

    let state = app.world().get::<ProductionFacility>(facility).unwrap();
    assert_eq!(state.current_target, Some(NanobotType::Defender));
    assert_eq!(state.progress, ticks_per_bot - 1);
    assert_eq!(state.input_amount, hopper);
    assert!(app.world().get::<MaintenanceAssignment>(worker).is_none());
    assert!(app.world().get::<MaintenanceProgress>(worker).is_none());
    let aging = app.world().get::<Structure>(facility).unwrap();
    assert_eq!(
        aging.ticks_since_maintained,
        maintained.ticks_since_maintained + 1
    );
    assert_eq!(aging.health, maintained.health);

    for _ in 0..3 {
        app.update();
    }
    assert!(app.world().get::<Structure>(facility).unwrap().health < maintained.health);
    assert_eq!(player_defenders(app.world_mut()), defenders_before_erase);

    apply_paint(
        &mut app,
        NEIGHBOR,
        IntentKind::Build,
        PlayerIntentAction::Paint,
    );
    app.update();

    assert_eq!(
        player_defenders(app.world_mut()),
        defenders_before_erase + 1
    );
    let state = app.world().get::<ProductionFacility>(facility).unwrap();
    assert_eq!(state.current_target, None);
    assert_eq!(state.input_amount, hopper);
}

fn player_defenders(world: &mut World) -> usize {
    world
        .query::<(&NanobotType, &SwarmMember)>()
        .iter(world)
        .filter(|(kind, swarm)| **kind == NanobotType::Defender && swarm.0 == SwarmId::PLAYER)
        .count()
}

#[test]
fn erasing_a_stockpiles_paint_stops_loading_until_the_same_stockpile_is_repainted() {
    const MINERALS: u32 = 10;
    let pickup_cell = IVec2::X;
    for (kind, role, paint) in [
        (
            PlannedKind::SourceStockpile,
            StockpileRole::Source,
            IntentKind::Gather,
        ),
        (
            PlannedKind::SinkStockpile,
            StockpileRole::Sink,
            IntentKind::Build,
        ),
    ] {
        let (mut app, owner) = sandbox();
        apply_paint(&mut app, pickup_cell, paint, PlayerIntentAction::Paint);
        let world = app.world_mut();
        let bots = world
            .query_filtered::<Entity, With<Nanobot>>()
            .iter(world)
            .collect::<Vec<_>>();
        for bot in bots {
            world.despawn(bot);
        }

        let terminal = world
            .query_filtered::<Entity, With<ProductionFacility>>()
            .single(world)
            .unwrap();
        world
            .get_mut::<ProductionFacility>(terminal)
            .unwrap()
            .current_target = Some(NanobotType::Worker);
        {
            let mut tuning = world.resource_mut::<Tuning>();
            tuning.production.ticks_per_bot = 600;
            tuning.logistics.material_work_ticks = 2;
        }
        let transform = kind.footprint_transform(Vec3::new(612., 252., 0.));
        let pickup = world
            .spawn((
                Stockpile {
                    kind: ResourceKind::Minerals,
                    amount: MINERALS,
                    capacity: MINERALS,
                    radius: 32.,
                },
                role,
                OwnerSwarm(owner),
                transform,
            ))
            .id();
        world.resource_mut::<ResourceLedger>().add_for(
            SwarmId::PLAYER,
            ResourceKind::Minerals,
            MINERALS,
        );

        let region = InteractionRegion::structure(&transform);
        let hauler = common::spawn_hauler_at(&mut app, region.approach(Vec2::new(396., 252.)));
        app.world_mut().entity_mut(hauler).insert((
            HaulerAssignment {
                source: pickup,
                sink: terminal,
                pickup_role: role,
            },
            LogisticsReservation::new(pickup, terminal, ResourceKind::Minerals, MINERALS),
        ));

        let stock = |app: &App| app.world().get::<Stockpile>(pickup).unwrap().amount;
        let cargo = |app: &App| {
            app.world()
                .get::<Cargo>(hauler)
                .map_or(0, |load| load.amount)
        };
        let received = |app: &App| {
            app.world()
                .get::<ProductionFacility>(terminal)
                .unwrap()
                .input_amount
        };

        for _ in 0..18 {
            app.update();
            if cargo(&app) > 0 {
                break;
            }
        }
        assert!(
            app.world().get::<HaulerLoading>(hauler).is_some(),
            "{role:?} loading never began"
        );
        let partial_load = cargo(&app);
        assert!((1..MINERALS).contains(&partial_load));
        let remaining = MINERALS - partial_load;
        assert_eq!(stock(&app), remaining);

        apply_paint(&mut app, pickup_cell, paint, PlayerIntentAction::Erase);
        app.update();

        assert!(
            app.world().get::<HaulerLoading>(hauler).is_none(),
            "{role:?} loading survived erasure"
        );
        assert_eq!(cargo(&app) + received(&app), partial_load);
        for _ in 0..24 {
            app.update();
        }
        assert_eq!(
            stock(&app),
            remaining,
            "{role:?} stock moved while unpainted"
        );

        apply_paint(&mut app, pickup_cell, paint, PlayerIntentAction::Paint);
        for _ in 0..100 {
            app.update();
            if stock(&app) < remaining {
                break;
            }
        }
        assert!(
            stock(&app) < remaining,
            "{role:?} loading did not resume after repainting"
        );
        assert_eq!(stock(&app) + cargo(&app) + received(&app), MINERALS);
        assert_eq!(
            app.world()
                .resource::<ResourceLedger>()
                .total_for(SwarmId::PLAYER, ResourceKind::Minerals),
            MINERALS
        );
    }
}
