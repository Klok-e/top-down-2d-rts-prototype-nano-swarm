//! Callback-driven offscreen screenshot harness.
//!
//! One screenshot test is a `fn(&mut TestContext) -> TestFlow` called during
//! each gameplay update. Tests mutate full game state through
//! `TestContext::world`; [`TestFlow::Screenshot`] pauses callback execution until
//! GPU readback, PNG writing, and artifact validation complete, then resumes it.

use std::{
    any::Any,
    path::{Component, Path, PathBuf},
    time::Duration,
};

use bevy::{prelude::*, time::TimeUpdateStrategy};
use image::GenericImageView;

use top_down_2d_rts_prototype_nano_swarm::{Presentation, build_app_with_presentation};

/// Directory where screenshot artifacts are written. Under `target/` so it is
/// gitignored with the rest of the build output; artifacts are never committed.
pub const SCREENSHOT_DIR: &str = "target/playtest-screenshots";

const WIDTH: u32 = 1280;
const HEIGHT: u32 = 720;
const MAX_CALLBACK_FRAMES: u32 = 10_000;
const MAX_READBACK_UPDATES: u32 = 600;
const MAX_PLUGIN_READY_TICKS: u32 = 10_000;
const STARTUP_RENDER_UPDATES: u32 = 10;

/// What a per-update test callback returns to driver.
pub enum TestFlow {
    /// Keep running app; call callback again next gameplay update.
    Continue,
    /// Capture `target/playtest-screenshots/<name>.png`. Callback pauses until
    /// artifact exists and validates, then resumes on next gameplay update.
    Screenshot(String),
    /// Stop app. Succeeds only after at least one validated capture.
    Exit,
}

/// Per-gameplay-update context handed to a test callback.
pub struct TestContext<'w> {
    /// Full mutable world: drive game state or inspect ECS state.
    pub world: &'w mut World,
    /// Zero-based callback frame.
    pub frame: u32,
}

#[derive(Clone, Copy)]
struct HarnessLimits {
    max_callback_frames: u32,
    max_readback_updates: u32,
}

impl Default for HarnessLimits {
    fn default() -> Self {
        Self {
            max_callback_frames: MAX_CALLBACK_FRAMES,
            max_readback_updates: MAX_READBACK_UPDATES,
        }
    }
}

#[derive(Resource)]
struct TestDriver {
    callback: fn(&mut TestContext) -> TestFlow,
    callback_frame: u32,
    max_callback_frames: u32,
    state: DriverState,
    failure: Option<String>,
}

enum DriverState {
    Running,
    CaptureRequested(String),
    Waiting { render_updates: u32 },
    Exiting,
}

/// Build full app with offscreen presentation, drive manual updates, and return
/// final captured artifact. Missing captures, callback panics, invalid images,
/// and stalled callbacks/readbacks are failures.
pub fn run_screenshot_test(cb: fn(&mut TestContext) -> TestFlow) -> Result<PathBuf, String> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        run_screenshot_test_with_limits(cb, HarnessLimits::default())
    })) {
        Ok(result) => result,
        Err(payload) => Err(format!(
            "screenshot harness panicked: {}",
            panic_message(payload.as_ref())
        )),
    }
}

fn run_screenshot_test_with_limits(
    cb: fn(&mut TestContext) -> TestFlow,
    limits: HarnessLimits,
) -> Result<PathBuf, String> {
    std::fs::create_dir_all(SCREENSHOT_DIR)
        .map_err(|err| format!("create {SCREENSHOT_DIR}: {err}"))?;

    let mut app = build_app_with_presentation(Presentation::Offscreen {
        width: WIDTH,
        height: HEIGHT,
    });
    app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_micros(
        16_667,
    )));

    let exporter = image_export_adapter::OffscreenImageExporter::install(&mut app);
    let mut session = ScreenshotSession::new(app, exporter);
    finish_plugins(&mut session.app)?;
    for _ in 0..STARTUP_RENDER_UPDATES {
        update_app(&mut session.app)?;
    }

    prime_readback_path(&mut session)?;

    session
        .app
        .insert_resource(TestDriver {
            callback: cb,
            callback_frame: 0,
            max_callback_frames: limits.max_callback_frames,
            state: DriverState::Running,
            failure: None,
        })
        .add_systems(First, test_driver_system);
    let run_result = drive_screenshot_test(&mut session, limits);
    let cleanup_result = session.cleanup();
    match (run_result, cleanup_result) {
        (Ok(path), Ok(())) => Ok(path),
        (Err(run_error), Ok(())) => Err(run_error),
        (Ok(_), Err(cleanup_error)) => Err(format!("screenshot cleanup failed: {cleanup_error}")),
        (Err(run_error), Err(cleanup_error)) => Err(format!(
            "{run_error}; screenshot cleanup failed: {cleanup_error}"
        )),
    }
}

struct ScreenshotSession {
    app: App,
    exporter: image_export_adapter::OffscreenImageExporter,
    cleaned_up: bool,
}

impl ScreenshotSession {
    fn new(app: App, exporter: image_export_adapter::OffscreenImageExporter) -> Self {
        Self {
            app,
            exporter,
            cleaned_up: false,
        }
    }

    fn cleanup(&mut self) -> Result<(), String> {
        let result = self.exporter.cleanup(self.app.world_mut());
        self.cleaned_up = result.is_ok();
        result
    }
}

impl Drop for ScreenshotSession {
    fn drop(&mut self) {
        if !self.cleaned_up {
            let _ = self.exporter.cleanup(self.app.world_mut());
        }
    }
}

/// Primes dependency-owned render asset and GPU readback state before trial's
/// first callback. Priming uses normal app updates and can advance gameplay.
fn prime_readback_path(session: &mut ScreenshotSession) -> Result<(), String> {
    let destination = PathBuf::from(SCREENSHOT_DIR).join(".readback-prime.png");
    let artifact = TemporaryArtifact::new(destination);
    session
        .exporter
        .begin(session.app.world_mut(), artifact.path().to_path_buf())?;

    for _ in 0..MAX_READBACK_UPDATES {
        update_app(&mut session.app)?;
        match session.exporter.poll(session.app.world_mut())? {
            image_export_adapter::CaptureStatus::Pending => {}
            image_export_adapter::CaptureStatus::Complete(_) => return artifact.remove(),
        }
    }

    Err(format!(
        "readback priming exhausted update budget of {MAX_READBACK_UPDATES}; \
         budget counts completed app updates and cannot preempt blocking GPU polling"
    ))
}

struct TemporaryArtifact {
    path: PathBuf,
    keep: bool,
}

impl TemporaryArtifact {
    fn new(path: PathBuf) -> Self {
        Self { path, keep: false }
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn remove(&self) -> Result<(), String> {
        if self.path.exists() {
            std::fs::remove_file(&self.path).map_err(|err| {
                format!("remove temporary artifact {}: {err}", self.path.display())
            })?;
        }
        Ok(())
    }

    fn persist(mut self) {
        self.keep = true;
    }
}

impl Drop for TemporaryArtifact {
    fn drop(&mut self) {
        if !self.keep {
            let _ = self.remove();
        }
    }
}

fn drive_screenshot_test(
    session: &mut ScreenshotSession,
    limits: HarnessLimits,
) -> Result<PathBuf, String> {
    let mut capture_artifact = None;
    let mut last_captured_path = None;
    loop {
        {
            let driver = session.app.world().resource::<TestDriver>();
            if driver
                .state
                .readback_budget_exhausted(limits.max_readback_updates)
            {
                return Err(format!(
                    "screenshot readback exhausted update budget of {}; budget counts \
                     completed app updates and cannot preempt blocking GPU polling",
                    limits.max_readback_updates
                ));
            }
        }

        update_app(&mut session.app)?;

        let mut driver = session
            .app
            .world_mut()
            .remove_resource::<TestDriver>()
            .expect("test driver must remain installed");
        if let Some(failure) = driver.failure.take() {
            return Err(failure);
        }

        match std::mem::replace(&mut driver.state, DriverState::Running) {
            DriverState::Running => {}
            DriverState::CaptureRequested(name) => {
                let artifact = TemporaryArtifact::new(screenshot_path(&name)?);
                session
                    .exporter
                    .begin(session.app.world_mut(), artifact.path().to_path_buf())?;
                capture_artifact = Some(artifact);
                driver.state = DriverState::Waiting { render_updates: 0 };
            }
            DriverState::Waiting { render_updates } => {
                match session.exporter.poll(session.app.world_mut())? {
                    image_export_adapter::CaptureStatus::Pending => {
                        driver.state = DriverState::Waiting {
                            render_updates: render_updates + 1,
                        };
                    }
                    image_export_adapter::CaptureStatus::Complete(path) => {
                        if !path.is_file() {
                            return Err(format!(
                                "capture completed but artifact {} does not exist",
                                path.display()
                            ));
                        }
                        validate_artifact(&path)?;
                        capture_artifact
                            .take()
                            .expect("waiting capture must retain artifact guard")
                            .persist();
                        last_captured_path = Some(path);
                        driver.state = DriverState::Running;
                    }
                }
            }
            DriverState::Exiting => {
                return last_captured_path
                    .ok_or_else(|| "callback exited without capturing a screenshot".to_string());
            }
        }
        session.app.world_mut().insert_resource(driver);
    }
}

impl DriverState {
    fn readback_budget_exhausted(&self, max_readback_updates: u32) -> bool {
        matches!(
            self,
            Self::Waiting { render_updates } if *render_updates >= max_readback_updates
        )
    }
}

/// Runs callbacks in `First`, after startup and before gameplay systems.
fn test_driver_system(world: &mut World) {
    let Some(mut driver) = world.remove_resource::<TestDriver>() else {
        return;
    };

    if matches!(driver.state, DriverState::Running) {
        if driver.callback_frame >= driver.max_callback_frames {
            driver.failure = Some(format!(
                "callback exceeded maximum of {} frames without exiting",
                driver.max_callback_frames
            ));
        } else {
            let frame = driver.callback_frame;
            driver.callback_frame += 1;
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let mut ctx = TestContext { world, frame };
                (driver.callback)(&mut ctx)
            }));
            match result {
                Ok(TestFlow::Continue) => {}
                Ok(TestFlow::Screenshot(name)) => {
                    driver.state = DriverState::CaptureRequested(name);
                }
                Ok(TestFlow::Exit) => {
                    driver.state = DriverState::Exiting;
                }
                Err(payload) => {
                    driver.failure = Some(format!(
                        "callback panicked at frame {frame}: {}",
                        panic_message(payload.as_ref())
                    ));
                }
            }
        }
    }

    world.insert_resource(driver);
}

fn finish_plugins(app: &mut App) -> Result<(), String> {
    let mut ticks = 0;
    while app.plugins_state() == bevy::app::PluginsState::Adding {
        if ticks >= MAX_PLUGIN_READY_TICKS {
            return Err(format!(
                "plugins did not become ready after {MAX_PLUGIN_READY_TICKS} task-pool ticks"
            ));
        }
        bevy::tasks::tick_global_task_pools_on_main_thread();
        ticks += 1;
    }
    app.finish();
    app.cleanup();
    Ok(())
}

/// Runs one full app update. Capture waiting cannot use a render-only main-world
/// schedule through Bevy's public App API: extraction depends on normal main
/// schedules. Callback remains suspended while full app updates continue, so
/// gameplay state can advance. Callback must assert and reset relevant state
/// after resume before requesting another capture.
fn update_app(app: &mut App) -> Result<(), String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| app.update()))
        .map_err(|payload| format!("app update panicked: {}", panic_message(payload.as_ref())))
}

fn validate_artifact(path: &Path) -> Result<(), String> {
    let image = image::open(path)
        .map_err(|err| format!("decode captured PNG {}: {err}", path.display()))?;
    let dimensions = image.dimensions();
    if dimensions != (WIDTH, HEIGHT) {
        return Err(format!(
            "captured PNG {} has dimensions {dimensions:?}, expected ({WIDTH}, {HEIGHT})",
            path.display()
        ));
    }

    let has_visible_color = image
        .to_rgba8()
        .pixels()
        .any(|pixel| pixel[3] != 0 && (pixel[0] != 0 || pixel[1] != 0 || pixel[2] != 0));
    if !has_visible_color {
        return Err(format!(
            "captured PNG {} is fully black or transparent",
            path.display()
        ));
    }
    Ok(())
}

fn screenshot_path(name: &str) -> Result<PathBuf, String> {
    let name = Path::new(name);
    let valid = name
        .components()
        .all(|component| matches!(component, Component::Normal(_)));
    if name.as_os_str().is_empty() || !valid {
        return Err(format!(
            "screenshot name must be a non-empty relative path without traversal: {}",
            name.display()
        ));
    }
    Ok(PathBuf::from(SCREENSHOT_DIR)
        .join(name)
        .with_extension("png"))
}

fn panic_message(payload: &(dyn Any + Send)) -> String {
    payload
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| {
            payload
                .downcast_ref::<&str>()
                .map(|message| message.to_string())
        })
        .unwrap_or_else(|| "non-string panic payload".to_string())
}

/// Narrow adapter around `bevy_image_export`; harness code does not depend on
/// crate-specific exporter entities, sequence naming, or thread tracking.
mod image_export_adapter {
    use std::{
        path::{Path, PathBuf},
        sync::atomic::{AtomicU64, Ordering},
    };

    use bevy::{camera::RenderTarget, prelude::*};
    use bevy_image_export::{
        ExportThreads, ImageExport, ImageExportPlugin, ImageExportSettings, ImageExportSource,
    };

    static NEXT_RUN: AtomicU64 = AtomicU64::new(0);

    pub(super) enum CaptureStatus {
        Pending,
        Complete(PathBuf),
    }

    struct CaptureTempDir(PathBuf);

    impl CaptureTempDir {
        fn new(path: PathBuf) -> Self {
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }

        fn remove(&self) -> Result<(), String> {
            if self.0.exists() {
                std::fs::remove_dir_all(&self.0).map_err(|err| {
                    format!("remove capture directory {}: {err}", self.0.display())
                })?;
            }
            Ok(())
        }
    }

    impl Drop for CaptureTempDir {
        fn drop(&mut self) {
            let _ = self.remove();
        }
    }

    struct ActiveCapture {
        exporter: Option<Entity>,
        temp_dir: CaptureTempDir,
        source_path: PathBuf,
        destination: PathBuf,
    }

    pub(super) struct OffscreenImageExporter {
        threads: ExportThreads,
        active: Option<ActiveCapture>,
        source: Option<Handle<ImageExportSource>>,
        run_id: u64,
        capture_id: u64,
    }

    impl OffscreenImageExporter {
        pub(super) fn install(app: &mut App) -> Self {
            let plugin = ImageExportPlugin::default();
            let threads = plugin.threads.clone();
            app.add_plugins(plugin);
            Self {
                threads,
                active: None,
                source: None,
                run_id: NEXT_RUN.fetch_add(1, Ordering::Relaxed),
                capture_id: 0,
            }
        }

        pub(super) fn begin(
            &mut self,
            world: &mut World,
            destination: PathBuf,
        ) -> Result<(), String> {
            if self.active.is_some() {
                return Err("cannot start capture while another readback is active".to_string());
            }

            let source = match self.source.as_ref() {
                Some(source) => source.clone(),
                None => {
                    let image = world
                        .query_filtered::<&RenderTarget, With<Camera2d>>()
                        .iter(world)
                        .find_map(|target| target.as_image().cloned())
                        .ok_or_else(|| {
                            "offscreen app has no Camera2d targeting an Image; no fallback available"
                                .to_string()
                        })?;
                    let source = world.resource_mut::<Assets<ImageExportSource>>().add(image);
                    self.source = Some(source.clone());
                    source
                }
            };

            if let Some(parent) = destination.parent() {
                std::fs::create_dir_all(parent).map_err(|err| {
                    format!("create screenshot directory {}: {err}", parent.display())
                })?;
            }
            if destination.exists() {
                std::fs::remove_file(&destination).map_err(|err| {
                    format!("remove stale screenshot {}: {err}", destination.display())
                })?;
            }

            let temp_path = PathBuf::from(super::SCREENSHOT_DIR)
                .join(format!(".capture-{}-{}", self.run_id, self.capture_id));
            self.capture_id += 1;
            if temp_path.exists() {
                std::fs::remove_dir_all(&temp_path).map_err(|err| {
                    format!(
                        "remove stale capture directory {}: {err}",
                        temp_path.display()
                    )
                })?;
            }
            // Readback source stays alive across captures; priming prepares its GPU buffer.
            let source_path = temp_path.join("00001.png");
            let temp_dir = CaptureTempDir::new(temp_path);

            let entity = world
                .spawn((
                    ImageExport(source),
                    ImageExportSettings {
                        output_dir: temp_dir.path().to_string_lossy().into_owned(),
                        extension: "png".to_string(),
                    },
                ))
                .id();
            self.active = Some(ActiveCapture {
                exporter: Some(entity),
                source_path,
                temp_dir,
                destination,
            });
            Ok(())
        }

        pub(super) fn poll(&mut self, world: &mut World) -> Result<CaptureStatus, String> {
            let active = self
                .active
                .as_mut()
                .ok_or_else(|| "readback poll requested without active capture".to_string())?;

            let capture_ready = active.source_path.is_file();
            if capture_ready {
                if let Some(entity) = active.exporter.take() {
                    let _ = world.despawn(entity);
                }
            }
            if !capture_ready || !self.threads.is_finished() {
                return Ok(CaptureStatus::Pending);
            }

            if let Some(entity) = active.exporter.take() {
                let _ = world.despawn(entity);
            }
            std::fs::rename(&active.source_path, &active.destination).map_err(|err| {
                format!(
                    "move exported frame {} to {}: {err}",
                    active.source_path.display(),
                    active.destination.display()
                )
            })?;
            if !active.destination.is_file() {
                return Err(format!(
                    "PNG export finished but {} is missing",
                    active.destination.display()
                ));
            }
            let result = active.destination.clone();
            active.temp_dir.remove()?;
            self.active = None;
            Ok(CaptureStatus::Complete(result))
        }

        /// Despawns active exporter, waits until dependency-reported workers
        /// finish, then removes temp output. `bevy_image_export::ExportThreads`
        /// exposes only a worker count and blocking `finish()`: it retains no
        /// `JoinHandle` and no save-error channel. Cleanup can wait for count
        /// zero, but cannot join workers or recover their logged save errors.
        pub(super) fn cleanup(&mut self, world: &mut World) -> Result<(), String> {
            if let Some(active) = self.active.as_mut() {
                if let Some(entity) = active.exporter.take() {
                    let _ = world.despawn(entity);
                }
            }
            self.threads.finish();
            if let Some(active) = self.active.take() {
                active.temp_dir.remove()?;
            }
            Ok(())
        }
    }

    pub(super) fn temp_dir_guard_removes_tree() -> Result<(), String> {
        let path = PathBuf::from(super::SCREENSHOT_DIR).join(format!(
            ".capture-cleanup-regression-{}",
            NEXT_RUN.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path)
            .map_err(|err| format!("create temp cleanup fixture {}: {err}", path.display()))?;
        std::fs::write(path.join("partial.png"), b"partial")
            .map_err(|err| format!("write temp cleanup fixture {}: {err}", path.display()))?;
        {
            let _guard = CaptureTempDir::new(path.clone());
        }
        if path.exists() {
            return Err(format!(
                "capture temp guard left fixture behind at {}",
                path.display()
            ));
        }
        Ok(())
    }
}

pub(crate) mod regression {
    use std::{path::PathBuf, time::Duration};

    use bevy::prelude::{Resource, Time};
    use libtest_mimic::Failed;

    use super::{
        HEIGHT, HarnessLimits, SCREENSHOT_DIR, TemporaryArtifact, TestContext, TestFlow, WIDTH,
        image_export_adapter, run_screenshot_test, run_screenshot_test_with_limits,
    };

    const FIRST_CAPTURE: &str = "harness-regression-first";
    const SECOND_CAPTURE: &str = "harness-regression-second";

    #[derive(Resource)]
    struct CallbackCheckpoint(Duration);

    fn multi_capture_callback(ctx: &mut TestContext) -> TestFlow {
        let first_path = PathBuf::from(SCREENSHOT_DIR).join(format!("{FIRST_CAPTURE}.png"));
        let second_path = PathBuf::from(SCREENSHOT_DIR).join(format!("{SECOND_CAPTURE}.png"));
        match ctx.frame {
            0 => {
                let elapsed = ctx.world.resource::<Time>().elapsed();
                ctx.world.insert_resource(CallbackCheckpoint(elapsed));
                TestFlow::Screenshot(FIRST_CAPTURE.to_string())
            }
            1 => {
                assert!(
                    first_path.is_file(),
                    "callback resumed before first artifact existed"
                );
                let elapsed = ctx.world.resource::<Time>().elapsed();
                let prior = ctx.world.resource::<CallbackCheckpoint>().0;
                assert!(
                    elapsed > prior,
                    "full app time did not advance during first readback"
                );
                ctx.world.resource_mut::<CallbackCheckpoint>().0 = elapsed;
                TestFlow::Screenshot(SECOND_CAPTURE.to_string())
            }
            2 => {
                assert!(
                    second_path.is_file(),
                    "callback resumed before second artifact existed"
                );
                let elapsed = ctx.world.resource::<Time>().elapsed();
                let prior = ctx.world.resource::<CallbackCheckpoint>().0;
                assert!(
                    elapsed > prior,
                    "full app time did not advance during second readback"
                );
                TestFlow::Exit
            }
            frame => panic!("callback continued after exit at frame {frame}"),
        }
    }

    pub(crate) fn screenshot_requests_pause_and_resume() -> Result<(), Failed> {
        let first = PathBuf::from(SCREENSHOT_DIR).join(format!("{FIRST_CAPTURE}.png"));
        let expected = PathBuf::from(SCREENSHOT_DIR).join(format!("{SECOND_CAPTURE}.png"));
        let captured = run_screenshot_test(multi_capture_callback).map_err(Failed::from)?;
        if captured != expected {
            return Err(Failed::from(format!(
                "expected final capture {}, got {}",
                expected.display(),
                captured.display()
            )));
        }
        for path in [&first, &captured] {
            if image::image_dimensions(path).map_err(|err| Failed::from(err.to_string()))?
                != (WIDTH, HEIGHT)
            {
                return Err(Failed::from(format!(
                    "captured dimensions do not match target for {}",
                    path.display()
                )));
            }
        }
        Ok(())
    }

    fn continue_forever(_: &mut TestContext) -> TestFlow {
        TestFlow::Continue
    }

    pub(crate) fn callback_limit_fails() -> Result<(), Failed> {
        expect_error(
            run_screenshot_test_with_limits(
                continue_forever,
                HarnessLimits {
                    max_callback_frames: 2,
                    max_readback_updates: 1,
                },
            ),
            "callback exceeded maximum of 2 frames",
        )
    }

    fn capture_immediately(_: &mut TestContext) -> TestFlow {
        TestFlow::Screenshot("harness-zero-readback-budget".to_string())
    }

    pub(crate) fn zero_readback_budget_fails_before_pump() -> Result<(), Failed> {
        expect_error(
            run_screenshot_test_with_limits(
                capture_immediately,
                HarnessLimits {
                    max_callback_frames: 1,
                    max_readback_updates: 0,
                },
            ),
            "screenshot readback exhausted update budget of 0",
        )
    }

    pub(crate) fn temporary_output_cleanup_is_raii() -> Result<(), Failed> {
        image_export_adapter::temp_dir_guard_removes_tree().map_err(Failed::from)?;

        let path = PathBuf::from(SCREENSHOT_DIR).join(".temporary-artifact-cleanup-regression");
        if path.exists() {
            std::fs::remove_file(&path).map_err(|err| Failed::from(err.to_string()))?;
        }
        {
            let _guard = TemporaryArtifact::new(path.clone());
            std::fs::write(&path, b"partial").map_err(|err| Failed::from(err.to_string()))?;
        }
        if path.exists() {
            return Err(Failed::from(format!(
                "temporary artifact guard left fixture behind at {}",
                path.display()
            )));
        }
        Ok(())
    }

    fn panic_immediately(_: &mut TestContext) -> TestFlow {
        panic!("intentional harness regression panic")
    }

    pub(crate) fn callback_panic_fails() -> Result<(), Failed> {
        expect_error(
            run_screenshot_test(panic_immediately),
            "callback panicked at frame 0: intentional harness regression panic",
        )
    }

    fn exit_without_capture(_: &mut TestContext) -> TestFlow {
        TestFlow::Exit
    }

    pub(crate) fn missing_screenshot_fails() -> Result<(), Failed> {
        expect_error(
            run_screenshot_test(exit_without_capture),
            "callback exited without capturing a screenshot",
        )
    }

    fn expect_error(result: Result<PathBuf, String>, expected: &str) -> Result<(), Failed> {
        match result {
            Err(message) if message.contains(expected) => Ok(()),
            Err(message) => Err(Failed::from(format!(
                "expected error containing {expected:?}, got {message:?}"
            ))),
            Ok(path) => Err(Failed::from(format!(
                "expected failure containing {expected:?}, got artifact {}",
                path.display()
            ))),
        }
    }
}
