//! End-of-tick Swarm Elimination and permanent match outcomes.

use bevy::prelude::*;

use crate::resources::Stockpile;

use super::{
    Charger, Nanobot, OwnerSwarm, PlannedStructure, ProductionFacility, Swarm, SwarmId,
    SwarmMember, nanobot_death_cleanup_system,
};

/// Current elimination flags for the two sides of a Standard match.
#[derive(Debug, Default, Resource, Clone, Copy)]
pub struct SwarmEliminationState {
    pub player_eliminated: bool,
    pub opponent_eliminated: bool,
}

/// The first terminal result remains permanent even if simulation continues.
#[derive(Debug, Default, Resource, Clone, Copy, PartialEq, Eq)]
pub enum MatchOutcome {
    #[default]
    InProgress,
    Victory,
    Defeat,
    Draw,
}

#[allow(clippy::type_complexity)]
fn detect_swarm_elimination(
    swarms: Query<(Entity, &SwarmId), With<Swarm>>,
    nanobots: Query<&SwarmMember, With<Nanobot>>,
    structures: Query<
        &OwnerSwarm,
        (
            Or<(With<ProductionFacility>, With<Stockpile>, With<Charger>)>,
            Without<PlannedStructure>,
        ),
    >,
    mut state: ResMut<SwarmEliminationState>,
    mut outcome: ResMut<MatchOutcome>,
) {
    *state = SwarmEliminationState::default();
    for (entity, id) in &swarms {
        let eliminated = !nanobots.iter().any(|member| member.0 == *id)
            && !structures.iter().any(|owner| owner.0 == entity);
        if id.is_player() {
            state.player_eliminated = eliminated;
        } else {
            state.opponent_eliminated = eliminated;
        }
    }
    if *outcome == MatchOutcome::InProgress {
        *outcome = match (state.player_eliminated, state.opponent_eliminated) {
            (false, false) => MatchOutcome::InProgress,
            (false, true) => MatchOutcome::Victory,
            (true, false) => MatchOutcome::Defeat,
            (true, true) => MatchOutcome::Draw,
        };
    }
}

pub struct SwarmEliminationPlugin;

impl Plugin for SwarmEliminationPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SwarmEliminationState>()
            .init_resource::<MatchOutcome>()
            .add_systems(
                FixedLast,
                detect_swarm_elimination
                    .after(nanobot_death_cleanup_system)
                    .run_if(scenario_has_outcomes),
            );
    }
}

fn scenario_has_outcomes(
    selection: Option<Res<crate::scenario_selection::ScenarioSelection>>,
) -> bool {
    selection
        .is_none_or(|selection| selection.current == crate::scenario_selection::Scenario::Standard)
}
