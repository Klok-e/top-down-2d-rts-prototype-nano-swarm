//! `harness = false` screenshot test binary, driven by `libtest-mimic`.
//!
//! Offscreen GPU-rendered visual tests live here, separate from headless
//! `tests/` cargo targets. Each trial drives deterministic `app.update()`
//! iterations without creating a desktop window or winit event loop. See
//! `docs/agents/testing.md` -> "Screenshot evidence".
//!
//! Each test is a callback `fn(&mut TestContext) -> TestFlow` (see
//! `harness.rs`). To add a test: write the `fn` in a module under
//! `screenshots/`, then add a `Trial::test("name", || run(your_fn))`
//! line to the `tests![...]` list in [`main`].
//!
//! Screenshot tests are ignored by default so normal test runs avoid GPU
//! setup. Run them with:
//!
//! ```bash
//! cargo test --test screenshots -- --ignored          # all
//! cargo test --test screenshots -- --ignored smoke     # filter by name
//! cargo test --test screenshots -- --list              # list all
//! ```

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
mod zone_binary_overlay;

use harness::{TestContext, TestFlow, regression, run_screenshot_test};

/// Wraps a screenshot callback in the `Result<(), Failed>` shape
/// `libtest-mimic`'s [`Trial::test`] expects. The artifact path is
/// discarded on success; failure carries the harness error message.
fn run(f: fn(&mut TestContext) -> TestFlow) -> Result<(), Failed> {
    run_screenshot_test(f).map_err(Failed::from).map(|_| ())
}

fn main() -> std::process::ExitCode {
    let mut args = Arguments::from_args();

    // Trials share GPU and artifact paths. Keep execution serial even though
    // no main-thread window/event-loop constraint remains.
    args.test_threads = Some(1);

    // Each test is ignored so default run skips GPU setup. `--ignored` runs
    // only ignored tests, matching standard `cargo test` convention.
    let tests = vec![
        Trial::test(
            "harness_screenshot_requests_pause_and_resume",
            regression::screenshot_requests_pause_and_resume,
        )
        .with_ignored_flag(true),
        Trial::test(
            "harness_callback_limit_fails",
            regression::callback_limit_fails,
        )
        .with_ignored_flag(true),
        Trial::test(
            "harness_zero_readback_budget_fails_before_pump",
            regression::zero_readback_budget_fails_before_pump,
        )
        .with_ignored_flag(true),
        Trial::test(
            "harness_temporary_output_cleanup_is_raii",
            regression::temporary_output_cleanup_is_raii,
        )
        .with_ignored_flag(true),
        Trial::test(
            "harness_callback_panic_fails",
            regression::callback_panic_fails,
        )
        .with_ignored_flag(true),
        Trial::test(
            "harness_missing_screenshot_fails",
            regression::missing_screenshot_fails,
        )
        .with_ignored_flag(true),
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
        Trial::test("zone_binary_overlay", || {
            run(zone_binary_overlay::zone_binary_overlay)
        })
        .with_ignored_flag(true),
    ];

    let conclusion: Conclusion = libtest_mimic::run(&args, tests);
    conclusion.exit_code()
}
