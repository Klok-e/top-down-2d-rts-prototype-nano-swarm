#[path = "../common/mod.rs"]
mod common;

use std::collections::BTreeMap;

use approx::assert_abs_diff_eq;
use bevy::{prelude::*, time::TimeUpdateStrategy};
use top_down_2d_rts_prototype_nano_swarm::battle_statistics::BattleCounters;
use top_down_2d_rts_prototype_nano_swarm::{
    intent::{IntentGrid, IntentKind},
    nanobot::{
        Charge, ChargerAssignment, CombatAppearance, CombatPlugin, DefenderAttackCooldown,
        DefenderResponse, DirectMovementComponent, Health, NanobotBundle, NanobotType, OwnerSwarm,
        PlannedKind, ProductionFacility, ResolvedCombatFact, Structure, StructureCombatAppearance,
        StructureKind, Swarm, SwarmId, SwarmMember, nanobot_death_cleanup_system,
    },
    structure_overlay::StructureOverlayPlugin,
    structure_sprites::StructureVisual,
    tuning::Tuning,
};

fn resolved_facts(app: &App) -> Vec<ResolvedCombatFact> {
    let messages = app.world().resource::<Messages<ResolvedCombatFact>>();
    let mut cursor = messages.get_cursor();
    cursor.read(messages).copied().collect()
}

fn effective_damage(app: &App, swarm: SwarmId) -> serde_json::Value {
    serde_json::to_value(app.world().resource::<BattleCounters>().totals_for(swarm)).unwrap()
}

fn assert_position(actual: Vec2, expected: Vec2) {
    assert_abs_diff_eq!(actual.x, expected.x, epsilon = 0.01);
    assert_abs_diff_eq!(actual.y, expected.y, epsilon = 0.01);
}

fn paint_player_territory(app: &mut App, cell: IVec2) {
    app.world_mut()
        .resource_mut::<IntentGrid>()
        .paint(cell, IntentKind::Gather, SwarmId::PLAYER);
}

#[test]
fn delivered_hit_publishes_the_resolved_combat_snapshot() {
    let mut app = common::minimal_app();
    app.add_plugins(CombatPlugin)
        .init_resource::<BattleCounters>();
    let center = Vec2::ZERO;
    let attacker_position = center - Vec2::X * 36.0;
    let target_position = center + Vec2::X * 36.0;
    let attacker = common::spawn_defender_at(&mut app, attacker_position);
    let target = common::spawn_worker_at(&mut app, target_position);
    app.world_mut()
        .entity_mut(target)
        .insert(SwarmMember::new(SwarmId(11)));

    let interval = app
        .world()
        .resource::<Tuning>()
        .combat
        .attack_interval_ticks;
    for _ in 0..interval {
        app.update();
        if app.world().get::<Health>(target).unwrap().current < 100 {
            break;
        }
    }

    assert!(app.world().get::<DefenderResponse>(attacker).is_none());
    assert_eq!(
        app.world().entity(target).get::<Health>().unwrap().current,
        74,
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
    assert_eq!(hit.damage, 26);
    assert!(!hit.target_destroyed);
    let damage = effective_damage(&app, SwarmId::PLAYER);
    assert_eq!(damage["effective_damage_total"], 26);
    assert_eq!(damage["effective_damage_nanobots"], 26);
    assert_eq!(damage["effective_damage_structures"], 0);
}

#[test]
fn simultaneous_overkill_is_attributed_once_with_a_stable_swarm_remainder() {
    let mut app = common::minimal_app();
    app.add_plugins(CombatPlugin)
        .init_resource::<BattleCounters>();
    for swarm in [SwarmId::PLAYER, SwarmId(11), SwarmId(22), SwarmId(99)] {
        app.world_mut().spawn((Swarm {}, swarm));
    }
    let target = common::spawn_worker_at(&mut app, Vec2::ZERO);
    app.world_mut().entity_mut(target).insert((
        SwarmMember::new(SwarmId(99)),
        Health {
            current: 7,
            max: 100,
        },
    ));
    for (swarm, position) in [
        (SwarmId::PLAYER, Vec2::new(90.0, 0.0)),
        (SwarmId(11), Vec2::new(-45.0, 77.94)),
        (SwarmId(22), Vec2::new(-45.0, -77.94)),
    ] {
        let attacker = common::spawn_defender_at(&mut app, position);
        app.world_mut()
            .entity_mut(attacker)
            .insert((SwarmMember::new(swarm), DefenderAttackCooldown::new(0)));
        if swarm == SwarmId(22) {
            app.world_mut().entity_mut(attacker).insert(Charge {
                current: 0.15,
                max: 1.0,
            });
        }
    }

    app.update();

    assert_eq!(app.world().get::<Health>(target).unwrap().current, 0);
    let facts = resolved_facts(&app);
    let mut nominal_hits = facts
        .iter()
        .filter_map(|fact| match fact {
            ResolvedCombatFact::Hit(hit) if hit.target_destroyed => Some(hit.damage),
            ResolvedCombatFact::Death(_) => None,
            fact => panic!("expected nominal lethal hit facts and one death: {fact:?}"),
        })
        .collect::<Vec<_>>();
    nominal_hits.sort_unstable();
    assert_eq!(nominal_hits, [18, 26, 26]);
    assert_eq!(
        effective_damage(&app, SwarmId::PLAYER)["effective_damage_total"],
        3
    );
    assert_eq!(
        effective_damage(&app, SwarmId(11))["effective_damage_total"],
        3
    );
    assert_eq!(
        effective_damage(&app, SwarmId(22))["effective_damage_total"],
        1
    );
    assert_eq!(
        [SwarmId::PLAYER, SwarmId(11), SwarmId(22)]
            .into_iter()
            .map(
                |swarm| effective_damage(&app, swarm)["effective_damage_total"]
                    .as_u64()
                    .unwrap()
            )
            .sum::<u64>(),
        7,
        "simultaneous nominal hits must not each claim the capped lethal damage",
    );
}

#[test]
fn repaired_health_can_be_removed_again_but_friendly_targets_never_count() {
    let mut app = common::minimal_app();
    app.add_plugins(CombatPlugin)
        .init_resource::<BattleCounters>();
    app.world_mut().spawn((Swarm {}, SwarmId::PLAYER));
    let target = common::spawn_worker_at(&mut app, Vec2::ZERO);
    app.world_mut()
        .entity_mut(target)
        .insert(Health { current: 5, max: 5 });
    let attacker = common::spawn_defender_at(&mut app, Vec2::X * 80.0);
    app.world_mut()
        .entity_mut(attacker)
        .insert(DefenderAttackCooldown::new(0));

    app.update();
    assert_eq!(
        effective_damage(&app, SwarmId::PLAYER)["effective_damage_total"],
        0
    );
    assert_eq!(app.world().get::<Health>(target).unwrap().current, 5);

    app.world_mut()
        .entity_mut(target)
        .insert(SwarmMember::new(SwarmId(11)));
    app.world_mut()
        .entity_mut(attacker)
        .insert(DefenderAttackCooldown::new(0));
    app.world_mut().spawn((Swarm {}, SwarmId(11)));
    app.update();
    assert_eq!(
        effective_damage(&app, SwarmId::PLAYER)["effective_damage_total"],
        5
    );

    app.world_mut()
        .entity_mut(target)
        .get_mut::<Health>()
        .unwrap()
        .current = 5;
    app.world_mut().spawn((Swarm {}, SwarmId(22)));
    let second_attacker = common::spawn_defender_at(&mut app, -Vec2::X * 80.0);
    app.world_mut().entity_mut(second_attacker).insert((
        SwarmMember::new(SwarmId(22)),
        DefenderAttackCooldown::new(0),
    ));
    app.update();
    assert_eq!(
        effective_damage(&app, SwarmId::PLAYER)["effective_damage_total"],
        5
    );
    assert_eq!(
        effective_damage(&app, SwarmId(22))["effective_damage_total"],
        5
    );
    assert_eq!(
        effective_damage(&app, SwarmId::PLAYER)["effective_damage_total"]
            .as_u64()
            .unwrap()
            + effective_damage(&app, SwarmId(22))["effective_damage_total"]
                .as_u64()
                .unwrap(),
        10,
        "repair permits more real damage to accumulate",
    );
}

#[test]
fn firing_prefers_defenders_then_other_nanobots_then_structures() {
    let mut app = common::minimal_app();
    app.add_plugins(CombatPlugin);
    app.world_mut()
        .resource_mut::<Tuning>()
        .combat
        .attack_interval_ticks = 1;
    let opponent = SwarmId(11);
    let opponent_swarm = app.world_mut().spawn((Swarm {}, opponent)).id();
    let attacker = common::spawn_defender_at(&mut app, Vec2::ZERO);
    let nearest = common::spawn_defender_at(&mut app, Vec2::X * 80.0);
    let tied = common::spawn_defender_at(&mut app, -Vec2::X * 80.0);
    let farther = common::spawn_defender_at(&mut app, Vec2::Y * 90.0);
    for hostile in [nearest, tied, farther] {
        app.world_mut().entity_mut(hostile).insert((
            SwarmMember::new(opponent),
            Charge {
                current: 0.0,
                max: 1.0,
            },
        ));
    }
    let worker = common::spawn_worker_at(&mut app, Vec2::X * 30.0);
    app.world_mut()
        .entity_mut(worker)
        .insert(SwarmMember::new(opponent));
    let hauler = common::spawn_hauler_at(&mut app, Vec2::X * 50.0);
    app.world_mut()
        .entity_mut(hauler)
        .insert(SwarmMember::new(opponent));
    app.world_mut()
        .entity_mut(attacker)
        .insert(DefenderResponse { target: hauler });
    let nearby_structure = app
        .world_mut()
        .spawn((
            Structure::new(StructureKind::Basic),
            OwnerSwarm(opponent_swarm),
            Transform::from_xyz(20.0, 0.0, 0.0),
        ))
        .id();
    let mut destroyed_structure = Structure::new(StructureKind::Basic);
    destroyed_structure.health = 0;
    app.world_mut().spawn((
        destroyed_structure,
        OwnerSwarm(opponent_swarm),
        Transform::from_xyz(10.0, 0.0, 0.0),
    ));

    let mut cursor = app
        .world()
        .resource::<Messages<ResolvedCombatFact>>()
        .get_cursor();
    for expected in [nearest, tied, farther, worker, hauler, nearby_structure] {
        app.update();
        let messages = app.world().resource::<Messages<ResolvedCombatFact>>();
        let [ResolvedCombatFact::Hit(hit)] = cursor.read(messages).copied().collect::<Vec<_>>()[..]
        else {
            panic!("the attacker should deal one hit");
        };
        assert_eq!(hit.attacker.entity, attacker);
        assert_eq!(hit.target.entity, expected);
        app.world_mut().despawn(expected);
    }
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
fn acquisition_and_jittered_cadence_are_bounded_and_frame_independent() {
    const DEFENDER_SPAWN_TICK: u32 = 1;

    #[derive(Default, Resource)]
    struct FiringTimeline {
        tick: u32,
        hits: BTreeMap<u32, Vec<u32>>,
    }

    let simulate = |ticks_per_frame| {
        let mut app = common::minimal_app();
        let timestep = app.world().resource::<Time<Fixed>>().timestep();
        app.insert_resource(TimeUpdateStrategy::ManualDuration(
            timestep * ticks_per_frame,
        ))
        .init_resource::<FiringTimeline>()
        .add_plugins((CombatPlugin, StructureOverlayPlugin))
        .add_systems(
            FixedPreUpdate,
            |mut commands: Commands, timeline: Res<FiringTimeline>, tuning: Res<Tuning>| {
                if timeline.tick == 0 {
                    // The facility's render entities can be allocated before or after
                    // the Defenders, depending on how many fixed ticks a frame runs.
                    commands.spawn((
                        ProductionFacility::with_tuning(&tuning),
                        Transform::from_xyz(10_000.0, 0.0, 0.0),
                    ));
                } else if timeline.tick == DEFENDER_SPAWN_TICK {
                    for index in 0..6 {
                        commands.spawn((
                            NanobotBundle {
                                nanobot_type: NanobotType::Defender,
                                ..default()
                            },
                            Charge::with_tuning(&tuning),
                            Transform::from_xyz(index as f32 * 320.0, 0.0, 0.0),
                        ));
                    }
                }
            },
        )
        .add_systems(
            FixedLast,
            |mut timeline: ResMut<FiringTimeline>, mut facts: MessageReader<ResolvedCombatFact>| {
                let tick = timeline.tick;
                for fact in facts.read() {
                    if let ResolvedCombatFact::Hit(hit) = fact {
                        timeline
                            .hits
                            .entry(hit.attacker.position.x.to_bits())
                            .or_default()
                            .push(tick - DEFENDER_SPAWN_TICK);
                    }
                }
                timeline.tick += 1;
            },
        );
        for index in 0..6 {
            let position = Vec2::X * (index as f32 * 320.0) + Vec2::Y * 80.0;
            let worker = common::spawn_worker_at(&mut app, position);
            app.world_mut()
                .entity_mut(worker)
                .insert((SwarmMember::new(SwarmId(11)), Health::full(10_000)));
        }
        for _ in 0..180 / ticks_per_frame {
            app.update();
        }
        app.world_mut()
            .remove_resource::<FiringTimeline>()
            .unwrap()
            .hits
    };

    let hits = simulate(1);
    assert_eq!(hits, simulate(2));
    assert_eq!(hits.len(), 6);
    let tuning = Tuning::default().combat;
    let interval = u32::from(tuning.attack_interval_ticks);
    let max_jitter = (interval as f32 * tuning.cadence_jitter).round() as u32;
    let mut first_hits = Vec::new();
    let mut intervals = Vec::new();
    for ticks in hits.values() {
        assert!(ticks.len() >= 4);
        assert!(
            ticks[0] < interval,
            "first shot exceeded its acquisition window"
        );
        first_hits.push(ticks[0]);
        for pair in ticks.windows(2) {
            let elapsed = pair[1] - pair[0];
            assert!((interval - max_jitter..=interval + max_jitter).contains(&elapsed));
            intervals.push(elapsed);
        }
    }
    assert!(first_hits.iter().any(|tick| *tick != first_hits[0]));
    assert!(intervals.iter().any(|elapsed| *elapsed != intervals[0]));
}

#[test]
fn continuous_in_range_targets_keep_cadence_and_reacquisition_waits_for_both_delays() {
    let mut app = common::minimal_app();
    app.add_plugins(CombatPlugin);
    let interval = 30;
    {
        let mut tuning = app.world_mut().resource_mut::<Tuning>();
        tuning.combat.attack_interval_ticks = interval;
        tuning.combat.cadence_jitter = 0.0;
    }
    common::spawn_defender_at(&mut app, Vec2::ZERO);
    let target = common::spawn_worker_at(&mut app, Vec2::X * 80.0);
    let replacement = common::spawn_worker_at(&mut app, Vec2::X * 200.0);
    for entity in [target, replacement] {
        app.world_mut()
            .entity_mut(entity)
            .insert(SwarmMember::new(SwarmId(11)));
    }
    let offset_until_hit = |app: &mut App, target, limit| {
        let health_before = app.world().get::<Health>(target).unwrap().current;
        (0..limit)
            .find(|_| {
                app.update();
                app.world().get::<Health>(target).unwrap().current < health_before
            })
            .expect("in-range target should receive a hit")
    };
    let set_x = |app: &mut App, entity, x| {
        app.world_mut()
            .get_mut::<Transform>(entity)
            .unwrap()
            .translation
            .x = x;
    };

    let initial_delay = offset_until_hit(&mut app, target, interval);
    for _ in 1..interval {
        app.update();
    }
    set_x(&mut app, target, 200.0);
    set_x(&mut app, replacement, 80.0);
    assert_eq!(offset_until_hit(&mut app, replacement, interval), 0);

    for out_of_range_ticks in [1, interval] {
        set_x(&mut app, replacement, 200.0);
        for _ in 0..out_of_range_ticks {
            app.update();
        }
        set_x(&mut app, replacement, 80.0);
        let recovery_remaining = interval.saturating_sub(out_of_range_ticks + 1);
        assert_eq!(
            offset_until_hit(&mut app, replacement, interval),
            initial_delay.max(recovery_remaining),
        );
    }
}

#[test]
fn simultaneous_lethal_hits_still_resolve_for_both_defenders() {
    let mut app = common::sim_app_with_combat();
    app.world_mut().spawn((Swarm {}, SwarmId::PLAYER));
    app.world_mut().spawn((Swarm {}, SwarmId(11)));
    app.add_systems(FixedLast, nanobot_death_cleanup_system);
    let cell = IVec2::ZERO;
    {
        let mut grid = app.world_mut().resource_mut::<IntentGrid>();
        grid.paint(cell, IntentKind::Gather, SwarmId::PLAYER);
        grid.paint(cell, IntentKind::Build, SwarmId(11));
    }
    let center = common::cell_world_center(cell);
    let player = common::spawn_defender_at(&mut app, center - Vec2::X * 16.0);
    let opponent = common::spawn_defender_at(&mut app, center + Vec2::X * 16.0);
    for entity in [player, opponent] {
        app.world_mut().entity_mut(entity).insert((
            Health {
                current: 5,
                max: 100,
            },
            DefenderAttackCooldown::new(0),
        ));
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
        "both Defenders should be removed after simultaneous hits; facts={facts:?}",
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
    app.world_mut().spawn((Swarm {}, SwarmId(11)));
    let cell = IVec2::ZERO;
    paint_player_territory(&mut app, cell);
    let center = common::cell_world_center(cell);
    let defender = common::spawn_defender_at(&mut app, center);
    app.world_mut()
        .entity_mut(defender)
        .insert(DefenderAttackCooldown::new(0));
    let dead = common::spawn_worker_at(&mut app, center + Vec2::X * 20.0);
    app.world_mut().entity_mut(dead).insert((
        SwarmMember::new(SwarmId(11)),
        Health {
            current: 0,
            max: 100,
        },
    ));
    let live = common::spawn_worker_at(&mut app, center + Vec2::X * 40.0);
    app.world_mut()
        .entity_mut(live)
        .insert(SwarmMember::new(SwarmId(11)));

    app.update();

    assert_eq!(
        app.world().entity(live).get::<Health>().unwrap().current,
        74
    );
    let facts = resolved_facts(&app);
    let [ResolvedCombatFact::Hit(hit)] = facts.as_slice() else {
        panic!("the nearest living hostile should receive one hit: {facts:?}");
    };
    assert_eq!(hit.target.entity, live);
}

#[test]
fn lethal_structure_hit_publishes_stable_appearance_and_despawns_target() {
    let mut app = common::sim_app_with_combat();
    app.init_resource::<BattleCounters>();
    app.world_mut().spawn((Swarm {}, SwarmId::PLAYER));
    let opponent = app.world_mut().spawn((Swarm {}, SwarmId(11))).id();
    let cell = IVec2::ZERO;
    paint_player_territory(&mut app, cell);
    let center = common::cell_world_center(cell);
    let attacker = common::spawn_defender_at(&mut app, center - Vec2::X * 16.0);
    app.world_mut()
        .entity_mut(attacker)
        .insert(DefenderAttackCooldown::new(0));
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
    assert_eq!(
        hit.damage, 7,
        "presentation retains nominal structure damage"
    );
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
    assert_eq!(
        app.world()
            .resource::<BattleCounters>()
            .totals_for(SwarmId(11))
            .structures_lost,
        1
    );
    let damage = effective_damage(&app, SwarmId::PLAYER);
    assert_eq!(damage["effective_damage_total"], 1);
    assert_eq!(damage["effective_damage_nanobots"], 0);
    assert_eq!(damage["effective_damage_structures"], 1);
}

#[derive(Debug, Clone, Copy)]
enum DefenderDuty {
    Staging,
    Passing,
    Pursuit,
    Combat,
    Charge,
}

#[test]
fn defender_presence_never_captures_paint_in_any_duty() {
    for duty in [
        DefenderDuty::Staging,
        DefenderDuty::Passing,
        DefenderDuty::Pursuit,
        DefenderDuty::Combat,
        DefenderDuty::Charge,
    ] {
        let mut app = common::sim_app_with_combat();
        app.world_mut().spawn((Swarm {}, SwarmId::PLAYER));
        app.world_mut().spawn((Swarm {}, SwarmId(11)));
        app.world_mut().resource_mut::<Tuning>().logistics.bot_speed = 0.0;
        let cell = IVec2::ZERO;
        {
            let mut grid = app.world_mut().resource_mut::<IntentGrid>();
            grid.paint(cell, IntentKind::Defend, SwarmId::PLAYER);
            grid.paint(cell, IntentKind::Defend, SwarmId(11));
        }
        let center = common::cell_world_center(cell);
        let challenger = common::spawn_defender_at(&mut app, center);
        app.world_mut()
            .entity_mut(challenger)
            .insert(SwarmMember::new(SwarmId(11)));

        match duty {
            DefenderDuty::Staging => {}
            DefenderDuty::Passing => {
                app.world_mut()
                    .entity_mut(challenger)
                    .insert(DirectMovementComponent {
                        speed: None,
                        interaction: None,
                        xy: center + Vec2::X * 200.0,
                        stop_radius: 0.0,
                    });
            }
            DefenderDuty::Pursuit | DefenderDuty::Combat => {
                app.world_mut().resource_mut::<IntentGrid>().paint(
                    cell,
                    IntentKind::Build,
                    SwarmId(11),
                );
                let offset = if matches!(duty, DefenderDuty::Combat) {
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
            DefenderDuty::Charge => {
                app.world_mut()
                    .entity_mut(challenger)
                    .insert(ChargerAssignment {
                        charger: Entity::PLACEHOLDER,
                    });
            }
        }

        for _ in 0..240 {
            app.update();
        }

        assert_eq!(
            app.world()
                .resource::<IntentGrid>()
                .cell(cell)
                .map(|intent| intent.owners(IntentKind::Defend).collect::<Vec<_>>()),
            Some(vec![SwarmId::PLAYER, SwarmId(11)]),
            "paint ownership must survive {duty:?} duty",
        );
    }
}

#[test]
fn combat_death_preserves_both_swarms_paint() {
    let mut app = common::sim_app_with_combat();
    app.world_mut().spawn((Swarm {}, SwarmId::PLAYER));
    app.world_mut().spawn((Swarm {}, SwarmId(11)));
    let cell = IVec2::ZERO;
    {
        let mut grid = app.world_mut().resource_mut::<IntentGrid>();
        grid.paint(cell, IntentKind::Defend, SwarmId::PLAYER);
        grid.paint(cell, IntentKind::Defend, SwarmId(11));
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
            .cell(cell)
            .unwrap()
            .has_owned(IntentKind::Defend, SwarmId::PLAYER)
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
            .map(|intent| intent.owners(IntentKind::Defend).collect::<Vec<_>>()),
        Some(vec![SwarmId::PLAYER, SwarmId(11)]),
    );
}
