//! Authored default scenario for `cargo run`.
//!
//! Rotationally symmetric rock terrain shelters two economies and separates
//! a direct central approach from longer mineral-bearing flanks.

use bevy::{math::vec3, prelude::*};

use crate::{
    GAMEPLAY_SPRITE_Z,
    ai::{AiStateComponent, get_world_from_zone},
    battle_experiment::{BattleExperimentConfig, ControllerId, LayoutId},
    building::{Minerals, ProcessingFacility},
    intent::{IntentGrid, IntentKind},
    nanobot::{
        Commitment, Health, Nanobot, NanobotBundle, NanobotType, OpponentSwarm, OwnerSwarm,
        ProductionFacility, StrategicController, Swarm, SwarmBundle, SwarmId, SwarmMember,
        VelocityComponent,
    },
    resources::{ResourceDeposit, ResourceKind},
};

mod layout;
mod map;
pub use layout::LayoutSetup;
pub use map::TerrainLayout;
pub(crate) use map::rock_surface_mesh;
pub use map::{default_rock_geometry, spawn_default_terrain};

pub const PLAYER_CELL: IVec2 = IVec2::new(0, 0);
pub const PLAYER_BUILD_FLANK_CELL: IVec2 = IVec2::new(0, 1);
pub const PLAYER_DEFEND_CELL: IVec2 = IVec2::new(1, 1);
pub const PLAYER_DEPOSIT_CELL: IVec2 = IVec2::new(-1, -1);
pub const OPPONENT_CELL: IVec2 = IVec2::new(24, 24);
pub const OPPONENT_BUILD_FLANK_CELL: IVec2 = IVec2::new(24, 23);
pub const OPPONENT_DEFEND_CELL: IVec2 = IVec2::new(23, 23);
pub const OPPONENT_DEPOSIT_CELL: IVec2 = IVec2::new(25, 25);
pub const NEUTRAL_DEPOSIT_CELLS: [IVec2; 4] = [
    IVec2::new(0, 7),
    IVec2::new(24, 17),
    IVec2::new(6, 18),
    IVec2::new(18, 6),
];

/// Keeps the seed facility visibly separate from seed nanobots while remaining
/// close enough for the initial Worker crew to maintain it.
pub const SEED_FACILITY_OFFSET: Vec2 = Vec2::new(160.0, 0.0);
pub const OPPONENT_FACILITY_OFFSET: Vec2 = Vec2::new(-160.0, 0.0);

pub const PLAYER_START_WORKERS: u32 = 4;
pub const PLAYER_START_HAULERS: u32 = 2;
pub const PLAYER_START_DEFENDERS: u32 = 3;
pub const OPPONENT_START_WORKERS: u32 = 4;
pub const OPPONENT_START_HAULERS: u32 = 2;
pub const OPPONENT_START_DEFENDERS: u32 = 3;

// Four starting workers extracting one unit per 60 Hz tick consume this in about five minutes.
pub const STARTING_DEPOSIT_AMOUNT: u32 = 72_000;
pub const STARTING_WORK_RADIUS: f32 = 64.0;

pub fn cell_origin(cell: IVec2) -> Vec2 {
    get_world_from_zone(cell)
}

pub fn paint_default_player_intent(grid: &mut IntentGrid) {
    for (cell, kind) in [
        (PLAYER_DEPOSIT_CELL, IntentKind::Gather),
        (PLAYER_CELL, IntentKind::Build),
        (PLAYER_BUILD_FLANK_CELL, IntentKind::Build),
        (PLAYER_DEFEND_CELL, IntentKind::Defend),
    ] {
        grid.paint(cell, kind, SwarmId::PLAYER);
    }
}

/// Paint the default opponent's independent Gather, Build, and Defend orders.
pub fn paint_default_opponent_intent(grid: &mut IntentGrid, owner: SwarmId) {
    for (cell, kind) in [
        (OPPONENT_DEPOSIT_CELL, IntentKind::Gather),
        (OPPONENT_CELL, IntentKind::Build),
        (OPPONENT_BUILD_FLANK_CELL, IntentKind::Build),
        (OPPONENT_DEFEND_CELL, IntentKind::Defend),
    ] {
        grid.paint(cell, kind, owner);
    }
}

pub fn spawn_default_player_scenario(
    commands: &mut Commands<'_, '_>,
    asset_server: &Res<'_, AssetServer>,
    grid: &mut IntentGrid,
) {
    spawn_player_scenario(
        commands,
        asset_server,
        grid,
        LayoutSetup::for_layout(LayoutId::Standard),
    );
}

fn paint_economy(
    grid: &mut IntentGrid,
    owner: SwarmId,
    home: IVec2,
    deposit: IVec2,
    direction: i32,
) {
    for (cell, kind) in [
        (deposit, IntentKind::Gather),
        (home, IntentKind::Build),
        (home + IVec2::Y * direction, IntentKind::Build),
        (home + IVec2::ONE * direction, IntentKind::Defend),
    ] {
        grid.paint(cell, kind, owner);
    }
}

fn spawn_player_scenario(
    commands: &mut Commands<'_, '_>,
    asset_server: &Res<'_, AssetServer>,
    grid: &mut IntentGrid,
    setup: LayoutSetup,
) {
    paint_economy(grid, SwarmId::PLAYER, setup.home, setup.home_deposit, 1);

    let player_pos = cell_origin(setup.home);
    let facility_pos = player_pos + SEED_FACILITY_OFFSET;
    let deposit_pos = cell_origin(setup.home_deposit);
    let facility_texture = asset_server.load("production_facility.png");

    let swarm = commands
        .spawn(SwarmBundle {
            swarm: Swarm {},
            swarm_id: SwarmId::PLAYER,
            transform: Transform::from_translation(player_pos.extend(0.0)),
            global_transform: GlobalTransform::default(),
            visibility: Visibility::default(),
        })
        .id();

    // Nanobots are top-level entities (issue #38 /
    // ADR-0004). The swarm's `Transform` is purely an
    // ownership / spawn-origin marker; nothing moves it
    // after spawn, and the bot systems read world
    // `Transform.translation` directly. Parented bots
    // would land at `local_destination + swarm_pos` --
    // the cell center + (256, 256) offset that drove
    // the original "top-right corner / bottom-left
    // structure" bug.
    spawn_seed_nanobots(
        commands,
        player_pos - Vec2::new(36.0, 0.0),
        -1.0,
        SwarmId::PLAYER,
        &[
            (NanobotType::Worker, PLAYER_START_WORKERS),
            (NanobotType::Hauler, PLAYER_START_HAULERS),
            (NanobotType::Defender, PLAYER_START_DEFENDERS),
        ],
    );

    spawn_deposit(commands, Some(swarm), deposit_pos);
    for cell in setup.neutral_deposits {
        spawn_deposit(commands, None, cell_origin(cell));
    }
    spawn_production_facility(commands, swarm, facility_pos, &facility_texture);
}

/// Preserve the opponent-side deposit when the map has no opponent swarm.
pub fn spawn_sandbox_resources(commands: &mut Commands<'_, '_>) {
    spawn_deposit(commands, None, cell_origin(OPPONENT_DEPOSIT_CELL));
}

pub fn spawn_default_opponent_scenario(
    commands: &mut Commands<'_, '_>,
    asset_server: &Res<'_, AssetServer>,
    grid: &mut IntentGrid,
    id_alloc: ResMut<crate::nanobot::OpponentSwarmIdAlloc>,
) {
    spawn_opponent_scenario(
        commands,
        asset_server,
        grid,
        id_alloc,
        LayoutSetup::for_layout(LayoutId::Standard),
    );
}

fn spawn_opponent_scenario(
    commands: &mut Commands<'_, '_>,
    asset_server: &Res<'_, AssetServer>,
    grid: &mut IntentGrid,
    mut id_alloc: ResMut<crate::nanobot::OpponentSwarmIdAlloc>,
    setup: LayoutSetup,
) {
    // The opponent id is allocated from the world's
    // `OpponentSwarmIdAlloc` resource so the swarm entity, the
    // prepainted intent, and the seed nanobots all share it.
    // Without a shared id the per-swarm intent filter would
    // route opponent paint to the wrong workers.
    let opponent_swarm_id = id_alloc.allocate();

    let home = LayoutSetup::opposite(setup.home);
    let deposit = LayoutSetup::opposite(setup.home_deposit);
    paint_economy(grid, opponent_swarm_id, home, deposit, -1);

    let opponent_pos = cell_origin(home);
    let facility_pos = opponent_pos + OPPONENT_FACILITY_OFFSET;
    let deposit_pos = cell_origin(deposit);
    let facility_texture = asset_server.load("production_facility.png");

    let opponent = commands
        .spawn((
            Swarm {},
            OpponentSwarm {},
            StrategicController::adaptive(opponent_swarm_id),
            opponent_swarm_id,
            Transform::from_translation(opponent_pos.extend(0.0)),
            GlobalTransform::default(),
            Visibility::default(),
        ))
        .id();

    // Opponent seed nanobots are top-level (issue #38 /
    // ADR-0004); see the player seed comment for the
    // rationale. The same half-cell offset broke the
    // opponent economy in `cargo run` while the test
    // helper, which spawns top-level, hid the bug.
    spawn_seed_nanobots(
        commands,
        opponent_pos + Vec2::new(36.0, 0.0),
        1.0,
        opponent_swarm_id,
        &[
            (NanobotType::Worker, OPPONENT_START_WORKERS),
            (NanobotType::Hauler, OPPONENT_START_HAULERS),
            (NanobotType::Defender, OPPONENT_START_DEFENDERS),
        ],
    );

    spawn_deposit(commands, Some(opponent), deposit_pos);
    spawn_production_facility(commands, opponent, facility_pos, &facility_texture);
}

/// Spawn the seed nanobots described by `seeds` as top-level
/// entities in a body-clear formation beside `world_pos`. Each bot carries a `Transform` whose
/// `translation` is the world position the rest of the simulation
/// reads (issue #38 / ADR-0004). The owning swarm is recorded on
/// each bot via `SwarmMember(swarm_id)`; the swarm's own
/// `Transform` is kept as a spawn-origin / ownership marker and is
/// not moved after this call.
fn spawn_seed_nanobots(
    commands: &mut Commands<'_, '_>,
    world_pos: Vec2,
    outward: f32,
    swarm_id: SwarmId,
    seeds: &[(NanobotType, u32)],
) {
    let mut index = 0;
    for (kind, count) in seeds {
        for _ in 0..*count {
            let position = world_pos
                + Vec2::new(
                    outward * (index % 3) as f32 * 72.0,
                    -outward * ((index / 3) as f32 * 72.0 - 72.0),
                );
            index += 1;
            commands.spawn((
                NanobotBundle {
                    nanobot: Nanobot {},
                    nanobot_type: *kind,
                    velocity: VelocityComponent::default(),
                    ai_state: AiStateComponent::new(),
                    health: Health::default(),
                    swarm_member: SwarmMember::new(swarm_id),
                },
                Commitment::Idle,
                Transform::from_translation(position.extend(GAMEPLAY_SPRITE_Z)),
            ));
        }
    }
}

fn spawn_deposit(commands: &mut Commands<'_, '_>, owner: Option<Entity>, world_pos: Vec2) {
    let mut entity = commands.spawn((
        Minerals {},
        ResourceDeposit {
            kind: ResourceKind::Minerals,
            amount: STARTING_DEPOSIT_AMOUNT,
            capacity: STARTING_DEPOSIT_AMOUNT,
            radius: STARTING_WORK_RADIUS,
        },
        (
            Transform::from_translation(vec3(world_pos.x, world_pos.y, GAMEPLAY_SPRITE_Z))
                .with_scale(vec3(2., 2., 1.)),
        ),
    ));
    if let Some(owner) = owner {
        entity.insert(OwnerSwarm(owner));
    }
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
        crate::structure_sprites::StructureVisual::completed(
            crate::nanobot::PlannedKind::ProductionFacility,
        ),
        OwnerSwarm(owner),
        (
            Sprite {
                image: texture.clone(),
                custom_size: Some(Vec2::splat(crate::navigation::STRUCTURE_SPRITE_SIZE)),
                ..default()
            },
            crate::navigation::align_structure(
                Transform::from_translation(vec3(world_pos.x, world_pos.y, GAMEPLAY_SPRITE_Z))
                    .with_scale(vec3(3., 3., 1.)),
            ),
        ),
    ));
}

/// Authored setup and session policy stay together in the scenario definition.
pub struct ScenarioDefinition {
    pub rules: crate::session::SessionRules,
    opponent: bool,
    automatic_player: bool,
}

impl crate::scenario_selection::Scenario {
    pub fn definition(self) -> ScenarioDefinition {
        use crate::session::{OutcomeMode, SessionRules};
        match self {
            Self::Standard => ScenarioDefinition {
                rules: SessionRules::default(),
                opponent: true,
                automatic_player: false,
            },
            Self::Sandbox => ScenarioDefinition {
                rules: SessionRules {
                    scenario_name: "sandbox",
                    outcomes: OutcomeMode::Disabled,
                    ..default()
                },
                opponent: false,
                automatic_player: false,
            },
            Self::AiBattle => ScenarioDefinition {
                rules: SessionRules {
                    scenario_name: "ai_battle",
                    player_swarm: None,
                    outcomes: OutcomeMode::SwarmRelative,
                    record_statistics: true,
                    accelerate_headless: true,
                },
                opponent: true,
                automatic_player: true,
            },
        }
    }
}

pub struct ScenarioPlugin;
impl Plugin for ScenarioPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<crate::scenario_selection::ScenarioSelection>()
            .init_resource::<crate::session::SessionRules>()
            .init_resource::<crate::session::SimulationSeed>()
            .init_resource::<BattleExperimentConfig>()
            .init_resource::<crate::gameplay_pacing::GameplayPacing>()
            .add_systems(PreStartup, configure_session)
            .add_systems(
                PostStartup,
                configure_controllers.run_if(not(resource_exists::<
                    crate::session_lifecycle::SessionGeneration,
                >)),
            );
    }
}

fn configure_session(
    selection: Res<crate::scenario_selection::ScenarioSelection>,
    experiment: Res<BattleExperimentConfig>,
    mut rules: ResMut<crate::session::SessionRules>,
    mut pacing: ResMut<crate::gameplay_pacing::GameplayPacing>,
) {
    *rules = session_rules(selection.current, &experiment);
    *pacing = gameplay_pacing(selection.current, &experiment);
}

pub(crate) fn gameplay_pacing(
    scenario: crate::scenario_selection::Scenario,
    experiment: &BattleExperimentConfig,
) -> crate::gameplay_pacing::GameplayPacing {
    if scenario == crate::scenario_selection::Scenario::AiBattle {
        experiment.pacing.into()
    } else {
        crate::gameplay_pacing::GameplayPacing::from(crate::battle_experiment::PacingId::Deliberate)
    }
}

pub(crate) fn session_rules(
    scenario: crate::scenario_selection::Scenario,
    experiment: &BattleExperimentConfig,
) -> crate::session::SessionRules {
    let mut rules = scenario.definition().rules;
    if experiment.realtime {
        rules.accelerate_headless = false;
    }
    rules
}

pub fn spawn_selected_scenario(
    scenario: crate::scenario_selection::Scenario,
    commands: &mut Commands<'_, '_>,
    assets: &Res<'_, AssetServer>,
    grid: &mut IntentGrid,
    ids: ResMut<crate::nanobot::OpponentSwarmIdAlloc>,
    layout: LayoutId,
) {
    let layout = if scenario == crate::scenario_selection::Scenario::AiBattle {
        layout
    } else {
        LayoutId::Standard
    };
    let setup = LayoutSetup::for_layout(layout);
    map::spawn_terrain(commands, layout);
    spawn_player_scenario(commands, assets, grid, setup);
    if scenario.definition().opponent {
        spawn_opponent_scenario(commands, assets, grid, ids, setup);
    } else {
        spawn_sandbox_resources(commands);
    }
}

pub(crate) fn configure_controllers(
    mut commands: Commands,
    selection: Res<crate::scenario_selection::ScenarioSelection>,
    experiment: Res<BattleExperimentConfig>,
    swarms: Query<(Entity, &SwarmId), With<Swarm>>,
) {
    if !selection.current.definition().automatic_player {
        return;
    }
    for (entity, id) in &swarms {
        let side = usize::from(!id.is_player());
        let controller_id = experiment.controllers[side ^ usize::from(experiment.swap_sides)];
        let setup = LayoutSetup::for_layout(experiment.layout);
        let (home, target, direction) = if side == 0 {
            (setup.home, LayoutSetup::opposite(setup.home), 1)
        } else {
            (LayoutSetup::opposite(setup.home), setup.home, -1)
        };
        let controller = match controller_id {
            ControllerId::Timed => StrategicController::timed(
                *id,
                home + IVec2::ONE * direction,
                target,
                5 * crate::SIMULATION_HZ as u32,
                6 * crate::SIMULATION_HZ as u32,
            ),
            ControllerId::Adaptive => StrategicController::adaptive(*id),
        };
        commands.entity(entity).insert(controller);
    }
}

#[cfg(test)]
mod tests {
    use approx::assert_abs_diff_eq;

    use super::*;

    #[test]
    fn default_player_intent_prepaints_gather_build_and_defend() {
        let mut grid = IntentGrid::new(64, 64);
        paint_default_player_intent(&mut grid);

        let deposit_cell = grid.cell(PLAYER_DEPOSIT_CELL).unwrap();
        assert!(deposit_cell.has(IntentKind::Gather));
        assert!(!deposit_cell.has(IntentKind::Corridor));

        let facility_cell = grid.cell(PLAYER_CELL).unwrap();
        assert!(facility_cell.has(IntentKind::Build));
        assert!(!facility_cell.has(IntentKind::Corridor));

        assert!(
            grid.cell(PLAYER_BUILD_FLANK_CELL)
                .unwrap()
                .has(IntentKind::Build)
        );
        assert!(
            !grid
                .cell(PLAYER_BUILD_FLANK_CELL)
                .unwrap()
                .has(IntentKind::Defend)
        );
        assert!(
            !grid.cell(IVec2::new(1, 0)).unwrap().has(IntentKind::Defend),
            "removed player Defender flank cell must stay unpainted"
        );

        // Defend is prepainted on its own cell, distinct from
        // the facility's Build cell (see PLAYER_DEFEND_CELL).
        let defend_cell = grid.cell(PLAYER_DEFEND_CELL).unwrap();
        assert!(defend_cell.has(IntentKind::Defend));
        assert!(!defend_cell.has(IntentKind::Corridor));
    }

    #[test]
    fn default_player_intent_is_owned_by_player_swarm() {
        // The default player intent is the visible end of the
        // per-swarm ownership contract: an opponent worker
        // wandering into range must not see this cell as a
        // free gather cell. The owner stamp on the
        // prepainted cell is what enforces that.
        let mut grid = IntentGrid::new(64, 64);
        paint_default_player_intent(&mut grid);

        let cell = grid.cell(PLAYER_DEPOSIT_CELL).unwrap();
        assert_eq!(
            cell.owners(IntentKind::Gather).collect::<Vec<_>>(),
            vec![SwarmId::PLAYER],
            "default player gather cell must be owned by SwarmId::PLAYER"
        );
        assert!(cell.has_owned(IntentKind::Gather, SwarmId::PLAYER));
        assert!(
            !cell.has_owned(IntentKind::Gather, SwarmId(1)),
            "opponent workers must NOT see the default player gather cell"
        );

        let facility_cell = grid.cell(PLAYER_CELL).unwrap();
        assert_eq!(
            facility_cell.owners(IntentKind::Build).collect::<Vec<_>>(),
            vec![SwarmId::PLAYER]
        );
        assert!(facility_cell.has_owned(IntentKind::Build, SwarmId::PLAYER));
        assert!(
            !facility_cell.has_owned(IntentKind::Build, SwarmId(1)),
            "opponent workers must NOT see default player Build intent"
        );

        let defend_cell = grid.cell(PLAYER_DEFEND_CELL).unwrap();
        assert_eq!(
            defend_cell.owners(IntentKind::Defend).collect::<Vec<_>>(),
            vec![SwarmId::PLAYER]
        );
        assert!(defend_cell.has_owned(IntentKind::Defend, SwarmId::PLAYER));
        assert!(
            !defend_cell.has_owned(IntentKind::Defend, SwarmId(1)),
            "opponent workers must NOT see default player Defend intent"
        );
    }

    #[test]
    fn default_opponent_intent_prepaints_gather_build_and_defend() {
        let mut grid = IntentGrid::new(64, 64);
        let opponent_id = SwarmId(7);
        paint_default_opponent_intent(&mut grid, opponent_id);

        let gather_cell = grid.cell(OPPONENT_DEPOSIT_CELL).unwrap();
        assert!(gather_cell.has(IntentKind::Gather));
        assert_eq!(
            gather_cell.owners(IntentKind::Gather).collect::<Vec<_>>(),
            vec![opponent_id]
        );

        let facility_cell = grid.cell(OPPONENT_CELL).unwrap();
        assert!(facility_cell.has(IntentKind::Build));
        assert_eq!(
            facility_cell.owners(IntentKind::Build).collect::<Vec<_>>(),
            vec![opponent_id]
        );

        let defend_cell = grid.cell(OPPONENT_DEFEND_CELL).unwrap();
        assert!(defend_cell.has(IntentKind::Defend));
        assert_eq!(
            defend_cell.owners(IntentKind::Defend).collect::<Vec<_>>(),
            vec![opponent_id]
        );
        assert_eq!(
            grid.cell(OPPONENT_BUILD_FLANK_CELL)
                .unwrap()
                .owners(IntentKind::Build)
                .collect::<Vec<_>>(),
            vec![opponent_id]
        );
        assert!(
            !grid
                .cell(OPPONENT_BUILD_FLANK_CELL)
                .unwrap()
                .has(IntentKind::Defend)
        );
        assert!(
            !grid
                .cell(IVec2::new(2, -1))
                .unwrap()
                .has(IntentKind::Defend),
            "removed opponent Defender flank cell must stay unpainted"
        );
    }

    #[test]
    fn default_scenario_assets_keep_the_authored_starting_layout() {
        let player_origin = cell_origin(PLAYER_CELL);
        let opponent_origin = cell_origin(OPPONENT_CELL);
        let player_facility = player_origin + SEED_FACILITY_OFFSET;
        let opponent_facility = opponent_origin + OPPONENT_FACILITY_OFFSET;

        assert_eq!(crate::nanobot::world_to_cell(player_facility), PLAYER_CELL);
        assert_eq!(
            crate::nanobot::world_to_cell(opponent_facility),
            OPPONENT_CELL
        );
        assert_abs_diff_eq!(player_facility.x - player_origin.x, 160.0, epsilon = 0.01);
        assert_abs_diff_eq!(player_facility.y, player_origin.y, epsilon = 0.01);
        assert_abs_diff_eq!(
            opponent_facility.x - opponent_origin.x,
            -160.0,
            epsilon = 0.01
        );
        assert_abs_diff_eq!(opponent_facility.y, opponent_origin.y, epsilon = 0.01);

        let player_deposit = cell_origin(PLAYER_DEPOSIT_CELL);
        let opponent_deposit = cell_origin(OPPONENT_DEPOSIT_CELL);
        assert_abs_diff_eq!(
            player_deposit.x - player_origin.x,
            -crate::ZONE_BLOCK_SIZE,
            epsilon = 0.01
        );
        assert_abs_diff_eq!(
            player_deposit.y - player_origin.y,
            -crate::ZONE_BLOCK_SIZE,
            epsilon = 0.01
        );
        assert_abs_diff_eq!(
            opponent_deposit.x - opponent_origin.x,
            crate::ZONE_BLOCK_SIZE,
            epsilon = 0.01
        );
        assert_abs_diff_eq!(
            opponent_deposit.y - opponent_origin.y,
            crate::ZONE_BLOCK_SIZE,
            epsilon = 0.01
        );
    }
}
