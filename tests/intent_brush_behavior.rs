//! Integration test for the swarm-owned [`IntentGrid`] resource.
//!
//! Wires the resource through a minimal Bevy `App` (no rendering plugins, no
//! zone material, no group entities) and asserts deterministic read/write
//! behaviour through the public Bevy resource API. This is the seam future
//! simulation systems (allocation, production, AI scoring) will read from
//! without going through rendering.

use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::intent::{
    brush_selection_keyboard_system, BrushSelection, IntentGrid, IntentKind, PAINT_STRENGTH_CAP,
};

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

#[test]
fn paint_saturates_at_cap_through_bevy_app() {
    let mut app = App::new();
    app.insert_resource(IntentGrid::new(4, 4));
    let point = IVec2::new(0, 0);

    for _ in 0..(PAINT_STRENGTH_CAP as usize + 5) {
        let mut grid = app.world_mut().resource_mut::<IntentGrid>();
        assert!(grid.paint(point, IntentKind::Build, 1));
    }
    app.update();

    let grid = app.world().resource::<IntentGrid>();
    let cell = grid.cell(point).expect("cell must exist");
    assert_eq!(cell.strength(IntentKind::Build), PAINT_STRENGTH_CAP);
}

#[test]
fn erase_to_zero_removes_the_layer_through_bevy_app() {
    let mut app = App::new();
    app.insert_resource(IntentGrid::new(4, 4));
    let point = IVec2::new(0, 0);
    let target = 4u8;

    for _ in 0..target {
        app.world_mut()
            .resource_mut::<IntentGrid>()
            .paint(point, IntentKind::Gather, 1);
    }
    for _ in 0..target {
        app.world_mut()
            .resource_mut::<IntentGrid>()
            .erase(point, IntentKind::Gather, 1);
    }
    app.update();

    let grid = app.world().resource::<IntentGrid>();
    let cell = grid.cell(point).expect("cell must exist");
    assert!(!cell.has(IntentKind::Gather));
    assert_eq!(cell.strength(IntentKind::Gather), 0);
    assert_eq!(cell.active, 0);
}

#[test]
fn paint_persists_across_app_updates_without_input() {
    let mut app = App::new();
    app.insert_resource(IntentGrid::new(4, 4));
    let point = IVec2::new(0, 0);

    app.world_mut()
        .resource_mut::<IntentGrid>()
        .paint(point, IntentKind::Gather, 3);
    app.update();

    // run many updates without any further input; nothing in the app
    // touches the grid, so the layer must persist unchanged (this also
    // covers the "depleted local work" persistence contract: no work
    // system ever clears intent).
    for _ in 0..20 {
        app.update();
    }

    let grid = app.world().resource::<IntentGrid>();
    let cell = grid.cell(point).expect("cell must exist");
    assert!(cell.has(IntentKind::Gather));
    assert_eq!(cell.strength(IntentKind::Gather), 3);
}

#[test]
fn overlapping_layers_keep_independent_strengths_through_bevy_app() {
    let mut app = App::new();
    app.insert_resource(IntentGrid::new(4, 4));
    let point = IVec2::new(0, 0);

    {
        let mut grid = app.world_mut().resource_mut::<IntentGrid>();
        grid.paint(point, IntentKind::Gather, 5);
        grid.paint(point, IntentKind::Build, 7);
        grid.paint(point, IntentKind::Defend, 2);
        // single big paint clamps to the cap independently of the others
        grid.paint(point, IntentKind::Corridor, 200);
    }
    app.update();

    {
        let mut grid = app.world_mut().resource_mut::<IntentGrid>();
        grid.erase(point, IntentKind::Gather, 2);
    }
    app.update();

    let grid = app.world().resource::<IntentGrid>();
    let cell = grid.cell(point).expect("cell must exist");
    assert_eq!(cell.strength(IntentKind::Gather), 3);
    assert_eq!(cell.strength(IntentKind::Build), 7);
    assert_eq!(cell.strength(IntentKind::Defend), 2);
    assert_eq!(cell.strength(IntentKind::Corridor), PAINT_STRENGTH_CAP);
    assert!(cell.has(IntentKind::Gather));
    assert!(cell.has(IntentKind::Build));
    assert!(cell.has(IntentKind::Defend));
    assert!(cell.has(IntentKind::Corridor));
}

#[test]
fn brush_selection_defaults_to_gather() {
    let selection = BrushSelection::default();
    assert_eq!(selection.kind, IntentKind::Gather);
}

#[test]
fn brush_selection_keyboard_switches_layer() {
    // Each case runs in a fresh App + ButtonInput. We intentionally do not
    // add Bevy's InputPlugin here: its PreUpdate keyboard_input_system calls
    // ButtonInput::clear() before our Update system runs, which would wipe
    // the manual press() calls below. Driving the resource directly is the
    // minimum seam that exercises the real system function.
    fn run_switch(pressed: &[KeyCode]) -> IntentKind {
        let mut app = App::new();
        app.init_resource::<BrushSelection>();
        app.add_systems(Update, brush_selection_keyboard_system);
        let mut keyboard = ButtonInput::<KeyCode>::default();
        for k in pressed {
            keyboard.press(*k);
        }
        app.insert_resource(keyboard);
        app.update();
        app.world().resource::<BrushSelection>().kind
    }

    assert_eq!(run_switch(&[]), IntentKind::Gather, "default selection");
    assert_eq!(
        run_switch(&[KeyCode::Digit1]),
        IntentKind::Gather,
        "1 selects Gather"
    );
    assert_eq!(
        run_switch(&[KeyCode::Digit2]),
        IntentKind::Build,
        "2 selects Build"
    );
    assert_eq!(
        run_switch(&[KeyCode::Digit3]),
        IntentKind::Defend,
        "3 selects Defend"
    );
    assert_eq!(
        run_switch(&[KeyCode::Digit4]),
        IntentKind::Corridor,
        "4 selects Corridor"
    );
    // numpad variants are also accepted
    assert_eq!(
        run_switch(&[KeyCode::Numpad4]),
        IntentKind::Corridor,
        "numpad 4 selects Corridor"
    );
}

#[test]
fn corridor_layer_is_stored_independently_of_work_layers() {
    // The Corridor kind must coexist in the same cell as Gather, Build, and
    // Defend without interfering with their strengths. This is the storage
    // contract: logistics corridor is path guidance, not a work-producing
    // zone, and lives as its own layer.
    let mut grid = IntentGrid::new(4, 4);
    let point = IVec2::new(0, 0);

    assert!(grid.paint(point, IntentKind::Gather, 4));
    assert!(grid.paint(point, IntentKind::Build, 6));
    assert!(grid.paint(point, IntentKind::Defend, 2));
    assert!(grid.paint(point, IntentKind::Corridor, 9));

    let cell = grid.cell(point).expect("cell must exist");
    assert_eq!(cell.strength(IntentKind::Gather), 4);
    assert_eq!(cell.strength(IntentKind::Build), 6);
    assert_eq!(cell.strength(IntentKind::Defend), 2);
    assert_eq!(cell.strength(IntentKind::Corridor), 9);

    // removing corridor must leave the other three layers untouched
    assert!(grid.remove(point, IntentKind::Corridor));
    let cell = grid.cell(point).expect("cell must exist");
    assert!(!cell.has(IntentKind::Corridor));
    assert_eq!(cell.strength(IntentKind::Corridor), 0);
    assert_eq!(cell.strength(IntentKind::Gather), 4);
    assert_eq!(cell.strength(IntentKind::Build), 6);
    assert_eq!(cell.strength(IntentKind::Defend), 2);
}

#[test]
fn brush_selection_kind_drives_which_layer_is_written() {
    // Reads through `BrushSelection::kind` must change which intent layer a
    // paint or erase targets. Switching the resource between writes must
    // reach the new kind while leaving previously-painted layers alone.
    let mut app = App::new();
    app.insert_resource(IntentGrid::new(4, 4));
    app.insert_resource(BrushSelection::default());
    let point = IVec2::new(0, 0);

    // default selection is Gather; painting through the resource must hit
    // the Gather layer.
    {
        let kind = app.world().resource::<BrushSelection>().kind;
        let mut grid = app.world_mut().resource_mut::<IntentGrid>();
        assert!(grid.paint(point, kind, 3));
    }

    // switch to Defend and paint; Gather must keep its prior strength.
    {
        let kind = {
            let mut selection = app.world_mut().resource_mut::<BrushSelection>();
            selection.kind = IntentKind::Defend;
            selection.kind
        };
        let mut grid = app.world_mut().resource_mut::<IntentGrid>();
        assert!(grid.paint(point, kind, 5));
    }

    let grid = app.world().resource::<IntentGrid>();
    let cell = grid.cell(point).expect("cell must exist");
    assert_eq!(cell.strength(IntentKind::Gather), 3);
    assert_eq!(cell.strength(IntentKind::Defend), 5);
    assert!(!cell.has(IntentKind::Build));
    assert!(!cell.has(IntentKind::Corridor));
}

#[test]
fn defend_layer_exists_in_data_with_no_attack_kind() {
    // Defend is the only combat-intent kind. The grid has no separate attack
    // variant; painting into "enemy territory" (a cell with no other
    // constraint) is a defend-layer write and nothing more.
    let mut grid = IntentGrid::new(4, 4);
    // (1, 1) sits inside the 4x4 grid's [-2, 2) bounds, so the write
    // must succeed regardless of whether the cell is "owned" by us or an
    // opponent. The grid has no ownership concept; intent paint is just
    // data.
    let enemy_cell = IVec2::new(1, 1);
    assert!(grid.paint(enemy_cell, IntentKind::Defend, 7));

    let cell = grid.cell(enemy_cell).expect("cell must exist");
    assert!(cell.has(IntentKind::Defend));
    assert_eq!(cell.strength(IntentKind::Defend), 7);
    // no other kind got an implicit write from the defend paint
    assert!(!cell.has(IntentKind::Gather));
    assert!(!cell.has(IntentKind::Build));
    assert!(!cell.has(IntentKind::Corridor));

    // the four IntentKind variants are the full set: Gather, Build, Defend,
    // Corridor. There is no Attack variant. Asserting the count here catches
    // a future addition that would silently violate the PRD.
    assert_eq!(IntentKind::COUNT, 4);
}

#[test]
fn brush_selection_persists_across_app_updates() {
    // No InputPlugin: see note in brush_selection_keyboard_switches_layer.
    // The keyboard system can re-fire the same kind on every update because
    // just_pressed is not cleared, but writing the same value is a no-op
    // and the selection must stay Build across frames.
    let mut app = App::new();
    app.init_resource::<BrushSelection>();
    app.add_systems(Update, brush_selection_keyboard_system);

    let mut keyboard = ButtonInput::<KeyCode>::default();
    keyboard.press(KeyCode::Digit2);
    app.insert_resource(keyboard);
    app.update();

    assert_eq!(
        app.world().resource::<BrushSelection>().kind,
        IntentKind::Build,
        "selection switches to Build on first press"
    );

    // many updates without input must keep Build selected
    for _ in 0..10 {
        app.update();
    }
    assert_eq!(
        app.world().resource::<BrushSelection>().kind,
        IntentKind::Build,
        "selection persists across idle updates"
    );
}
