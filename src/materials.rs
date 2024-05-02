use bevy::{
    asset::Asset,
    reflect::TypePath,
    render::render_resource::{AsBindGroup, ShaderRef},
    sprite::Material2d,
};

#[derive(AsBindGroup, TypePath, Asset, Debug, Clone)]
pub struct BackgroundMaterial {}

impl Material2d for BackgroundMaterial {
    fn fragment_shader() -> ShaderRef {
        "shaders/background_shader.wgsl".into()
    }
}
