//! Near-runtime Defender feel flow: authored-front travel, readable combat,
//! and staggered swarm-wide recharge.

use std::{collections::HashMap, time::Duration};

use bevy::{asset::AssetPlugin, math::Vec2, prelude::*};
use top_down_2d_rts_prototype_nano_swarm::{
    MAP_HEIGHT, MAP_WIDTH,
    ai::AiPlugin,
    game_settings::GameSettings,
    intent::{IntentGrid, IntentKind},
    nanobot::{
        Charge, ChargePlugin, Charger, ChargerAssignment, ChargerProgress, CollapsePlugin,
        CombatPlugin, DEFAULT_PLANNED_WORK_TICKS, DEGRADATION_INTERVAL_TICKS, DefenderResponse,
        DirectMovementComponent, GatherPlugin, HaulPlugin, Health, LOW_CHARGE_THRESHOLD,
        MAINTENANCE_BUFFER_TICKS, MAINTENANCE_NEEDS_THRESHOLD, MaintenanceAssignment,
        MaintenancePlugin, MaintenanceProgress, MatchOutcome, Nanobot, NanobotPlugin, NanobotType,
        OpponentIntentPlugin, OpponentSwarmIdAlloc, OwnerSwarm, PlannedKind, PlannedStructure,
        PlannedStructurePlugin, PopulationDemand, PopulationDemandPlugin, ProductionCollapseState,
        ProductionPlugin, ProductionPriority, RegionalAllocationPlugin, STRUCTURE_MAX_HEALTH,
        Structure, Swarm, SwarmId, SwarmMember, TerritorySnapshot, nanobot_death_cleanup_system,
        world_to_cell,
    },
    resources::{ResourceKind, ResourceLedger},
    scenario::{
        OPPONENT_BUILD_FLANK_CELL, OPPONENT_CELL, OPPONENT_DEFEND_CELL, PLAYER_BUILD_FLANK_CELL,
        PLAYER_CELL, PLAYER_DEFEND_CELL, spawn_default_opponent_scenario,
        spawn_default_player_scenario,
    },
};

#[path = "../common/mod.rs"]
mod common;

const OPPONENT_SWARM: SwarmId = SwarmId(1);
const MAX_TICKS_AFTER_CONTACT: u32 = 900;
const MAX_CONTACT_TICKS: u32 = 500;
const MOVEMENT_DISTANCE_TOLERANCE: f32 = 1e-3;

fn spawn_default_scenario_startup(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut grid: ResMut<IntentGrid>,
    opponent_id_alloc: ResMut<OpponentSwarmIdAlloc>,
) {
    spawn_default_player_scenario(&mut commands, &asset_server, &mut grid);
    spawn_default_opponent_scenario(&mut commands, &asset_server, &mut grid, opponent_id_alloc);
}

fn default_headless_app() -> App {
    let mut app = common::minimal_app();
    app.world_mut().resource_mut::<GameSettings>().bot_speed = 5.25;
    app.insert_resource(IntentGrid::new(MAP_WIDTH as i32, MAP_HEIGHT as i32))
        .add_plugins(TaskPoolPlugin::default())
        .add_plugins(AssetPlugin::default())
        .init_asset::<Image>()
        .insert_resource(ProductionPriority::default())
        .init_resource::<OpponentSwarmIdAlloc>()
        .add_plugins(NanobotPlugin::default())
        .add_plugins(GatherPlugin)
        .add_plugins(HaulPlugin)
        .add_plugins(PlannedStructurePlugin)
        .add_plugins(MaintenancePlugin)
        .add_plugins(ProductionPlugin)
        .add_plugins(CollapsePlugin)
        .add_plugins(ChargePlugin)
        .add_plugins(CombatPlugin)
        .add_plugins(OpponentIntentPlugin)
        .add_plugins(RegionalAllocationPlugin)
        .add_plugins(PopulationDemandPlugin)
        .add_plugins(AiPlugin)
        .add_systems(Startup, spawn_default_scenario_startup);
    app
}

#[test]
fn authored_default_scenario_reaches_primary_defend_contest() {
    let mut app = default_headless_app();
    app.update();

    let initial_defenders = |world: &mut World, swarm: SwarmId| {
        world
            .query_filtered::<(&SwarmMember, &NanobotType), With<Nanobot>>()
            .iter(world)
            .filter(|(member, kind)| member.0 == swarm && **kind == NanobotType::Defender)
            .count()
    };
    assert_eq!(initial_defenders(app.world_mut(), SwarmId::PLAYER), 3);
    assert_eq!(initial_defenders(app.world_mut(), SwarmId(1)), 3);
    let territory = app.world().resource::<TerritorySnapshot>();
    assert_eq!(territory.tile_count(SwarmId::PLAYER), 4);
    assert_eq!(territory.tile_count(SwarmId(1)), 4);
    let demand = app.world().resource::<PopulationDemand>();
    assert_eq!(
        demand.desired_for(SwarmId::PLAYER, NanobotType::Defender),
        2,
        "four player Swarm Tiles create a peaceful reserve of two Defenders",
    );
    assert_eq!(
        demand.desired_for(SwarmId(1), NanobotType::Defender),
        2,
        "four opponent Swarm Tiles create a peaceful reserve of two Defenders",
    );

    {
        let grid = app.world().resource::<IntentGrid>();
        assert!(
            grid.cell(PLAYER_DEFEND_CELL)
                .is_some_and(|cell| cell.has(IntentKind::Defend))
        );
        assert!(
            grid.cell(OPPONENT_DEFEND_CELL)
                .is_some_and(|cell| cell.has(IntentKind::Defend))
        );
        assert!(
            !grid
                .cell(PLAYER_BUILD_FLANK_CELL)
                .is_some_and(|cell| cell.has(IntentKind::Defend))
        );
        assert!(
            !grid
                .cell(OPPONENT_BUILD_FLANK_CELL)
                .is_some_and(|cell| cell.has(IntentKind::Defend))
        );
        assert!(grid.defend_contest(PLAYER_DEFEND_CELL).is_none());
    }

    for _ in 0..299 {
        app.update();
    }
    assert_eq!(
        initial_defenders(app.world_mut(), SwarmId::PLAYER),
        3,
        "the third seeded player Defender remains as excess reserve while the front is peaceful",
    );
    assert_eq!(
        initial_defenders(app.world_mut(), SwarmId(1)),
        3,
        "the third seeded opponent Defender remains as excess reserve while the front is peaceful",
    );
    for _ in 0..2 {
        app.update();
    }

    assert!(
        app.world()
            .resource::<IntentGrid>()
            .defend_contest(PLAYER_DEFEND_CELL)
            .is_some(),
        "authored opponent cadence must contest the player's primary Defend cell"
    );
    assert_eq!(
        *app.world().resource::<MatchOutcome>(),
        MatchOutcome::InProgress,
        "quiet staging must not overbuild a Charger and collapse the opponent economy"
    );

    let initial_player_health = aggregate_defender_health(app.world_mut(), SwarmId::PLAYER);
    let initial_opponent_health = aggregate_defender_health(app.world_mut(), SwarmId(1));
    let mut previous_positions = positions(app.world_mut());
    let mut saw_both_participants = false;
    for _ in 0..301 {
        let rotations = RotationSnapshot::capture(app.world_mut());
        app.update();
        rotations.assert_admissions(app.world_mut());
        previous_positions = assert_default_tick_state(
            app.world_mut(),
            &previous_positions,
            &mut saw_both_participants,
        );
    }
    for _ in 0..900 {
        let rotations = RotationSnapshot::capture(app.world_mut());
        app.update();
        rotations.assert_admissions(app.world_mut());
        previous_positions = assert_default_tick_state(
            app.world_mut(),
            &previous_positions,
            &mut saw_both_participants,
        );
    }
    let final_player_health = aggregate_defender_health(app.world_mut(), SwarmId::PLAYER);
    let final_opponent_health = aggregate_defender_health(app.world_mut(), SwarmId(1));
    let final_contest = app
        .world()
        .resource::<IntentGrid>()
        .defend_contest(PLAYER_DEFEND_CELL)
        .is_some();
    assert!(
        final_player_health < initial_player_health
            || final_opponent_health < initial_opponent_health
            || !final_contest,
        "authored default contact must cause combat or capture by +900"
    );
    assert_eq!(
        *app.world().resource::<MatchOutcome>(),
        MatchOutcome::InProgress,
        "the four-tile authored economy should remain recoverable through the proof horizon",
    );
    let collapse = app.world().resource::<ProductionCollapseState>();
    assert!(!collapse.player_collapsed);
    assert!(!collapse.opponent_collapsed);
    assert!(
        saw_both_participants,
        "default fronts never established physical participants from both swarms"
    );
}

#[test]
fn authored_paint_edit_redistributes_the_full_unengaged_cohort_without_parking() {
    let mut app = common::sim_app();
    let original_cell = IVec2::ZERO;
    let added_cell = IVec2::X;
    app.world_mut().resource_mut::<IntentGrid>().paint_owned(
        original_cell,
        IntentKind::Defend,
        Some(SwarmId::PLAYER),
    );
    let center = common::cell_world_center(original_cell);
    let defenders = [
        common::spawn_defender_at(&mut app, center + Vec2::new(-20.0, 0.0)),
        common::spawn_defender_at(&mut app, center),
        common::spawn_defender_at(&mut app, center + Vec2::new(20.0, 0.0)),
    ];

    app.update();
    let before_roaming = defenders.map(|defender| {
        app.world()
            .entity(defender)
            .get::<Transform>()
            .expect("authored Defender has a position")
            .translation
            .truncate()
    });
    for _ in 0..48 {
        app.update();
    }
    for (defender, before) in defenders.into_iter().zip(before_roaming) {
        let after = app
            .world()
            .entity(defender)
            .get::<Transform>()
            .expect("authored Defender remains alive while staging")
            .translation
            .truncate();
        assert_eq!(world_to_cell(after), original_cell);
        assert!(
            after.distance(before) > 1.0,
            "every unengaged Defender should keep moving before the paint edit",
        );
    }

    app.world_mut().resource_mut::<IntentGrid>().paint_owned(
        added_cell,
        IntentKind::Defend,
        Some(SwarmId::PLAYER),
    );
    app.update();

    let moving_to_added = defenders
        .iter()
        .filter(|defender| {
            app.world()
                .entity(**defender)
                .get::<DirectMovementComponent>()
                .is_some_and(|movement| world_to_cell(movement.xy) == added_cell)
        })
        .count();
    assert_eq!(
        moving_to_added, 1,
        "adding a second Defend cell should immediately redistribute one of three Defenders",
    );

    for _ in 0..180 {
        app.update();
    }
    let mut occupancy = [0_usize; 2];
    let arrived_positions = defenders.map(|defender| {
        let position = app
            .world()
            .entity(defender)
            .get::<Transform>()
            .expect("the full authored cohort remains alive after redistribution")
            .translation
            .truncate();
        match world_to_cell(position) {
            cell if cell == original_cell => occupancy[0] += 1,
            cell if cell == added_cell => occupancy[1] += 1,
            cell => panic!("Defender left the authored staging layout for {cell:?}"),
        }
        position
    });
    occupancy.sort_unstable();
    assert_eq!(occupancy, [1, 2]);

    for _ in 0..48 {
        app.update();
    }
    for (defender, arrived) in defenders.into_iter().zip(arrived_positions) {
        let roaming = app
            .world()
            .entity(defender)
            .get::<Transform>()
            .expect("redistributed Defender remains alive while roaming")
            .translation
            .truncate();
        assert!(
            roaming.distance(arrived) > 1.0,
            "every redistributed Defender should continue roaming instead of parking",
        );
    }
}

#[test]
fn authored_non_defend_pursuit_returns_to_current_staging_for_each_swarm() {
    for (defending_swarm, hostile_swarm) in [
        (SwarmId::PLAYER, SwarmId(11)),
        (SwarmId(11), SwarmId::PLAYER),
    ] {
        let mut app = common::sim_app();
        app.world_mut().spawn((Swarm {}, defending_swarm));
        app.world_mut().spawn((Swarm {}, hostile_swarm));
        let original_staging = IVec2::ZERO;
        let current_staging = IVec2::Y;
        let non_defend_territory = IVec2::new(2, 0);
        let diagonal_halo = non_defend_territory + IVec2::ONE;
        {
            let mut grid = app.world_mut().resource_mut::<IntentGrid>();
            grid.paint_owned(original_staging, IntentKind::Defend, Some(defending_swarm));
            grid.paint_owned(
                non_defend_territory,
                IntentKind::Gather,
                Some(defending_swarm),
            );
        }
        let defender =
            common::spawn_defender_at(&mut app, common::cell_world_center(original_staging));
        app.world_mut()
            .entity_mut(defender)
            .insert(SwarmMember::new(defending_swarm));
        app.update();

        let target =
            common::spawn_worker_at(&mut app, common::cell_world_center(non_defend_territory));
        app.world_mut()
            .entity_mut(target)
            .insert(SwarmMember::new(hostile_swarm));
        let before_intercept = app
            .world()
            .entity(defender)
            .get::<Transform>()
            .expect("authored Defender has a position")
            .translation
            .truncate();

        app.update();

        assert_eq!(
            app.world()
                .entity(defender)
                .get::<DefenderResponse>()
                .map(|response| response.target),
            Some(target),
            "{defending_swarm:?} should intercept a hostile on owned non-Defend territory",
        );
        for _ in 0..12 {
            app.update();
        }
        let intercepting = app
            .world()
            .entity(defender)
            .get::<Transform>()
            .expect("intercepting Defender remains alive")
            .translation
            .truncate();
        assert!(
            intercepting.distance(common::cell_world_center(non_defend_territory))
                < before_intercept.distance(common::cell_world_center(non_defend_territory)),
            "{defending_swarm:?} Defender should physically close on the non-Defend Threat",
        );

        app.world_mut()
            .entity_mut(target)
            .get_mut::<Transform>()
            .expect("authored hostile has a position")
            .translation = common::cell_world_center(diagonal_halo).extend(0.0);
        {
            let mut grid = app.world_mut().resource_mut::<IntentGrid>();
            grid.remove(original_staging, IntentKind::Defend);
            grid.paint_owned(current_staging, IntentKind::Defend, Some(defending_swarm));
        }
        app.update();
        assert_eq!(
            app.world()
                .entity(defender)
                .get::<DefenderResponse>()
                .map(|response| response.target),
            Some(target),
            "{defending_swarm:?} should retain its claim through the diagonal Pursuit Halo",
        );

        app.world_mut().despawn(target);
        app.update();
        let returning = app.world().entity(defender);
        assert!(
            returning.get::<DefenderResponse>().is_none(),
            "removed Threat should release the response",
        );
        assert_eq!(
            returning
                .get::<DirectMovementComponent>()
                .map(|movement| world_to_cell(movement.xy)),
            Some(current_staging),
            "{defending_swarm:?} should return to the staging layout painted during pursuit",
        );
        for _ in 0..180 {
            app.update();
        }
        let returned_cell = world_to_cell(
            app.world()
                .entity(defender)
                .get::<Transform>()
                .expect("returned Defender remains alive")
                .translation
                .truncate(),
        );
        assert_eq!(returned_cell, current_staging);
    }
}

#[test]
fn authored_charge_rotation_replaces_active_coverage_for_each_swarm() {
    for (defending_swarm, hostile_swarm) in [
        (SwarmId::PLAYER, SwarmId(11)),
        (SwarmId(11), SwarmId::PLAYER),
    ] {
        let mut app = common::sim_app_with_charge();
        let fixed_step = Duration::from_nanos(16_666_667);
        app.insert_resource(Time::<Fixed>::from_duration(fixed_step));
        app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(fixed_step));
        let defending_entity = app.world_mut().spawn((Swarm {}, defending_swarm)).id();
        app.world_mut().spawn((Swarm {}, hostile_swarm));
        let charger_cell = IVec2::ZERO;
        let threatened_cell = IVec2::X;
        {
            let mut grid = app.world_mut().resource_mut::<IntentGrid>();
            grid.paint_owned(charger_cell, IntentKind::Defend, Some(defending_swarm));
            grid.paint_owned(threatened_cell, IntentKind::Gather, Some(defending_swarm));
        }
        let charger = common::spawn_operational_charger_at(&mut app, charger_cell, 100);
        app.world_mut()
            .entity_mut(charger)
            .insert(OwnerSwarm(defending_entity));
        let defenders = [
            common::spawn_defender_at(
                &mut app,
                common::cell_world_center(threatened_cell) + Vec2::new(-24.0, 0.0),
            ),
            common::spawn_defender_at(
                &mut app,
                common::cell_world_center(threatened_cell) + Vec2::new(24.0, 0.0),
            ),
        ];
        for defender in defenders {
            app.world_mut()
                .entity_mut(defender)
                .insert(SwarmMember::new(defending_swarm));
        }
        let threat = common::spawn_worker_at(&mut app, common::cell_world_center(threatened_cell));
        app.world_mut()
            .entity_mut(threat)
            .insert(SwarmMember::new(hostile_swarm));

        app.update();
        let responder = defenders
            .into_iter()
            .find(|defender| {
                app.world()
                    .entity(*defender)
                    .get::<DefenderResponse>()
                    .is_some_and(|response| response.target == threat)
            })
            .expect("one authored Defender should cover the non-Defend Threat");
        let replacement = defenders
            .into_iter()
            .find(|defender| *defender != responder)
            .expect("the authored flow keeps one staged replacement");
        app.world_mut()
            .entity_mut(responder)
            .get_mut::<Charge>()
            .expect("Defender has Charge")
            .current = LOW_CHARGE_THRESHOLD;

        app.update();

        let departing = app.world().entity(responder);
        assert_eq!(
            departing
                .get::<ChargerAssignment>()
                .map(|assignment| assignment.charger),
            Some(charger),
            "{defending_swarm:?} low-Charge responder should rotate to its owned Charger",
        );
        assert!(departing.get::<DefenderResponse>().is_none());
        assert_eq!(
            app.world()
                .entity(replacement)
                .get::<DefenderResponse>()
                .map(|response| response.target),
            Some(threat),
            "{defending_swarm:?} should replace coverage on the same fixed step",
        );
    }
}

#[test]
fn authored_charger_planning_and_maintenance_follow_observed_service_need() {
    let mut app = common::sim_app_with_charge_planned();
    app.add_plugins(MaintenancePlugin);
    let charger_cell = IVec2::ZERO;
    let center = common::cell_world_center(charger_cell);
    let swarm = common::spawn_swarm_at(&mut app, center);
    app.world_mut().resource_mut::<IntentGrid>().paint_owned(
        charger_cell,
        IntentKind::Defend,
        Some(SwarmId::PLAYER),
    );
    let defender = common::spawn_defender_at(&mut app, center);
    app.world_mut()
        .entity_mut(defender)
        .get_mut::<Charge>()
        .expect("authored Defender has Charge")
        .current = LOW_CHARGE_THRESHOLD;
    let builder = common::spawn_worker_at(&mut app, center + Vec2::X * 68.0);

    app.update();

    let planned_charger = {
        let world = app.world_mut();
        world
            .query::<(Entity, &PlannedStructure, &OwnerSwarm)>()
            .iter(world)
            .find_map(|(entity, planned, owner)| {
                (planned.kind == PlannedKind::Charger && owner.0 == swarm).then_some(entity)
            })
            .expect("unserved low Charge should create one owner-scoped Charger plan")
    };
    app.world_mut()
        .entity_mut(defender)
        .get_mut::<Charge>()
        .expect("authored Defender has Charge")
        .current = 1.0;

    for _ in 0..(1 + DEFAULT_PLANNED_WORK_TICKS as usize + 2) {
        app.update();
    }
    assert!(
        app.world()
            .entity(planned_charger)
            .get::<Charger>()
            .is_some(),
        "the authored Worker should complete the observed-need plan",
    );
    {
        let mut charger_entity = app.world_mut().entity_mut(planned_charger);
        let mut charger = charger_entity
            .get_mut::<Charger>()
            .expect("completed plan is a Charger");
        charger.amount = charger.capacity;
    }
    app.world_mut()
        .entity_mut(planned_charger)
        .get_mut::<Structure>()
        .expect("completed Charger has shared condition")
        .ticks_since_maintained = MAINTENANCE_NEEDS_THRESHOLD;
    app.world_mut()
        .entity_mut(defender)
        .get_mut::<Charge>()
        .expect("authored Defender has Charge")
        .current = LOW_CHARGE_THRESHOLD;
    app.world_mut().despawn(builder);
    let service_worker = common::spawn_worker_at(&mut app, center);

    app.update();

    let serviced_defender = app.world().entity(defender);
    assert_eq!(
        serviced_defender
            .get::<ChargerAssignment>()
            .map(|assignment| assignment.charger),
        Some(planned_charger),
        "supplied capacity should serve the low-Charge Defender",
    );
    assert!(
        serviced_defender.get::<ChargerProgress>().is_some()
            || serviced_defender.get::<DirectMovementComponent>().is_some(),
        "service should be physically in progress or en route",
    );
    let worker = app.world().entity(service_worker);
    let maintenance_target = worker
        .get::<MaintenanceAssignment>()
        .map(|assignment| assignment.target)
        .or_else(|| {
            worker
                .get::<MaintenanceProgress>()
                .map(|progress| progress.target)
        });
    assert_eq!(
        maintenance_target,
        Some(planned_charger),
        "active Charger service should create observable Worker Maintenance",
    );

    app.world_mut().despawn(defender);
    app.world_mut().despawn(service_worker);
    {
        let mut charger_entity = app.world_mut().entity_mut(planned_charger);
        let mut condition = charger_entity
            .get_mut::<Structure>()
            .expect("unattended Charger remains a structure");
        condition.health = STRUCTURE_MAX_HEALTH;
        condition.ticks_since_maintained =
            MAINTENANCE_BUFFER_TICKS + DEGRADATION_INTERVAL_TICKS - 1;
    }
    let idle_worker = common::spawn_worker_at(&mut app, center);

    app.update();

    let idle_worker = app.world().entity(idle_worker);
    assert!(
        idle_worker.get::<MaintenanceAssignment>().is_none()
            && idle_worker.get::<MaintenanceProgress>().is_none(),
        "unattended Charger capacity should not consume Worker upkeep",
    );
    assert_eq!(
        app.world()
            .entity(planned_charger)
            .get::<Structure>()
            .expect("unattended Charger remains standing while degrading")
            .health,
        STRUCTURE_MAX_HEALTH - 1,
        "unattended Charger should decay through shared condition rules",
    );
}

fn live_defenders(world: &mut World, swarm: SwarmId) -> usize {
    world
        .query_filtered::<(&SwarmMember, &NanobotType, &Health), With<Nanobot>>()
        .iter(world)
        .filter(|(member, kind, health)| {
            member.0 == swarm && **kind == NanobotType::Defender && health.current > 0
        })
        .count()
}

fn aggregate_defender_health(world: &mut World, swarm: SwarmId) -> u32 {
    world
        .query_filtered::<(&SwarmMember, &NanobotType, &Health), With<Nanobot>>()
        .iter(world)
        .filter(|(member, kind, _)| member.0 == swarm && **kind == NanobotType::Defender)
        .map(|(_, _, health)| health.current)
        .sum()
}

fn positions(world: &mut World) -> HashMap<Entity, Vec2> {
    world
        .query_filtered::<(Entity, &Transform), With<Nanobot>>()
        .iter(world)
        .map(|(entity, transform)| (entity, transform.translation.truncate()))
        .collect()
}

fn assert_charge_bounds(world: &mut World) {
    for (charge, kind) in world
        .query_filtered::<(&Charge, &NanobotType), With<Nanobot>>()
        .iter(world)
    {
        if *kind != NanobotType::Defender {
            continue;
        }
        assert!(charge.current.is_finite());
        assert!(charge.current >= 0.0);
        assert!(charge.current <= charge.max);
    }
}

fn charger_loads(world: &mut World) -> HashMap<Entity, usize> {
    let mut loads = HashMap::new();
    for assignment in world.query::<&ChargerAssignment>().iter(world) {
        *loads.entry(assignment.charger).or_default() += 1;
    }
    loads
}

struct RotationSnapshot {
    assignments: HashMap<Entity, (SwarmId, Entity)>,
    living: HashMap<SwarmId, usize>,
}

impl RotationSnapshot {
    fn capture(world: &mut World) -> Self {
        Self {
            assignments: world
                .query::<(Entity, &SwarmMember, &ChargerAssignment)>()
                .iter(world)
                .map(|(entity, member, assignment)| (entity, (member.0, assignment.charger)))
                .collect(),
            living: [SwarmId::PLAYER, OPPONENT_SWARM]
                .map(|swarm| (swarm, live_defenders(world, swarm)))
                .into_iter()
                .collect(),
        }
    }

    fn assert_admissions(&self, world: &mut World) {
        let current = Self::capture(world);
        for swarm in [SwarmId::PLAYER, OPPONENT_SWARM] {
            let has_new_rotation = current.assignments.iter().any(|(entity, assignment)| {
                assignment.0 == swarm && self.assignments.get(entity) != Some(assignment)
            });
            if !has_new_rotation {
                continue;
            }
            // Combat and production can change the population within this tick.
            // Focused Charge tests cover exact admission-time capacity.
            let living = self.living[&swarm].max(current.living[&swarm]);
            let rotating = current
                .assignments
                .values()
                .filter(|(owner, _)| *owner == swarm)
                .count();
            assert!(
                rotating <= (living / 2).max(1),
                "swarm {swarm:?} admitted a new rotation with {rotating} active and {living} living Defenders"
            );
        }
    }
}

fn participants_in_front(world: &mut World, cell: IVec2, swarm: SwarmId) -> usize {
    world
        .query_filtered::<(&Transform, &SwarmMember, &NanobotType, &Health), With<Nanobot>>()
        .iter(world)
        .filter(|(transform, member, kind, health)| {
            member.0 == swarm
                && **kind == NanobotType::Defender
                && health.current > 0
                && world_to_cell(transform.translation.truncate()) == cell
        })
        .count()
}

fn assert_default_tick_state(
    world: &mut World,
    previous_positions: &HashMap<Entity, Vec2>,
    saw_both_participants: &mut bool,
) -> HashMap<Entity, Vec2> {
    let current_positions = positions(world);
    for (entity, position) in &current_positions {
        assert!(position.is_finite());
        if let Some(previous) = previous_positions.get(entity) {
            let displacement = position.distance(*previous);
            assert!(
                displacement <= 5.25 + MOVEMENT_DISTANCE_TOLERANCE,
                "authored default nanobot {entity:?} exceeded fixed-tick speed limit: {displacement}"
            );
        }
    }
    assert_charge_bounds(world);
    for (charger, load) in charger_loads(world) {
        assert!(
            load <= 3,
            "authored default charger {charger:?} exceeded its three-Defender service limit: {load}"
        );
    }

    let player_participants = participants_in_front(world, PLAYER_DEFEND_CELL, SwarmId::PLAYER);
    let opponent_participants = participants_in_front(world, PLAYER_DEFEND_CELL, SwarmId(1));
    if player_participants > 0 && opponent_participants > 0 {
        *saw_both_participants = true;
    }
    current_positions
}

fn spawn_runtime_front() -> (App, IVec2, Entity, Entity) {
    let mut app = common::sim_app_with_charge();
    app.world_mut().resource_mut::<GameSettings>().bot_speed = 5.25;
    app.insert_resource(ProductionPriority::default());
    app.add_plugins(CombatPlugin);
    app.add_plugins(CollapsePlugin);
    app.add_systems(FixedLast, nanobot_death_cleanup_system);

    let player_swarm = common::spawn_swarm_at(&mut app, common::cell_world_center(PLAYER_CELL));
    let opponent_swarm = common::spawn_opponent_swarm_with_nanobots(
        &mut app,
        common::cell_world_center(OPPONENT_CELL),
        ProductionPriority::default(),
        &[(NanobotType::Defender, 3)],
    );
    let opponent_id = *app
        .world()
        .entity(opponent_swarm)
        .get::<SwarmId>()
        .expect("opponent swarm id");
    assert_eq!(opponent_id, OPPONENT_SWARM);

    for _ in 0..6 {
        common::spawn_defender_at(&mut app, common::cell_world_center(PLAYER_CELL));
    }
    let _player_worker = common::spawn_worker_at(&mut app, common::cell_world_center(PLAYER_CELL));
    let opponent_worker =
        common::spawn_worker_at(&mut app, common::cell_world_center(OPPONENT_CELL));
    app.world_mut()
        .entity_mut(opponent_worker)
        .insert(SwarmMember::new(opponent_id));
    let _player_hauler = common::spawn_hauler_at(&mut app, common::cell_world_center(PLAYER_CELL));
    let opponent_hauler =
        common::spawn_hauler_at(&mut app, common::cell_world_center(OPPONENT_CELL));
    app.world_mut()
        .entity_mut(opponent_hauler)
        .insert(SwarmMember::new(opponent_id));
    let front = OPPONENT_DEFEND_CELL;
    {
        let mut grid = app.world_mut().resource_mut::<IntentGrid>();
        grid.paint_owned(
            PLAYER_DEFEND_CELL,
            IntentKind::Defend,
            Some(SwarmId::PLAYER),
        );
        grid.paint_owned(front, IntentKind::Defend, Some(opponent_id));
        grid.contest_defend(front, SwarmId::PLAYER);
    }

    let player_charger = common::spawn_operational_charger_at(&mut app, PLAYER_DEFEND_CELL, 60);
    app.world_mut()
        .entity_mut(player_charger)
        .insert(OwnerSwarm(player_swarm));
    let opponent_charger = common::spawn_operational_charger_at(&mut app, front, 60);
    app.world_mut()
        .entity_mut(opponent_charger)
        .insert(OwnerSwarm(opponent_swarm));
    {
        let ledger = &mut *app.world_mut().resource_mut::<ResourceLedger>();
        ledger.add_for(SwarmId::PLAYER, ResourceKind::Minerals, 60);
        ledger.add_for(opponent_id, ResourceKind::Minerals, 60);
    }
    let _player_facility = common::spawn_facility_at(
        &mut app,
        player_swarm,
        common::cell_world_center(PLAYER_CELL) + Vec2::new(160.0, 0.0),
    );
    let _opponent_facility = common::spawn_facility_at(
        &mut app,
        opponent_swarm,
        common::cell_world_center(OPPONENT_CELL) - Vec2::new(160.0, 0.0),
    );

    let mut player_defenders = {
        let world = app.world_mut();
        world
            .query_filtered::<(Entity, &SwarmMember, &NanobotType), With<Nanobot>>()
            .iter(world)
            .filter_map(|(entity, member, kind)| {
                (member.0 == SwarmId::PLAYER && *kind == NanobotType::Defender).then_some(entity)
            })
            .collect::<Vec<_>>()
    };
    player_defenders.sort_by_key(|entity| entity.to_bits());
    for entity in player_defenders.into_iter().take(3) {
        app.world_mut()
            .entity_mut(entity)
            .get_mut::<Charge>()
            .unwrap()
            .current = 0.5;
    }
    let mut opponent_defenders = {
        let world = app.world_mut();
        world
            .query_filtered::<(Entity, &SwarmMember, &NanobotType), With<Nanobot>>()
            .iter(world)
            .filter_map(|(entity, member, kind)| {
                (member.0 == opponent_id && *kind == NanobotType::Defender).then_some(entity)
            })
            .collect::<Vec<_>>()
    };
    opponent_defenders.sort_by_key(|entity| entity.to_bits());
    for entity in opponent_defenders.into_iter().take(1) {
        app.world_mut()
            .entity_mut(entity)
            .get_mut::<Charge>()
            .unwrap()
            .current = 0.5;
    }

    (app, front, player_charger, opponent_charger)
}

#[test]
fn default_front_has_readable_combat_and_staggered_sustain() {
    let (mut app, front, player_charger, opponent_charger) = spawn_runtime_front();
    let initial_player_health = aggregate_defender_health(app.world_mut(), SwarmId::PLAYER);
    let initial_opponent_health = aggregate_defender_health(app.world_mut(), OPPONENT_SWARM);
    let mut previous_positions = positions(app.world_mut());
    let mut contact_tick = None;
    let mut checked_survival_window = false;
    let mut saw_both_participants = false;

    for tick in 0..(MAX_CONTACT_TICKS + MAX_TICKS_AFTER_CONTACT) {
        let rotations = RotationSnapshot::capture(app.world_mut());
        app.update();

        let current_positions = positions(app.world_mut());
        for (entity, position) in &current_positions {
            assert!(position.is_finite());
            if let Some(previous) = previous_positions.get(entity) {
                let displacement = position.distance(*previous);
                assert!(
                    displacement <= 5.25 + MOVEMENT_DISTANCE_TOLERANCE,
                    "nanobot {entity:?} exceeded fixed-tick speed limit: {displacement}"
                );
            }
        }
        previous_positions = current_positions;
        assert_charge_bounds(app.world_mut());

        let loads = charger_loads(app.world_mut());
        for charger in [player_charger, opponent_charger] {
            assert!(loads.get(&charger).copied().unwrap_or_default() <= 3);
        }
        rotations.assert_admissions(app.world_mut());

        let player_participants = participants_in_front(app.world_mut(), front, SwarmId::PLAYER);
        let opponent_participants = participants_in_front(app.world_mut(), front, OPPONENT_SWARM);
        if player_participants > 0 && opponent_participants > 0 {
            saw_both_participants = true;
        }

        if contact_tick.is_none()
            && (aggregate_defender_health(app.world_mut(), SwarmId::PLAYER) < initial_player_health
                || aggregate_defender_health(app.world_mut(), OPPONENT_SWARM)
                    < initial_opponent_health)
        {
            contact_tick = Some(tick);
        }

        if let Some(contact) = contact_tick {
            if tick == contact + 180 {
                assert!(live_defenders(app.world_mut(), SwarmId::PLAYER) > 0);
                assert!(live_defenders(app.world_mut(), OPPONENT_SWARM) > 0);
                checked_survival_window = true;
            }

            if tick == contact + MAX_TICKS_AFTER_CONTACT {
                break;
            }
        }
    }

    assert!(contact_tick.is_some(), "authored fronts never made contact");
    assert!(
        contact_tick.unwrap() <= MAX_CONTACT_TICKS,
        "front contact arrived too late"
    );
    assert!(checked_survival_window, "survival window was not reached");
    assert!(
        saw_both_participants,
        "front never established physical participants from both swarms"
    );
    assert_eq!(
        *app.world().resource::<MatchOutcome>(),
        MatchOutcome::InProgress,
        "combat feel flow must not collapse either production side"
    );
    let collapse = app.world().resource::<ProductionCollapseState>();
    assert!(!collapse.player_collapsed);
    assert!(!collapse.opponent_collapsed);
    assert!(live_defenders(app.world_mut(), SwarmId::PLAYER) > 0);
}

#[test]
fn authored_starting_facilities_align_without_changing_deposit_geometry() {
    use top_down_2d_rts_prototype_nano_swarm::{
        nanobot::ProductionFacility, resources::ResourceDeposit,
    };
    let mut app = default_headless_app();
    app.update();
    let world = app.world_mut();
    let facilities = world
        .query_filtered::<&Transform, With<ProductionFacility>>()
        .iter(world)
        .collect::<Vec<_>>();
    assert_eq!(facilities.len(), 2);
    for transform in facilities {
        let size = transform.scale.truncate() * 64.0;
        assert!((size - Vec2::splat(216.0)).length() < 0.001);
        let min = transform.translation.truncate() - size / 2.0;
        for edge in [min.x, min.y, min.x + 216.0, min.y + 216.0] {
            assert!((edge / 72.0 - (edge / 72.0).round()).abs() < 0.001);
        }
    }
    let deposits = world
        .query::<(&ResourceDeposit, &Transform)>()
        .iter(world)
        .collect::<Vec<_>>();
    assert_eq!(deposits.len(), 4);
    for (deposit, transform) in deposits {
        assert!((deposit.radius - 64.0).abs() < 0.001);
        assert!((transform.scale.truncate() - Vec2::splat(2.0)).length() < 0.001);
        let offset = transform.translation.truncate() - Vec2::splat(256.0);
        assert!((offset / 512.0 - (offset / 512.0).round()).length() < 0.001);
    }
}
