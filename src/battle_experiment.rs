//! Reproducible configuration for AI Battle experiments.

use bevy::prelude::Resource;
use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControllerId {
    #[default]
    Timed,
    Adaptive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LayoutId {
    #[default]
    Standard,
    Flanks,
    Narrows,
    Crossroads,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PacingId {
    #[default]
    Baseline,
    Deliberate,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown {kind}: {value}; expected {expected}")]
pub struct UnknownExperimentId {
    kind: &'static str,
    value: String,
    expected: &'static str,
}

macro_rules! experiment_id {
    ($type:ty, $kind:literal, $expected:literal, {$($name:literal => $variant:path),+ $(,)?}) => {
        impl fmt::Display for $type {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(match self {
                    $($variant => $name),+
                })
            }
        }

        impl FromStr for $type {
            type Err = UnknownExperimentId;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                match value {
                    $($name => Ok($variant)),+,
                    _ => Err(UnknownExperimentId {
                        kind: $kind,
                        value: value.into(),
                        expected: $expected,
                    }),
                }
            }
        }
    };
}

experiment_id!(ControllerId, "controller", "timed or adaptive", {
    "timed" => ControllerId::Timed,
    "adaptive" => ControllerId::Adaptive,
});
experiment_id!(LayoutId, "layout", "standard, flanks, narrows, or crossroads", {
    "standard" => LayoutId::Standard,
    "flanks" => LayoutId::Flanks,
    "narrows" => LayoutId::Narrows,
    "crossroads" => LayoutId::Crossroads,
});
experiment_id!(PacingId, "pacing", "baseline or deliberate", {
    "baseline" => PacingId::Baseline,
    "deliberate" => PacingId::Deliberate,
});

#[derive(Resource, Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BattleExperimentConfig {
    pub controllers: [ControllerId; 2],
    pub layout: LayoutId,
    pub swap_sides: bool,
    pub pacing: PacingId,
    pub cutoff_seconds: Option<u32>,
    pub realtime: bool,
}

impl Default for BattleExperimentConfig {
    fn default() -> Self {
        Self {
            controllers: [ControllerId::Timed; 2],
            layout: LayoutId::Standard,
            swap_sides: false,
            pacing: PacingId::Baseline,
            cutoff_seconds: None,
            realtime: false,
        }
    }
}
