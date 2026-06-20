pub mod ai;
pub mod building;
pub mod fly_camera;
pub mod game_settings;
pub mod intent;
pub mod materials;
pub mod nanobot;
pub mod resources;
pub mod ui;
pub mod zones;

use ai::AiPlugin;
use anyhow::Result;
use bevy::{
    math::vec3,
    prelude::*,
    render::storage::ShaderStorageBuffer,
    sprite_render::{Material2dPlugin, MeshMaterial2d},
};
use building::{Minerals, ProcessingFacility};
use fly_camera::{Camera2dFlyPlugin, CameraZoom2d, FlyCamera2d};
use game_settings::GameSettings;
use intent::{IntentGrid, IntentKind, PAINT_STRENGTH_CAP};
use materials::BackgroundMaterial;
use nanobot::{
    spawn_opponent_swarm, CollapsePlugin, NanobotBundle, NanobotPlugin, NanobotType,
    PrepaintedIntent, ProductionPlugin, ProductionRatio, SeedNanobots, Swarm, SwarmBundle,
};
use resources::{ResourceDeposit, ResourceKind, ResourceLedger, Stockpile};
use ui::NanoswarmUiSetupPlugin;
use zones::{ZoneMaterial, ZoneMaterialHandleComponent, ZonesPlugin};

pub fn build_app() -> App {
    let mut app = App::new();
    app.insert_resource(IntentGrid::new(MAP_WIDTH as i32, MAP_HEIGHT as i32))
        .init_resource::<ResourceLedger>()
        .init_resource::<ProductionRatio>()
        .init_resource::<nanobot::SoftWorkSlots>()
        .add_plugins(DefaultPlugins)
        .add_plugins(Material2dPlugin::<BackgroundMaterial>::default())
        // must be before NanobotPlugin because otherwise it receives events with despawned entities
        .add_plugins(NanoswarmUiSetupPlugin)
        // must be before NanobotPlugin because otherwise it receives events with despawned entities
        .add_plugins(ZonesPlugin::default())
        .add_plugins(NanobotPlugin::default())
        // GatherPlugin must come after NanobotPlugin: the gather
        // chain orders itself behind `move_velocity_system`, which
        // only exists once NanobotPlugin is registered.
        .add_plugins(nanobot::GatherPlugin)
        // HaulPlugin chains after `move_velocity_system`, which is
        // registered by NanobotPlugin above. The arrival signal the
        // hauler systems wait for is the same one the gather chain
        // uses, so they only need to run after the movement step.
        .add_plugins(nanobot::HaulPlugin)
        // BuildPlugin chains after `move_velocity_system` so the
        // arrive system sees the pruned DirectMovementComponent.
        .add_plugins(nanobot::BuildPlugin)
        // MaintenancePlugin chains after BuildPlugin so the
        // maintenance work system can reset a structure's
        // buffer counter before the degradation system runs.
        .add_plugins(nanobot::MaintenancePlugin)
        // ProductionPlugin chains after `move_velocity_system`
        // for the same reason; auto-creation runs last in its
        // own chain so it sees the post-pick / post-work state
        // before deciding to spawn a new facility.
        .add_plugins(ProductionPlugin)
        // CollapsePlugin must run after the production work
        // system so the "is this facility currently busy?"
        // check sees the post-work state, not the pre-work
        // state of the same tick.
        .add_plugins(CollapsePlugin)
        // DefendPlugin chains after `move_velocity_system` so
        // the arrive system sees the pruned
        // DirectMovementComponent, the same signal the rest of
        // the per-role systems use.
        .add_plugins(nanobot::DefendPlugin)
        // ChargePlugin chains after `move_velocity_system`
        // and after DefendPlugin so the defend hold is
        // established before the rotation system releases it.
        // The internal order (drain -> health loss ->
        // auto-creation -> rotation -> arrive -> work) keeps
        // the charge loop self-consistent per tick.
        .add_plugins(nanobot::ChargePlugin)
        .add_plugins(AiPlugin)
        .add_plugins(Camera2dFlyPlugin)
        .add_systems(Startup, setup_things_startup.pipe(error_handler))
        .add_systems(Startup, setup_opponent_swarm_startup);
    app
}

pub fn run() {
    build_app().run();
}

pub const MAP_WIDTH: u32 = 1000;
pub const MAP_HEIGHT: u32 = 1000;
pub const ZONE_BLOCK_SIZE: f32 = 512.;

fn setup_things_startup(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut bg_mats: ResMut<Assets<BackgroundMaterial>>,
    mut zone_mats: ResMut<Assets<ZoneMaterial>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut buffers: ResMut<Assets<ShaderStorageBuffer>>,
) -> Result<()> {
    let handle = zone_mats.add(ZoneMaterial::new(MAP_WIDTH, MAP_HEIGHT, &mut buffers));
    commands
        .spawn(Camera2d)
        .insert(FlyCamera2d::default())
        .insert(CameraZoom2d {
            zoom_speed: 10.,
            zoom_min_max: (1., 100.),
            zoom: 1.,
        })
        .insert(ZoneMaterialHandleComponent {
            handle: handle.clone(),
        });

    commands.insert_resource(GameSettings::from_file_ron("config/game_settings.ron")?);

    spawn_initial_swarm(&mut commands, &asset_server);

    // minerals (deposit + visual marker)
    let minerals_texture = asset_server.load("minerals.png");
    commands.spawn((
        Minerals {},
        ResourceDeposit {
            kind: ResourceKind::Minerals,
            amount: 1000,
            capacity: 1000,
            radius: 64.0,
        },
        (
            Sprite::from_image(minerals_texture.clone()),
            Transform::from_translation(vec3(-800., 0., 1.)).with_scale(vec3(2., 2., 1.)),
        ),
    ));

    // processing (stockpile + visual marker)
    let processing_texture = asset_server.load("mineral processing.png");
    commands.spawn((
        ProcessingFacility {},
        Stockpile {
            kind: ResourceKind::Minerals,
            amount: 0,
            capacity: 1000,
            radius: 64.0,
        },
        (
            Sprite::from_image(processing_texture.clone()),
            Transform::from_translation(vec3(-300., 0., 1.)).with_scale(vec3(3., 3., 1.)),
        ),
    ));

    // background
    commands.spawn((
        Mesh2d(meshes.add(Mesh::from(Rectangle::default()))),
        MeshMaterial2d(bg_mats.add(BackgroundMaterial {})),
        Transform::default().with_scale(
            Vec2::new(
                MAP_WIDTH as f32 * ZONE_BLOCK_SIZE,
                MAP_HEIGHT as f32 * ZONE_BLOCK_SIZE,
            )
            .extend(-100.),
        ),
    ));

    // zones
    commands.spawn((
        Mesh2d(meshes.add(Mesh::from(Rectangle::default()))),
        MeshMaterial2d(handle),
        Transform::default().with_scale(
            Vec2::new(
                MAP_WIDTH as f32 * ZONE_BLOCK_SIZE,
                MAP_HEIGHT as f32 * ZONE_BLOCK_SIZE,
            )
            .extend(-101.),
        ),
    ));
    Ok(())
}

fn spawn_initial_swarm(commands: &mut Commands<'_, '_>, asset_server: &Res<'_, AssetServer>) {
    let texture = asset_server.load("circle.png");
    commands
        .spawn(SwarmBundle {
            swarm: Swarm {},
            transform: Transform::default(),
            global_transform: GlobalTransform::default(),
        })
        .with_children(|p| {
            for _ in 0..4 {
                p.spawn((
                    NanobotBundle::default(),
                    Sprite::from_image(texture.clone()),
                    Transform::from_translation(vec3(0., 0., 1.)),
                ));
            }
            for _ in 0..100 {
                p.spawn((
                    NanobotBundle::default(),
                    Sprite::from_image(texture.clone()),
                    Transform::from_translation(vec3(100., 0., 1.)),
                ));
            }
        });
}

fn error_handler(In(result): In<Result<()>>) {
    if let Err(err) = result {
        println!("encountered an error {:?}", err);
    }
}

/// Materialise the first opponent swarm on the far side of
/// the map with a fixed Hauler-heavy production ratio and a
/// small prepainted Gather/Defend territory, so a player
/// running the game out of the box sees the opponent working
/// through the same systems as the player swarm. Kept as a
/// separate system because it needs `&mut World` access to
/// paint the intent grid, while the main startup system
/// stays `Commands`-based.
fn setup_opponent_swarm_startup(world: &mut World) {
    let opponent_world_pos = vec3(40.0 * ZONE_BLOCK_SIZE, 0.0, 0.0);
    let mut opponent_ratio = ProductionRatio::new();
    // Hauler-heavy so logistics keep up; a Defender
    // presence so the prepainted base holds. No active AI.
    opponent_ratio.set_target(NanobotType::Hauler, 6);
    opponent_ratio.set_target(NanobotType::Defender, 4);
    opponent_ratio.set_target(NanobotType::Worker, 4);

    spawn_opponent_swarm(
        world,
        opponent_world_pos.truncate(),
        opponent_ratio,
        &[
            PrepaintedIntent::new(IVec2::new(40, 0), IntentKind::Gather, PAINT_STRENGTH_CAP),
            PrepaintedIntent::new(IVec2::new(41, 0), IntentKind::Defend, PAINT_STRENGTH_CAP),
        ],
        &[
            SeedNanobots::new(NanobotType::Worker, 2),
            SeedNanobots::new(NanobotType::Hauler, 1),
            SeedNanobots::new(NanobotType::Defender, 1),
        ],
    );
}
