use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    intent::{IntentGrid, IntentKind},
    nanobot::{
        Charge, ChargerAssignment, DefenderDisengagement, DefenderResponse, DefenderRout,
        DirectMovementComponent, Health, OwnerSwarm, SwarmId, SwarmMember, VelocityComponent,
        world_to_cell,
    },
    navigation::{BODY_RADIUS, Obstacle},
    tuning::Tuning,
};

#[path = "../common/mod.rs"]
mod common;

const OPPONENT: SwarmId = SwarmId(1);

fn position(app: &App, entity: Entity) -> Vec2 {
    app.world()
        .get::<Transform>(entity)
        .unwrap()
        .translation
        .truncate()
}

fn move_hostile(app: &mut App, entity: Entity, position: Vec2) {
    app.world_mut()
        .get_mut::<Transform>(entity)
        .unwrap()
        .translation = position.extend(0.0);
}

fn paint(app: &mut App, cell: IVec2, kind: IntentKind) {
    app.world_mut()
        .resource_mut::<IntentGrid>()
        .paint(cell, kind, SwarmId::PLAYER);
}

#[test]
fn weakened_defender_releases_response_and_holds_until_charger_admits_it() {
    let mut app = common::sim_app_with_charge();
    let center = common::cell_world_center(IVec2::ZERO);
    let owner = common::spawn_swarm_at(&mut app, center);
    paint(&mut app, IVec2::ZERO, IntentKind::Defend);
    let charger_cell = -IVec2::X;
    paint(&mut app, charger_cell, IntentKind::Defend);
    let charger = common::spawn_operational_charger_at(&mut app, charger_cell, 100);
    app.world_mut()
        .entity_mut(charger)
        .insert(OwnerSwarm(owner));
    let rotating = common::spawn_defender_at(&mut app, center + Vec2::Y * 600.0);
    app.world_mut()
        .entity_mut(rotating)
        .insert((
            Charge {
                current: 0.2,
                max: 1.0,
            },
            ChargerAssignment { charger },
        ))
        .remove::<VelocityComponent>();

    let hostile_position = center + Vec2::X * 144.0;
    let hostile = common::spawn_stationary_defender_at(&mut app, hostile_position, OPPONENT);
    let defender = common::spawn_defender_at(&mut app, center);
    let tuning = app.world().resource::<Tuning>();
    let weakened = tuning.charge.weakened_threshold;
    let low = tuning.charge.low_threshold;
    let desired_distance = tuning.combat.attack_range + 4.0 * BODY_RADIUS;
    let charge_tuning = &mut app.world_mut().resource_mut::<Tuning>().charge;
    charge_tuning.drain_per_tick = 0.0;
    charge_tuning.pulse_interval_ticks = 5;
    app.world_mut().entity_mut(defender).insert((
        Charge {
            current: weakened * 0.75,
            max: 1.0,
        },
        DefenderResponse { target: hostile },
    ));

    app.update();
    assert!(app.world().get::<DefenderResponse>(defender).is_none());
    assert!(app.world().get::<DefenderDisengagement>(defender).is_some());
    assert!(position(&app, defender).x < center.x);

    app.world_mut().get_mut::<Charge>(defender).unwrap().current = (weakened + low) * 0.5;
    for _ in 0..80 {
        app.update();
    }
    let holding = position(&app, defender);
    assert!(holding.distance(hostile_position) >= desired_distance - 0.001);
    assert!((holding.distance(hostile_position) - desired_distance).abs() < 0.01);
    for _ in 0..20 {
        app.update();
        assert_eq!(position(&app, defender), holding);
        assert!(app.world().get::<DefenderResponse>(defender).is_none());
        assert!(app.world().get::<ChargerAssignment>(defender).is_none());
    }

    app.world_mut().despawn(rotating);
    for _ in 0..20 {
        app.update();
        if app.world().get::<ChargerAssignment>(defender).is_some() {
            break;
        }
    }
    assert_eq!(
        app.world()
            .get::<ChargerAssignment>(defender)
            .map(|state| state.charger),
        Some(charger)
    );
    for _ in 0..300 {
        app.update();
        if app.world().get::<DefenderDisengagement>(defender).is_none() {
            break;
        }
    }
    assert!(app.world().get::<Charge>(defender).unwrap().current >= low);
    assert!(app.world().get::<DefenderDisengagement>(defender).is_none());
}

#[test]
fn rout_confirmation_restarts_when_local_strength_recovers() {
    let mut app = common::sim_app();
    let cell = IVec2::ONE;
    let retreat_cell = cell - IVec2::X;
    paint(&mut app, cell, IntentKind::Defend);
    paint(&mut app, retreat_cell, IntentKind::Defend);
    let center = common::cell_world_center(cell);
    let hostile =
        common::spawn_stationary_defender_at(&mut app, center + Vec2::X * 160.0, OPPONENT);
    let defender = common::spawn_defender_at(&mut app, center);
    app.world_mut()
        .entity_mut(defender)
        .insert((
            Health {
                current: 40,
                max: 100,
            },
            DefenderResponse { target: hostile },
        ))
        .remove::<VelocityComponent>();
    let confirm = 4;
    app.world_mut()
        .resource_mut::<Tuning>()
        .combat
        .rout_confirm_ticks = confirm;

    for _ in 0..confirm {
        app.update();
        assert!(app.world().get::<DefenderRout>(defender).is_none());
    }
    app.world_mut().get_mut::<Health>(defender).unwrap().current = 20;

    for _ in 0..confirm - 1 {
        app.update();
        assert!(app.world().get::<DefenderRout>(defender).is_none());
    }
    let weakened = app.world().resource::<Tuning>().charge.weakened_threshold;
    app.world_mut().get_mut::<Charge>(hostile).unwrap().current = weakened * 0.1;
    app.update();
    assert!(app.world().get::<DefenderRout>(defender).is_none());
    app.world_mut().get_mut::<Charge>(hostile).unwrap().current = 1.0;
    for _ in 0..confirm - 1 {
        app.update();
        assert!(app.world().get::<DefenderRout>(defender).is_none());
        assert_eq!(
            app.world()
                .get::<DefenderResponse>(defender)
                .map(|response| response.target),
            Some(hostile)
        );
    }

    app.update();
    assert_eq!(
        app.world()
            .get::<DefenderRout>(defender)
            .map(|rout| rout.cell),
        Some(retreat_cell)
    );
    assert!(app.world().get::<DefenderResponse>(defender).is_none());
}

#[test]
fn rout_reaches_safe_cell_and_rejoins_after_minimum_time_clearance_and_charge_recovery() {
    let mut app = common::sim_app();
    let cell = IVec2::ONE;
    let retreat_cell = cell - IVec2::X;
    paint(&mut app, cell, IntentKind::Defend);
    paint(&mut app, retreat_cell, IntentKind::Defend);
    let start = common::cell_world_center(cell);
    let destination = common::cell_world_center(retreat_cell);
    let obstacle_at = |app: &mut App, point| {
        let structure = common::spawn_structure_at(app, point);
        Obstacle::structure(app.world().get::<Transform>(structure).unwrap())
    };
    let obstacles = [
        obstacle_at(&mut app, start.midpoint(destination)),
        obstacle_at(&mut app, destination),
    ];
    assert!(!obstacles[1].admits_body(destination));
    let hostile_position = start + Vec2::X * 160.0;
    let hostile = common::spawn_stationary_defender_at(&mut app, hostile_position, OPPONENT);
    let defender = common::spawn_defender_at(&mut app, start);
    app.world_mut().entity_mut(defender).insert((
        Health {
            current: 20,
            max: 100,
        },
        Charge {
            current: 0.2,
            max: 1.0,
        },
    ));
    let minimum = 220;
    let mut tuning = app.world_mut().resource_mut::<Tuning>();
    tuning.combat.rout_confirm_ticks = 1;
    tuning.combat.rout_min_ticks = minimum;
    let clear_distance = 2.0 * tuning.combat.rout_radius;
    assert!(destination.distance(hostile_position) < clear_distance);

    app.update();
    assert!(app.world().get::<DefenderRout>(defender).is_some());
    let mut arrived = false;
    for _ in 0..minimum - 2 {
        app.update();
        let current = position(&app, defender);
        assert!(
            obstacles
                .iter()
                .all(|obstacle| obstacle.admits_body(current))
        );
        arrived |= world_to_cell(current) == retreat_cell
            && app
                .world()
                .get::<DirectMovementComponent>(defender)
                .is_none();
        assert!(app.world().get::<DefenderResponse>(defender).is_none());
    }
    assert!(
        arrived,
        "routing Defender failed to reach {destination:?}, ended at {:?}",
        position(&app, defender)
    );
    assert_eq!(world_to_cell(position(&app, defender)), retreat_cell);

    let distant_hostile = destination + Vec2::X * (clear_distance + 256.0);
    move_hostile(&mut app, hostile, distant_hostile);
    app.update();
    assert!(
        app.world().get::<DefenderRout>(defender).is_some(),
        "clearance alone cannot end routing before the minimum"
    );

    move_hostile(&mut app, hostile, hostile_position);
    app.update();
    assert!(
        app.world().get::<DefenderRout>(defender).is_some(),
        "reaching the minimum cannot end routing with hostiles nearby"
    );

    move_hostile(&mut app, hostile, distant_hostile);
    let worker = common::spawn_worker_at(&mut app, destination + Vec2::Y * 140.0);
    app.world_mut()
        .entity_mut(worker)
        .insert(SwarmMember(OPPONENT));
    app.update();
    assert!(app.world().get::<DefenderRout>(defender).is_none());
    assert!(app.world().get::<DefenderResponse>(defender).is_none());
    let low = app.world().resource::<Tuning>().charge.low_threshold;
    app.world_mut().get_mut::<Charge>(defender).unwrap().current = low;
    app.update();
    assert_eq!(
        app.world()
            .get::<DefenderResponse>(defender)
            .map(|response| response.target),
        Some(worker)
    );
}

#[test]
fn unsafe_defend_paint_prevents_routing_to_other_swarm_tiles() {
    let mut app = common::sim_app();
    let cell = IVec2::ONE;
    let fallback = cell - IVec2::X;
    paint(&mut app, cell, IntentKind::Defend);
    paint(&mut app, fallback, IntentKind::Gather);
    let center = common::cell_world_center(cell);
    let hostile =
        common::spawn_stationary_defender_at(&mut app, center + Vec2::X * 160.0, OPPONENT);
    let defender = common::spawn_defender_at(&mut app, center);
    app.world_mut().entity_mut(defender).insert((
        Health {
            current: 20,
            max: 100,
        },
        DefenderResponse { target: hostile },
    ));
    let confirm = 2;
    app.world_mut()
        .resource_mut::<Tuning>()
        .combat
        .rout_confirm_ticks = confirm;
    for _ in 0..confirm + 4 {
        app.update();
    }
    assert!(app.world().get::<DefenderRout>(defender).is_none());
    assert!(app.world().get::<DefenderResponse>(defender).is_some());

    app.world_mut()
        .resource_mut::<IntentGrid>()
        .erase(cell, IntentKind::Defend, SwarmId::PLAYER);
    app.update();
    assert_eq!(
        app.world()
            .get::<DefenderRout>(defender)
            .map(|rout| rout.cell),
        Some(fallback)
    );
    let movement = app
        .world()
        .get::<DirectMovementComponent>(defender)
        .unwrap();
    assert_eq!(movement.xy, common::cell_world_center(fallback));
}

#[test]
fn rout_uses_the_accessible_part_of_a_retreat_cell_across_a_wall() {
    let mut app = common::sim_app();
    app.insert_resource(IntentGrid::new(4, 4));
    let start_cell = IVec2::ONE;
    let retreat_cell = start_cell - IVec2::X;
    paint(&mut app, start_cell, IntentKind::Defend);
    paint(&mut app, retreat_cell, IntentKind::Defend);
    let start = common::cell_world_center(start_cell);
    let retreat_center = common::cell_world_center(retreat_cell);
    let wall = common::spawn_structure_at(&mut app, Vec2::new(360.0, 0.0));
    let mut wall_transform = app.world_mut().get_mut::<Transform>(wall).unwrap();
    wall_transform.scale.y = 33.0;
    let footprint = Obstacle::structure(&wall_transform);
    assert!(footprint.admits_body(retreat_center));
    common::spawn_stationary_defender_at(&mut app, start + Vec2::X * 160.0, OPPONENT);
    let defender = common::spawn_defender_at(&mut app, start);
    app.world_mut().entity_mut(defender).insert(Health {
        current: 20,
        max: 100,
    });
    app.world_mut()
        .resource_mut::<Tuning>()
        .combat
        .rout_confirm_ticks = 1;

    for _ in 0..240 {
        app.update();
        let current = position(&app, defender);
        assert!(footprint.admits_body(current));
        if world_to_cell(current) == retreat_cell
            && app
                .world()
                .get::<DirectMovementComponent>(defender)
                .is_none()
        {
            assert!(current.x > 360.0);
            assert_eq!(
                app.world().get::<DefenderRout>(defender).unwrap().cell,
                retreat_cell
            );
            return;
        }
    }
    panic!("Defender failed to reach the accessible part of its retreat cell");
}
