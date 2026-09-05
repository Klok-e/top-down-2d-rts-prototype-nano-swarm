//! Resource quantity expressed as crystals above a permanent physical mineral base.
use bevy::{
    prelude::*,
    render::render_resource::AsBindGroup,
    shader::ShaderRef,
    sprite_render::{AlphaMode2d, Material2d, Material2dPlugin},
};

use crate::{fly_camera::CameraZoom2d, resources::ResourceDeposit};

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct DepositMaterial {
    /// Remaining fraction and projected footprint radius in pixels.
    #[uniform(0)]
    pub appearance: Vec4,
}

impl Material2d for DepositMaterial {
    fn fragment_shader() -> ShaderRef {
        "shaders/deposit_shader.wgsl".into()
    }
    fn alpha_mode(&self) -> AlphaMode2d {
        AlphaMode2d::Blend
    }
}

pub struct DepositPresentationPlugin;
impl Plugin for DepositPresentationPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(Material2dPlugin::<DepositMaterial>::default())
            .add_systems(Update, sync_deposit_presentation);
    }
}

#[derive(Component)]
pub struct DepositVisual {
    local_size: Vec2,
}

/// Reconciles authored sprites and quantity changes without changing simulation footprints.
#[allow(clippy::type_complexity)]
pub fn sync_deposit_presentation(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<DepositMaterial>>,
    zoom: Query<&CameraZoom2d>,
    mut deposits: Query<(
        Entity,
        &ResourceDeposit,
        &Transform,
        Option<(
            &Mesh2d,
            &MeshMaterial2d<DepositMaterial>,
            &mut DepositVisual,
        )>,
    )>,
) {
    let zoom = zoom.iter().next().map_or(1.0, |zoom| zoom.zoom).max(0.001);
    for (entity, deposit, transform, visual) in &mut deposits {
        let fraction = if deposit.capacity == 0 {
            0.0
        } else {
            (deposit.amount as f32 / deposit.capacity as f32).clamp(0.0, 1.0)
        };
        let appearance = Vec4::new(fraction, deposit.radius / zoom, 0.0, 0.0);
        let local_size = Vec2::splat(deposit.radius * 2.0)
            / transform.scale.truncate().abs().max(Vec2::splat(0.001));
        if let Some((mesh, material, mut visual)) = visual {
            if materials
                .get(&material.0)
                .is_some_and(|value| value.appearance != appearance)
                && let Some(material) = materials.get_mut(&material.0)
            {
                material.appearance = appearance;
            }
            if visual.local_size != local_size {
                if let Some(mesh) = meshes.get_mut(&mesh.0) {
                    *mesh = Rectangle::from_size(local_size).into();
                }
                visual.local_size = local_size;
            }
        } else {
            commands
                .entity(entity)
                .remove::<Sprite>()
                .remove::<MeshMaterial2d<ColorMaterial>>()
                .insert((
                    Mesh2d(meshes.add(Rectangle::from_size(local_size))),
                    MeshMaterial2d(materials.add(DepositMaterial { appearance })),
                    DepositVisual { local_size },
                ));
        }
    }
}
