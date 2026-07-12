//! `harness = false` screenshot test binary, driven by `libtest-mimic`.
//!
//! Window-creating (visual) tests live here, separate from the headless
//! `tests/` cargo targets, because they must run on the main thread
//! (winit requires it) under a real `app.run()` event loop. See
//! `docs/agents/testing.md` -> "Screenshot evidence".
//!
//! Each test is a callback `fn(&mut TestContext) -> TestFlow` (see
//! `harness.rs`). To add a test: write the `fn` in a module under
//! `screenshots/`, then add a `Trial::test("name", || run(your_fn))`
//! line to the `tests![...]` list in [`main`].
//!
//! Screenshot tests are marked ignored so the default `cargo test`
//! run skips them without a display. Run them with:
//!
//! ```bash
//! cargo test --test screenshots -- --ignored          # all
//! cargo test --test screenshots -- --ignored smoke     # filter by name
//! cargo test --test screenshots -- --list              # list all
//! ```
//!
//! The harness forces `--test-threads=1` regardless of the passed
//! flags, because each test drives its own `app.run()` on the main
//! thread; parallel execution inside `libtest-mimic`'s pool would run
//! callbacks on worker threads and winit would refuse.

use libtest_mimic::{Arguments, Conclusion, Failed, Trial};

mod defender_spread;
mod fill_indicators;
mod harness;
mod idle_spread;
mod physical_logistics;
mod production_ratio_panel;
mod regional_allocation;
mod smoke;
mod world_space_nanobots;
mod zone_strength_ramp;

use harness::{run_screenshot_test, TestContext, TestFlow};

/// Wraps a screenshot callback in the `Result<(), Failed>` shape
/// `libtest-mimic`'s [`Trial::test`] expects. The artifact path is
/// discarded on success; failure carries the harness error message.
fn run(f: fn(&mut TestContext) -> TestFlow) -> Result<(), Failed> {
    run_screenshot_test(f).map_err(Failed::from).map(|_| ())
}

fn main() -> std::process::ExitCode {
    let mut args = Arguments::from_args();

    // Screenshot tests drive a real winit event loop on the main
    // thread. `libtest-mimic`'s default thread-pool mode would run
    // the trials on worker threads, where winit refuses to construct
    // the event loop. Force serial execution so every `app.run()` --
    // and therefore the trial body -- runs on the calling (main)
    // thread, overriding any `--test-threads` the user passed.
    args.test_threads = Some(1);

    // Each test is marked ignored so the default run skips it (no
    // display). `--ignored` runs only ignored tests, exactly matching
    // the standard `cargo test` convention.
    let tests = vec![
        Trial::test("defender_spread", || run(defender_spread::defender_spread))
            .with_ignored_flag(true),
        Trial::test("idle_spread", || run(idle_spread::idle_spread)).with_ignored_flag(true),
        Trial::test("fill_indicators", || run(fill_indicators::fill_indicators))
            .with_ignored_flag(true),
        Trial::test("smoke", || run(smoke::smoke)).with_ignored_flag(true),
        Trial::test("physical_logistics", || {
            run(physical_logistics::physical_logistics)
        })
        .with_ignored_flag(true),
        Trial::test("production_ratio_panel", || {
            run(production_ratio_panel::production_ratio_panel)
        })
        .with_ignored_flag(true),
        Trial::test("regional_allocation", || {
            run(regional_allocation::regional_allocation)
        })
        .with_ignored_flag(true),
        Trial::test("world_space_nanobots", || {
            run(world_space_nanobots::world_space_nanobots)
        })
        .with_ignored_flag(true),
        Trial::test("zone_strength_ramp", || {
            run(zone_strength_ramp::zone_strength_ramp)
        })
        .with_ignored_flag(true),
    ];

    let conclusion: Conclusion = libtest_mimic::run(&args, tests);
    conclusion.exit_code()
}
