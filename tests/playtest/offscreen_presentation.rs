use bevy::{
    app::TerminalCtrlCHandlerPlugin,
    camera::RenderTarget,
    log::LogPlugin,
    prelude::*,
    render::{pipelined_rendering::PipelinedRenderingPlugin, render_resource::TextureUsages},
};
use top_down_2d_rts_prototype_nano_swarm::{Presentation, build_app_with_presentation};

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
}
