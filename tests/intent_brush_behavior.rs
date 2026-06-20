//! Integration test for the swarm-owned [`IntentGrid`] resource.
//!
//! Wires the resource through a minimal Bevy `App` (no rendering plugins, no
//! zone material, no group entities) and asserts deterministic read/write
//! behaviour through the public Bevy resource API. This is the seam future
//! simulation systems (allocation, production, AI scoring) will read from
//! without going through rendering.

use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::intent::{IntentGrid, IntentKind};

#[test]
fn intent_grid_resource_round_trips_through_bevy_app() {
    let mut app = App::new();
    let width = 6;
    let height = 4;
    app.insert_resource(IntentGrid::new(width, height));

    {
        let mut grid = app.world_mut().resource_mut::<IntentGrid>();
        assert!(grid.add(IVec2::new(2, 1), IntentKind::Gather, 4));
        assert!(grid.add(IVec2::new(2, 1), IntentKind::Defend, 9));
        assert!(grid.add(IVec2::new(-3, -2), IntentKind::Build, 1));
        // out-of-bounds writes are rejected silently
        assert!(!grid.add(IVec2::new(-4, 0), IntentKind::Gather, 1));
        assert!(!grid.add(IVec2::new(3, 0), IntentKind::Gather, 1));
    }

    app.update();

    let grid = app.world().resource::<IntentGrid>();
    assert_eq!(grid.width(), width);
    assert_eq!(grid.height(), height);

    let cell = grid.cell(IVec2::new(2, 1)).expect("cell must exist");
    assert!(cell.has(IntentKind::Gather));
    assert_eq!(cell.strength(IntentKind::Gather), 4);
    assert!(cell.has(IntentKind::Defend));
    assert_eq!(cell.strength(IntentKind::Defend), 9);
    assert!(!cell.has(IntentKind::Build));

    let other = grid.cell(IVec2::new(-3, -2)).expect("cell must exist");
    assert!(other.has(IntentKind::Build));
    assert_eq!(other.strength(IntentKind::Build), 1);
    assert!(!other.has(IntentKind::Gather));

    // cells the brush never touched stay empty
    let untouched = grid.cell(IVec2::new(1, 1)).expect("cell must exist");
    assert!(untouched.is_empty());
}

#[test]
fn remove_command_clears_only_target_kind_through_resource() {
    let mut app = App::new();
    app.insert_resource(IntentGrid::new(3, 3));

    {
        let mut grid = app.world_mut().resource_mut::<IntentGrid>();
        grid.add(IVec2::new(1, 1), IntentKind::Gather, 7);
        grid.add(IVec2::new(1, 1), IntentKind::Corridor, 3);
        assert!(grid.remove(IVec2::new(1, 1), IntentKind::Gather));
    }
    app.update();

    let grid = app.world().resource::<IntentGrid>();
    let cell = grid.cell(IVec2::new(1, 1)).expect("cell must exist");
    assert!(!cell.has(IntentKind::Gather));
    assert_eq!(cell.strength(IntentKind::Gather), 0);
    assert!(cell.has(IntentKind::Corridor));
    assert_eq!(cell.strength(IntentKind::Corridor), 3);
}

#[test]
fn drain_dirty_is_stable_across_runs() {
    fn run(commands: &[(IVec2, IntentKind, u8, bool)]) -> Vec<IVec2> {
        let mut app = App::new();
        app.insert_resource(IntentGrid::new(4, 4));
        {
            let mut grid = app.world_mut().resource_mut::<IntentGrid>();
            for (p, k, s, add) in commands {
                if *add {
                    grid.add(*p, *k, *s);
                } else {
                    grid.remove(*p, *k);
                }
            }
        }
        app.update();
        let mut grid = app.world_mut().resource_mut::<IntentGrid>();
        grid.drain_dirty()
    }

    let commands = vec![
        (IVec2::new(1, -2), IntentKind::Gather, 1, true),
        (IVec2::new(-2, 0), IntentKind::Build, 2, true),
        (IVec2::new(0, 1), IntentKind::Defend, 3, true),
        (IVec2::new(1, -2), IntentKind::Gather, 1, false),
        // re-touching the same cell must not produce duplicates
        (IVec2::new(0, 1), IntentKind::Defend, 5, true),
    ];

    let first = run(&commands);
    let second = run(&commands);
    assert_eq!(first, second);
    // sorted by (y, x)
    assert_eq!(
        first,
        vec![IVec2::new(1, -2), IVec2::new(-2, 0), IVec2::new(0, 1),]
    );
}
