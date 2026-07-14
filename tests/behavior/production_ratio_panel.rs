//! Behavior coverage for segmented production-ratio UI.

use bevy::{prelude::*, ui::RelativeCursorPosition};
use top_down_2d_rts_prototype_nano_swarm::{
    nanobot::{
        Nanobot, NanobotType, ProductionRatio, SwarmId, SwarmMember, SwarmProduction,
        spawn_opponent_swarm,
    },
    ui::{
        FontsResource,
        production_ratio_panel::{
            ActualCompositionTick, HANDLE_WIDTH, HandleBoundary, ProductionRatioDragState,
            ProductionRatioHandle, ProductionRatioPanelRoot, ProductionRatioSegment,
            ProductionRatioTrack, ProductionRatioValueText, production_ratio_drag_system,
            setup_production_ratio_panel, update_production_ratio_panel,
        },
    },
};

#[path = "../common/mod.rs"]
mod common;

fn build_app() -> App {
    let mut app = common::minimal_app();
    app.insert_resource(FontsResource {
        font: Handle::default(),
    })
    .insert_resource(ProductionRatio::default())
    .init_resource::<ProductionRatioDragState>()
    .init_resource::<ButtonInput<MouseButton>>()
    .add_systems(Startup, setup_production_ratio_panel)
    .add_systems(
        Update,
        (production_ratio_drag_system, update_production_ratio_panel).chain(),
    );
    app.update();
    app
}

fn entity_for_handle(world: &mut World, boundary: HandleBoundary) -> Entity {
    world
        .query::<(Entity, &ProductionRatioHandle)>()
        .iter(world)
        .find(|(_, handle)| handle.0 == boundary)
        .map(|(entity, _)| entity)
        .expect("requested handle must exist")
}

fn set_track_cursor(app: &mut App, normalized_x: f32) {
    let mut query = app
        .world_mut()
        .query_filtered::<&mut RelativeCursorPosition, With<ProductionRatioTrack>>();
    let mut cursor = query.single_mut(app.world_mut()).unwrap();
    cursor.normalized = Some(Vec2::new(normalized_x - 0.5, 0.0));
    cursor.cursor_over = (0.0..=1.0).contains(&normalized_x);
}

fn set_hovered_handle(app: &mut App, boundary: HandleBoundary) {
    let target = entity_for_handle(app.world_mut(), boundary);
    let entities: Vec<_> = app
        .world_mut()
        .query::<(Entity, &ProductionRatioHandle)>()
        .iter(app.world())
        .map(|(entity, _)| entity)
        .collect();
    for entity in entities {
        let mut handle_entity = app.world_mut().entity_mut(entity);
        let mut cursor = handle_entity.get_mut::<RelativeCursorPosition>().unwrap();
        cursor.cursor_over = entity == target;
        cursor.normalized = (entity == target).then_some(Vec2::splat(0.5));
    }
}

fn press_and_drag(app: &mut App, boundary: HandleBoundary, normalized_x: f32) {
    set_hovered_handle(app, boundary);
    set_track_cursor(app, normalized_x);
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .press(MouseButton::Left);
    app.update();
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .clear();
}

fn release(app: &mut App) {
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .release(MouseButton::Left);
    app.update();
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .clear();
}

fn percent(value: Val) -> f32 {
    let Val::Percent(value) = value else {
        panic!("expected percent value, got {value:?}");
    };
    value
}

#[test]
fn panel_has_segmented_structure_persistent_labels_and_no_buttons() {
    let mut app = build_app();
    assert_eq!(
        app.world_mut()
            .query_filtered::<Entity, With<ProductionRatioPanelRoot>>()
            .iter(app.world())
            .count(),
        1
    );
    assert_eq!(
        app.world_mut()
            .query::<&ProductionRatioTrack>()
            .iter(app.world())
            .count(),
        1
    );
    assert_eq!(
        app.world_mut()
            .query::<&ProductionRatioSegment>()
            .iter(app.world())
            .count(),
        3
    );
    assert_eq!(
        app.world_mut()
            .query::<&ProductionRatioHandle>()
            .iter(app.world())
            .count(),
        2
    );
    assert_eq!(
        app.world_mut()
            .query::<&ActualCompositionTick>()
            .iter(app.world())
            .count(),
        2
    );
    assert_eq!(
        app.world_mut().query::<&Button>().iter(app.world()).count(),
        0,
        "segmented panel must not retain old +/- buttons"
    );

    let labels: Vec<_> = app
        .world_mut()
        .query::<(&ProductionRatioValueText, &Text)>()
        .iter(app.world())
        .map(|(marker, text)| (marker.0, text.0.clone()))
        .collect();
    assert_eq!(labels.len(), 3);
    for (kind, name) in [
        (NanobotType::Worker, "Worker 60%"),
        (NanobotType::Hauler, "Hauler 30%"),
        (NanobotType::Defender, "Defender 10%"),
    ] {
        assert!(
            labels
                .iter()
                .any(|label| label == &(kind, name.to_string()))
        );
    }
}

#[test]
fn ticks_are_track_children_hidden_without_player_nanobots() {
    let mut app = build_app();
    let track = app
        .world_mut()
        .query_filtered::<Entity, With<ProductionRatioTrack>>()
        .single(app.world())
        .unwrap();
    let children = app.world().entity(track).get::<Children>().unwrap();
    let tick_children = children
        .iter()
        .filter(|entity| {
            app.world()
                .entity(*entity)
                .contains::<ActualCompositionTick>()
        })
        .count();
    assert_eq!(tick_children, 2, "both current-mix ticks belong to track");
    for visibility in app
        .world_mut()
        .query_filtered::<&Visibility, With<ActualCompositionTick>>()
        .iter(app.world())
    {
        assert_eq!(*visibility, Visibility::Hidden);
    }
}

#[test]
fn ticks_show_player_mix_and_ignore_opponents() {
    let mut app = build_app();
    for kind in [
        NanobotType::Worker,
        NanobotType::Worker,
        NanobotType::Hauler,
        NanobotType::Defender,
    ] {
        app.world_mut()
            .spawn((Nanobot {}, kind, SwarmMember::new(SwarmId::PLAYER)));
    }
    for _ in 0..8 {
        app.world_mut().spawn((
            Nanobot {},
            NanobotType::Worker,
            SwarmMember::new(SwarmId(9)),
        ));
    }
    app.update();

    for (tick, node, visibility) in app
        .world_mut()
        .query::<(&ActualCompositionTick, &Node, &Visibility)>()
        .iter(app.world())
    {
        assert_eq!(*visibility, Visibility::Visible);
        let expected = match tick.0 {
            HandleBoundary::WorkerEnd => 50.0,
            HandleBoundary::HaulerEnd => 75.0,
        };
        assert_eq!(percent(node.left), expected);
    }
}

#[test]
fn zero_share_updates_segments_and_labels_without_removing_them() {
    let mut app = build_app();
    press_and_drag(&mut app, HandleBoundary::WorkerEnd, 0.0);
    release(&mut app);

    let ratio = app.world().resource::<ProductionRatio>();
    assert_eq!(ratio.weight(NanobotType::Worker), 0);
    assert_eq!(ratio.weight(NanobotType::Hauler), 90);
    assert_eq!(ratio.weight(NanobotType::Defender), 10);
    let worker = app
        .world_mut()
        .query::<(&ProductionRatioSegment, &Node)>()
        .iter(app.world())
        .find(|(segment, _)| segment.0 == NanobotType::Worker)
        .unwrap();
    assert_eq!(percent(worker.1.width), 0.0);
    assert!(
        app.world_mut()
            .query::<(&ProductionRatioValueText, &Text)>()
            .iter(app.world())
            .any(|(label, text)| label.0 == NanobotType::Worker && text.0 == "Worker 0%")
    );
}

#[test]
fn only_hovered_handle_starts_drag_and_drag_reaches_edges() {
    let mut app = build_app();
    set_track_cursor(&mut app, 0.2);
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .press(MouseButton::Left);
    app.update();
    assert_eq!(
        app.world()
            .resource::<ProductionRatio>()
            .weight(NanobotType::Worker),
        6,
        "track press away from handles must not select nearest boundary"
    );
    release(&mut app);

    press_and_drag(&mut app, HandleBoundary::WorkerEnd, 0.0);
    assert_eq!(
        app.world()
            .resource::<ProductionRatio>()
            .weight(NanobotType::Worker),
        0
    );
    release(&mut app);
    press_and_drag(&mut app, HandleBoundary::HaulerEnd, 1.0);
    assert_eq!(
        app.world()
            .resource::<ProductionRatio>()
            .weight(NanobotType::Defender),
        0
    );
}

#[test]
fn coincident_handles_are_offset_and_independently_selectable() {
    let mut app = build_app();
    press_and_drag(&mut app, HandleBoundary::WorkerEnd, 0.9);
    release(&mut app);

    let offsets: Vec<_> = app
        .world_mut()
        .query::<(&ProductionRatioHandle, &Node)>()
        .iter(app.world())
        .map(|(handle, node)| (handle.0, node.margin.left))
        .collect();
    assert!(offsets.contains(&(HandleBoundary::WorkerEnd, Val::Px(-HANDLE_WIDTH))));
    assert!(offsets.contains(&(HandleBoundary::HaulerEnd, Val::Px(0.0))));

    press_and_drag(&mut app, HandleBoundary::WorkerEnd, 0.4);
    release(&mut app);
    assert_eq!(
        app.world()
            .resource::<ProductionRatio>()
            .weight(NanobotType::Worker),
        40
    );

    press_and_drag(&mut app, HandleBoundary::HaulerEnd, 1.0);
    assert_eq!(
        app.world()
            .resource::<ProductionRatio>()
            .weight(NanobotType::Defender),
        0
    );
}

#[test]
fn handles_and_track_participate_in_world_pointer_capture() {
    let mut app = build_app();
    let targets: Vec<_> = app
        .world_mut()
        .query_filtered::<Entity, Or<(With<ProductionRatioTrack>, With<ProductionRatioHandle>)>>()
        .iter(app.world())
        .collect();
    assert_eq!(targets.len(), 3);
    for entity in targets {
        assert!(
            app.world()
                .entity(entity)
                .contains::<RelativeCursorPosition>()
        );
    }
}

#[test]
fn drag_changes_only_player_ratio_not_opponent_production() {
    let mut app = build_app();
    let mut opponent_ratio = ProductionRatio::new();
    opponent_ratio.set_weight(NanobotType::Worker, 8);
    opponent_ratio.set_weight(NanobotType::Hauler, 4);
    opponent_ratio.set_weight(NanobotType::Defender, 3);
    let opponent = spawn_opponent_swarm(
        app.world_mut(),
        Vec2::new(2000.0, 0.0),
        opponent_ratio,
        &[],
        &[],
    );
    let before = app
        .world()
        .entity(opponent)
        .get::<SwarmProduction>()
        .unwrap()
        .ratio
        .weights
        .clone();

    press_and_drag(&mut app, HandleBoundary::WorkerEnd, 0.4);
    release(&mut app);

    assert_eq!(
        app.world()
            .entity(opponent)
            .get::<SwarmProduction>()
            .unwrap()
            .ratio
            .weights,
        before
    );
    assert_eq!(
        app.world()
            .resource::<ProductionRatio>()
            .weight(NanobotType::Worker),
        40
    );
}
