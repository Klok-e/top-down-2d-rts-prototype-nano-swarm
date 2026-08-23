#[path = "../common/mod.rs"]
mod common;

use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    intent::{IntentGrid, IntentKind, UNCONTESTED_CAPTURE_TICKS},
    nanobot::{
        Charge, CombatAppearance, CombatPlugin, DefendHold, DefendPressure, DefenderAttackCooldown,
        DirectMovementComponent, EMPTY_CHARGE_DAMAGE_INTERVAL_TICKS, Health, NanobotType,
        OwnerSwarm, PlannedKind, ResolvedCombatDeath, ResolvedCombatFact, Structure,
        StructureCombatAppearance, StructureKind, Swarm, SwarmId, SwarmMember,
        defender_health_loss_when_empty_system, nanobot_death_cleanup_system,
    },
    structure_sprites::StructureVisual,
};

fn resolved_facts(app: &App) -> Vec<ResolvedCombatFact> {
    let messages = app.world().resource::<Messages<ResolvedCombatFact>>();
    let mut cursor = messages.get_cursor();
    cursor.read(messages).copied().collect()
}

#[test]
fn delivered_hit_publishes_the_resolved_combat_snapshot() {
    let mut app = common::sim_app_with_defend();
    app.add_plugins(CombatPlugin);
    let cell = IVec2::ZERO;
    let center = common::cell_world_center(cell);
    app.world_mut().resource_mut::<IntentGrid>().paint_owned(
        cell,
        IntentKind::Defend,
        Some(SwarmId::PLAYER),
    );
    let attacker_position = center + Vec2::new(-16.0, 0.0);
    let target_position = center + Vec2::new(16.0, 0.0);
    let attacker = common::spawn_defender_at(&mut app, attacker_position);
    app.world_mut()
        .entity_mut(attacker)
        .insert(DefendHold { cell });
    let target = common::spawn_worker_at(&mut app, target_position);
    app.world_mut()
        .entity_mut(target)
        .insert(SwarmMember::new(SwarmId(11)));

    app.update();

    assert_eq!(
        app.world().entity(target).get::<Health>().unwrap().current,
        90,
        "the fact must describe damage that already resolved",
    );
    let facts = resolved_facts(&app);
    let [ResolvedCombatFact::Hit(hit)] = facts.as_slice() else {
        panic!("one delivered attack must publish exactly one hit fact: {facts:?}");
    };
    assert_eq!(hit.attacker.entity, attacker);
    assert_eq!(hit.attacker.position, attacker_position);
    assert_eq!(hit.attacker.swarm, SwarmId::PLAYER);
    assert_eq!(
        hit.attacker.appearance,
        CombatAppearance::Nanobot(
            top_down_2d_rts_prototype_nano_swarm::nanobot::NanobotType::Defender
        ),
    );
    assert_eq!(hit.target.entity, target);
    assert_eq!(hit.target.position, target_position);
    assert_eq!(hit.target.swarm, SwarmId(11));
    assert_eq!(
        hit.target.appearance,
        CombatAppearance::Nanobot(
            top_down_2d_rts_prototype_nano_swarm::nanobot::NanobotType::Worker
        ),
    );
    assert_eq!(hit.damage, 10);
    assert!(!hit.target_destroyed);
}

#[test]
fn lethal_combat_publishes_one_death_snapshot_and_removes_the_nanobot() {
    let mut app = common::sim_app_with_defend();
    app.add_plugins(CombatPlugin)
        .add_systems(FixedLast, nanobot_death_cleanup_system);
    let cell = IVec2::ZERO;
    let center = common::cell_world_center(cell);
    app.world_mut().resource_mut::<IntentGrid>().paint_owned(
        cell,
        IntentKind::Defend,
        Some(SwarmId::PLAYER),
    );
    let attacker = common::spawn_defender_at(&mut app, center + Vec2::new(-16.0, 0.0));
    app.world_mut()
        .entity_mut(attacker)
        .insert(DefendHold { cell });
    let target_position = center + Vec2::new(16.0, 0.0);
    let target = common::spawn_worker_at(&mut app, target_position);
    app.world_mut().entity_mut(target).insert((
        SwarmMember::new(SwarmId(11)),
        Health {
            current: 10,
            max: 100,
        },
    ));

    app.update();

    assert!(!app.world().entities().contains(target));
    let facts = resolved_facts(&app);
    let [
        ResolvedCombatFact::Hit(hit),
        ResolvedCombatFact::Death(death),
    ] = facts.as_slice()
    else {
        panic!("lethal combat must publish its hit followed by one death fact: {facts:?}");
    };
    assert!(hit.target_destroyed);
    assert_eq!(
        *death,
        ResolvedCombatDeath {
            victim: top_down_2d_rts_prototype_nano_swarm::nanobot::CombatVisualSnapshot {
                entity: target,
                position: target_position,
                swarm: SwarmId(11),
                appearance: CombatAppearance::Nanobot(NanobotType::Worker),
            },
        }
    );
}

#[test]
fn simultaneous_lethal_hits_publish_every_hit_and_one_death() {
    let mut app = common::sim_app_with_defend();
    app.add_plugins(CombatPlugin);
    let cell = IVec2::ZERO;
    let center = common::cell_world_center(cell);
    app.world_mut().resource_mut::<IntentGrid>().paint_owned(
        cell,
        IntentKind::Defend,
        Some(SwarmId::PLAYER),
    );
    let left = common::spawn_defender_at(&mut app, center + Vec2::new(-24.0, -8.0));
    let right = common::spawn_defender_at(&mut app, center + Vec2::new(-24.0, 8.0));
    app.world_mut().entity_mut(left).insert(DefendHold { cell });
    app.world_mut()
        .entity_mut(right)
        .insert(DefendHold { cell });
    let target = common::spawn_worker_at(&mut app, center + Vec2::new(24.0, 0.0));
    app.world_mut().entity_mut(target).insert((
        SwarmMember::new(SwarmId(11)),
        Health {
            current: 15,
            max: 100,
        },
    ));

    app.update();

    let facts = resolved_facts(&app);
    assert_eq!(facts.len(), 3);
    let hits = facts
        .iter()
        .filter_map(|fact| match fact {
            ResolvedCombatFact::Hit(hit) => Some(hit),
            ResolvedCombatFact::Death(_) => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(hits.len(), 2);
    assert!(hits.iter().all(|hit| hit.target.entity == target));
    assert!(hits.iter().all(|hit| hit.target_destroyed));
    assert_eq!(
        facts
            .iter()
            .filter(|fact| matches!(fact, ResolvedCombatFact::Death(_)))
            .count(),
        1,
    );
}

#[test]
fn combat_death_snapshots_every_nanobot_type_for_both_swarms() {
    for victim_swarm in [SwarmId::PLAYER, SwarmId(11)] {
        for kind in NanobotType::ALL {
            let mut app = common::sim_app_with_defend();
            app.add_plugins(CombatPlugin);
            let cell = IVec2::ZERO;
            let center = common::cell_world_center(cell);
            let attacker = common::spawn_defender_at(&mut app, center - Vec2::X * 16.0);
            let attacker_swarm = if victim_swarm.is_player() {
                SwarmId(11)
            } else {
                SwarmId::PLAYER
            };
            app.world_mut().resource_mut::<IntentGrid>().paint_owned(
                cell,
                IntentKind::Defend,
                Some(attacker_swarm),
            );
            app.world_mut()
                .entity_mut(attacker)
                .insert((SwarmMember::new(attacker_swarm), DefendHold { cell }));
            let victim_position = center + Vec2::X * 16.0;
            let victim = match kind {
                NanobotType::Worker => common::spawn_worker_at(&mut app, victim_position),
                NanobotType::Hauler => common::spawn_hauler_at(&mut app, victim_position),
                NanobotType::Defender => common::spawn_defender_at(&mut app, victim_position),
            };
            app.world_mut().entity_mut(victim).insert((
                SwarmMember::new(victim_swarm),
                Health {
                    current: 1,
                    max: 100,
                },
            ));

            app.update();

            let facts = resolved_facts(&app);
            let [ResolvedCombatFact::Hit(_), ResolvedCombatFact::Death(death)] = facts.as_slice()
            else {
                panic!("{victim_swarm:?} {kind:?} needs one hit and one death: {facts:?}");
            };
            assert_eq!(death.victim.entity, victim);
            assert_eq!(death.victim.position, victim_position);
            assert_eq!(death.victim.swarm, victim_swarm);
            assert_eq!(death.victim.appearance, CombatAppearance::Nanobot(kind));
        }
    }
}

#[test]
fn charge_depletion_removes_the_nanobot_without_a_combat_death_fact() {
    let mut app = common::sim_app_with_defend();
    app.add_plugins(CombatPlugin)
        .add_systems(FixedUpdate, defender_health_loss_when_empty_system)
        .add_systems(FixedLast, nanobot_death_cleanup_system);
    let defender = common::spawn_defender_at(&mut app, Vec2::ZERO);
    app.world_mut()
        .entity_mut(defender)
        .get_mut::<Charge>()
        .unwrap()
        .current = 0.0;
    app.world_mut().entity_mut(defender).insert(Health {
        current: 1,
        max: 100,
    });

    for _ in 0..usize::from(EMPTY_CHARGE_DAMAGE_INTERVAL_TICKS) {
        app.update();
    }

    assert!(!app.world().entities().contains(defender));
    assert!(resolved_facts(&app).is_empty());
}

#[test]
fn cooldown_only_tick_publishes_no_resolved_hit() {
    let mut app = common::sim_app_with_defend();
    app.add_plugins(CombatPlugin);
    let cell = IVec2::ZERO;
    let center = common::cell_world_center(cell);
    app.world_mut().resource_mut::<IntentGrid>().paint_owned(
        cell,
        IntentKind::Defend,
        Some(SwarmId::PLAYER),
    );
    let attacker = common::spawn_defender_at(&mut app, center);
    app.world_mut().entity_mut(attacker).insert((
        DefendHold { cell },
        DefenderAttackCooldown { ticks_remaining: 3 },
    ));
    let target = common::spawn_worker_at(&mut app, center + Vec2::new(16.0, 0.0));
    app.world_mut()
        .entity_mut(target)
        .insert(SwarmMember::new(SwarmId(11)));

    app.update();

    assert_eq!(
        app.world().entity(target).get::<Health>().unwrap().current,
        100,
    );
    assert!(resolved_facts(&app).is_empty());
}

#[test]
fn pursuit_without_delivered_damage_publishes_no_resolved_hit() {
    let mut app = common::sim_app_with_defend();
    app.add_plugins(CombatPlugin);
    let cell = IVec2::ZERO;
    let center = common::cell_world_center(cell);
    app.world_mut().resource_mut::<IntentGrid>().paint_owned(
        cell,
        IntentKind::Defend,
        Some(SwarmId::PLAYER),
    );
    let attacker = common::spawn_defender_at(&mut app, center);
    app.world_mut()
        .entity_mut(attacker)
        .insert(DefendHold { cell });
    let target = common::spawn_worker_at(&mut app, center + Vec2::new(160.0, 0.0));
    app.world_mut()
        .entity_mut(target)
        .insert(SwarmMember::new(SwarmId(11)));

    app.update();

    assert!(
        app.world()
            .entity(attacker)
            .get::<DirectMovementComponent>()
            .is_some(),
    );
    assert!(resolved_facts(&app).is_empty());
}

#[test]
fn nearby_hostile_without_delivered_damage_publishes_no_resolved_hit() {
    let mut app = common::sim_app_with_defend();
    app.add_plugins(CombatPlugin);
    let cell = IVec2::ZERO;
    let center = common::cell_world_center(cell);
    let attacker = common::spawn_defender_at(&mut app, center);
    app.world_mut()
        .entity_mut(attacker)
        .insert(DefendHold { cell });
    app.world_mut()
        .entity_mut(attacker)
        .get_mut::<top_down_2d_rts_prototype_nano_swarm::nanobot::Charge>()
        .unwrap()
        .current = 0.0;
    let target = common::spawn_worker_at(&mut app, center + Vec2::new(16.0, 0.0));
    app.world_mut()
        .entity_mut(target)
        .insert(SwarmMember::new(SwarmId(11)));

    app.update();

    assert_eq!(
        app.world().entity(target).get::<Health>().unwrap().current,
        100,
    );
    assert!(resolved_facts(&app).is_empty());
}

#[test]
fn equal_full_charge_defenders_resolve_in_readable_ttk_window() {
    let mut app = common::sim_app_with_defend();
    app.add_plugins(CombatPlugin);
    let cell = IVec2::ZERO;
    app.world_mut().resource_mut::<IntentGrid>().paint_owned(
        cell,
        IntentKind::Defend,
        Some(SwarmId::PLAYER),
    );
    app.world_mut()
        .resource_mut::<IntentGrid>()
        .contest_defend(cell, SwarmId(11));
    let center = common::cell_world_center(cell);
    let player = common::spawn_defender_at(&mut app, center + Vec2::new(-8.0, 0.0));
    app.world_mut()
        .entity_mut(player)
        .insert(DefendHold { cell });
    let opponent = common::spawn_defender_at(&mut app, center + Vec2::new(8.0, 0.0));
    app.world_mut()
        .entity_mut(opponent)
        .insert((SwarmMember::new(SwarmId(11)), DefendHold { cell }));

    for _ in 0..284 {
        app.update();
    }
    assert!(app.world().entities().contains(player));
    assert!(app.world().entities().contains(opponent));
    assert!(app.world().entity(player).get::<Health>().unwrap().current > 0);
    assert!(
        app.world()
            .entity(opponent)
            .get::<Health>()
            .unwrap()
            .current
            > 0
    );

    for _ in 0..31 {
        app.update();
    }
    assert!(
        !app.world().entities().contains(player)
            || app.world().entity(player).get::<Health>().unwrap().current == 0
    );
    assert!(
        !app.world().entities().contains(opponent)
            || app
                .world()
                .entity(opponent)
                .get::<Health>()
                .unwrap()
                .current
                == 0
    );
}

#[test]
fn combat_damage_only_occurs_when_cooldown_is_ready() {
    let mut app = common::sim_app_with_defend();
    app.add_plugins(CombatPlugin);
    let cell = IVec2::ZERO;
    app.world_mut().resource_mut::<IntentGrid>().paint_owned(
        cell,
        IntentKind::Defend,
        Some(SwarmId::PLAYER),
    );
    app.world_mut()
        .resource_mut::<IntentGrid>()
        .contest_defend(cell, SwarmId(11));
    let center = common::cell_world_center(cell);
    let player = common::spawn_defender_at(&mut app, center + Vec2::new(-8.0, 0.0));
    app.world_mut()
        .entity_mut(player)
        .insert(DefendHold { cell });
    let opponent = common::spawn_defender_at(&mut app, center + Vec2::new(8.0, 0.0));
    app.world_mut()
        .entity_mut(opponent)
        .insert((SwarmMember::new(SwarmId(11)), DefendHold { cell }));

    app.update();
    assert_eq!(
        app.world().entity(player).get::<Health>().unwrap().current,
        95
    );
    for _ in 0..14 {
        app.update();
    }
    assert_eq!(
        app.world().entity(player).get::<Health>().unwrap().current,
        95
    );

    app.update();
    assert_eq!(
        app.world().entity(player).get::<Health>().unwrap().current,
        90
    );
}

#[test]
fn simultaneous_lethal_attacks_still_exchange() {
    let mut app = common::sim_app_with_defend();
    app.add_plugins(CombatPlugin);
    let cell = IVec2::ZERO;
    app.world_mut().resource_mut::<IntentGrid>().paint_owned(
        cell,
        IntentKind::Defend,
        Some(SwarmId::PLAYER),
    );
    app.world_mut()
        .resource_mut::<IntentGrid>()
        .contest_defend(cell, SwarmId(11));
    let center = common::cell_world_center(cell);
    let player = common::spawn_defender_at(&mut app, center + Vec2::new(-8.0, 0.0));
    app.world_mut()
        .entity_mut(player)
        .insert(DefendHold { cell });
    app.world_mut()
        .entity_mut(player)
        .get_mut::<Health>()
        .unwrap()
        .current = 5;
    let opponent = common::spawn_defender_at(&mut app, center + Vec2::new(8.0, 0.0));
    app.world_mut()
        .entity_mut(opponent)
        .insert((SwarmMember::new(SwarmId(11)), DefendHold { cell }));
    app.world_mut()
        .entity_mut(opponent)
        .get_mut::<Health>()
        .unwrap()
        .current = 5;

    app.update();

    assert_eq!(
        app.world().entity(player).get::<Health>().unwrap().current,
        0
    );
    assert_eq!(
        app.world()
            .entity(opponent)
            .get::<Health>()
            .unwrap()
            .current,
        0
    );
}

#[test]
fn pursuit_updates_between_attack_pulses() {
    let mut app = common::sim_app_with_defend();
    app.add_plugins(CombatPlugin);
    let cell = IVec2::ZERO;
    app.world_mut().resource_mut::<IntentGrid>().paint_owned(
        cell,
        IntentKind::Defend,
        Some(SwarmId::PLAYER),
    );
    let center = common::cell_world_center(cell);
    let defender = common::spawn_defender_at(&mut app, center);
    app.world_mut().entity_mut(defender).insert((
        DefendHold { cell },
        DefenderAttackCooldown {
            ticks_remaining: 10,
        },
    ));
    let hostile = common::spawn_defender_at(&mut app, center + Vec2::new(160.0, 0.0));
    app.world_mut()
        .entity_mut(hostile)
        .insert(SwarmMember::new(SwarmId(11)));

    app.update();
    assert_eq!(
        app.world()
            .entity(defender)
            .get::<DirectMovementComponent>()
            .unwrap()
            .xy,
        center + Vec2::new(160.0, 0.0)
    );

    let moved_target = center + Vec2::new(200.0, 0.0);
    app.world_mut()
        .entity_mut(hostile)
        .get_mut::<Transform>()
        .unwrap()
        .translation = moved_target.extend(0.0);
    app.update();

    assert_eq!(
        app.world()
            .entity(defender)
            .get::<DirectMovementComponent>()
            .unwrap()
            .xy,
        moved_target,
        "pursuit must refresh target position while attack cooldown is active"
    );
    assert_eq!(
        app.world()
            .entity(defender)
            .get::<DefenderAttackCooldown>()
            .unwrap()
            .ticks_remaining,
        8
    );
}

#[test]
fn single_defender_does_not_delete_structure_before_readable_window() {
    let mut app = common::sim_app_with_defend();
    app.add_plugins(CombatPlugin);
    let cell = IVec2::ZERO;
    let center = common::cell_world_center(cell);
    app.world_mut().resource_mut::<IntentGrid>().paint_owned(
        cell,
        IntentKind::Defend,
        Some(SwarmId::PLAYER),
    );
    let defender = common::spawn_defender_at(&mut app, center);
    app.world_mut()
        .entity_mut(defender)
        .insert(DefendHold { cell });
    let enemy_swarm = app.world_mut().spawn((Swarm {}, SwarmId(11))).id();
    let structure = app
        .world_mut()
        .spawn((
            Structure::new(StructureKind::Basic),
            OwnerSwarm(enemy_swarm),
            Transform::from_translation((center + Vec2::new(16.0, 0.0)).extend(0.0)),
        ))
        .id();

    for _ in 0..284 {
        app.update();
    }
    assert!(app.world().entities().contains(structure));
    assert!(
        app.world()
            .entity(structure)
            .get::<Structure>()
            .unwrap()
            .health
            > 0
    );

    for _ in 0..31 {
        app.update();
    }
    assert!(!app.world().entities().contains(structure));
}

#[test]
fn opposing_holding_defenders_exchange_damage_in_a_contested_cell() {
    let mut app = common::sim_app_with_defend();
    app.add_plugins(CombatPlugin);
    let cell = IVec2::ZERO;
    app.world_mut().resource_mut::<IntentGrid>().paint_owned(
        cell,
        IntentKind::Defend,
        Some(SwarmId::PLAYER),
    );
    app.world_mut()
        .resource_mut::<IntentGrid>()
        .contest_defend(cell, SwarmId(11));
    let center = common::cell_world_center(cell);
    let player = common::spawn_defender_at(&mut app, center + Vec2::new(-8.0, 0.0));
    app.world_mut()
        .entity_mut(player)
        .insert(DefendHold { cell });
    let enemy = common::spawn_defender_at(&mut app, center + Vec2::new(8.0, 0.0));
    app.world_mut()
        .entity_mut(enemy)
        .insert((SwarmMember::new(SwarmId(11)), DefendHold { cell }));

    let player_before = app.world().entity(player).get::<Health>().unwrap().current;
    let enemy_before = app.world().entity(enemy).get::<Health>().unwrap().current;
    app.update();

    assert!(app.world().entity(player).get::<Health>().unwrap().current < player_before);
    assert!(app.world().entity(enemy).get::<Health>().unwrap().current < enemy_before);
}

#[test]
fn shared_defend_intent_brings_opposing_defenders_into_combat() {
    let mut app = common::sim_app_with_defend();
    app.add_plugins(CombatPlugin);
    let cell = IVec2::ZERO;
    app.world_mut()
        .resource_mut::<IntentGrid>()
        .paint(cell, IntentKind::Defend);

    let player = common::spawn_defender_at(&mut app, common::cell_world_center(IVec2::new(-1, 0)));
    let opponent = common::spawn_defender_at(&mut app, common::cell_world_center(IVec2::new(1, 0)));
    app.world_mut()
        .entity_mut(opponent)
        .insert(SwarmMember::new(SwarmId(11)));

    let player_before = app.world().entity(player).get::<Health>().unwrap().current;
    let opponent_before = app
        .world()
        .entity(opponent)
        .get::<Health>()
        .unwrap()
        .current;
    for _ in 0..240 {
        app.update();
        let player_health = app.world().entity(player).get::<Health>().unwrap().current;
        let opponent_health = app
            .world()
            .entity(opponent)
            .get::<Health>()
            .unwrap()
            .current;
        if player_health < player_before && opponent_health < opponent_before {
            return;
        }
    }

    panic!("opposing Defenders assigned through shared Defend intent never exchanged damage");
}

#[test]
fn surviving_defender_captures_a_contested_defend_cell() {
    let mut app = common::sim_app_with_defend();
    app.add_plugins(CombatPlugin);
    let cell = IVec2::ZERO;
    let opponent_swarm = SwarmId(11);
    {
        let mut grid = app.world_mut().resource_mut::<IntentGrid>();
        grid.paint_owned(cell, IntentKind::Defend, Some(opponent_swarm));
        grid.contest_defend(cell, SwarmId::PLAYER);
    }
    let center = common::cell_world_center(cell);
    let player = common::spawn_defender_at(&mut app, center + Vec2::new(-8.0, 0.0));
    app.world_mut()
        .entity_mut(player)
        .insert(DefendHold { cell });
    let opponent = common::spawn_defender_at(&mut app, center + Vec2::new(8.0, 0.0));
    app.world_mut()
        .entity_mut(opponent)
        .insert((SwarmMember::new(opponent_swarm), DefendHold { cell }));

    app.update();
    app.world_mut()
        .entity_mut(opponent)
        .get_mut::<Health>()
        .unwrap()
        .current = 0;
    app.update();

    assert_eq!(
        app.world()
            .resource::<IntentGrid>()
            .cell(cell)
            .unwrap()
            .owner(IntentKind::Defend),
        Some(SwarmId::PLAYER),
    );
}

#[test]
fn sole_challenger_eventually_captures_an_undefended_contested_cell() {
    let mut app = common::sim_app_with_defend();
    app.add_plugins(CombatPlugin);
    let cell = IVec2::ZERO;
    {
        let mut grid = app.world_mut().resource_mut::<IntentGrid>();
        grid.paint_owned(cell, IntentKind::Defend, Some(SwarmId(11)));
        grid.contest_defend(cell, SwarmId::PLAYER);
    }
    let player = common::spawn_defender_at(&mut app, common::cell_world_center(cell));
    app.world_mut()
        .entity_mut(player)
        .insert(DefendHold { cell });

    for _ in 0..UNCONTESTED_CAPTURE_TICKS {
        app.update();
    }

    assert_eq!(
        app.world()
            .resource::<IntentGrid>()
            .cell(cell)
            .unwrap()
            .owner(IntentKind::Defend),
        Some(SwarmId::PLAYER),
    );
}

#[test]
fn incumbent_does_not_cancel_contest_while_challenger_is_approaching() {
    let mut app = common::sim_app_with_defend();
    app.add_plugins(CombatPlugin);
    let cell = IVec2::ZERO;
    let opponent = SwarmId(11);
    {
        let mut grid = app.world_mut().resource_mut::<IntentGrid>();
        grid.paint_owned(cell, IntentKind::Defend, Some(opponent));
        grid.contest_defend(cell, SwarmId::PLAYER);
    }
    let incumbent = common::spawn_defender_at(&mut app, common::cell_world_center(cell));
    app.world_mut()
        .entity_mut(incumbent)
        .insert((SwarmMember::new(opponent), DefendHold { cell }));
    let challenger =
        common::spawn_defender_at(&mut app, common::cell_world_center(IVec2::new(-2, 0)));

    for _ in 0..(UNCONTESTED_CAPTURE_TICKS + 10) {
        app.update();
    }

    assert!(app.world().entity(challenger).get::<DefendHold>().is_none());
    assert_eq!(
        app.world().resource::<IntentGrid>().defend_contest(cell),
        Some((opponent, SwarmId::PLAYER)),
        "persistent Defend intent must remain contested while its challenger travels",
    );
}

#[test]
fn withdrawing_from_contest_releases_player_hold_and_late_arrival() {
    let mut app = common::sim_app_with_defend();
    app.add_plugins(CombatPlugin);
    let cell = IVec2::ZERO;
    {
        let mut grid = app.world_mut().resource_mut::<IntentGrid>();
        grid.paint_owned(cell, IntentKind::Defend, Some(SwarmId(11)));
        grid.contest_defend(cell, SwarmId::PLAYER);
    }
    let holder = common::spawn_defender_at(&mut app, common::cell_world_center(cell));
    app.world_mut()
        .entity_mut(holder)
        .insert(DefendHold { cell });
    let late_arrival = common::spawn_defender_at(&mut app, common::cell_world_center(cell));
    app.world_mut()
        .entity_mut(late_arrival)
        .insert(top_down_2d_rts_prototype_nano_swarm::nanobot::DefendAssignment { cell });
    app.world_mut()
        .resource_mut::<IntentGrid>()
        .withdraw_defend_contest(cell, SwarmId::PLAYER);

    app.update();

    assert!(app.world().entity(holder).get::<DefendHold>().is_none());
    assert!(
        app.world()
            .entity(late_arrival)
            .get::<DefendHold>()
            .is_none()
    );
}

#[test]
fn threat_pressure_observes_post_integration_cell() {
    let mut app = common::sim_app_with_defend();
    app.add_plugins(CombatPlugin);
    let cell = IVec2::ZERO;
    let center = common::cell_world_center(cell);
    app.world_mut().resource_mut::<IntentGrid>().paint_owned(
        cell,
        IntentKind::Defend,
        Some(SwarmId::PLAYER),
    );
    let enemy = common::spawn_defender_at(&mut app, Vec2::new(-1.0, center.y));
    app.world_mut().entity_mut(enemy).insert((
        SwarmMember::new(SwarmId(11)),
        DirectMovementComponent {
            xy: center,
            stop_radius: 0.0,
        },
    ));

    app.update();

    assert!(
        app.world()
            .entity(enemy)
            .get::<Transform>()
            .unwrap()
            .translation
            .x
            > 0.0
    );
    assert!(
        app.world()
            .resource::<DefendPressure>()
            .get_for(SwarmId::PLAYER, cell)
            > 1.0,
        "threat projection reads the cell reached during this fixed tick",
    );
}

#[test]
fn contested_defend_pressure_counts_hostiles_for_both_participants() {
    let mut app = common::sim_app_with_defend();
    app.add_plugins(CombatPlugin);
    let cell = IVec2::ZERO;
    let opponent = SwarmId(11);
    {
        let mut grid = app.world_mut().resource_mut::<IntentGrid>();
        grid.paint_owned(cell, IntentKind::Defend, Some(SwarmId::PLAYER));
        grid.contest_defend(cell, opponent);
    }
    let center = common::cell_world_center(cell);
    common::spawn_defender_at(&mut app, center);
    let hostile = common::spawn_defender_at(&mut app, center);
    app.world_mut()
        .entity_mut(hostile)
        .insert(SwarmMember::new(opponent));

    app.update();

    let pressure = app.world().resource::<DefendPressure>();
    assert_eq!(pressure.get_for(SwarmId::PLAYER, cell), 2.0);
    assert_eq!(pressure.get_for(opponent, cell), 2.0);
}

#[test]
fn holding_defender_damages_hostile_support_structure() {
    let mut app = common::sim_app_with_defend();
    app.add_plugins(CombatPlugin);
    let cell = IVec2::ZERO;
    let center = common::cell_world_center(cell);
    app.world_mut().resource_mut::<IntentGrid>().paint_owned(
        cell,
        IntentKind::Defend,
        Some(SwarmId::PLAYER),
    );
    let defender = common::spawn_defender_at(&mut app, center);
    app.world_mut()
        .entity_mut(defender)
        .insert(DefendHold { cell });
    let enemy_swarm = app.world_mut().spawn((Swarm {}, SwarmId(11))).id();
    let structure = common::spawn_owned_completed_structure(
        &mut app,
        enemy_swarm,
        PlannedKind::Charger,
        Transform::from_translation((center + Vec2::new(16.0, 0.0)).extend(0.0)),
        None,
    );
    let structure_position = app
        .world()
        .entity(structure)
        .get::<Transform>()
        .unwrap()
        .translation
        .truncate();
    let before = app
        .world()
        .entity(structure)
        .get::<Structure>()
        .unwrap()
        .health;

    app.update();

    assert!(
        app.world()
            .entity(structure)
            .get::<Structure>()
            .unwrap()
            .health
            < before,
        "hostile support structure is a secondary combat target",
    );
    let facts = resolved_facts(&app);
    let [ResolvedCombatFact::Hit(hit)] = facts.as_slice() else {
        panic!("one delivered structure attack must publish exactly one hit fact: {facts:?}");
    };
    assert_eq!(hit.target.entity, structure);
    assert_eq!(hit.target.position, structure_position);
    assert_eq!(hit.target.swarm, SwarmId(11));
    assert_eq!(
        hit.target.appearance,
        CombatAppearance::Structure(StructureCombatAppearance {
            kind: StructureKind::Basic,
            visual: Some(StructureVisual::completed(PlannedKind::Charger)),
        }),
    );
    assert_eq!(hit.damage, 5);
    assert!(!hit.target_destroyed);
}

#[test]
fn holding_defender_closes_on_a_hostile_structure_in_its_defend_cell() {
    let mut app = common::sim_app_with_defend();
    app.add_plugins(CombatPlugin);
    let cell = IVec2::ZERO;
    let center = common::cell_world_center(cell);
    app.world_mut().resource_mut::<IntentGrid>().paint_owned(
        cell,
        IntentKind::Defend,
        Some(SwarmId::PLAYER),
    );
    let defender = common::spawn_defender_at(&mut app, center);
    app.world_mut()
        .entity_mut(defender)
        .insert(DefendHold { cell });
    let enemy_swarm = app.world_mut().spawn((Swarm {}, SwarmId(11))).id();
    let structure = app
        .world_mut()
        .spawn((
            Structure::new(StructureKind::Basic),
            OwnerSwarm(enemy_swarm),
            Transform::from_translation((center + Vec2::new(160.0, 0.0)).extend(0.0)),
        ))
        .id();
    let before = app
        .world()
        .entity(structure)
        .get::<Structure>()
        .unwrap()
        .health;

    for _ in 0..40 {
        app.update();
        if app
            .world()
            .entity(structure)
            .get::<Structure>()
            .unwrap()
            .health
            < before
        {
            return;
        }
    }

    panic!("holding Defender never closed on the hostile structure in its Defend cell");
}

#[test]
fn lethal_combat_despawns_support_structure_immediately() {
    let mut app = common::sim_app_with_defend();
    app.add_plugins(CombatPlugin);
    let cell = IVec2::ZERO;
    let center = common::cell_world_center(cell);
    app.world_mut().resource_mut::<IntentGrid>().paint_owned(
        cell,
        IntentKind::Defend,
        Some(SwarmId::PLAYER),
    );
    let defender = common::spawn_defender_at(&mut app, center);
    app.world_mut()
        .entity_mut(defender)
        .insert(DefendHold { cell });
    let enemy_swarm = app.world_mut().spawn((Swarm {}, SwarmId(11))).id();
    let structure = common::spawn_owned_completed_structure(
        &mut app,
        enemy_swarm,
        PlannedKind::ProductionFacility,
        Transform::from_translation((center + Vec2::new(16.0, 0.0)).extend(0.0)),
        Some(1),
    );
    let structure_position = center + Vec2::new(16.0, 0.0);

    app.update();

    assert!(
        !app.world().entities().contains(structure),
        "zero-health support structure must not remain repairable",
    );
    let facts = resolved_facts(&app);
    let [
        ResolvedCombatFact::Hit(hit),
        ResolvedCombatFact::Death(death),
    ] = facts.as_slice()
    else {
        panic!("lethal structure combat must publish one hit and one death fact: {facts:?}");
    };
    assert!(hit.target_destroyed);
    assert_eq!(
        *death,
        ResolvedCombatDeath {
            victim: top_down_2d_rts_prototype_nano_swarm::nanobot::CombatVisualSnapshot {
                entity: structure,
                position: structure_position,
                swarm: SwarmId(11),
                appearance: CombatAppearance::Structure(StructureCombatAppearance {
                    kind: StructureKind::Basic,
                    visual: Some(StructureVisual::completed(PlannedKind::ProductionFacility)),
                }),
            },
        },
    );
}

#[test]
fn simultaneous_lethal_structure_hits_publish_every_hit_and_one_death() {
    let mut app = common::sim_app_with_defend();
    app.add_plugins(CombatPlugin);
    let cell = IVec2::ZERO;
    let center = common::cell_world_center(cell);
    app.world_mut().resource_mut::<IntentGrid>().paint_owned(
        cell,
        IntentKind::Defend,
        Some(SwarmId::PLAYER),
    );
    for offset in [Vec2::new(-24.0, -8.0), Vec2::new(-24.0, 8.0)] {
        let defender = common::spawn_defender_at(&mut app, center + offset);
        app.world_mut()
            .entity_mut(defender)
            .insert(DefendHold { cell });
    }
    let enemy_swarm = app.world_mut().spawn((Swarm {}, SwarmId(11))).id();
    let structure = common::spawn_owned_completed_structure(
        &mut app,
        enemy_swarm,
        PlannedKind::SinkStockpile,
        Transform::from_translation((center + Vec2::new(24.0, 0.0)).extend(0.0)),
        Some(9),
    );

    app.update();

    assert!(!app.world().entities().contains(structure));
    let facts = resolved_facts(&app);
    let mut hits = Vec::new();
    let mut deaths = Vec::new();
    for fact in &facts {
        match fact {
            ResolvedCombatFact::Hit(hit) => hits.push(hit),
            ResolvedCombatFact::Death(death) => deaths.push(death),
        }
    }
    assert_eq!(hits.len(), 2);
    assert!(hits.iter().all(|hit| hit.target.entity == structure));
    assert!(hits.iter().all(|hit| hit.target_destroyed));
    assert_eq!(deaths.len(), 1);
    assert_eq!(
        deaths[0].victim.appearance,
        CombatAppearance::Structure(StructureCombatAppearance {
            kind: StructureKind::Basic,
            visual: Some(StructureVisual::completed(PlannedKind::SinkStockpile)),
        }),
    );
}

#[test]
fn shared_defend_paint_does_not_assign_self_hostility_to_player() {
    let mut app = common::sim_app_with_defend();
    app.add_plugins(CombatPlugin);
    let cell = IVec2::ZERO;
    app.world_mut()
        .resource_mut::<IntentGrid>()
        .paint(cell, IntentKind::Defend);
    let enemy = common::spawn_defender_at(&mut app, common::cell_world_center(cell));
    app.world_mut()
        .entity_mut(enemy)
        .insert(SwarmMember::new(SwarmId(11)));

    app.update();

    let pressure = app.world().resource::<DefendPressure>();
    assert_eq!(pressure.get_for(SwarmId::PLAYER, cell), 1.0);
    assert_eq!(pressure.get_for(SwarmId(11), cell), 1.0);
}
