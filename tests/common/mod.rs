//! Shared test helpers for the deterministic simulation test seams
//! introduced by issue #17.
//!
//! Every behavior/playtest integration file in this crate starts from a tiny
//! Bevy `App` assembled from the same handful of resources and systems. Before
//! this module landed that setup was duplicated across eleven files, and the
//! per-file `build_app` functions drifted apart in subtle ways (different grid
//! sizes, missing `ResourceLedger`, different `GameSettings`).
//!
//! The helpers here are the canonical seams for future behaviour
//! tests:
//!
//! - [`sim_app`] and the `sim_app_with_*` builders start the same
//!   minimal Bevy `App`: `bevy::time::TimePlugin`, a default
//!   `GameSettings`, an `IntentGrid`, a `SoftWorkSlots` resource, a
//!   `ResourceLedger`, the four shared movement systems chained in
//!   the order the rest of the plugin graph expects, plus any
//!   simulation plugin the test needs.
//! - [`cell_world_center`] turns an `IVec2` cell coordinate into
//!   the world position the auto-creation systems actually use, so
//!   tests pin placement the same way the systems do.
//! - The `spawn_*` helpers produce the entities each behaviour test
//!   cares about. They always use the project's default
//!   `ResourceKind::Minerals` and a `Transform` at the world
//!   position, so tests can compare ECS state without repeating
//!   spawn boilerplate.
//!
//! The module is `pub` and consumed by nested integration tests via
//! `#[path = "../common/mod.rs"] mod common;`. Each test file imports the
//! helpers it actually needs; nothing here is `pub` outside the test crate, so
//! the production crate stays free of test-only types.

#![allow(dead_code)]

use bevy::{math::Vec2, prelude::*};
use top_down_2d_rts_prototype_nano_swarm::{
    game_settings::GameSettings,
    intent::IntentGrid,
    nanobot::{
        bot_debug_circle_system, move_velocity_system, separation_system, velocity_system,
        BuildPlugin, Charge, ChargePlugin, Charger, CollapsePlugin, Commitment, DefendPlugin,
        GatherPlugin, HaulPlugin, Health, MaintenancePlugin, Nanobot, NanobotBundle, NanobotType,
        OwnerSwarm, PlannedStructure, PlannedStructurePlugin, ProductionFacility, ProductionPlugin,
        SoftWorkSlots, Structure, StructureKind, Swarm, SwarmId, SwarmMember, VelocityComponent,
    },
    resources::{ResourceDeposit, ResourceKind, ResourceLedger, Stockpile},
};

/// Default `GameSettings` for behaviour tests. The values match the
/// starter map size and the project's per-tick movement speed so
/// distance / tick math is obvious to a reader of the test.
pub fn default_game_settings() -> GameSettings {
    GameSettings {
        width: 1000.0,
        height: 1000.0,
        bot_speed: 5.0,
        debug_draw_circles: false,
    }
}

/// Default `IntentGrid` size for behaviour tests. The starter
/// map is 8 cells per side, which covers a 4096x4096 world
/// region -- enough for every test in the suite without forcing
/// the test to spell out its own grid size. Tests that need a
/// different shape can replace the resource after construction.
pub const DEFAULT_GRID_WIDTH: i32 = 8;
pub const DEFAULT_GRID_HEIGHT: i32 = 8;

/// Register the four shared movement systems on `app` in the chain
/// order the rest of the plugin graph expects: separation
/// (steering apart), velocity (steering intent), move_velocity
/// (integrate), and the debug-circle overlay (off in tests). Every
/// simulation plugin in the crate chains after `move_velocity_system`,
/// so this must be present in every test that uses any of them.
fn register_movement_systems(app: &mut App) {
    app.add_systems(
        Update,
        (
            separation_system,
            velocity_system,
            move_velocity_system,
            bot_debug_circle_system,
        )
            .chain(),
    );
}

/// Build the smallest Bevy `App` that can host the simulation
/// plugins: a time resource, a default `GameSettings`, an
/// `IntentGrid`, and the two swarm-wide resources the autonomy
/// scoring and resource economy code read. No movement systems,
/// no plugins -- use this for tests that drive the pure scoring
/// helpers directly.
pub fn minimal_app() -> App {
    let mut app = App::new();
    app.add_plugins(bevy::time::TimePlugin);
    app.insert_resource(IntentGrid::new(DEFAULT_GRID_WIDTH, DEFAULT_GRID_HEIGHT));
    app.insert_resource(default_game_settings());
    app.init_resource::<SoftWorkSlots>();
    app.init_resource::<ResourceLedger>();
    app
}

/// Build the smallest Bevy `App` that can host the simulation
/// plugins **and** drive a nanobot through movement: everything
/// [`minimal_app`] sets up plus the four shared movement systems.
/// No simulation plugin is registered; tests opt in via the
/// `sim_app_with_*` builders below.
pub fn sim_app() -> App {
    let mut app = minimal_app();
    register_movement_systems(&mut app);
    app
}

/// `sim_app` + the gather systems. The gather plugin is the
/// foundation for the Build, Haul, and Charge plugins because they
/// all chain after `move_velocity_system` and consume deposits and
/// stockpiles the gather plugin can populate.
pub fn sim_app_with_gather() -> App {
    let mut app = sim_app();
    app.add_plugins(GatherPlugin);
    app
}

/// `sim_app` + gather + haul. Use for any test that needs a hauler
/// to move resources between a deposit and a stockpile (or
/// charger).
pub fn sim_app_with_gather_haul() -> App {
    let mut app = sim_app_with_gather();
    app.add_plugins(HaulPlugin);
    app
}

/// `sim_app` + gather + build. The gather plugin is registered for
/// completeness so the shared nanobot chain does not panic if a
/// future test in this file spawns a deposit. Build is the focus.
pub fn sim_app_with_build() -> App {
    let mut app = sim_app_with_gather();
    app.add_plugins(BuildPlugin);
    app
}

/// `sim_app` + planned structure plugin. The planned-structure
/// foundation runs the auto-creation / claim / work lifecycle;
/// tests that exercise planned structures use this builder so
/// they can paint Build intent and observe a planned structure
/// emerge without the gather/haul/build chain running around
/// the same cells.
pub fn sim_app_with_planned() -> App {
    let mut app = sim_app();
    app.add_plugins(PlannedStructurePlugin);
    app
}

/// `sim_app` + build + maintenance. The maintenance plugin chains
/// after the build plugin so its work system can reset a
/// structure's buffer counter before the degradation system runs.
pub fn sim_app_with_maintenance() -> App {
    let mut app = sim_app_with_build();
    app.add_plugins(MaintenancePlugin);
    app
}

/// `sim_app` + defend. The defend plugin brings its own assignment,
/// hold, and home-cell systems; tests that exercise it can use
/// this builder as-is.
pub fn sim_app_with_defend() -> App {
    let mut app = sim_app();
    app.add_plugins(DefendPlugin);
    app
}

/// `sim_app` + gather + haul + defend + charge. The full
/// defend/charge loop: defenders hold a cell, drain, rotate to a
/// working charger, and return. Haul is registered so a hauler
/// can deliver to a charger (the logistics support half of the
/// charge contract).
pub fn sim_app_with_charge() -> App {
    let mut app = sim_app_with_gather_haul();
    app.add_plugins(DefendPlugin);
    app.add_plugins(ChargePlugin);
    app
}

/// `sim_app` + production. The production plugin is independent of
/// the per-role plugins; tests that exercise only the production
/// chain use this builder.
pub fn sim_app_with_production() -> App {
    let mut app = sim_app();
    app.add_plugins(ProductionPlugin);
    app
}

/// `sim_app` + production + collapse. The collapse detection
/// system runs after the production work system, so both plugins
/// must be registered together for the order to match the
/// production app.
pub fn sim_app_with_collapse() -> App {
    let mut app = sim_app_with_production();
    app.add_plugins(CollapsePlugin);
    app
}

/// World position of the centre of `cell` in the project's
/// coordinate system. Thin wrapper around the canonical
/// `ai::get_world_from_zone` so tests share the same formula
/// as the auto-creation systems; pinning "the BuildSite lives
/// at the cell's world center" stays in lock-step with
/// gameplay code.
pub fn cell_world_center(cell: IVec2) -> Vec2 {
    top_down_2d_rts_prototype_nano_swarm::ai::get_world_from_zone(cell)
}

/// Spawn an empty [`Swarm`] at `world_pos`. The marker carries no
/// nanobots; use [`spawn_swarm_with_nanobots`] when the test needs
/// a populated population, or call this builder and then add
/// children by hand.
///
/// The swarm is stamped with [`SwarmId::PLAYER`] so the
/// production chain can route new nanobots to the right owner.
pub fn spawn_swarm_at(app: &mut App, world_pos: Vec2) -> Entity {
    app.world_mut()
        .spawn((
            Swarm {},
            SwarmId::PLAYER,
            Transform::from_translation(world_pos.extend(0.0)),
        ))
        .id()
}

/// Spawn a [`Swarm`] at `world_pos` with `counts` of each
/// [`NanobotType`] as children. Each child is a fresh
/// [`NanobotBundle`] with `Commitment::Idle` and a `Transform` at
/// the swarm's position, so it is immediately eligible for the
/// autonomy scoring path. Use for production and collapse tests
/// that need a known starting population.
pub fn spawn_swarm_with_nanobots(
    app: &mut App,
    world_pos: Vec2,
    counts: &[(NanobotType, u32)],
) -> Entity {
    let swarm = spawn_swarm_at(app, world_pos);
    {
        let world = app.world_mut();
        let mut entity = world.entity_mut(swarm);
        entity.with_children(|p| {
            for (kind, n) in counts {
                for _ in 0..*n {
                    p.spawn((
                        NanobotBundle {
                            nanobot: Nanobot {},
                            nanobot_type: *kind,
                            velocity: VelocityComponent::default(),
                            ai_state: Default::default(),
                            health: Health::default(),
                            swarm_member: SwarmMember::new(SwarmId::PLAYER),
                        },
                        Commitment::Idle,
                        Transform::from_translation(world_pos.extend(0.0)),
                    ));
                }
            }
        });
    }
    swarm
}

/// Spawn a Worker nanobot at `world_pos` with an idle commitment,
/// zero velocity, and a fresh `Health` component. Matches the
/// gather and build worker fixtures in the existing tests;
/// future tests that need additional components (Charge, etc.)
/// should call this helper and then `entity_mut(...).insert(...)`.
///
/// The worker is tagged as a member of the player swarm so the
/// per-swarm intent filter passes for any player-painted cell.
pub fn spawn_worker_at(app: &mut App, world_pos: Vec2) -> Entity {
    app.world_mut()
        .spawn((
            Nanobot {},
            NanobotType::Worker,
            Commitment::Idle,
            VelocityComponent::default(),
            Health::default(),
            SwarmMember::new(SwarmId::PLAYER),
            Transform::from_translation(world_pos.extend(0.0)),
        ))
        .id()
}

/// Spawn a Defender nanobot at `world_pos` with full [`Health`]
/// and full [`Charge`]. The charge test fixtures start the
/// defender at full charge; tests that need a partially drained
/// defender mutate the component after the spawn.
pub fn spawn_defender_at(app: &mut App, world_pos: Vec2) -> Entity {
    app.world_mut()
        .spawn((
            Nanobot {},
            NanobotType::Defender,
            Commitment::Idle,
            VelocityComponent::default(),
            Health::default(),
            Charge::default(),
            SwarmMember::new(SwarmId::PLAYER),
            Transform::from_translation(world_pos.extend(0.0)),
        ))
        .id()
}

/// Spawn a Hauler nanobot at `world_pos` with an idle commitment,
/// zero velocity, and a fresh `Health` component. The hauler is
/// the only type that runs through the haul systems, so the
/// helper does not pre-seed a load.
pub fn spawn_hauler_at(app: &mut App, world_pos: Vec2) -> Entity {
    app.world_mut()
        .spawn((
            Nanobot {},
            NanobotType::Hauler,
            Commitment::Idle,
            VelocityComponent::default(),
            Health::default(),
            SwarmMember::new(SwarmId::PLAYER),
            Transform::from_translation(world_pos.extend(0.0)),
        ))
        .id()
}

/// Spawn a [`ResourceDeposit`] of `ResourceKind::Minerals` at
/// `world_pos` with `amount` units, a `capacity` that matches
/// `amount`, and the standard gather-test `radius` of `32.0`.
/// Tests that need a different `radius` (e.g. the issue #22
/// overlap suite, where the deposit's circle is the eligibility
/// geometry) call [`spawn_deposit_with_radius`] instead.
pub fn spawn_deposit(app: &mut App, world_pos: Vec2, amount: u32) -> Entity {
    spawn_deposit_with_radius(app, world_pos, amount, 32.0)
}

/// Spawn a [`ResourceDeposit`] of `ResourceKind::Minerals` at
/// `world_pos` with an explicit `radius`. The `capacity` matches
/// `amount`, mirroring [`spawn_deposit`]; tests that need a
/// different cap override the field after the spawn.
pub fn spawn_deposit_with_radius(
    app: &mut App,
    world_pos: Vec2,
    amount: u32,
    radius: f32,
) -> Entity {
    app.world_mut()
        .spawn((
            ResourceDeposit {
                kind: ResourceKind::Minerals,
                amount,
                capacity: amount.max(1000),
                radius,
            },
            Transform::from_translation(world_pos.extend(0.0)),
        ))
        .id()
}

/// Spawn a [`Stockpile`] of `ResourceKind::Minerals` at
/// `world_pos` with the given `amount` and `capacity`. The
/// `radius` matches the default used by the gameplay code, so
/// hauler delivery distances are realistic.
pub fn spawn_stockpile(app: &mut App, world_pos: Vec2, amount: u32, capacity: u32) -> Entity {
    app.world_mut()
        .spawn((
            Stockpile {
                kind: ResourceKind::Minerals,
                amount,
                capacity,
                radius: 32.0,
            },
            Transform::from_translation(world_pos.extend(0.0)),
        ))
        .id()
}

/// Spawn a [`Charger`] in `cell` with the given `amount` of
/// minerals. The charger lives at the cell's world centre, so
/// tests that assert "the charger is in the cell" can compare the
/// transform without doing the cell-to-world math themselves.
pub fn spawn_charger_at(app: &mut App, cell: IVec2, amount: u32) -> Entity {
    let mut c = Charger::new(cell);
    c.amount = amount;
    app.world_mut()
        .spawn((
            c,
            Transform::from_translation(cell_world_center(cell).extend(0.0)),
        ))
        .id()
}

/// Spawn an idle [`ProductionFacility`] at `world_pos`. The
/// facility has no `OwnerSwarm`, so it falls back to the global
/// `ProductionRatio` resource and the first swarm in the world
/// when the work system spawns a new nanobot -- the same fallback
/// the pre-multi-swarm production tests rely on.
pub fn spawn_idle_facility_at(app: &mut App, world_pos: Vec2) -> Entity {
    app.world_mut()
        .spawn((
            ProductionFacility::new(),
            Transform::from_translation(world_pos.extend(0.0)),
        ))
        .id()
}

/// Spawn a fresh [`Structure`] of kind `StructureKind::Basic` at
/// `world_pos`. The structure starts at full health (used as the
/// baseline for degradation and maintenance tests).
pub fn spawn_structure_at(app: &mut App, world_pos: Vec2) -> Entity {
    app.world_mut()
        .spawn((
            Structure::new(StructureKind::Basic),
            Transform::from_translation(world_pos.extend(0.0)),
        ))
        .id()
}

/// Spawn a fresh [`PlannedStructure`] in `cell` for the Source
/// Stockpile demo kind. The test-only entry point mirrors what
/// the auto-creation system does so behaviour tests that
/// already have paint can skip the spawn tick. The planned
/// visual is included so tests that pin the visual flip can
/// compare colors against the same starting state the
/// production code produces.
pub fn spawn_planned_structure_at_cell(app: &mut App, cell: IVec2) -> Entity {
    use top_down_2d_rts_prototype_nano_swarm::nanobot::{
        planned_visual_color, PlannedKind, PLANNED_STRUCTURE_FOOTPRINT,
    };
    let center = cell_world_center(cell);
    app.world_mut()
        .spawn((
            PlannedStructure::new(PlannedKind::SourceStockpile, cell),
            Sprite {
                color: planned_visual_color(),
                custom_size: Some(Vec2::splat(PLANNED_STRUCTURE_FOOTPRINT)),
                ..default()
            },
            Transform::from_translation(center.extend(0.0)),
        ))
        .id()
}

/// Spawn an [`OpponentSwarm`] at `world_pos` with the given
/// `ratio` and `counts` of each nanobot type as children. The
/// opponent marker and per-swarm `SwarmProduction` are wired in
/// so the production systems use the opponent's fixed ratio
/// rather than the global `ProductionRatio` resource.
///
/// Thin wrapper over the production
/// [`top_down_2d_rts_prototype_nano_swarm::nanobot::spawn_opponent_swarm`]
/// helper, which already allocates a fresh non-player
/// [`SwarmId`] and stamps every child with
/// `SwarmMember(swarm_id)` so the per-swarm intent filter
/// routes opponent paint to opponent workers only.
pub fn spawn_opponent_swarm_with_nanobots(
    app: &mut App,
    world_pos: Vec2,
    ratio: top_down_2d_rts_prototype_nano_swarm::nanobot::ProductionRatio,
    counts: &[(NanobotType, u32)],
) -> Entity {
    use top_down_2d_rts_prototype_nano_swarm::nanobot::{spawn_opponent_swarm, SeedNanobots};
    let seeds: Vec<SeedNanobots> = counts
        .iter()
        .map(|(kind, n)| SeedNanobots::new(*kind, *n))
        .collect();
    spawn_opponent_swarm(app.world_mut(), world_pos, ratio, &[], &seeds)
}

/// Spawn an idle [`ProductionFacility`] owned by `owner` at
/// `pos`. The owner marker is what tells the production
/// systems to use the owner's ratio and children for the
/// deficit math; without it, the facility falls back to the
/// global `ProductionRatio` resource.
pub fn spawn_facility_at(app: &mut App, owner: Entity, pos: Vec2) -> Entity {
    app.world_mut()
        .spawn((
            ProductionFacility::new(),
            OwnerSwarm(owner),
            Transform::from_translation(pos.extend(0.0)),
        ))
        .id()
}

/// Spawn a busy [`ProductionFacility`] at `world_pos` with the
/// given production `target` and `progress = 1`. A busy facility
/// is the "production is currently working" half of the collapse
/// contract, so production and collapse tests can use this helper
/// to skip the pick/work setup and assert collapse-related
/// behaviour directly.
pub fn spawn_busy_facility_at(app: &mut App, world_pos: Vec2, target: NanobotType) -> Entity {
    let mut f = ProductionFacility::new();
    f.current_target = Some(target);
    f.progress = 1;
    app.world_mut()
        .spawn((f, Transform::from_translation(world_pos.extend(0.0))))
        .id()
}
