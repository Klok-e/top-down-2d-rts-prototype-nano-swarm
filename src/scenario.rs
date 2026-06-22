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
        OwnerSwarm, ProductionFacility, ProductionRatio, Swarm, SwarmBundle, SwarmId, SwarmMember,
        SwarmProduction, VelocityComponent,
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
    // Named helper so the call site reads as "the default
    // player ratio" and future tuning can override the mix
    // without touching `nanobot::production`.
    ProductionRatio::default()
}

pub fn default_opponent_ratio() -> ProductionRatio {
    // Fixed authored mix (~53/27/20%), deliberately
    // distinct from the player default so the two swarms
    // diverge over time.
    let mut ratio = ProductionRatio::new();
    ratio.set_weight(NanobotType::Worker, 8);
    ratio.set_weight(NanobotType::Hauler, 4);
    ratio.set_weight(NanobotType::Defender, 3);
    ratio
}

pub fn paint_default_player_intent(grid: &mut IntentGrid) {
    // Stamp the player `SwarmId` on the prepainted cell so the
    // per-swarm intent filter from issue #20 keeps the player
    // gather cell visible only to player workers. Without the
    // owner stamp the cell would be unowned, and opponent
    // workers wandering into range would see it as a free
    // gather cell and try to mine it.
    grid.paint_owned(
        PLAYER_DEPOSIT_CELL,
        IntentKind::Gather,
        PAINT_STRENGTH_CAP,
        Some(SwarmId::PLAYER),
    );
}

/// Paint the default opponent intent at `OPPONENT_DEPOSIT_CELL` and
/// `OPPONENT_CELL`, stamping the cells with `owner` so they belong
/// to the opponent swarm rather than the player. The opponent id
/// is whatever the caller passes (the same id stamped on the
/// opponent Swarm entity); using `None` would mark the cells as
/// unowned and break the per-swarm separation.
pub fn paint_default_opponent_intent(grid: &mut IntentGrid, owner: SwarmId) {
    grid.paint_owned(
        OPPONENT_DEPOSIT_CELL,
        IntentKind::Gather,
        PAINT_STRENGTH_CAP,
        Some(owner),
    );
    grid.paint_owned(
        OPPONENT_CELL,
        IntentKind::Defend,
        PAINT_STRENGTH_CAP,
        Some(owner),
    );
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
            swarm_id: SwarmId::PLAYER,
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
                SwarmId::PLAYER,
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
    mut id_alloc: ResMut<crate::nanobot::OpponentSwarmIdAlloc>,
) {
    // The opponent id is allocated from the world's
    // `OpponentSwarmIdAlloc` resource so the swarm entity, the
    // prepainted intent, and the seed nanobots all share it.
    // Without a shared id the per-swarm intent filter would
    // route opponent paint to the wrong workers.
    let opponent_swarm_id = id_alloc.allocate();

    paint_default_opponent_intent(grid, opponent_swarm_id);

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
            opponent_swarm_id,
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
                opponent_swarm_id,
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
    swarm_id: SwarmId,
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
                    swarm_member: SwarmMember::new(swarm_id),
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
    fn default_ratios_match_60_30_10_player_and_authored_opponent() {
        // Player default must normalize to the issue #32
        // 60/30/10 mix. The stored weights are 6/3/1 so
        // the slider's step-5 tick lines up cleanly with
        // the percentage labels.
        let player = default_player_ratio();
        assert!((player.normalized_share(NanobotType::Worker) - 0.60).abs() < 1e-6);
        assert!((player.normalized_share(NanobotType::Hauler) - 0.30).abs() < 1e-6);
        assert!((player.normalized_share(NanobotType::Defender) - 0.10).abs() < 1e-6);
        // All three types must be set so the picker can
        // choose between them; an unset type reads as zero
        // share and never gets produced.
        assert!(player.weight(NanobotType::Worker) > 0);
        assert!(player.weight(NanobotType::Hauler) > 0);
        assert!(player.weight(NanobotType::Defender) > 0);

        // Opponent mix is a fixed authored ratio the
        // slider must not be able to mutate.
        let opponent = default_opponent_ratio();
        assert_eq!(opponent.weight(NanobotType::Worker), 8);
        assert_eq!(opponent.weight(NanobotType::Hauler), 4);
        assert_eq!(opponent.weight(NanobotType::Defender), 3);
        assert!(
            (opponent.normalized_share(NanobotType::Worker) - 0.60).abs() > 0.01,
            "opponent mix must remain distinct from the player 60/30/10 default"
        );
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
    fn default_player_intent_is_owned_by_player_swarm() {
        // The default player intent is the visible end of the
        // per-swarm ownership contract: an opponent worker
        // wandering into range must not see this cell as a
        // free gather cell. The owner stamp on the
        // prepainted cell is what enforces that.
        let mut grid = IntentGrid::new(32, 32);
        paint_default_player_intent(&mut grid);

        let cell = grid.cell(PLAYER_DEPOSIT_CELL).unwrap();
        assert_eq!(
            cell.owner(IntentKind::Gather),
            Some(SwarmId::PLAYER),
            "default player gather cell must be owned by SwarmId::PLAYER"
        );
        assert!(cell.visible_to(IntentKind::Gather, SwarmId::PLAYER));
        assert!(
            !cell.visible_to(IntentKind::Gather, SwarmId(1)),
            "opponent workers must NOT see the default player gather cell"
        );
    }

    #[test]
    fn default_opponent_intent_prepaints_gather_and_defend() {
        let mut grid = IntentGrid::new(32, 32);
        let opponent_id = SwarmId(7);
        paint_default_opponent_intent(&mut grid, opponent_id);

        let gather_cell = grid.cell(OPPONENT_DEPOSIT_CELL).unwrap();
        assert!(gather_cell.has(IntentKind::Gather));
        assert_eq!(gather_cell.strength(IntentKind::Gather), PAINT_STRENGTH_CAP);
        assert_eq!(gather_cell.owner(IntentKind::Gather), Some(opponent_id));

        let defend_cell = grid.cell(OPPONENT_CELL).unwrap();
        assert!(defend_cell.has(IntentKind::Defend));
        assert_eq!(defend_cell.strength(IntentKind::Defend), PAINT_STRENGTH_CAP);
        assert_eq!(defend_cell.owner(IntentKind::Defend), Some(opponent_id));
    }

    #[test]
    fn opponent_starts_far_from_player() {
        assert_eq!(OPPONENT_CELL.x - PLAYER_CELL.x, 12);
        assert!(
            cell_origin(OPPONENT_CELL).distance(cell_origin(PLAYER_CELL)) >= 10.0 * ZONE_BLOCK_SIZE
        );
    }
}
