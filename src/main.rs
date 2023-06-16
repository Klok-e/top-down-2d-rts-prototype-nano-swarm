mod fly_camera;
mod game_settings;
mod highlight_unit;
mod nanobot;
mod ui;

use anyhow::Result;
use bevy::{math::vec3, prelude::*};
use bevy_prototype_debug_lines::DebugLinesPlugin;
use fly_camera::{camera_2d_movement_system, FlyCamera2d};
use game_settings::GameSettings;
use highlight_unit::highlight_selected_system;
use nanobot::{GroupIdCounterResource, NanobotBundle, NanobotGroup, NanobotPlugin};
use ui::NanoswarmUiSetupPlugin;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins)
        .add_plugin(DebugLinesPlugin::default())
        .add_plugin(NanoswarmUiSetupPlugin)
        .add_plugin(NanobotPlugin::default())
        .add_startup_system(setup_things_startup.pipe(error_handler))
        .add_system(camera_2d_movement_system)
        .add_system(highlight_selected_system)
        .run();
}

fn setup_things_startup(
    mut commands: Commands,
    images: Res<AssetServer>,
    mut group_counter: ResMut<GroupIdCounterResource>,
) -> Result<()> {
    commands
        .spawn(Camera2dBundle::default())
        .insert(FlyCamera2d::default());

    commands.insert_resource(GameSettings::from_file_ron("config/game_settings.ron")?);

    commands
        .spawn((
            NanobotGroup {
                display_identifier: group_counter.next_id(),
            },
            SpatialBundle {
                ..Default::default()
            },
        ))
        .with_children(|p| {
            let texture = images.load("circle.png");
            p.spawn((NanobotBundle::default(),)).insert(SpriteBundle {
                texture: texture.clone(),
                ..default()
            });
            p.spawn((NanobotBundle::default(),)).insert(SpriteBundle {
                texture: texture.clone(),
                ..default()
            });
            p.spawn((NanobotBundle::default(),)).insert(SpriteBundle {
                texture: texture.clone(),
                ..default()
            });
            p.spawn((NanobotBundle::default(),)).insert(SpriteBundle {
                texture,
                ..default()
            });
        });

    commands
        .spawn((
            NanobotGroup {
                display_identifier: group_counter.next_id(),
            },
            SpatialBundle {
                ..Default::default()
            },
        ))
        .with_children(|p| {
            let texture = images.load("circle.png");
            p.spawn((NanobotBundle::default(),)).insert(SpriteBundle {
                texture: texture.clone(),
                transform: Transform::from_translation(vec3(100., 0., 0.)),
                ..default()
            });
            p.spawn((NanobotBundle::default(),)).insert(SpriteBundle {
                texture: texture.clone(),
                transform: Transform::from_translation(vec3(100., 0., 0.)),
                ..default()
            });
            p.spawn((NanobotBundle::default(),)).insert(SpriteBundle {
                texture: texture.clone(),
                transform: Transform::from_translation(vec3(100., 0., 0.)),
                ..default()
            });
            p.spawn((NanobotBundle::default(),)).insert(SpriteBundle {
                texture,
                transform: Transform::from_translation(vec3(100., 0., 0.)),
                ..default()
            });
        });
    Ok(())
}

fn error_handler(In(result): In<Result<()>>) {
    if let Err(err) = result {
        println!("encountered an error {:?}", err);
    }
}
