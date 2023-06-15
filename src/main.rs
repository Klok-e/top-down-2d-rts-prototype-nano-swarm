mod fly_camera;
mod game_settings;
mod groups_merge_split;
mod highlight_unit;
mod nanobot;
mod ui_setup;
mod unit_select;

use anyhow::Result;
use bevy::{math::vec3, prelude::*};
use bevy_prototype_debug_lines::DebugLinesPlugin;
use fly_camera::{camera_2d_movement_system, FlyCamera2d};
use game_settings::GameSettings;
use groups_merge_split::group_action_system;
use highlight_unit::highlight_selected_system;
use nanobot::{
    bot_debug_circle_system, move_velocity_system, separation_system, velocity_system,
    NanobotBundle, NanobotGroup,
};
use ui_setup::NanoswarmUiSetupPlugin;
use unit_select::{unit_select_system, NanobotGroupAction, SelectedGroupsChanged};

fn main() {
    App::new()
        .add_plugins(DefaultPlugins)
        .add_plugin(DebugLinesPlugin::default())
        .add_event::<SelectedGroupsChanged>()
        .add_event::<NanobotGroupAction>()
        .add_startup_system(setup_things_startup.pipe(error_handler))
        .add_system(camera_2d_movement_system)
        .add_system(move_velocity_system)
        .add_system(bot_debug_circle_system)
        .add_system(unit_select_system)
        .add_system(highlight_selected_system)
        .add_system(separation_system)
        .add_system(velocity_system)
        .add_system(group_action_system)
        .add_plugin(NanoswarmUiSetupPlugin)
        .run();
}

fn setup_things_startup(mut commands: Commands, images: Res<AssetServer>) -> Result<()> {
    commands
        .spawn(Camera2dBundle::default())
        .insert(FlyCamera2d::default());

    commands.insert_resource(GameSettings::from_file_ron("config/game_settings.ron")?);
    commands
        .spawn((
            NanobotGroup {
                display_identifier: 1,
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
                display_identifier: 2,
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
