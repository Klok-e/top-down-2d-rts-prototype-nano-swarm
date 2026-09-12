//! Pure strategic intent planning.
//!
//! Controllers receive an immutable snapshot of the exact simulation state and
//! return edits which the runtime applies only for [`Controller::owner`]. The
//! module never writes ECS state or chooses work for individual nanobots.

use std::collections::BTreeSet;

use bevy::prelude::{IVec2, Vec2};

use crate::{
    intent::{IntentGrid, IntentKind},
    nanobot::{NanobotType, SwarmId},
    navigation::Obstacle,
};

/// Maximum accounted planner operations shared by all decisions in one review window.
pub const PLANNING_WORK_BUDGET: usize = 100_000;
pub const REVIEW_PERIOD_TICKS: u64 = crate::SIMULATION_HZ as u64 / 2;

const EDIT_WORK_RESERVE: usize = 4_096;
const MAX_BOTS: usize = 256;
const MAX_STRUCTURES: usize = 128;
const MAX_DEPOSITS: usize = 24;
const MAX_ACTIVE_CELLS: usize = 256;
const MAX_PLAN_CELLS: usize = 96;
const SWITCH_MARGIN: f32 = 1.15;
const MIN_SECOND_FRONT_DEFENDERS: usize = 4;
const MIN_FRONT_HEALTH: u32 = crate::nanobot::NANOBOT_DEFAULT_MAX_HEALTH / 2;
const MIN_FRONT_CHARGE: f32 = 0.45;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SwarmState {
    pub id: SwarmId,
    pub home: Vec2,
    pub minerals: u32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BotState {
    pub id: u64,
    pub owner: SwarmId,
    pub kind: NanobotType,
    pub position: Vec2,
    pub health: u32,
    pub charge: f32,
    pub cargo: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StructureKind {
    Facility,
    Source,
    Sink,
    Charger,
    Planned,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StructureState {
    pub id: u64,
    pub owner: SwarmId,
    pub position: Vec2,
    pub kind: StructureKind,
    pub health: u32,
    pub minerals: u32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DepositState {
    pub id: u64,
    pub position: Vec2,
    pub amount: u32,
    pub radius: f32,
}

#[derive(Debug)]
pub struct GameState<'a> {
    pub grid: &'a IntentGrid,
    pub swarms: &'a [SwarmState],
    pub bots: &'a [BotState],
    pub structures: &'a [StructureState],
    pub deposits: &'a [DepositState],
    pub terrain: &'a [Obstacle],
    pub tick: u64,
    pub finished: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntentEditAction {
    Paint,
    Erase,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IntentEdit {
    pub cell: IVec2,
    pub kind: IntentKind,
    pub action: IntentEditAction,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decision {
    pub edits: Vec<IntentEdit>,
    pub explanation: String,
    pub reviewed: bool,
    pub work_units: usize,
}

impl Decision {
    fn idle(explanation: impl Into<String>) -> Self {
        Self {
            edits: Vec::new(),
            explanation: explanation.into(),
            reviewed: false,
            work_units: 0,
        }
    }
}

#[derive(Debug)]
pub struct Controller {
    owner: SwarmId,
    state: lifecycle::PlannerState,
}

#[derive(Debug, Clone)]
struct IntentPlan {
    objective: Objective,
    primary_deposit: Option<u64>,
    attack_targets: Vec<AttackObligation>,
    key_charger: Option<u64>,
    desired: Vec<DesiredIntent>,
}

#[derive(Debug, Default)]
struct PlanValidation {
    invalidation: Option<String>,
    observed_attack_targets: Vec<AttackObligation>,
    covered_attack_progress: bool,
    covered_attack_movement: bool,
    all_attack_targets_covered: bool,
}

#[derive(Debug, Clone, Copy, Default)]
struct EconomySelection {
    primary: Option<DepositState>,
    value: f32,
}

#[derive(Debug, Clone)]
struct PlanCandidate {
    objective: Objective,
    economy: EconomySelection,
    attack_targets: Vec<AttackObligation>,
    anchor: IVec2,
    attack_target: Option<IVec2>,
    key_charger: Option<StructureState>,
    defend_target: usize,
    score: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AttackTargetKind {
    Structure,
    Nanobot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AttackObligation {
    kind: AttackTargetKind,
    id: u64,
    cell: IVec2,
    health: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Objective {
    HoldHome,
    SecureDeposit(u64),
    ForwardPressure(IVec2),
    RaidLogistics(u64),
    HuntRemnant(u64),
}

impl Objective {
    fn label(self) -> String {
        match self {
            Self::HoldHome => "hold the home economy".to_string(),
            Self::SecureDeposit(id) => format!("secure Resource Deposit {id}"),
            Self::ForwardPressure(cell) => format!("establish a supplied front at {cell}"),
            Self::RaidLogistics(id) => format!("attack enemy logistics structure {id}"),
            Self::HuntRemnant(id) => format!("hunt remaining Nanobot {id}"),
        }
    }
}

impl IntentPlan {
    fn is_coordinated(&self) -> bool {
        [IntentKind::Build, IntentKind::Defend, IntentKind::Corridor]
            .into_iter()
            .all(|kind| self.desired.iter().any(|intent| intent.kind == kind))
            && (self.primary_deposit.is_none()
                || self
                    .desired
                    .iter()
                    .any(|intent| intent.kind == IntentKind::Gather))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DesiredIntent {
    cell: IVec2,
    kind: IntentKind,
}

#[derive(Debug, Default)]
struct ForceEstimate {
    friendly_defenders: usize,
    friendly_strength: f32,
    enemy_strength: f32,
    low_charge_defenders: usize,
}

mod lifecycle;
mod materialize;
mod strategy;

#[cfg(test)]
mod tests;
