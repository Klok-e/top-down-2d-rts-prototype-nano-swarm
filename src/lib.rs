pub mod ai;
pub mod building;
pub mod fly_camera;
pub mod game_settings;
pub mod intent;
pub mod materials;
pub mod nanobot;
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
use intent::IntentGrid;
use materials::BackgroundMaterial;
use nanobot::{NanobotBundle, NanobotPlugin, Swarm, SwarmBundle};
use ui::NanoswarmUiSetupPlugin;
use zones::{ZoneMaterial, ZoneMaterialHandleComponent, ZonesPlugin};

pub fn build_app() -> App {
    let mut app = App::new();
    app.add_plugins(DefaultPlugins)
        .add_plugins(Material2dPlugin::<BackgroundMaterial>::default())
        // must be before NanobotPlugin because otherwise it receives events with despawned entities
        .add_plugins(NanoswarmUiSetupPlugin)
        // must be before NanobotPlugin because otherwise it receives events with despawned entities
        .add_plugins(ZonesPlugin::default())
        .add_plugins(NanobotPlugin::default())
        .add_plugins(AiPlugin)
        .add_plugins(Camera2dFlyPlugin)
        .add_systems(Startup, setup_things_startup.pipe(error_handler));
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
    commands.insert_resource(IntentGrid::new(MAP_WIDTH as i32, MAP_HEIGHT as i32));

    spawn_initial_swarm(&mut commands, &asset_server);

    // minerals
    let minerals_texture = asset_server.load("minerals.png");
    commands.spawn((
        Minerals {},
        (
            Sprite::from_image(minerals_texture.clone()),
            Transform::from_translation(vec3(-800., 0., 1.)).with_scale(vec3(2., 2., 1.)),
        ),
    ));

    // processing
    let processing_texture = asset_server.load("mineral processing.png");
    commands.spawn((
        ProcessingFacility {},
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
