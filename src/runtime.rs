use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

use bevy::{
    app::{AppExit, PluginsState, TerminalCtrlCHandlerPlugin},
    log::LogPlugin,
    prelude::*,
    time::{TimeSystems, TimeUpdateStrategy},
};

#[cfg(unix)]
use crate::agent_control::{AgentControlConfig, AgentControlPlugin, AgentControlServerError};
use crate::{
    Presentation,
    battle_experiment::{BattleExperimentConfig, ControllerId, LayoutId, PacingId},
    battle_statistics::BattleStatisticsConfig,
    build_app, build_app_with_presentation,
    scenario_selection::{Scenario, ScenarioSelection},
};

pub const DEFAULT_HEADLESS_WIDTH: u32 = 1280;
pub const DEFAULT_HEADLESS_HEIGHT: u32 = 720;
pub const MAX_HEADLESS_DIMENSION: u32 = 8192;
pub const MAX_HEADLESS_PIXELS: u64 = 16_777_216;
const HEADLESS_FRAME_DURATION: Duration = Duration::from_nanos(16_666_667);

/// Headless runner whose pacing follows the active session instead of launch options.
pub struct DynamicHeadlessPacingPlugin;

impl Plugin for DynamicHeadlessPacingPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<crate::session::SessionRules>()
            .init_resource::<TimeUpdateStrategy>()
            .add_systems(First, sync_headless_time_strategy.before(TimeSystems))
            .set_runner(run_dynamic_headless);
    }
}

fn sync_headless_time_strategy(
    rules: Res<crate::session::SessionRules>,
    mut strategy: ResMut<TimeUpdateStrategy>,
) {
    if rules.accelerate_headless {
        if !matches!(*strategy, TimeUpdateStrategy::FixedTimesteps(1)) {
            *strategy = TimeUpdateStrategy::FixedTimesteps(1);
        }
    } else if !matches!(*strategy, TimeUpdateStrategy::Automatic) {
        *strategy = TimeUpdateStrategy::Automatic;
    }
}

fn headless_frame_delay(accelerated: bool, elapsed: Duration) -> Duration {
    if accelerated {
        Duration::ZERO
    } else {
        HEADLESS_FRAME_DURATION.saturating_sub(elapsed)
    }
}

fn run_dynamic_headless(mut app: App) -> AppExit {
    if app.plugins_state() != PluginsState::Cleaned {
        while app.plugins_state() == PluginsState::Adding {
            bevy::tasks::tick_global_task_pools_on_main_thread();
        }
        app.finish();
        app.cleanup();
    }

    loop {
        let started = Instant::now();
        app.update();
        if let Some(exit) = app.should_exit() {
            return exit;
        }
        let accelerated = app
            .world()
            .resource::<crate::session::SessionRules>()
            .accelerate_headless;
        let delay = headless_frame_delay(accelerated, started.elapsed());
        if !delay.is_zero() {
            std::thread::sleep(delay);
        }
    }
}

pub const RUNTIME_HELP: &str = "\
Usage: top-down-2d-rts-prototype-nano-swarm [OPTIONS]\n\
\n\
Options:\n\
    --agent-socket     Enable the Unix agent control socket\n\
    --headless         Render offscreen without creating a desktop window\n\
    --width <PIXELS>   Set headless width (default: 1280, max: 8192)\n\
    --height <PIXELS>  Set headless height (default: 720, max: 8192)\n\
    --scenario <NAME>  Select standard, sandbox, or ai-battle for this run\n\
    --output-root <PATH>  Battle results directory (default: target/battle-runs)\n\
    --seed <INTEGER>   Battle starting seed (default: 0)\n\
    --experiment      Run a bounded AI Battle experiment (default cutoff: 600 simulated seconds)\n\
    --controllers <A,B>  Select timed or adaptive for each swarm\n\
    --layout <NAME>   Select standard, flanks, narrows, or crossroads\n\
    --swap-sides      Swap the configured controllers between starting sides\n\
    --pacing <NAME>   Select baseline or deliberate shared pacing\n\
    --trial-seconds <SECONDS>  Set a positive simulated-time experiment cutoff\n\
    --realtime        Observe the experiment with real-time headless pacing\n\
    -h, --help         Print help\n\
\n\
Headless AI Battle advances without real-time pacing unless an experiment uses --realtime.\n";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeOptions {
    pub headless: bool,
    pub agent_socket: bool,
    pub width: u32,
    pub height: u32,
    pub scenario: Option<Scenario>,
    pub output_root: PathBuf,
    pub seed: u64,
    pub experiment: Option<BattleExperimentConfig>,
}

impl Default for RuntimeOptions {
    fn default() -> Self {
        Self {
            headless: false,
            agent_socket: false,
            width: DEFAULT_HEADLESS_WIDTH,
            height: DEFAULT_HEADLESS_HEIGHT,
            scenario: None,
            output_root: PathBuf::from("target/battle-runs"),
            seed: 0,
            experiment: None,
        }
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RuntimeOptionsError {
    #[error("missing value for {0}")]
    MissingValue(String),
    #[error("invalid positive integer for {flag}: {value}")]
    InvalidDimension { flag: String, value: String },
    #[error("{flag} cannot exceed {max} pixels, got {value}")]
    DimensionTooLarge {
        flag: &'static str,
        value: u32,
        max: u32,
    },
    #[error("headless resolution {width}x{height} exceeds {max_pixels} pixels")]
    ResolutionTooLarge {
        width: u32,
        height: u32,
        max_pixels: u64,
    },
    #[error("unknown scenario: {0}; expected standard, sandbox, or ai-battle")]
    UnknownScenario(String),
    #[error("invalid unsigned integer for --seed: {0}")]
    InvalidSeed(String),
    #[error("invalid controller pair: {0}; expected two comma-separated values: timed or adaptive")]
    InvalidControllers(String),
    #[error("unknown controller: {0}; expected timed or adaptive")]
    UnknownController(String),
    #[error("unknown layout: {0}; expected standard, flanks, narrows, or crossroads")]
    UnknownLayout(String),
    #[error("unknown pacing: {0}; expected baseline or deliberate")]
    UnknownPacing(String),
    #[error("invalid positive integer for --trial-seconds: {0}")]
    InvalidTrialSeconds(String),
    #[error("--experiment requires an explicit --scenario ai-battle")]
    ExperimentRequiresAiBattle,
    #[error("{flag} requires --experiment")]
    OptionRequiresExperiment { flag: String },
    #[error("--output-root cannot be empty")]
    EmptyOutputRoot,
    #[error("unknown argument: {0}")]
    UnknownArgument(String),
}

#[derive(Debug, thiserror::Error)]
pub enum RuntimeBuildError {
    #[error(transparent)]
    InvalidOptions(#[from] RuntimeOptionsError),
    #[cfg(unix)]
    #[error(transparent)]
    AgentControl(#[from] AgentControlServerError),
    #[cfg(not(unix))]
    #[error("--agent-socket is supported only on Unix platforms")]
    UnsupportedAgentSocket,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeCommand {
    Run(RuntimeOptions),
    Help,
}

impl RuntimeCommand {
    pub fn parse<I, S>(args: I) -> Result<Self, RuntimeOptionsError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let args = args
            .into_iter()
            .map(|argument| argument.as_ref().to_string())
            .collect::<Vec<_>>();
        if args
            .iter()
            .any(|argument| argument == "--help" || argument == "-h")
        {
            return Ok(Self::Help);
        }
        RuntimeOptions::parse(args).map(Self::Run)
    }
}

impl RuntimeOptions {
    pub fn parse<I, S>(args: I) -> Result<Self, RuntimeOptionsError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut options = Self::default();
        let mut experiment = BattleExperimentConfig::default();
        let mut experiment_enabled = false;
        let mut experiment_option = None;
        let mut trial_seconds = None;
        let mut args = args.into_iter();
        while let Some(argument) = args.next() {
            match argument.as_ref() {
                "--headless" => options.headless = true,
                "--agent-socket" => options.agent_socket = true,
                "--experiment" => experiment_enabled = true,
                "--swap-sides" => {
                    experiment.swap_sides = true;
                    experiment_option.get_or_insert_with(|| "--swap-sides".to_string());
                }
                "--realtime" => {
                    experiment.realtime = true;
                    experiment_option.get_or_insert_with(|| "--realtime".to_string());
                }
                flag @ ("--scenario" | "--output-root" | "--seed" | "--controllers"
                | "--layout" | "--pacing" | "--trial-seconds") => {
                    let value = args
                        .next()
                        .ok_or_else(|| RuntimeOptionsError::MissingValue(flag.to_string()))?;
                    let value = value.as_ref();
                    match flag {
                        "--scenario" => {
                            options.scenario = Some(match value {
                                "standard" => Scenario::Standard,
                                "sandbox" => Scenario::Sandbox,
                                "ai-battle" => Scenario::AiBattle,
                                unknown => {
                                    return Err(RuntimeOptionsError::UnknownScenario(
                                        unknown.to_string(),
                                    ));
                                }
                            })
                        }
                        "--output-root" => options.output_root = PathBuf::from(value),
                        "--seed" => {
                            options.seed = value
                                .parse()
                                .map_err(|_| RuntimeOptionsError::InvalidSeed(value.to_string()))?
                        }
                        "--controllers" => {
                            experiment_option.get_or_insert_with(|| "--controllers".to_string());
                            let controllers = value.split(',').collect::<Vec<_>>();
                            if controllers.len() != 2
                                || controllers.iter().any(|controller| controller.is_empty())
                            {
                                return Err(RuntimeOptionsError::InvalidControllers(value.into()));
                            }
                            experiment.controllers = [
                                controllers[0].parse::<ControllerId>().map_err(|_| {
                                    RuntimeOptionsError::UnknownController(controllers[0].into())
                                })?,
                                controllers[1].parse::<ControllerId>().map_err(|_| {
                                    RuntimeOptionsError::UnknownController(controllers[1].into())
                                })?,
                            ];
                        }
                        "--layout" => {
                            experiment_option.get_or_insert_with(|| "--layout".to_string());
                            experiment.layout = value
                                .parse::<LayoutId>()
                                .map_err(|_| RuntimeOptionsError::UnknownLayout(value.into()))?;
                        }
                        "--pacing" => {
                            experiment_option.get_or_insert_with(|| "--pacing".to_string());
                            experiment.pacing = value
                                .parse::<PacingId>()
                                .map_err(|_| RuntimeOptionsError::UnknownPacing(value.into()))?;
                        }
                        "--trial-seconds" => {
                            experiment_option.get_or_insert_with(|| "--trial-seconds".to_string());
                            trial_seconds =
                                Some(value.parse::<u32>().ok().filter(|n| *n > 0).ok_or_else(
                                    || RuntimeOptionsError::InvalidTrialSeconds(value.into()),
                                )?);
                        }
                        _ => unreachable!(),
                    }
                }
                flag @ ("--width" | "--height") => {
                    let value = args
                        .next()
                        .ok_or_else(|| RuntimeOptionsError::MissingValue(flag.to_string()))?;
                    let value = value.as_ref();
                    let parsed = value
                        .parse::<u32>()
                        .ok()
                        .filter(|value| *value > 0)
                        .ok_or_else(|| RuntimeOptionsError::InvalidDimension {
                            flag: flag.to_string(),
                            value: value.to_string(),
                        })?;
                    if flag == "--width" {
                        options.width = parsed;
                    } else {
                        options.height = parsed;
                    }
                }
                unknown => {
                    return Err(RuntimeOptionsError::UnknownArgument(unknown.to_string()));
                }
            }
        }
        if !experiment_enabled && let Some(flag) = experiment_option {
            return Err(RuntimeOptionsError::OptionRequiresExperiment { flag });
        }
        if experiment_enabled {
            if options.scenario != Some(Scenario::AiBattle) {
                return Err(RuntimeOptionsError::ExperimentRequiresAiBattle);
            }
            experiment.cutoff_seconds = Some(trial_seconds.unwrap_or(600));
            options.experiment = Some(experiment);
        }
        options.validate()
    }

    fn validate(self) -> Result<Self, RuntimeOptionsError> {
        if self.output_root.as_os_str().is_empty() {
            return Err(RuntimeOptionsError::EmptyOutputRoot);
        }
        if self.experiment.is_some() && self.scenario != Some(Scenario::AiBattle) {
            return Err(RuntimeOptionsError::ExperimentRequiresAiBattle);
        }
        for (flag, value) in [("--width", self.width), ("--height", self.height)] {
            if value > MAX_HEADLESS_DIMENSION {
                return Err(RuntimeOptionsError::DimensionTooLarge {
                    flag,
                    value,
                    max: MAX_HEADLESS_DIMENSION,
                });
            }
        }
        let pixels = u64::from(self.width) * u64::from(self.height);
        if pixels > MAX_HEADLESS_PIXELS {
            return Err(RuntimeOptionsError::ResolutionTooLarge {
                width: self.width,
                height: self.height,
                max_pixels: MAX_HEADLESS_PIXELS,
            });
        }
        Ok(self)
    }
}

pub fn build_runtime_app(options: RuntimeOptions) -> Result<App, RuntimeBuildError> {
    let options = options.validate()?;
    let mut selection = ScenarioSelection::from_environment();
    if let Some(scenario) = options.scenario {
        selection.current = scenario;
    }
    let mut app = if options.headless {
        let mut app = build_app_with_presentation(Presentation::Offscreen {
            width: options.width,
            height: options.height,
        });
        app.add_plugins((
            LogPlugin::default(),
            DynamicHeadlessPacingPlugin,
            TerminalCtrlCHandlerPlugin,
        ));
        app
    } else {
        build_app()
    };
    app.insert_resource(selection);
    app.insert_resource(crate::session::SimulationSeed(options.seed));
    app.insert_resource(options.experiment.clone().unwrap_or_default());
    app.insert_resource(BattleStatisticsConfig {
        output_root: options.output_root,
        seed: options.seed,
        headless: options.headless,
    });
    #[cfg(unix)]
    if options.agent_socket {
        app.add_plugins(AgentControlPlugin::bind(
            AgentControlConfig::from_environment()?,
        )?);
    }
    #[cfg(not(unix))]
    if options.agent_socket {
        return Err(RuntimeBuildError::UnsupportedAgentSocket);
    }
    Ok(app)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::{
        app::TerminalCtrlCHandlerPlugin, log::LogPlugin, prelude::Window, winit::WinitPlugin,
    };

    #[test]
    fn battle_options_are_accepted_for_an_explicit_benchmark_run() {
        let options = RuntimeOptions::parse([
            "--headless",
            "--scenario",
            "ai-battle",
            "--output-root",
            "/tmp/battle-results",
            "--seed",
            "42",
        ])
        .unwrap();
        assert!(options.headless);
        assert_eq!(options.scenario, Some(Scenario::AiBattle));
        assert_eq!(options.output_root, PathBuf::from("/tmp/battle-results"));
        assert_eq!(options.seed, 42);
        assert!(options.experiment.is_none());
    }

    #[test]
    fn experiment_options_describe_the_exact_ai_battle_trial() {
        use crate::battle_experiment::{ControllerId, LayoutId, PacingId};

        let options = RuntimeOptions::parse([
            "--headless",
            "--scenario",
            "ai-battle",
            "--experiment",
            "--controllers",
            "adaptive,timed",
            "--layout",
            "crossroads",
            "--swap-sides",
            "--pacing",
            "deliberate",
            "--trial-seconds",
            "75",
            "--realtime",
        ])
        .unwrap();

        assert_eq!(
            options.experiment,
            Some(crate::battle_experiment::BattleExperimentConfig {
                controllers: [ControllerId::Adaptive, ControllerId::Timed],
                layout: LayoutId::Crossroads,
                swap_sides: true,
                pacing: PacingId::Deliberate,
                cutoff_seconds: Some(75),
                realtime: true,
            })
        );
    }

    #[test]
    fn experiment_defaults_to_the_initial_ten_minute_trial_budget() {
        let options = RuntimeOptions::parse(["--scenario", "ai-battle", "--experiment"]).unwrap();
        assert_eq!(options.experiment.unwrap().cutoff_seconds, Some(600));
        assert!(
            RuntimeOptions::parse(["--scenario", "ai-battle"])
                .unwrap()
                .experiment
                .is_none(),
            "normal AI Battle must remain unlimited"
        );
    }

    #[test]
    fn experiment_flags_reject_ambiguous_or_ignored_runtime_configuration() {
        assert!(matches!(
            RuntimeOptions::parse(["--experiment"]),
            Err(RuntimeOptionsError::ExperimentRequiresAiBattle)
        ));
        assert!(matches!(
            RuntimeOptions::parse(["--scenario", "standard", "--experiment"]),
            Err(RuntimeOptionsError::ExperimentRequiresAiBattle)
        ));
        assert!(matches!(
            RuntimeOptions::parse(["--scenario", "ai-battle", "--layout", "flanks"]),
            Err(RuntimeOptionsError::OptionRequiresExperiment { .. })
        ));
        assert!(matches!(
            RuntimeOptions::parse([
                "--scenario",
                "ai-battle",
                "--experiment",
                "--trial-seconds",
                "0"
            ]),
            Err(RuntimeOptionsError::InvalidTrialSeconds(_))
        ));
    }

    #[test]
    fn invalid_benchmark_arguments_fail_before_runtime_startup() {
        assert_eq!(
            RuntimeOptions::parse(["--scenario", "unknown"]),
            Err(RuntimeOptionsError::UnknownScenario("unknown".into()))
        );
        assert_eq!(
            RuntimeOptions::parse(["--seed", "-1"]),
            Err(RuntimeOptionsError::InvalidSeed("-1".into()))
        );
        assert_eq!(
            RuntimeOptions::parse(["--seed", "18446744073709551616"]),
            Err(RuntimeOptionsError::InvalidSeed(
                "18446744073709551616".into()
            ))
        );
        assert_eq!(
            RuntimeOptions::parse(["--output-root", ""]),
            Err(RuntimeOptionsError::EmptyOutputRoot)
        );
        for flag in [
            "--scenario",
            "--output-root",
            "--seed",
            "--controllers",
            "--layout",
            "--pacing",
            "--trial-seconds",
        ] {
            assert_eq!(
                RuntimeOptions::parse([flag]),
                Err(RuntimeOptionsError::MissingValue(flag.into()))
            );
        }
    }

    #[test]
    fn parses_headless_agent_socket_and_resolution() {
        let options = RuntimeOptions::parse([
            "--headless",
            "--agent-socket",
            "--width",
            "1600",
            "--height",
            "900",
        ])
        .unwrap();

        assert_eq!(
            options,
            RuntimeOptions {
                headless: true,
                agent_socket: true,
                width: 1600,
                height: 900,
                ..RuntimeOptions::default()
            }
        );
    }

    #[test]
    fn headless_runtime_uses_a_no_window_schedule_runner() {
        let mut app = build_runtime_app(RuntimeOptions {
            headless: true,
            ..RuntimeOptions::default()
        })
        .unwrap();

        assert!(app.is_plugin_added::<DynamicHeadlessPacingPlugin>());
        assert!(app.is_plugin_added::<TerminalCtrlCHandlerPlugin>());
        assert!(app.is_plugin_added::<LogPlugin>());
        assert!(!app.is_plugin_added::<WinitPlugin>());
        assert_eq!(
            app.world_mut().query::<&Window>().iter(app.world()).count(),
            0
        );
        assert_eq!(
            app.world().resource::<BattleExperimentConfig>(),
            &BattleExperimentConfig::default()
        );
    }

    #[test]
    fn runtime_installs_the_parsed_experiment_configuration() {
        let options = RuntimeOptions::parse([
            "--headless",
            "--scenario",
            "ai-battle",
            "--experiment",
            "--controllers",
            "adaptive,timed",
            "--layout",
            "flanks",
            "--trial-seconds",
            "12",
            "--realtime",
        ])
        .unwrap();
        let expected = options.experiment.clone().unwrap();
        let app = build_runtime_app(options).unwrap();

        assert_eq!(app.world().resource::<BattleExperimentConfig>(), &expected);
    }

    #[test]
    fn headless_delay_accounts_for_the_complete_app_update() {
        assert_eq!(
            headless_frame_delay(false, Duration::from_millis(10)),
            HEADLESS_FRAME_DURATION - Duration::from_millis(10),
        );
        assert_eq!(
            headless_frame_delay(false, Duration::from_millis(20)),
            Duration::ZERO,
        );
        assert_eq!(headless_frame_delay(true, Duration::ZERO), Duration::ZERO,);
    }

    #[test]
    fn help_is_parsed_without_starting_a_runtime() {
        assert_eq!(RuntimeCommand::parse(["--help"]), Ok(RuntimeCommand::Help));
    }

    #[test]
    fn rejects_invalid_cli_values_and_unknown_arguments() {
        assert!(matches!(
            RuntimeOptions::parse(["--width", "0"]),
            Err(RuntimeOptionsError::InvalidDimension { .. })
        ));
        assert_eq!(
            RuntimeOptions::parse(["--height"]),
            Err(RuntimeOptionsError::MissingValue("--height".to_string()))
        );
        assert_eq!(
            RuntimeOptions::parse(["--windowed"]),
            Err(RuntimeOptionsError::UnknownArgument(
                "--windowed".to_string()
            ))
        );
        assert!(matches!(
            RuntimeOptions::parse(["--headless", "--width", "100000"]),
            Err(RuntimeOptionsError::DimensionTooLarge { .. })
        ));
        assert!(matches!(
            RuntimeOptions::parse(["--headless", "--width", "8192", "--height", "8192"]),
            Err(RuntimeOptionsError::ResolutionTooLarge { .. })
        ));
    }
}
