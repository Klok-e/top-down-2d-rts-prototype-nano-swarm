//! Read-only world observation and owner-scoped application of strategic intent.

use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::{
    intent::IntentGrid,
    nanobot::{
        Cargo, Charge, Charger, Health, MatchOutcome, Nanobot, NanobotType, OwnerSwarm,
        PlannedStructure, ProductionFacility, RegionalAllocationSet, Structure, Swarm, SwarmId,
        SwarmMember,
    },
    resources::{ResourceDeposit, ResourceKind, ResourceLedger, Stockpile, StockpileRole},
    strategic_controller::{
        BotState, Controller, DepositState, GameState, IntentEditAction, StructureKind,
        StructureState, SwarmState,
    },
    terrain::RockFormation,
};

#[derive(Component, Debug)]
pub struct StrategicController {
    planner: Controller,
}

impl StrategicController {
    pub fn new(owner: SwarmId) -> Self {
        Self {
            planner: Controller::new(owner),
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
    pub last_explanation: String,
}

#[derive(Resource, Default)]
pub struct ControllerTelemetry {
    profiles: BTreeMap<u32, ControllerProfile>,
}

impl ControllerTelemetry {
    pub fn profiles(&self) -> BTreeMap<u32, ControllerProfile> {
        self.profiles.clone()
    }

    fn record_review(
        &mut self,
        owner: u32,
        intent_edits: usize,
        work_units: usize,
        explanation: String,
    ) {
        let profile = self.profiles.entry(owner).or_default();
        profile.reviews += 1;
        profile.intent_edits += intent_edits as u64;
        profile.work_units += work_units as u64;
        profile.last_explanation = explanation;
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
        let decision = controller.planner.decide(&state);
        if decision.reviewed {
            telemetry.record_review(
                owner.0,
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
            match edit.action {
                IntentEditAction::Paint => grid.paint(edit.cell, edit.kind, owner),
                IntentEditAction::Erase => grid.erase(edit.cell, edit.kind, owner),
            };
        }
    }
}
