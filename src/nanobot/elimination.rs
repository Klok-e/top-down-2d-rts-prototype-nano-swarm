//! End-of-tick Swarm Elimination and permanent match outcomes.

use bevy::prelude::*;
use std::collections::BTreeSet;

use crate::resources::Stockpile;

use super::{
    Charger, Nanobot, OwnerSwarm, PlannedStructure, ProductionFacility, Swarm, SwarmId,
    SwarmMember, nanobot_death_cleanup_system,
};

/// Current elimination set for all participating swarms.
#[derive(Debug, Default, Resource, Clone, PartialEq, Eq)]
pub struct SwarmEliminationState {
    pub eliminated: BTreeSet<SwarmId>,
}

impl SwarmEliminationState {
    pub fn is_eliminated(&self, id: SwarmId) -> bool {
        self.eliminated.contains(&id)
    }
}

/// The first terminal result remains permanent even if simulation continues.
#[derive(Debug, Default, Resource, Clone, Copy, PartialEq, Eq)]
pub enum MatchOutcome {
    #[default]
    InProgress,
    Winner(SwarmId),
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
    state.eliminated.clear();
    let mut living = 0;
    let mut participating = 0;
    let mut winner = None;
    for (entity, id) in &swarms {
        participating += 1;
        let eliminated = !nanobots.iter().any(|member| member.0 == *id)
            && !structures.iter().any(|owner| owner.0 == entity);
        if eliminated {
            state.eliminated.insert(*id);
        } else {
            living += 1;
            winner = Some(*id);
        }
    }
    if *outcome == MatchOutcome::InProgress {
        *outcome = if participating < 2 || living > 1 {
            MatchOutcome::InProgress
        } else if living == 1 {
            MatchOutcome::Winner(winner.expect("living swarm has an id"))
        } else {
            MatchOutcome::Draw
        };
    }
}

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SwarmEliminationSet;

pub struct SwarmEliminationPlugin;

impl Plugin for SwarmEliminationPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SwarmEliminationState>()
            .init_resource::<MatchOutcome>()
            .init_resource::<crate::session::SessionRules>()
            .add_systems(
                FixedLast,
                detect_swarm_elimination
                    .in_set(SwarmEliminationSet)
                    .after(nanobot_death_cleanup_system)
                    .run_if(outcomes_enabled),
            );
    }
}

fn outcomes_enabled(rules: Res<crate::session::SessionRules>) -> bool {
    rules.outcomes.enabled()
}
