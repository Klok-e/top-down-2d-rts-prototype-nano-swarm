mod fly_camera;
mod game_settings;
mod highlight_unit;
mod nanobot;
mod ui;

use anyhow::Result;
use bevy::{
    math::vec3,
    prelude::*,
    reflect::TypeUuid,
    render::render_resource::{AsBindGroup, ShaderRef},
    sprite::{Material2d, Material2dPlugin, MaterialMesh2dBundle},
};
use bevy_prototype_debug_lines::DebugLinesPlugin;
use fly_camera::{Camera2dFlyPlugin, CameraZoom2d, FlyCamera2d};
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
        .add_plugin(Material2dPlugin::<BackgroundMaterial>::default())
        .add_plugin(Camera2dFlyPlugin)
        .add_startup_system(setup_things_startup.pipe(error_handler))
        .add_system(highlight_selected_system)
        .run();
}

fn setup_things_startup(
    mut commands: Commands,
    images: Res<AssetServer>,
    mut mats: ResMut<Assets<BackgroundMaterial>>,
    mut meshes: ResMut<Assets<Mesh>>,
    group_counter: ResMut<GroupIdCounterResource>,
) -> Result<()> {
    commands
        .spawn(Camera2dBundle::default())
        .insert(FlyCamera2d::default())
        .insert(CameraZoom2d {
            zoom_speed: 10.,
            zoom_min_max: (1., 100.),
            zoom: 1.,
        });

    commands.insert_resource(GameSettings::from_file_ron("config/game_settings.ron")?);

    spawn_nanobots_for_testing(&mut commands, group_counter, images);

    commands.spawn(MaterialMesh2dBundle {
        mesh: meshes.add(Mesh::from(shape::Quad::default())).into(),
        material: mats.add(BackgroundMaterial {}),
        transform: Transform::default().with_scale(Vec2::splat(128000.).extend(-100.)),
        ..default()
    });
    Ok(())
}

fn spawn_nanobots_for_testing(
    commands: &mut Commands<'_, '_>,
    mut group_counter: ResMut<'_, GroupIdCounterResource>,
    images: Res<'_, AssetServer>,
) {
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
                transform: Transform::from_translation(vec3(100., 0., 1.)),
                ..default()
            });
            p.spawn((NanobotBundle::default(),)).insert(SpriteBundle {
                texture: texture.clone(),
                transform: Transform::from_translation(vec3(100., 0., 1.)),
                ..default()
            });
            p.spawn((NanobotBundle::default(),)).insert(SpriteBundle {
                texture: texture.clone(),
                transform: Transform::from_translation(vec3(100., 0., 1.)),
                ..default()
            });
            p.spawn((NanobotBundle::default(),)).insert(SpriteBundle {
                texture,
                transform: Transform::from_translation(vec3(100., 0., 1.)),
                ..default()
            });
        });
}

fn error_handler(In(result): In<Result<()>>) {
    if let Err(err) = result {
        println!("encountered an error {:?}", err);
    }
}

#[derive(AsBindGroup, TypeUuid, Debug, Clone)]
#[uuid = "606560b9-c6c2-442f-987b-b781237cf9d5"]
pub struct BackgroundMaterial {}

impl Material2d for BackgroundMaterial {
    fn fragment_shader() -> ShaderRef {
        "shaders/background_shader.wgsl".into()
    }
}
