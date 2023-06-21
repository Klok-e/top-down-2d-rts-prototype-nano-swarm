use bevy::{
    math::{ivec2, vec2},
    prelude::{
        Assets, Camera, Component, EventReader, EventWriter, GlobalTransform, Handle, IVec2, Image,
        Input, MouseButton, Query, Res, ResMut,
    },
    reflect::TypeUuid,
    render::render_resource::AsBindGroup,
    sprite::Material2d,
    window::Window,
};

use crate::{
    ui::{zone_button::MouseActionMode, UiHandling},
    MAP_HEIGHT, MAP_WIDTH, ZONE_BLOCK_SIZE,
};

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

#[derive(Debug)]
pub struct ZoneChangedEvent {
    point: IVec2,
    kind: ZoneChangedKind,
}

#[derive(Debug)]
pub enum ZoneChangedKind {
    PointAdded,
    PointRemoved,
}

pub fn zone_texture_update_system(
    mut zone_mats: ResMut<Assets<ZoneMaterial>>,
    mut images: ResMut<Assets<Image>>,
    zone_handle: Query<&ZoneMaterialHandleComponent>,
    mut ev_zone_changed: EventReader<ZoneChangedEvent>,
) {
    for handle in &zone_handle {
        let bytes_per_pixel = 4;
        for ev in ev_zone_changed.iter() {
            let mat = zone_mats
                .get_mut(&handle.handle)
                .expect("Handle must be valid");
            let image = images.get_mut(&mat.texture).expect("Handle must be valid");

            let point = ev.point;

            // add offset
            let mut point = point + ivec2(MAP_WIDTH as i32, MAP_HEIGHT as i32) / 2;
            point.y = MAP_HEIGHT as i32 - point.y - 1;
            if point.x < 0
                || point.x >= MAP_WIDTH as i32
                || point.y < 0
                || point.y >= MAP_HEIGHT as i32
            {
                log::warn!("point {point} out of range");
                continue;
            }

            let idx = ((point.y * image.size().x as i32 + point.x) * bytes_per_pixel) as usize;

            match ev.kind {
                ZoneChangedKind::PointAdded => {
                    image.data[idx..idx + 4].copy_from_slice(&[0, 0, 255, 128])
                }
                ZoneChangedKind::PointRemoved => {
                    image.data[idx..idx + 4].copy_from_slice(&[0, 0, 0, 0])
                }
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
    mut ev_zone_changed: EventWriter<ZoneChangedEvent>,
    mouse_mode: Res<MouseActionMode>,
) {
    // don't do anything if cursor over ui
    if ui_handling.is_pointer_over_ui {
        return;
    }

    // mouse mode must be appropriate for this system
    if *mouse_mode != MouseActionMode::ZoneDraw {
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

    let idx = vec2(
        (cursor_pos_world.x / ZONE_BLOCK_SIZE).floor(),
        (cursor_pos_world.y / ZONE_BLOCK_SIZE).floor(),
    )
    .as_ivec2();
    if idx.x < -(MAP_WIDTH as i32 / 2)
        || idx.x >= (MAP_WIDTH as i32 / 2)
        || idx.y < -(MAP_HEIGHT as i32 / 2)
        || idx.y >= (MAP_HEIGHT as i32 / 2)
    {
        return;
    }
    if mouse_button_input.pressed(MouseButton::Left) {
        for mut zone in &mut zones {
            zone.zone_points.insert(idx);

            // notify
            ev_zone_changed.send(ZoneChangedEvent {
                point: idx,
                kind: ZoneChangedKind::PointAdded,
            })
        }
    } else if mouse_button_input.pressed(MouseButton::Right) {
        for mut zone in &mut zones {
            zone.zone_points.remove(&idx);

            // notify
            ev_zone_changed.send(ZoneChangedEvent {
                point: idx,
                kind: ZoneChangedKind::PointRemoved,
            })
        }
    }
}
