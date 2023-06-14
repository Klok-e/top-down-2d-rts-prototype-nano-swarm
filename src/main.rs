mod fly_camera;
mod game_settings;
mod highlight_unit;
mod nanobot;
mod unit_select;

use bevy::prelude::*;
use bevy_prototype_debug_lines::DebugLinesPlugin;
use fly_camera::{camera_2d_movement_system, FlyCamera2d};
use game_settings::GameSettings;
use highlight_unit::highlight_selected_system;
use nanobot::{bot_debug_circle_system, move_velocity_system, Nanobot};
use unit_select::unit_select_system;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins)
        .add_plugin(DebugLinesPlugin::default())
        .add_startup_system(setup_things_startup)
        .add_system(camera_2d_movement_system)
        .add_system(move_velocity_system)
        .add_system(bot_debug_circle_system)
        .add_system(unit_select_system)
        .add_system(highlight_selected_system)
        .run();
}

fn setup_things_startup(mut commands: Commands, images: Res<AssetServer>) {
    commands
        .spawn(Camera2dBundle::default())
        .insert(FlyCamera2d::default());

    commands.insert_resource(GameSettings {
        width: 1000.,
        height: 1000.,
        bot_speed: 5.,
    });

    commands.spawn((Nanobot {},)).insert(SpriteBundle {
        texture: images.load("circle.png"),
        ..default()
    });
}
