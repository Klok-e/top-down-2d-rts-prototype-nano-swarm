pub mod ai;
pub mod building;
pub mod fly_camera;
pub mod game_settings;
pub mod highlight_unit;
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
use highlight_unit::highlight_selected_system;
use materials::BackgroundMaterial;
use nanobot::{
    GroupIdCounterResource, NanobotBundle, NanobotGroup, NanobotGroupBundle, NanobotPlugin,
};
use ui::NanoswarmUiSetupPlugin;
use zones::{ZoneComponent, ZoneMaterial, ZoneMaterialHandleComponent, ZonePointData, ZonesPlugin};

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
        .add_systems(Startup, setup_things_startup.pipe(error_handler))
        .add_systems(Update, highlight_selected_system);
    app
}

pub fn run() {
    build_app().run();
}

const MAP_WIDTH: u32 = 1000;
const MAP_HEIGHT: u32 = 1000;
const ZONE_BLOCK_SIZE: f32 = 512.;

fn setup_things_startup(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut bg_mats: ResMut<Assets<BackgroundMaterial>>,
    mut zone_mats: ResMut<Assets<ZoneMaterial>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut buffers: ResMut<Assets<ShaderStorageBuffer>>,
    group_counter: ResMut<GroupIdCounterResource>,
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

    spawn_initial_swarm(&mut commands, group_counter, &asset_server);

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

fn spawn_initial_swarm(
    commands: &mut Commands<'_, '_>,
    mut group_counter: ResMut<'_, GroupIdCounterResource>,
    asset_server: &Res<'_, AssetServer>,
) {
    commands
        .spawn((NanobotGroupBundle {
            group: NanobotGroup {
                id: group_counter.next_id(),
            },
            zone: ZoneComponent {
                zone_points: default(),
                zone_color: ZonePointData::ZONE2,
            },
            ..default()
        },))
        .with_children(|p| {
            let texture = asset_server.load("circle.png");
            p.spawn((
                NanobotBundle::default(),
                Sprite::from_image(texture.clone()),
                Transform::from_translation(vec3(0., 0., 1.)),
            ));
            p.spawn((
                NanobotBundle::default(),
                Sprite::from_image(texture.clone()),
                Transform::from_translation(vec3(0., 0., 1.)),
            ));
            p.spawn((
                NanobotBundle::default(),
                Sprite::from_image(texture.clone()),
                Transform::from_translation(vec3(0., 0., 1.)),
            ));
            p.spawn((
                NanobotBundle::default(),
                Sprite::from_image(texture),
                Transform::from_translation(vec3(0., 0., 1.)),
            ));
        });

    commands
        .spawn(NanobotGroupBundle {
            group: NanobotGroup {
                id: group_counter.next_id(),
            },
            zone: ZoneComponent {
                zone_points: default(),
                zone_color: ZonePointData::ZONE1,
            },
            ..default()
        })
        .with_children(|p| {
            let texture = asset_server.load("circle.png");
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
