#[path = "../common/mod.rs"]
mod common;

use approx::assert_abs_diff_eq;
use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    game_settings::GameSettings,
    intent::{IntentGrid, IntentKind, UNCONTESTED_CAPTURE_TICKS},
    nanobot::{
        ChargerAssignment, CombatAppearance, DEFENDER_ATTACK_INTERVAL_TICKS,
        DefenderAttackCooldown, DefenderResponse, DirectMovementComponent, Health, NanobotType,
        OwnerSwarm, PlannedKind, ResolvedCombatFact, Structure, StructureCombatAppearance,
        StructureKind, Swarm, SwarmId, SwarmMember, nanobot_death_cleanup_system,
    },
    structure_sprites::StructureVisual,
};

fn resolved_facts(app: &App) -> Vec<ResolvedCombatFact> {
    let messages = app.world().resource::<Messages<ResolvedCombatFact>>();
    let mut cursor = messages.get_cursor();
    cursor.read(messages).copied().collect()
}

fn assert_position(actual: Vec2, expected: Vec2) {
    assert_abs_diff_eq!(actual.x, expected.x, epsilon = 0.01);
    assert_abs_diff_eq!(actual.y, expected.y, epsilon = 0.01);
}

fn paint_player_territory(app: &mut App, cell: IVec2) {
    app.world_mut().resource_mut::<IntentGrid>().paint_owned(
        cell,
        IntentKind::Gather,
        Some(SwarmId::PLAYER),
    );
}

#[test]
fn delivered_hit_publishes_the_resolved_combat_snapshot() {
    let mut app = common::sim_app_with_combat();
    app.world_mut().spawn((Swarm {}, SwarmId::PLAYER));
    app.world_mut().spawn((Swarm {}, SwarmId(11)));
    let cell = IVec2::ZERO;
    paint_player_territory(&mut app, cell);
    let center = common::cell_world_center(cell);
    let attacker_position = center - Vec2::X * 36.0;
    let target_position = center + Vec2::X * 36.0;
    let attacker = common::spawn_defender_at(&mut app, attacker_position);
    let target = common::spawn_worker_at(&mut app, target_position);
    app.world_mut()
        .entity_mut(target)
        .insert(SwarmMember::new(SwarmId(11)));

    app.update();

    assert_eq!(
        app.world().entity(target).get::<Health>().unwrap().current,
        90,
    );
    let facts = resolved_facts(&app);
    let [ResolvedCombatFact::Hit(hit)] = facts.as_slice() else {
        panic!("one delivered attack must publish one hit fact: {facts:?}");
    };
    assert_eq!(hit.attacker.entity, attacker);
    assert_position(hit.attacker.position, attacker_position);
    assert_eq!(hit.attacker.swarm, SwarmId::PLAYER);
    assert_eq!(
        hit.attacker.appearance,
        CombatAppearance::Nanobot(NanobotType::Defender),
    );
    assert_eq!(hit.target.entity, target);
    assert_position(hit.target.position, target_position);
    assert_eq!(hit.target.swarm, SwarmId(11));
    assert_eq!(
        hit.target.appearance,
        CombatAppearance::Nanobot(NanobotType::Worker),
    );
    assert_eq!(hit.damage, 10);
    assert!(!hit.target_destroyed);
}

#[test]
fn response_moves_for_claim_but_attacks_nearest_hostile_in_range() {
    let mut app = common::sim_app_with_combat();
    app.world_mut().spawn((Swarm {}, SwarmId::PLAYER));
    let opponent = app.world_mut().spawn((Swarm {}, SwarmId(11))).id();
    let cell = IVec2::new(2, 2);
    paint_player_territory(&mut app, cell);
    let center = common::cell_world_center(cell);
    let responder = common::spawn_defender_at(&mut app, center);
    let claimed_worker = common::spawn_worker_at(&mut app, center + Vec2::X * 80.0);
    app.world_mut()
        .entity_mut(claimed_worker)
        .insert(SwarmMember::new(SwarmId(11)));
    let structure = Structure::new(StructureKind::Basic);
    let initial_structure_health = structure.health;
    let nearby_structure = app
        .world_mut()
        .spawn((
            structure,
            OwnerSwarm(opponent),
            Transform::from_translation((center + Vec2::X * 20.0).extend(0.0)),
        ))
        .id();

    app.update();

    assert_eq!(
        app.world()
            .entity(responder)
            .get::<DefenderResponse>()
            .map(|response| response.target),
        Some(claimed_worker),
        "response allocation should cover the higher-tier mobile Threat",
    );
    assert_eq!(
        app.world()
            .entity(claimed_worker)
            .get::<Health>()
            .unwrap()
            .current,
        100,
        "the claimed target is not exclusive attack ownership",
    );
    assert!(
        app.world()
            .entity(nearby_structure)
            .get::<Structure>()
            .unwrap()
            .health
            < initial_structure_health,
        "combat must choose the nearest hostile across nanobots and structures",
    );
}

#[test]
fn pursuit_destination_tracks_the_claimed_entity_each_fixed_step() {
    let mut app = common::sim_app_with_combat();
    app.world_mut().spawn((Swarm {}, SwarmId::PLAYER));
    app.world_mut().spawn((Swarm {}, SwarmId(11)));
    let cell = IVec2::new(2, 2);
    paint_player_territory(&mut app, cell);
    let center = common::cell_world_center(cell);
    let responder = common::spawn_defender_at(&mut app, center - Vec2::X * 200.0);
    let target = common::spawn_worker_at(&mut app, center + Vec2::X * 200.0);
    app.world_mut()
        .entity_mut(target)
        .insert(SwarmMember::new(SwarmId(11)));

    app.update();
    assert_eq!(
        app.world()
            .entity(responder)
            .get::<DirectMovementComponent>()
            .map(|movement| movement.xy),
        Some(center + Vec2::X * 200.0),
    );
    assert!(resolved_facts(&app).is_empty());

    let moved_target = center + Vec2::X * 120.0;
    app.world_mut()
        .entity_mut(target)
        .get_mut::<Transform>()
        .unwrap()
        .translation = moved_target.extend(0.0);
    app.update();

    assert_eq!(
        app.world()
            .entity(responder)
            .get::<DirectMovementComponent>()
            .map(|movement| movement.xy),
        Some(moved_target),
    );
}

#[test]
fn combat_damage_obeys_the_existing_cooldown() {
    let mut app = common::sim_app_with_combat();
    app.world_mut().spawn((Swarm {}, SwarmId::PLAYER));
    app.world_mut().spawn((Swarm {}, SwarmId(11)));
    let cell = IVec2::ZERO;
    paint_player_territory(&mut app, cell);
    let center = common::cell_world_center(cell);
    let responder = common::spawn_defender_at(&mut app, center - Vec2::X * 16.0);
    let target = common::spawn_worker_at(&mut app, center + Vec2::X * 16.0);
    app.world_mut()
        .entity_mut(target)
        .insert(SwarmMember::new(SwarmId(11)));

    app.update();
    assert_eq!(
        app.world().entity(target).get::<Health>().unwrap().current,
        90
    );
    for _ in 0..DEFENDER_ATTACK_INTERVAL_TICKS.saturating_sub(1) {
        app.update();
    }
    assert_eq!(
        app.world().entity(target).get::<Health>().unwrap().current,
        90
    );
    assert_eq!(
        app.world()
            .entity(responder)
            .get::<DefenderAttackCooldown>()
            .map(|cooldown| cooldown.ticks_remaining),
        Some(0),
    );
    app.update();
    assert_eq!(
        app.world().entity(target).get::<Health>().unwrap().current,
        80
    );
}

#[test]
fn simultaneous_lethal_responders_still_exchange_hits() {
    let mut app = common::sim_app_with_combat();
    app.world_mut().spawn((Swarm {}, SwarmId::PLAYER));
    app.world_mut().spawn((Swarm {}, SwarmId(11)));
    app.add_systems(FixedLast, nanobot_death_cleanup_system);
    let cell = IVec2::ZERO;
    {
        let mut grid = app.world_mut().resource_mut::<IntentGrid>();
        grid.paint_owned(cell, IntentKind::Gather, Some(SwarmId::PLAYER));
        grid.paint_owned(cell, IntentKind::Build, Some(SwarmId(11)));
    }
    let center = common::cell_world_center(cell);
    let player = common::spawn_defender_at(&mut app, center - Vec2::X * 16.0);
    let opponent = common::spawn_defender_at(&mut app, center + Vec2::X * 16.0);
    for entity in [player, opponent] {
        app.world_mut().entity_mut(entity).insert(Health {
            current: 5,
            max: 100,
        });
    }
    app.world_mut()
        .entity_mut(opponent)
        .insert(SwarmMember::new(SwarmId(11)));

    app.update();

    let facts = resolved_facts(&app);
    let player_health = app
        .world()
        .get::<Health>(player)
        .map(|health| health.current);
    let opponent_health = app
        .world()
        .get::<Health>(opponent)
        .map(|health| health.current);
    assert_eq!(
        (player_health, opponent_health),
        (None, None),
        "simultaneous responders should both be removed; facts={facts:?}",
    );
    assert_eq!(
        facts
            .iter()
            .filter(|fact| matches!(fact, ResolvedCombatFact::Hit(_)))
            .count(),
        2,
    );
    assert_eq!(
        facts
            .iter()
            .filter(|fact| matches!(fact, ResolvedCombatFact::Death(_)))
            .count(),
        2,
    );
}

#[test]
fn zero_health_nearest_target_is_skipped_for_the_nearest_living_hostile() {
    let mut app = common::sim_app_with_combat();
    app.world_mut().spawn((Swarm {}, SwarmId::PLAYER));
    let opponent = app.world_mut().spawn((Swarm {}, SwarmId(11))).id();
    let cell = IVec2::ZERO;
    paint_player_territory(&mut app, cell);
    let center = common::cell_world_center(cell);
    let responder = common::spawn_defender_at(&mut app, center);
    let dead = common::spawn_worker_at(&mut app, center + Vec2::X * 20.0);
    app.world_mut().entity_mut(dead).insert((
        SwarmMember::new(SwarmId(11)),
        Health {
            current: 0,
            max: 100,
        },
    ));
    let mut destroyed_structure = Structure::new(StructureKind::Basic);
    destroyed_structure.health = 0;
    app.world_mut().spawn((
        destroyed_structure,
        OwnerSwarm(opponent),
        Transform::from_translation((center + Vec2::X * 30.0).extend(0.0)),
    ));
    let live = common::spawn_worker_at(&mut app, center + Vec2::X * 40.0);
    app.world_mut()
        .entity_mut(live)
        .insert(SwarmMember::new(SwarmId(11)));

    app.update();

    assert_eq!(
        app.world().entity(live).get::<Health>().unwrap().current,
        90
    );
    let facts = resolved_facts(&app);
    let [ResolvedCombatFact::Hit(hit)] = facts.as_slice() else {
        panic!("the nearest living hostile should receive one hit: {facts:?}");
    };
    assert_eq!(hit.target.entity, live);
    assert_eq!(
        app.world()
            .entity(responder)
            .get::<DefenderAttackCooldown>()
            .map(|cooldown| cooldown.ticks_remaining),
        Some(DEFENDER_ATTACK_INTERVAL_TICKS.saturating_sub(1)),
    );
}

#[test]
fn lethal_structure_hit_publishes_stable_appearance_and_despawns_target() {
    let mut app = common::sim_app_with_combat();
    app.world_mut().spawn((Swarm {}, SwarmId::PLAYER));
    let opponent = app.world_mut().spawn((Swarm {}, SwarmId(11))).id();
    let cell = IVec2::ZERO;
    paint_player_territory(&mut app, cell);
    let center = common::cell_world_center(cell);
    let attacker = common::spawn_defender_at(&mut app, center - Vec2::X * 16.0);
    let mut structure = Structure::new(StructureKind::Basic);
    structure.health = 1;
    let target_position = center + Vec2::X * 16.0;
    let target = app
        .world_mut()
        .spawn((
            structure,
            OwnerSwarm(opponent),
            StructureVisual::completed(PlannedKind::Charger),
            Transform::from_translation(target_position.extend(0.0)),
        ))
        .id();

    app.update();

    assert!(!app.world().entities().contains(target));
    let facts = resolved_facts(&app);
    let [
        ResolvedCombatFact::Hit(hit),
        ResolvedCombatFact::Death(death),
    ] = facts.as_slice()
    else {
        panic!("lethal structure combat must publish hit then death: {facts:?}");
    };
    assert_eq!(hit.attacker.entity, attacker);
    assert!(hit.target_destroyed);
    assert_eq!(death.victim.entity, target);
    assert_position(death.victim.position, target_position);
    assert_eq!(
        death.victim.appearance,
        CombatAppearance::Structure(StructureCombatAppearance {
            kind: StructureKind::Basic,
            visual: Some(StructureVisual::completed(PlannedKind::Charger)),
        }),
    );
}

#[test]
fn physical_charging_defender_establishes_uncontested_presence() {
    let mut app = common::sim_app_with_combat();
    app.world_mut().spawn((Swarm {}, SwarmId::PLAYER));
    app.world_mut().spawn((Swarm {}, SwarmId(11)));
    let cell = IVec2::ZERO;
    {
        let mut grid = app.world_mut().resource_mut::<IntentGrid>();
        grid.paint_owned(cell, IntentKind::Defend, Some(SwarmId::PLAYER));
        grid.contest_defend(cell, SwarmId(11));
    }
    let challenger = common::spawn_defender_at(&mut app, common::cell_world_center(cell));
    app.world_mut().entity_mut(challenger).insert((
        SwarmMember::new(SwarmId(11)),
        ChargerAssignment {
            charger: Entity::PLACEHOLDER,
        },
    ));

    for _ in 0..UNCONTESTED_CAPTURE_TICKS {
        app.update();
    }

    let grid = app.world().resource::<IntentGrid>();
    assert_eq!(
        grid.cell(cell)
            .and_then(|intent| intent.owner(IntentKind::Defend)),
        Some(SwarmId(11)),
        "physical presence must count even while the Defender is on Charge duty",
    );
    assert!(grid.defend_contest(cell).is_none());
}

#[derive(Debug, Clone, Copy)]
enum ContestDuty {
    Staging,
    Passing,
    Pursuit,
    Combat,
    Charge,
}

#[test]
fn physical_defender_presence_is_independent_of_current_duty() {
    for duty in [
        ContestDuty::Staging,
        ContestDuty::Passing,
        ContestDuty::Pursuit,
        ContestDuty::Combat,
        ContestDuty::Charge,
    ] {
        let mut app = common::sim_app_with_combat();
        app.world_mut().spawn((Swarm {}, SwarmId::PLAYER));
        app.world_mut().spawn((Swarm {}, SwarmId(11)));
        app.world_mut().resource_mut::<GameSettings>().bot_speed = 0.0;
        let cell = IVec2::ZERO;
        {
            let mut grid = app.world_mut().resource_mut::<IntentGrid>();
            grid.paint_owned(cell, IntentKind::Defend, Some(SwarmId::PLAYER));
            grid.contest_defend(cell, SwarmId(11));
        }
        let center = common::cell_world_center(cell);
        let challenger = common::spawn_defender_at(&mut app, center);
        app.world_mut()
            .entity_mut(challenger)
            .insert(SwarmMember::new(SwarmId(11)));

        match duty {
            ContestDuty::Staging => {}
            ContestDuty::Passing => {
                app.world_mut()
                    .entity_mut(challenger)
                    .insert(DirectMovementComponent {
                        speed: None,
                        interaction: None,
                        xy: center + Vec2::X * 200.0,
                        stop_radius: 0.0,
                    });
            }
            ContestDuty::Pursuit | ContestDuty::Combat => {
                app.world_mut().resource_mut::<IntentGrid>().paint_owned(
                    cell,
                    IntentKind::Build,
                    Some(SwarmId(11)),
                );
                let offset = if matches!(duty, ContestDuty::Combat) {
                    16.0
                } else {
                    200.0
                };
                let target = common::spawn_worker_at(&mut app, center + Vec2::X * offset);
                app.world_mut()
                    .entity_mut(target)
                    .insert(Health::full(10_000));
                app.world_mut()
                    .entity_mut(challenger)
                    .insert(DefenderResponse { target });
            }
            ContestDuty::Charge => {
                app.world_mut()
                    .entity_mut(challenger)
                    .insert(ChargerAssignment {
                        charger: Entity::PLACEHOLDER,
                    });
            }
        }

        for _ in 0..UNCONTESTED_CAPTURE_TICKS {
            app.update();
        }

        assert_eq!(
            app.world()
                .resource::<IntentGrid>()
                .cell(cell)
                .and_then(|intent| intent.owner(IntentKind::Defend)),
            Some(SwarmId(11)),
            "physical presence should capture while the Defender is in {duty:?} duty",
        );
    }
}

#[test]
fn sole_living_physical_survivor_captures_after_both_sides_engage() {
    let mut app = common::sim_app_with_combat();
    app.world_mut().spawn((Swarm {}, SwarmId::PLAYER));
    app.world_mut().spawn((Swarm {}, SwarmId(11)));
    let cell = IVec2::ZERO;
    {
        let mut grid = app.world_mut().resource_mut::<IntentGrid>();
        grid.paint_owned(cell, IntentKind::Defend, Some(SwarmId::PLAYER));
        grid.contest_defend(cell, SwarmId(11));
    }
    let player = common::spawn_defender_at(&mut app, common::cell_world_center(cell) - Vec2::Y);
    let opponent = common::spawn_defender_at(&mut app, common::cell_world_center(cell) + Vec2::Y);
    app.world_mut()
        .entity_mut(opponent)
        .insert(SwarmMember::new(SwarmId(11)));
    app.update();
    assert!(
        app.world()
            .resource::<IntentGrid>()
            .defend_contest(cell)
            .is_some()
    );

    app.world_mut()
        .entity_mut(player)
        .get_mut::<Health>()
        .unwrap()
        .current = 0;
    app.update();

    let grid = app.world().resource::<IntentGrid>();
    assert_eq!(
        grid.cell(cell)
            .and_then(|intent| intent.owner(IntentKind::Defend)),
        Some(SwarmId(11)),
    );
    assert!(grid.defend_contest(cell).is_none());
}
