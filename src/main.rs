mod creature;
mod fly_camera;

use bevy::{core::Zeroable, prelude::*};
use bevy_rapier2d::prelude::*;
use creature::{move_creature_system, Creature};
use fly_camera::{camera_2d_movement_system, FlyCamera2d};
use rand::Rng;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins)
        .insert_resource(RapierConfiguration {
            gravity: Vec2::zeroed(),
            ..default()
        })
        .add_plugin(RapierPhysicsPlugin::<NoUserData>::default())
        .add_plugin(RapierDebugRenderPlugin::default())
        .add_startup_system(setup_things_startup)
        .add_system(camera_2d_movement_system)
        .add_system(spawn_food_system)
        .add_system(move_creature_system)
        .run();
}

fn setup_things_startup(mut commands: Commands, images: Res<AssetServer>) {
    commands
        .spawn_bundle(Camera2dBundle::default())
        .insert(FlyCamera2d::default());

    commands.insert_resource(GameSettings {
        width: 1000.,
        height: 1000.,
        food_amount: 100,
        food_timeout: 2.,
    });

    commands
        .spawn()
        .insert(RigidBody::Dynamic)
        .insert(Collider::ball(12.))
        .insert(Restitution::coefficient(0.7))
        .insert(Velocity::default())
        .insert(Creature {})
        .insert_bundle(SpriteBundle {
            texture: images.load("food_sprite.png"),
            ..default()
        });
}

fn spawn_food_system(
    images: Res<AssetServer>,
    mut foods: Query<(Entity, &mut Food)>,
    mut commands: Commands,
    time: Res<Time>,
    settings: Res<GameSettings>,
) {
    let mut rng = rand::thread_rng();

    let foods_count = settings.food_amount;
    if foods.iter().count() < foods_count {
        let x = rng.gen_range(-settings.width..settings.width);
        let y = rng.gen_range(-settings.height..settings.height);
        commands
            .spawn()
            .insert(Food {
                spawned: Timer::from_seconds(settings.food_timeout, false),
            })
            .insert_bundle(SpriteBundle {
                texture: images.load("food_sprite.png"),
                transform: Transform::from_translation(Vec3::new(x, y, 0.)),
                ..default()
            })
            .insert(RigidBody::Dynamic)
            .insert(Collider::ball(12.))
            .insert(Restitution::coefficient(0.7));
    }

    for (ent, mut food) in foods.iter_mut() {
        let food: &mut Food = &mut *food;
        let ent: Entity = ent;

        food.spawned.tick(time.delta());

        if food.spawned.finished() {
            commands.entity(ent).despawn();
        }
    }
}

#[derive(Component)]
pub struct Food {
    pub spawned: Timer,
}

pub struct GameSettings {
    pub width: f32,
    pub height: f32,
    pub food_amount: usize,
    pub food_timeout: f32,
}
