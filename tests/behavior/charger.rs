//! Integration tests for issue #14: Chargers and defender
//! Charge sustain loop.
//!
//! Each test isolates one behavior so a failure points at a
//! single contract: charger auto-emergence from Defend Zone
//! load, charger emergence respecting existing busyness,
//! logistics dependence (a charger without material is not a
//! working rotation target), weakening of attack/defense on
//! low charge, health loss on empty/ignored charge, and the
//! automatic rotation of defenders to working chargers.
//!
//! The pure-helper unit tests (charge helpers, multipliers,
//! Charger data) live in `src/nanobot/charge.rs`.

use bevy::{math::Vec2, prelude::*};
use top_down_2d_rts_prototype_nano_swarm::{
    ai::AiPlugin,
    intent::{IntentGrid, IntentKind},
    nanobot::{
        AllocationRegion, CHARGE_DRAIN_PER_TICK, CHARGE_PER_PULSE, CHARGE_PULSE_INTERVAL_TICKS,
        CHARGER_MATERIAL_PER_PULSE, Cargo, Charge, Charger, ChargerAssignment, ChargerProgress,
        ChargerPulseProgress, DEFENDER_BASE_ATTACK, DEFENDER_BASE_DEFENSE, DefendAssignment,
        DefendHold, DirectMovementComponent, EMPTY_CHARGE_DAMAGE_INTERVAL_TICKS,
        EMPTY_CHARGE_HEALTH_DAMAGE, Health, LOW_CHARGE_THRESHOLD, LogisticsReservation, MAX_CHARGE,
        MAX_DEFENDERS_PER_CHARGER, NANOBOT_DEFAULT_MAX_HEALTH, Nanobot, NanobotBundle,
        NanobotPlugin, NanobotType, OpportunityCategory, OpportunityTarget, OwnerSwarm,
        PlannedKind, PlannedStructure, RegionalLease, RegionalLeaseState,
        SUPPORT_OPERATIONAL_HEALTH_THRESHOLD, Structure, StructureKind, Swarm, SwarmBundle,
        SwarmId, SwarmMember, WEAKENED_CHARGE_THRESHOLD, defender_charger_arrive_system,
        defender_charger_work_system, nanobot_death_cleanup_system,
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
fn full_defender_holds_until_field_endurance_threshold() {
    let mut app = build_app();
    common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let cell = IVec2::ZERO;
    app.world_mut()
        .resource_mut::<IntentGrid>()
        .paint(cell, IntentKind::Defend);
    let defender = common::spawn_defender_at(&mut app, common::cell_world_center(cell));
    app.world_mut()
        .entity_mut(defender)
        .insert(DefendHold { cell });

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
        .paint(cell, IntentKind::Defend);
    let charger = common::spawn_charger_at(&mut app, cell, 60);
    app.world_mut()
        .entity_mut(charger)
        .insert(OwnerSwarm(swarm));
    let defender = common::spawn_defender_at(&mut app, common::cell_world_center(cell));
    app.world_mut().entity_mut(defender).insert((
        ChargerAssignment {
            charger,
            source_cell: cell,
        },
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
        .paint(cell, IntentKind::Defend);
    let charger = common::spawn_charger_at(&mut app, cell, 60);
    app.world_mut()
        .entity_mut(charger)
        .insert(OwnerSwarm(swarm));
    app.world_mut().resource_mut::<ResourceLedger>().add_for(
        SwarmId::PLAYER,
        ResourceKind::Minerals,
        60,
    );
    let defender = common::spawn_defender_at(&mut app, common::cell_world_center(cell));
    app.world_mut().entity_mut(defender).insert((
        ChargerAssignment {
            charger,
            source_cell: cell,
        },
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
    app.world_mut()
        .resource_mut::<IntentGrid>()
        .paint(cell, IntentKind::Defend);
    let defender = common::spawn_defender_at(&mut app, common::cell_world_center(cell));
    app.world_mut()
        .entity_mut(defender)
        .insert(DefendHold { cell });
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
fn charger_never_serves_more_than_maximum_concurrent_defenders() {
    let mut app = build_app();
    common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let cell = IVec2::ZERO;
    app.world_mut()
        .resource_mut::<IntentGrid>()
        .paint(cell, IntentKind::Defend);
    let charger = common::spawn_charger_at(&mut app, cell, 100);
    let center = common::cell_world_center(cell);
    for index in 0..6 {
        let defender = common::spawn_defender_at(
            &mut app,
            center + Vec2::new((index as f32 - 2.5) * 4.0, 0.0),
        );
        app.world_mut()
            .entity_mut(defender)
            .insert(DefendHold { cell });
        app.world_mut()
            .entity_mut(defender)
            .get_mut::<Charge>()
            .unwrap()
            .current = LOW_CHARGE_THRESHOLD;
    }

    app.update();

    let world = app.world_mut();
    let mut assignments = world.query::<&ChargerAssignment>();
    let assigned = assignments
        .iter(world)
        .filter(|assignment| assignment.charger == charger)
        .count();
    assert_eq!(assigned, MAX_DEFENDERS_PER_CHARGER as usize);
}

#[test]
fn defend_cell_retains_holders_during_charge_rotation() {
    let mut app = build_app();
    common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let cell = IVec2::ZERO;
    app.world_mut()
        .resource_mut::<IntentGrid>()
        .paint(cell, IntentKind::Defend);
    let charger = common::spawn_charger_at(&mut app, cell, 100);
    let center = common::cell_world_center(cell);
    for index in 0..3 {
        let defender = common::spawn_defender_at(
            &mut app,
            center + Vec2::new((index as f32 - 1.0) * 8.0, 0.0),
        );
        app.world_mut()
            .entity_mut(defender)
            .insert(DefendHold { cell });
        app.world_mut()
            .entity_mut(defender)
            .get_mut::<Charge>()
            .unwrap()
            .current = LOW_CHARGE_THRESHOLD;
    }

    app.update();

    let world = app.world_mut();
    let mut holds = world.query::<&DefendHold>();
    let mut assignments = world.query::<&ChargerAssignment>();
    assert_eq!(
        holds.iter(world).filter(|hold| hold.cell == cell).count(),
        2
    );
    assert_eq!(
        assignments
            .iter(world)
            .filter(|assignment| assignment.charger == charger)
            .count(),
        1,
    );
}

#[test]
fn charging_holder_allows_replacement_without_displacement_on_return() {
    let mut app = build_app();
    common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let cell = IVec2::ZERO;
    app.world_mut()
        .resource_mut::<IntentGrid>()
        .paint(cell, IntentKind::Defend);
    let _charger = common::spawn_charger_at(&mut app, cell, 200);
    let center = common::cell_world_center(cell);
    let charging = common::spawn_defender_at(&mut app, center);
    app.world_mut()
        .entity_mut(charging)
        .insert(DefendHold { cell });
    app.world_mut()
        .entity_mut(charging)
        .get_mut::<Charge>()
        .unwrap()
        .current = 0.2;
    let replacement = common::spawn_defender_at(&mut app, center + Vec2::new(8.0, 0.0));

    for _ in 0..500 {
        app.update();
    }

    let replacement_entity = app.world().entity(replacement);
    assert!(
        replacement_entity.get::<DefendHold>().is_some()
            || replacement_entity.get::<DefendAssignment>().is_some(),
        "replacement must retain Defend work while the original holder charges"
    );
    assert!(
        app.world()
            .entity(replacement)
            .get::<RegionalLease>()
            .is_some(),
        "replacement must retain its regional lease when the original holder returns"
    );
    assert!(
        app.world().entity(charging).get::<DefendHold>().is_some()
            || app
                .world()
                .entity(charging)
                .get::<DefendAssignment>()
                .is_some()
            || app
                .world()
                .entity(charging)
                .get::<ChargerAssignment>()
                .is_some(),
        "returning holder must remain in the Defend/charge lifecycle"
    );
}

#[test]
fn material_is_not_overdrawn_under_charger_contention() {
    let mut app = App::new();
    app.insert_resource(IntentGrid::new(4, 4))
        .init_resource::<ResourceLedger>()
        .add_systems(Update, defender_charger_work_system);
    let cell = IVec2::ZERO;
    app.world_mut().resource_mut::<IntentGrid>().paint_owned(
        cell,
        IntentKind::Defend,
        Some(SwarmId::PLAYER),
    );
    let swarm = app.world_mut().spawn(SwarmBundle::default()).id();
    let mut charger_state = Charger::new(cell);
    charger_state.amount = CHARGER_MATERIAL_PER_PULSE;
    let charger = app
        .world_mut()
        .spawn((charger_state, OwnerSwarm(swarm), Transform::default()))
        .id();
    app.world_mut().resource_mut::<ResourceLedger>().add_for(
        SwarmId::PLAYER,
        ResourceKind::Minerals,
        CHARGER_MATERIAL_PER_PULSE,
    );
    for _ in 0..2 {
        app.world_mut().spawn((
            Nanobot {},
            NanobotType::Defender,
            SwarmMember::new(SwarmId::PLAYER),
            Charge {
                current: 0.5,
                max: MAX_CHARGE,
            },
            ChargerAssignment {
                charger,
                source_cell: cell,
            },
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
        app.world_mut().resource_mut::<IntentGrid>().paint_owned(
            cell,
            IntentKind::Defend,
            Some(SwarmId::PLAYER),
        );
        let swarm = app.world_mut().spawn(SwarmBundle::default()).id();
        let mut charger_state = Charger::new(cell);
        charger_state.amount = CHARGER_MATERIAL_PER_PULSE;
        let charger = app
            .world_mut()
            .spawn((charger_state, OwnerSwarm(swarm), Transform::default()))
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
                    NanobotType::Defender,
                    SwarmMember::new(SwarmId::PLAYER),
                    Charge {
                        current: 0.5,
                        max: MAX_CHARGE,
                    },
                    ChargerAssignment {
                        charger,
                        source_cell: cell,
                    },
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
    common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let cell = IVec2::ZERO;
    app.world_mut()
        .resource_mut::<IntentGrid>()
        .paint(cell, IntentKind::Defend);
    let charger = common::spawn_charger_at(&mut app, cell, 100);
    let center = common::cell_world_center(cell);
    for index in 0..4 {
        let defender = common::spawn_defender_at(
            &mut app,
            center + Vec2::new((index as f32 - 1.5) * 8.0, 0.0),
        );
        app.world_mut()
            .entity_mut(defender)
            .insert(DefendHold { cell });
        app.world_mut()
            .entity_mut(defender)
            .get_mut::<Charge>()
            .unwrap()
            .current = LOW_CHARGE_THRESHOLD;
    }

    app.update();
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
        .remove::<DirectMovementComponent>()
        .insert(DefendHold { cell });

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
fn charger_auto_emerges_in_defend_cell_with_defender_load() {
    // Acceptance: "Chargers emerge from Defend Zone load..."
    // As of issue #28, demand creates a Planned Charger
    // (not a completed Charger). A Defend-painted cell
    // with a holding defender must gain a Planned Charger
    // on the next tick. The plan lives in the cell so the
    // player can see the support structure co-located with
    // the defense; the completed Charger only appears
    // after a Worker builds the plan.
    let mut app = build_app();
    let _swarm = common::spawn_swarm_at(&mut app, Vec2::new(0.0, 0.0));
    let cell = IVec2::new(1, 0);
    app.world_mut()
        .resource_mut::<IntentGrid>()
        .paint(cell, IntentKind::Defend);
    let cell_center = common::cell_world_center(cell);
    // Spawn the defender and then plant it in a DefendHold
    // below; the auto-creation system reads "holding or
    // assigned" so a bare idle defender would not show up
    // in the load count.
    common::spawn_defender_at(&mut app, cell_center);

    // Pre-condition: zero chargers and zero planned
    // chargers.
    assert_eq!(charger_count(app.world_mut()), 0);
    assert_eq!(planned_charger_count(app.world_mut()), 0);

    // Place a defender into hold on the same cell so the
    // auto-creation system sees load.
    {
        let w = app.world_mut();
        let entity = w
            .query_filtered::<Entity, With<Nanobot>>()
            .iter(w)
            .next()
            .expect("defender was just spawned");
        w.entity_mut(entity).insert(DefendHold { cell });
    }

    app.update();

    // Demand created a planned charger; the completed
    // Charger does NOT exist yet (a Worker must build the
    // plan first).
    assert_eq!(
        planned_charger_count(app.world_mut()),
        1,
        "one planned charger must emerge from a Defend cell with a holding defender"
    );
    assert_eq!(
        charger_count(app.world_mut()),
        0,
        "no completed charger must exist before a Worker builds the plan"
    );
    // The plan is in the painted cell and at the cell's
    // world center.
    let world = app.world_mut();
    let mut q = world.query::<(&PlannedStructure, &Transform)>();
    let (planned, transform) = q
        .iter(world)
        .find(|(p, _)| p.kind == PlannedKind::Charger)
        .expect("Planned Charger exists");
    assert_eq!(planned.cell, cell);
    assert!(
        (transform.translation.truncate() - cell_center).length() < 1.0,
        "Planned Charger must be at the cell's world center"
    );
}

#[test]
fn charger_does_not_emerge_in_cell_without_load() {
    // Sanity: a Defend cell with no defenders must not
    // spawn a charger (planned or completed). The "load"
    // half of the emergence contract requires at least one
    // defender committed to the cell.
    let mut app = build_app();
    let _swarm = common::spawn_swarm_at(&mut app, Vec2::new(0.0, 0.0));
    let cell = IVec2::new(1, 0);
    app.world_mut()
        .resource_mut::<IntentGrid>()
        .paint(cell, IntentKind::Defend);

    app.update();

    assert_eq!(
        charger_count(app.world_mut()),
        0,
        "no charger without a holding defender"
    );
    assert_eq!(
        planned_charger_count(app.world_mut()),
        0,
        "no planned charger without a holding defender"
    );
}

#[test]
fn enemy_defender_does_not_create_player_charger_demand() {
    let mut app = build_app();
    common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let cell = IVec2::ZERO;
    app.world_mut().resource_mut::<IntentGrid>().paint_owned(
        cell,
        IntentKind::Defend,
        Some(SwarmId::PLAYER),
    );
    let defender = common::spawn_defender_at(&mut app, common::cell_world_center(cell));
    app.world_mut()
        .entity_mut(defender)
        .insert((SwarmMember::new(SwarmId(11)), DefendHold { cell }));

    app.update();

    assert_eq!(
        planned_charger_count(app.world_mut()),
        0,
        "hostile defenders must not count toward player charger demand",
    );
}

#[test]
fn charger_emergence_respects_existing_charger_busyness() {
    // Acceptance: "Chargers emerge from Defend Zone load AND
    // existing charger busyness." A cell with one charger
    // and many defenders must spawn additional chargers; a
    // cell with one charger and few defenders must not.
    //
    // The MAX_DEFENDERS_PER_CHARGER threshold drives the
    // emergence: 1 charger covers up to 3 defenders; 4+
    // defenders ask for a second charger. The test plants
    // 5 holding defenders in one cell, then asserts that
    // a second (planned) charger appears and the first
    // charger is still there (the existing one is not
    // destroyed).
    //
    // As of issue #28 the additional charger emerges as a
    // Planned Charger (the demand path produces a plan, a
    // Worker builds it, the completed Charger takes over).
    // The busyness count includes BOTH completed Chargers
    // AND Planned Chargers in the same cell so the
    // auto-creation loop does not pile plans.
    let mut app = build_app();
    let _swarm = common::spawn_swarm_at(&mut app, Vec2::new(0.0, 0.0));
    let cell = IVec2::new(0, 0);
    app.world_mut()
        .resource_mut::<IntentGrid>()
        .paint(cell, IntentKind::Defend);
    let cell_center = common::cell_world_center(cell);
    let _charger = common::spawn_charger_at(&mut app, cell, 100);

    // Plant 5 holding defenders in the same cell. The cell
    // already has 1 charger; with MAX_DEFENDERS_PER_CHARGER
    // = 3 the demand is 2 chargers.
    for i in 0..5 {
        // Spread them out a tiny bit so separation forces
        // do not pile them on top of each other. The cell
        // is much larger than a bot radius so a few-pixel
        // jitter is invisible to the cell-classification
        // step.
        let jitter = (i as f32 - 2.0) * 2.0;
        let d = common::spawn_defender_at(&mut app, cell_center + Vec2::new(jitter, 0.0));
        app.world_mut().entity_mut(d).insert(DefendHold { cell });
    }

    app.update();

    let completed = charger_count(app.world_mut());
    let planned = planned_charger_count(app.world_mut());
    let total = completed + planned;
    assert!(
        total >= 2,
        "busy cell must spawn an additional charger (completed={completed}, planned={planned})"
    );
    // The completed charger is still there (not destroyed).
    assert_eq!(
        completed, 1,
        "pre-existing completed charger must not be destroyed by the demand loop"
    );
    // And the demand produced at least one more plan.
    assert!(
        planned >= 1,
        "busy cell must plan at least one additional charger; got {planned}"
    );
    // All chargers (planned and completed) are in the
    // same cell.
    let world = app.world_mut();
    let mut q = world.query::<&Charger>();
    for c in q.iter(world) {
        assert_eq!(c.cell, cell);
    }
    let mut qp = world.query::<&PlannedStructure>();
    for p in qp.iter(world).filter(|p| p.kind == PlannedKind::Charger) {
        assert_eq!(p.cell, cell);
    }
}

#[test]
fn charger_does_not_emerge_extra_when_load_below_busy_threshold() {
    // Companion to the busyness test: a cell with one charger
    // and fewer defenders than MAX_DEFENDERS_PER_CHARGER must
    // not spawn a second charger. The existing one is enough.
    let mut app = build_app();
    let _swarm = common::spawn_swarm_at(&mut app, Vec2::new(0.0, 0.0));
    let cell = IVec2::new(0, 0);
    app.world_mut()
        .resource_mut::<IntentGrid>()
        .paint(cell, IntentKind::Defend);
    let cell_center = common::cell_world_center(cell);
    let _charger = common::spawn_charger_at(&mut app, cell, 100);
    // 1 charger and 2 defenders: 2 < MAX_DEFENDERS_PER_CHARGER
    // (3), so the demand is still 1 charger. No second one.
    for i in 0..2 {
        let jitter = (i as f32 - 0.5) * 4.0;
        let d = common::spawn_defender_at(&mut app, cell_center + Vec2::new(jitter, 0.0));
        app.world_mut().entity_mut(d).insert(DefendHold { cell });
    }

    app.update();

    assert_eq!(
        charger_count(app.world_mut()),
        1,
        "no extra charger when load is below the busyness threshold"
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
fn low_charge_reduces_defender_attack_and_defense() {
    // Acceptance: "Low Charge reduces Defender attack/defense."
    // The pure helper returns the multiplier; the test
    // verifies the contract by calling the helper with
    // several charge values and checking the resulting
    // attack/defense.
    use top_down_2d_rts_prototype_nano_swarm::nanobot::{
        charge_strength_multiplier, effective_attack, effective_defense,
    };

    // Full charge: full attack and full defense.
    assert!((effective_attack(MAX_CHARGE) - DEFENDER_BASE_ATTACK).abs() < 1e-5);
    assert!((effective_defense(MAX_CHARGE) - DEFENDER_BASE_DEFENSE).abs() < 1e-5);

    // At the weakened threshold: still full strength (the
    // helper is `>=` on the threshold).
    assert!((effective_attack(WEAKENED_CHARGE_THRESHOLD) - DEFENDER_BASE_ATTACK).abs() < 1e-5);
    assert!((effective_defense(WEAKENED_CHARGE_THRESHOLD) - DEFENDER_BASE_DEFENSE).abs() < 1e-5);

    // Below the weakened threshold: attack and defense scale
    // linearly. A charge of 0.1 (one third of the threshold)
    // yields a 1/3 multiplier.
    let third = WEAKENED_CHARGE_THRESHOLD / 3.0;
    let mult = charge_strength_multiplier(third);
    assert!((mult - 1.0 / 3.0).abs() < 1e-5);
    assert!((effective_attack(third) - DEFENDER_BASE_ATTACK * mult).abs() < 1e-5);
    assert!((effective_defense(third) - DEFENDER_BASE_DEFENSE * mult).abs() < 1e-5);

    // Empty charge: zero attack and zero defense.
    assert_eq!(effective_attack(0.0), 0.0);
    assert_eq!(effective_defense(0.0), 0.0);
}

#[test]
fn empty_charge_causes_defender_health_loss_when_no_charger() {
    // Acceptance: "Empty/ignored Charge causes Defender
    // health loss." A defender in DefendHold with empty
    // charge and no working charger reachable must lose
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
    app.world_mut()
        .resource_mut::<IntentGrid>()
        .paint(cell, IntentKind::Defend);
    let cell_center = common::cell_world_center(cell);
    let _charger = common::spawn_charger_at(&mut app, cell, 0);
    let defender = common::spawn_defender_at(&mut app, cell_center);
    // Empty out the charge and place the defender in hold.
    {
        let w = app.world_mut();
        w.entity_mut(defender).get_mut::<Charge>().unwrap().current = 0.0;
        w.entity_mut(defender).insert(DefendHold { cell });
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
    let _swarm = common::spawn_swarm_at(&mut app, Vec2::new(0.0, 0.0));
    let cell = IVec2::new(0, 0);
    let cell_center = common::cell_world_center(cell);
    let charger = common::spawn_charger_at(&mut app, cell, 100);
    let defender = common::spawn_defender_at(&mut app, cell_center);
    {
        let w = app.world_mut();
        w.entity_mut(defender).get_mut::<Charge>().unwrap().current = 0.0;
        w.entity_mut(defender).insert(ChargerAssignment {
            charger,
            source_cell: cell,
        });
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
fn defender_uses_charger_in_held_cell() {
    // Acceptance: "Defenders automatically rotate to working
    // chargers when low on Charge." A holding defender with
    // charge at or below LOW_CHARGE_THRESHOLD must receive a
    // ChargerAssignment aimed at a working charger; the
    // DefendHold marker is removed and its regional lease is
    // suspended.
    let mut app = build_app();
    let _swarm = common::spawn_swarm_at(&mut app, Vec2::new(0.0, 0.0));
    let cell = IVec2::new(0, 0);
    app.world_mut()
        .resource_mut::<IntentGrid>()
        .paint(cell, IntentKind::Defend);
    let cell_center = common::cell_world_center(cell);
    let _charger = common::spawn_charger_at(&mut app, cell, 100);
    let defender = common::spawn_defender_at(&mut app, cell_center);
    {
        let w = app.world_mut();
        // Charge at exactly the low threshold: must trigger rotation.
        w.entity_mut(defender).get_mut::<Charge>().unwrap().current = LOW_CHARGE_THRESHOLD;
        w.entity_mut(defender).insert(DefendHold { cell });
    }

    // The test directly inserts `DefendHold`; the rotation
    // path still routes it through the local charger contract.

    app.update();

    // Post-rotation: the defender has a ChargerAssignment
    // and no DefendHold.
    let world = app.world();
    let has_charger_assignment = world.entity(defender).get::<ChargerAssignment>().is_some();
    assert!(
        has_charger_assignment,
        "low-charge defender must be assigned to a charger"
    );
    assert!(
        world.entity(defender).get::<DefendHold>().is_none(),
        "DefendHold must be removed when the defender rotates to a charger"
    );
    // The charger is the right one.
    let assignment = world.entity(defender).get::<ChargerAssignment>().unwrap();
    assert_eq!(assignment.charger, _charger);
}

#[test]
fn defender_ignores_closer_enemy_charger() {
    let mut app = build_app();
    let player = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let enemy = app
        .world_mut()
        .spawn((Swarm {}, SwarmId(11), Transform::default()))
        .id();
    let hold_cell = IVec2::ZERO;
    app.world_mut().resource_mut::<IntentGrid>().paint_owned(
        hold_cell,
        IntentKind::Defend,
        Some(SwarmId::PLAYER),
    );

    let enemy_charger = common::spawn_charger_at(&mut app, hold_cell, 100);
    app.world_mut()
        .entity_mut(enemy_charger)
        .insert(OwnerSwarm(enemy));
    let owned_charger = common::spawn_charger_at(&mut app, hold_cell, 100);
    app.world_mut()
        .entity_mut(owned_charger)
        .insert(OwnerSwarm(player));

    let defender = common::spawn_defender_at(&mut app, common::cell_world_center(hold_cell));
    app.world_mut().entity_mut(defender).insert((
        SwarmMember::new(SwarmId::PLAYER),
        DefendHold { cell: hold_cell },
    ));
    app.world_mut()
        .entity_mut(defender)
        .get_mut::<Charge>()
        .unwrap()
        .current = LOW_CHARGE_THRESHOLD;

    app.update();

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
fn remote_charger_does_not_pull_defender_off_front() {
    let mut app = build_app();
    let swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let source_cell = IVec2::ZERO;
    app.world_mut().resource_mut::<IntentGrid>().paint_owned(
        source_cell,
        IntentKind::Defend,
        Some(SwarmId::PLAYER),
    );
    let remote = common::spawn_charger_at(&mut app, IVec2::new(1, 0), 100);
    app.world_mut().entity_mut(remote).insert(OwnerSwarm(swarm));
    let defender = common::spawn_defender_at(&mut app, common::cell_world_center(source_cell));
    app.world_mut()
        .entity_mut(defender)
        .insert(DefendHold { cell: source_cell });
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
    assert!(app.world().entity(defender).get::<DefendHold>().is_some());
}

#[test]
fn invalidated_assignment_releases_capacity_and_resumes_lease() {
    let mut app = App::new();
    app.insert_resource(IntentGrid::new(8, 8));
    app.init_resource::<ResourceLedger>();
    app.add_systems(Update, defender_charger_work_system);
    let cell = IVec2::ZERO;
    let charger = common::spawn_charger_at(&mut app, cell, 100);
    let mut lease = RegionalLease::new(
        AllocationRegion::for_cell(cell),
        OpportunityCategory::Defend,
        OpportunityTarget::Defend { cell },
        Some(SwarmId::PLAYER),
        0,
        0,
        30,
    );
    lease.suspend_for_charge();
    let defender = app
        .world_mut()
        .spawn((
            Nanobot {},
            NanobotType::Defender,
            SwarmMember::new(SwarmId::PLAYER),
            Health::default(),
            Charge::default(),
            Transform::from_translation(common::cell_world_center(cell).extend(0.0)),
            ChargerAssignment {
                charger,
                source_cell: cell,
            },
            ChargerProgress { charger },
            ChargerPulseProgress::default(),
            lease,
        ))
        .id();

    app.update();

    let world = app.world();
    assert!(world.entity(defender).get::<ChargerAssignment>().is_none());
    assert!(world.entity(defender).get::<ChargerProgress>().is_none());
    assert_eq!(
        world.entity(defender).get::<RegionalLease>().unwrap().state,
        RegionalLeaseState::ResumePending,
    );
}

#[test]
fn emptied_charger_cancels_en_route_assignment_before_arrival() {
    let mut app = App::new();
    let cell = IVec2::ZERO;
    app.insert_resource(IntentGrid::new(8, 8));
    app.world_mut().resource_mut::<IntentGrid>().paint_owned(
        cell,
        IntentKind::Defend,
        Some(SwarmId::PLAYER),
    );
    let charger = app
        .world_mut()
        .spawn((
            Charger::new(cell),
            Transform::from_translation(common::cell_world_center(cell).extend(0.0)),
        ))
        .id();
    let mut lease = RegionalLease::new(
        AllocationRegion::for_cell(cell),
        OpportunityCategory::Defend,
        OpportunityTarget::Defend { cell },
        Some(SwarmId::PLAYER),
        0,
        0,
        30,
    );
    lease.suspend_for_charge();
    let defender = app
        .world_mut()
        .spawn((
            Nanobot {},
            NanobotType::Defender,
            SwarmMember::new(SwarmId::PLAYER),
            Transform::from_translation(Vec2::ZERO.extend(0.0)),
            ChargerAssignment {
                charger,
                source_cell: cell,
            },
            DirectMovementComponent {
                xy: common::cell_world_center(cell),
                stop_radius: 0.0,
            },
            lease,
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
    assert_eq!(
        world.entity(defender).get::<RegionalLease>().unwrap().state,
        RegionalLeaseState::ResumePending,
    );
}

#[test]
fn defender_does_not_rotate_to_empty_charger() {
    // Companion: a charger with no material is not a
    // "working" rotation target, so a low-charge defender
    // must not rotate to it. The defender stays in hold and
    // the empty-charge health loss system fires (covered by
    // another test).
    let mut app = build_app();
    let _swarm = common::spawn_swarm_at(&mut app, Vec2::new(0.0, 0.0));
    let cell = IVec2::new(0, 0);
    app.world_mut()
        .resource_mut::<IntentGrid>()
        .paint(cell, IntentKind::Defend);
    let cell_center = common::cell_world_center(cell);
    // Charger with NO material: not a working rotation target.
    let _charger = common::spawn_charger_at(&mut app, cell, 0);
    let defender = common::spawn_defender_at(&mut app, cell_center);
    {
        let w = app.world_mut();
        w.entity_mut(defender).get_mut::<Charge>().unwrap().current = LOW_CHARGE_THRESHOLD;
        w.entity_mut(defender).insert(DefendHold { cell });
    }

    app.update();

    let world = app.world();
    assert!(
        world.entity(defender).get::<ChargerAssignment>().is_none(),
        "defender must not rotate to an empty charger"
    );
    assert!(
        world.entity(defender).get::<DefendHold>().is_some(),
        "defender must stay in hold when no working charger is available"
    );
}

#[test]
fn defender_does_not_rotate_to_degraded_charger() {
    let mut app = build_app();
    common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let cell = IVec2::ZERO;
    app.world_mut()
        .resource_mut::<IntentGrid>()
        .paint(cell, IntentKind::Defend);
    let charger = common::spawn_charger_at(&mut app, cell, 100);
    let mut condition = Structure::new(StructureKind::Basic);
    condition.health = SUPPORT_OPERATIONAL_HEALTH_THRESHOLD - 1;
    app.world_mut().entity_mut(charger).insert(condition);
    let defender = common::spawn_defender_at(&mut app, common::cell_world_center(cell));
    app.world_mut()
        .entity_mut(defender)
        .insert(DefendHold { cell });
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
fn defender_charges_at_a_working_charger_and_returns_to_defend() {
    // End-to-end: a defender in DefendHold with low charge
    // rotates to a working charger, the work system refills
    // the charge, and once the charge is full the work
    // system releases the defender so the defend pool can
    // re-assign them. The final assertion is that the
    // defender is again in DefendHold after enough ticks.
    let mut app = build_app();
    let _swarm = common::spawn_swarm_at(&mut app, Vec2::new(0.0, 0.0));
    let cell = IVec2::new(0, 0);
    app.world_mut()
        .resource_mut::<IntentGrid>()
        .paint(cell, IntentKind::Defend);
    let cell_center = common::cell_world_center(cell);
    let _charger = common::spawn_charger_at(&mut app, cell, 200);
    let defender = common::spawn_defender_at(&mut app, cell_center);
    {
        let w = app.world_mut();
        w.entity_mut(defender).get_mut::<Charge>().unwrap().current = 0.2;
        w.entity_mut(defender).insert(DefendHold { cell });
    }

    // Run enough ticks for: rotation (1) + travel (~1
    // tick at the same cell) + arrival (1) + charge refill
    // Nineteen supplied pulses move charge from 0.2 to full;
    // the remaining budget covers reassignment, travel, and
    // hold detection.
    for _ in 0..500 {
        app.update();
    }

    // The defender's charge is somewhere in the cycle:
    // either fully charged after a recent refill, or
    // between LOW_CHARGE_THRESHOLD and MAX_CHARGE while
    // holding the cell. The cycle is "hold -> drain ->
    // rotate to charger -> refill -> release -> hold".
    // The test pins the observable shape of the cycle: the
    // defender is in the defend hold (released from the
    // charger) and the charger has been drained by the
    // per-tick rate.
    assert!(
        app.world().entity(defender).get::<DefendHold>().is_some(),
        "defender must return to DefendHold after charging"
    );
    let charge = read_charge(&app, defender).expect("defender has Charge");
    assert!(
        charge >= LOW_CHARGE_THRESHOLD,
        "defender in DefendHold must have charge above the rotation threshold; got {charge}"
    );
    assert!(
        charge <= MAX_CHARGE,
        "charge cannot exceed the cap; got {charge}"
    );
    // No leftover charger markers.
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
    // The charger's material has been drained by the per-tick
    // rate. The exact amount is not pinned because the test
    // also allows for ticks where the defender was in
    // transit, but it must be strictly less than the
    // initial 200.
    let c = app.world().entity(_charger).get::<Charger>().unwrap();
    assert!(
        c.amount < 200,
        "charger material must be drained while defenders charge from it; got {}",
        c.amount
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
    // end: a defender at a held cell with an empty charger
    // in the same cell must end up in worse health than a
    // defender at the same cell with a stocked charger.
    let mut app_empty = build_app();
    {
        let _swarm = common::spawn_swarm_at(&mut app_empty, Vec2::new(0.0, 0.0));
        let cell = IVec2::new(0, 0);
        app_empty
            .world_mut()
            .resource_mut::<IntentGrid>()
            .paint(cell, IntentKind::Defend);
        let cell_center = common::cell_world_center(cell);
        // Pre-spawn an empty charger so the auto-creation
        // system does not also create a working charger
        // (which would let the rotation chain absorb the
        // defender and stop the health loss). The empty
        // charger is "not working" because it has no supply.
        let _charger = common::spawn_charger_at(&mut app_empty, cell, 0);
        let defender = common::spawn_defender_at(&mut app_empty, cell_center);
        {
            let w = app_empty.world_mut();
            w.entity_mut(defender).get_mut::<Charge>().unwrap().current = 0.0;
            w.entity_mut(defender).insert(DefendHold { cell });
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
        let _swarm = common::spawn_swarm_at(&mut app_filled, Vec2::new(0.0, 0.0));
        let cell = IVec2::new(0, 0);
        app_filled
            .world_mut()
            .resource_mut::<IntentGrid>()
            .paint(cell, IntentKind::Defend);
        let cell_center = common::cell_world_center(cell);
        let _charger = common::spawn_charger_at(&mut app_filled, cell, 200);
        let defender = common::spawn_defender_at(&mut app_filled, cell_center);
        {
            let w = app_filled.world_mut();
            w.entity_mut(defender).get_mut::<Charge>().unwrap().current = LOW_CHARGE_THRESHOLD;
            w.entity_mut(defender).insert(DefendHold { cell });
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
    // The "logistics support" half of the contract is the
    // physical resource flow: haulers can deliver minerals
    // to a charger with free space. A stocked-up charger
    // (via hauler delivery) is what keeps a defended cell
    // supplied when the player is not actively painting
    // intent. The test plants a deposit, a hauler, and a
    // charger with zero amount, then asserts the hauler
    // routes to the charger and the charger's amount grows.
    let mut app = build_app();
    let swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let source_pos = Vec2::new(100.0, 0.0);
    let cell = IVec2::new(2, 0);
    // Terminal leg source: a sink stockpile. Chargers cannot
    // bypass shared staging by drawing from source stockpiles.
    let source = common::spawn_sink_stockpile(&mut app, source_pos, 1000, 1000);
    let charger = common::spawn_charger_at(&mut app, cell, 0);
    app.world_mut().entity_mut(source).insert(OwnerSwarm(swarm));
    app.world_mut()
        .entity_mut(charger)
        .insert(OwnerSwarm(swarm));
    let _hauler = common::spawn_hauler_at(&mut app, source_pos);
    // Paint a Defend cell so the charger auto-creation
    // system would not also create one (we have a manual
    // charger). The system only creates chargers in cells
    // with a holding defender; without a defender the
    // system does nothing.
    app.world_mut()
        .resource_mut::<IntentGrid>()
        .paint(cell, IntentKind::Defend);

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
    // The defender stays in hold.
    let mut app = build_app();
    let _swarm = common::spawn_swarm_at(&mut app, Vec2::new(0.0, 0.0));
    let cell = IVec2::new(0, 0);
    app.world_mut()
        .resource_mut::<IntentGrid>()
        .paint(cell, IntentKind::Defend);
    let cell_center = common::cell_world_center(cell);
    let _charger = common::spawn_charger_at(&mut app, cell, 100);
    let defender = common::spawn_defender_at(&mut app, cell_center);
    {
        let w = app.world_mut();
        // Above LOW_CHARGE_THRESHOLD: still strong, no rotation.
        w.entity_mut(defender).get_mut::<Charge>().unwrap().current = LOW_CHARGE_THRESHOLD + 0.1;
        w.entity_mut(defender).insert(DefendHold { cell });
    }

    app.update();

    let world = app.world();
    assert!(
        world.entity(defender).get::<ChargerAssignment>().is_none(),
        "fully-charged defender must not rotate to a charger"
    );
    assert!(
        world.entity(defender).get::<DefendHold>().is_some(),
        "defender must stay in hold when charge is above the rotation threshold"
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
    let _swarm = common::spawn_swarm_at(&mut app, Vec2::new(0.0, 0.0));
    let cell = IVec2::new(0, 0);
    app.world_mut()
        .resource_mut::<IntentGrid>()
        .paint(cell, IntentKind::Defend);
    let cell_center = common::cell_world_center(cell);
    let charger = common::spawn_charger_at(&mut app, cell, 100);
    let defender = common::spawn_defender_at(&mut app, cell_center);
    {
        let w = app.world_mut();
        w.entity_mut(defender).get_mut::<Charge>().unwrap().current = 0.5;
        // Plant the defender in the charging state directly
        // so arrival mechanics are not in the way.
        w.entity_mut(defender).insert(ChargerAssignment {
            charger,
            source_cell: cell,
        });
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
fn defender_charger_assignment_does_not_block_defend_reassignment() {
    // Companion: a defender in the charging state must not
    // be re-routed to a fresh Defend cell by the defend
    // assignment system. The defend assignment system
    // filters out ChargerAssignment and ChargerProgress,
    // so a charging defender stays charging until the work
    // system releases them.
    let mut app = build_app();
    let _swarm = common::spawn_swarm_at(&mut app, Vec2::new(0.0, 0.0));
    let cell = IVec2::new(0, 0);
    let other_cell = IVec2::new(2, 0);
    app.world_mut()
        .resource_mut::<IntentGrid>()
        .paint(cell, IntentKind::Defend);
    app.world_mut()
        .resource_mut::<IntentGrid>()
        .paint(other_cell, IntentKind::Defend);
    let cell_center = common::cell_world_center(cell);
    let charger = common::spawn_charger_at(&mut app, cell, 100);
    let defender = common::spawn_defender_at(&mut app, cell_center);
    {
        let w = app.world_mut();
        w.entity_mut(defender).get_mut::<Charge>().unwrap().current = 0.1;
        w.entity_mut(defender).insert(ChargerAssignment {
            charger,
            source_cell: cell,
        });
        w.entity_mut(defender).insert(ChargerProgress { charger });
    }

    // The defend assignment system would normally pick up a
    // defender with no DefendHold and no DefendAssignment
    // and re-route them. The ChargerAssignment /
    // ChargerProgress filters must keep this defender out
    // of the routing pool.
    app.update();

    let world = app.world();
    assert!(
        world.entity(defender).get::<DefendAssignment>().is_none(),
        "defender with ChargerAssignment must not receive a DefendAssignment"
    );
    assert!(
        world.entity(defender).get::<DefendHold>().is_none(),
        "defender with ChargerAssignment must not enter DefendHold"
    );
}

#[test]
fn charger_work_consumes_owning_swarm_resources() {
    let mut app = App::new();
    app.init_resource::<ResourceLedger>()
        .add_systems(Update, defender_charger_work_system);
    let swarm = app.world_mut().spawn(SwarmBundle::default()).id();
    let mut charger_component = Charger::new(IVec2::ZERO);
    charger_component.amount = 10;
    let charger = app
        .world_mut()
        .spawn((charger_component, OwnerSwarm(swarm)))
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
            Charge {
                current: 0.5,
                max: MAX_CHARGE,
            },
            ChargerAssignment {
                charger,
                source_cell: IVec2::ZERO,
            },
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
