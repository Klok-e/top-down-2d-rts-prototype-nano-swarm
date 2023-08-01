use bevy::{
    reflect::{TypePath, TypeUuid},
    render::render_resource::{AsBindGroup, ShaderRef},
    sprite::Material2d,
};

#[derive(AsBindGroup, TypeUuid, TypePath, Debug, Clone)]
#[uuid = "606560b9-c6c2-442f-987b-b781237cf9d5"]
pub struct BackgroundMaterial {}

impl Material2d for BackgroundMaterial {
    fn fragment_shader() -> ShaderRef {
        "shaders/background_shader.wgsl".into()
    }
}
