use bevy::{
    asset::Asset, prelude::*, reflect::TypePath, render::render_resource::AsBindGroup,
    shader::ShaderRef, sprite_render::Material2d,
};

use crate::zones::BrushFeedback;

#[derive(AsBindGroup, TypePath, Asset, Debug, Clone)]
pub struct BackgroundMaterial {
    #[uniform(0)]
    pub paint_grid: Vec4,
}

impl Default for BackgroundMaterial {
    fn default() -> Self {
        Self {
            paint_grid: Vec4::new(0.0, crate::ZONE_BLOCK_SIZE, 0.0, 0.0),
        }
    }
}

/// Cell guides appear only during a world-space paint or erase gesture.
pub fn update_paint_grid(
    materials: Option<ResMut<Assets<BackgroundMaterial>>>,
    feedback: Res<BrushFeedback>,
) {
    let Some(mut materials) = materials else {
        return;
    };
    let active = feedback.active_gesture;
    let visibility = if active { 1.0 } else { 0.0 };
    let changed = materials
        .iter()
        .filter_map(|(id, material)| ((material.paint_grid.x > 0.5) != active).then_some(id))
        .collect::<Vec<_>>();
    for id in changed {
        if let Some(material) = materials.get_mut(id) {
            material.paint_grid.x = visibility;
        }
    }
}

impl Material2d for BackgroundMaterial {
    fn fragment_shader() -> ShaderRef {
        "shaders/background_shader.wgsl".into()
    }
}
