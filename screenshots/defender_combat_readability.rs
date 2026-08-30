//! Offscreen visual evidence for readable Defender combat and swarm-wide recharge.

use std::collections::HashMap;

use bevy::{math::Vec2, prelude::*};
use top_down_2d_rts_prototype_nano_swarm::{
    GAMEPLAY_SPRITE_Z, ZONE_BLOCK_SIZE,
    intent::{IntentGrid, IntentKind},
    nanobot::{
        Charge, Charger, ChargerAssignment, Commitment, DefenderResponse, Health, Nanobot,
        NanobotType, OpponentSwarm, OwnerSwarm, Structure, StructureKind, Swarm, SwarmId,
        SwarmMember, VelocityComponent,
    },
};

use crate::harness::{TestContext, TestFlow};

const CENTER_CELL: IVec2 = IVec2::new(0, 5);
const OPPONENT_CHARGER_CELL: IVec2 = IVec2::new(1, 5);

#[derive(Component)]
struct FeelDefender;

fn cell_center(cell: IVec2) -> Vec2 {
    Vec2::new(
        (cell.x as f32 + 0.5) * ZONE_BLOCK_SIZE,
        (cell.y as f32 + 0.5) * ZONE_BLOCK_SIZE,
    )
}

fn swarm_entity(world: &mut World, id: SwarmId) -> Entity {
    world
        .query_filtered::<(Entity, &SwarmId), With<Swarm>>()
        .iter(world)
        .find_map(|(entity, swarm)| (*swarm == id).then_some(entity))
        .expect("default scenario must contain both swarm entities")
}

fn focus_camera(world: &mut World) {
    let target = cell_center(CENTER_CELL);
    let mut camera = world.query_filtered::<(&mut Transform, &mut Projection), With<Camera2d>>();
    for (mut transform, mut projection) in camera.iter_mut(world) {
        transform.translation.x = target.x;
        transform.translation.y = target.y;
        if let Projection::Orthographic(orthographic) = &mut *projection {
            orthographic.scale = 0.85;
        }
    }
}

fn spawn_defender(world: &mut World, position: Vec2, swarm: SwarmId, charge: f32) {
    let entity = world
        .spawn((
            FeelDefender,
            Nanobot {},
            NanobotType::Defender,
            Commitment::Idle,
            VelocityComponent::default(),
            Health::default(),
            SwarmMember::new(swarm),
            Transform::from_translation(position.extend(GAMEPLAY_SPRITE_Z)),
        ))
        .id();
    world.entity_mut(entity).insert(Charge {
        current: charge,
        max: 1.0,
    });
}

fn setup_scene(world: &mut World) {
    focus_camera(world);
    let player_swarm = swarm_entity(world, SwarmId::PLAYER);
    let opponent_swarm_id = world
        .query_filtered::<&SwarmId, (With<Swarm>, With<OpponentSwarm>)>()
        .iter(world)
        .next()
        .copied()
        .expect("default scenario must contain an opponent swarm");
    let opponent_swarm = swarm_entity(world, opponent_swarm_id);
    let center = cell_center(CENTER_CELL);
    world.resource_mut::<IntentGrid>().paint_owned(
        CENTER_CELL,
        IntentKind::Defend,
        Some(SwarmId::PLAYER),
    );
    world
        .resource_mut::<IntentGrid>()
        .contest_defend(CENTER_CELL, opponent_swarm_id);
    world.resource_mut::<IntentGrid>().paint_owned(
        OPPONENT_CHARGER_CELL,
        IntentKind::Defend,
        Some(opponent_swarm_id),
    );

    for (index, offset) in [
        Vec2::new(-54.0, -42.0),
        Vec2::new(-54.0, 42.0),
        Vec2::new(-36.0, 0.0),
        Vec2::new(-72.0, 0.0),
        Vec2::new(-90.0, 0.0),
    ]
    .into_iter()
    .enumerate()
    {
        spawn_defender(
            world,
            center + offset,
            SwarmId::PLAYER,
            if index < 2 { 0.5 } else { 1.0 },
        );
    }
    for (index, offset) in [
        Vec2::new(54.0, -42.0),
        Vec2::new(54.0, 42.0),
        Vec2::new(36.0, 0.0),
    ]
    .into_iter()
    .enumerate()
    {
        spawn_defender(
            world,
            center + offset,
            opponent_swarm_id,
            if index == 0 { 0.5 } else { 1.0 },
        );
    }

    let mut player_charger = Charger::new(CENTER_CELL);
    player_charger.amount = 60;
    world.spawn((
        player_charger,
        OwnerSwarm(player_swarm),
        Structure::new(StructureKind::Basic),
        Sprite {
            color: Color::srgb(0.20, 0.55, 0.95),
            custom_size: Some(Vec2::splat(54.0)),
            ..default()
        },
        Transform::from_translation((center + Vec2::new(-132.0, 116.0)).extend(GAMEPLAY_SPRITE_Z)),
    ));
    let mut opponent_charger = Charger::new(OPPONENT_CHARGER_CELL);
    opponent_charger.amount = 60;
    let opponent_charger_center = cell_center(OPPONENT_CHARGER_CELL);
    world.spawn((
        opponent_charger,
        OwnerSwarm(opponent_swarm),
        Structure::new(StructureKind::Basic),
        Sprite {
            color: Color::srgb(0.90, 0.25, 0.30),
            custom_size: Some(Vec2::splat(54.0)),
            ..default()
        },
        Transform::from_translation(
            (opponent_charger_center + Vec2::new(-132.0, 116.0)).extend(GAMEPLAY_SPRITE_Z),
        ),
    ));
}

fn assert_front_state(world: &mut World, require_rotation: bool) {
    let mut player_responders = 0;
    let mut opponent_responders = 0;
    let mut charger_loads = HashMap::<Entity, usize>::new();
    let mut living_by_swarm = HashMap::<SwarmId, u32>::new();
    let mut rotating_by_swarm = HashMap::<SwarmId, u32>::new();
    let mut low_charge = 0;
    for (response, assignment, member, health, charge, transform) in world
        .query_filtered::<(
            Option<&DefenderResponse>,
            Option<&ChargerAssignment>,
            &SwarmMember,
            &Health,
            &Charge,
            &Transform,
        ), With<FeelDefender>>()
        .iter(world)
    {
        assert!(transform.translation.is_finite());
        if health.current > 0 {
            *living_by_swarm.entry(member.0).or_default() += 1;
        }
        if charge.needs_rotation() {
            low_charge += 1;
        }
        if response.is_some() {
            if member.0 == SwarmId::PLAYER {
                player_responders += 1;
            }
            if member.0 != SwarmId::PLAYER {
                opponent_responders += 1;
            }
        }
        if let Some(assignment) = assignment {
            *charger_loads.entry(assignment.charger).or_default() += 1;
            *rotating_by_swarm.entry(member.0).or_default() += 1;
        }
        assert!(health.current <= health.max);
    }
    let total_assignments = charger_loads.values().sum::<usize>();
    for (charger, load) in charger_loads {
        assert!(
            load <= 3,
            "charger {charger:?} exceeded its three-Defender service limit: {load}"
        );
    }
    for (swarm, load) in rotating_by_swarm {
        let living = living_by_swarm.get(&swarm).copied().unwrap_or_default();
        let within_cap = match living {
            0 => load == 0,
            1 => load <= 1,
            _ => load.saturating_mul(2) <= living,
        };
        assert!(
            within_cap,
            "swarm {swarm:?} exceeded its rotation allowance: {load} of {living} living"
        );
    }
    if require_rotation {
        assert!(
            total_assignments > 0,
            "focused Defender scene must show active Charge rotation; low={low_charge}, player_responders={player_responders}, opponent_responders={opponent_responders}"
        );
    }
    let total_living = living_by_swarm.values().sum::<u32>();
    assert!(
        total_living > 0,
        "focused Defender scene must remain populated"
    );
}

pub fn defender_combat_readability(ctx: &mut TestContext) -> TestFlow {
    focus_camera(ctx.world);
    if ctx.frame == 0 {
        setup_scene(ctx.world);
        return TestFlow::Continue;
    }
    if ctx.frame < 30 {
        return TestFlow::Continue;
    }
    if ctx.frame == 30 {
        let grid = ctx.world.resource::<IntentGrid>();
        assert!(grid.defend_contest(CENTER_CELL).is_some());
        assert_front_state(ctx.world, true);
        return TestFlow::Screenshot("defender_combat_early".to_string());
    }
    if ctx.frame < 125 {
        return TestFlow::Continue;
    }
    if ctx.frame == 125 {
        focus_camera(ctx.world);
        assert_front_state(ctx.world, true);
        return TestFlow::Screenshot("defender_combat_rotation".to_string());
    }
    if ctx.frame < 300 {
        return TestFlow::Continue;
    }
    if ctx.frame == 300 {
        focus_camera(ctx.world);
        assert_front_state(ctx.world, false);
        return TestFlow::Screenshot("defender_combat_late".to_string());
    }
    TestFlow::Exit
}
