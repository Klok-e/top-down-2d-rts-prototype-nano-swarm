#[path = "../common/mod.rs"]
mod common;

use approx::assert_abs_diff_eq;
use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    battle_experiment::{BattleExperimentConfig, PacingId},
    gameplay_pacing::GameplayPacing,
    nanobot::{
        Charge, ChargerAssignment, CombatPlugin, DefenderResponse, Health, LogisticsReservation,
        NanobotSimulationSet, OwnerSwarm, PlannedKind, PlannedStructure, PlannedStructureProgress,
        Swarm, SwarmId, SwarmMember,
    },
    scenario_selection::{Scenario, ScenarioSelection},
};

#[derive(Resource)]
struct AttackObservations {
    tick: u32,
    targets: [Entity; 2],
    previous_health: [u32; 2],
    hit_ticks: [Vec<u32>; 2],
}

fn observe_attack_ticks(mut observations: ResMut<AttackObservations>, health: Query<&Health>) {
    observations.tick += 1;
    for index in 0..observations.targets.len() {
        let current = health.get(observations.targets[index]).unwrap().current;
        if current < observations.previous_health[index] {
            let tick = observations.tick;
            observations.hit_ticks[index].push(tick);
            observations.previous_health[index] = current;
        }
    }
}

#[test]
fn deliberate_construction_uses_one_real_budget_for_both_swarms() {
    let mut app = common::sim_app_with_planned();
    let pacing = GameplayPacing::from(PacingId::Deliberate);
    app.update();
    app.insert_resource(pacing.clone());
    let player = app.world_mut().spawn((Swarm {}, SwarmId::PLAYER)).id();
    let opponent_id = SwarmId(11);
    let opponent = app.world_mut().spawn((Swarm {}, opponent_id)).id();

    let mut plans = Vec::new();
    for (index, (owner, swarm)) in [(player, SwarmId::PLAYER), (opponent, opponent_id)]
        .into_iter()
        .enumerate()
    {
        let cell = IVec2::new(index as i32 * 3, 0);
        let center = common::cell_world_center(cell);
        let plan = app
            .world_mut()
            .spawn((
                PlannedStructure::new(PlannedKind::SourceStockpile, cell)
                    .with_work_budget(pacing.construction_work_ticks),
                OwnerSwarm(owner),
                Transform::from_translation(center.extend(0.0)),
            ))
            .id();
        let worker = common::spawn_worker_at(&mut app, center + Vec2::X * 68.0);
        app.world_mut().entity_mut(worker).insert((
            SwarmMember::new(swarm),
            PlannedStructureProgress { cell, target: plan },
        ));
        plans.push(plan);
    }

    for plan in plans.iter().copied() {
        assert_eq!(
            app.world()
                .entity(plan)
                .get::<PlannedStructure>()
                .unwrap()
                .construction_progress(),
            (0, 90),
        );
    }

    for _ in 0..30 {
        app.update();
    }

    for plan in plans {
        let plan = app.world().entity(plan).get::<PlannedStructure>().unwrap();
        assert_eq!(plan.construction_progress(), (30, 90));
        assert_eq!(plan.available_work(), 60);
    }
}

#[test]
fn deliberate_attack_interval_is_shared_by_both_swarms() {
    let mut app = common::sim_app_with_movement();
    app.add_plugins(CombatPlugin);
    let pacing = GameplayPacing::from(PacingId::Deliberate);
    app.update();
    app.insert_resource(pacing.clone());
    app.world_mut().spawn((Swarm {}, SwarmId::PLAYER));
    let opponent_id = SwarmId(11);
    app.world_mut().spawn((Swarm {}, opponent_id));

    let player_cell = IVec2::ZERO;
    let opponent_cell = IVec2::new(3, 0);
    let player_center = common::cell_world_center(player_cell);
    let opponent_center = common::cell_world_center(opponent_cell);
    let player_attacker = common::spawn_defender_at(&mut app, player_center - Vec2::X * 16.0);
    let player_target = common::spawn_worker_at(&mut app, player_center + Vec2::X * 16.0);
    let opponent_attacker = common::spawn_defender_at(&mut app, opponent_center - Vec2::X * 16.0);
    let opponent_target = common::spawn_worker_at(&mut app, opponent_center + Vec2::X * 16.0);
    for entity in [opponent_attacker, player_target] {
        app.world_mut()
            .entity_mut(entity)
            .insert(SwarmMember::new(opponent_id));
    }
    app.world_mut()
        .entity_mut(player_attacker)
        .insert(DefenderResponse {
            target: player_target,
        });
    app.world_mut()
        .entity_mut(opponent_attacker)
        .insert(DefenderResponse {
            target: opponent_target,
        });

    app.insert_resource(AttackObservations {
        tick: 0,
        targets: [player_target, opponent_target],
        previous_health: [100; 2],
        hit_ticks: [Vec::new(), Vec::new()],
    });
    app.add_systems(
        FixedUpdate,
        observe_attack_ticks.after(NanobotSimulationSet::Combat),
    );

    for _ in 0..40 {
        app.update();
        if app
            .world()
            .resource::<AttackObservations>()
            .hit_ticks
            .iter()
            .all(|ticks| ticks.len() >= 2)
        {
            break;
        }
    }
    let observations = app.world().resource::<AttackObservations>();
    for hit_ticks in &observations.hit_ticks {
        assert!(hit_ticks.len() >= 2, "both swarms must deliver two attacks");
        assert_eq!(
            hit_ticks[1] - hit_ticks[0],
            u32::from(pacing.attack_interval_ticks),
        );
    }
}

#[test]
fn deliberate_charge_drain_is_shared_by_both_swarms() {
    let mut app = common::sim_app_with_charge();
    let pacing = GameplayPacing::from(PacingId::Deliberate);
    app.update();
    app.insert_resource(pacing.clone());
    let player = common::spawn_defender_at(&mut app, Vec2::ZERO);
    let opponent = common::spawn_defender_at(&mut app, Vec2::X * 300.0);
    app.world_mut()
        .entity_mut(opponent)
        .insert(SwarmMember::new(SwarmId(11)));

    app.update();

    let expected = 0.999875;
    for defender in [player, opponent] {
        let actual = app
            .world()
            .entity(defender)
            .get::<Charge>()
            .unwrap()
            .current;
        assert_abs_diff_eq!(actual, expected, epsilon = 1e-6);
    }
}

#[test]
fn deliberate_charge_drain_sets_the_actual_charger_delivery_demand() {
    let mut app = common::sim_app_with_gather_haul();
    app.insert_resource(BattleExperimentConfig {
        pacing: PacingId::Deliberate,
        ..default()
    });
    app.world_mut().resource_mut::<ScenarioSelection>().current = Scenario::AiBattle;
    app.insert_resource(GameplayPacing::from(PacingId::Deliberate));
    let swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let source = common::spawn_sink_stockpile(&mut app, Vec2::new(-200.0, 0.0), 100, 100);
    app.world_mut().entity_mut(source).insert(OwnerSwarm(swarm));
    let charger = common::spawn_operational_charger_at(&mut app, IVec2::ZERO, 1);
    app.world_mut()
        .entity_mut(charger)
        .insert(OwnerSwarm(swarm));
    let defender = common::spawn_defender_at(&mut app, Vec2::new(20.0, 0.0));
    app.world_mut().entity_mut(defender).insert((
        Charge {
            current: 0.47,
            max: 1.0,
        },
        ChargerAssignment { charger },
    ));
    let hauler = common::spawn_hauler_at(&mut app, Vec2::new(-600.0, -200.0));

    for _ in 0..30 {
        app.update();
        if app.world().get::<LogisticsReservation>(hauler).is_some() {
            break;
        }
    }

    let reservation = *app
        .world()
        .entity(hauler)
        .get::<LogisticsReservation>()
        .expect("empty assigned Defender creates reservable Charger demand");
    assert_eq!(reservation.source, source);
    assert_eq!(reservation.destination, charger);
    assert_eq!(
        reservation.amount, 18,
        "Deliberate refill pulses must reserve their actual mineral demand",
    );
}

#[test]
fn gameplay_plugins_install_the_baseline_when_no_profile_is_supplied() {
    let combat = common::sim_app_with_combat();
    assert_eq!(
        combat
            .world()
            .resource::<GameplayPacing>()
            .attack_interval_ticks,
        15,
    );
    let charge = common::sim_app_with_charge();
    assert_abs_diff_eq!(
        charge
            .world()
            .resource::<GameplayPacing>()
            .charge_drain_per_tick,
        0.00025,
    );
    let planned = common::sim_app_with_planned();
    assert_eq!(
        planned
            .world()
            .resource::<GameplayPacing>()
            .construction_work_ticks,
        5,
    );
}
