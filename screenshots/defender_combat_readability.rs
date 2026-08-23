//! Offscreen visual evidence for readable Defender combat and local recharge.

use std::collections::HashMap;

use bevy::{math::Vec2, prelude::*};
use top_down_2d_rts_prototype_nano_swarm::{
    GAMEPLAY_SPRITE_Z, ZONE_BLOCK_SIZE,
    intent::{IntentGrid, IntentKind},
    nanobot::{
        Charge, Charger, ChargerAssignment, Commitment, DefendHold, Health, Nanobot,
        NanobotSprites, NanobotType, OpponentSwarm, OwnerSwarm, Swarm, SwarmId, SwarmMember,
        VelocityComponent,
    },
};

use crate::harness::{TestContext, TestFlow};

const CENTER_CELL: IVec2 = IVec2::new(0, 5);

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

fn spawn_defender(
    world: &mut World,
    position: Vec2,
    swarm: SwarmId,
    hold: DefendHold,
    charge: f32,
    sprite: Handle<Image>,
) {
    world.spawn((
        FeelDefender,
        Nanobot {},
        NanobotType::Defender,
        Commitment::Idle,
        VelocityComponent::default(),
        Health::default(),
        Charge {
            current: charge,
            max: 1.0,
        },
        SwarmMember::new(swarm),
        hold,
        Transform::from_translation(position.extend(GAMEPLAY_SPRITE_Z)),
        Sprite::from_image(sprite),
    ));
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

    let player_sprite = world
        .resource::<NanobotSprites>()
        .handle(NanobotType::Defender, false);
    let opponent_sprite = world
        .resource::<NanobotSprites>()
        .handle(NanobotType::Defender, true);
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
            DefendHold { cell: CENTER_CELL },
            if index < 2 { 0.5 } else { 1.0 },
            player_sprite.clone(),
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
            DefendHold { cell: CENTER_CELL },
            if index == 0 { 0.5 } else { 1.0 },
            opponent_sprite.clone(),
        );
    }

    let mut player_charger = Charger::new(CENTER_CELL);
    player_charger.amount = 60;
    world.spawn((
        player_charger,
        OwnerSwarm(player_swarm),
        Sprite {
            color: Color::srgb(0.20, 0.55, 0.95),
            custom_size: Some(Vec2::splat(54.0)),
            ..default()
        },
        Transform::from_translation((center + Vec2::new(-132.0, 116.0)).extend(GAMEPLAY_SPRITE_Z)),
    ));
    let mut opponent_charger = Charger::new(CENTER_CELL);
    opponent_charger.amount = 60;
    world.spawn((
        opponent_charger,
        OwnerSwarm(opponent_swarm),
        Sprite {
            color: Color::srgb(0.90, 0.25, 0.30),
            custom_size: Some(Vec2::splat(54.0)),
            ..default()
        },
        Transform::from_translation((center + Vec2::new(132.0, 116.0)).extend(GAMEPLAY_SPRITE_Z)),
    ));
}

fn assert_front_state(world: &mut World, require_holders: bool) {
    let mut player_holders = 0;
    let mut opponent_holders = 0;
    let mut charger_loads = HashMap::<Entity, usize>::new();
    let mut cohort_sizes = HashMap::<(SwarmId, IVec2), usize>::new();
    let mut cohort_loads = HashMap::<(SwarmId, IVec2), usize>::new();
    for (hold, assignment, member, health, transform) in world
        .query_filtered::<(
            Option<&DefendHold>,
            Option<&ChargerAssignment>,
            &SwarmMember,
            &Health,
            &Transform,
        ), With<FeelDefender>>()
        .iter(world)
    {
        assert!(transform.translation.is_finite());
        let source_cell = assignment
            .map(|assignment| assignment.source_cell)
            .or_else(|| hold.map(|hold| hold.cell));
        if let Some(source_cell) = source_cell {
            *cohort_sizes.entry((member.0, source_cell)).or_default() += 1;
        }
        if let Some(hold) = hold {
            if member.0 == SwarmId::PLAYER && hold.cell == CENTER_CELL {
                player_holders += 1;
            }
            if member.0 != SwarmId::PLAYER && hold.cell == CENTER_CELL {
                opponent_holders += 1;
            }
        }
        if let Some(assignment) = assignment {
            *charger_loads.entry(assignment.charger).or_default() += 1;
            *cohort_loads
                .entry((member.0, assignment.source_cell))
                .or_default() += 1;
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
    for ((swarm, cell), load) in cohort_loads {
        let cohort_size = cohort_sizes
            .get(&(swarm, cell))
            .copied()
            .expect("assigned Defender must belong to a source cohort");
        let limit = (cohort_size / 2).max(1);
        assert!(
            load <= limit,
            "cohort {swarm:?} at {cell:?} exceeded its rotation allowance: {load} > {limit}"
        );
    }
    if require_holders {
        assert!(
            player_holders > 0,
            "player front fully evacuated for recharge"
        );
        assert!(
            opponent_holders > 0,
            "opponent front fully evacuated for recharge"
        );
    }
    assert!(
        player_holders + opponent_holders + total_assignments > 0,
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
