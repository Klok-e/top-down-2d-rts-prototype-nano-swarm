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

use crate::{
    nanobot::{NanobotGroup, Selected},
    ui::{zone_button::MouseActionMode, SelectedGroupsChanged, UiHandling},
    MAP_HEIGHT, MAP_WIDTH, ZONE_BLOCK_SIZE,
};

use super::ZoneComponent;

#[derive(AsBindGroup, TypeUuid, Debug, Clone)]
#[uuid = "4dd16810-1f6c-4cc3-9e12-6f363e0211c7"]
pub struct ZoneMaterial {
    #[storage(2, read_only)]
    pub zone_map: Vec<ZonePointData>,
    #[uniform(3)]
    pub width: u32,
    #[uniform(4)]
    pub height: u32,
    #[uniform(1)]
    pub highlight_zone_id: u32,
}

impl ZoneMaterial {
    pub fn new(width: u32, height: u32) -> ZoneMaterial {
        ZoneMaterial {
            zone_map: vec![ZonePointData::new(); (width * height) as usize],
            width,
            height,
            highlight_zone_id: 0,
        }
    }

    pub fn at_zone(&self, x: u32, y: u32) -> Option<&ZonePointData> {
        if x >= self.width || y >= self.height {
            None
        } else {
            Some(&self.zone_map[(y * self.width + x) as usize])
        }
    }

    pub fn at_zone_mut(&mut self, x: u32, y: u32) -> Option<&mut ZonePointData> {
        if x >= self.width || y >= self.height {
            None
        } else {
            Some(&mut self.zone_map[(y * self.width + x) as usize])
        }
    }
}

#[derive(Debug, Clone, Copy, ShaderType)]
pub struct ZonePointData {
    /// First 4 bits are zone color indicators, rest are zone id (14 bits for each)
    zones: u32,
    /// 2 zone id indicators 14 bits each, last 4 bits are unused
    bits: u32,
}

impl ZonePointData {
    // Definitions
    pub const ZONE1: u32 = 1 << 0;
    pub const ZONE2: u32 = 1 << 1;
    pub const ZONE3: u32 = 1 << 2;
    pub const ZONE4: u32 = 1 << 3;

    pub const ZONE_ID_MASK: u32 = (1 << 14) - 1;

    pub fn new() -> Self {
        ZonePointData { zones: 0, bits: 0 }
    }

    // Set a specific zone to active or inactive
    pub fn set_zone(&mut self, zone: u32, active: bool) {
        if active {
            self.zones |= zone;
        } else {
            self.zones &= !zone;
        }
    }

    // Check if a specific zone is active
    pub fn is_zone_active(&self, zone: u32) -> bool {
        (self.zones & zone) != 0
    }

    pub fn get_zone_id(&self, zone: u32) -> u32 {
        match zone {
            Self::ZONE1 => (self.zones >> 4) & Self::ZONE_ID_MASK,
            Self::ZONE2 => (self.zones >> 18) & Self::ZONE_ID_MASK,
            Self::ZONE3 => self.bits & Self::ZONE_ID_MASK,
            Self::ZONE4 => (self.bits >> 14) & Self::ZONE_ID_MASK,
            _ => panic!("Invalid zone"),
        }
    }

    pub fn set_zone_id(&mut self, zone: u32, id: u32) {
        assert!(id <= Self::ZONE_ID_MASK, "ID too large for 14 bits");
        let id = id & Self::ZONE_ID_MASK; // ensure id fits in 14 bits
        match zone {
            Self::ZONE1 => {
                self.zones &= !(Self::ZONE_ID_MASK << 4); // clear existing id
                self.zones |= id << 4; // set new id
            }
            Self::ZONE2 => {
                self.zones &= !(Self::ZONE_ID_MASK << 18); // clear existing id
                self.zones |= id << 18; // set new id
            }
            Self::ZONE3 => {
                self.bits &= !Self::ZONE_ID_MASK; // clear existing id
                self.bits |= id; // set new id
            }
            Self::ZONE4 => {
                self.bits &= !(Self::ZONE_ID_MASK << 14); // clear existing id
                self.bits |= id << 14; // set new id
            }
            _ => panic!("Invalid zone"),
        }
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
    /// only 4 first bits are used
    zone_color: u32,
    /// only the first 14 bits are used
    zone_id: u32,
    kind: ZoneChangedKind,
}

#[derive(Debug)]
pub enum ZoneChangedKind {
    PointAdded,
    PointRemoved,
}

pub fn handle_zone_event_system(
    mut zone_mats: ResMut<Assets<ZoneMaterial>>,
    zone_handle: Query<&ZoneMaterialHandleComponent>,
    mut ev_zone_changed: EventReader<ZoneChangedEvent>,
    mut zones: Query<(&mut ZoneComponent,), (With<Selected>, With<NanobotGroup>)>,
) {
    for ev in ev_zone_changed.iter() {
        let handle = zone_handle.single();
        let mat = zone_mats
            .get_mut(&handle.handle)
            .expect("Handle must be valid");

        let point = ev.point;

        // add offset
        let mut idx = point + ivec2(MAP_WIDTH as i32, MAP_HEIGHT as i32) / 2;
        idx.y = MAP_HEIGHT as i32 - idx.y - 1;
        if idx.x < 0 || idx.x >= MAP_WIDTH as i32 || idx.y < 0 || idx.y >= MAP_HEIGHT as i32 {
            log::warn!("point {point} out of range");
            continue;
        }

        let (mut zone,) = zones
            .iter_mut()
            .next()
            .expect("It's impossible for there to be no selected groups at this point");

        match ev.kind {
            ZoneChangedKind::PointAdded => {
                let zone_data = mat
                    .at_zone(idx.x as u32, idx.y as u32)
                    .expect("Bounds check already happened");
                if zone_data.is_zone_active(ev.zone_color)
                    && zone_data.get_zone_id(ev.zone_color) != ev.zone_id
                {
                    log::warn!("Tried to add a point to a zone, but this point was already in another zone")
                } else {
                    let zone_data = mat
                        .at_zone_mut(idx.x as u32, idx.y as u32)
                        .expect("Bounds check already happened");
                    zone_data.set_zone(ev.zone_color, true);
                    zone_data.set_zone_id(ev.zone_color, ev.zone_id);

                    zone.zone_points.insert(point);
                }
            }
            ZoneChangedKind::PointRemoved => {
                let zone_data = mat
                    .at_zone_mut(idx.x as u32, idx.y as u32)
                    .expect("Bounds check already happened");
                zone_data.set_zone(ev.zone_color, false);
                zone_data.set_zone_id(ev.zone_color, ev.zone_id);

                zone.zone_points.remove(&point);
            }
        }
    }
}

pub fn zone_brush_system(
    windows: Query<&Window>,
    mouse_button_input: Res<Input<MouseButton>>,
    ui_handling: Res<UiHandling>,
    camera_query: Query<(&GlobalTransform, &Camera)>,
    zones: Query<(&ZoneComponent, &NanobotGroup), With<Selected>>,
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
        if let Some((zone, group)) = zones.iter().next() {
            // notify
            ev_zone_changed.send(ZoneChangedEvent {
                point: idx,
                zone_color: zone.zone_color,
                zone_id: group.id as u32,
                kind: ZoneChangedKind::PointAdded,
            })
        }
    } else if mouse_button_input.pressed(MouseButton::Right) {
        if let Some((zone, group)) = zones.iter().next() {
            // notify
            ev_zone_changed.send(ZoneChangedEvent {
                point: idx,
                zone_color: zone.zone_color,
                zone_id: group.id as u32,
                kind: ZoneChangedKind::PointRemoved,
            })
        }
    }
}

pub fn selected_zone_highlight_system(
    zones: Query<(&NanobotGroup,)>,
    mut zone_mats: ResMut<Assets<ZoneMaterial>>,
    zone_handle: Query<&ZoneMaterialHandleComponent>,
    mut ev_zone_select: EventReader<SelectedGroupsChanged>,
) {
    for ev in ev_zone_select.iter() {
        let handle = zone_handle.single();
        let mat = zone_mats
            .get_mut(&handle.handle)
            .expect("Handle must be valid");
        match ev {
            SelectedGroupsChanged::Selected(ent) => {
                let (group,) = zones.get(*ent).expect("All references must be valid");
                mat.highlight_zone_id = group.id as u32;
            }
            SelectedGroupsChanged::Deselected(ent) => {
                let (group,) = zones.get(*ent).expect("All references must be valid");
                if mat.highlight_zone_id == group.id as u32 {
                    mat.highlight_zone_id = 0;
                }
            }
        }
    }
}
