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
    intent::{BrushSelection, IntentCell, IntentGrid, IntentKind},
    nanobot::{MatchOutcome, SwarmId},
    ui::UiHandling,
};

/// Per-cell presence and ownership bits uploaded to the zone shader.
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
    /// Packed presence and ownership bits in [`IntentKind::index`] order.
    pub active: u32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[repr(u32)]
pub enum ZoneOwnership {
    #[default]
    Shared = 0,
    Player = 1,
    Opponent = 2,
    Contested = 3,
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

    pub fn set_ownership(&mut self, kind_index: u32, ownership: ZoneOwnership) {
        assert!(
            kind_index < IntentKind::COUNT as u32,
            "kind_index out of range"
        );
        let shift = IntentKind::COUNT as u32 + kind_index * 2;
        self.active = (self.active & !(0b11 << shift)) | ((ownership as u32) << shift);
    }

    pub fn ownership(&self, kind_index: u32) -> ZoneOwnership {
        assert!(
            kind_index < IntentKind::COUNT as u32,
            "kind_index out of range"
        );
        let shift = IntentKind::COUNT as u32 + kind_index * 2;
        match (self.active >> shift) & 0b11 {
            0 => ZoneOwnership::Shared,
            1 => ZoneOwnership::Player,
            2 => ZoneOwnership::Opponent,
            3 => ZoneOwnership::Contested,
            _ => unreachable!(),
        }
    }
}

fn zone_ownership(cell: &IntentCell, kind: IntentKind, contested_defend: bool) -> ZoneOwnership {
    if !cell.has(kind) {
        ZoneOwnership::Shared
    } else if kind == IntentKind::Defend && contested_defend {
        ZoneOwnership::Contested
    } else {
        match cell.owner(kind) {
            Some(SwarmId::PLAYER) => ZoneOwnership::Player,
            Some(_) => ZoneOwnership::Opponent,
            None => ZoneOwnership::Shared,
        }
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayerIntentAction {
    Paint,
    Erase,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PlayerIntentError {
    #[error("the match is already complete")]
    MatchFinished,
    #[error("intent cell is outside the map")]
    OutOfBounds,
}

/// Apply one player-owned intent edit using the same ownership and Defend
/// contest rules regardless of whether it originated from mouse or agent input.
pub fn apply_player_intent(
    intent_grid: &mut IntentGrid,
    outcome: MatchOutcome,
    cell: IVec2,
    kind: IntentKind,
    action: PlayerIntentAction,
) -> Result<bool, PlayerIntentError> {
    if outcome != MatchOutcome::InProgress {
        return Err(PlayerIntentError::MatchFinished);
    }
    let before = intent_grid
        .cell(cell)
        .copied()
        .ok_or(PlayerIntentError::OutOfBounds)?;
    let contest_before = intent_grid.defend_contest(cell);

    match action {
        PlayerIntentAction::Paint if kind == IntentKind::Defend => {
            intent_grid.contest_defend(cell, SwarmId::PLAYER);
        }
        PlayerIntentAction::Paint => {
            intent_grid.paint_owned_if_available(cell, kind, Some(SwarmId::PLAYER));
        }
        PlayerIntentAction::Erase
            if kind == IntentKind::Defend
                && intent_grid.withdraw_defend_contest(cell, SwarmId::PLAYER) => {}
        PlayerIntentAction::Erase => {
            intent_grid.erase_owned(cell, kind, Some(SwarmId::PLAYER));
        }
    }

    Ok(
        before != *intent_grid.cell(cell).expect("validated intent cell")
            || contest_before != intent_grid.defend_contest(cell),
    )
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
    outcome: Option<Res<MatchOutcome>>,
    camera_query: Query<(&GlobalTransform, &Camera)>,
    mut intent_grid: ResMut<IntentGrid>,
) {
    if outcome
        .as_deref()
        .is_some_and(|outcome| *outcome != MatchOutcome::InProgress)
    {
        return;
    }
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

    let action = if mouse_button_input.pressed(MouseButton::Left) {
        PlayerIntentAction::Paint
    } else if mouse_button_input.pressed(MouseButton::Right) {
        PlayerIntentAction::Erase
    } else {
        return;
    };
    let _ = apply_player_intent(
        &mut intent_grid,
        outcome.as_deref().copied().unwrap_or_default(),
        get_zone_pos_from_world(cursor_pos_world),
        brush_selection.kind,
        action,
    );
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
        let contested_defend = intent_grid.defend_contest(point).is_some();

        if let Some(zone_data) = mat.at_zone_mut(idx.x as u32, idx.y as u32) {
            for kind in IntentKind::ALL {
                zone_data.set_present(kind.index() as u32, cell.has(kind));
                zone_data.set_ownership(
                    kind.index() as u32,
                    zone_ownership(cell, kind, contested_defend),
                );
            }
        }
    }

    let zone_map = mat.zone_map.clone();
    let packed_zone_data = mat
        .zone_data
        .iter()
        .map(|cell| cell.active)
        .collect::<Vec<_>>();
    buffers
        .get_mut(&zone_map)
        .expect("zone storage buffer handle must remain valid")
        .set_data(packed_zone_data);
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
    fn ownership_bits_do_not_corrupt_presence_or_other_layers() {
        let mut point = ZonePointData::new();
        point.set_present(IntentKind::Gather.index() as u32, true);
        point.set_present(IntentKind::Defend.index() as u32, true);
        point.set_ownership(IntentKind::Gather.index() as u32, ZoneOwnership::Player);
        point.set_ownership(IntentKind::Defend.index() as u32, ZoneOwnership::Contested);

        assert!(point.present(IntentKind::Gather.index() as u32));
        assert!(point.present(IntentKind::Defend.index() as u32));
        assert_eq!(
            point.ownership(IntentKind::Gather.index() as u32),
            ZoneOwnership::Player
        );
        assert_eq!(
            point.ownership(IntentKind::Defend.index() as u32),
            ZoneOwnership::Contested
        );
        assert_eq!(
            point.ownership(IntentKind::Build.index() as u32),
            ZoneOwnership::Shared
        );
    }

    #[test]
    fn render_ownership_distinguishes_player_opponent_shared_and_contested() {
        let mut player = IntentCell::default();
        player.add_owned(IntentKind::Defend, Some(SwarmId::PLAYER));
        let mut opponent = IntentCell::default();
        opponent.add_owned(IntentKind::Defend, Some(SwarmId(9)));
        let mut shared = IntentCell::default();
        shared.add(IntentKind::Defend);

        assert_eq!(
            zone_ownership(&player, IntentKind::Defend, false),
            ZoneOwnership::Player
        );
        assert_eq!(
            zone_ownership(&opponent, IntentKind::Defend, false),
            ZoneOwnership::Opponent
        );
        assert_eq!(
            zone_ownership(&shared, IntentKind::Defend, false),
            ZoneOwnership::Shared
        );
        assert_eq!(
            zone_ownership(&shared, IntentKind::Defend, true),
            ZoneOwnership::Contested
        );
    }

    #[test]
    fn player_defend_action_contests_and_withdraws_from_hostile_paint() {
        let mut grid = IntentGrid::new(5, 5);
        let cell = ivec2(1, -1);
        let opponent = SwarmId(9);
        grid.paint_owned(cell, IntentKind::Defend, Some(opponent));

        assert_eq!(
            apply_player_intent(
                &mut grid,
                MatchOutcome::InProgress,
                cell,
                IntentKind::Defend,
                PlayerIntentAction::Paint,
            ),
            Ok(true)
        );
        assert_eq!(grid.cell(cell).unwrap().owner(IntentKind::Defend), None);
        assert_eq!(
            grid.defend_contests(),
            vec![(cell, opponent, SwarmId::PLAYER)]
        );

        assert_eq!(
            apply_player_intent(
                &mut grid,
                MatchOutcome::InProgress,
                cell,
                IntentKind::Defend,
                PlayerIntentAction::Erase,
            ),
            Ok(true)
        );
        assert_eq!(
            grid.cell(cell).unwrap().owner(IntentKind::Defend),
            Some(opponent)
        );
        assert!(grid.defend_contests().is_empty());
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
