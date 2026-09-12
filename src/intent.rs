//! Swarm-owned player intent data.
//!
//! The [`IntentGrid`] resource is the simulation-side source of truth for player
//! intent paint. It is plain Rust data with no dependency on Bevy rendering or
//! shader storage buffers. The GPU zone material reads from this resource via a
//! mirror system; the resource itself never reads from rendering.

use std::collections::HashSet;

use bevy::{
    input::{ButtonInput, keyboard::KeyCode},
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

/// Independent intent layers for each swarm at one cell. `active` aggregates
/// all owners' [`IntentKind::bit`] flags for consumers of the combined overlay.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IntentCell {
    pub active: u8,
    swarm_layers: Vec<(SwarmId, u8)>,
}

impl IntentCell {
    /// True when no swarm has intent at this cell.
    pub fn is_empty(&self) -> bool {
        self.active == 0
    }

    /// True when any swarm has the given intent kind at this cell.
    pub fn has(&self, kind: IntentKind) -> bool {
        (self.active & kind.bit()) != 0
    }

    /// True when `swarm` has its own `kind` intent at this cell.
    pub fn has_owned(&self, kind: IntentKind, swarm: SwarmId) -> bool {
        self.swarm_layers
            .binary_search_by_key(&swarm.0, |(owner, _)| owner.0)
            .is_ok_and(|index| self.swarm_layers[index].1 & kind.bit() != 0)
    }

    /// Owners of `kind` in ascending swarm-ID order.
    pub fn owners(&self, kind: IntentKind) -> impl Iterator<Item = SwarmId> + '_ {
        self.swarm_layers
            .iter()
            .filter_map(move |&(swarm, layers)| (layers & kind.bit() != 0).then_some(swarm))
    }

    /// Iterate distinct active intent kinds in declaration order.
    pub fn iter_layers(&self) -> impl Iterator<Item = IntentLayer> + '_ {
        IntentKind::ALL
            .into_iter()
            .filter(|&kind| self.has(kind))
            .map(|kind| IntentLayer { kind })
    }

    fn paint(&mut self, kind: IntentKind, swarm: SwarmId) -> bool {
        match self
            .swarm_layers
            .binary_search_by_key(&swarm.0, |(owner, _)| owner.0)
        {
            Ok(index) => {
                let layers = &mut self.swarm_layers[index].1;
                if *layers & kind.bit() != 0 {
                    return false;
                }
                *layers |= kind.bit();
            }
            Err(index) => self.swarm_layers.insert(index, (swarm, kind.bit())),
        }
        self.active |= kind.bit();
        true
    }

    fn erase(&mut self, kind: IntentKind, swarm: SwarmId) -> bool {
        let Ok(index) = self
            .swarm_layers
            .binary_search_by_key(&swarm.0, |(owner, _)| owner.0)
        else {
            return false;
        };
        let layers = &mut self.swarm_layers[index].1;
        if *layers & kind.bit() == 0 {
            return false;
        }
        *layers &= !kind.bit();
        if *layers == 0 {
            self.swarm_layers.remove(index);
        }
        self.active = self
            .swarm_layers
            .iter()
            .fold(0, |active, (_, layers)| active | layers);
        true
    }
}

/// Swarm-owned simulation state for player intent. Plain Rust, no rendering
/// dependencies. Inserted as a Bevy [`Resource`] so simulation systems can read
/// and write it without going through any GPU buffer or zone material.
#[derive(Debug, Clone, Resource)]
pub struct IntentGrid {
    width: i32,
    height: i32,
    revision: u64,
    cells: Vec<IntentCell>,
    /// Non-empty cells in deterministic `(y, x)` order. Simulation systems use
    /// this sparse index instead of scanning the million-cell map every tick.
    active_cells: Vec<IVec2>,
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
            revision: 0,
            cells: vec![IntentCell::default(); size],
            active_cells: Vec::new(),
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

    /// Clear session intent while invalidating external snapshots and render mirrors.
    pub(crate) fn reset_session(&mut self) {
        let revision = self.revision.saturating_add(1);
        let mut dirty = self.render_dirty.clone();
        dirty.extend(self.active_cells.iter().copied());
        *self = Self::new(self.width, self.height);
        self.revision = revision;
        self.render_dirty.extend(dirty);
    }

    /// Monotonic revision of externally visible cell state.
    pub fn revision(&self) -> u64 {
        self.revision
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

    /// Paint `kind` for `swarm` without changing any other swarm's intent.
    /// Returns whether `point` is in bounds; repeated paint is a no-op.
    pub fn paint(&mut self, point: IVec2, kind: IntentKind, swarm: SwarmId) -> bool {
        if !self.in_bounds(point) {
            return false;
        }
        let idx = self.index(point);
        let was_empty = self.cells[idx].is_empty();
        if self.cells[idx].paint(kind, swarm) {
            if was_empty {
                self.insert_active(point);
            }
            self.mark_dirty(point);
        }
        true
    }

    /// Erase only `swarm`'s `kind` intent at `point`.
    /// Returns whether `point` is in bounds; absent paint is a no-op.
    pub fn erase(&mut self, point: IVec2, kind: IntentKind, swarm: SwarmId) -> bool {
        if !self.in_bounds(point) {
            return false;
        }
        let idx = self.index(point);
        if self.cells[idx].erase(kind, swarm) {
            if self.cells[idx].is_empty() {
                self.remove_active(point);
            }
            self.mark_dirty(point);
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

    /// Iterate every cell in row-major order. Reserved for consumers that truly
    /// need empty cells too, such as full-grid serialization.
    pub fn iter_cells(&self) -> impl Iterator<Item = (IVec2, &IntentCell)> {
        let w = self.width;
        let min = self.origin_min();
        self.cells.iter().enumerate().map(move |(i, cell)| {
            let x = min.x + (i as i32) % w;
            let y = min.y + (i as i32) / w;
            (IVec2::new(x, y), cell)
        })
    }

    /// Iterate only non-empty cells in deterministic row-major order.
    pub fn iter_active_cells(&self) -> impl Iterator<Item = (IVec2, &IntentCell)> {
        self.active_cells
            .iter()
            .copied()
            .map(|point| (point, &self.cells[self.index(point)]))
    }

    /// Iterate non-empty cells strictly after `after` in deterministic row-major order.
    pub(crate) fn iter_active_cells_after(
        &self,
        after: Option<IVec2>,
    ) -> impl Iterator<Item = (IVec2, &IntentCell)> {
        let start = after.map_or(0, |point| {
            self.active_cells
                .partition_point(|candidate| (candidate.y, candidate.x) <= (point.y, point.x))
        });
        self.active_cells[start..]
            .iter()
            .copied()
            .map(|point| (point, &self.cells[self.index(point)]))
    }

    /// Unique cells containing any of `swarm`'s intent, in row-major order.
    /// Overlapping cells independently belong to each owner.
    pub fn swarm_tiles(&self, swarm: SwarmId) -> Vec<IVec2> {
        self.iter_active_cells()
            .filter_map(|(point, cell)| {
                cell.swarm_layers
                    .binary_search_by_key(&swarm.0, |(owner, _)| owner.0)
                    .is_ok()
                    .then_some(point)
            })
            .collect()
    }

    fn insert_active(&mut self, point: IVec2) {
        let key = |candidate: &IVec2| (candidate.y, candidate.x);
        if let Err(index) = self
            .active_cells
            .binary_search_by_key(&(point.y, point.x), key)
        {
            self.active_cells.insert(index, point);
        }
    }

    fn remove_active(&mut self, point: IVec2) {
        let key = |candidate: &IVec2| (candidate.y, candidate.x);
        if let Ok(index) = self
            .active_cells
            .binary_search_by_key(&(point.y, point.x), key)
        {
            self.active_cells.remove(index);
        }
    }

    fn mark_dirty(&mut self, point: IVec2) {
        self.revision = self.revision.saturating_add(1);
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
    menu: Option<Res<crate::ui::scenario_menu::ScenarioMenu>>,
) {
    if menu.is_some_and(|menu| menu.blocks_world_input) {
        return;
    }
    for &(main, numpad, kind) in BRUSH_KEY_BINDINGS {
        if keyboard_input.just_pressed(main) || keyboard_input.just_pressed(numpad) {
            brush_selection.kind = kind;
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_reset_invalidates_snapshots_and_clears_previously_rendered_intent() {
        let mut grid = IntentGrid::new(8, 8);
        grid.paint(IVec2::ZERO, IntentKind::Build, SwarmId::PLAYER);
        grid.paint(IVec2::ONE, IntentKind::Gather, SwarmId(2));
        grid.render_dirty.clear();
        grid.erase(IVec2::ONE, IntentKind::Gather, SwarmId(2));
        let previous_revision = grid.revision();
        grid.reset_session();
        assert!(grid.revision() > previous_revision);
        assert_eq!(grid.iter_active_cells().count(), 0);
        assert!(!grid.cell(IVec2::ZERO).unwrap().has(IntentKind::Build));
        assert_eq!(grid.render_dirty, HashSet::from([IVec2::ZERO, IVec2::ONE]));
    }

    #[test]
    fn three_swarms_keep_every_kind_independently_at_one_cell() {
        let mut grid = IntentGrid::new(4, 4);
        let point = IVec2::ZERO;
        for swarm in [SwarmId(9), SwarmId(2), SwarmId(7)] {
            for kind in IntentKind::ALL {
                assert!(grid.paint(point, kind, swarm));
            }
        }

        let cell = grid.cell(point).unwrap();
        for kind in IntentKind::ALL {
            assert_eq!(
                cell.owners(kind).collect::<Vec<_>>(),
                vec![SwarmId(2), SwarmId(7), SwarmId(9)]
            );
            assert!(cell.has_owned(kind, SwarmId(2)));
            assert!(cell.has_owned(kind, SwarmId(7)));
            assert!(cell.has_owned(kind, SwarmId(9)));
            assert!(!cell.has_owned(kind, SwarmId(3)));
        }
        assert_eq!(
            cell.iter_layers()
                .map(|layer| layer.kind)
                .collect::<Vec<_>>(),
            vec![
                IntentKind::Gather,
                IntentKind::Build,
                IntentKind::Defend,
                IntentKind::Corridor
            ]
        );
    }

    #[test]
    fn repeated_paint_and_absent_erase_leave_revision_and_dirty_queues_unchanged() {
        let mut grid = IntentGrid::new(4, 4);
        let point = IVec2::ZERO;
        grid.paint(point, IntentKind::Gather, SwarmId(7));
        assert_eq!(grid.revision(), 1);
        assert_eq!(grid.drain_render_dirty(), vec![point]);
        assert_eq!(grid.drain_projection_dirty(), vec![point]);

        grid.paint(point, IntentKind::Gather, SwarmId(7));
        grid.erase(point, IntentKind::Gather, SwarmId(9));
        grid.erase(point, IntentKind::Build, SwarmId(7));

        assert_eq!(grid.revision(), 1);
        assert_eq!(grid.render_dirty_count(), 0);
        assert_eq!(grid.projection_dirty_count(), 0);
        assert!(
            grid.cell(point)
                .unwrap()
                .has_owned(IntentKind::Gather, SwarmId(7))
        );
    }

    #[test]
    fn adding_and_erasing_overlap_notifies_consumers_without_removing_other_orders() {
        let mut grid = IntentGrid::new(4, 4);
        let point = IVec2::ZERO;
        grid.paint(point, IntentKind::Build, SwarmId(2));
        grid.paint(point, IntentKind::Gather, SwarmId(7));
        grid.drain_render_dirty();
        grid.drain_projection_dirty();

        grid.paint(point, IntentKind::Build, SwarmId(7));
        assert_eq!(grid.revision(), 3);
        assert_eq!(grid.drain_render_dirty(), vec![point]);
        assert_eq!(grid.drain_projection_dirty(), vec![point]);

        grid.erase(point, IntentKind::Build, SwarmId(7));
        let cell = grid.cell(point).unwrap();
        assert_eq!(
            cell.owners(IntentKind::Build).collect::<Vec<_>>(),
            vec![SwarmId(2)]
        );
        assert!(cell.has_owned(IntentKind::Gather, SwarmId(7)));
        assert!(!cell.has_owned(IntentKind::Build, SwarmId(7)));
        assert!(cell.has(IntentKind::Build));
        assert_eq!(grid.revision(), 4);
        assert_eq!(grid.drain_render_dirty(), vec![point]);
        assert_eq!(grid.drain_projection_dirty(), vec![point]);
    }

    #[test]
    fn every_kind_establishes_territory_for_all_its_owners_once() {
        let mut grid = IntentGrid::new(8, 8);
        let first = IVec2::new(-1, 0);
        let overlap = IVec2::ZERO;
        let last = IVec2::new(1, 0);
        grid.paint(last, IntentKind::Corridor, SwarmId(7));
        grid.paint(first, IntentKind::Gather, SwarmId(7));
        grid.paint(first, IntentKind::Build, SwarmId(7));
        for swarm in [SwarmId(7), SwarmId(9), SwarmId(2)] {
            grid.paint(overlap, IntentKind::Defend, swarm);
        }

        assert_eq!(grid.swarm_tiles(SwarmId(7)), vec![first, overlap, last]);
        assert_eq!(grid.swarm_tiles(SwarmId(9)), vec![overlap]);
        assert_eq!(grid.swarm_tiles(SwarmId(2)), vec![overlap]);
        assert!(grid.swarm_tiles(SwarmId(3)).is_empty());

        grid.erase(overlap, IntentKind::Defend, SwarmId(7));
        assert_eq!(grid.swarm_tiles(SwarmId(7)), vec![first, last]);
        assert_eq!(grid.swarm_tiles(SwarmId(9)), vec![overlap]);
    }

    #[test]
    fn sparse_index_keeps_overlap_until_last_order_is_erased() {
        let mut grid = IntentGrid::new(1000, 1000);
        let first = IVec2::new(7, -3);
        let second = IVec2::new(-4, 8);
        grid.paint(second, IntentKind::Build, SwarmId(2));
        grid.paint(first, IntentKind::Gather, SwarmId(2));
        grid.paint(first, IntentKind::Gather, SwarmId(7));
        grid.paint(first, IntentKind::Defend, SwarmId(2));
        assert_eq!(
            grid.iter_active_cells()
                .map(|(point, _)| point)
                .collect::<Vec<_>>(),
            vec![first, second]
        );

        grid.erase(first, IntentKind::Gather, SwarmId(2));
        grid.erase(first, IntentKind::Defend, SwarmId(2));
        assert_eq!(
            grid.iter_active_cells()
                .map(|(point, _)| point)
                .collect::<Vec<_>>(),
            vec![first, second]
        );
        grid.erase(first, IntentKind::Gather, SwarmId(7));
        assert_eq!(
            grid.iter_active_cells()
                .map(|(point, _)| point)
                .collect::<Vec<_>>(),
            vec![second]
        );
        assert!(grid.cell(first).unwrap().is_empty());
        assert_eq!(grid.cell(first).unwrap().iter_layers().count(), 0);
        assert_eq!(
            grid.cell(first).unwrap().owners(IntentKind::Gather).count(),
            0
        );
    }

    #[test]
    fn dirty_cells_drain_independently_in_row_major_order() {
        let mut grid = IntentGrid::new(4, 4);
        grid.paint(IVec2::new(0, -1), IntentKind::Build, SwarmId(7));
        grid.paint(IVec2::new(-2, -2), IntentKind::Gather, SwarmId(2));
        grid.paint(IVec2::new(0, -1), IntentKind::Build, SwarmId(2));

        let expected = vec![IVec2::new(-2, -2), IVec2::new(0, -1)];
        assert_eq!(grid.drain_render_dirty(), expected);
        assert_eq!(grid.projection_dirty_count(), 2);
        assert_eq!(grid.drain_projection_dirty(), expected);
        assert_eq!(grid.render_dirty_count(), 0);
    }

    #[test]
    fn iter_cells_covers_empty_and_painted_cells_in_row_major_order() {
        let mut grid = IntentGrid::new(2, 2);
        grid.paint(IVec2::ZERO, IntentKind::Defend, SwarmId(7));
        let seen = grid
            .iter_cells()
            .map(|(point, cell)| (point, cell.has(IntentKind::Defend)))
            .collect::<Vec<_>>();
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
    fn centered_bounds_accept_edge_cells_and_reject_outside_writes() {
        let mut grid = IntentGrid::new(3, 3);
        assert!(grid.paint(IVec2::new(-1, -1), IntentKind::Build, SwarmId(7)));
        assert!(grid.paint(IVec2::new(1, 1), IntentKind::Build, SwarmId(7)));
        assert!(!grid.paint(IVec2::new(-2, 0), IntentKind::Gather, SwarmId(7)));
        assert!(!grid.erase(IVec2::new(2, 0), IntentKind::Build, SwarmId(7)));
        assert!(grid.cell(IVec2::new(0, 2)).is_none());
        assert_eq!(grid.revision(), 2);
        assert_eq!(grid.iter_active_cells().count(), 2);
    }

    #[test]
    fn zero_and_negative_dimensions_are_empty_and_reject_writes() {
        for dimensions in [(0, 0), (-3, 4), (4, -3)] {
            let mut grid = IntentGrid::new(dimensions.0, dimensions.1);
            assert!(!grid.paint(IVec2::ZERO, IntentKind::Gather, SwarmId(7)));
            assert!(!grid.erase(IVec2::ZERO, IntentKind::Gather, SwarmId(7)));
            assert!(grid.cell(IVec2::ZERO).is_none());
            assert_eq!(grid.iter_cells().count(), 0);
            assert_eq!(grid.iter_active_cells().count(), 0);
            assert_eq!(grid.revision(), 0);
        }
    }
}
