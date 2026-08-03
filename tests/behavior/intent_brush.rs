use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::intent::{
    BrushSelection, IntentGrid, IntentKind, UNCONTESTED_CAPTURE_TICKS, brush_key_for_kind,
    brush_selection_keyboard_system,
};
use top_down_2d_rts_prototype_nano_swarm::nanobot::SwarmId;

fn press_key(app: &mut App, key: KeyCode) {
    let mut keyboard = ButtonInput::<KeyCode>::default();
    keyboard.press(key);
    app.insert_resource(keyboard);
    app.update();
}

#[test]
fn binary_paint_and_erase_round_trip_through_bevy_resource() {
    let mut app = App::new();
    app.insert_resource(IntentGrid::new(4, 4));
    let point = IVec2::ZERO;

    {
        let mut grid = app.world_mut().resource_mut::<IntentGrid>();
        assert!(grid.paint(point, IntentKind::Gather));
        assert!(grid.paint(point, IntentKind::Gather));
        assert!(grid.paint(point, IntentKind::Build));
    }
    app.update();

    let grid = app.world().resource::<IntentGrid>();
    let cell = grid.cell(point).unwrap();
    assert!(cell.has(IntentKind::Gather));
    assert!(cell.has(IntentKind::Build));
    assert_eq!(grid.render_dirty_count(), 1);
    assert_eq!(grid.projection_dirty_count(), 1);

    app.world_mut()
        .resource_mut::<IntentGrid>()
        .erase(point, IntentKind::Gather);
    let cell = app.world().resource::<IntentGrid>().cell(point).unwrap();
    assert!(!cell.has(IntentKind::Gather));
    assert!(cell.has(IntentKind::Build));
}

#[test]
fn repeated_binary_writes_do_not_create_dirty_work() {
    let mut grid = IntentGrid::new(4, 4);
    let point = IVec2::ZERO;
    grid.paint_owned(point, IntentKind::Defend, Some(SwarmId(4)));
    grid.drain_render_dirty();
    grid.drain_projection_dirty();

    grid.paint_owned(point, IntentKind::Defend, Some(SwarmId(4)));
    assert_eq!(grid.render_dirty_count(), 0);
    assert_eq!(grid.projection_dirty_count(), 0);

    grid.erase(point, IntentKind::Corridor);
    assert_eq!(grid.render_dirty_count(), 0);
    assert_eq!(grid.projection_dirty_count(), 0);
}

#[test]
fn challenging_enemy_defend_intent_neutralizes_only_that_layer() {
    let mut grid = IntentGrid::new(4, 4);
    let point = IVec2::ZERO;
    let opponent = SwarmId(7);
    grid.paint_owned(point, IntentKind::Defend, Some(opponent));
    grid.paint_owned(point, IntentKind::Build, Some(opponent));

    grid.contest_defend(point, SwarmId::PLAYER);

    let cell = grid.cell(point).unwrap();
    assert!(cell.has(IntentKind::Defend));
    assert_eq!(cell.owner(IntentKind::Defend), None);
    assert_eq!(cell.owner(IntentKind::Build), Some(opponent));
}

#[test]
fn challenger_can_withdraw_and_restore_incumbent_defend_control() {
    let mut grid = IntentGrid::new(4, 4);
    let point = IVec2::ZERO;
    let opponent = SwarmId(7);
    grid.paint_owned(point, IntentKind::Defend, Some(opponent));
    grid.contest_defend(point, SwarmId::PLAYER);

    assert!(grid.withdraw_defend_contest(point, SwarmId::PLAYER));

    assert_eq!(
        grid.cell(point).unwrap().owner(IntentKind::Defend),
        Some(opponent),
    );
}

#[test]
fn uncontested_capture_timer_resets_when_the_sole_holder_changes() {
    let mut grid = IntentGrid::new(4, 4);
    let point = IVec2::ZERO;
    let opponent = SwarmId(7);
    grid.paint_owned(point, IntentKind::Defend, Some(opponent));
    grid.contest_defend(point, SwarmId::PLAYER);

    for _ in 0..UNCONTESTED_CAPTURE_TICKS / 2 {
        assert_eq!(
            grid.update_defend_contest_presence(point, true, false),
            None
        );
    }
    for _ in 0..UNCONTESTED_CAPTURE_TICKS / 2 {
        assert_eq!(
            grid.update_defend_contest_presence(point, false, true),
            None
        );
    }

    assert_eq!(grid.cell(point).unwrap().owner(IntentKind::Defend), None);
}

#[test]
fn keyboard_selection_paints_and_erases_each_kind_independently() {
    let target = IVec2::ZERO;
    let mut app = App::new();
    app.insert_resource(IntentGrid::new(4, 4));
    app.init_resource::<BrushSelection>();
    app.init_resource::<ButtonInput<KeyCode>>();
    app.add_systems(Update, brush_selection_keyboard_system);

    for kind in IntentKind::ALL {
        press_key(&mut app, brush_key_for_kind(kind).unwrap());
        let selected = app.world().resource::<BrushSelection>().kind;
        assert_eq!(selected, kind);
        app.world_mut()
            .resource_mut::<IntentGrid>()
            .paint(target, selected);
    }

    let grid = app.world().resource::<IntentGrid>();
    for kind in IntentKind::ALL {
        assert!(grid.cell(target).unwrap().has(kind));
    }

    press_key(&mut app, brush_key_for_kind(IntentKind::Gather).unwrap());
    app.world_mut()
        .resource_mut::<IntentGrid>()
        .erase(target, IntentKind::Gather);
    let cell = app.world().resource::<IntentGrid>().cell(target).unwrap();
    assert!(!cell.has(IntentKind::Gather));
    for kind in [IntentKind::Build, IntentKind::Defend, IntentKind::Corridor] {
        assert!(cell.has(kind));
    }
}

#[test]
fn brush_selection_defaults_to_gather_and_persists() {
    let mut app = App::new();
    app.init_resource::<BrushSelection>();
    app.init_resource::<ButtonInput<KeyCode>>();
    app.add_systems(Update, brush_selection_keyboard_system);
    assert_eq!(
        app.world().resource::<BrushSelection>().kind,
        IntentKind::Gather
    );

    press_key(&mut app, KeyCode::Digit2);
    for _ in 0..5 {
        app.update();
    }
    assert_eq!(
        app.world().resource::<BrushSelection>().kind,
        IntentKind::Build
    );
}
