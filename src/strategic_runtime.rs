//! Read-only world observation and owner-scoped application of strategic intent.

use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, VecDeque},
    time::Instant,
};

use crate::{
    intent::IntentGrid,
    nanobot::{
        Cargo, Charge, Charger, Health, MatchOutcome, Nanobot, NanobotType, OwnerSwarm,
        PlannedStructure, ProductionFacility, RegionalAllocationSet, Structure, Swarm, SwarmId,
        SwarmMember,
    },
    resources::{ResourceDeposit, ResourceKind, ResourceLedger, Stockpile, StockpileRole},
    strategic_controller::{
        BotState, Controller, DepositState, GameState, StructureKind, StructureState, SwarmState,
    },
    terrain::RockFormation,
};

#[derive(Component, Debug)]
pub struct StrategicController {
    planner: Controller,
}

impl StrategicController {
    pub fn adaptive(owner: SwarmId) -> Self {
        Self {
            planner: Controller::adaptive(owner),
        }
    }

    pub fn timed(owner: SwarmId, assault: IVec2, target: IVec2, delay: u32, period: u32) -> Self {
        Self {
            planner: Controller::timed(owner, assault, target, delay, period),
        }
    }

    pub fn owner(&self) -> SwarmId {
        self.planner.owner()
    }
}

pub struct StrategicControllerPlugin;

impl Plugin for StrategicControllerPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ControllerTelemetry>().add_systems(
            FixedUpdate,
            strategic_intent_system.before(RegionalAllocationSet::Project),
        );
    }
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct ControllerProfile {
    pub reviews: u64,
    pub intent_edits: u64,
    pub work_units: u64,
    pub mean_ms: f64,
    pub p95_ms: f64,
    /// Number of most recent reviews represented by `p95_ms`.
    pub p95_window_samples: usize,
    pub max_ms: f64,
    pub last_explanation: String,
}

/// Maximum number of recent controller reviews represented by `ControllerProfile::p95_ms`.
pub const CONTROLLER_P95_WINDOW_REVIEWS: usize = 4_096;

#[derive(Default)]
struct ReviewDurationWindow {
    samples: VecDeque<f64>,
}

impl ReviewDurationWindow {
    fn record(&mut self, duration_ms: f64) {
        if self.samples.len() == CONTROLLER_P95_WINDOW_REVIEWS {
            self.samples.pop_front();
        }
        self.samples.push_back(duration_ms);
    }

    fn p95_ms(&self) -> Option<f64> {
        if self.samples.is_empty() {
            return None;
        }
        let mut sorted = self.samples.iter().copied().collect::<Vec<_>>();
        sorted.sort_unstable_by(f64::total_cmp);
        Some(sorted[(sorted.len() * 95).div_ceil(100).saturating_sub(1)])
    }
}

#[derive(Resource, Default)]
pub struct ControllerTelemetry {
    profiles: BTreeMap<u32, ControllerProfile>,
    samples: BTreeMap<u32, ReviewDurationWindow>,
    pub observation_max_ms: f64,
}

impl ControllerTelemetry {
    pub fn profiles(&self) -> BTreeMap<u32, ControllerProfile> {
        self.profiles
            .iter()
            .map(|(&owner, profile)| {
                let mut profile = profile.clone();
                if let Some(window) = self.samples.get(&owner) {
                    profile.p95_window_samples = window.samples.len();
                    if let Some(p95_ms) = window.p95_ms() {
                        profile.p95_ms = p95_ms;
                    }
                }
                (owner, profile)
            })
            .collect()
    }

    fn record_review(
        &mut self,
        owner: u32,
        elapsed_ms: f64,
        intent_edits: usize,
        work_units: usize,
        explanation: String,
    ) {
        let profile = self.profiles.entry(owner).or_default();
        profile.reviews += 1;
        profile.intent_edits += intent_edits as u64;
        profile.work_units += work_units as u64;
        profile.mean_ms += (elapsed_ms - profile.mean_ms) / profile.reviews as f64;
        profile.max_ms = profile.max_ms.max(elapsed_ms);
        profile.last_explanation = explanation;
        self.samples.entry(owner).or_default().record(elapsed_ms);
    }
}

#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn strategic_intent_system(
    mut controllers: Query<(Entity, &SwarmId, &mut StrategicController)>,
    swarms: Query<(Entity, &SwarmId, Option<&Transform>), With<Swarm>>,
    bots: Query<
        (
            Entity,
            &SwarmMember,
            &NanobotType,
            &Transform,
            &Health,
            Option<&Charge>,
            Option<&Cargo>,
        ),
        With<Nanobot>,
    >,
    structures: Query<
        (
            Entity,
            &Transform,
            Option<&OwnerSwarm>,
            Option<&Structure>,
            Option<&ProductionFacility>,
            Option<&Stockpile>,
            Option<&StockpileRole>,
            Option<&Charger>,
            Option<&PlannedStructure>,
        ),
        Or<(
            With<Structure>,
            With<ProductionFacility>,
            With<Stockpile>,
            With<Charger>,
            With<PlannedStructure>,
        )>,
    >,
    deposits: Query<(Entity, &Transform, &ResourceDeposit)>,
    rocks: Query<(&RockFormation, &Transform)>,
    ledger: Res<ResourceLedger>,
    mut grid: ResMut<IntentGrid>,
    time: Res<Time<Fixed>>,
    outcome: Option<Res<MatchOutcome>>,
    mut telemetry: ResMut<ControllerTelemetry>,
) {
    if controllers.is_empty()
        || outcome
            .as_deref()
            .is_some_and(|outcome| *outcome != MatchOutcome::InProgress)
    {
        return;
    }
    let started = Instant::now();
    let owners: BTreeMap<_, _> = swarms.iter().map(|(entity, id, _)| (entity, *id)).collect();
    let mut swarm_states: Vec<_> = swarms
        .iter()
        .map(|(_, id, position)| SwarmState {
            id: *id,
            home: position.map_or(Vec2::ZERO, |t| t.translation.truncate()),
            minerals: ledger.total_for(*id, ResourceKind::Minerals),
        })
        .collect();
    swarm_states.sort_by_key(|swarm| swarm.id);
    let mut bot_states: Vec<_> = bots
        .iter()
        .map(
            |(entity, owner, kind, transform, health, charge, cargo)| BotState {
                id: entity.to_bits(),
                owner: owner.0,
                kind: *kind,
                position: transform.translation.truncate(),
                health: health.current,
                charge: charge.map_or(1.0, |c| c.current),
                cargo: cargo.map_or(0, |c| c.amount),
            },
        )
        .collect();
    bot_states.sort_by_key(|bot| bot.id);
    let mut structure_states: Vec<_> = structures
        .iter()
        .filter_map(
            |(entity, transform, owner, condition, facility, stockpile, role, charger, plan)| {
                let owner = owner.and_then(|owner| owners.get(&owner.0)).copied()?;
                let kind = if plan.is_some() {
                    StructureKind::Planned
                } else if facility.is_some() {
                    StructureKind::Facility
                } else if charger.is_some() {
                    StructureKind::Charger
                } else if stockpile.is_some() {
                    if role == Some(&StockpileRole::Sink) {
                        StructureKind::Sink
                    } else {
                        StructureKind::Source
                    }
                } else {
                    StructureKind::Other
                };
                Some(StructureState {
                    id: entity.to_bits(),
                    owner,
                    position: transform.translation.truncate(),
                    kind,
                    health: condition.map_or(100, |c| c.health),
                    minerals: facility.map_or(0, |f| f.input_amount)
                        + stockpile.map_or(0, |s| s.amount)
                        + charger.map_or(0, |c| c.amount),
                })
            },
        )
        .collect();
    structure_states.sort_by_key(|structure| structure.id);
    let mut deposit_states: Vec<_> = deposits
        .iter()
        .map(|(entity, transform, deposit)| DepositState {
            id: entity.to_bits(),
            position: transform.translation.truncate(),
            amount: deposit.amount,
            radius: deposit.radius,
        })
        .collect();
    deposit_states.sort_by_key(|deposit| deposit.id);
    let terrain: Vec<_> = rocks
        .iter()
        .map(|(rock, transform)| rock.obstacle(transform))
        .collect();
    telemetry.observation_max_ms = telemetry
        .observation_max_ms
        .max(started.elapsed().as_secs_f64() * 1000.0);
    let state = GameState {
        grid: &grid,
        swarms: &swarm_states,
        bots: &bot_states,
        structures: &structure_states,
        deposits: &deposit_states,
        terrain: &terrain,
        tick: (time.elapsed_secs_f64() * crate::SIMULATION_HZ).round() as u64,
        finished: false,
    };
    let mut ordered: Vec<_> = controllers
        .iter()
        .map(|(entity, owner, _)| (*owner, entity))
        .collect();
    ordered.sort_by_key(|(owner, entity)| (*owner, entity.to_bits()));
    let mut decisions = Vec::with_capacity(ordered.len());
    for (owner, entity) in ordered {
        let Ok((_, _, mut controller)) = controllers.get_mut(entity) else {
            continue;
        };
        if controller.owner() != owner {
            continue;
        }
        telemetry.profiles.entry(owner.0).or_default();
        let started = Instant::now();
        let decision = controller.planner.decide(&state);
        if decision.reviewed {
            let elapsed = started.elapsed().as_secs_f64() * 1000.0;
            telemetry.record_review(
                owner.0,
                elapsed,
                decision.edits.len(),
                decision.work_units,
                decision.explanation.clone(),
            );
        } else {
            telemetry.profiles.entry(owner.0).or_default().work_units += decision.work_units as u64;
        }
        decisions.push((owner, decision));
    }
    for (owner, decision) in decisions {
        for edit in decision.edits {
            if edit.paint {
                grid.paint(edit.cell, edit.kind, owner);
            } else {
                grid.erase(edit.cell, edit.kind, owner);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn review_duration_history_is_bounded_for_unlimited_matches() {
        let mut telemetry = ControllerTelemetry::default();
        for duration in 0..=4_096 {
            telemetry.record_review(7, f64::from(duration), 0, 0, String::new());
        }

        let window = &telemetry.samples[&7];
        assert_eq!(window.samples.len(), CONTROLLER_P95_WINDOW_REVIEWS);
        assert_eq!(window.samples.front(), Some(&1.0));
        assert_eq!(window.samples.back(), Some(&4_096.0));
    }

    #[test]
    fn profile_p95_uses_the_nearest_rank_of_the_recent_window() {
        let mut telemetry = ControllerTelemetry::default();
        for duration in 1..=20 {
            telemetry.record_review(7, f64::from(duration), 0, 0, String::new());
        }

        let profiles = telemetry.profiles();
        let profile = &profiles[&7];
        assert_eq!(profile.p95_ms, 19.0);
        assert_eq!(profile.p95_window_samples, 20);
    }

    #[test]
    fn lifetime_totals_and_mean_are_not_reduced_to_the_recent_window() {
        let mut telemetry = ControllerTelemetry::default();
        telemetry.record_review(7, 100.0, 2, 3, "first".into());
        for _ in 0..CONTROLLER_P95_WINDOW_REVIEWS {
            telemetry.record_review(7, 0.0, 1, 2, "recent".into());
        }

        let profiles = telemetry.profiles();
        let profile = &profiles[&7];
        assert_eq!(profile.reviews, 4_097);
        assert_eq!(profile.intent_edits, 4_098);
        assert_eq!(profile.work_units, 8_195);
        assert!((profile.mean_ms - 100.0 / 4_097.0).abs() < 1e-12);
        assert_eq!(profile.p95_ms, 0.0);
        assert_eq!(profile.max_ms, 100.0);
        assert_eq!(profile.p95_window_samples, CONTROLLER_P95_WINDOW_REVIEWS);
        assert_eq!(profile.last_explanation, "recent");
    }
}
