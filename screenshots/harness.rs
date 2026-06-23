//! Callback-driven screenshot harness.
//!
//! One screenshot "test" is a `fn(&mut TestContext) -> TestFlow`
//! called once per frame inside a real `app.run()` loop (winit on the
//! main thread, real window + render pipeline). The callback drives
//! game state through the context's `&mut World`, asserts on ECS
//! state, and requests captures via [`TestFlow::Screenshot`]. There
//! is no script language: the agent writes Rust that uses the same
//! seams `tests/common/mod.rs` exposes to the headless tests.
//!
//! Capture timing is signal-driven, not frame-guessed: when the
//! callback returns [`TestFlow::Screenshot`], a `Screenshot` entity is
//! spawned with `save_to_disk` plus a `ScreenshotCaptured` observer;
//! the callback is not called again until that observer fires, so the
//! PNG is guaranteed on disk before the next callback frame.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use bevy::{
    app::AppExit,
    prelude::*,
    render::view::screenshot::{save_to_disk, Screenshot, ScreenshotCaptured},
};

use top_down_2d_rts_prototype_nano_swarm::build_app;

/// Directory where screenshot artifacts are written. Under `target/`
/// so it is gitignored with the rest of the build output; artifacts
/// are never committed.
pub const SCREENSHOT_DIR: &str = "target/playtest-screenshots";

/// What a per-frame test callback returns to the driver.
pub enum TestFlow {
    /// Keep running the app; call the callback again next frame.
    Continue,
    /// Capture a screenshot as `target/playtest-screenshots/<name>.png`
    /// this frame, then resume the callback next frame after the PNG
    /// has landed on disk.
    Screenshot(String),
    /// Stop the app and report success.
    Exit,
}

/// Per-frame context handed to a test callback.
pub struct TestContext<'w> {
    /// Full mutable world: drive intent paint, camera, production
    /// ratio, spawn entities, or read resources/components to assert.
    pub world: &'w mut World,
    /// Zero-based frame index since the app started running. Useful
    /// for phase-based callbacks ("for the first 60 frames do X,
    /// then capture").
    pub frame: u32,
}

#[derive(Resource)]
struct TestDriver {
    callback: fn(&mut TestContext) -> TestFlow,
    state: DriverState,
    frame: u32,
}

enum DriverState {
    /// Callback is called each frame.
    Running,
    /// Waiting for the `ScreenshotCaptured` observer to fire before
    /// resuming the callback.
    Waiting,
    /// Callback requested exit; waiting for the app to stop.
    Exiting,
}

/// Remembers the path of the last captured artifact so
/// [`run_screenshot_test`] can return it after `app.run()` returns.
///
/// `App::run()` empties the world (`mem::replace(self, App::empty())`),
/// so the path can't be read back from the world after run -- it must
/// travel out through this `Arc` side channel.
#[derive(Resource, Clone)]
struct LastCapture(Arc<Mutex<Option<PathBuf>>>);

/// Build the full app, drive it under the real winit runner with `cb`
/// called each frame, and return the captured artifact path on
/// success. Hard-fails (returns `Err`) if no window/render pipeline is
/// available or the callback panics -- consistent with the screenshot
/// evidence policy ("no window = fail, not a skip").
pub fn run_screenshot_test(cb: fn(&mut TestContext) -> TestFlow) -> Result<PathBuf, String> {
    std::fs::create_dir_all(SCREENSHOT_DIR).map_err(|e| format!("create {SCREENSHOT_DIR}: {e}"))?;

    let mut app = build_app();
    let last = LastCapture(Arc::new(Mutex::new(None)));
    app.insert_resource(last.clone());
    app.insert_resource(TestDriver {
        callback: cb,
        state: DriverState::Running,
        frame: 0,
    });
    app.add_systems(Update, test_driver_system);

    // `build_app` registers scene/camera setup on `Startup`; the winit
    // runner drives Startup on the first loop iteration. A panic
    // inside the callback or a system bubbles out of `app.run()`, so
    // catch it and report as a test failure rather than aborting the
    // whole harness.
    let exit = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| app.run()));
    let exit = match exit {
        Ok(e) => e,
        Err(payload) => {
            let msg = payload
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| payload.downcast_ref::<&str>().map(|s| s.to_string()))
                .unwrap_or_else(|| "panic in callback/system".to_string());
            return Err(format!("callback panicked: {msg}"));
        }
    };

    // `app.run()` empties the world, so read the captured path from the
    // `Arc` side channel rather than from `app.world()`.
    let captured = last.0.lock().unwrap().clone();
    match captured {
        Some(path) if path.exists() => Ok(path),
        Some(path) => Err(format!(
            "driver exited {exit:?} but artifact {} does not exist on disk",
            path.display()
        )),
        None => Err(format!(
            "driver exited {exit:?} but no screenshot was captured before exit; \
             the callback must return TestFlow::Screenshot at least once, or the \
             environment cannot create a window/render a frame (no window = fail)"
        )),
    }
}

/// Exclusive driver system: called once per frame with full `&mut
/// World`. The [`TestDriver`] resource is borrowed out of the world so
/// the callback can borrow the world mutably. When the callback
/// requests a screenshot, a `Screenshot` entity is spawned whose
/// `ScreenshotCaptured` observer flips the driver back to `Running`,
/// guaranteeing the PNG is on disk before the next callback frame.
fn test_driver_system(world: &mut World) {
    // Borrow the driver out of the world so the callback can borrow
    // the world mutably without a double-borrow of the resource.
    let Some(mut driver) = world.remove_resource::<TestDriver>() else {
        return;
    };

    match driver.state {
        DriverState::Exiting | DriverState::Waiting => {
            // Exiting: wait for the winit runner to observe the queued
            // AppExit. Waiting: wait for the ScreenshotCaptured observer
            // (spawned below) to flip state back to Running. Either way,
            // do not call the callback this frame.
            world.insert_resource(driver);
        }
        DriverState::Running => {
            let frame = driver.frame;
            driver.frame = frame.wrapping_add(1);

            let (flow, world) = {
                let mut ctx = TestContext { world, frame };
                let flow = (driver.callback)(&mut ctx);
                (flow, ctx.world)
            };

            match flow {
                TestFlow::Continue => {
                    driver.state = DriverState::Running;
                    world.insert_resource(driver);
                }
                TestFlow::Screenshot(name) => {
                    let path = PathBuf::from(SCREENSHOT_DIR).join(format!("{name}.png"));
                    let _ = std::fs::remove_file(&path);
                    if let Some(l) = world.get_resource::<LastCapture>() {
                        *l.0.lock().unwrap() = Some(path.clone());
                    }
                    world
                        .spawn(Screenshot::primary_window())
                        .observe(save_to_disk(path.clone()))
                        .observe(
                            move |_event: On<ScreenshotCaptured>,
                                  mut driver: ResMut<TestDriver>| {
                                driver.state = DriverState::Running;
                            },
                        );
                    driver.state = DriverState::Waiting;
                    world.insert_resource(driver);
                }
                TestFlow::Exit => {
                    // Queue AppExit so the winit runner stops cleanly.
                    // `World::write_message` lazily inserts the
                    // `Messages<AppExit>` resource if absent.
                    let _ = world.write_message(AppExit::Success);
                    driver.state = DriverState::Exiting;
                    world.insert_resource(driver);
                }
            }
        }
    }
}
