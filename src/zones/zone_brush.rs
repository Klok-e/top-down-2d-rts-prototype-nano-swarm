use bevy::{
    math::{ivec2, vec2},
    prelude::{
        Assets, Camera, Component, GlobalTransform, Handle, Image, Input, MouseButton, Query, Res,
        ResMut,
    },
    reflect::TypeUuid,
    render::render_resource::AsBindGroup,
    sprite::Material2d,
    window::Window,
};

use crate::{ui::UiHandling, MAP_HEIGHT, MAP_WIDTH, ZONE_BLOCK_SIZE};

use super::ZoneComponent;

#[derive(AsBindGroup, TypeUuid, Debug, Clone)]
#[uuid = "4dd16810-1f6c-4cc3-9e12-6f363e0211c7"]
pub struct ZoneMaterial {
    #[texture(0)]
    #[sampler(1)]
    pub texture: Handle<Image>,
}

impl Material2d for ZoneMaterial {
    fn fragment_shader() -> bevy::render::render_resource::ShaderRef {
        "shaders/zone_shader.wgsl".into()
    }
}

#[derive(Debug, Component)]
pub struct ZoneMaterialHandleComponent {
    pub handle: Handle<ZoneMaterial>,
}

pub fn zone_texture_update_system(
    mut zone_mats: ResMut<Assets<ZoneMaterial>>,
    mut images: ResMut<Assets<Image>>,
    zone_handle: Query<&ZoneMaterialHandleComponent>,
    zones: Query<&ZoneComponent>,
) {
    for handle in &zone_handle {
        let mat = zone_mats
            .get_mut(&handle.handle)
            .expect("Handle must be valid");
        let image = images.get_mut(&mat.texture).expect("Handle must be valid");

        let bytes_per_pixel = 4;
        for zone in zones.iter() {
            for point in zone.zone_points.iter() {
                // add offset
                let mut point = *point + ivec2(MAP_WIDTH as i32, MAP_HEIGHT as i32) / 2;
                point.y = MAP_HEIGHT as i32 - point.y;
                if point.x < 0
                    || point.x >= MAP_WIDTH as i32
                    || point.y < 0
                    || point.y >= MAP_HEIGHT as i32
                {
                    log::warn!("point {point} out of range");
                    continue;
                }

                let idx = ((point.y * image.size().x as i32 + point.x) * bytes_per_pixel) as usize;

                image.data[idx..idx + 4].copy_from_slice(&[0, 0, 255, 255]);
            }
        }
    }
}

pub fn zone_brush_system(
    windows: Query<&Window>,
    mouse_button_input: Res<Input<MouseButton>>,
    ui_handling: Res<UiHandling>,
    camera_query: Query<(&GlobalTransform, &Camera)>,
    mut zones: Query<&mut ZoneComponent>,
) {
    if ui_handling.is_pointer_over_ui {
        return;
    }

    // Get the cursor position in window coordinates
    let Some(cursor_pos) = windows.single().cursor_position() else {
        return;
    };

    // Convert the cursor position to world coordinates using viewport_to_world_2d
    let (camera_transform, camera) = camera_query.single();
    let cursor_pos_world =
        if let Some(pos) = camera.viewport_to_world_2d(camera_transform, cursor_pos) {
            pos
        } else {
            return;
        };

    if mouse_button_input.pressed(MouseButton::Left) {
        let value = vec2(
            (cursor_pos_world.x / ZONE_BLOCK_SIZE).floor(),
            (cursor_pos_world.y / ZONE_BLOCK_SIZE).floor(),
        )
        .as_ivec2();
        if value.x < -(MAP_WIDTH as i32 / 2)
            || value.x >= (MAP_WIDTH as i32 / 2)
            || value.y < -(MAP_HEIGHT as i32 / 2)
            || value.y >= (MAP_HEIGHT as i32 / 2)
        {
            dbg!(value);
        } else {
            dbg!("in bounds", value);
            for mut zone in &mut zones {
                zone.zone_points.insert(value);
            }
        }
    }
}
