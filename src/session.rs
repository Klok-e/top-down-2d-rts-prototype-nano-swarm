//! Scenario-independent policies consumed by simulation, input, and presentation.

use crate::nanobot::{MatchOutcome, SwarmId};
use bevy::prelude::*;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum OutcomeMode {
    Disabled,
    #[default]
    PlayerRelative,
    SwarmRelative,
}

impl OutcomeMode {
    pub fn enabled(self) -> bool {
        self != Self::Disabled
    }
}

#[derive(Debug, Resource, Clone, Copy)]
pub struct SessionRules {
    pub scenario_name: &'static str,
    pub player_swarm: Option<SwarmId>,
    pub outcomes: OutcomeMode,
    pub record_statistics: bool,
    pub accelerate_headless: bool,
}

impl Default for SessionRules {
    fn default() -> Self {
        Self {
            scenario_name: "standard",
            player_swarm: Some(SwarmId::PLAYER),
            outcomes: OutcomeMode::PlayerRelative,
            record_statistics: false,
            accelerate_headless: false,
        }
    }
}

impl SessionRules {
    pub fn spectator(self) -> bool {
        self.player_swarm.is_none()
    }

    pub fn outcome_label(self, outcome: MatchOutcome) -> Option<String> {
        match outcome {
            MatchOutcome::InProgress => None,
            MatchOutcome::Draw => (self.outcomes != OutcomeMode::Disabled)
                .then(|| "DRAW\nBoth Swarms Eliminated".to_string()),
            MatchOutcome::Winner(winner) => match self.outcomes {
                OutcomeMode::Disabled => None,
                OutcomeMode::PlayerRelative => Some(if self.player_swarm == Some(winner) {
                    "VICTORY\nOpponent Swarm Eliminated".to_string()
                } else {
                    "DEFEAT\nPlayer Swarm Eliminated".to_string()
                }),
                OutcomeMode::SwarmRelative => Some(format!("SWARM {} WINS", winner.0)),
            },
        }
    }

    pub fn outcome_name(self, outcome: MatchOutcome) -> String {
        if self.outcomes == OutcomeMode::Disabled {
            return "in_progress".to_string();
        }
        match outcome {
            MatchOutcome::InProgress => "in_progress".to_string(),
            MatchOutcome::Draw => "draw".to_string(),
            MatchOutcome::Winner(winner) => match self.outcomes {
                OutcomeMode::PlayerRelative => if self.player_swarm == Some(winner) {
                    "victory"
                } else {
                    "defeat"
                }
                .to_string(),
                OutcomeMode::SwarmRelative => format!("swarm_{}_wins", winner.0),
                OutcomeMode::Disabled => unreachable!(),
            },
        }
    }
}

/// Starting seed for procedural simulation decisions, independent of recording.
#[derive(Debug, Resource, Clone, Copy, Default)]
pub struct SimulationSeed(pub u64);
