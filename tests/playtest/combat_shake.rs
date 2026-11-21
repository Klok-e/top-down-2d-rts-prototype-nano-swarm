use std::time::Duration;

use bevy::{
    prelude::*,
    render::{RenderApp, sync_world::RenderEntity, view::ExtractedView},
    time::TimeUpdateStrategy,
};
use serde_json::{Value, json};
use top_down_2d_rts_prototype_nano_swarm::{
    MainCamera, Presentation, ZONE_BLOCK_SIZE,
    agent_control::{
        AgentCommand, AgentControlCorePlugin, AgentControlHandle, AgentRequest, RequestId,
    },
    build_app_with_presentation,
    intent::{BrushSelection, IntentGrid, IntentKind},
    nanobot::{
        CombatAppearance, CombatVisualSnapshot, NanobotType, NonCombatDestruction,
        ResolvedCombatDeath, ResolvedCombatFact, ResolvedCombatHit, StructureCombatAppearance,
        StructureKind, SwarmId,
    },
    scenario_selection::ScenarioSelection,
    tuning::Tuning,
    zones::get_zone_pos_from_world,
};

const VIEWPORT: UVec2 = UVec2::new(640, 360);
const CENTER: Vec2 = Vec2::splat(ZONE_BLOCK_SIZE);
const ZOOM: f32 = 6.0;

fn shake_app() -> (App, AgentControlHandle, Entity) {
    let mut app = build_app_with_presentation(Presentation::Offscreen {
        width: VIEWPORT.x,
        height: VIEWPORT.y,
    });
    let (control, plugin) = AgentControlCorePlugin::channel(1);
    app.add_plugins(plugin)
        .insert_resource(ScenarioSelection::default())
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
            16,
        )));
    app.world_mut()
        .resource_mut::<Tuning>()
        .presentation
        .camera_shake_intensity = 1.0;
    while app.plugins_state() == bevy::app::PluginsState::Adding {
        bevy::tasks::tick_global_task_pools_on_main_thread();
    }
    app.finish();
    app.cleanup();
    app.update();
    let camera = app
        .world_mut()
        .query_filtered::<Entity, With<MainCamera>>()
        .single(app.world())
        .unwrap();
    assert_eq!(
        request(
            &mut app,
            &control,
            AgentCommand::CameraSet {
                x: CENTER.x,
                y: CENTER.y,
                zoom: Some(ZOOM)
            },
        ),
        json!({"x": CENTER.x, "y": CENTER.y, "zoom": ZOOM}),
    );
    (app, control, camera)
}

fn request(app: &mut App, control: &AgentControlHandle, command: AgentCommand) -> Value {
    let response = control
        .submit(AgentRequest {
            id: RequestId::Number(1),
            command,
        })
        .unwrap();
    app.update();
    let response = response.try_recv().unwrap();
    assert!(response.ok, "{:?}", response.error);
    response.result.unwrap()
}

fn extracted_center(app: &App, camera: Entity) -> Vec2 {
    let render_camera = app.world().get::<RenderEntity>(camera).unwrap().id();
    app.sub_app(RenderApp)
        .world()
        .get::<ExtractedView>(render_camera)
        .unwrap()
        .world_from_view
        .translation()
        .truncate()
}

fn destroy_structure(app: &mut App, position: Vec2) {
    let target = CombatVisualSnapshot {
        entity: Entity::from_raw_u32(900_000).unwrap(),
        position,
        swarm: SwarmId(1),
        appearance: CombatAppearance::Structure(StructureCombatAppearance {
            kind: StructureKind::Basic,
            visual: None,
        }),
    };
    let attacker = CombatVisualSnapshot {
        entity: Entity::PLACEHOLDER,
        position: position - Vec2::X * 100.0,
        swarm: SwarmId::PLAYER,
        appearance: CombatAppearance::Nanobot(NanobotType::Defender),
    };
    app.world_mut()
        .write_message(ResolvedCombatFact::Hit(ResolvedCombatHit {
            attacker,
            target,
            damage: 1,
            target_destroyed: true,
        }));
    app.world_mut()
        .write_message(ResolvedCombatFact::Death(ResolvedCombatDeath {
            victim: target,
        }));
}

#[test]
#[ignore = "requires a GPU adapter; run with `cargo test --test playtest combat_shake -- --ignored`"]
fn structure_shake_moves_only_the_rendered_view_and_respects_disabled_tuning() {
    let (mut app, control, camera) = shake_app();
    let outside_view = CENTER + Vec2::X * (VIEWPORT.x as f32 * ZOOM / 2.0 + 100.0);
    destroy_structure(&mut app, outside_view);
    app.update();
    assert_eq!(extracted_center(&app, camera), CENTER);

    destroy_structure(&mut app, CENTER);
    app.update();
    let offset = extracted_center(&app, camera) - CENTER;
    let pixels = offset.length() / ZOOM;
    assert!(
        pixels > 1.0 && pixels <= 4.001,
        "render displacement was {pixels} pixels"
    );

    let brush_position = CENTER - offset * 0.5;
    let intended_cell = get_zone_pos_from_world(brush_position);
    let shifted_cell = get_zone_pos_from_world(brush_position + offset);
    assert_ne!(
        intended_cell, shifted_cell,
        "the shake would cross the brush's cell boundary"
    );
    let brush_offset = (brush_position - CENTER) / ZOOM;
    let mut window = Window {
        resolution: (VIEWPORT.x, VIEWPORT.y).into(),
        ..default()
    };
    window.set_cursor_position(Some(
        VIEWPORT.as_vec2() / 2.0 + Vec2::new(brush_offset.x, -brush_offset.y),
    ));
    app.world_mut().spawn(window);
    app.world_mut().resource_mut::<BrushSelection>().kind = IntentKind::Corridor;
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .press(MouseButton::Left);
    app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::ZERO));
    let state = request(
        &mut app,
        &control,
        AgentCommand::StateGet {
            cell_offset: 0,
            cell_limit: 1,
            map_revision: None,
            details: false,
        },
    );
    assert_eq!(
        state["camera"],
        json!({"x": CENTER.x, "y": CENTER.y, "zoom": ZOOM})
    );
    let grid = app.world().resource::<IntentGrid>();
    assert!(
        grid.cell(intended_cell)
            .unwrap()
            .has_owned(IntentKind::Corridor, SwarmId::PLAYER)
    );
    assert!(
        !grid
            .cell(shifted_cell)
            .unwrap()
            .has_owned(IntentKind::Corridor, SwarmId::PLAYER)
    );

    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .release(MouseButton::Left);
    let new_center = CENTER + Vec2::splat(20.0);
    assert_eq!(
        request(
            &mut app,
            &control,
            AgentCommand::CameraSet {
                x: new_center.x,
                y: new_center.y,
                zoom: Some(ZOOM)
            },
        ),
        json!({"x": new_center.x, "y": new_center.y, "zoom": ZOOM}),
    );
    assert_ne!(
        extracted_center(&app, camera),
        new_center,
        "setting the center preserves the active shake"
    );

    app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
        50,
    )));
    for _ in 0..3 {
        app.update();
    }
    assert_eq!(
        extracted_center(&app, camera),
        new_center,
        "a shake ends within 150 ms"
    );

    destroy_structure(&mut app, new_center);
    app.update();
    assert_ne!(extracted_center(&app, camera), new_center);
    app.world_mut()
        .resource_mut::<Tuning>()
        .presentation
        .camera_shake_intensity = 0.0;
    destroy_structure(&mut app, new_center);
    app.update();
    assert_eq!(
        extracted_center(&app, camera),
        new_center,
        "zero intensity clears an existing shake and suppresses the next one"
    );
}

fn resolve_casualties(app: &mut App, entities: std::ops::Range<u32>, position: Vec2) {
    for index in entities {
        app.world_mut()
            .write_message(ResolvedCombatFact::Death(ResolvedCombatDeath {
                victim: CombatVisualSnapshot {
                    entity: Entity::from_raw_u32(index).unwrap(),
                    position,
                    swarm: SwarmId(1),
                    appearance: CombatAppearance::Nanobot(NanobotType::Defender),
                },
            }));
    }
    app.update();
}

#[test]
#[ignore = "requires a GPU adapter; run with `cargo test --test playtest combat_shake -- --ignored`"]
fn only_a_recent_visible_cluster_of_ten_casualties_shakes_the_view() {
    let (mut app, _, camera) = shake_app();
    let cluster = CENTER + Vec2::splat(20.0);
    let outside_view = CENTER + Vec2::X * (VIEWPORT.x as f32 * ZOOM / 2.0 + 100.0);
    resolve_casualties(&mut app, 100_000..100_010, outside_view);
    assert_eq!(extracted_center(&app, camera), CENTER);

    destroy_structure(&mut app, cluster);
    app.update();
    assert_ne!(extracted_center(&app, camera), CENTER);
    app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
        50,
    )));
    for _ in 0..3 {
        app.update();
    }
    resolve_casualties(&mut app, 100_010..100_019, cluster);
    assert_eq!(
        extracted_center(&app, camera),
        CENTER,
        "a structure destroyed 200 ms earlier does not count as a nanobot casualty"
    );
    resolve_casualties(&mut app, 100_019..100_020, cluster + Vec2::X * 256.0);
    assert_eq!(extracted_center(&app, camera), CENTER);

    for _ in 0..6 {
        app.update();
    }
    resolve_casualties(&mut app, 100_020..100_029, cluster);
    assert_eq!(extracted_center(&app, camera), CENTER);
    let occurred_at = app.world().resource::<Time<Fixed>>().elapsed();
    app.world_mut()
        .write_message(NonCombatDestruction::Nanobot {
            victim: CombatVisualSnapshot {
                entity: Entity::from_raw_u32(100_029).unwrap(),
                position: cluster,
                swarm: SwarmId(1),
                appearance: CombatAppearance::Nanobot(NanobotType::Worker),
            },
            occurred_at,
        });
    app.update();
    assert_ne!(extracted_center(&app, camera), CENTER);
}
