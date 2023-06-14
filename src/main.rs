mod fly_camera;
mod game_settings;
mod nanobot;

use bevy::prelude::*;
use fly_camera::{camera_2d_movement_system, FlyCamera2d};
use game_settings::GameSettings;
use nanobot::{move_creature_system, Nanobot};

fn main() {
    App::new()
        .add_plugins(DefaultPlugins)
        .add_startup_system(setup_things_startup)
        .add_system(camera_2d_movement_system)
        .add_system(move_creature_system)
        .run();
}

fn setup_things_startup(mut commands: Commands, images: Res<AssetServer>) {
    commands
        .spawn(Camera2dBundle::default())
        .insert(FlyCamera2d::default());

    commands.insert_resource(GameSettings {
        width: 1000.,
        height: 1000.,
    });

    commands.spawn((Nanobot {},)).insert(SpriteBundle {
        texture: images.load("circle.png"),
        ..default()
    });
}
