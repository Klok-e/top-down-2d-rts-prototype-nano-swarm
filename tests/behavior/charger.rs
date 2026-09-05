//! Integration tests for issue #14: Chargers and defender
//! Charge sustain loop.
//!
//! Each test isolates one behavior so a failure points at a
//! single contract: Charger planning from unmet low Charge,
//! logistics dependence (a charger without material is not a
//! working rotation target), weakening of attack/defense on
//! low charge, health loss on empty/ignored charge, and the
//! automatic rotation of defenders to working chargers.
//!
//! The pure-helper unit tests (charge helpers, multipliers,
//! Charger data) live in `src/nanobot/charge.rs`.

use std::time::Duration;

use bevy::{math::Vec2, prelude::*, time::TimeUpdateStrategy};
use top_down_2d_rts_prototype_nano_swarm::{
    ai::AiPlugin,
    intent::{IntentGrid, IntentKind},
    nanobot::{
        CHARGE_DRAIN_PER_TICK, CHARGE_PER_PULSE, CHARGE_PULSE_INTERVAL_TICKS,
        CHARGER_MATERIAL_PER_PULSE, Cargo, Charge, Charger, ChargerAssignment, ChargerProgress,
        ChargerPulseProgress, DefenderResponse, DirectMovementComponent,
        EMPTY_CHARGE_DAMAGE_INTERVAL_TICKS, EMPTY_CHARGE_HEALTH_DAMAGE, Health,
        LOW_CHARGE_THRESHOLD, LogisticsReservation, MAX_CHARGE, NANOBOT_DEFAULT_MAX_HEALTH,
        Nanobot, NanobotBundle, NanobotPlugin, NanobotType, OwnerSwarm, PlannedKind,
        PlannedStructure, SUPPORT_OPERATIONAL_HEALTH_THRESHOLD, Structure, StructureKind, Swarm,
        SwarmBundle, SwarmId, SwarmMember, defender_charger_arrive_system,
        defender_charger_work_system, defender_rotation_to_charger_system,
        nanobot_death_cleanup_system,
    },
    resources::{ResourceKind, ResourceLedger},
};

#[path = "../common/mod.rs"]
mod common;

fn build_app() -> App {
    // Issue #28: Chargers emerge as Planned Structures and
    // are built by a Worker. The behaviour tests that
    // exercise the demand side need the planned-structure
    // plugin loaded so the auto-creation system can spawn
    // a plan and a Worker can build it. The
    // `sim_app_with_charge_planned` seam bundles both
    // plugins in the order the production code wires them.
    common::sim_app_with_charge_planned()
}

fn charger_count(world: &mut World) -> usize {
    let mut q = world.query::<&Charger>();
    q.iter(world).count()
}

fn planned_charger_count(world: &mut World) -> usize {
    let mut q = world.query::<&PlannedStructure>();
    q.iter(world)
        .filter(|p| p.kind == PlannedKind::Charger)
        .count()
}

fn read_charge(app: &App, defender: Entity) -> Option<f32> {
    app.world()
        .entity(defender)
        .get::<Charge>()
        .map(|c| c.current)
}

fn read_health(app: &App, defender: Entity) -> Option<u32> {
    app.world()
        .entity(defender)
        .get::<Health>()
        .map(|h| h.current)
}

#[test]
fn full_defender_remains_on_duty_until_field_endurance_threshold() {
    let mut app = build_app();
    common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let cell = IVec2::ZERO;
    app.world_mut()
        .resource_mut::<IntentGrid>()
        .paint(cell, IntentKind::Defend, SwarmId::PLAYER);
    let defender = common::spawn_defender_at(&mut app, common::cell_world_center(cell));

    for _ in 0..1_999 {
        app.update();
    }
    assert!(app.world().entities().contains(defender));
    assert!(
        app.world()
            .entity(defender)
            .get::<Charge>()
            .unwrap()
            .current
            > LOW_CHARGE_THRESHOLD,
        "full Defender must stay above rotation threshold through tick 1,999",
    );

    app.update();
    assert!(
        app.world()
            .entity(defender)
            .get::<Charge>()
            .unwrap()
            .current
            > LOW_CHARGE_THRESHOLD,
        "f32 drain boundary remains above threshold at tick 2,000",
    );
    app.update();
    assert!(
        app.world()
            .entity(defender)
            .get::<Charge>()
            .unwrap()
            .current
            <= LOW_CHARGE_THRESHOLD,
        "rotation threshold must be reached on tick 2,001",
    );
}

#[test]
fn low_defender_recharges_in_readable_bounded_time() {
    let mut app = build_app();
    let swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let cell = IVec2::ZERO;
    app.world_mut()
        .resource_mut::<IntentGrid>()
        .paint(cell, IntentKind::Defend, SwarmId::PLAYER);
    let charger = common::spawn_operational_charger_at(&mut app, cell, 60);
    app.world_mut()
        .entity_mut(charger)
        .insert(OwnerSwarm(swarm));
    let defender =
        common::spawn_defender_at(&mut app, common::cell_world_center(cell) + Vec2::X * 68.0);
    app.world_mut().entity_mut(defender).insert((
        ChargerAssignment { charger },
        ChargerProgress { charger },
        ChargerPulseProgress::default(),
    ));
    app.world_mut()
        .entity_mut(defender)
        .get_mut::<Charge>()
        .unwrap()
        .current = LOW_CHARGE_THRESHOLD;

    for _ in 0..180 {
        app.update();
    }
    assert!(
        app.world()
            .entity(defender)
            .get::<Charge>()
            .unwrap()
            .current
            < MAX_CHARGE,
        "recharge must not finish before 180 fixed ticks",
    );

    let mut reached_full = false;
    for _ in 0..120 {
        app.update();
        if (app
            .world()
            .entity(defender)
            .get::<Charge>()
            .unwrap()
            .current
            - MAX_CHARGE)
            .abs()
            < 1e-6
        {
            reached_full = true;
            break;
        }
    }
    assert!(reached_full, "recharge must finish by fixed tick 300");
}

#[test]
fn normal_rotation_consumes_exact_minerals() {
    let mut app = build_app();
    let swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let cell = IVec2::ZERO;
    app.world_mut()
        .resource_mut::<IntentGrid>()
        .paint(cell, IntentKind::Defend, SwarmId::PLAYER);
    let charger = common::spawn_operational_charger_at(&mut app, cell, 60);
    app.world_mut()
        .entity_mut(charger)
        .insert(OwnerSwarm(swarm));
    app.world_mut().resource_mut::<ResourceLedger>().add_for(
        SwarmId::PLAYER,
        ResourceKind::Minerals,
        60,
    );
    let defender =
        common::spawn_defender_at(&mut app, common::cell_world_center(cell) + Vec2::X * 68.0);
    app.world_mut().entity_mut(defender).insert((
        ChargerAssignment { charger },
        ChargerProgress { charger },
        ChargerPulseProgress::default(),
    ));
    app.world_mut()
        .entity_mut(defender)
        .get_mut::<Charge>()
        .unwrap()
        .current = LOW_CHARGE_THRESHOLD;

    for _ in 0..190 {
        app.update();
    }

    assert_eq!(
        app.world().entity(charger).get::<Charger>().unwrap().amount,
        41,
        "0.5-to-full rotation must consume exactly 19 supplied pulses",
    );
    assert_eq!(
        app.world()
            .resource::<ResourceLedger>()
            .total_for(SwarmId::PLAYER, ResourceKind::Minerals,),
        41,
    );
    assert!(
        (app.world()
            .entity(defender)
            .get::<Charge>()
            .unwrap()
            .current
            - MAX_CHARGE)
            .abs()
            < 1e-6
    );
}

#[test]
fn empty_unsupported_defender_dies_after_grace_period() {
    let mut app = build_app();
    common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let cell = IVec2::ZERO;
    app.world_mut().resource_mut::<IntentGrid>().paint(
        cell,
        IntentKind::Defend,
        top_down_2d_rts_prototype_nano_swarm::nanobot::SwarmId::PLAYER,
    );
    let defender = common::spawn_defender_at(&mut app, common::cell_world_center(cell));
    app.world_mut()
        .entity_mut(defender)
        .get_mut::<Charge>()
        .unwrap()
        .current = 0.0;

    for _ in 0..5 {
        app.update();
    }
    assert_eq!(
        read_health(&app, defender),
        Some(NANOBOT_DEFAULT_MAX_HEALTH)
    );

    app.update();
    assert_eq!(
        read_health(&app, defender),
        Some(NANOBOT_DEFAULT_MAX_HEALTH - 1)
    );
    for _ in 0..594 {
        app.update();
    }
    assert!(read_health(&app, defender).is_none_or(|health| health == 0));
}

#[test]
fn swarm_rotation_cap_counts_defenders_across_staging_cells() {
    let mut app = build_app();
    let swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    for index in -3..3 {
        let cell = IVec2::new(index, 0);
        app.world_mut().resource_mut::<IntentGrid>().paint(
            cell,
            IntentKind::Defend,
            SwarmId::PLAYER,
        );
        let charger = common::spawn_operational_charger_at(&mut app, cell, 100);
        app.world_mut()
            .entity_mut(charger)
            .insert(OwnerSwarm(swarm));
        let defender =
            common::spawn_defender_at(&mut app, common::cell_world_center(cell) + Vec2::X * 68.0);
        app.world_mut()
            .entity_mut(defender)
            .get_mut::<Charge>()
            .unwrap()
            .current = LOW_CHARGE_THRESHOLD;
    }

    wait_for_rotations(&mut app, 3);

    let world = app.world_mut();
    let mut assignments = world.query::<&ChargerAssignment>();
    assert_eq!(assignments.iter(world).count(), 3);
}

#[test]
fn casualty_preserves_accepted_travel_and_charging_until_completion() {
    let mut app = common::sim_app_with_movement();
    app.add_systems(
        Update,
        (
            defender_rotation_to_charger_system,
            defender_charger_arrive_system,
            defender_charger_work_system,
        )
            .chain(),
    );
    let swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let mut accepted = Vec::new();
    for (cell, distance) in [
        (IVec2::new(-2, 0), 68.0),
        (IVec2::ZERO, 400.0),
        (IVec2::new(2, 0), 400.0),
    ] {
        app.world_mut().resource_mut::<IntentGrid>().paint(
            cell,
            IntentKind::Defend,
            SwarmId::PLAYER,
        );
        let charger = common::spawn_operational_charger_at(&mut app, cell, 100);
        app.world_mut()
            .entity_mut(charger)
            .insert(OwnerSwarm(swarm));
        let defender = common::spawn_defender_at(
            &mut app,
            common::cell_world_center(cell) + Vec2::Y * distance,
        );
        app.world_mut()
            .entity_mut(defender)
            .get_mut::<Charge>()
            .unwrap()
            .current = 0.5;
        accepted.push(defender);
    }
    let casualty = common::spawn_defender_at(&mut app, Vec2::new(-1000.0, -1000.0));
    let waiting = common::spawn_defender_at(&mut app, Vec2::new(0.0, -1000.0));
    common::spawn_defender_at(&mut app, Vec2::new(1000.0, -1000.0));

    wait_for_rotations(&mut app, 3);
    app.update();
    for &defender in &accepted {
        assert!(app.world().entity(defender).contains::<ChargerAssignment>());
    }
    assert!(
        app.world()
            .entity(accepted[0])
            .contains::<ChargerProgress>()
    );
    assert!(
        !app.world()
            .entity(accepted[1])
            .contains::<ChargerProgress>()
    );
    assert!(
        app.world()
            .entity(accepted[1])
            .contains::<DirectMovementComponent>()
    );

    app.world_mut().despawn(casualty);
    app.world_mut()
        .entity_mut(waiting)
        .get_mut::<Charge>()
        .unwrap()
        .current = 0.5;
    let mut completed = [false; 3];
    let mut waited_at_capacity = false;
    for _ in 0..400 {
        let active_before = accepted
            .iter()
            .filter(|&&defender| app.world().entity(defender).contains::<ChargerAssignment>())
            .count();
        let waiting_before = app.world().entity(waiting).contains::<ChargerAssignment>();
        app.update();
        if active_before >= 2 && !waiting_before {
            waited_at_capacity |= active_before == 2;
            assert!(
                !app.world().entity(waiting).contains::<ChargerAssignment>(),
                "five living Defenders cannot admit another rotation with two or more active",
            );
        }
        for (index, &defender) in accepted.iter().enumerate() {
            if !app.world().entity(defender).contains::<ChargerAssignment>() {
                assert!(
                    (read_charge(&app, defender).unwrap() - 1.0).abs() < 1e-6,
                    "an accepted trip or recharge must finish, not be cancelled by the casualty",
                );
                completed[index] = true;
            }
        }
        if completed == [true; 3] {
            break;
        }
    }
    assert_eq!(completed, [true; 3], "all accepted rotations must finish");
    assert!(
        waited_at_capacity,
        "new admissions must also wait at exactly two active rotations"
    );
    for _ in 0..120 {
        app.update();
        if app.world().get::<ChargerAssignment>(waiting).is_some() {
            break;
        }
    }
    assert!(
        app.world().entity(waiting).contains::<ChargerAssignment>(),
        "fresh admission resumes once fewer than two rotations remain",
    );
}

#[test]
fn destroyed_charger_ends_rotation_without_grandfathering_replacement_admission() {
    let mut app = common::sim_app_with_movement();
    app.add_systems(
        Update,
        (
            defender_rotation_to_charger_system,
            defender_charger_arrive_system,
            defender_charger_work_system,
        )
            .chain(),
    );
    let swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let mut accepted = Vec::new();
    let mut chargers = Vec::new();
    for cell in [IVec2::new(-2, 0), IVec2::ZERO, IVec2::new(2, 0)] {
        app.world_mut().resource_mut::<IntentGrid>().paint(
            cell,
            IntentKind::Defend,
            SwarmId::PLAYER,
        );
        let charger = common::spawn_operational_charger_at(&mut app, cell, 100);
        app.world_mut()
            .entity_mut(charger)
            .insert(OwnerSwarm(swarm));
        chargers.push(charger);
        let defender =
            common::spawn_defender_at(&mut app, common::cell_world_center(cell) + Vec2::Y * 68.0);
        app.world_mut()
            .entity_mut(defender)
            .get_mut::<Charge>()
            .unwrap()
            .current = 0.5;
        accepted.push(defender);
    }
    let casualty = common::spawn_defender_at(&mut app, Vec2::new(-1000.0, -1000.0));
    common::spawn_defender_at(&mut app, Vec2::new(0.0, -1000.0));
    common::spawn_defender_at(&mut app, Vec2::new(1000.0, -1000.0));
    wait_for_rotations(&mut app, 3);
    app.update();
    for (&defender, &charger) in accepted.iter().zip(&chargers) {
        assert_eq!(
            app.world()
                .entity(defender)
                .get::<ChargerAssignment>()
                .unwrap()
                .charger,
            charger
        );
        assert!(app.world().entity(defender).contains::<ChargerProgress>());
    }

    app.world_mut().despawn(casualty);
    app.world_mut().despawn(chargers[0]);
    for _ in 0..3 {
        app.update();
        let released = app.world().entity(accepted[0]);
        assert!(!released.contains::<ChargerAssignment>());
        assert!(!released.contains::<ChargerProgress>());
        assert!(!released.contains::<ChargerPulseProgress>());
        assert!(!released.contains::<DirectMovementComponent>());
        assert!(read_charge(&app, accepted[0]).unwrap() < 0.51);
    }
    for &defender in &accepted[1..] {
        assert!(app.world().entity(defender).contains::<ChargerAssignment>());
    }

    app.world_mut()
        .entity_mut(accepted[1])
        .get_mut::<Charge>()
        .unwrap()
        .current = 0.99;
    for _ in 0..12 {
        app.update();
    }
    assert!(
        !app.world()
            .entity(accepted[1])
            .contains::<ChargerAssignment>()
    );
    let replacement = app
        .world()
        .entity(accepted[0])
        .get::<ChargerAssignment>()
        .expect("the released Defender can seek another Charger after capacity opens");
    assert_ne!(replacement.charger, chargers[0]);
}

#[test]
fn swarm_rotation_keeps_half_of_defenders_on_duty() {
    let mut app = build_app();
    let swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let cell = IVec2::ZERO;
    app.world_mut()
        .resource_mut::<IntentGrid>()
        .paint(cell, IntentKind::Defend, SwarmId::PLAYER);
    let charger = common::spawn_operational_charger_at(&mut app, cell, 100);
    app.world_mut()
        .entity_mut(charger)
        .insert(OwnerSwarm(swarm));
    let center = common::cell_world_center(cell);
    for index in 0..3 {
        let defender = common::spawn_defender_at(
            &mut app,
            center + Vec2::new(100.0, (index as f32 - 1.0) * 72.0),
        );
        app.world_mut()
            .entity_mut(defender)
            .get_mut::<Charge>()
            .unwrap()
            .current = LOW_CHARGE_THRESHOLD;
    }

    wait_for_rotations(&mut app, 1);

    let world = app.world_mut();
    let mut assignments = world.query::<&ChargerAssignment>();
    assert_eq!(
        assignments
            .iter(world)
            .filter(|assignment| assignment.charger == charger)
            .count(),
        1,
    );
}

#[test]
fn material_is_not_overdrawn_under_charger_contention() {
    let mut app = App::new();
    app.insert_resource(IntentGrid::new(4, 4))
        .init_resource::<ResourceLedger>()
        .add_systems(Update, defender_charger_work_system);
    let cell = IVec2::ZERO;
    app.world_mut()
        .resource_mut::<IntentGrid>()
        .paint(cell, IntentKind::Defend, SwarmId::PLAYER);
    let swarm = app.world_mut().spawn(SwarmBundle::default()).id();
    let mut charger_state = Charger::new(cell);
    charger_state.amount = CHARGER_MATERIAL_PER_PULSE;
    let charger = app
        .world_mut()
        .spawn((
            charger_state,
            OwnerSwarm(swarm),
            Structure::new(StructureKind::Basic),
            Transform::default(),
        ))
        .id();
    app.world_mut().resource_mut::<ResourceLedger>().add_for(
        SwarmId::PLAYER,
        ResourceKind::Minerals,
        CHARGER_MATERIAL_PER_PULSE,
    );
    for _ in 0..2 {
        app.world_mut().spawn((
            Nanobot {},
            Transform::from_xyz(68.0, 0.0, 0.0),
            NanobotType::Defender,
            SwarmMember::new(SwarmId::PLAYER),
            Charge {
                current: 0.5,
                max: MAX_CHARGE,
            },
            ChargerAssignment { charger },
            ChargerProgress { charger },
            ChargerPulseProgress {
                ticks_elapsed: CHARGE_PULSE_INTERVAL_TICKS - 1,
            },
        ));
    }

    app.update();

    assert_eq!(
        app.world().entity(charger).get::<Charger>().unwrap().amount,
        0,
        "one mineral must satisfy at most one concurrent pulse"
    );
    assert_eq!(
        app.world()
            .resource::<ResourceLedger>()
            .total_for(SwarmId::PLAYER, ResourceKind::Minerals,),
        0,
        "ledger debit must match the single consumed mineral"
    );
    let mut charges = app.world_mut().query::<&Charge>();
    let charged = charges
        .iter(app.world())
        .filter(|charge| charge.current > 0.5)
        .count();
    assert_eq!(
        charged, 1,
        "only one defender may receive the supplied pulse"
    );
}

#[test]
fn charger_pulse_recipient_is_stable_across_spawn_order() {
    fn charged_entity_rank(reverse_spawn_order: bool) -> usize {
        let mut app = App::new();
        app.insert_resource(IntentGrid::new(4, 4))
            .init_resource::<ResourceLedger>()
            .add_systems(Update, defender_charger_work_system);
        let cell = IVec2::ZERO;
        app.world_mut().resource_mut::<IntentGrid>().paint(
            cell,
            IntentKind::Defend,
            SwarmId::PLAYER,
        );
        let swarm = app.world_mut().spawn(SwarmBundle::default()).id();
        let mut charger_state = Charger::new(cell);
        charger_state.amount = CHARGER_MATERIAL_PER_PULSE;
        let charger = app
            .world_mut()
            .spawn((
                charger_state,
                OwnerSwarm(swarm),
                Structure::new(StructureKind::Basic),
                Transform::default(),
            ))
            .id();
        app.world_mut().resource_mut::<ResourceLedger>().add_for(
            SwarmId::PLAYER,
            ResourceKind::Minerals,
            CHARGER_MATERIAL_PER_PULSE,
        );

        let spawn_order = if reverse_spawn_order { [1, 0] } else { [0, 1] };
        let mut defenders_by_label = [Entity::PLACEHOLDER; 2];
        for label in spawn_order {
            defenders_by_label[label] = app
                .world_mut()
                .spawn((
                    Nanobot {},
                    Transform::from_xyz(68.0, 0.0, 0.0),
                    NanobotType::Defender,
                    SwarmMember::new(SwarmId::PLAYER),
                    Charge {
                        current: 0.5,
                        max: MAX_CHARGE,
                    },
                    ChargerAssignment { charger },
                    ChargerProgress { charger },
                    ChargerPulseProgress {
                        ticks_elapsed: CHARGE_PULSE_INTERVAL_TICKS - 1,
                    },
                ))
                .id();
        }

        app.update();

        let mut ordered = defenders_by_label.to_vec();
        ordered.sort_by_key(|entity| entity.to_bits());
        ordered
            .iter()
            .position(|entity| {
                app.world()
                    .entity(*entity)
                    .get::<Charge>()
                    .is_some_and(|charge| charge.current > 0.5)
            })
            .expect("one ordered Defender must receive the only supplied pulse")
    }

    assert_eq!(charged_entity_rank(false), 0);
    assert_eq!(charged_entity_rank(true), 0);
}

#[test]
fn released_charger_slot_is_claimed_deterministically() {
    let mut app = build_app();
    let swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let cell = IVec2::ZERO;
    app.world_mut()
        .resource_mut::<IntentGrid>()
        .paint(cell, IntentKind::Defend, SwarmId::PLAYER);
    let charger = common::spawn_operational_charger_at(&mut app, cell, 100);
    app.world_mut()
        .entity_mut(charger)
        .insert(OwnerSwarm(swarm));
    let center = common::cell_world_center(cell);
    for index in 0..4 {
        let defender = common::spawn_defender_at(
            &mut app,
            center + Vec2::new(100.0, (index as f32 - 1.5) * 72.0),
        );
        app.world_mut()
            .entity_mut(defender)
            .get_mut::<Charge>()
            .unwrap()
            .current = LOW_CHARGE_THRESHOLD;
    }

    wait_for_rotations(&mut app, 2);
    let released = {
        let world = app.world_mut();
        let mut query = world.query::<(Entity, &ChargerAssignment)>();
        let mut assigned = query
            .iter(world)
            .filter(|(_, assignment)| assignment.charger == charger)
            .map(|(entity, _)| entity)
            .collect::<Vec<_>>();
        assigned.sort_by_key(|entity| entity.to_bits());
        assert_eq!(assigned.len(), 2);
        assigned[0]
    };
    app.world_mut()
        .entity_mut(released)
        .remove::<ChargerAssignment>()
        .remove::<ChargerProgress>()
        .remove::<ChargerPulseProgress>()
        .remove::<DirectMovementComponent>();

    app.update();

    assert_eq!(
        app.world()
            .entity(released)
            .get::<ChargerAssignment>()
            .unwrap()
            .charger,
        charger,
    );
    let world = app.world_mut();
    let mut query = world.query::<&ChargerAssignment>();
    assert_eq!(
        query
            .iter(world)
            .filter(|assignment| assignment.charger == charger)
            .count(),
        2,
    );
}

#[test]
fn low_charge_defender_plans_charger_in_owned_defend_paint() {
    let mut app = build_app();
    common::spawn_worker_at(&mut app, Vec2::new(-1024.0, -1024.0));
    let _swarm = common::spawn_swarm_at(&mut app, Vec2::new(0.0, 0.0));
    let cell = IVec2::new(1, 0);
    app.world_mut()
        .resource_mut::<IntentGrid>()
        .paint(cell, IntentKind::Defend, SwarmId::PLAYER);
    let cell_center = common::cell_world_center(cell);
    let defender = common::spawn_defender_at(&mut app, cell_center);
    app.world_mut()
        .entity_mut(defender)
        .get_mut::<Charge>()
        .expect("Defender has Charge")
        .current = LOW_CHARGE_THRESHOLD;

    // Pre-condition: zero chargers and zero planned
    // chargers.
    assert_eq!(charger_count(app.world_mut()), 0);
    assert_eq!(planned_charger_count(app.world_mut()), 0);

    for _ in 0..120 {
        for _ in 0..100 {
            app.update();
            if planned_charger_count(app.world_mut()) > 0 {
                break;
            }
        }
        if planned_charger_count(app.world_mut()) > 0 {
            break;
        }
    }

    // Demand created a planned charger; the completed
    // Charger does NOT exist yet (a Worker must build the
    // plan first).
    assert_eq!(
        planned_charger_count(app.world_mut()),
        1,
        "one planned Charger must emerge for unserved low Charge"
    );
    assert_eq!(
        charger_count(app.world_mut()),
        0,
        "no completed charger must exist before a Worker builds the plan"
    );
    // The plan is in eligible owned Defend paint.
    let world = app.world_mut();
    let mut q = world.query::<(&PlannedStructure, &Transform)>();
    let (planned, transform) = q
        .iter(world)
        .find(|(p, _)| p.kind == PlannedKind::Charger)
        .expect("Planned Charger exists");
    assert_eq!(planned.cell, cell);
    assert!(
        (transform.translation.truncate() - Vec2::new(756.0, 252.0)).length() < 0.001,
        "nearest whole-cell footprint spans x720..792, y216..288"
    );
}

#[test]
fn defend_paint_without_low_charge_does_not_plan_charger() {
    let mut app = build_app();
    let _swarm = common::spawn_swarm_at(&mut app, Vec2::new(0.0, 0.0));
    let cell = IVec2::new(1, 0);
    app.world_mut().resource_mut::<IntentGrid>().paint(
        cell,
        IntentKind::Defend,
        top_down_2d_rts_prototype_nano_swarm::nanobot::SwarmId::PLAYER,
    );

    app.update();

    assert_eq!(
        charger_count(app.world_mut()),
        0,
        "Defend paint alone does not create a completed Charger"
    );
    assert_eq!(
        planned_charger_count(app.world_mut()),
        0,
        "Defend paint alone does not create a Charger plan"
    );
}

#[test]
fn enemy_defender_does_not_create_player_charger_demand() {
    let mut app = build_app();
    common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let cell = IVec2::ZERO;
    app.world_mut()
        .resource_mut::<IntentGrid>()
        .paint(cell, IntentKind::Defend, SwarmId::PLAYER);
    let defender = common::spawn_defender_at(&mut app, common::cell_world_center(cell));
    app.world_mut()
        .entity_mut(defender)
        .insert(SwarmMember::new(SwarmId(11)));
    app.world_mut()
        .entity_mut(defender)
        .get_mut::<Charge>()
        .expect("Defender has Charge")
        .current = LOW_CHARGE_THRESHOLD;

    app.update();

    assert_eq!(
        planned_charger_count(app.world_mut()),
        0,
        "hostile defenders must not count toward player charger demand",
    );
}

#[test]
fn only_defenders_have_charge_component() {
    // Acceptance: "Only Defenders use Charge." The Charge
    // component is only inserted on Defenders, and the charge
    // systems filter on NanobotType::Defender. The test
    // spawns a Worker, a Hauler, and a Defender, asserts the
    // Charge component is only present on the Defender, and
    // runs a few ticks to verify the drain does not touch
    // the other types.
    let mut app = build_app();
    let _swarm = common::spawn_swarm_at(&mut app, Vec2::new(0.0, 0.0));

    let worker = common::spawn_worker_at(&mut app, Vec2::new(0.0, 0.0));
    let hauler = common::spawn_hauler_at(&mut app, Vec2::new(0.0, 0.0));
    let defender = common::spawn_defender_at(&mut app, Vec2::new(0.0, 0.0));

    // Pre-condition: only the defender has Charge.
    assert!(app.world().entity(worker).get::<Charge>().is_none());
    assert!(app.world().entity(hauler).get::<Charge>().is_none());
    assert!(app.world().entity(defender).get::<Charge>().is_some());

    for _ in 0..5 {
        app.update();
    }

    // Drain only affected the defender. Worker and Hauler
    // still have no Charge component.
    assert!(app.world().entity(worker).get::<Charge>().is_none());
    assert!(app.world().entity(hauler).get::<Charge>().is_none());
    let defender_charge = read_charge(&app, defender).expect("defender has Charge");
    // Defender drained 5 ticks. The exact value depends on
    // whether the rotation/auto-creation systems also fired
    // (they didn't because there is no Defend paint and no
    // Charger), so the post-condition is "strictly less than
    // MAX_CHARGE and non-negative".
    assert!(defender_charge < MAX_CHARGE);
    assert!(defender_charge > 0.0);
}

#[test]
fn empty_charge_causes_defender_health_loss_when_no_charger() {
    // Acceptance: "Empty/ignored Charge causes Defender health loss." A
    // Defender with empty Charge and no working Charger reachable must lose
    // health per tick. The empty charge is the trigger; the
    // absence of a working charger is what makes it
    // "ignored" (no rotation happens).
    //
    // The test pre-spawns a charger with `amount = 0` so
    // the auto-creation system does not also spawn a working
    // charger (which would let the rotation chain absorb the
    // defender and stop the health loss). The pre-spawned
    // charger is "not working" because it has no supply, so
    // the rotation system does not pick it.
    let mut app = build_app();
    let _swarm = common::spawn_swarm_at(&mut app, Vec2::new(0.0, 0.0));
    let cell = IVec2::new(0, 0);
    app.world_mut().resource_mut::<IntentGrid>().paint(
        cell,
        IntentKind::Defend,
        top_down_2d_rts_prototype_nano_swarm::nanobot::SwarmId::PLAYER,
    );
    let cell_center = common::cell_world_center(cell);
    let _charger = common::spawn_operational_charger_at(&mut app, cell, 0);
    let defender = common::spawn_defender_at(&mut app, cell_center);
    // Empty out the Defender's Charge.
    {
        let w = app.world_mut();
        w.entity_mut(defender).get_mut::<Charge>().unwrap().current = 0.0;
    }

    let start_health = read_health(&app, defender).expect("defender has Health");
    assert_eq!(start_health, NANOBOT_DEFAULT_MAX_HEALTH);

    // First five empty ticks are grace; sixth tick delivers one
    // damage pulse.
    for _ in 0..usize::from(EMPTY_CHARGE_DAMAGE_INTERVAL_TICKS) {
        app.update();
    }

    let end_health = read_health(&app, defender).expect("defender still alive");
    let lost = start_health - end_health;
    assert!(
        lost > 0,
        "defender must lose health with empty charge; lost {lost}"
    );
    assert_eq!(
        lost, EMPTY_CHARGE_HEALTH_DAMAGE,
        "empty charge must damage once per six-tick pulse"
    );
}

#[test]
fn defender_does_not_lose_health_while_charging_at_a_working_charger() {
    // Companion: a defender that is at a working charger is
    // NOT in the "ignored" case, so the health loss system
    // must not fire for them. The test plants the defender
    // directly in the ChargerProgress state so the strict
    // assertion is not gated on rotation mechanics.
    let mut app = build_app();
    let swarm = common::spawn_swarm_at(&mut app, Vec2::new(0.0, 0.0));
    let cell = IVec2::new(0, 0);
    app.world_mut()
        .resource_mut::<IntentGrid>()
        .paint(cell, IntentKind::Defend, SwarmId::PLAYER);
    let cell_center = common::cell_world_center(cell);
    let charger = common::spawn_operational_charger_at(&mut app, cell, 100);
    app.world_mut()
        .entity_mut(charger)
        .insert(OwnerSwarm(swarm));
    let defender = common::spawn_defender_at(&mut app, cell_center);
    {
        let w = app.world_mut();
        w.entity_mut(defender).get_mut::<Charge>().unwrap().current = 0.0;
        w.entity_mut(defender).insert(ChargerAssignment { charger });
        w.entity_mut(defender).insert(ChargerProgress { charger });
    }

    let start_health = read_health(&app, defender).expect("defender has Health");
    assert_eq!(start_health, NANOBOT_DEFAULT_MAX_HEALTH);

    for _ in 0..5 {
        app.update();
    }

    // Drain still fires, but health loss is gated on
    // ChargerAssignment / ChargerProgress absence, so the
    // defender at a working charger must not lose any
    // health.
    let end_health = read_health(&app, defender).expect("defender has Health");
    assert_eq!(
        end_health, NANOBOT_DEFAULT_MAX_HEALTH,
        "defender at a working charger must not lose health"
    );
}

#[test]
fn defender_uses_eligible_charger_in_owned_defend_paint() {
    // Acceptance: "Defenders automatically rotate to working
    // chargers when low on Charge." A Defender with
    // charge at or below LOW_CHARGE_THRESHOLD must receive a
    // ChargerAssignment aimed at a working charger; the
    // current response is released.
    let mut app = build_app();
    let swarm = common::spawn_swarm_at(&mut app, Vec2::new(0.0, 0.0));
    let cell = IVec2::new(0, 0);
    app.world_mut()
        .resource_mut::<IntentGrid>()
        .paint(cell, IntentKind::Defend, SwarmId::PLAYER);
    let cell_center = common::cell_world_center(cell);
    let charger = common::spawn_operational_charger_at(&mut app, cell, 100);
    app.world_mut()
        .entity_mut(charger)
        .insert(OwnerSwarm(swarm));
    let defender = common::spawn_defender_at(&mut app, cell_center + Vec2::X * 68.0);
    {
        let w = app.world_mut();
        // Charge at exactly the low threshold: must trigger rotation.
        w.entity_mut(defender).get_mut::<Charge>().unwrap().current = LOW_CHARGE_THRESHOLD;
        w.entity_mut(defender).insert(DefenderResponse {
            target: Entity::PLACEHOLDER,
        });
    }

    wait_for_rotations(&mut app, 1);

    // Post-rotation: the Defender has a ChargerAssignment and no response.
    let world = app.world();
    let has_charger_assignment = world.entity(defender).get::<ChargerAssignment>().is_some();
    assert!(
        has_charger_assignment,
        "low-charge defender must be assigned to a charger"
    );
    assert!(
        world.entity(defender).get::<DefenderResponse>().is_none(),
        "the response must be released when the Defender rotates to a Charger"
    );
    // The charger is the right one.
    let assignment = world.entity(defender).get::<ChargerAssignment>().unwrap();
    assert_eq!(assignment.charger, charger);
}

#[test]
fn defender_ignores_closer_enemy_charger() {
    let mut app = build_app();
    let player = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let enemy = app
        .world_mut()
        .spawn((Swarm {}, SwarmId(11), Transform::default()))
        .id();
    let charger_cell = IVec2::ZERO;
    app.world_mut().resource_mut::<IntentGrid>().paint(
        charger_cell,
        IntentKind::Defend,
        SwarmId::PLAYER,
    );

    let enemy_charger = common::spawn_operational_charger_at(&mut app, charger_cell, 100);
    app.world_mut()
        .entity_mut(enemy_charger)
        .insert(OwnerSwarm(enemy));
    let owned_charger = common::spawn_operational_charger_at(&mut app, charger_cell, 100);
    app.world_mut()
        .entity_mut(owned_charger)
        .insert(OwnerSwarm(player));

    app.world_mut()
        .entity_mut(owned_charger)
        .get_mut::<Transform>()
        .unwrap()
        .translation
        .x += 200.0;
    let defender = common::spawn_defender_at(
        &mut app,
        common::cell_world_center(charger_cell) - Vec2::X * 68.0,
    );
    app.world_mut()
        .entity_mut(defender)
        .insert(SwarmMember::new(SwarmId::PLAYER));
    app.world_mut()
        .entity_mut(defender)
        .get_mut::<Charge>()
        .unwrap()
        .current = LOW_CHARGE_THRESHOLD;

    wait_for_rotations(&mut app, 1);

    assert_eq!(
        app.world()
            .entity(defender)
            .get::<ChargerAssignment>()
            .expect("low-charge defender rotates to an owned charger")
            .charger,
        owned_charger,
    );
}

#[test]
fn defender_uses_nearest_owned_charger_even_under_overlapping_enemy_paint() {
    let mut app = build_app();
    let swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let defender_cell = IVec2::ZERO;
    let near_cell = IVec2::new(1, 0);
    let far_cell = IVec2::new(3, 0);
    for cell in [defender_cell, near_cell, far_cell] {
        app.world_mut().resource_mut::<IntentGrid>().paint(
            cell,
            IntentKind::Defend,
            SwarmId::PLAYER,
        );
    }
    app.world_mut()
        .resource_mut::<IntentGrid>()
        .paint(near_cell, IntentKind::Defend, SwarmId(11));
    let far = common::spawn_operational_charger_at(&mut app, far_cell, 100);
    app.world_mut().entity_mut(far).insert(OwnerSwarm(swarm));
    let near = common::spawn_operational_charger_at(&mut app, near_cell, 100);
    app.world_mut().entity_mut(near).insert(OwnerSwarm(swarm));
    let defender = common::spawn_defender_at(&mut app, common::cell_world_center(defender_cell));
    app.world_mut()
        .entity_mut(defender)
        .get_mut::<Charge>()
        .unwrap()
        .current = LOW_CHARGE_THRESHOLD;

    wait_for_rotations(&mut app, 1);

    assert_eq!(
        app.world()
            .entity(defender)
            .get::<ChargerAssignment>()
            .expect("low-Charge Defender rotates across its swarm's Defend paint")
            .charger,
        near,
    );
}

#[test]
fn charger_without_valid_owner_is_not_eligible() {
    let mut app = build_app();
    common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let cell = IVec2::ZERO;
    app.world_mut()
        .resource_mut::<IntentGrid>()
        .paint(cell, IntentKind::Defend, SwarmId::PLAYER);
    let charger = common::spawn_operational_charger_at(&mut app, cell, 100);
    assert!(app.world().entity(charger).get::<OwnerSwarm>().is_none());
    let defender = common::spawn_defender_at(&mut app, common::cell_world_center(cell));
    app.world_mut()
        .entity_mut(defender)
        .get_mut::<Charge>()
        .unwrap()
        .current = LOW_CHARGE_THRESHOLD;

    app.update();

    assert!(
        app.world()
            .entity(defender)
            .get::<ChargerAssignment>()
            .is_none()
    );
}

#[test]
fn charger_in_only_enemy_defend_paint_is_inactive() {
    let mut app = build_app();
    let swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let staging_cell = IVec2::ZERO;
    let charger_cell = IVec2::new(1, 0);
    app.world_mut().resource_mut::<IntentGrid>().paint(
        staging_cell,
        IntentKind::Defend,
        SwarmId::PLAYER,
    );
    app.world_mut().resource_mut::<IntentGrid>().paint(
        charger_cell,
        IntentKind::Defend,
        SwarmId(11),
    );
    assert_eq!(
        app.world()
            .resource::<IntentGrid>()
            .cell(charger_cell)
            .map(|cell| cell.owners(IntentKind::Defend).collect::<Vec<_>>()),
        Some(vec![SwarmId(11)]),
    );
    let charger = common::spawn_operational_charger_at(&mut app, charger_cell, 100);
    app.world_mut()
        .entity_mut(charger)
        .insert(OwnerSwarm(swarm));
    let defender = common::spawn_defender_at(&mut app, common::cell_world_center(staging_cell));
    app.world_mut()
        .entity_mut(defender)
        .get_mut::<Charge>()
        .unwrap()
        .current = LOW_CHARGE_THRESHOLD;

    app.update();

    assert!(
        app.world()
            .entity(defender)
            .get::<ChargerAssignment>()
            .is_none()
    );
}

#[test]
fn charger_without_support_condition_is_not_eligible() {
    let mut app = build_app();
    let swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let cell = IVec2::ZERO;
    app.world_mut()
        .resource_mut::<IntentGrid>()
        .paint(cell, IntentKind::Defend, SwarmId::PLAYER);
    let charger = common::spawn_operational_charger_at(&mut app, cell, 100);
    app.world_mut()
        .entity_mut(charger)
        .insert(OwnerSwarm(swarm))
        .remove::<Structure>();
    assert!(app.world().entity(charger).get::<Structure>().is_none());
    let defender = common::spawn_defender_at(&mut app, common::cell_world_center(cell));
    app.world_mut()
        .entity_mut(defender)
        .get_mut::<Charge>()
        .unwrap()
        .current = LOW_CHARGE_THRESHOLD;

    app.update();

    assert!(
        app.world()
            .entity(defender)
            .get::<ChargerAssignment>()
            .is_none()
    );
}

#[test]
fn en_route_and_charging_defenders_share_charger_capacity() {
    let mut app = build_app();
    let swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let cell = IVec2::ZERO;
    app.world_mut()
        .resource_mut::<IntentGrid>()
        .paint(cell, IntentKind::Defend, SwarmId::PLAYER);
    let charger = common::spawn_operational_charger_at(&mut app, cell, 100);
    app.world_mut()
        .entity_mut(charger)
        .insert(OwnerSwarm(swarm));
    let center = common::cell_world_center(cell);
    let en_route = common::spawn_defender_at(&mut app, center + Vec2::new(-140.0, 0.0));
    app.world_mut().entity_mut(en_route).insert((
        ChargerAssignment { charger },
        DirectMovementComponent {
            speed: None,
            interaction: None,
            xy: center - Vec2::X * 68.0,
            stop_radius: 0.0,
        },
    ));
    let charging = common::spawn_defender_at(&mut app, center + Vec2::X * 68.0);
    app.world_mut().entity_mut(charging).insert((
        ChargerAssignment { charger },
        ChargerProgress { charger },
        ChargerPulseProgress::default(),
    ));
    for offset in [140.0, 212.0, 284.0, 356.0] {
        let defender = common::spawn_defender_at(&mut app, center + Vec2::new(offset, 0.0));
        app.world_mut()
            .entity_mut(defender)
            .get_mut::<Charge>()
            .unwrap()
            .current = LOW_CHARGE_THRESHOLD;
    }

    wait_for_rotations(&mut app, 3);

    let world = app.world_mut();
    let mut assignments = world.query::<&ChargerAssignment>();
    assert_eq!(
        assignments
            .iter(world)
            .filter(|assignment| assignment.charger == charger)
            .count(),
        3,
    );
}

#[test]
fn charge_departure_releases_response_for_same_step_replacement() {
    let mut app = build_app();
    let frame = Duration::from_nanos(16_666_667);
    app.insert_resource(Time::<Fixed>::from_duration(frame));
    app.insert_resource(TimeUpdateStrategy::ManualDuration(frame));
    let swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let cell = IVec2::ZERO;
    app.world_mut()
        .resource_mut::<IntentGrid>()
        .paint(cell, IntentKind::Defend, SwarmId::PLAYER);
    let charger = common::spawn_operational_charger_at(&mut app, cell, 100);
    app.world_mut()
        .entity_mut(charger)
        .insert(OwnerSwarm(swarm));
    let defender =
        common::spawn_defender_at(&mut app, common::cell_world_center(cell) + Vec2::X * 68.0);
    let replacement = common::spawn_defender_at(
        &mut app,
        common::cell_world_center(cell) + Vec2::new(68.0, 72.0),
    );
    let threat = common::spawn_worker_at(
        &mut app,
        common::cell_world_center(cell) + Vec2::new(140.0, 72.0),
    );
    app.world_mut()
        .entity_mut(threat)
        .insert(SwarmMember::new(SwarmId(11)));
    app.world_mut()
        .entity_mut(defender)
        .insert(DefenderResponse { target: threat });
    app.world_mut()
        .entity_mut(defender)
        .get_mut::<Charge>()
        .unwrap()
        .current = LOW_CHARGE_THRESHOLD;

    wait_for_rotations(&mut app, 1);

    let defender = app.world().entity(defender);
    assert_eq!(
        defender
            .get::<ChargerAssignment>()
            .expect("low-Charge Defender departs for service")
            .charger,
        charger,
    );
    assert!(defender.get::<DefenderResponse>().is_none());
    assert_eq!(
        app.world()
            .entity(replacement)
            .get::<DefenderResponse>()
            .map(|response| response.target),
        Some(threat),
        "Charge departure must permit replacement on the same fixed step",
    );
}

#[test]
fn equal_charge_prefers_staged_defender_over_current_tactical_duty() {
    let mut app = build_app();
    let swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let cell = IVec2::ZERO;
    app.world_mut()
        .resource_mut::<IntentGrid>()
        .paint(cell, IntentKind::Defend, SwarmId::PLAYER);
    let charger = common::spawn_operational_charger_at(&mut app, cell, 100);
    app.world_mut()
        .entity_mut(charger)
        .insert(OwnerSwarm(swarm));
    let first =
        common::spawn_defender_at(&mut app, common::cell_world_center(cell) + Vec2::X * 68.0);
    let second = common::spawn_defender_at(
        &mut app,
        common::cell_world_center(cell) + Vec2::new(68.0, 72.0),
    );
    let (tactical, staged) = if first.to_bits() < second.to_bits() {
        (first, second)
    } else {
        (second, first)
    };
    let threat = common::spawn_worker_at(
        &mut app,
        common::cell_world_center(cell) + Vec2::new(140.0, 72.0),
    );
    app.world_mut()
        .entity_mut(threat)
        .insert(SwarmMember::new(SwarmId(11)));
    app.world_mut()
        .entity_mut(tactical)
        .insert(DefenderResponse { target: threat });
    app.world_mut()
        .entity_mut(tactical)
        .get_mut::<Charge>()
        .unwrap()
        .current = 0.2;
    app.world_mut()
        .entity_mut(staged)
        .get_mut::<Charge>()
        .unwrap()
        .current = 0.2;

    wait_for_rotations(&mut app, 1);

    assert!(
        app.world()
            .entity(staged)
            .get::<ChargerAssignment>()
            .is_some()
    );
    assert!(
        app.world()
            .entity(tactical)
            .get::<ChargerAssignment>()
            .is_none()
    );
}

#[test]
fn invalidated_assignment_releases_capacity_and_charge_state() {
    let mut app = App::new();
    app.insert_resource(IntentGrid::new(8, 8));
    app.init_resource::<ResourceLedger>();
    app.add_systems(Update, defender_charger_work_system);
    let cell = IVec2::ZERO;
    let charger = common::spawn_operational_charger_at(&mut app, cell, 100);
    let defender = app
        .world_mut()
        .spawn((
            Nanobot {},
            NanobotType::Defender,
            SwarmMember::new(SwarmId::PLAYER),
            Health::default(),
            Charge::default(),
            Transform::from_translation(common::cell_world_center(cell).extend(0.0)),
            ChargerAssignment { charger },
            ChargerProgress { charger },
            ChargerPulseProgress::default(),
        ))
        .id();

    app.update();

    let world = app.world();
    assert!(world.entity(defender).get::<ChargerAssignment>().is_none());
    assert!(world.entity(defender).get::<ChargerProgress>().is_none());
}

#[test]
fn emptied_charger_cancels_en_route_assignment_before_arrival() {
    let mut app = App::new();
    let cell = IVec2::ZERO;
    app.insert_resource(IntentGrid::new(8, 8));
    app.world_mut()
        .resource_mut::<IntentGrid>()
        .paint(cell, IntentKind::Defend, SwarmId::PLAYER);
    let charger = app
        .world_mut()
        .spawn((
            Charger::new(cell),
            Transform::from_translation(common::cell_world_center(cell).extend(0.0)),
        ))
        .id();
    let defender = app
        .world_mut()
        .spawn((
            Nanobot {},
            NanobotType::Defender,
            SwarmMember::new(SwarmId::PLAYER),
            Transform::from_translation(Vec2::ZERO.extend(0.0)),
            ChargerAssignment { charger },
            DirectMovementComponent {
                speed: None,
                interaction: None,
                xy: common::cell_world_center(cell),
                stop_radius: 0.0,
            },
        ))
        .id();

    app.add_systems(Update, defender_charger_arrive_system);
    app.update();

    let world = app.world();
    assert!(world.entity(defender).get::<ChargerAssignment>().is_none());
    assert!(
        world
            .entity(defender)
            .get::<DirectMovementComponent>()
            .is_none()
    );
    assert!(world.entity(defender).get::<ChargerProgress>().is_none());
}

#[test]
fn defender_does_not_rotate_to_empty_charger() {
    // Companion: a charger with no material is not a
    // "working" rotation target, so a low-charge defender
    // must not rotate to it. The Defender stays on current duty and the
    // empty-Charge health loss system fires (covered by
    // another test).
    let mut app = build_app();
    let swarm = common::spawn_swarm_at(&mut app, Vec2::new(0.0, 0.0));
    let cell = IVec2::new(0, 0);
    app.world_mut()
        .resource_mut::<IntentGrid>()
        .paint(cell, IntentKind::Defend, SwarmId::PLAYER);
    let cell_center = common::cell_world_center(cell);
    // Charger with NO material: not a working rotation target.
    let charger = common::spawn_operational_charger_at(&mut app, cell, 0);
    app.world_mut()
        .entity_mut(charger)
        .insert(OwnerSwarm(swarm));
    let defender = common::spawn_defender_at(&mut app, cell_center);
    {
        let w = app.world_mut();
        w.entity_mut(defender).get_mut::<Charge>().unwrap().current = LOW_CHARGE_THRESHOLD;
    }

    app.update();

    let world = app.world();
    assert!(
        world.entity(defender).get::<ChargerAssignment>().is_none(),
        "defender must not rotate to an empty charger"
    );
}

#[test]
fn defender_does_not_rotate_to_degraded_charger() {
    let mut app = build_app();
    let swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let cell = IVec2::ZERO;
    app.world_mut()
        .resource_mut::<IntentGrid>()
        .paint(cell, IntentKind::Defend, SwarmId::PLAYER);
    let charger = common::spawn_operational_charger_at(&mut app, cell, 100);
    let mut condition = Structure::new(StructureKind::Basic);
    condition.health = SUPPORT_OPERATIONAL_HEALTH_THRESHOLD - 1;
    app.world_mut()
        .entity_mut(charger)
        .insert((OwnerSwarm(swarm), condition));
    let defender = common::spawn_defender_at(&mut app, common::cell_world_center(cell));
    app.world_mut()
        .entity_mut(defender)
        .get_mut::<Charge>()
        .unwrap()
        .current = LOW_CHARGE_THRESHOLD;

    app.update();

    assert!(
        app.world()
            .entity(defender)
            .get::<ChargerAssignment>()
            .is_none(),
        "degraded Charger must stop operating until repaired",
    );
}

#[test]
fn completed_rotation_reenters_current_allocation_without_old_ownership() {
    let mut app = common::sim_app_with_charge();
    let swarm = common::spawn_swarm_at(&mut app, Vec2::new(0.0, 0.0));
    let cell = IVec2::ZERO;
    app.world_mut()
        .resource_mut::<IntentGrid>()
        .paint(cell, IntentKind::Defend, SwarmId::PLAYER);
    let charger = common::spawn_operational_charger_at(&mut app, cell, 200);
    app.world_mut()
        .entity_mut(charger)
        .insert(OwnerSwarm(swarm));
    let center = common::cell_world_center(cell);
    let defender = common::spawn_defender_at(&mut app, center + Vec2::X * 68.0);
    let replacement = common::spawn_defender_at(&mut app, center + Vec2::new(68.0, 72.0));
    let threat = common::spawn_worker_at(&mut app, center + Vec2::new(140.0, 72.0));
    app.world_mut()
        .entity_mut(threat)
        .insert(SwarmMember::new(SwarmId(11)));
    {
        let w = app.world_mut();
        w.entity_mut(defender).get_mut::<Charge>().unwrap().current = 0.2;
        w.entity_mut(defender)
            .insert(DefenderResponse { target: threat });
    }

    wait_for_rotations(&mut app, 1);
    assert!(
        app.world()
            .entity(defender)
            .get::<ChargerAssignment>()
            .is_some()
    );
    assert_eq!(
        app.world()
            .entity(replacement)
            .get::<DefenderResponse>()
            .map(|response| response.target),
        Some(threat),
    );

    for _ in 0..700 {
        app.update();
    }

    assert_eq!(
        app.world()
            .entity(replacement)
            .get::<DefenderResponse>()
            .map(|response| response.target),
        Some(threat),
        "the recharged Defender must not displace the current responder",
    );
    let defender_state = app.world().entity(defender);
    assert!(defender_state.get::<DefenderResponse>().is_none());
    let charge = read_charge(&app, defender).expect("defender has Charge");
    assert!(
        charge >= LOW_CHARGE_THRESHOLD,
        "reassigned Defender stays above the next rotation threshold; got {charge}"
    );
    assert!(
        charge <= MAX_CHARGE,
        "charge cannot exceed the cap; got {charge}"
    );
    assert!(
        app.world()
            .entity(defender)
            .get::<ChargerAssignment>()
            .is_none(),
        "ChargerAssignment must be removed after charging"
    );
    assert!(
        app.world()
            .entity(defender)
            .get::<ChargerProgress>()
            .is_none(),
        "ChargerProgress must be removed after charging"
    );
    let c = app.world().entity(charger).get::<Charger>().unwrap();
    assert!(
        c.amount < 200,
        "charger material must be drained while defenders charge from it; got {}",
        c.amount
    );
}

#[test]
fn invalid_charger_cleanup_reenters_current_allocation_in_the_same_tick() {
    let mut app = common::sim_app_with_charge();
    let swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let staging_cell = IVec2::ZERO;
    let inactive_charger_cell = IVec2::new(1, 0);
    app.world_mut().resource_mut::<IntentGrid>().paint(
        staging_cell,
        IntentKind::Gather,
        SwarmId::PLAYER,
    );
    let threat = common::spawn_worker_at(&mut app, common::cell_world_center(staging_cell));
    app.world_mut()
        .entity_mut(threat)
        .insert(SwarmMember::new(SwarmId(11)));
    let charger = common::spawn_operational_charger_at(&mut app, inactive_charger_cell, 100);
    app.world_mut()
        .entity_mut(charger)
        .insert(OwnerSwarm(swarm));
    let defender = common::spawn_defender_at(&mut app, common::cell_world_center(staging_cell));
    app.world_mut().entity_mut(defender).insert((
        ChargerAssignment { charger },
        DirectMovementComponent {
            speed: None,
            interaction: None,
            xy: common::cell_world_center(inactive_charger_cell),
            stop_radius: 0.0,
        },
    ));

    app.update();

    let defender = app.world().entity(defender);
    assert!(defender.get::<ChargerAssignment>().is_none());
    assert_eq!(
        defender
            .get::<DefenderResponse>()
            .map(|response| response.target),
        Some(threat),
    );
}

#[test]
fn invalid_charger_cleanup_wakes_allocation_between_regular_ten_hertz_ticks() {
    let mut app = common::sim_app_with_charge();
    let frame = Duration::from_nanos(16_666_667);
    app.insert_resource(Time::<Fixed>::from_duration(frame));
    app.insert_resource(TimeUpdateStrategy::ManualDuration(frame));
    let swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let staging_cell = IVec2::ZERO;
    let inactive_charger_cell = IVec2::new(1, 0);
    app.world_mut().resource_mut::<IntentGrid>().paint(
        staging_cell,
        IntentKind::Build,
        SwarmId::PLAYER,
    );

    // Consume the allocator's initial due pass. The next five fixed steps are
    // normally below its deterministic 100 ms cadence.
    app.update();

    let threat = common::spawn_worker_at(&mut app, common::cell_world_center(staging_cell));
    app.world_mut()
        .entity_mut(threat)
        .insert(SwarmMember::new(SwarmId(11)));

    let charger = common::spawn_operational_charger_at(&mut app, inactive_charger_cell, 100);
    app.world_mut()
        .entity_mut(charger)
        .insert(OwnerSwarm(swarm));
    let defender = common::spawn_defender_at(&mut app, common::cell_world_center(staging_cell));
    app.world_mut().entity_mut(defender).insert((
        ChargerAssignment { charger },
        DirectMovementComponent {
            speed: None,
            interaction: None,
            xy: common::cell_world_center(inactive_charger_cell),
            stop_radius: 0.0,
        },
    ));

    app.update();

    let defender = app.world().entity(defender);
    assert!(defender.get::<ChargerAssignment>().is_none());
    assert_eq!(
        defender
            .get::<DefenderResponse>()
            .map(|response| response.target),
        Some(threat),
    );
}

#[test]
fn completed_recharge_wakes_allocation_between_regular_ten_hertz_ticks() {
    let mut app = common::sim_app_with_charge();
    let frame = Duration::from_nanos(16_666_667);
    app.insert_resource(Time::<Fixed>::from_duration(frame));
    app.insert_resource(TimeUpdateStrategy::ManualDuration(frame));
    let swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let cell = IVec2::ZERO;
    app.world_mut()
        .resource_mut::<IntentGrid>()
        .paint(cell, IntentKind::Defend, SwarmId::PLAYER);
    let charger = common::spawn_operational_charger_at(&mut app, cell, 100);
    app.world_mut()
        .entity_mut(charger)
        .insert(OwnerSwarm(swarm));

    app.update();

    let threat =
        common::spawn_worker_at(&mut app, common::cell_world_center(cell) + Vec2::X * 160.0);
    app.world_mut()
        .entity_mut(threat)
        .insert(SwarmMember::new(SwarmId(11)));

    let defender =
        common::spawn_defender_at(&mut app, common::cell_world_center(cell) + Vec2::X * 68.0);
    app.world_mut().entity_mut(defender).insert((
        ChargerAssignment { charger },
        ChargerProgress { charger },
        ChargerPulseProgress {
            ticks_elapsed: CHARGE_PULSE_INTERVAL_TICKS - 1,
        },
    ));
    app.world_mut()
        .entity_mut(defender)
        .get_mut::<Charge>()
        .unwrap()
        .current = MAX_CHARGE - CHARGE_PER_PULSE / 2.0;

    app.update();

    let defender = app.world().entity(defender);
    assert!(defender.get::<ChargerAssignment>().is_none());
    assert!(defender.get::<ChargerProgress>().is_none());
    assert_eq!(
        defender
            .get::<DefenderResponse>()
            .map(|response| response.target),
        Some(threat),
        "completed recharge returns through current response allocation",
    );
}

#[test]
fn charger_requires_logistics_support_via_physical_resources() {
    // Acceptance: "Chargers require logistics support via
    // physical resources." A charger with no material is not
    // a working rotation target; a defender with low charge
    // and no working charger does not rotate and starts
    // losing health. The same scenario, with a stocked
    // charger, lets the defender rotate and recover.
    //
    // The test pins the "logistics support" contract end to
    // end: a Defender beside an empty Charger must end up in worse health than a
    // defender at the same cell with a stocked charger.
    let mut app_empty = build_app();
    {
        let swarm = common::spawn_swarm_at(&mut app_empty, Vec2::new(0.0, 0.0));
        let cell = IVec2::new(0, 0);
        app_empty.world_mut().resource_mut::<IntentGrid>().paint(
            cell,
            IntentKind::Defend,
            SwarmId::PLAYER,
        );
        let cell_center = common::cell_world_center(cell);
        // Pre-spawn an empty charger so the auto-creation
        // system does not also create a working charger
        // (which would let the rotation chain absorb the
        // defender and stop the health loss). The empty
        // charger is "not working" because it has no supply.
        let charger = common::spawn_operational_charger_at(&mut app_empty, cell, 0);
        app_empty
            .world_mut()
            .entity_mut(charger)
            .insert(OwnerSwarm(swarm));
        let defender = common::spawn_defender_at(&mut app_empty, cell_center);
        {
            let w = app_empty.world_mut();
            w.entity_mut(defender).get_mut::<Charge>().unwrap().current = 0.0;
        }
        for _ in 0..usize::from(EMPTY_CHARGE_DAMAGE_INTERVAL_TICKS) {
            app_empty.update();
        }
        let health_empty = read_health(&app_empty, defender);
        // The defender is collapsing; the entity may be
        // despawned if health reached zero. The "logistics
        // dependence" contract is "no material => no
        // recovery => health drops"; either the entity
        // exists with reduced health or it was despawned.
        if let Some(h) = health_empty {
            assert!(
                h < NANOBOT_DEFAULT_MAX_HEALTH,
                "empty charger must lead to health loss; got {h}"
            );
        }
    }

    let mut app_filled = build_app();
    {
        let swarm = common::spawn_swarm_at(&mut app_filled, Vec2::new(0.0, 0.0));
        let cell = IVec2::new(0, 0);
        app_filled.world_mut().resource_mut::<IntentGrid>().paint(
            cell,
            IntentKind::Defend,
            SwarmId::PLAYER,
        );
        let cell_center = common::cell_world_center(cell);
        let charger = common::spawn_operational_charger_at(&mut app_filled, cell, 200);
        app_filled
            .world_mut()
            .entity_mut(charger)
            .insert(OwnerSwarm(swarm));
        let defender = common::spawn_defender_at(&mut app_filled, cell_center);
        {
            let w = app_filled.world_mut();
            w.entity_mut(defender).get_mut::<Charge>().unwrap().current = LOW_CHARGE_THRESHOLD;
        }
        // Drive a few ticks: rotation + arrival + at least
        // one charging tick. 30 is a safe margin.
        for _ in 0..30 {
            app_filled.update();
        }
        let health_filled = read_health(&app_filled, defender)
            .expect("defender with a stocked charger must still be alive");
        assert_eq!(
            health_filled, NANOBOT_DEFAULT_MAX_HEALTH,
            "stocked charger must keep defender at full health; got {health_filled}"
        );
    }
}

#[test]
fn hauler_delivers_minerals_to_a_charger_with_free_space() {
    // Physical logistics can fill owner-compatible Charger capacity without
    // Defend paint; paint independently controls Defender service eligibility.
    let mut app = build_app();
    let swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let source_pos = Vec2::new(100.0, 0.0);
    let cell = IVec2::new(2, 0);
    // Terminal leg source: a sink stockpile. Chargers cannot
    // bypass shared staging by drawing from source stockpiles.
    let source = common::spawn_sink_stockpile(&mut app, source_pos, 1000, 1000);
    let charger = common::spawn_operational_charger_at(&mut app, cell, 0);
    app.world_mut().entity_mut(source).insert(OwnerSwarm(swarm));
    app.world_mut()
        .entity_mut(charger)
        .insert(OwnerSwarm(swarm));
    let _hauler = common::spawn_hauler_at(&mut app, source_pos + Vec2::X * 68.0);
    // Drive enough ticks for the hauler to load (5 ticks at
    // HAULER_EXTRACT_PER_TICK) and walk from the deposit at
    // (100, 0) to the charger at the cell (2, 0) center
    // (~1280, 256). Distance ~ 1200 world units; at
    // bot_speed 5.0 = ~240 ticks. 500 is a safe margin.
    for _ in 0..500 {
        app.update();
    }

    let c = app.world().entity(charger).get::<Charger>().unwrap();
    assert!(
        c.amount > 0,
        "hauler must have delivered minerals to the charger; charger.amount = {}",
        c.amount
    );
    let s = app
        .world()
        .entity(source)
        .get::<top_down_2d_rts_prototype_nano_swarm::resources::Stockpile>()
        .unwrap();
    assert!(
        s.amount < 1000,
        "source stockpile must have lost material to the hauler; amount = {}",
        s.amount
    );
}

#[test]
fn defender_without_low_charge_does_not_rotate_to_charger() {
    // Companion: a defender with charge above
    // LOW_CHARGE_THRESHOLD must not be picked up by the
    // rotation system even if a working charger exists.
    // The Defender stays on current duty.
    let mut app = build_app();
    let swarm = common::spawn_swarm_at(&mut app, Vec2::new(0.0, 0.0));
    let cell = IVec2::new(0, 0);
    app.world_mut()
        .resource_mut::<IntentGrid>()
        .paint(cell, IntentKind::Defend, SwarmId::PLAYER);
    let cell_center = common::cell_world_center(cell);
    let charger = common::spawn_operational_charger_at(&mut app, cell, 100);
    app.world_mut()
        .entity_mut(charger)
        .insert(OwnerSwarm(swarm));
    let defender = common::spawn_defender_at(&mut app, cell_center);
    {
        let w = app.world_mut();
        // Above LOW_CHARGE_THRESHOLD: still strong, no rotation.
        w.entity_mut(defender).get_mut::<Charge>().unwrap().current = LOW_CHARGE_THRESHOLD + 0.1;
    }

    app.update();

    let world = app.world();
    assert!(
        world.entity(defender).get::<ChargerAssignment>().is_none(),
        "fully-charged defender must not rotate to a charger"
    );
}

#[test]
fn defender_charge_drains_passively_when_idle() {
    // Acceptance: "Defenders use Charge." A defender with
    // a Charge component must see the charge decrease over
    // time even when no charger is reachable. The drain
    // rate is CHARGE_DRAIN_PER_TICK per tick.
    let mut app = build_app();
    let _swarm = common::spawn_swarm_at(&mut app, Vec2::new(0.0, 0.0));
    let defender = common::spawn_defender_at(&mut app, Vec2::new(0.0, 0.0));
    let start = read_charge(&app, defender).expect("defender has Charge");
    assert_eq!(start, MAX_CHARGE);

    // Run 10 ticks without any Defend paint and without any
    // charger. The drain system must fire each tick.
    for _ in 0..10 {
        app.update();
    }
    let end = read_charge(&app, defender).expect("defender has Charge");
    // Pin the per-tick rate exactly: 10 ticks at
    // CHARGE_DRAIN_PER_TICK.
    let expected = MAX_CHARGE - CHARGE_DRAIN_PER_TICK * 10.0;
    assert!(
        (end - expected).abs() < 1e-5,
        "charge must drain by CHARGE_DRAIN_PER_TICK per tick; expected {expected}, got {end}"
    );
    // The drain must be strictly positive (the constant
    // matters, not just the sign).
    assert!(end < start, "charge must decrease over time");
}

#[test]
fn defender_charge_refills_in_supplied_pulses() {
    // One supplied pulse grants charge after ten drain ticks.
    // The test plants the defender in ChargerProgress
    // state directly so arrival mechanics do not interfere
    // with the rate check.
    let mut app = build_app();
    let swarm = common::spawn_swarm_at(&mut app, Vec2::new(0.0, 0.0));
    let cell = IVec2::new(0, 0);
    app.world_mut()
        .resource_mut::<IntentGrid>()
        .paint(cell, IntentKind::Defend, SwarmId::PLAYER);
    let cell_center = common::cell_world_center(cell);
    let charger = common::spawn_operational_charger_at(&mut app, cell, 100);
    app.world_mut()
        .entity_mut(charger)
        .insert(OwnerSwarm(swarm));
    let defender = common::spawn_defender_at(&mut app, cell_center + Vec2::X * 68.0);
    {
        let w = app.world_mut();
        w.entity_mut(defender).get_mut::<Charge>().unwrap().current = 0.5;
        // Plant the defender in the charging state directly
        // so arrival mechanics are not in the way.
        w.entity_mut(defender).insert(ChargerAssignment { charger });
        w.entity_mut(defender).insert((
            ChargerProgress { charger },
            ChargerPulseProgress {
                ticks_elapsed: CHARGE_PULSE_INTERVAL_TICKS - 1,
            },
        ));
    }
    let start = read_charge(&app, defender).expect("defender has Charge");
    assert!((start - 0.5).abs() < 1e-5);

    app.update();
    let end = read_charge(&app, defender).expect("defender has Charge");
    let expected_saturated = (0.5 - CHARGE_DRAIN_PER_TICK + CHARGE_PER_PULSE).min(MAX_CHARGE);
    assert!(
        (end - expected_saturated).abs() < 1e-5,
        "charge must refill at supplied pulse; expected {expected_saturated}, got {end}"
    );
    assert!(
        end > start,
        "charge must increase while at a working charger"
    );
}

#[test]
fn charge_duty_excludes_defender_from_response_acquisition() {
    // A Defender in the charging state stays outside response acquisition
    // until the Charge lifecycle releases it back to current allocation.
    let mut app = build_app();
    let swarm = common::spawn_swarm_at(&mut app, Vec2::new(0.0, 0.0));
    let cell = IVec2::new(0, 0);
    let other_cell = IVec2::new(2, 0);
    app.world_mut()
        .resource_mut::<IntentGrid>()
        .paint(cell, IntentKind::Defend, SwarmId::PLAYER);
    app.world_mut().resource_mut::<IntentGrid>().paint(
        other_cell,
        IntentKind::Defend,
        SwarmId::PLAYER,
    );
    let cell_center = common::cell_world_center(cell);
    let charger = common::spawn_operational_charger_at(&mut app, cell, 100);
    app.world_mut()
        .entity_mut(charger)
        .insert(OwnerSwarm(swarm));
    let defender = common::spawn_defender_at(&mut app, cell_center);
    {
        let w = app.world_mut();
        w.entity_mut(defender).get_mut::<Charge>().unwrap().current = 0.1;
        w.entity_mut(defender).insert(ChargerAssignment { charger });
        w.entity_mut(defender).insert(ChargerProgress { charger });
    }

    // Charge duty must keep this Defender out of response acquisition.
    app.update();

    let world = app.world();
    assert!(
        world.entity(defender).get::<DefenderResponse>().is_none(),
        "a Defender on Charge duty must not acquire a response"
    );
}

#[test]
fn charger_work_consumes_owning_swarm_resources() {
    let mut app = App::new();
    app.insert_resource(IntentGrid::new(4, 4))
        .init_resource::<ResourceLedger>()
        .add_systems(Update, defender_charger_work_system);
    let swarm = app.world_mut().spawn(SwarmBundle::default()).id();
    app.world_mut().resource_mut::<IntentGrid>().paint(
        IVec2::ZERO,
        IntentKind::Defend,
        SwarmId::PLAYER,
    );
    let mut charger_component = Charger::new(IVec2::ZERO);
    charger_component.amount = 10;
    let charger = app
        .world_mut()
        .spawn((
            charger_component,
            OwnerSwarm(swarm),
            Structure::new(StructureKind::Basic),
            Transform::default(),
        ))
        .id();
    app.world_mut().resource_mut::<ResourceLedger>().add_for(
        top_down_2d_rts_prototype_nano_swarm::nanobot::SwarmId::PLAYER,
        ResourceKind::Minerals,
        10,
    );
    let defender = app
        .world_mut()
        .spawn((
            NanobotBundle {
                nanobot_type: NanobotType::Defender,
                ..default()
            },
            Transform::from_xyz(68.0, 0.0, 0.0),
            Charge {
                current: 0.5,
                max: MAX_CHARGE,
            },
            ChargerAssignment { charger },
            (
                ChargerProgress { charger },
                ChargerPulseProgress {
                    ticks_elapsed: CHARGE_PULSE_INTERVAL_TICKS - 1,
                },
            ),
        ))
        .id();

    app.update();

    assert_eq!(
        app.world().entity(charger).get::<Charger>().unwrap().amount,
        9
    );
    assert_eq!(
        app.world().resource::<ResourceLedger>().total_for(
            top_down_2d_rts_prototype_nano_swarm::nanobot::SwarmId::PLAYER,
            ResourceKind::Minerals,
        ),
        10 - CHARGER_MATERIAL_PER_PULSE,
        "charger work removes consumed material from owning swarm",
    );
    assert!(app.world().get_entity(defender).is_ok());
}

#[test]
fn loaded_nanobot_death_loses_exact_cargo_and_releases_reservation() {
    let mut app = App::new();
    app.init_resource::<ResourceLedger>()
        .add_systems(Update, nanobot_death_cleanup_system);
    let source = app.world_mut().spawn_empty().id();
    let sink = app.world_mut().spawn_empty().id();
    let cargo_amount = 7;
    app.world_mut().resource_mut::<ResourceLedger>().add_for(
        SwarmId::PLAYER,
        ResourceKind::Minerals,
        cargo_amount + 5,
    );
    let nanobot = app
        .world_mut()
        .spawn((
            NanobotBundle {
                nanobot_type: NanobotType::Hauler,
                health: Health {
                    current: 0,
                    max: 100,
                },
                ..default()
            },
            Cargo {
                kind: ResourceKind::Minerals,
                amount: cargo_amount,
            },
            LogisticsReservation::new(source, sink, ResourceKind::Minerals, cargo_amount),
        ))
        .id();

    app.update();

    assert!(app.world().get_entity(nanobot).is_err());
    assert_eq!(
        app.world()
            .resource::<ResourceLedger>()
            .total_for(SwarmId::PLAYER, ResourceKind::Minerals),
        5,
        "death removes exact remaining cargo, not original reservation amount",
    );
    let mut reservations = app.world_mut().query::<&LogisticsReservation>();
    assert_eq!(
        reservations.iter(app.world()).count(),
        0,
        "death releases reservation before entity removal",
    );
}

#[test]
fn nanobot_plugin_cleans_dead_bot_after_ai_deferred_commands() {
    let mut app = common::minimal_app();
    app.add_plugins((NanobotPlugin::default(), AiPlugin));
    let cargo_amount = 7;
    app.world_mut().resource_mut::<ResourceLedger>().add_for(
        SwarmId::PLAYER,
        ResourceKind::Minerals,
        cargo_amount,
    );
    let mut bundle = NanobotBundle::default();
    bundle.health.current = 0;
    let nanobot = app
        .world_mut()
        .spawn((
            bundle,
            Cargo {
                kind: ResourceKind::Minerals,
                amount: cargo_amount,
            },
            Transform::default(),
        ))
        .id();

    app.update();

    assert!(app.world().get_entity(nanobot).is_err());
    assert_eq!(
        app.world()
            .resource::<ResourceLedger>()
            .total_for(SwarmId::PLAYER, ResourceKind::Minerals),
        0,
    );
}

fn wait_for_rotations(app: &mut App, count: usize) {
    for _ in 0..120 {
        app.update();
        let world = app.world_mut();
        if world.query::<&ChargerAssignment>().iter(world).count() >= count {
            return;
        }
    }
    panic!("expected {count} admitted Charge rotations within 120 navigation ticks");
}
