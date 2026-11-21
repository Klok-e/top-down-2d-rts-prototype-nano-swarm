use bevy::{
    app::TerminalCtrlCHandlerPlugin,
    camera::RenderTarget,
    log::LogPlugin,
    prelude::*,
    render::{pipelined_rendering::PipelinedRenderingPlugin, render_resource::TextureUsages},
    time::TimeUpdateStrategy,
};
use top_down_2d_rts_prototype_nano_swarm::{
    Presentation, ZONE_BLOCK_SIZE, build_app_with_presentation,
    fly_camera::CameraZoom2d,
    intent::{IntentGrid, IntentKind},
    nanobot::{
        ActiveCombatPulses, DefenderResponse, Health, Nanobot, NanobotSprites, NanobotType,
        OpponentSwarm, Swarm, SwarmId, SwarmMember, VelocityComponent,
    },
    scenario::{PLAYER_CELL, cell_origin},
    structure_overlay::{StructureOverlay, StructureOverlayKind},
    tactical_overlay::{TacticalClusterKey, TacticalMarkerKind},
};

use std::{collections::HashMap, time::Duration};

#[path = "../common/mod.rs"]
mod common;

fn finish_plugins(app: &mut App) {
    while app.plugins_state() == bevy::app::PluginsState::Adding {
        bevy::tasks::tick_global_task_pools_on_main_thread();
    }
    app.finish();
    app.cleanup();
}

#[test]
#[ignore = "requires a GPU adapter; run with `cargo test --test playtest offscreen_presentation -- --ignored`"]
fn offscreen_presentation_starts_full_scene_without_a_window() {
    let mut app = build_app_with_presentation(Presentation::Offscreen {
        width: 640,
        height: 360,
    });
    assert!(
        !app.is_plugin_added::<LogPlugin>(),
        "offscreen Apps must not reinstall Bevy's process-global LogPlugin"
    );
    assert!(
        !app.is_plugin_added::<TerminalCtrlCHandlerPlugin>(),
        "offscreen Apps must not reinstall Bevy's process-global Ctrl-C handler"
    );
    assert!(
        !app.is_plugin_added::<PipelinedRenderingPlugin>(),
        "offscreen Apps must render in lockstep with manual updates"
    );
    finish_plugins(&mut app);
    app.update();

    assert_eq!(
        app.world_mut().query::<&Window>().iter(app.world()).count(),
        0
    );

    let opponent = app
        .world_mut()
        .query_filtered::<&SwarmId, (With<Swarm>, With<OpponentSwarm>)>()
        .single(app.world())
        .expect("authored opponent swarm must exist")
        .to_owned();
    let sprites = app.world().resource::<NanobotSprites>().clone();
    let nanobots = app
        .world_mut()
        .query_filtered::<(Entity, &NanobotType, &SwarmMember), With<Nanobot>>()
        .iter(app.world())
        .map(|(entity, kind, member)| (entity, *kind, member.0))
        .collect::<Vec<_>>();
    assert!(!nanobots.is_empty(), "full startup must spawn nanobots");
    let mut type_icons = HashMap::new();
    for (root, kind, member) in nanobots {
        assert!(
            app.world().get::<ChildOf>(root).is_none(),
            "full-app nanobot roots must remain top-level"
        );
        assert!(
            app.world().get::<Sprite>(root).is_none(),
            "full-app nanobot roots must not retain direct sprites"
        );
        let visual = common::nanobot_visual_child(app.world(), root);
        assert_eq!(
            app.world()
                .get::<Sprite>(visual)
                .expect("visual child must render a Sprite")
                .image,
            sprites.handle(kind, member == opponent)
        );
        let type_icon = app
            .world()
            .get::<Children>(visual)
            .and_then(|children| {
                children
                    .iter()
                    .find_map(|child| app.world().get::<Sprite>(child)?.rect)
            })
            .expect("every nanobot needs a visible type insignia");
        if let Some(other_icon) = type_icons.insert(kind, type_icon) {
            assert_eq!(
                type_icon, other_icon,
                "both swarms use the same type symbols"
            );
        }
    }
    for (index, kind) in NanobotType::ALL.iter().enumerate() {
        for other in &NanobotType::ALL[..index] {
            assert_ne!(type_icons[kind], type_icons[other]);
        }
    }

    let deposit_fill = app
        .world_mut()
        .query::<(&StructureOverlay, &InheritedVisibility)>()
        .iter(app.world())
        .find(|(overlay, visibility)| {
            overlay.kind == StructureOverlayKind::Deposit && visibility.get()
        })
        .map(|(overlay, _)| overlay.fill)
        .expect("a Resource Deposit has a visible amount bar at opening zoom");
    let color = app
        .world()
        .get::<Sprite>(deposit_fill)
        .unwrap()
        .color
        .to_srgba();
    assert!(
        color.red > color.green && color.green > color.blue,
        "deposit quantity must read as amber: {color:?}"
    );

    let (camera, image_handle) = {
        let mut cameras = app
            .world_mut()
            .query_filtered::<(Entity, &RenderTarget), (With<Camera2d>, With<IsDefaultUiCamera>)>();
        let (camera, target) = cameras
            .single(app.world())
            .expect("full startup must spawn one main 2D camera for Bevy UI");
        let image_handle = target
            .as_image()
            .expect("offscreen camera must target an Image")
            .clone();
        (camera, image_handle)
    };

    let image = app
        .world()
        .resource::<Assets<Image>>()
        .get(&image_handle)
        .expect("camera target must remain in Assets<Image>");
    assert_eq!((image.width(), image.height()), (640, 360));
    let usages = image.texture_descriptor.usage;
    assert!(usages.contains(TextureUsages::RENDER_ATTACHMENT));
    assert!(usages.contains(TextureUsages::TEXTURE_BINDING));
    assert!(usages.contains(TextureUsages::COPY_DST));
    assert!(usages.contains(TextureUsages::COPY_SRC));

    let scene_meshes = app.world_mut().query::<&Mesh2d>().iter(app.world()).count();
    assert!(
        scene_meshes >= 2,
        "startup must spawn background and zone scene meshes"
    );

    let mut ui_roots = app
        .world_mut()
        .query_filtered::<&ComputedUiTargetCamera, (With<Node>, Without<ChildOf>)>();
    let targeted_roots = ui_roots
        .iter(app.world())
        .map(|target| target.get())
        .collect::<Vec<_>>();
    assert!(
        !targeted_roots.is_empty(),
        "full startup must spawn Bevy UI"
    );
    assert!(
        targeted_roots.iter().all(|target| *target == Some(camera)),
        "every root UI node must render through main offscreen camera"
    );

    let mut view = app.world_mut().entity_mut(camera);
    view.get_mut::<CameraZoom2d>().unwrap().zoom = 30.0;
    if let Projection::Orthographic(ortho) = &mut *view.get_mut::<Projection>().unwrap() {
        ortho.scale = 30.0;
    }
    app.update();

    let mut mark_icons = HashMap::new();
    let wall_center = cell_origin(PLAYER_CELL) + Vec2::splat(12.0 * ZONE_BLOCK_SIZE);
    let wall_half = 14.0 * ZONE_BLOCK_SIZE;
    let corner_radius = 0.85 * ZONE_BLOCK_SIZE;
    for (key, transform, sprite, visibility) in app
        .world_mut()
        .query::<(
            &TacticalClusterKey,
            &Transform,
            &Sprite,
            &InheritedVisibility,
        )>()
        .iter(app.world())
    {
        assert!(visibility.get(), "far zoom must reveal the tactical icons");
        let half_size = transform.scale.truncate() * sprite.custom_size.unwrap() / 2.0;
        for corner in [
            Vec2::ONE,
            Vec2::new(-1., 1.),
            Vec2::new(1., -1.),
            Vec2::NEG_ONE,
        ] {
            let point = transform.translation.truncate() + half_size * corner;
            let round_corner = (point - wall_center).abs() - (wall_half - corner_radius);
            assert!(
                round_corner.max(Vec2::ZERO).length() <= corner_radius,
                "{:?} icon extends outside the map wall at {point:?}",
                key.kind,
            );
        }
        if key.kind == TacticalMarkerKind::Facility {
            let color = sprite.color.to_srgba();
            assert!(
                if key.owner == SwarmId::PLAYER {
                    color.blue > color.red
                } else {
                    color.red > color.blue
                },
                "facility icons must retain the owning swarm's appearance"
            );
        }
        mark_icons.insert(
            key.kind,
            sprite
                .rect
                .expect("a landmark needs a recognizable pictogram"),
        );
    }
    assert!(mark_icons.len() >= 4);
    for (kind, icon) in &mark_icons {
        assert!(
            mark_icons
                .iter()
                .all(|(other, other_icon)| kind == other || icon != other_icon)
        );
    }
}

#[test]
#[ignore = "requires a GPU adapter; run with `cargo test --test playtest full_app_offscreen_combat -- --ignored`"]
fn full_app_offscreen_combat_uses_real_facts_and_keeps_gameplay_roots_fixed() {
    let mut app = build_app_with_presentation(Presentation::Offscreen {
        width: 640,
        height: 360,
    });
    finish_plugins(&mut app);
    app.update();

    let authored_nanobots = app
        .world_mut()
        .query_filtered::<Entity, With<Nanobot>>()
        .iter(app.world())
        .collect::<Vec<_>>();
    for entity in authored_nanobots {
        app.world_mut().despawn(entity);
    }

    let opponent = *app
        .world_mut()
        .query_filtered::<&SwarmId, (With<Swarm>, With<OpponentSwarm>)>()
        .single(app.world())
        .expect("full app needs its authored Opponent Swarm");
    let cell = IVec2::new(0, 5);
    let center = common::cell_world_center(cell);
    app.world_mut()
        .resource_mut::<IntentGrid>()
        .paint(cell, IntentKind::Defend, SwarmId::PLAYER);
    let attacker = common::spawn_defender_at(&mut app, center + Vec2::new(-44.0, 0.0));
    app.world_mut()
        .entity_mut(attacker)
        .remove::<VelocityComponent>();
    let target = common::spawn_worker_at(&mut app, center + Vec2::new(44.0, 0.0));
    app.world_mut()
        .entity_mut(target)
        .insert(SwarmMember::new(opponent))
        .remove::<VelocityComponent>();
    app.world_mut()
        .entity_mut(attacker)
        .insert(DefenderResponse { target });
    let attacker_root = *app.world().get::<Transform>(attacker).unwrap();
    let target_root = *app.world().get::<Transform>(target).unwrap();
    app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
        17,
    )));
    app.update();

    assert_eq!(
        app.world().get::<Health>(target).unwrap().current,
        90,
        "the full app must apply its ordinary Defender damage before presentation",
    );
    assert_eq!(app.world().resource::<ActiveCombatPulses>().len(), 1);
    let attacker_visual = common::nanobot_visual_child(app.world(), attacker);
    let target_visual = common::nanobot_visual_child(app.world(), target);
    assert!(
        app.world()
            .get::<Transform>(attacker_visual)
            .unwrap()
            .translation
            .length()
            > 0.0,
    );
    assert_ne!(
        app.world().get::<Sprite>(target_visual).unwrap().color,
        Color::WHITE,
    );
    assert_eq!(app.world().get::<Transform>(attacker), Some(&attacker_root));
    assert_eq!(app.world().get::<Transform>(target), Some(&target_root));
    assert_eq!(
        app.world_mut().query::<&Window>().iter(app.world()).count(),
        0,
        "the full-app combat playtest must not create a window",
    );
}
