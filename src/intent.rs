//! Swarm-owned player intent data.
//!
//! The [`IntentGrid`] resource is the simulation-side source of truth for player
//! intent paint. It is plain Rust data with no dependency on Bevy rendering or
//! shader storage buffers. The GPU zone material reads from this resource via a
//! mirror system; the resource itself never reads from rendering.

use std::collections::HashSet;

use bevy::{
    input::{keyboard::KeyCode, ButtonInput},
    prelude::{IVec2, Res, ResMut, Resource},
};

use crate::nanobot::SwarmId;

/// Player intent kinds. Declaration order matches zone overlay colour slots, so
/// [`IntentKind::index`] is stable cross-module layer key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IntentKind {
    Gather,
    Build,
    Defend,
    Corridor,
}

impl IntentKind {
    /// Number of distinct intent kinds. Equal to the number of intent layers that
    /// can coexist at a single map cell.
    pub const COUNT: usize = 4;

    /// All intent kinds in stable shader-slot order.
    pub const ALL: [IntentKind; Self::COUNT] = [
        IntentKind::Gather,
        IntentKind::Build,
        IntentKind::Defend,
        IntentKind::Corridor,
    ];

    /// Stable per-kind index in `[0, COUNT)`. Used to address per-layer data
    /// inside [`IntentCell`].
    pub const fn index(self) -> usize {
        match self {
            IntentKind::Gather => 0,
            IntentKind::Build => 1,
            IntentKind::Defend => 2,
            IntentKind::Corridor => 3,
        }
    }

    /// Bit flag for this kind inside an [`IntentCell::active`] bitmask.
    pub const fn bit(self) -> u8 {
        1 << (self.index() as u8)
    }
}

/// One active intent layer at a cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IntentLayer {
    pub kind: IntentKind,
}

/// Multiple intent layers at one cell. `active` is a bitmask of
/// [`IntentKind::bit`] flags. Ownership is stored independently per kind, so
/// overlapping kinds can belong to different swarms.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct IntentCell {
    pub active: u8,
    pub owner: [Option<SwarmId>; IntentKind::COUNT],
}

impl IntentCell {
    /// True when no intent layer is active at this cell.
    pub fn is_empty(&self) -> bool {
        self.active == 0
    }

    /// True when given intent kind is active at this cell.
    pub fn has(&self, kind: IntentKind) -> bool {
        (self.active & kind.bit()) != 0
    }

    /// Activate `kind` as unowned paint.
    pub fn add(&mut self, kind: IntentKind) {
        self.add_owned(kind, None);
    }

    /// Activate `kind` and stamp its owner. Returns whether state changed.
    pub fn add_owned(&mut self, kind: IntentKind, owner: Option<SwarmId>) -> bool {
        let changed = !self.has(kind) || self.owner[kind.index()] != owner;
        self.active |= kind.bit();
        self.owner[kind.index()] = owner;
        changed
    }

    /// Deactivate `kind` and clear its owner. Returns whether state changed.
    pub fn remove(&mut self, kind: IntentKind) -> bool {
        if !self.has(kind) {
            return false;
        }
        self.active &= !kind.bit();
        self.owner[kind.index()] = None;
        true
    }

    /// Owner of active `kind`, or `None` for unowned or absent paint.
    pub fn owner(&self, kind: IntentKind) -> Option<SwarmId> {
        self.has(kind).then(|| self.owner[kind.index()]).flatten()
    }

    /// True when `kind` is visible to `swarm`. Unowned paint is shared.
    pub fn visible_to(&self, kind: IntentKind, swarm: SwarmId) -> bool {
        self.has(kind) && self.owner(kind).is_none_or(|owner| owner == swarm)
    }

    /// Iterate active intent layers in declaration order.
    pub fn iter_layers(&self) -> impl Iterator<Item = IntentLayer> + '_ {
        IntentKind::ALL
            .into_iter()
            .filter(|&kind| self.has(kind))
            .map(|kind| IntentLayer { kind })
    }
}

/// Swarm-owned simulation state for player intent. Plain Rust, no rendering
/// dependencies. Inserted as a Bevy [`Resource`] so simulation systems can read
/// and write it without going through any GPU buffer or zone material.
#[derive(Debug, Clone, Resource)]
pub struct IntentGrid {
    width: i32,
    height: i32,
    cells: Vec<IntentCell>,
    /// Cells awaiting render-mirror consumption.
    render_dirty: HashSet<IVec2>,
    /// Cells awaiting actionable-projection consumption.
    projection_dirty: HashSet<IVec2>,
}

impl IntentGrid {
    /// Build a new grid of `width` x `height` empty cells.
    pub fn new(width: i32, height: i32) -> Self {
        let size = (width.max(0) as usize) * (height.max(0) as usize);
        Self {
            width: width.max(0),
            height: height.max(0),
            cells: vec![IntentCell::default(); size],
            render_dirty: HashSet::new(),
            projection_dirty: HashSet::new(),
        }
    }

    pub fn width(&self) -> i32 {
        self.width
    }

    pub fn height(&self) -> i32 {
        self.height
    }

    /// True when `point` falls inside the grid bounds.
    ///
    /// Intent grid coordinates are world-aligned zone-cell coordinates, centered
    /// around `(0, 0)`. For example, a `4 x 4` grid spans `x/y = -2..2`, and a
    /// `3 x 3` grid spans `x/y = -1..2`.
    pub fn in_bounds(&self, point: IVec2) -> bool {
        let min = self.origin_min();
        let max = IVec2::new(min.x + self.width, min.y + self.height);
        point.x >= min.x && point.x < max.x && point.y >= min.y && point.y < max.y
    }

    /// Read the cell at `point`, or `None` if `point` is out of bounds.
    pub fn cell(&self, point: IVec2) -> Option<&IntentCell> {
        if !self.in_bounds(point) {
            None
        } else {
            Some(&self.cells[self.index(point)])
        }
    }

    /// Set unowned `kind` intent at `point`. Returns whether point is in bounds.
    pub fn add(&mut self, point: IVec2, kind: IntentKind) -> bool {
        self.set_owned(point, kind, None)
    }

    /// Set `kind` intent and owner at `point`. Returns whether point is in bounds.
    pub fn add_owned(&mut self, point: IVec2, kind: IntentKind, owner: Option<SwarmId>) -> bool {
        self.set_owned(point, kind, owner)
    }

    /// Clear `kind` intent at `point`. Returns whether point is in bounds.
    pub fn remove(&mut self, point: IVec2, kind: IntentKind) -> bool {
        if !self.in_bounds(point) {
            return false;
        }
        let idx = self.index(point);
        if self.cells[idx].remove(kind) {
            self.mark_dirty(point);
        }
        true
    }

    /// Paint unowned `kind` at `point`. Repeated paint is a no-op.
    pub fn paint(&mut self, point: IVec2, kind: IntentKind) -> bool {
        self.set_owned(point, kind, None)
    }

    /// Paint owned `kind` at `point`. Repeated paint by same owner is a no-op.
    pub fn paint_owned(&mut self, point: IVec2, kind: IntentKind, owner: Option<SwarmId>) -> bool {
        self.set_owned(point, kind, owner)
    }

    /// Erase `kind` at `point` immediately. Erasing absent paint is a no-op.
    pub fn erase(&mut self, point: IVec2, kind: IntentKind) -> bool {
        self.remove(point, kind)
    }

    /// Erase `kind` only when its active paint belongs to `owner`.
    pub fn erase_owned(&mut self, point: IVec2, kind: IntentKind, owner: Option<SwarmId>) -> bool {
        if !self.in_bounds(point) {
            return false;
        }
        if self.cells[self.index(point)].owner(kind) == owner {
            self.remove(point, kind);
        }
        true
    }

    /// Number of changed cells awaiting the render mirror.
    pub fn render_dirty_count(&self) -> usize {
        self.render_dirty.len()
    }

    /// Number of changed cells awaiting actionable projection.
    pub fn projection_dirty_count(&self) -> usize {
        self.projection_dirty.len()
    }

    /// Drain changed cells for the render mirror in deterministic `(y, x)` order.
    pub fn drain_render_dirty(&mut self) -> Vec<IVec2> {
        drain_sorted(&mut self.render_dirty)
    }

    /// Drain changed cells for actionable projection in deterministic `(y, x)` order.
    pub fn drain_projection_dirty(&mut self) -> Vec<IVec2> {
        drain_sorted(&mut self.projection_dirty)
    }

    /// Iterate every cell in row-major order. Useful for systems that need to
    /// read the full grid state.
    pub fn iter_cells(&self) -> impl Iterator<Item = (IVec2, &IntentCell)> {
        let w = self.width;
        let min = self.origin_min();
        self.cells.iter().enumerate().map(move |(i, cell)| {
            let x = min.x + (i as i32) % w;
            let y = min.y + (i as i32) / w;
            (IVec2::new(x, y), cell)
        })
    }

    fn set_owned(&mut self, point: IVec2, kind: IntentKind, owner: Option<SwarmId>) -> bool {
        if !self.in_bounds(point) {
            return false;
        }
        let idx = self.index(point);
        if self.cells[idx].add_owned(kind, owner) {
            self.mark_dirty(point);
        }
        true
    }

    fn mark_dirty(&mut self, point: IVec2) {
        self.render_dirty.insert(point);
        self.projection_dirty.insert(point);
    }

    fn index(&self, point: IVec2) -> usize {
        let min = self.origin_min();
        let local = point - min;
        (local.y as usize) * (self.width as usize) + (local.x as usize)
    }

    fn origin_min(&self) -> IVec2 {
        IVec2::new(-(self.width / 2), -(self.height / 2))
    }
}

fn drain_sorted(dirty: &mut HashSet<IVec2>) -> Vec<IVec2> {
    let mut points: Vec<IVec2> = dirty.drain().collect();
    points.sort_by_key(|point| (point.y, point.x));
    points
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_grid_has_no_active_layers() {
        let grid = IntentGrid::new(4, 4);
        let cell = grid.cell(IVec2::new(1, 1)).unwrap();
        assert!(cell.is_empty());
        assert_eq!(cell.iter_layers().count(), 0);
    }

    #[test]
    fn paint_is_binary_and_repeated_paint_stays_clean() {
        let mut grid = IntentGrid::new(4, 4);
        let point = IVec2::ZERO;

        assert!(grid.paint(point, IntentKind::Gather));
        assert!(grid.cell(point).unwrap().has(IntentKind::Gather));
        assert_eq!(grid.render_dirty_count(), 1);
        assert_eq!(grid.drain_render_dirty(), vec![point]);
        assert_eq!(grid.drain_projection_dirty(), vec![point]);

        assert!(grid.paint(point, IntentKind::Gather));
        assert_eq!(grid.render_dirty_count(), 0);
        assert_eq!(grid.projection_dirty_count(), 0);
    }

    #[test]
    fn ownership_change_marks_binary_layer_dirty() {
        let mut grid = IntentGrid::new(4, 4);
        let point = IVec2::ZERO;
        grid.paint_owned(point, IntentKind::Build, Some(SwarmId::PLAYER));
        grid.drain_render_dirty();
        grid.drain_projection_dirty();

        grid.paint_owned(point, IntentKind::Build, Some(SwarmId(7)));

        let cell = grid.cell(point).unwrap();
        assert_eq!(cell.owner(IntentKind::Build), Some(SwarmId(7)));
        assert_eq!(grid.render_dirty_count(), 1);
        assert_eq!(grid.projection_dirty_count(), 1);
    }

    #[test]
    fn overlapping_kinds_keep_independent_ownership() {
        let mut grid = IntentGrid::new(4, 4);
        let point = IVec2::ZERO;
        grid.paint_owned(point, IntentKind::Gather, Some(SwarmId::PLAYER));
        grid.paint_owned(point, IntentKind::Defend, Some(SwarmId(7)));

        let cell = grid.cell(point).unwrap();
        assert!(cell.has(IntentKind::Gather));
        assert!(cell.has(IntentKind::Defend));
        assert_eq!(cell.owner(IntentKind::Gather), Some(SwarmId::PLAYER));
        assert_eq!(cell.owner(IntentKind::Defend), Some(SwarmId(7)));
    }

    #[test]
    fn erase_clears_selected_kind_immediately_and_stays_clean_when_repeated() {
        let mut grid = IntentGrid::new(4, 4);
        let point = IVec2::ZERO;
        grid.paint(point, IntentKind::Gather);
        grid.paint(point, IntentKind::Corridor);
        grid.drain_render_dirty();
        grid.drain_projection_dirty();

        assert!(grid.erase(point, IntentKind::Gather));
        let cell = grid.cell(point).unwrap();
        assert!(!cell.has(IntentKind::Gather));
        assert!(cell.has(IntentKind::Corridor));
        assert_eq!(grid.drain_render_dirty(), vec![point]);
        assert_eq!(grid.drain_projection_dirty(), vec![point]);

        assert!(grid.erase(point, IntentKind::Gather));
        assert_eq!(grid.render_dirty_count(), 0);
        assert_eq!(grid.projection_dirty_count(), 0);
    }

    #[test]
    fn out_of_bounds_writes_are_rejected() {
        let mut grid = IntentGrid::new(3, 3);
        assert!(!grid.paint(IVec2::new(-2, 0), IntentKind::Gather));
        assert!(!grid.erase(IVec2::new(2, 0), IntentKind::Gather));
    }

    #[test]
    fn dirty_cells_drain_independently_in_deterministic_order() {
        let mut grid = IntentGrid::new(4, 4);
        grid.add(IVec2::new(0, -1), IntentKind::Build);
        grid.add(IVec2::new(-2, -2), IntentKind::Gather);

        let expected = vec![IVec2::new(-2, -2), IVec2::new(0, -1)];
        assert_eq!(grid.drain_render_dirty(), expected);
        assert_eq!(grid.projection_dirty_count(), 2);
        assert_eq!(grid.drain_projection_dirty(), expected);
    }

    #[test]
    fn iter_cells_covers_whole_grid_in_row_major_order() {
        let mut grid = IntentGrid::new(2, 2);
        grid.add(IVec2::ZERO, IntentKind::Defend);
        let seen: Vec<(IVec2, bool)> = grid
            .iter_cells()
            .map(|(point, cell)| (point, cell.has(IntentKind::Defend)))
            .collect();
        assert_eq!(
            seen,
            vec![
                (IVec2::new(-1, -1), false),
                (IVec2::new(0, -1), false),
                (IVec2::new(-1, 0), false),
                (IVec2::ZERO, true),
            ]
        );
    }

    #[test]
    fn zero_dim_grid_is_empty_and_writes_are_rejected() {
        let mut grid = IntentGrid::new(0, 0);
        assert_eq!(grid.width(), 0);
        assert_eq!(grid.height(), 0);
        assert!(!grid.add(IVec2::ZERO, IntentKind::Gather));
        assert!(grid.cell(IVec2::ZERO).is_none());
    }
}

/// Which intent layer the player brush is currently writing. The brush
/// systems read this resource and target the selected kind instead of a
/// hard-coded one, so the player can switch between Gather, Build, Defend,
/// and Corridor layers at runtime. Default is [`IntentKind::Gather`]
/// because that is the most common production layer.
#[derive(Debug, Clone, Copy, Resource, PartialEq, Eq)]
pub struct BrushSelection {
    pub kind: IntentKind,
}

impl Default for BrushSelection {
    fn default() -> Self {
        Self {
            kind: IntentKind::Gather,
        }
    }
}

impl BrushSelection {
    pub const fn new(kind: IntentKind) -> Self {
        Self { kind }
    }
}

/// Number-row bindings for the brush layer. `Digit1` selects Gather,
/// `Digit2` Build, `3` Defend, `4` Corridor. Numpad variants are also
/// accepted. Uses `just_pressed` so holding the key does not strobe the
/// selection; if multiple keys are pressed in one frame the first matching
/// binding wins.
const BRUSH_KEY_BINDINGS: &[(KeyCode, KeyCode, IntentKind)] = &[
    (KeyCode::Digit1, KeyCode::Numpad1, IntentKind::Gather),
    (KeyCode::Digit2, KeyCode::Numpad2, IntentKind::Build),
    (KeyCode::Digit3, KeyCode::Numpad3, IntentKind::Defend),
    (KeyCode::Digit4, KeyCode::Numpad4, IntentKind::Corridor),
];

/// Primary number-row [`KeyCode`] for `kind`, or `None` if the kind has no
/// binding. Tests and other automation can use this to drive
/// [`brush_selection_keyboard_system`] through Bevy's `ButtonInput` with
/// the same key the player would press.
pub fn brush_key_for_kind(kind: IntentKind) -> Option<KeyCode> {
    BRUSH_KEY_BINDINGS
        .iter()
        .find(|(_, _, k)| *k == kind)
        .map(|(main, _, _)| *main)
}

/// Reads number-row key presses and updates the active [`BrushSelection`].
pub fn brush_selection_keyboard_system(
    keyboard_input: Res<ButtonInput<KeyCode>>,
    mut brush_selection: ResMut<BrushSelection>,
) {
    for &(main, numpad, kind) in BRUSH_KEY_BINDINGS {
        if keyboard_input.just_pressed(main) || keyboard_input.just_pressed(numpad) {
            brush_selection.kind = kind;
            break;
        }
    }
}
