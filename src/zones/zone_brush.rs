use bevy::{
    math::{ivec2, vec2},
    prelude::{
        Assets, Camera, Component, EventReader, EventWriter, GlobalTransform, Handle, IVec2, Input,
        MouseButton, Query, Res, ResMut, With,
    },
    reflect::TypeUuid,
    render::render_resource::{AsBindGroup, ShaderType},
    sprite::Material2d,
    window::Window,
};
use bitflags::{Flag, Flags};

use crate::{
    nanobot::Selected,
    ui::{zone_button::MouseActionMode, UiHandling},
    MAP_HEIGHT, MAP_WIDTH, ZONE_BLOCK_SIZE,
};

use super::ZoneComponent;

#[derive(AsBindGroup, TypeUuid, Debug, Clone)]
#[uuid = "4dd16810-1f6c-4cc3-9e12-6f363e0211c7"]
pub struct ZoneMaterial {
    #[storage(2, read_only)]
    pub zone_map: Vec<ZoneMapPointColorData>,
    #[uniform(3)]
    pub width: u32,
    #[uniform(4)]
    pub height: u32,
}

impl ZoneMaterial {
    pub fn new(width: u32, height: u32) -> ZoneMaterial {
        ZoneMaterial {
            zone_map: vec![ZoneMapPointColorData { zones: 0 }; (width * height) as usize],
            width,
            height,
        }
    }

    pub fn _at_zone(&self, x: u32, y: u32) -> Option<&ZoneMapPointColorData> {
        if x >= self.width || y >= self.height {
            None
        } else {
            Some(&self.zone_map[(y * self.width + x) as usize])
        }
    }

    pub fn at_zone_mut(&mut self, x: u32, y: u32) -> Option<&mut ZoneMapPointColorData> {
        if x >= self.width || y >= self.height {
            None
        } else {
            Some(&mut self.zone_map[(y * self.width + x) as usize])
        }
    }
}

#[derive(Debug, Clone, Copy, ShaderType)]
pub struct ZoneMapPointColorData {
    zones: u32,
}

impl ZoneMapPointColorData {
    pub const ZONE1: ZoneMapPointColorData = ZoneMapPointColorData { zones: 1 << 0 };
    pub const ZONE2: ZoneMapPointColorData = ZoneMapPointColorData { zones: 1 << 1 };
    pub const ZONE3: ZoneMapPointColorData = ZoneMapPointColorData { zones: 1 << 2 };
    pub const ZONE4: ZoneMapPointColorData = ZoneMapPointColorData { zones: 1 << 3 };
    pub const ZONE5: ZoneMapPointColorData = ZoneMapPointColorData { zones: 1 << 4 };
    pub const ZONE6: ZoneMapPointColorData = ZoneMapPointColorData { zones: 1 << 5 };
}

impl Flags for ZoneMapPointColorData {
    const FLAGS: &'static [bitflags::Flag<Self>] = &[
        Flag::new("zone1", Self::ZONE1),
        Flag::new("zone2", Self::ZONE2),
        Flag::new("zone3", Self::ZONE3),
        Flag::new("zone4", Self::ZONE4),
        Flag::new("zone5", Self::ZONE5),
        Flag::new("zone6", Self::ZONE6),
    ];

    type Bits = u32;

    fn bits(&self) -> Self::Bits {
        self.zones
    }

    fn from_bits_retain(bits: Self::Bits) -> Self {
        Self { zones: bits }
    }
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
    zone_color: ZoneMapPointColorData,
    kind: ZoneChangedKind,
}

#[derive(Debug)]
pub enum ZoneChangedKind {
    PointAdded,
    PointRemoved,
}

pub fn zone_texture_update_system(
    mut zone_mats: ResMut<Assets<ZoneMaterial>>,
    zone_handle: Query<&ZoneMaterialHandleComponent>,
    mut ev_zone_changed: EventReader<ZoneChangedEvent>,
) {
    for handle in &zone_handle {
        for ev in ev_zone_changed.iter() {
            let mat = zone_mats
                .get_mut(&handle.handle)
                .expect("Handle must be valid");

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

            match ev.kind {
                ZoneChangedKind::PointAdded => {
                    let zone_data = mat
                        .at_zone_mut(point.x as u32, point.y as u32)
                        .expect("Bounds check already happened");
                    *zone_data = zone_data.union(ev.zone_color);
                }
                ZoneChangedKind::PointRemoved => {
                    let zone_data = mat
                        .at_zone_mut(point.x as u32, point.y as u32)
                        .expect("Bounds check already happened");
                    *zone_data = zone_data.intersection(ev.zone_color.complement());
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
    mut zones: Query<&mut ZoneComponent, With<Selected>>,
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
        if let Some(mut zone) = zones.iter_mut().next() {
            zone.zone_points.insert(idx);

            // notify
            ev_zone_changed.send(ZoneChangedEvent {
                point: idx,
                zone_color: zone.zone_color,
                kind: ZoneChangedKind::PointAdded,
            })
        }
    } else if mouse_button_input.pressed(MouseButton::Right) {
        if let Some(mut zone) = zones.iter_mut().next() {
            zone.zone_points.remove(&idx);

            // notify
            ev_zone_changed.send(ZoneChangedEvent {
                point: idx,
                zone_color: zone.zone_color,
                kind: ZoneChangedKind::PointRemoved,
            })
        }
    }
}
