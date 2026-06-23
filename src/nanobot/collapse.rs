//! Production Collapse win/loss detection for issue #16.
//!
//! A swarm collapses when it has no working production
//! capacity and too few remaining nanobots to rebuild it.
//! This module owns:
//!
//!   - a pure helper [`evaluate_collapse`] that decides
//!     "collapsed or not" from explicit inputs,
//!   - a Bevy system [`production_collapse_detection_system`]
//!     that runs every tick, gathers inputs for each swarm,
//!     and updates a [`ProductionCollapseState`] resource,
//!   - a [`ProductionCollapseState`] resource the UI layer
//!     (or a future game-over screen) reads.
//!
//! ## Decision model
//!
//! A swarm is **collapsed** when both:
//!
//!   1. no facility owned by the swarm is currently busy
//!      (`is_busy() == true`), AND
//!   2. the swarm does not have at least one Worker and one
//!      Hauler, the minimum crew that can gather and deliver
//!      minerals to feed the production chain's auto-creation
//!      path. A Worker alone is not enough because the
//!      production chain pulls from stockpiles, and a stockpile
//!      is only fed by haulers (Workers can carry small amounts
//!      but the production chain is calibrated against the
//!      full delivery flow).
//!
//! A swarm with **no unmet demand** (already at the production
//! ratio target) is never collapsed. "Nothing to produce" is
//! the success state, not the collapse state.

use bevy::prelude::*;

use crate::nanobot::autonomy::NanobotType;
use crate::nanobot::components::Swarm;
use crate::nanobot::production::{
    count_swarm_nanobots_by_type, total_deficit, OwnerSwarm, ProductionFacility, ProductionRatio,
    SwarmProduction,
};
use crate::nanobot::OpponentSwarm;

/// Why a swarm is or is not in Production Collapse. Stored on
/// the [`CollapseOutcome`] so callers (UI, tests, future
/// game-over screen) can distinguish "we won" from "we lost"
/// from "everything is fine" without re-deriving the inputs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CollapseReason {
    /// Default: the swarm is functioning or can recover. No
    /// collapse has been detected.
    #[default]
    NotCollapsed,
    /// No facility owned by the swarm is currently busy, AND
    /// the swarm does not have the nanobots required to
    /// rebuild production capacity.
    NoWorkingProductionAndInsufficientNanobots,
    /// No facility owned by the swarm is currently busy. The
    /// swarm still has enough nanobots to recover, so this is
    /// a warning state rather than a collapse.
    NoWorkingProduction,
    /// The swarm has at least one busy facility, so
    /// production is currently working. The reason field is
    /// kept so a caller can distinguish "production is
    /// running" from "no demand" without re-reading the
    /// inputs.
    Working,
}

/// Result of [`evaluate_collapse`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CollapseOutcome {
    pub collapsed: bool,
    pub reason: CollapseReason,
}

/// Minimum crew required to recover production. A single
/// Worker can extract minerals, but the production chain
/// consumes minerals from a local stockpile, and a stockpile
/// is fed by haulers. So both a Worker and a Hauler are
/// required for the auto-creation path to fire end-to-end.
const MIN_WORKERS_TO_RECOVER: u32 = 1;
const MIN_HAULERS_TO_RECOVER: u32 = 1;

/// Pure collapse decision. The Bevy system calls this for
/// each swarm after gathering the inputs; tests call it
/// directly with synthetic inputs.
///
/// Inputs:
///
///   - `busy_facilities`: number of `ProductionFacility`
///     entities owned by the swarm whose `is_busy()` returns
///     `true`. Any non-zero count counts as "production is
///     currently working".
///   - `workers`: number of Worker nanobots parented to the
///     swarm.
///   - `haulers`: number of Hauler nanobots parented to the
///     swarm.
///   - `has_unmet_demand`: `true` when the swarm's per-type
///     deficit is non-zero. A swarm that has reached its
///     production-ratio target is at rest, not collapsing.
///
/// A swarm with no unmet demand is never collapsed: there is
/// nothing to produce, so the absence of busy facilities is
/// the success state, not a failure state.
pub fn evaluate_collapse(
    busy_facilities: u32,
    workers: u32,
    haulers: u32,
    has_unmet_demand: bool,
) -> CollapseOutcome {
    if !has_unmet_demand {
        return CollapseOutcome::default();
    }
    let has_working_production = busy_facilities > 0;
    let can_recover = workers >= MIN_WORKERS_TO_RECOVER && haulers >= MIN_HAULERS_TO_RECOVER;
    let reason = if has_working_production {
        CollapseReason::Working
    } else if can_recover {
        CollapseReason::NoWorkingProduction
    } else {
        CollapseReason::NoWorkingProductionAndInsufficientNanobots
    };
    CollapseOutcome {
        collapsed: !has_working_production && !can_recover,
        reason,
    }
}

/// Bevy resource that records the latest collapse state for
/// each side. Read by the UI layer (or a future game-over
/// screen) to render a win/loss banner. The detection system
/// overwrites both fields every tick so callers always see
/// the most recent evaluation.
#[derive(Debug, Default, Resource, Clone, Copy)]
pub struct ProductionCollapseState {
    /// `true` when the player swarm is in Production Collapse.
    pub player_collapsed: bool,
    /// `true` when the opponent swarm is in Production Collapse.
    pub opponent_collapsed: bool,
}

impl ProductionCollapseState {
    /// Convenience: the player has won iff the opponent
    /// swarm has collapsed while the player swarm has not.
    /// "Both collapsed" is not a player win; the helpers
    /// stay separate so the UI can render the more nuanced
    /// state.
    pub fn player_won(&self) -> bool {
        self.opponent_collapsed && !self.player_collapsed
    }

    /// Convenience: the player has lost iff the player swarm
    /// has collapsed.
    pub fn player_lost(&self) -> bool {
        self.player_collapsed
    }
}

/// Bevy system: evaluate collapse for every swarm in the
/// world, then write the player / opponent flags into
/// [`ProductionCollapseState`]. Runs every tick; cheap
/// because it only counts owned nanobots and busy
/// facilities.
///
/// The chain sits after the production work system so the
/// "is a facility busy?" check sees the post-work state
/// (e.g. a facility that just finished a cycle is idle and
/// has not yet been picked again). Running before the work
/// system would report the facility as busy for one extra
/// tick after it should be re-evaluating.
///
/// Issue #38 / ADR-0004: the per-swarm count uses the
/// swarm's `SwarmId` rather than walking `Children`,
/// because nanobots are top-level entities. A swarm
/// without an explicit `SwarmId` falls back to
/// `SwarmId::PLAYER` for the count match, matching the
/// pre-multi-swarm behaviour where every nanobot's
/// `SwarmMember` defaulted to the player id.
#[allow(clippy::type_complexity)]
pub fn production_collapse_detection_system(
    swarms: Query<
        (
            Entity,
            Option<&crate::nanobot::components::SwarmId>,
            Option<&OpponentSwarm>,
        ),
        With<Swarm>,
    >,
    facilities: Query<(&ProductionFacility, &OwnerSwarm)>,
    nanobots: Query<
        (
            &crate::nanobot::NanobotType,
            &crate::nanobot::components::SwarmMember,
        ),
        With<crate::nanobot::components::Nanobot>,
    >,
    global_ratio: Res<ProductionRatio>,
    swarm_productions: Query<&SwarmProduction>,
    mut state: ResMut<ProductionCollapseState>,
) {
    state.player_collapsed = false;
    state.opponent_collapsed = false;
    for (swarm_entity, swarm_id, opponent) in &swarms {
        let swarm_id = swarm_id
            .copied()
            .unwrap_or(crate::nanobot::components::SwarmId::PLAYER);
        let counts = count_swarm_nanobots_by_type(swarm_id, &nanobots);
        let workers = *counts.get(&NanobotType::Worker).unwrap_or(&0);
        let haulers = *counts.get(&NanobotType::Hauler).unwrap_or(&0);

        // The swarm's own per-swarm ratio (if present) takes
        // precedence over the global ratio, matching the
        // production systems' behaviour.
        let ratio = swarm_productions
            .get(swarm_entity)
            .map(|sp| &sp.ratio)
            .unwrap_or(&*global_ratio);
        let has_unmet_demand = total_deficit(ratio, &counts) > 0;

        // Busy = "currently producing a nanobot". Idle
        // facilities (no current_target) do not count, because
        // they may be blocked on resources; the collapse rule
        // is about production that is actually working.
        let busy_facilities = facilities
            .iter()
            .filter(|(f, owner)| owner.0 == swarm_entity && f.is_busy())
            .count() as u32;

        let outcome = evaluate_collapse(busy_facilities, workers, haulers, has_unmet_demand);
        if outcome.collapsed {
            if opponent.is_some() {
                state.opponent_collapsed = true;
            } else {
                state.player_collapsed = true;
            }
        }
    }
}

/// Plugin that wires the production-collapse detection
/// system into the Update schedule. Auto-initialises the
/// [`ProductionCollapseState`] resource so callers do not
/// have to do it in their own startup system.
pub struct CollapsePlugin;

impl Plugin for CollapsePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ProductionCollapseState>().add_systems(
            Update,
            production_collapse_detection_system
                .after(crate::nanobot::production::production_facility_work_system),
        );
    }
}

#[cfg(test)]
mod tests {
    //! Unit tests for the pure collapse decision. Bevy-free
    //! so they cover the contract in isolation. End-to-end
    //! behaviour lives in
    //! `tests/production_collapse_behavior.rs`.

    use super::*;

    #[test]
    fn collapsed_when_no_production_and_no_haulers() {
        // A swarm that lost its facility and has no haulers
        // left cannot deliver minerals to a stockpile, so
        // the auto-creation path is dead. The worker alone
        // is not enough.
        let out = evaluate_collapse(0, 1, 0, true);
        assert!(out.collapsed);
        assert_eq!(
            out.reason,
            CollapseReason::NoWorkingProductionAndInsufficientNanobots
        );
    }

    #[test]
    fn collapsed_when_no_production_and_no_workers() {
        // Mirror of the previous test: haulers alone
        // cannot extract from deposits, so they cannot
        // rebuild production on their own.
        let out = evaluate_collapse(0, 0, 1, true);
        assert!(out.collapsed);
    }

    #[test]
    fn collapsed_when_swarm_is_fully_empty_and_still_has_demand() {
        // Edge case: a swarm with no nanobots and no
        // facility. The contract must report a collapse.
        let out = evaluate_collapse(0, 0, 0, true);
        assert!(out.collapsed);
        assert_eq!(
            out.reason,
            CollapseReason::NoWorkingProductionAndInsufficientNanobots
        );
    }

    #[test]
    fn not_collapsed_when_no_production_but_recoverable() {
        // The swarm lost its facility but still has the
        // minimum crew (1 worker + 1 hauler). It can
        // recover, so this is a warning state, not a
        // collapse.
        let out = evaluate_collapse(0, 1, 1, true);
        assert!(!out.collapsed);
        assert_eq!(out.reason, CollapseReason::NoWorkingProduction);
    }

    #[test]
    fn not_collapsed_when_production_is_working() {
        // A facility is busy: production is currently
        // producing. The swarm is healthy regardless of
        // the crew size.
        let out = evaluate_collapse(1, 0, 0, true);
        assert!(!out.collapsed);
        assert_eq!(out.reason, CollapseReason::Working);
    }

    #[test]
    fn not_collapsed_when_no_demand_even_without_production() {
        // A swarm that has reached its production ratio
        // target has no unmet demand. There is nothing to
        // produce, so "no busy facility" is the success
        // state, not a collapse.
        let out = evaluate_collapse(0, 0, 0, false);
        assert!(!out.collapsed);
        assert_eq!(out.reason, CollapseReason::NotCollapsed);
    }

    #[test]
    fn multiple_busy_facilities_still_means_working() {
        // Two or more busy facilities -- the swarm is
        // clearly healthy. The exact count does not change
        // the boolean decision.
        let out = evaluate_collapse(3, 0, 0, true);
        assert!(!out.collapsed);
        assert_eq!(out.reason, CollapseReason::Working);
    }

    #[test]
    fn production_collapse_state_default_is_neither_collapsed() {
        // The resource must default to "no collapse" so a
        // freshly started game does not flash a win/loss
        // banner before the first tick.
        let s = ProductionCollapseState::default();
        assert!(!s.player_collapsed);
        assert!(!s.opponent_collapsed);
        assert!(!s.player_won());
        assert!(!s.player_lost());
    }

    #[test]
    fn player_wins_when_opponent_collapsed_and_player_healthy() {
        let s = ProductionCollapseState {
            opponent_collapsed: true,
            ..Default::default()
        };
        assert!(s.player_won());
        assert!(!s.player_lost());
    }

    #[test]
    fn player_loses_when_player_collapsed() {
        let s = ProductionCollapseState {
            player_collapsed: true,
            ..Default::default()
        };
        assert!(s.player_lost());
        // Player_lost takes priority over player_won even
        // if both swarms happen to collapse. The UI can
        // show both flags separately for a richer state.
        let s = ProductionCollapseState {
            player_collapsed: true,
            opponent_collapsed: true,
        };
        assert!(s.player_lost());
        assert!(!s.player_won());
    }
}
