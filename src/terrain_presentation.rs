//! Continuous rock surfaces derived from the same footprints as navigation.
use bevy::{
    prelude::*,
    render::render_resource::AsBindGroup,
    shader::ShaderRef,
    sprite_render::{Material2d, Material2dPlugin, MeshMaterial2d},
};

#[derive(Component)]
pub struct RockSurfaceRoot;

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct RockMaterial {
    #[uniform(0)]
    tint: Vec4,
}
impl Material2d for RockMaterial {
    fn fragment_shader() -> ShaderRef {
        "shaders/rock_shader.wgsl".into()
    }
    fn alpha_mode(&self) -> bevy::sprite_render::AlphaMode2d {
        bevy::sprite_render::AlphaMode2d::Blend
    }
}

pub struct TerrainPresentationPlugin;
impl Plugin for TerrainPresentationPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(Material2dPlugin::<RockMaterial>::default())
            .add_systems(Update, present_rocks);
    }
}
fn present_rocks(
    mut commands: Commands,
    roots: Query<(Entity, &Transform), Added<RockSurfaceRoot>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<RockMaterial>>,
) {
    for (root, transform) in &roots {
        commands.entity(root).with_children(|children| {
            children.spawn((
                Mesh2d(meshes.add(crate::scenario::rock_surface_mesh(
                    transform.translation.truncate(),
                ))),
                MeshMaterial2d(materials.add(RockMaterial { tint: Vec4::ONE })),
                Transform::default(),
            ));
        });
    }
}
