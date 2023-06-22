mod fly_camera;
mod game_settings;
mod highlight_unit;
mod materials;
mod nanobot;
mod ui;
mod zones;

use anyhow::Result;
use bevy::{
    math::vec3,
    prelude::*,
    sprite::{Material2dPlugin, MaterialMesh2dBundle},
};
use bevy_prototype_debug_lines::DebugLinesPlugin;
use fly_camera::{Camera2dFlyPlugin, CameraZoom2d, FlyCamera2d};
use game_settings::GameSettings;
use highlight_unit::highlight_selected_system;
use materials::BackgroundMaterial;
use nanobot::{
    GroupIdCounterResource, NanobotBundle, NanobotGroup, NanobotGroupBundle, NanobotPlugin,
};
use ui::NanoswarmUiSetupPlugin;
use zones::{ZoneComponent, ZoneMaterial, ZoneMaterialHandleComponent, ZonePointData, ZonesPlugin};

fn main() {
    App::new()
        .add_plugins(DefaultPlugins)
        .add_plugin(DebugLinesPlugin::default())
        .add_plugin(NanoswarmUiSetupPlugin)
        .add_plugin(NanobotPlugin::default())
        .add_plugin(Material2dPlugin::<BackgroundMaterial>::default())
        .add_plugin(Camera2dFlyPlugin)
        .add_plugin(ZonesPlugin::default())
        .add_startup_system(setup_things_startup.pipe(error_handler))
        .add_system(highlight_selected_system)
        .run();
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
    group_counter: ResMut<GroupIdCounterResource>,
) -> Result<()> {
    let handle = zone_mats.add(ZoneMaterial::new(MAP_WIDTH, MAP_HEIGHT));
    commands
        .spawn(Camera2dBundle::default())
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

    spawn_nanobots_for_testing(&mut commands, group_counter, asset_server);

    // background
    commands.spawn(MaterialMesh2dBundle {
        mesh: meshes.add(Mesh::from(shape::Quad::default())).into(),
        material: bg_mats.add(BackgroundMaterial {}),
        transform: Transform::default().with_scale(
            Vec2::new(
                MAP_WIDTH as f32 * ZONE_BLOCK_SIZE,
                MAP_HEIGHT as f32 * ZONE_BLOCK_SIZE,
            )
            .extend(-101.),
        ),
        ..default()
    });

    // zones
    commands.spawn(MaterialMesh2dBundle {
        mesh: meshes.add(Mesh::from(shape::Quad::default())).into(),
        material: handle,
        transform: Transform::default().with_scale(
            Vec2::new(
                MAP_WIDTH as f32 * ZONE_BLOCK_SIZE,
                MAP_HEIGHT as f32 * ZONE_BLOCK_SIZE,
            )
            .extend(-100.),
        ),
        ..default()
    });
    Ok(())
}

fn spawn_nanobots_for_testing(
    commands: &mut Commands<'_, '_>,
    mut group_counter: ResMut<'_, GroupIdCounterResource>,
    asset_server: Res<'_, AssetServer>,
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
            p.spawn((NanobotBundle::default(),)).insert(SpriteBundle {
                texture: texture.clone(),
                transform: Transform::from_translation(vec3(0., 0., 1.)),
                ..default()
            });
            p.spawn((NanobotBundle::default(),)).insert(SpriteBundle {
                texture: texture.clone(),
                transform: Transform::from_translation(vec3(0., 0., 1.)),
                ..default()
            });
            p.spawn((NanobotBundle::default(),)).insert(SpriteBundle {
                texture: texture.clone(),
                transform: Transform::from_translation(vec3(0., 0., 1.)),
                ..default()
            });
            p.spawn((NanobotBundle::default(),)).insert(SpriteBundle {
                texture,
                transform: Transform::from_translation(vec3(0., 0., 1.)),
                ..default()
            });
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
                p.spawn((NanobotBundle::default(),)).insert(SpriteBundle {
                    texture: texture.clone(),
                    transform: Transform::from_translation(vec3(100., 0., 1.)),
                    ..default()
                });
            }
        });
}

fn error_handler(In(result): In<Result<()>>) {
    if let Err(err) = result {
        println!("encountered an error {:?}", err);
    }
}
