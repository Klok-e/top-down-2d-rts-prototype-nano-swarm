//! Finished production competes for exterior space through real movement.
use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    nanobot::{
        Commitment, DirectMovementComponent, Nanobot, NanobotType, OwnerSwarm,
        PRODUCTION_COST_PER_BOT, ProductionFacility, SwarmId, SwarmMember,
        production_facility_work_system,
    },
    resources::{ResourceKind, ResourceLedger},
};

#[path = "../common/mod.rs"]
mod common;

#[test]
fn both_swarms_release_paid_output_once_when_movement_opens_the_shared_exit() {
    let mut app = common::sim_app_with_movement();
    app.add_systems(Update, production_facility_work_system);
    let mut facilities = Vec::new();
    for (owner, kind, x, minerals) in [
        (SwarmId::PLAYER, NanobotType::Defender, 36., 20),
        (SwarmId(1), NanobotType::Hauler, 180., 33),
    ] {
        let swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
        app.world_mut().entity_mut(swarm).insert(owner);
        let remainder = minerals - PRODUCTION_COST_PER_BOT;
        let mut facility = ProductionFacility::new();
        facility.input_amount = remainder;
        facility.current_target = Some(kind);
        let facility = app
            .world_mut()
            .spawn((
                facility,
                OwnerSwarm(swarm),
                Transform::from_xyz(x, 36., 0.).with_scale(Vec3::new(1.125, 1.125, 1.)),
            ))
            .id();
        facilities.push(facility);
        app.world_mut().resource_mut::<ResourceLedger>().add_for(
            owner,
            ResourceKind::Minerals,
            remainder,
        );
    }
    let mut opening = None;
    for y in [-36., 36., 108.] {
        for x in [-36., 36., 108., 180., 252.] {
            if y == 36. && (x == 36. || x == 180.) {
                continue;
            }
            let bot = common::spawn_worker_at(&mut app, Vec2::new(x, y));
            app.world_mut().entity_mut(bot).insert(Commitment::Working);
            if x == 108. && y == -36. {
                opening = Some(bot);
            }
        }
    }
    for _ in 0..250 {
        app.update();
        assert_clear_bodies(app.world_mut());
    }
    assert_eq!(
        app.world_mut()
            .query_filtered::<Entity, With<Nanobot>>()
            .iter(app.world())
            .count(),
        13
    );
    for (facility, kind, remainder) in [
        (facilities[0], NanobotType::Defender, 0),
        (facilities[1], NanobotType::Hauler, 13),
    ] {
        let state = app.world().get::<ProductionFacility>(facility).unwrap();
        assert_eq!(state.current_target, Some(kind));
        assert_eq!(state.progress, 120);
        assert_eq!(state.input_amount, remainder);
        assert!(!state.is_busy());
    }
    app.world_mut()
        .entity_mut(opening.unwrap())
        .insert(DirectMovementComponent {
            xy: Vec2::new(108., -396.),
            stop_radius: 0.,
            interaction: None,
            speed: Some(5.),
        });
    let mut released = Vec::new();
    for _ in 0..400 {
        app.update();
        assert_clear_bodies(app.world_mut());
        let outputs: Vec<_> = app
            .world_mut()
            .query::<(Entity, &NanobotType, &SwarmMember, &Transform)>()
            .iter(app.world())
            .filter(|(_, kind, _, _)| **kind != NanobotType::Worker)
            .map(|(entity, kind, member, transform)| {
                (entity, *kind, member.0, transform.translation.truncate())
            })
            .collect();
        for (entity, kind, owner, position) in outputs {
            if released.contains(&entity) {
                continue;
            }
            assert!(
                position.distance(Vec2::new(108., -36.)) < 0.001,
                "output must use the only open cell: {position:?}"
            );
            assert_eq!(
                owner,
                if kind == NanobotType::Defender {
                    SwarmId::PLAYER
                } else {
                    SwarmId(1)
                }
            );
            released.push(entity);
            app.world_mut().entity_mut(entity).insert((
                Commitment::Working,
                DirectMovementComponent {
                    xy: Vec2::new(
                        if kind == NanobotType::Defender {
                            -180.
                        } else {
                            396.
                        },
                        -180.,
                    ),
                    stop_radius: 0.,
                    interaction: None,
                    speed: Some(5.),
                },
            ));
        }
    }
    assert_eq!(released.len(), 2, "each funded cycle releases exactly once");
    assert_eq!(
        app.world_mut()
            .query_filtered::<Entity, With<Nanobot>>()
            .iter(app.world())
            .count(),
        15
    );
    for (facility, owner, remainder) in [
        (facilities[0], SwarmId::PLAYER, 0),
        (facilities[1], SwarmId(1), 13),
    ] {
        let state = app.world().get::<ProductionFacility>(facility).unwrap();
        assert_eq!(state.current_target, None);
        assert_eq!(state.input_amount, remainder);
        assert_eq!(
            app.world()
                .resource::<ResourceLedger>()
                .total_for(owner, ResourceKind::Minerals),
            remainder
        );
    }
}

fn assert_clear_bodies(world: &mut World) {
    let positions: Vec<_> = world
        .query_filtered::<&Transform, With<Nanobot>>()
        .iter(world)
        .map(|transform| transform.translation.truncate())
        .collect();
    for (index, position) in positions.iter().enumerate() {
        for other in &positions[index + 1..] {
            assert!(
                position.distance(*other) >= 67.99,
                "bodies overlap: {position:?}, {other:?}"
            );
        }
        for center in [Vec2::new(36., 36.), Vec2::new(180., 36.)] {
            let distance = ((*position - center).abs() - Vec2::splat(36.))
                .max(Vec2::ZERO)
                .length();
            assert!(distance >= 33.99, "body overlaps facility: {position:?}");
        }
    }
}
