use bevy::{
    asset::Asset,
    math::{ivec2, vec2},
    prelude::{
        Assets, ButtonInput, Camera, GlobalTransform, Handle, IVec2, MouseButton, Query, Res,
        ResMut, Vec2, Window,
    },
    reflect::TypePath,
    render::{render_resource::AsBindGroup, storage::ShaderStorageBuffer},
    shader::ShaderRef,
    sprite_render::{AlphaMode2d, Material2d},
};

use crate::{
    ZONE_BLOCK_SIZE,
    intent::{BrushSelection, IntentGrid, IntentKind},
    nanobot::SwarmId,
    ui::UiHandling,
};

/// Per-cell presence bits uploaded to zone shader storage buffer.
#[derive(AsBindGroup, Asset, TypePath, Debug, Clone)]
pub struct ZoneMaterial {
    #[storage(2, read_only)]
    pub zone_map: Handle<ShaderStorageBuffer>,
    pub zone_data: Vec<ZonePointData>,
    #[uniform(3)]
    pub width: u32,
    #[uniform(4)]
    pub height: u32,
}

impl ZoneMaterial {
    pub fn new(width: u32, height: u32, buffers: &mut Assets<ShaderStorageBuffer>) -> ZoneMaterial {
        let zone_data = vec![ZonePointData::new(); (width * height) as usize];
        ZoneMaterial {
            zone_map: buffers.add(ShaderStorageBuffer::from(vec![0u32; zone_data.len()])),
            zone_data,
            width,
            height,
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

#[derive(Debug, Clone, Copy)]
pub struct ZonePointData {
    /// Presence bits in [`IntentKind::index`] order.
    pub active: u32,
}

impl Default for ZonePointData {
    fn default() -> Self {
        Self::new()
    }
}

impl ZonePointData {
    pub fn new() -> Self {
        Self { active: 0 }
    }

    /// Set one intent-kind presence bit.
    pub fn set_present(&mut self, kind_index: u32, present: bool) {
        assert!(
            kind_index < IntentKind::COUNT as u32,
            "kind_index out of range"
        );
        let bit = 1 << kind_index;
        if present {
            self.active |= bit;
        } else {
            self.active &= !bit;
        }
    }

    /// Read one intent-kind presence bit.
    pub fn present(&self, kind_index: u32) -> bool {
        assert!(
            kind_index < IntentKind::COUNT as u32,
            "kind_index out of range"
        );
        (self.active & (1 << kind_index)) != 0
    }
}

impl Material2d for ZoneMaterial {
    fn fragment_shader() -> ShaderRef {
        "shaders/zone_shader.wgsl".into()
    }

    fn alpha_mode(&self) -> AlphaMode2d {
        AlphaMode2d::Blend
    }
}

#[derive(Debug, bevy::prelude::Component)]
pub struct ZoneMaterialHandleComponent {
    pub handle: Handle<ZoneMaterial>,
}

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
        intent_grid.paint_owned_if_available(idx, brush_kind, Some(SwarmId::PLAYER));
    } else if mouse_button_input.pressed(MouseButton::Right) {
        intent_grid.erase_owned(idx, brush_kind, Some(SwarmId::PLAYER));
    }
}

/// Drains render-dirty cells from [`IntentGrid`] and mirrors them into the
/// [`ZoneMaterial`] GPU buffer. Projection dirty state remains available to
/// simulation consumers.
pub fn mirror_intent_to_zone_material_system(
    mut zone_mats: ResMut<Assets<ZoneMaterial>>,
    mut buffers: ResMut<Assets<ShaderStorageBuffer>>,
    zone_handle: Query<&ZoneMaterialHandleComponent>,
    mut intent_grid: ResMut<IntentGrid>,
) {
    let Ok(handle) = zone_handle.single() else {
        return;
    };
    let dirty = intent_grid.drain_render_dirty();
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
                zone_data.set_present(kind.index() as u32, cell.has(kind));
            }
        }
    }

    let zone_map = mat.zone_map.clone();
    let packed_presence = mat
        .zone_data
        .iter()
        .map(|cell| cell.active)
        .collect::<Vec<_>>();
    buffers
        .get_mut(&zone_map)
        .expect("zone storage buffer handle must remain valid")
        .set_data(packed_presence);
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
    fn presence_bits_do_not_corrupt_each_other() {
        let mut point = ZonePointData::new();
        point.set_present(0, true);
        point.set_present(2, true);

        assert!(point.present(0));
        assert!(!point.present(1));
        assert!(point.present(2));
        assert!(!point.present(3));

        point.set_present(0, false);
        assert!(!point.present(0));
        assert!(point.present(2));
    }

    #[test]
    fn presence_storage_buffer_has_one_u32_per_cell() {
        let buffer = ShaderStorageBuffer::from(vec![1u32, 2, 4, 8]);
        assert_eq!(
            buffer.data.unwrap(),
            [1u32, 2, 4, 8]
                .into_iter()
                .flat_map(u32::to_ne_bytes)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn empty_point_reports_every_kind_absent() {
        let point = ZonePointData::new();
        for kind_index in 0..IntentKind::COUNT as u32 {
            assert!(!point.present(kind_index));
        }
    }

    #[test]
    fn zone_material_uses_alpha_blending_so_empty_pixels_show_background() {
        let mut buffers = Assets::<ShaderStorageBuffer>::default();
        let material = ZoneMaterial::new(2, 2, &mut buffers);

        assert_eq!(material.alpha_mode(), AlphaMode2d::Blend);
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
