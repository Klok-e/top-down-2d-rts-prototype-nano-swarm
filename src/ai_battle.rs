//! Runtime configuration for the symmetric AI Battle scenario.

use bevy::prelude::Resource;
use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AiBattleLayout {
    #[default]
    Standard,
    Flanks,
}

impl fmt::Display for AiBattleLayout {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Standard => "standard",
            Self::Flanks => "flanks",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown AI Battle layout: {value}; expected standard or flanks")]
pub struct UnknownAiBattleLayout {
    value: String,
}

impl FromStr for AiBattleLayout {
    type Err = UnknownAiBattleLayout;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "standard" => Ok(Self::Standard),
            "flanks" => Ok(Self::Flanks),
            _ => Err(UnknownAiBattleLayout {
                value: value.into(),
            }),
        }
    }
}

/// Options that affect an AI Battle without changing its simulation rules.
#[derive(Resource, Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AiBattleConfig {
    pub layout: AiBattleLayout,
    pub realtime: bool,
}
