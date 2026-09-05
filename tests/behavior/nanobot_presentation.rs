use std::collections::HashMap;

use approx::assert_abs_diff_eq;
use bevy::{asset::AssetPlugin, prelude::*};
use top_down_2d_rts_prototype_nano_swarm::{
    GAMEPLAY_SPRITE_Z,
    intent::IntentGrid,
    nanobot::{
        Commitment, Health, Nanobot, NanobotPresentationPlugin, NanobotSprites, NanobotType,
        NanobotVisual, OpponentSwarm, OpponentSwarmIdAlloc, Swarm, SwarmId, SwarmMember,
        VelocityComponent, production_facility_work_system,
    },
    scenario::{
        OPPONENT_START_DEFENDERS, OPPONENT_START_HAULERS, OPPONENT_START_WORKERS,
        PLAYER_START_DEFENDERS, PLAYER_START_HAULERS, PLAYER_START_WORKERS,
        spawn_default_opponent_scenario, spawn_default_player_scenario,
    },
};

#[path = "../common/mod.rs"]
mod common;

fn spawn_authored_scenarios(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut grid: ResMut<IntentGrid>,
    opponent_id_alloc: ResMut<OpponentSwarmIdAlloc>,
) {
    spawn_default_player_scenario(&mut commands, &asset_server, &mut grid);
    spawn_default_opponent_scenario(&mut commands, &asset_server, &mut grid, opponent_id_alloc);
}

#[test]
fn authored_nanobots_render_through_one_neutral_presentation_child() {
    let mut app = App::new();
    app.add_plugins(TaskPoolPlugin::default())
        .add_plugins(AssetPlugin::default())
        .init_asset::<Image>()
        .insert_resource(IntentGrid::new(32, 32))
        .init_resource::<OpponentSwarmIdAlloc>()
        .add_plugins(NanobotPresentationPlugin)
        .add_systems(Startup, spawn_authored_scenarios);

    app.update();

    let world = app.world_mut();
    let opponent = world
        .query_filtered::<&SwarmId, (With<Swarm>, With<OpponentSwarm>)>()
        .single(world)
        .expect("authored opponent swarm must exist")
        .to_owned();
    let sprites = world.resource::<NanobotSprites>().clone();
    let roots = world
        .query_filtered::<(
            Entity,
            &NanobotType,
            &SwarmMember,
            &Transform,
            &Health,
            &VelocityComponent,
            &Commitment,
        ), With<Nanobot>>()
        .iter(world)
        .map(
            |(entity, kind, member, transform, health, velocity, commitment)| {
                (
                    entity,
                    *kind,
                    member.0,
                    *transform,
                    *health,
                    *velocity,
                    *commitment,
                )
            },
        )
        .collect::<Vec<_>>();

    let expected_counts = HashMap::from([
        (
            (SwarmId::PLAYER, NanobotType::Worker),
            PLAYER_START_WORKERS as usize,
        ),
        (
            (SwarmId::PLAYER, NanobotType::Hauler),
            PLAYER_START_HAULERS as usize,
        ),
        (
            (SwarmId::PLAYER, NanobotType::Defender),
            PLAYER_START_DEFENDERS as usize,
        ),
        (
            (opponent, NanobotType::Worker),
            OPPONENT_START_WORKERS as usize,
        ),
        (
            (opponent, NanobotType::Hauler),
            OPPONENT_START_HAULERS as usize,
        ),
        (
            (opponent, NanobotType::Defender),
            OPPONENT_START_DEFENDERS as usize,
        ),
    ]);
    let actual_counts = roots.iter().fold(HashMap::new(), |mut counts, root| {
        *counts.entry((root.2, root.1)).or_insert(0usize) += 1;
        counts
    });
    assert_eq!(actual_counts, expected_counts);

    for (index, first) in roots.iter().enumerate() {
        for second in &roots[index + 1..] {
            assert!(
                first.3.translation.distance(second.3.translation) >= 71.99,
                "authored seed bodies must begin separated"
            );
        }
    }
    for first in roots.iter().filter(|root| root.2 == SwarmId::PLAYER) {
        assert!(
            roots.iter().any(|other| {
                other.2 == opponent
                    && other.1 == first.1
                    && (first.3.translation.truncate() + other.3.translation.truncate())
                        .abs_diff_eq(Vec2::splat(12800.0), 0.01)
            }),
            "each player seed must have a same-type rotational counterpart"
        );
    }
    for (root, kind, swarm, transform, health, velocity, commitment) in roots {
        assert_abs_diff_eq!(transform.translation.z, GAMEPLAY_SPRITE_Z, epsilon = 0.01);
        let full_health = Health::default();
        assert_eq!(health.current, full_health.current);
        assert_eq!(health.max, full_health.max);
        assert_abs_diff_eq!(velocity.value.x, 0.0, epsilon = 1e-5);
        assert_abs_diff_eq!(velocity.value.y, 0.0, epsilon = 1e-5);
        assert_eq!(commitment, Commitment::Idle);

        let root_ref = world.entity(root);
        assert!(
            root_ref.get::<ChildOf>().is_none(),
            "nanobot gameplay roots must remain top-level"
        );
        assert!(
            root_ref.get::<Sprite>().is_none(),
            "nanobot gameplay roots must not remain a second visual authority"
        );
        assert!(
            root_ref.get::<Visibility>().is_some(),
            "presented roots must participate in hierarchy visibility"
        );
        let visual_children = root_ref
            .get::<Children>()
            .expect("presented nanobot must own a visual child")
            .iter()
            .filter(|child| world.get::<NanobotVisual>(*child).is_some())
            .collect::<Vec<_>>();
        assert_eq!(visual_children.len(), 1);

        let visual_entity = visual_children[0];
        {
            let visual = world.entity(visual_entity);
            assert_eq!(
                visual.get::<ChildOf>().map(ChildOf::parent),
                Some(root),
                "visual child must belong to its gameplay root"
            );
            assert_eq!(
                visual
                    .get::<Sprite>()
                    .expect("visual child needs a Sprite")
                    .image,
                sprites.handle(kind, swarm == opponent),
                "presentation child must preserve the authored type/faction image"
            );
            let visual_transform = visual
                .get::<Transform>()
                .expect("visual child needs a local Transform");
            assert!(visual_transform.translation.abs_diff_eq(Vec3::ZERO, 1e-5));
            assert!(visual_transform.rotation.angle_between(Quat::IDENTITY) <= 1e-5);
            assert!(visual_transform.scale.abs_diff_eq(Vec3::ONE, 1e-5));
        }

        world
            .get_mut::<Transform>(visual_entity)
            .expect("visual child needs an independently mutable Transform")
            .translation = Vec3::new(20.0, -10.0, 0.0);
        let root_translation = world
            .get::<Transform>(root)
            .expect("gameplay root must retain its Transform")
            .translation;
        assert_abs_diff_eq!(root_translation.x, transform.translation.x, epsilon = 0.01);
        assert_abs_diff_eq!(root_translation.y, transform.translation.y, epsilon = 0.01);
        assert_abs_diff_eq!(root_translation.z, transform.translation.z, epsilon = 0.01);
    }
}

#[test]
fn produced_nanobots_use_the_same_type_and_faction_visual_children() {
    let mut app = App::new();
    app.insert_resource(top_down_2d_rts_prototype_nano_swarm::intent::IntentGrid::new(8, 8));
    app.add_plugins(TaskPoolPlugin::default())
        .add_plugins(AssetPlugin::default())
        .init_asset::<Image>()
        .add_plugins(NanobotPresentationPlugin)
        .add_systems(Update, production_facility_work_system);
    let opponent = SwarmId(17);
    let expected = common::spawn_completed_facilities_for_all_nanobot_types(&mut app, opponent);

    app.update();

    let world = app.world_mut();
    let sprites = world.resource::<NanobotSprites>().clone();
    let produced = world
        .query_filtered::<(Entity, &NanobotType, &SwarmMember), With<Nanobot>>()
        .iter(world)
        .map(|(entity, kind, member)| (entity, *kind, member.0))
        .collect::<Vec<_>>();
    let mut actual = produced
        .iter()
        .map(|(_, kind, swarm)| (*kind, *swarm))
        .collect::<Vec<_>>();
    actual.sort_by_key(|(kind, swarm)| (*swarm, *kind as u8));
    let mut expected = expected;
    expected.sort_by_key(|(kind, swarm)| (*swarm, *kind as u8));
    assert_eq!(actual, expected);

    for (root, kind, swarm) in produced {
        assert!(world.get::<ChildOf>(root).is_none());
        assert!(world.get::<Sprite>(root).is_none());
        let children = world
            .get::<Children>(root)
            .expect("produced nanobot must own a visual child");
        let visual = children
            .iter()
            .find(|child| world.get::<NanobotVisual>(*child).is_some())
            .expect("produced nanobot must own its one presentation child");
        assert_eq!(
            world
                .get::<Sprite>(visual)
                .expect("presentation child needs a Sprite")
                .image,
            sprites.handle(kind, swarm == opponent)
        );
    }
}

#[test]
fn production_remains_asset_free_without_the_presentation_plugin() {
    let mut app = App::new();
    app.insert_resource(top_down_2d_rts_prototype_nano_swarm::intent::IntentGrid::new(8, 8));
    app.add_systems(Update, production_facility_work_system);
    let expected = common::spawn_completed_facilities_for_all_nanobot_types(&mut app, SwarmId(17));

    app.update();

    assert!(app.world().get_resource::<NanobotSprites>().is_none());
    let world = app.world_mut();
    let roots = world
        .query_filtered::<Entity, With<Nanobot>>()
        .iter(world)
        .collect::<Vec<_>>();
    assert_eq!(roots.len(), expected.len());
    for root in roots {
        assert!(world.get::<Sprite>(root).is_none());
        assert!(world.get::<Children>(root).is_none());
    }
}
