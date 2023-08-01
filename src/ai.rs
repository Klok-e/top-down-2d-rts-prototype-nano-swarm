use bevy::{
    math::vec2,
    prelude::{
        Commands, Component, Entity, IVec2, Parent, Plugin, Query, Transform, Vec2, With, Without,
    },
};

use rand::seq::IteratorRandom;

use crate::{
    nanobot::{DirectMovementComponent, Nanobot},
    zones::{get_zone_pos_from_world, ZoneComponent},
    ZONE_BLOCK_SIZE,
};

#[derive(Debug, Component, Default)]
pub struct AiStateComponent {
    pub action_requests: Vec<AiActionRequest>,
    pub current_action: AiActionKind,
}

impl AiStateComponent {
    pub fn new() -> Self {
        Self::default()
    }
}

#[derive(Debug)]
pub struct AiActionRequest {
    kind: AiActionKind,
    priority: u8,
}

#[derive(Debug)]
pub enum AiActionKind {
    Idle(Idle),
    Move(Move),
    Gather(Gather),
    Build(Build),
}
impl Default for AiActionKind {
    fn default() -> Self {
        Self::Idle(Idle)
    }
}

#[derive(Debug, Component, Default)]
pub struct Idle;

#[derive(Debug, Component)]
pub struct Move {
    pub destination: Vec2,
}

#[derive(Debug, Component)]
pub struct Gather;

#[derive(Debug, Component)]
pub struct Build;

pub fn idle_behaviour_system(
    mut states: Query<(Entity, &mut AiStateComponent)>,
    bot_positions: Query<(&Transform, &Parent), With<Nanobot>>,
    zones: Query<(&ZoneComponent,)>,
) {
    let mut rng = rand::thread_rng();

    for (ent, mut action_state) in &mut states {
        action_state.action_requests.push(AiActionRequest {
            kind: AiActionKind::Idle(Idle),
            priority: 0,
        });

        let (curr_trans, curr_par) = bot_positions.get(ent).unwrap();
        let (curr_zone,) = zones.get(curr_par.get()).unwrap();

        let zone_pos = get_zone_pos_from_world(curr_trans.translation.truncate());
        if !curr_zone.zone_points.is_empty() && !curr_zone.zone_points.contains(&zone_pos) {
            let rand_zone_point = curr_zone.zone_points.iter().choose(&mut rng).unwrap();
            action_state.action_requests.push(AiActionRequest {
                kind: AiActionKind::Move(Move {
                    destination: get_world_from_zone(*rand_zone_point),
                }),
                priority: 1,
            });
        }
    }
}

pub fn gather_behaviour_system(_nanobots: Query<&mut AiStateComponent>) {}

pub fn build_behaviour_system(_nanobots: Query<&mut AiStateComponent>) {}

pub fn move_action_system(
    mut commands: Commands,
    nanobots: Query<(Entity, &Move), Without<DirectMovementComponent>>,
) {
    for (ent, bot) in &nanobots {
        commands.entity(ent).insert(DirectMovementComponent {
            xy: bot.destination,
        });
    }
}

pub fn gather_action_system(_nanobots: Query<&mut AiStateComponent>) {}

pub fn build_action_system(_nanobots: Query<&mut AiStateComponent>) {}

pub fn decision_system(
    mut command: Commands,
    mut nanobots: Query<(Entity, &mut AiStateComponent)>,
) {
    for (ent, mut nanobot) in &mut nanobots {
        let requests = std::mem::take(&mut nanobot.action_requests);
        let Some(action) = requests.into_iter().max_by_key(|x| x.priority) else {
            continue;
        };
        match &nanobot.current_action {
            AiActionKind::Idle(_) => {
                command.entity(ent).remove::<Idle>();
            }
            AiActionKind::Move(_) => {
                command.entity(ent).remove::<Move>();
            }
            AiActionKind::Gather(_) => {
                command.entity(ent).remove::<Gather>();
            }
            AiActionKind::Build(_) => {
                command.entity(ent).remove::<Build>();
            }
        }
        match action.kind {
            AiActionKind::Idle(c) => {
                command.entity(ent).insert(c);
            }
            AiActionKind::Move(c) => {
                command.entity(ent).insert(c);
            }
            AiActionKind::Gather(c) => {
                command.entity(ent).insert(c);
            }
            AiActionKind::Build(c) => {
                command.entity(ent).insert(c);
            }
        }
    }
}

pub const BLOCK_SIZE: u32 = 2;

pub fn get_block_from_world(world_pos: Vec2) -> IVec2 {
    vec2(
        (world_pos.x / BLOCK_SIZE as f32).floor(),
        (world_pos.y / BLOCK_SIZE as f32).floor(),
    )
    .as_ivec2()
}

pub fn get_zone_from_block(block_pos: IVec2) -> IVec2 {
    vec2(
        (block_pos.x * BLOCK_SIZE as i32) as f32 / ZONE_BLOCK_SIZE,
        (block_pos.y * BLOCK_SIZE as i32) as f32 / ZONE_BLOCK_SIZE,
    )
    .as_ivec2()
}

pub fn get_world_from_zone(zone_pos: IVec2) -> Vec2 {
    vec2(
        (zone_pos.x) as f32 * ZONE_BLOCK_SIZE,
        (zone_pos.y) as f32 * ZONE_BLOCK_SIZE,
    ) + ZONE_BLOCK_SIZE / 2.
}

pub struct AiPlugin;

impl Plugin for AiPlugin {
    fn build(&self, app: &mut bevy::prelude::App) {
        app.add_system(idle_behaviour_system)
            .add_system(gather_behaviour_system)
            .add_system(build_behaviour_system)
            .add_system(move_action_system)
            .add_system(gather_action_system)
            .add_system(build_action_system)
            .add_system(decision_system);
    }
}
