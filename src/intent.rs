//! Swarm-owned player intent data.
//!
//! The [`IntentGrid`] resource is the simulation-side source of truth for player
//! intent paint. It is plain Rust data with no dependency on Bevy rendering or
//! shader storage buffers. The GPU zone material reads from this resource via a
//! mirror system; the resource itself never reads from rendering.

use std::collections::HashSet;

use bevy::prelude::{IVec2, Resource};

/// Player intent kinds. The order matches the four zone colour slots used by the
/// existing zone shader, so the bit index of each kind lines up with the shader's
/// per-cell bit layout until a future issue replaces that mirror.
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

/// One active intent layer at a cell: which kind and how strong the paint is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IntentLayer {
    pub kind: IntentKind,
    pub strength: u8,
}

/// Multiple intent layers at one cell. `active` is a bitmask of
/// [`IntentKind::bit`] flags; `strength` stores the paint strength for each
/// kind, but the entry is only meaningful while the matching bit in `active`
/// is set.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct IntentCell {
    pub active: u8,
    pub strength: [u8; IntentKind::COUNT],
}

impl IntentCell {
    /// True when no intent layer is active at this cell.
    pub fn is_empty(&self) -> bool {
        self.active == 0
    }

    /// True when the given intent kind is active at this cell.
    pub fn has(&self, kind: IntentKind) -> bool {
        (self.active & kind.bit()) != 0
    }

    /// Paint strength for `kind`, or `0` if the kind is not active.
    pub fn strength(&self, kind: IntentKind) -> u8 {
        if self.has(kind) {
            self.strength[kind.index()]
        } else {
            0
        }
    }

    /// Activate `kind` with the given paint strength. Overwrites any previous
    /// strength for the same kind.
    pub fn add(&mut self, kind: IntentKind, strength: u8) {
        self.active |= kind.bit();
        self.strength[kind.index()] = strength;
    }

    /// Deactivate `kind` and clear its stored strength.
    pub fn remove(&mut self, kind: IntentKind) {
        self.active &= !kind.bit();
        self.strength[kind.index()] = 0;
    }

    /// Iterate active intent layers at this cell. Order matches
    /// [`IntentKind`] declaration order.
    pub fn iter_layers(&self) -> impl Iterator<Item = IntentLayer> + '_ {
        IntentKind::ALL.into_iter().filter_map(move |kind| {
            self.has(kind).then_some(IntentLayer {
                kind,
                strength: self.strength[kind.index()],
            })
        })
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
    /// Cells touched since the last [`IntentGrid::drain_dirty`] call. Mirror
    /// systems can use this to push only changed cells to the GPU buffer.
    dirty: HashSet<IVec2>,
}

impl IntentGrid {
    /// Build a new grid of `width` x `height` empty cells.
    pub fn new(width: i32, height: i32) -> Self {
        let size = (width.max(0) as usize) * (height.max(0) as usize);
        Self {
            width: width.max(0),
            height: height.max(0),
            cells: vec![IntentCell::default(); size],
            dirty: HashSet::new(),
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

    /// Add `kind` intent at `point` with the given paint strength. Returns
    /// `true` when the cell was within bounds and the layer was added.
    pub fn add(&mut self, point: IVec2, kind: IntentKind, strength: u8) -> bool {
        if let Some(cell) = self.cell_mut(point) {
            cell.add(kind, strength);
            true
        } else {
            false
        }
    }

    /// Remove `kind` intent at `point`. Returns `true` when the cell was
    /// within bounds (regardless of whether the kind was active).
    pub fn remove(&mut self, point: IVec2, kind: IntentKind) -> bool {
        if let Some(cell) = self.cell_mut(point) {
            cell.remove(kind);
            true
        } else {
            false
        }
    }

    /// Number of cells that have been mutated since the last drain. Mirror
    /// systems use this for cheap "anything to push" checks.
    pub fn dirty_count(&self) -> usize {
        self.dirty.len()
    }

    /// Take the set of dirty cells, sorted by `(y, x)` for deterministic
    /// iteration order.
    pub fn drain_dirty(&mut self) -> Vec<IVec2> {
        let mut points: Vec<IVec2> = self.dirty.drain().collect();
        points.sort_by_key(|p| (p.y, p.x));
        points
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

    fn cell_mut(&mut self, point: IVec2) -> Option<&mut IntentCell> {
        if !self.in_bounds(point) {
            return None;
        }
        let idx = self.index(point);
        self.dirty.insert(point);
        Some(&mut self.cells[idx])
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_grid_has_no_active_layers() {
        let grid = IntentGrid::new(4, 4);
        let cell = grid.cell(IVec2::new(1, 1)).unwrap();
        assert!(cell.is_empty());
        assert!(!cell.has(IntentKind::Gather));
        assert!(!cell.has(IntentKind::Build));
        assert_eq!(cell.strength(IntentKind::Gather), 0);
        assert_eq!(cell.iter_layers().count(), 0);
    }

    #[test]
    fn add_activates_a_layer_with_stored_strength() {
        let mut grid = IntentGrid::new(4, 4);
        assert!(grid.add(IVec2::new(-2, -2), IntentKind::Gather, 7));

        let cell = grid.cell(IVec2::new(-2, -2)).unwrap();
        assert!(cell.has(IntentKind::Gather));
        assert_eq!(cell.strength(IntentKind::Gather), 7);
        assert!(!cell.has(IntentKind::Build));
    }

    #[test]
    fn multiple_kinds_coexist_at_one_cell() {
        let mut grid = IntentGrid::new(4, 4);
        let point = IVec2::new(1, 1);
        assert!(grid.add(point, IntentKind::Gather, 3));
        assert!(grid.add(point, IntentKind::Defend, 5));
        assert!(grid.add(point, IntentKind::Corridor, 9));

        let cell = grid.cell(point).unwrap();
        assert!(cell.has(IntentKind::Gather));
        assert!(!cell.has(IntentKind::Build));
        assert!(cell.has(IntentKind::Defend));
        assert!(cell.has(IntentKind::Corridor));

        let layers: Vec<IntentLayer> = cell.iter_layers().collect();
        assert_eq!(layers.len(), 3);
        assert!(layers.contains(&IntentLayer {
            kind: IntentKind::Gather,
            strength: 3
        }));
        assert!(layers.contains(&IntentLayer {
            kind: IntentKind::Defend,
            strength: 5
        }));
        assert!(layers.contains(&IntentLayer {
            kind: IntentKind::Corridor,
            strength: 9
        }));
    }

    #[test]
    fn remove_clears_only_the_target_kind() {
        let mut grid = IntentGrid::new(4, 4);
        let point = IVec2::new(1, 1);
        grid.add(point, IntentKind::Gather, 4);
        grid.add(point, IntentKind::Build, 6);

        assert!(grid.remove(point, IntentKind::Gather));

        let cell = grid.cell(point).unwrap();
        assert!(!cell.has(IntentKind::Gather));
        assert_eq!(cell.strength(IntentKind::Gather), 0);
        assert!(cell.has(IntentKind::Build));
        assert_eq!(cell.strength(IntentKind::Build), 6);
        assert!(!cell.is_empty());
    }

    #[test]
    fn out_of_bounds_writes_are_rejected() {
        let mut grid = IntentGrid::new(3, 3);
        assert!(grid.in_bounds(IVec2::new(-1, -1)));
        assert!(grid.in_bounds(IVec2::new(1, 1)));
        assert!(!grid.in_bounds(IVec2::new(-2, 0)));
        assert!(!grid.in_bounds(IVec2::new(2, 0)));
        assert!(!grid.in_bounds(IVec2::new(0, 2)));
        assert!(!grid.in_bounds(IVec2::new(0, -2)));

        assert!(!grid.add(IVec2::new(-2, 0), IntentKind::Gather, 1));
        assert!(!grid.add(IVec2::new(2, 0), IntentKind::Gather, 1));
        assert!(!grid.remove(IVec2::new(0, 2), IntentKind::Gather));

        assert!(grid.cell(IVec2::new(-2, 0)).is_none());
        assert!(grid.cell(IVec2::new(2, 0)).is_none());
    }

    #[test]
    fn dirty_cells_are_tracked_and_drainable() {
        let mut grid = IntentGrid::new(4, 4);
        grid.add(IVec2::new(-2, -2), IntentKind::Gather, 1);
        grid.add(IVec2::new(0, -1), IntentKind::Build, 2);
        // re-touching the same cell does not double-count
        grid.add(IVec2::new(-2, -2), IntentKind::Gather, 1);
        assert_eq!(grid.dirty_count(), 2);

        let drained = grid.drain_dirty();
        assert_eq!(drained.len(), 2);
        // deterministic order: by y then x
        assert_eq!(drained[0], IVec2::new(-2, -2));
        assert_eq!(drained[1], IVec2::new(0, -1));
        assert_eq!(grid.dirty_count(), 0);
    }

    #[test]
    fn iter_cells_covers_whole_grid_in_row_major_order() {
        let mut grid = IntentGrid::new(2, 2);
        grid.add(IVec2::new(0, 0), IntentKind::Defend, 2);

        let seen: Vec<(IVec2, bool)> = grid
            .iter_cells()
            .map(|(p, c)| (p, c.has(IntentKind::Defend)))
            .collect();
        assert_eq!(
            seen,
            vec![
                (IVec2::new(-1, -1), false),
                (IVec2::new(0, -1), false),
                (IVec2::new(-1, 0), false),
                (IVec2::new(0, 0), true),
            ]
        );
    }

    #[test]
    fn deterministic_sequence_produces_same_grid_state() {
        fn run(ops: &[(IVec2, IntentKind, u8, bool)]) -> Vec<u8> {
            // bool: true = add, false = remove
            let mut grid = IntentGrid::new(2, 2);
            for (p, k, s, add) in ops {
                if *add {
                    grid.add(*p, *k, *s);
                } else {
                    grid.remove(*p, *k);
                }
            }
            grid.cells
                .iter()
                .flat_map(|c| [c.active, c.strength[0], c.strength[1]])
                .collect()
        }

        let ops = vec![
            (IVec2::new(-1, -1), IntentKind::Gather, 4, true),
            (IVec2::new(0, -1), IntentKind::Build, 2, true),
            (IVec2::new(-1, -1), IntentKind::Gather, 4, false),
            (IVec2::new(-1, 0), IntentKind::Defend, 9, true),
        ];

        let first = run(&ops);
        let second = run(&ops);
        assert_eq!(first, second);
    }

    #[test]
    fn zero_dim_grid_is_empty_and_writes_are_rejected() {
        let mut grid = IntentGrid::new(0, 0);
        assert_eq!(grid.width(), 0);
        assert_eq!(grid.height(), 0);
        assert!(!grid.add(IVec2::new(0, 0), IntentKind::Gather, 1));
        assert!(grid.cell(IVec2::new(0, 0)).is_none());
    }
}
