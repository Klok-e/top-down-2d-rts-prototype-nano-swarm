#[path = "../common/mod.rs"]
mod common;

use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::nanobot::*;

#[test]
fn maintenance_suspends_when_displaced_and_resumes_outside_scaled_rotated_structure() {
    let mut app = common::sim_app();
    app.add_systems(
        Update,
        (
            worker_maintenance_arrive_system,
            worker_maintenance_work_system,
        )
            .chain(),
    );
    let target = common::spawn_structure_at(&mut app, Vec2::ZERO);
    app.world_mut().entity_mut(target).insert(
        Transform::from_rotation(Quat::from_rotation_z(std::f32::consts::FRAC_PI_2))
            .with_scale(Vec3::new(2.0, 1.0, 1.0)),
    );
    app.world_mut().get_mut::<Structure>(target).unwrap().health = 20;
    let worker = common::spawn_worker_at(&mut app, Vec2::new(0.0, 100.0));
    app.world_mut()
        .entity_mut(worker)
        .insert(MaintenanceAssignment {
            cell: IVec2::ZERO,
            target,
        });
    app.update();
    assert_eq!(app.world().get::<Structure>(target).unwrap().health, 22);
    app.world_mut()
        .get_mut::<Transform>(worker)
        .unwrap()
        .translation
        .y = 140.0;
    app.update();
    assert_eq!(
        app.world().get::<Structure>(target).unwrap().health,
        22,
        "displaced worker must not repair remotely"
    );
    for _ in 0..40 {
        app.update();
        if app.world().get::<Structure>(target).unwrap().health > 22 {
            break;
        }
    }
    assert!(app.world().get::<Structure>(target).unwrap().health > 22);
    let position = app.world().get::<Transform>(worker).unwrap().translation;
    assert!(
        (98.0..=102.01).contains(&position.y),
        "body needs 34 units beyond the 64-unit rotated half extent: {position}"
    );
}

#[test]
fn gathering_preserves_cargo_on_displacement_then_unloads_outside_owned_scaled_stockpile() {
    use top_down_2d_rts_prototype_nano_swarm::resources::{ResourceDeposit, Stockpile};
    for swarm_id in [SwarmId::PLAYER, SwarmId(7)] {
        let mut app = common::sim_app();
        app.add_systems(
            Update,
            (
                worker_gather_arrive_system,
                worker_gather_extract_system,
                worker_gather_carry_assign_system,
                worker_gather_delivery_system,
            )
                .chain(),
        );
        let owner = common::spawn_swarm_at(&mut app, Vec2::ZERO);
        app.world_mut().entity_mut(owner).insert(swarm_id);
        let deposit = common::spawn_deposit(
            &mut app,
            common::DepositFixture {
                world_pos: Vec2::ZERO,
                amount: 20,
                capacity: 20,
                radius: 64.0,
            },
        );
        let stockpile = common::spawn_stockpile(&mut app, Vec2::new(240.0, 0.0), 0, 20);
        app.world_mut().entity_mut(stockpile).insert((
            OwnerSwarm(owner),
            Transform::from_xyz(240.0, 0.0, 0.0).with_scale(Vec3::new(2.0, 1.0, 1.0)),
        ));
        let worker = common::spawn_worker_at(&mut app, Vec2::new(100.0, 0.0));
        app.world_mut().entity_mut(worker).insert((
            SwarmMember(swarm_id),
            GatherAssignment::new(IVec2::ZERO, deposit),
        ));
        app.update();
        app.update();
        assert_eq!(
            app.world().get::<ResourceDeposit>(deposit).unwrap().amount,
            19
        );
        assert_eq!(app.world().get::<Cargo>(worker).unwrap().amount, 1);
        app.world_mut()
            .get_mut::<Transform>(worker)
            .unwrap()
            .translation
            .x = 140.0;
        app.update();
        assert_eq!(
            app.world().get::<ResourceDeposit>(deposit).unwrap().amount,
            19,
            "no remote extraction"
        );
        assert_eq!(
            app.world().get::<Cargo>(worker).unwrap().amount,
            1,
            "displacement retains cargo"
        );
        for _ in 0..120 {
            app.update();
            if app.world().get::<ExtractProgress>(worker).is_some() {
                let position = app
                    .world()
                    .get::<Transform>(worker)
                    .unwrap()
                    .translation
                    .truncate();
                assert!(
                    position.length() >= 98.0 - 0.01,
                    "body remains outside deposit: {position}"
                );
            }
            if app.world().get::<Stockpile>(stockpile).unwrap().amount == 4 {
                break;
            }
        }
        assert_eq!(app.world().get::<Stockpile>(stockpile).unwrap().amount, 4);
        assert_eq!(
            app.world().get::<ResourceDeposit>(deposit).unwrap().amount,
            16
        );
        let position = app.world().get::<Transform>(worker).unwrap().translation;
        assert!(
            (138.0..=142.01).contains(&position.x),
            "worker delivers beyond scaled left edge with body clearance: {position}"
        );
    }
}

#[test]
fn construction_pauses_after_displacement_and_finishes_from_exterior() {
    let mut app = common::sim_app();
    app.add_systems(
        Update,
        (
            worker_planned_structure_arrive_system,
            worker_planned_structure_work_system,
        )
            .chain(),
    );
    let target = common::spawn_planned_structure_at_cell(&mut app, IVec2::ZERO);
    app.world_mut()
        .entity_mut(target)
        .insert(Transform::from_scale(Vec3::new(2.0, 1.0, 1.0)));
    app.world_mut()
        .get_mut::<PlannedStructure>(target)
        .unwrap()
        .work_remaining = 4;
    let worker = common::spawn_worker_at(&mut app, Vec2::new(100.0, 0.0));
    app.world_mut()
        .entity_mut(worker)
        .insert(PlannedStructureClaim {
            cell: IVec2::ZERO,
            target,
        });
    app.world_mut()
        .get_mut::<PlannedStructure>(target)
        .unwrap()
        .active_worker = Some(worker);
    app.update();
    assert_eq!(
        app.world()
            .get::<PlannedStructure>(target)
            .unwrap()
            .work_remaining,
        3
    );
    app.world_mut()
        .get_mut::<Transform>(worker)
        .unwrap()
        .translation
        .x = 140.0;
    app.update();
    assert_eq!(
        app.world()
            .get::<PlannedStructure>(target)
            .unwrap()
            .work_remaining,
        3
    );
    for _ in 0..60 {
        app.update();
        if app.world().get::<PlannedStructure>(target).is_none() {
            break;
        }
    }
    assert!(
        app.world().get::<PlannedStructure>(target).is_none(),
        "exterior worker completes construction"
    );
    let position = app.world().get::<Transform>(worker).unwrap().translation;
    assert!(
        (98.0..=102.01).contains(&position.x),
        "completed footprint must not contain worker body: {position}"
    );
}

#[test]
fn charging_resumes_from_exterior_without_spending_supply_while_displaced() {
    use top_down_2d_rts_prototype_nano_swarm::{
        intent::{IntentGrid, IntentKind},
        resources::{ResourceKind, ResourceLedger},
    };
    for swarm_id in [SwarmId::PLAYER, SwarmId(7)] {
        let mut app = common::sim_app();
        app.add_systems(
            Update,
            (defender_charger_arrive_system, defender_charger_work_system).chain(),
        );
        let owner = common::spawn_swarm_at(&mut app, Vec2::ZERO);
        app.world_mut().entity_mut(owner).insert(swarm_id);
        app.world_mut().resource_mut::<IntentGrid>().paint_owned(
            IVec2::ZERO,
            IntentKind::Defend,
            Some(swarm_id),
        );
        let charger = common::spawn_charger(
            &mut app,
            common::ChargerFixture {
                cell: IVec2::ZERO,
                amount: 20,
                ticks_since_maintained: 0,
            },
        );
        app.world_mut().entity_mut(charger).insert((
            OwnerSwarm(owner),
            Transform::from_scale(Vec3::new(2.0, 1.0, 1.0)),
        ));
        app.world_mut().resource_mut::<ResourceLedger>().add_for(
            swarm_id,
            ResourceKind::Minerals,
            20,
        );
        let defender = common::spawn_defender_at(&mut app, Vec2::new(100.0, 0.0));
        app.world_mut().entity_mut(defender).insert((
            SwarmMember(swarm_id),
            Charge {
                current: 0.1,
                max: 1.0,
            },
            ChargerAssignment { charger },
        ));
        for _ in 0..10 {
            app.update();
        }
        assert!(
            app.world().get::<Charge>(defender).unwrap().current > 0.1,
            "supplied exterior charger restores charge"
        );
        let charge_before = app.world().get::<Charge>(defender).unwrap().current;
        let supply_before = app.world().get::<Charger>(charger).unwrap().amount;
        app.world_mut()
            .get_mut::<Transform>(defender)
            .unwrap()
            .translation
            .x = 300.0;
        app.update();
        assert_eq!(
            app.world().get::<Charger>(charger).unwrap().amount,
            supply_before
        );
        assert!(
            (app.world().get::<Charge>(defender).unwrap().current - charge_before).abs() < 0.00001
        );
        for _ in 0..140 {
            app.update();
            if app.world().get::<Charge>(defender).unwrap().current > charge_before {
                break;
            }
        }
        assert!(app.world().get::<Charge>(defender).unwrap().current > charge_before);
        assert!(app.world().get::<Charger>(charger).unwrap().amount < supply_before);
        let position = app.world().get::<Transform>(defender).unwrap().translation;
        assert!(
            (98.0..=102.01).contains(&position.x),
            "charging body stays outside scaled charger: {position}"
        );
    }
}
