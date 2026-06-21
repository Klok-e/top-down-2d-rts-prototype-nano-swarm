//! Authored default scenario for `cargo run`.
//!
//! The default map is intentionally small on player pressure: it
//! starts the core economy moving without tutorial text, then leaves
//! the player to discover the rest of the prototype. The opponent is
//! a glossary "Opponent Swarm": prepainted intent and fixed ratios,
//! not active AI.

use bevy::{math::vec3, prelude::*};

use crate::{
    ai::AiStateComponent,
    building::{Minerals, ProcessingFacility},
    intent::{IntentGrid, IntentKind, PAINT_STRENGTH_CAP},
    nanobot::{
        Commitment, Health, Nanobot, NanobotBundle, NanobotSprites, NanobotType, OpponentSwarm,
        OwnerSwarm, ProductionFacility, ProductionRatio, Swarm, SwarmBundle, SwarmProduction,
        VelocityComponent,
    },
    resources::{ResourceDeposit, ResourceKind, Stockpile},
    GAMEPLAY_SPRITE_Z, ZONE_BLOCK_SIZE,
};

pub const PLAYER_CELL: IVec2 = IVec2::new(0, 0);
pub const PLAYER_DEPOSIT_CELL: IVec2 = IVec2::new(-2, 0);
pub const OPPONENT_CELL: IVec2 = IVec2::new(12, 0);
pub const OPPONENT_DEPOSIT_CELL: IVec2 = IVec2::new(10, 0);

pub const PLAYER_START_WORKERS: u32 = 4;
pub const PLAYER_START_HAULERS: u32 = 2;
pub const OPPONENT_START_WORKERS: u32 = 3;
pub const OPPONENT_START_HAULERS: u32 = 2;
pub const OPPONENT_START_DEFENDERS: u32 = 1;

pub const STARTING_DEPOSIT_AMOUNT: u32 = 1000;
pub const STARTING_STOCKPILE_CAPACITY: u32 = 1000;
pub const STARTING_WORK_RADIUS: f32 = 64.0;

pub fn cell_origin(cell: IVec2) -> Vec2 {
    Vec2::new(
        cell.x as f32 * ZONE_BLOCK_SIZE,
        cell.y as f32 * ZONE_BLOCK_SIZE,
    )
}

pub fn default_player_ratio() -> ProductionRatio {
    let mut ratio = ProductionRatio::new();
    ratio.set_target(NanobotType::Worker, 10);
    ratio.set_target(NanobotType::Hauler, 4);
    ratio.set_target(NanobotType::Defender, 1);
    ratio
}

pub fn default_opponent_ratio() -> ProductionRatio {
    let mut ratio = ProductionRatio::new();
    ratio.set_target(NanobotType::Worker, 8);
    ratio.set_target(NanobotType::Hauler, 4);
    ratio.set_target(NanobotType::Defender, 3);
    ratio
}

pub fn paint_default_player_intent(grid: &mut IntentGrid) {
    grid.paint(PLAYER_DEPOSIT_CELL, IntentKind::Gather, PAINT_STRENGTH_CAP);
}

pub fn paint_default_opponent_intent(grid: &mut IntentGrid) {
    grid.paint(
        OPPONENT_DEPOSIT_CELL,
        IntentKind::Gather,
        PAINT_STRENGTH_CAP,
    );
    grid.paint(OPPONENT_CELL, IntentKind::Defend, PAINT_STRENGTH_CAP);
}

pub fn spawn_default_player_scenario(
    commands: &mut Commands<'_, '_>,
    asset_server: &Res<'_, AssetServer>,
    grid: &mut IntentGrid,
) {
    paint_default_player_intent(grid);

    let player_pos = cell_origin(PLAYER_CELL);
    let deposit_pos = cell_origin(PLAYER_DEPOSIT_CELL);
    let sprites = NanobotSprites::load(asset_server);
    commands.insert_resource(sprites.clone());
    let deposit_texture = asset_server.load("resource_deposit.png");
    let facility_texture = asset_server.load("production_facility.png");

    let swarm = commands
        .spawn(SwarmBundle {
            swarm: Swarm {},
            transform: Transform::from_translation(player_pos.extend(0.0)),
            global_transform: GlobalTransform::default(),
            visibility: Visibility::default(),
        })
        .with_children(|p| {
            spawn_seed_nanobots(
                p,
                Vec2::ZERO,
                &sprites,
                false,
                &[
                    (NanobotType::Worker, PLAYER_START_WORKERS),
                    (NanobotType::Hauler, PLAYER_START_HAULERS),
                ],
            );
        })
        .id();

    spawn_deposit(commands, swarm, deposit_pos, &deposit_texture);
    spawn_production_facility(commands, swarm, player_pos, &facility_texture);
}

pub fn spawn_default_opponent_scenario(
    commands: &mut Commands<'_, '_>,
    asset_server: &Res<'_, AssetServer>,
    grid: &mut IntentGrid,
) {
    paint_default_opponent_intent(grid);

    let opponent_pos = cell_origin(OPPONENT_CELL);
    let deposit_pos = cell_origin(OPPONENT_DEPOSIT_CELL);
    let sprites = NanobotSprites::load(asset_server);
    let deposit_texture = asset_server.load("resource_deposit.png");
    let facility_texture = asset_server.load("production_facility.png");

    let opponent = commands
        .spawn((
            Swarm {},
            OpponentSwarm {},
            SwarmProduction::new(default_opponent_ratio()),
            Transform::from_translation(opponent_pos.extend(0.0)),
            GlobalTransform::default(),
            Visibility::default(),
        ))
        .with_children(|p| {
            spawn_seed_nanobots(
                p,
                Vec2::ZERO,
                &sprites,
                true,
                &[
                    (NanobotType::Worker, OPPONENT_START_WORKERS),
                    (NanobotType::Hauler, OPPONENT_START_HAULERS),
                    (NanobotType::Defender, OPPONENT_START_DEFENDERS),
                ],
            );
        })
        .id();

    spawn_deposit(commands, opponent, deposit_pos, &deposit_texture);
    spawn_production_facility(commands, opponent, opponent_pos, &facility_texture);
}

fn spawn_seed_nanobots(
    parent: &mut ChildSpawnerCommands<'_>,
    local_pos: Vec2,
    sprites: &NanobotSprites,
    is_opponent: bool,
    seeds: &[(NanobotType, u32)],
) {
    for (kind, count) in seeds {
        for _ in 0..*count {
            parent.spawn((
                NanobotBundle {
                    nanobot: Nanobot {},
                    nanobot_type: *kind,
                    velocity: VelocityComponent::default(),
                    ai_state: AiStateComponent::new(),
                    health: Health::default(),
                },
                Commitment::Idle,
                Sprite::from_image(sprites.handle(*kind, is_opponent)),
                Transform::from_translation(local_pos.extend(GAMEPLAY_SPRITE_Z)),
            ));
        }
    }
}

fn spawn_deposit(
    commands: &mut Commands<'_, '_>,
    owner: Entity,
    world_pos: Vec2,
    texture: &Handle<Image>,
) {
    commands.spawn((
        Minerals {},
        ResourceDeposit {
            kind: ResourceKind::Minerals,
            amount: STARTING_DEPOSIT_AMOUNT,
            capacity: STARTING_DEPOSIT_AMOUNT,
            radius: STARTING_WORK_RADIUS,
        },
        OwnerSwarm(owner),
        (
            Sprite::from_image(texture.clone()),
            Transform::from_translation(vec3(world_pos.x, world_pos.y, GAMEPLAY_SPRITE_Z))
                .with_scale(vec3(2., 2., 1.)),
        ),
    ));
}

fn spawn_production_facility(
    commands: &mut Commands<'_, '_>,
    owner: Entity,
    world_pos: Vec2,
    texture: &Handle<Image>,
) {
    commands.spawn((
        ProductionFacility::new(),
        ProcessingFacility {},
        OwnerSwarm(owner),
        Stockpile {
            kind: ResourceKind::Minerals,
            amount: 0,
            capacity: STARTING_STOCKPILE_CAPACITY,
            radius: STARTING_WORK_RADIUS,
        },
        (
            Sprite::from_image(texture.clone()),
            Transform::from_translation(vec3(world_pos.x, world_pos.y, GAMEPLAY_SPRITE_Z))
                .with_scale(vec3(3., 3., 1.)),
        ),
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_ratios_match_minimal_guided_sandbox() {
        let player = default_player_ratio();
        assert_eq!(player.target(NanobotType::Worker), 10);
        assert_eq!(player.target(NanobotType::Hauler), 4);
        assert_eq!(player.target(NanobotType::Defender), 1);

        let opponent = default_opponent_ratio();
        assert_eq!(opponent.target(NanobotType::Worker), 8);
        assert_eq!(opponent.target(NanobotType::Hauler), 4);
        assert_eq!(opponent.target(NanobotType::Defender), 3);
    }

    #[test]
    fn default_player_intent_prepaints_gather_only() {
        let mut grid = IntentGrid::new(32, 32);
        paint_default_player_intent(&mut grid);

        let deposit_cell = grid.cell(PLAYER_DEPOSIT_CELL).unwrap();
        assert!(deposit_cell.has(IntentKind::Gather));
        assert_eq!(
            deposit_cell.strength(IntentKind::Gather),
            PAINT_STRENGTH_CAP
        );
        assert!(!deposit_cell.has(IntentKind::Build));
        assert!(!deposit_cell.has(IntentKind::Defend));
        assert!(!deposit_cell.has(IntentKind::Corridor));
    }

    #[test]
    fn default_opponent_intent_prepaints_gather_and_defend() {
        let mut grid = IntentGrid::new(32, 32);
        paint_default_opponent_intent(&mut grid);

        let gather_cell = grid.cell(OPPONENT_DEPOSIT_CELL).unwrap();
        assert!(gather_cell.has(IntentKind::Gather));
        assert_eq!(gather_cell.strength(IntentKind::Gather), PAINT_STRENGTH_CAP);

        let defend_cell = grid.cell(OPPONENT_CELL).unwrap();
        assert!(defend_cell.has(IntentKind::Defend));
        assert_eq!(defend_cell.strength(IntentKind::Defend), PAINT_STRENGTH_CAP);
    }

    #[test]
    fn opponent_starts_far_from_player() {
        assert_eq!(OPPONENT_CELL.x - PLAYER_CELL.x, 12);
        assert!(
            cell_origin(OPPONENT_CELL).distance(cell_origin(PLAYER_CELL)) >= 10.0 * ZONE_BLOCK_SIZE
        );
    }
}
