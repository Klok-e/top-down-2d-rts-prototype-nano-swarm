use approx::assert_abs_diff_eq;
use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    intent::{IntentGrid, IntentKind},
    nanobot::{
        Charge, Charger, ChargerAssignment, ChargerProgress, ChargerPulseProgress,
        DefenderAttackCooldown, DefenderResponse, EmptyChargeProgress, Health, Nanobot,
        NanobotBundle, NanobotType, OwnerSwarm, ResolvedCombatFact, Structure, StructureKind,
        Swarm, SwarmId, defender_charge_drain_system, defender_charger_work_system,
        defender_combat_system, defender_health_loss_when_empty_system, reconcile_charge_tuning,
        reconcile_combat_tuning,
    },
    resources::{ResourceKind, ResourceLedger},
    tuning::Tuning,
};

#[test]
fn combat_edits_change_existing_defender_reach_damage_and_attack_cadence() {
    let mut app = App::new();
    app.init_resource::<Tuning>()
        .add_message::<ResolvedCombatFact>()
        .add_systems(Update, defender_combat_system);
    let victim = app
        .world_mut()
        .spawn((
            NanobotBundle::default(),
            Transform::from_xyz(140.0, 0.0, 0.0),
        ))
        .id();
    let attacker = app
        .world_mut()
        .spawn((
            NanobotBundle {
                nanobot_type: NanobotType::Defender,
                swarm_member: top_down_2d_rts_prototype_nano_swarm::nanobot::SwarmMember(SwarmId(
                    1,
                )),
                ..default()
            },
            Transform::default(),
            Charge::default(),
            DefenderResponse { target: victim },
        ))
        .id();
    app.update();
    assert_eq!(app.world().get::<Health>(victim).unwrap().current, 100);

    let old = app.world().resource::<Tuning>().clone();
    let mut new = old.clone();
    new.combat.attack_range = 160.0;
    new.combat.base_attack = 24.0;
    new.combat.attack_interval_ticks = 4;
    app.insert_resource(new.clone());
    reconcile_combat_tuning(app.world_mut(), &old, &new);
    app.update();
    assert_eq!(app.world().get::<Health>(victim).unwrap().current, 76);
    assert_eq!(
        app.world()
            .get::<DefenderAttackCooldown>(attacker)
            .unwrap()
            .ticks_remaining,
        3
    );
    for _ in 0..3 {
        app.update();
    }
    assert_eq!(app.world().get::<Health>(victim).unwrap().current, 76);
    app.update();
    assert_eq!(app.world().get::<Health>(victim).unwrap().current, 52);
}

#[test]
fn combat_and_charge_reconcile_preserve_life_progress_and_material_custody() {
    let mut world = World::new();
    let old = Tuning::default();
    let mut new = old.clone();
    new.combat.nanobot_max_health = 200;
    new.combat.attack_interval_ticks = 60;
    new.charge.max_charge = 2.0;
    new.charge.pulse_interval_ticks = 300;
    new.charge.per_pulse = 0.06;
    new.charge.empty_damage_interval_ticks = 12;
    new.charge.charger_capacity = 10;
    new.charge.max_defenders_per_charger = 1;
    let charger = world
        .spawn(Charger {
            amount: 90,
            ..Charger::new(IVec2::ZERO)
        })
        .id();
    let living = world
        .spawn((
            Nanobot {},
            Health {
                current: 25,
                max: 100,
            },
            Charge {
                current: 0.25,
                max: 1.0,
            },
            DefenderAttackCooldown {
                ticks_remaining: 15,
            },
            ChargerPulseProgress { ticks_elapsed: 75 },
            EmptyChargeProgress { ticks_elapsed: 3 },
            ChargerAssignment { charger },
            ChargerProgress { charger },
        ))
        .id();
    let dead = world
        .spawn((
            Nanobot {},
            Health {
                current: 0,
                max: 100,
            },
        ))
        .id();
    let fragile = world
        .spawn((
            Nanobot {},
            Health {
                current: 1,
                max: 1000,
            },
        ))
        .id();

    reconcile_combat_tuning(&mut world, &old, &new);
    reconcile_charge_tuning(&mut world, &old, &new);

    let health = world.get::<Health>(living).unwrap();
    assert_eq!((health.current, health.max), (50, 200));
    assert_eq!(world.get::<Health>(dead).unwrap().current, 0);
    assert_eq!(world.get::<Health>(fragile).unwrap().current, 1);
    assert_abs_diff_eq!(world.get::<Charge>(living).unwrap().current, 0.5);
    assert_abs_diff_eq!(world.get::<Charge>(living).unwrap().max, 2.0);
    assert_eq!(
        world
            .get::<DefenderAttackCooldown>(living)
            .unwrap()
            .ticks_remaining,
        30
    );
    assert_eq!(
        world
            .get::<ChargerPulseProgress>(living)
            .unwrap()
            .ticks_elapsed,
        150
    );
    assert_eq!(
        world
            .get::<EmptyChargeProgress>(living)
            .unwrap()
            .ticks_elapsed,
        6
    );
    let buffer = world.get::<Charger>(charger).unwrap();
    assert_eq!(
        (buffer.amount, buffer.capacity, buffer.free_space()),
        (90, 10, 0)
    );
    assert_eq!(
        world.get::<ChargerAssignment>(living).unwrap().charger,
        charger
    );
    assert_eq!(
        world.get::<ChargerProgress>(living).unwrap().charger,
        charger
    );
}

#[test]
fn supplied_charge_uses_edited_pulses_and_empty_damage_honors_one_tick_interval() {
    let mut app = App::new();
    let mut tuning = Tuning::default();
    tuning.charge.drain_per_tick = 0.002;
    tuning.charge.pulse_interval_ticks = 2;
    tuning.charge.per_pulse = 0.2;
    tuning.charge.material_per_pulse = 7;
    tuning.charge.empty_damage_interval_ticks = 1;
    tuning.charge.empty_health_damage = 4;
    app.insert_resource(tuning)
        .insert_resource(IntentGrid::new(4, 4))
        .init_resource::<ResourceLedger>()
        .add_systems(
            Update,
            (
                defender_charge_drain_system,
                defender_health_loss_when_empty_system,
                defender_charger_work_system,
            )
                .chain(),
        );
    let owner = app.world_mut().spawn((Swarm {}, SwarmId::PLAYER)).id();
    app.world_mut().resource_mut::<IntentGrid>().paint(
        IVec2::ZERO,
        IntentKind::Defend,
        SwarmId::PLAYER,
    );
    let charger = app
        .world_mut()
        .spawn((
            Charger {
                amount: 20,
                ..Charger::new(IVec2::ZERO)
            },
            OwnerSwarm(owner),
            Structure::new(StructureKind::Basic),
            Transform::default(),
        ))
        .id();
    app.world_mut().resource_mut::<ResourceLedger>().add_for(
        SwarmId::PLAYER,
        ResourceKind::Minerals,
        20,
    );
    let charging = app
        .world_mut()
        .spawn((
            NanobotBundle {
                nanobot_type: NanobotType::Defender,
                ..default()
            },
            Transform::from_xyz(68.0, 0.0, 0.0),
            Charge {
                current: 0.5,
                max: 1.0,
            },
            ChargerAssignment { charger },
            ChargerProgress { charger },
            ChargerPulseProgress::default(),
        ))
        .id();
    let empty = app
        .world_mut()
        .spawn((
            NanobotBundle {
                nanobot_type: NanobotType::Defender,
                ..default()
            },
            Charge {
                current: 0.0,
                max: 1.0,
            },
        ))
        .id();

    app.update();
    assert_abs_diff_eq!(
        app.world().get::<Charge>(charging).unwrap().current,
        0.498,
        epsilon = 1e-5
    );
    assert_eq!(app.world().get::<Charger>(charger).unwrap().amount, 20);
    assert_eq!(app.world().get::<Health>(empty).unwrap().current, 96);
    app.update();
    assert_abs_diff_eq!(
        app.world().get::<Charge>(charging).unwrap().current,
        0.696,
        epsilon = 1e-5
    );
    assert_eq!(app.world().get::<Charger>(charger).unwrap().amount, 13);
    assert_eq!(
        app.world()
            .resource::<ResourceLedger>()
            .total_for(SwarmId::PLAYER, ResourceKind::Minerals),
        13
    );
    assert_eq!(app.world().get::<Health>(empty).unwrap().current, 92);
}
