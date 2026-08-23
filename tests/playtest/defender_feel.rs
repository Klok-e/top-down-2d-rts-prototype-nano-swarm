//! Near-runtime Defender feel flow: authored-front travel, readable combat,
//! and staggered local recharge.

use std::collections::HashMap;

use bevy::{asset::AssetPlugin, math::Vec2, prelude::*};
use top_down_2d_rts_prototype_nano_swarm::{
    ai::AiPlugin,
    intent::{IntentGrid, IntentKind},
    nanobot::{
        Charge, ChargePlugin, ChargerAssignment, CollapsePlugin, CombatPlugin, DefendHold,
        DefendPlugin, GatherPlugin, HaulPlugin, Health, MaintenancePlugin, MatchOutcome, Nanobot,
        NanobotPlugin, NanobotType, OpponentIntentPlugin, OpponentSwarmIdAlloc, OwnerSwarm,
        PlannedStructurePlugin, PopulationDemandPlugin, ProductionCollapseState, ProductionPlugin,
        ProductionPriority, RegionalAllocationPlugin, SwarmId, SwarmMember,
        nanobot_death_cleanup_system,
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
    app.add_plugins(TaskPoolPlugin::default())
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
        .add_plugins(DefendPlugin)
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

    for _ in 0..301 {
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
        MatchOutcome::Victory,
        "authored default bootstrap must deterministically latch the opponent's terminal result"
    );

    let initial_player_health = aggregate_defender_health(app.world_mut(), SwarmId::PLAYER);
    let initial_opponent_health = aggregate_defender_health(app.world_mut(), SwarmId(1));
    let mut previous_positions = positions(app.world_mut());
    let mut saw_both_holders = false;
    for _ in 0..301 {
        app.update();
        previous_positions =
            assert_default_tick_state(app.world_mut(), &previous_positions, &mut saw_both_holders);
    }
    for tick in 0..900 {
        app.update();
        previous_positions =
            assert_default_tick_state(app.world_mut(), &previous_positions, &mut saw_both_holders);
        if tick == 179 {
            assert!(
                live_defenders(app.world_mut(), SwarmId::PLAYER) > 0,
                "authored default player front must retain a live Defender after contact +180"
            );
            assert!(
                live_defenders(app.world_mut(), SwarmId(1)) > 0,
                "authored default opponent front must retain a live Defender after contact +180"
            );
        }
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
        MatchOutcome::Victory
    );
    assert!(
        saw_both_holders,
        "default fronts never established both holders"
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

fn holders_in_front(world: &mut World, cell: IVec2, swarm: SwarmId) -> usize {
    world
        .query::<(&DefendHold, &SwarmMember)>()
        .iter(world)
        .filter(|(hold, member)| hold.cell == cell && member.0 == swarm)
        .count()
}

fn assert_default_tick_state(
    world: &mut World,
    previous_positions: &HashMap<Entity, Vec2>,
    saw_both_holders: &mut bool,
) -> HashMap<Entity, Vec2> {
    let current_positions = positions(world);
    for (entity, position) in &current_positions {
        assert!(position.is_finite());
        if let Some(previous) = previous_positions.get(entity) {
            let displacement = position.distance(*previous);
            assert!(
                displacement <= common::default_game_settings().bot_speed + 1e-4,
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

    let player_holders = holders_in_front(world, PLAYER_DEFEND_CELL, SwarmId::PLAYER);
    let opponent_holders = holders_in_front(world, PLAYER_DEFEND_CELL, SwarmId(1));
    if player_holders > 0 && opponent_holders > 0 {
        *saw_both_holders = true;
    }
    if *saw_both_holders {
        if live_defenders(world, SwarmId::PLAYER) > 0 {
            assert!(player_holders > 0, "player lost its authored Defend holder");
        }
        if live_defenders(world, SwarmId(1)) > 0 {
            assert!(
                opponent_holders > 0,
                "opponent lost its authored Defend holder"
            );
        }
    }
    current_positions
}

fn spawn_runtime_front() -> (App, IVec2, Entity, Entity) {
    let mut app = common::sim_app_with_charge();
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
        grid.paint_owned(front, IntentKind::Defend, Some(opponent_id));
        grid.contest_defend(front, SwarmId::PLAYER);
    }

    let player_charger = common::spawn_charger_at(&mut app, front, 60);
    app.world_mut()
        .entity_mut(player_charger)
        .insert(OwnerSwarm(player_swarm));
    let opponent_charger = common::spawn_charger_at(&mut app, front, 60);
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
    let mut saw_holders = false;

    for tick in 0..(MAX_CONTACT_TICKS + MAX_TICKS_AFTER_CONTACT) {
        app.update();

        let current_positions = positions(app.world_mut());
        for (entity, position) in &current_positions {
            assert!(position.is_finite());
            if let Some(previous) = previous_positions.get(entity) {
                let displacement = position.distance(*previous);
                assert!(
                    displacement <= common::default_game_settings().bot_speed + 1e-4,
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

        let player_holders = holders_in_front(app.world_mut(), front, SwarmId::PLAYER);
        let opponent_holders = holders_in_front(app.world_mut(), front, OPPONENT_SWARM);
        let player_live = live_defenders(app.world_mut(), SwarmId::PLAYER);
        let opponent_live = live_defenders(app.world_mut(), OPPONENT_SWARM);
        if player_holders > 0 && opponent_holders > 0 {
            saw_holders = true;
        }
        if saw_holders {
            if player_live > 0 {
                assert!(
                    player_holders > 0,
                    "player cohort fully evacuated the front"
                );
            }
            if opponent_live > 0 {
                assert!(
                    opponent_holders > 0,
                    "opponent cohort fully evacuated the front"
                );
            }
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
                let owner = app
                    .world()
                    .resource::<IntentGrid>()
                    .cell(front)
                    .expect("front cell")
                    .owner(IntentKind::Defend);
                assert_eq!(owner, Some(SwarmId::PLAYER));
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
    assert!(saw_holders, "front never established active holders");
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
