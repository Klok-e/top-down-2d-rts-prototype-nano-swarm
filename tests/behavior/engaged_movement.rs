use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    nanobot::{
        ClearingEvacuation, CombatPlugin, DefenderEngagements, DefenderResponse,
        DirectMovementComponent, StructureClearing, SwarmId, SwarmMember, VelocityComponent,
        defender_engagement_system, separation_system,
    },
    navigation::{Obstacle, align_structure},
    tuning::Tuning,
};

#[path = "../common/mod.rs"]
mod common;

fn position(app: &App, defender: Entity) -> Vec2 {
    app.world()
        .get::<Transform>(defender)
        .unwrap()
        .translation
        .truncate()
}

#[test]
fn engaged_responder_approaches_the_nearest_hostile_defender_at_full_speed() {
    let mut app = common::sim_app_with_movement();
    let tuning = &mut app.world_mut().resource_mut::<Tuning>().combat;
    tuning.attack_range = 96.0;
    tuning.band_depth = 5.0;
    let start = Vec2::splat(256.0);
    let defender = common::spawn_defender_at(&mut app, start);
    let nearby = common::spawn_defender_at(&mut app, start + Vec2::X * 93.0);
    let claimed_position = start + Vec2::Y * 168.0;
    let claimed = common::spawn_defender_at(&mut app, claimed_position);
    for hostile in [nearby, claimed] {
        app.world_mut()
            .entity_mut(hostile)
            .insert(SwarmMember(SwarmId(1)));
    }
    app.world_mut().entity_mut(defender).insert((
        DefenderResponse { target: claimed },
        DirectMovementComponent {
            xy: claimed_position,
            stop_radius: 96.0,
            speed: None,
            interaction: None,
        },
    ));

    app.update();

    let displacement = position(&app, defender) - start;
    let speed = app.world().resource::<Tuning>().logistics.bot_speed;
    assert!(
        (displacement - Vec2::X * speed).length() < 0.01,
        "defender should close with the nearby hostile at full speed, moved {displacement:?}"
    );
    assert_eq!(
        app.world()
            .get::<DefenderResponse>(defender)
            .unwrap()
            .target,
        claimed
    );
}

#[test]
fn defender_strafes_in_the_engagement_band_for_300_ticks() {
    let mut app = common::sim_app_with_movement();
    app.add_plugins(CombatPlugin);
    let (inner, outer) = {
        let combat = &mut app.world_mut().resource_mut::<Tuning>().combat;
        combat.base_attack = 0.0;
        (
            combat.attack_range - combat.band_depth,
            combat.attack_range - 4.0,
        )
    };
    let tuning = app.world().resource::<Tuning>();
    let strafe_speed = tuning.combat.strafe_speed_fraction * tuning.logistics.bot_speed;
    let hostile_position = Vec2::splat(256.0);
    let hostile = common::spawn_defender_at(&mut app, hostile_position);
    app.world_mut()
        .entity_mut(hostile)
        .insert(SwarmMember(SwarmId(1)))
        .remove::<VelocityComponent>();
    let defender = common::spawn_defender_at(&mut app, hostile_position - Vec2::X * 144.0);

    for _ in 0..32 {
        app.update();
        if position(&app, defender).distance(hostile_position) <= outer {
            break;
        }
    }
    assert!(
        (inner..=outer).contains(&position(&app, defender).distance(hostile_position)),
        "defender did not reach the engagement band"
    );
    let mut tangent_distance = 0.0;
    for tick in 1..=300 {
        let previous = position(&app, defender);
        app.update();
        let current = position(&app, defender);
        let radius = current.distance(hostile_position);
        assert!(
            (inner - 2.0..=outer + 2.0).contains(&radius),
            "defender left the engagement band at tick {tick}, distance {radius}"
        );
        let radial = (previous - hostile_position).normalize();
        let displacement = current - previous;
        let tangent_step = displacement.dot(radial.perp()).abs();
        if (inner..=outer).contains(&previous.distance(hostile_position)) {
            assert!(
                displacement.dot(radial).abs() < 0.01 && (tangent_step - strafe_speed).abs() < 0.01,
                "strafe at tick {tick} was {displacement:?} for radial direction {radial:?}"
            );
        }
        tangent_distance += tangent_step;
    }
    assert!(tangent_distance > 100.0, "defender must keep strafing");
    assert_eq!(position(&app, hostile), hostile_position);
}

#[test]
fn engaged_separation_redirects_toward_the_less_crowded_side() {
    let mut app = common::minimal_app();
    app.init_resource::<DefenderEngagements>().add_systems(
        FixedUpdate,
        (defender_engagement_system, separation_system).chain(),
    );
    let combat = &mut app.world_mut().resource_mut::<Tuning>().combat;
    combat.strafe_speed_fraction = 0.0;
    let band_center = combat.attack_range - (combat.band_depth + 4.0) * 0.5;
    let front = Vec2::splat(256.0);
    let defender = common::spawn_defender_at(&mut app, front);
    let hostile = common::spawn_defender_at(&mut app, front + Vec2::X * band_center);
    app.world_mut()
        .entity_mut(hostile)
        .insert(SwarmMember(SwarmId(1)));
    common::spawn_defender_at(&mut app, front - Vec2::X * 34.0);
    common::spawn_defender_at(&mut app, front + Vec2::Y * 95.0);

    app.update();

    let push = app
        .world()
        .get::<VelocityComponent>(defender)
        .unwrap()
        .value;
    assert!(
        push.x.abs() < 0.001,
        "separation must not push forward: {push:?}"
    );
    assert!(
        push.y < -1.0,
        "separation should push to the vacant side: {push:?}"
    );
}

#[test]
fn an_engaged_defender_clears_a_completing_footprint() {
    let mut app = common::sim_app_with_movement();
    let plan = common::spawn_planned_structure_at_cell(&mut app, IVec2::ZERO);
    let transform = align_structure(*app.world().get::<Transform>(plan).unwrap());
    let start = transform.translation.truncate();
    let exit = start - Vec2::X * 108.0;
    let hostile_position = start + Vec2::X * 144.0;
    let defender = common::spawn_defender_at(&mut app, start);
    let hostile = common::spawn_defender_at(&mut app, hostile_position);
    app.world_mut()
        .entity_mut(plan)
        .insert((transform, StructureClearing::validated(exit, 0)));
    app.world_mut()
        .entity_mut(hostile)
        .insert(SwarmMember(SwarmId(1)))
        .remove::<VelocityComponent>();
    app.world_mut().entity_mut(defender).insert((
        ClearingEvacuation {
            structure: plan,
            goal: exit,
        },
        DirectMovementComponent {
            xy: hostile_position,
            stop_radius: 0.0,
            speed: None,
            interaction: None,
        },
    ));

    let footprint = Obstacle::structure(&transform);
    for _ in 0..40 {
        app.update();
        let current = position(&app, defender);
        if footprint.admits_body(current) {
            assert!(
                current.x < start.x,
                "defender must leave by its assigned exit"
            );
            return;
        }
    }
    panic!("engaged defender failed to clear the completing footprint");
}

#[test]
fn recovering_traffic_does_not_overlap_an_engaged_defender() {
    let mut app = common::sim_app_with_movement();
    let mut tuning = app.world_mut().resource_mut::<Tuning>();
    tuning.combat.strafe_speed_fraction = 0.0;
    for y in [252.0, 396.0] {
        let wall = common::spawn_structure_at(&mut app, Vec2::new(432.0, y));
        app.world_mut().get_mut::<Transform>(wall).unwrap().scale = Vec3::new(4.5, 1.125, 1.0);
    }
    let front = Vec2::new(432.0, 324.0);
    let defender = common::spawn_defender_at(&mut app, front);
    let combat = &app.world().resource::<Tuning>().combat;
    let band_center = combat.attack_range - (combat.band_depth + 4.0) * 0.5;
    let hostile = common::spawn_defender_at(&mut app, front + Vec2::X * band_center);
    app.world_mut()
        .entity_mut(hostile)
        .insert(SwarmMember(SwarmId(1)))
        .remove::<VelocityComponent>();
    let traveller = common::spawn_worker_at(&mut app, Vec2::new(360.0, 324.0));
    app.world_mut()
        .entity_mut(traveller)
        .insert(DirectMovementComponent {
            xy: Vec2::new(504.0, 324.0),
            stop_radius: 0.0,
            speed: None,
            interaction: None,
        });

    let mut closest = f32::INFINITY;
    for _ in 0..180 {
        app.update();
        let distance = position(&app, traveller).distance(position(&app, defender));
        assert!(
            distance >= 67.999,
            "recovering traffic overlapped the front at distance {distance}"
        );
        closest = closest.min(distance);
    }
    assert!(closest < 68.1, "traveller did not reach the blocked front");
}
