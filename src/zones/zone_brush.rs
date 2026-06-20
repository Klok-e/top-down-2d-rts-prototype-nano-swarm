use bevy::{
    asset::Asset,
    math::{ivec2, vec2},
    prelude::{
        Assets, ButtonInput, Camera, GlobalTransform, Handle, IVec2, MouseButton, Query, Res,
        ResMut, Vec2, Window,
    },
    reflect::TypePath,
    render::{
        render_resource::{AsBindGroup, ShaderType},
        storage::ShaderStorageBuffer,
    },
    shader::ShaderRef,
    sprite_render::Material2d,
};

use crate::{
    intent::{BrushSelection, IntentGrid, IntentKind},
    ui::UiHandling,
    ZONE_BLOCK_SIZE,
};

/// Per-cell data uploaded to the zone shader storage buffer. The shader still
/// uses a 4-bit colour mask + 14-bit id layout; the id bits are written as
/// zero because the swarm-owned intent resource has no per-cell ids.
#[derive(AsBindGroup, Asset, TypePath, Debug, Clone)]
pub struct ZoneMaterial {
    #[storage(2, read_only)]
    pub zone_map: Handle<ShaderStorageBuffer>,
    pub zone_data: Vec<ZonePointData>,
    #[uniform(3)]
    pub width: u32,
    #[uniform(4)]
    pub height: u32,
    #[uniform(1)]
    pub highlight_zone_id: u32,
}

impl ZoneMaterial {
    pub fn new(width: u32, height: u32, buffers: &mut Assets<ShaderStorageBuffer>) -> ZoneMaterial {
        let zone_data = vec![ZonePointData::new(); (width * height) as usize];
        ZoneMaterial {
            zone_map: buffers.add(ShaderStorageBuffer::from(zone_data.clone())),
            zone_data,
            width,
            height,
            highlight_zone_id: 0,
        }
    }

    pub fn at_zone_mut(&mut self, x: u32, y: u32) -> Option<&mut ZonePointData> {
        if x >= self.width || y >= self.height {
            None
        } else {
            Some(&mut self.zone_data[(y * self.width + x) as usize])
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

impl Default for ZonePointData {
    fn default() -> Self {
        Self::new()
    }
}

impl ZonePointData {
    // Definitions
    pub const ZONE1: u32 = 1 << 0;
    pub const ZONE2: u32 = 1 << 1;
    pub const ZONE3: u32 = 1 << 2;
    pub const ZONE4: u32 = 1 << 3;

    pub const ZONE_ID_MASK: u32 = (1 << 14) - 1;

    pub fn id_to_zone(id: u32) -> u32 {
        1 << (id % 4)
    }

    pub fn new() -> Self {
        ZonePointData { zones: 0, bits: 0 }
    }

    pub fn set_zone(&mut self, zone: u32, active: bool) {
        if active {
            self.zones |= zone;
        } else {
            self.zones &= !zone;
        }
    }

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
        let id = id & Self::ZONE_ID_MASK;
        match zone {
            Self::ZONE1 => {
                self.zones &= !(Self::ZONE_ID_MASK << 4);
                self.zones |= id << 4;
            }
            Self::ZONE2 => {
                self.zones &= !(Self::ZONE_ID_MASK << 18);
                self.zones |= id << 18;
            }
            Self::ZONE3 => {
                self.bits &= !Self::ZONE_ID_MASK;
                self.bits |= id;
            }
            Self::ZONE4 => {
                self.bits &= !(Self::ZONE_ID_MASK << 14);
                self.bits |= id << 14;
            }
            _ => panic!("Invalid zone"),
        }
    }
}

impl Material2d for ZoneMaterial {
    fn fragment_shader() -> ShaderRef {
        "shaders/zone_shader.wgsl".into()
    }
}

#[derive(Debug, bevy::prelude::Component)]
pub struct ZoneMaterialHandleComponent {
    pub handle: Handle<ZoneMaterial>,
}

/// Per-frame paint delta. Held mouse input that calls
/// [`IntentGrid::paint`] every frame accumulates strength up to
/// `PAINT_STRENGTH_CAP` defined in [`crate::intent`].
const BRUSH_PAINT_DELTA: u8 = 1;

/// Reads mouse input and writes player intent into the [`IntentGrid`]
/// resource for the layer currently selected in [`BrushSelection`]. The
/// simulation owns the grid; the GPU zone material is a downstream mirror of
/// the resource, updated by [`mirror_intent_to_zone_material_system`].
pub fn zone_brush_system(
    windows: Query<&Window>,
    mouse_button_input: Res<ButtonInput<MouseButton>>,
    brush_selection: Res<BrushSelection>,
    ui_handling: Res<UiHandling>,
    camera_query: Query<(&GlobalTransform, &Camera)>,
    mut intent_grid: ResMut<IntentGrid>,
) {
    if ui_handling.is_pointer_over_ui {
        return;
    }

    let Ok(window) = windows.single() else {
        return;
    };
    let Some(cursor_pos) = window.cursor_position() else {
        return;
    };

    let Ok((camera_transform, camera)) = camera_query.single() else {
        return;
    };
    let cursor_pos_world =
        if let Ok(pos) = camera.viewport_to_world_2d(camera_transform, cursor_pos) {
            pos
        } else {
            return;
        };

    let idx = get_zone_pos_from_world(cursor_pos_world);
    let half_w = intent_grid.width() / 2;
    let half_h = intent_grid.height() / 2;
    if idx.x < -half_w || idx.x >= half_w || idx.y < -half_h || idx.y >= half_h {
        return;
    }

    let brush_kind = brush_selection.kind;
    if mouse_button_input.pressed(MouseButton::Left) {
        intent_grid.paint(idx, brush_kind, BRUSH_PAINT_DELTA);
    } else if mouse_button_input.pressed(MouseButton::Right) {
        intent_grid.erase(idx, brush_kind, BRUSH_PAINT_DELTA);
    }
}

/// Drains dirty cells from [`IntentGrid`] and mirrors them into the
/// [`ZoneMaterial`] GPU buffer. Pure data flow: the resource is the source of
/// truth, the GPU buffer is a read-only render view.
pub fn mirror_intent_to_zone_material_system(
    mut zone_mats: ResMut<Assets<ZoneMaterial>>,
    mut buffers: ResMut<Assets<ShaderStorageBuffer>>,
    zone_handle: Query<&ZoneMaterialHandleComponent>,
    mut intent_grid: ResMut<IntentGrid>,
) {
    let Ok(handle) = zone_handle.single() else {
        return;
    };
    let dirty = intent_grid.drain_dirty();
    if dirty.is_empty() {
        return;
    }
    // Single GPU upload per frame: snapshot the material, mutate, push.
    let mat = zone_mats
        .get_mut(&handle.handle)
        .expect("Zone material handle must be valid");

    for point in dirty {
        let Some(idx) =
            zone_buffer_index_from_grid_point(point, intent_grid.width(), intent_grid.height())
        else {
            continue;
        };

        let cell = intent_grid
            .cell(point)
            .expect("dirty point must be in-bounds");

        if let Some(zone_data) = mat.at_zone_mut(idx.x as u32, idx.y as u32) {
            for kind in IntentKind::ALL {
                let zone_color = ZonePointData::id_to_zone(kind.index() as u32);
                zone_data.set_zone(zone_color, cell.has(kind));
                // The render mirror never assigns id bits; clear them so any
                // stale value from prior frames is not shown.
                zone_data.set_zone_id(zone_color, 0);
            }
        }
    }

    let zone_map = mat.zone_map.clone();
    let zone_data = mat.zone_data.clone();
    if let Some(buffer) = buffers.get_mut(&zone_map) {
        buffer.set_data(zone_data);
    }
}

fn zone_buffer_index_from_grid_point(point: IVec2, width: i32, height: i32) -> Option<IVec2> {
    let half = ivec2(width / 2, height / 2);
    let mut idx = point + half;
    idx.y = height - idx.y - 1;
    (idx.x >= 0 && idx.x < width && idx.y >= 0 && idx.y < height).then_some(idx)
}

pub fn get_zone_pos_from_world(world_pos: Vec2) -> IVec2 {
    vec2(
        (world_pos.x / ZONE_BLOCK_SIZE).floor(),
        (world_pos.y / ZONE_BLOCK_SIZE).floor(),
    )
    .as_ivec2()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zone_ids_do_not_corrupt_each_other() {
        let mut point = ZonePointData::new();

        point.set_zone_id(ZonePointData::ZONE1, 17);
        point.set_zone_id(ZonePointData::ZONE2, 42);
        point.set_zone_id(ZonePointData::ZONE3, 99);
        point.set_zone_id(ZonePointData::ZONE4, ZonePointData::ZONE_ID_MASK);

        assert_eq!(point.get_zone_id(ZonePointData::ZONE1), 17);
        assert_eq!(point.get_zone_id(ZonePointData::ZONE2), 42);
        assert_eq!(point.get_zone_id(ZonePointData::ZONE3), 99);
        assert_eq!(
            point.get_zone_id(ZonePointData::ZONE4),
            ZonePointData::ZONE_ID_MASK
        );
    }

    #[test]
    fn centered_intent_points_map_to_gpu_buffer_indices() {
        let grid = IntentGrid::new(crate::MAP_WIDTH as i32, crate::MAP_HEIGHT as i32);

        assert_eq!(
            zone_buffer_index_from_grid_point(IVec2::new(-500, -500), grid.width(), grid.height()),
            Some(IVec2::new(0, 999))
        );
        assert_eq!(
            zone_buffer_index_from_grid_point(IVec2::new(499, 499), grid.width(), grid.height()),
            Some(IVec2::new(999, 0))
        );
        assert_eq!(
            zone_buffer_index_from_grid_point(IVec2::new(500, 0), grid.width(), grid.height()),
            None
        );
    }
}
