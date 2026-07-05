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
    sprite_render::{AlphaMode2d, Material2d},
};

use crate::{
    intent::{BrushSelection, IntentGrid, IntentKind, PAINT_STRENGTH_CAP},
    nanobot::SwarmId,
    ui::UiHandling,
    ZONE_BLOCK_SIZE,
};

/// Per-cell data uploaded to the zone shader storage buffer. Each cell packs
/// four 5-bit paint-strength slots (one per [`IntentKind`]) into a single
/// `u32`. A slot value of `0` means the kind is absent; a non-zero value is
/// the paint strength in `[1, PAINT_STRENGTH_CAP]`. The mirror system copies
/// [`IntentCell::strength`] into these slots and the zone shader maps each
/// strength to an overlay alpha.
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
            zone_map: buffers.add(ShaderStorageBuffer::from(zone_data.clone())),
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

/// Number of strength slots packed into [`ZonePointData`]. Slot order matches
/// [`IntentKind::index`], but the packed render data stays index-based.
const ZONE_STRENGTH_SLOT_COUNT: u32 = 4;

/// Number of bits used to store one kind's paint strength in [`ZonePointData`].
/// A strength in `[0, PAINT_STRENGTH_CAP]` (`PAINT_STRENGTH_CAP = 16`) fits in
/// 5 bits with headroom to spare, so the four slots occupy 20 of the 32 bits.
const ZONE_STRENGTH_SLOT_BITS: u32 = 5;

/// Bit mask covering a single 5-bit strength slot.
const ZONE_STRENGTH_SLOT_MASK: u32 = (1u32 << ZONE_STRENGTH_SLOT_BITS) - 1;

#[derive(Debug, Clone, Copy, ShaderType)]
pub struct ZonePointData {
    /// Packed paint strength for each kind. Slot `i` occupies bits
    /// `[5*i, 5*i+5)` and stores the strength in
    /// `[0, PAINT_STRENGTH_CAP]`. Slot order matches [`IntentKind::index`]:
    /// 0 = Gather, 1 = Build, 2 = Defend, 3 = Corridor. The top 12 bits are
    /// spare.
    pub strength: u32,
}

impl Default for ZonePointData {
    fn default() -> Self {
        Self::new()
    }
}

impl ZonePointData {
    pub fn new() -> Self {
        ZonePointData { strength: 0 }
    }

    /// Write the paint strength of kind `kind_index` into its 5-bit slot.
    /// `kind_index` is `IntentKind::index()` (`0..4`); out-of-range indices
    /// panic. Strengths above `PAINT_STRENGTH_CAP` are clamped to it so the
    /// slot can never exceed the shader's alpha ramp ceiling.
    pub fn set_strength(&mut self, kind_index: u32, strength: u8) {
        assert!(
            kind_index < ZONE_STRENGTH_SLOT_COUNT,
            "kind_index out of range"
        );
        let clamped = (strength as u32).min(PAINT_STRENGTH_CAP as u32);
        let shift = kind_index * ZONE_STRENGTH_SLOT_BITS;
        self.strength &= !(ZONE_STRENGTH_SLOT_MASK << shift);
        self.strength |= clamped << shift;
    }

    /// Read the paint strength stored for kind `kind_index`, or `0` if the
    /// kind is absent at this cell. `kind_index` is `IntentKind::index()`
    /// (`0..4`); out-of-range indices panic.
    pub fn strength(&self, kind_index: u32) -> u8 {
        assert!(
            kind_index < ZONE_STRENGTH_SLOT_COUNT,
            "kind_index out of range"
        );
        let shift = kind_index * ZONE_STRENGTH_SLOT_BITS;
        ((self.strength >> shift) & ZONE_STRENGTH_SLOT_MASK) as u8
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
        // The player brush always writes with the player
        // `SwarmId` so the per-swarm intent filter routes
        // player paint to player workers (and never to
        // opponent workers). Without this stamp a player
        // brush would write unowned paint that opponent
        // workers could also see.
        intent_grid.paint_owned(idx, brush_kind, BRUSH_PAINT_DELTA, Some(SwarmId::PLAYER));
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
                // `IntentCell::strength` returns 0 for an inactive kind, so a
                // single write per kind both clears absent layers (slot 0)
                // and mirrors the active strength. The setter clamps to
                // PAINT_STRENGTH_CAP as a defensive ceiling.
                zone_data.set_strength(kind.index() as u32, cell.strength(kind));
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
    fn strength_slots_do_not_corrupt_each_other() {
        // Each of the four 5-bit slots is independent: writing one must
        // never disturb another. Cover the full kind range with distinct
        // values including the paint-strength cap, then read every slot
        // back. This pins the packed layout the zone shader reads.
        let mut point = ZonePointData::new();

        point.set_strength(0, 1);
        point.set_strength(1, 4);
        point.set_strength(2, 8);
        point.set_strength(3, PAINT_STRENGTH_CAP);

        assert_eq!(point.strength(0), 1);
        assert_eq!(point.strength(1), 4);
        assert_eq!(point.strength(2), 8);
        assert_eq!(point.strength(3), PAINT_STRENGTH_CAP);
    }

    #[test]
    fn empty_point_reports_zero_strength_for_every_kind() {
        let point = ZonePointData::new();
        for kind_index in 0..IntentKind::COUNT as u32 {
            assert_eq!(
                point.strength(kind_index),
                0,
                "freshly-created point must report strength 0 for every kind"
            );
        }
    }

    #[test]
    fn set_strength_clamps_above_cap_so_slot_never_overflows() {
        // The 5-bit slot could hold up to 31, but the paint contract caps
        // strength at PAINT_STRENGTH_CAP. The setter must enforce that cap
        // so a stray caller cannot push the alpha ramp past its ceiling.
        let mut point = ZonePointData::new();
        point.set_strength(2, 250);
        assert_eq!(point.strength(2), PAINT_STRENGTH_CAP);
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
