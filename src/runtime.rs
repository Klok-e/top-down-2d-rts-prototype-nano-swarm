use std::time::Duration;

use bevy::{
    app::{ScheduleRunnerPlugin, TerminalCtrlCHandlerPlugin},
    log::LogPlugin,
    prelude::*,
};

#[cfg(unix)]
use crate::agent_control::{AgentControlConfig, AgentControlPlugin, AgentControlServerError};
use crate::{Presentation, build_app, build_app_with_presentation};

pub const DEFAULT_HEADLESS_WIDTH: u32 = 1280;
pub const DEFAULT_HEADLESS_HEIGHT: u32 = 720;
pub const MAX_HEADLESS_DIMENSION: u32 = 8192;
pub const MAX_HEADLESS_PIXELS: u64 = 16_777_216;
const HEADLESS_FRAME_DURATION: Duration = Duration::from_nanos(16_666_667);

pub const RUNTIME_HELP: &str = "\
Usage: top-down-2d-rts-prototype-nano-swarm [OPTIONS]\n\
\n\
Options:\n\
    --agent-socket     Enable the Unix agent control socket\n\
    --headless         Render offscreen without creating a desktop window\n\
    --width <PIXELS>   Set headless width (default: 1280, max: 8192)\n\
    --height <PIXELS>  Set headless height (default: 720, max: 8192)\n\
    -h, --help         Print help\n";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeOptions {
    pub headless: bool,
    pub agent_socket: bool,
    pub width: u32,
    pub height: u32,
}

impl Default for RuntimeOptions {
    fn default() -> Self {
        Self {
            headless: false,
            agent_socket: false,
            width: DEFAULT_HEADLESS_WIDTH,
            height: DEFAULT_HEADLESS_HEIGHT,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
        let mut args = args.into_iter();
        while let Some(argument) = args.next() {
            match argument.as_ref() {
                "--headless" => options.headless = true,
                "--agent-socket" => options.agent_socket = true,
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
        options.validate()
    }

    fn validate(self) -> Result<Self, RuntimeOptionsError> {
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
    let mut app = if options.headless {
        let mut app = build_app_with_presentation(Presentation::Offscreen {
            width: options.width,
            height: options.height,
        });
        app.add_plugins((
            LogPlugin::default(),
            ScheduleRunnerPlugin::run_loop(HEADLESS_FRAME_DURATION),
            TerminalCtrlCHandlerPlugin,
        ));
        app
    } else {
        build_app()
    };
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
        app::{ScheduleRunnerPlugin, TerminalCtrlCHandlerPlugin},
        log::LogPlugin,
        prelude::Window,
        winit::WinitPlugin,
    };

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

        assert!(app.is_plugin_added::<ScheduleRunnerPlugin>());
        assert!(app.is_plugin_added::<TerminalCtrlCHandlerPlugin>());
        assert!(app.is_plugin_added::<LogPlugin>());
        assert!(!app.is_plugin_added::<WinitPlugin>());
        assert_eq!(
            app.world_mut().query::<&Window>().iter(app.world()).count(),
            0
        );
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
