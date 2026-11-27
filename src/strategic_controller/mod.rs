//! Pure strategic intent planning.
//!
//! Controllers receive an immutable snapshot of the exact simulation state and
//! return edits which the runtime applies only for [`Controller::owner`]. The
//! module never writes ECS state or chooses work for individual nanobots.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use crate::tuning::{AutonomyTuning, ChargeTuning, LogisticsTuning};

use bevy::prelude::{IVec2, Vec2};

use crate::{
    intent::{IntentGrid, IntentKind},
    nanobot::{HaulEconomy, NanobotType, SwarmEconomy, SwarmId},
    navigation::{Obstacle, RouteGoal, RouteOutcome},
    reachability::SwarmReachabilityMap,
};

/// Maximum accounted planner operations shared by all decisions in one review window.
pub const PLANNING_WORK_BUDGET: usize = 100_000;
const WORK_WINDOW_TICKS: u64 = crate::SIMULATION_HZ as u64 / 2;

const EDIT_WORK_RESERVE: usize = 4_096;
const MAX_ACTIVE_CELLS: usize = 256;
const EXPANSION_PLAN_CELLS: usize = 96;
const MAX_PLAN_CELLS: usize = EDIT_WORK_RESERVE / 2;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SwarmState {
    pub id: SwarmId,
    pub home: Vec2,
    /// Minerals currently held in this swarm’s operational structure buffers.
    pub minerals: u32,
    pub mineral_income_per_second: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BotState {
    pub id: u64,
    pub owner: SwarmId,
    pub kind: NanobotType,
    pub position: Vec2,
    pub health: u32,
    pub max_health: u32,
    pub charge: f32,
    pub max_charge: f32,
    pub cargo: u32,
    pub displacement: Vec2,
    pub defender_duty: DefenderDuty,
}

#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub enum DefenderDuty {
    #[default]
    Available,
    Unavailable,
    RespondingTo(Vec2),
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
    pub operational: bool,
    pub health: u32,
    pub max_health: u32,
    pub minerals: u32,
}

/// The physical bounds of an owned structure that still relies on Build intent.
#[derive(Debug, Clone, Copy)]
pub struct BuildFootprint {
    pub owner: SwarmId,
    pub center: Vec2,
    pub half: Vec2,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DepositState {
    pub id: u64,
    pub owner: Option<SwarmId>,
    pub position: Vec2,
    pub amount: u32,
    pub radius: f32,
}

/// A shared navigation query. Ask for each work unit and return no outcome
/// if the planner denies further work.
pub type RouteQuery<'a> =
    dyn Fn(Vec2, RouteGoal, &mut dyn FnMut() -> bool) -> Option<RouteOutcome> + 'a;

#[derive(Debug, Clone, Copy, Default)]
pub struct MineObservation {
    pub worker_capacity: u32,
    pub active_workers: u32,
    pub delivered_minerals: u64,
    pub completed_dropoffs: u32,
    pub observation_ticks: u64,
    pub ticks_since_dropoff: Option<u64>,
    pub has_source: bool,
    pub has_space: bool,
    pub sustained_dropoffs: bool,
}

impl MineObservation {
    pub fn is_saturated(&self) -> bool {
        self.worker_capacity > 0
            && self.active_workers >= self.worker_capacity
            && self.sustained_dropoffs
    }
}

/// Recent Worker activity, completed deliveries and reachable Source capacity at a deposit.
pub type MineQuery<'a> =
    dyn Fn(SwarmId, &DepositState, IVec2, &mut dyn FnMut() -> bool) -> Option<MineObservation> + 'a;

#[derive(Debug, Clone, Copy, Default)]
pub struct EconomyObservation {
    pub gather: SwarmEconomy,
    pub haul: Option<HaulEconomy>,
    pub low_reserve_ticks: u64,
    pub upkeep_workers: u32,
    pub committed_haulers: u32,
}

#[derive(Clone, Copy)]
pub struct SupplyState<'a> {
    pub economies: &'a BTreeMap<SwarmId, EconomyObservation>,
    pub logistics: &'a LogisticsTuning,
}

pub struct GameState<'a> {
    pub grid: &'a IntentGrid,
    pub swarms: &'a [SwarmState],
    pub bots: &'a [BotState],
    pub structures: &'a [StructureState],
    pub build_footprints: &'a [BuildFootprint],
    pub deposits: &'a [DepositState],
    pub terrain: &'a [Obstacle],
    pub charge: &'a ChargeTuning,
    /// Present for gameplay controllers; omitted by terrain-agnostic unit fixtures.
    pub reachability: Option<&'a SwarmReachabilityMap>,
    pub route_query: Option<&'a RouteQuery<'a>>,
    pub mine_query: Option<&'a MineQuery<'a>>,
    pub supply: Option<SupplyState<'a>>,
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
    owned_deposits: Vec<economy::OwnedDeposit>,
    attack: Option<AttackObligation>,
    staging: Option<staging::StagingFront>,
    key_charger: Option<u64>,
    desired: HashSet<DesiredIntent>,
    reachability: Option<SwarmReachabilityMap>,
    grid_size: IVec2,
}

#[derive(Debug, Default)]
struct PlanValidation {
    lost_deposit: Option<u64>,
    invalidation: Option<String>,
    attack_progressed: bool,
    covered_attack: bool,
    covered_attack_movement: bool,
}

#[derive(Debug, Clone)]
struct PlanCandidate {
    objective: Objective,
    attack: Option<AttackObligation>,
    anchor: IVec2,
    attack_target: Option<IVec2>,
    approach: Option<RouteGoal>,
    score: f32,
    protected_home: Option<IVec2>,
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
    DefendDeposit(u64),
    ContestDeposit(u64),
    RaidDeposit { deposit: u64, target: u64 },
    DenyDeposit(u64),
    PushHome(SwarmId),
    FinishStructure(u64),
    HuntRemnant(u64),
}

impl Objective {
    fn requires_raid_readiness(self) -> bool {
        matches!(self, Self::RaidDeposit { .. } | Self::DenyDeposit(_))
    }

    fn priority(self) -> u8 {
        match self {
            Self::DefendDeposit(_) => 6,
            Self::ContestDeposit(_) => 5,
            Self::SecureDeposit(_) => 4,
            Self::RaidDeposit { .. } | Self::DenyDeposit(_) => 3,
            Self::PushHome(_) => 2,
            Self::FinishStructure(_) | Self::HuntRemnant(_) => 1,
            Self::HoldHome => 0,
        }
    }

    fn label(self) -> String {
        match self {
            Self::HoldHome => "hold the home economy".to_string(),
            Self::SecureDeposit(id) => format!("secure Resource Deposit {id}"),
            Self::DefendDeposit(id) => format!("defend owned Resource Deposit {id}"),
            Self::ContestDeposit(id) => format!("contest neutral Resource Deposit {id}"),
            Self::RaidDeposit { deposit, target } => {
                format!("raid enemy Resource Deposit {deposit} at target {target}")
            }
            Self::DenyDeposit(id) => format!("raid enemy Resource Deposit {id}"),
            Self::PushHome(swarm) => format!("push enemy home of Swarm {}", swarm.0),
            Self::FinishStructure(id) => format!("finish remaining structure {id}"),
            Self::HuntRemnant(id) => format!("hunt remaining Nanobot {id}"),
        }
    }
}

impl IntentPlan {
    fn append_context(&self, explanation: &mut String) {
        if let Some(front) = &self.staging {
            explanation.push_str(&format!(", staging point {}", front.point()));
        } else {
            explanation.push_str(", no active staging point");
        }
        explanation.push_str(&format!(
            ", owned Resource Deposits {:?}",
            self.owned_deposits
                .iter()
                .map(|mine| mine.id)
                .collect::<Vec<_>>()
        ));
    }

    fn is_coordinated(&self) -> bool {
        let mut kinds = [false; IntentKind::COUNT];
        for intent in &self.desired {
            kinds[intent.kind.index()] = true;
        }
        kinds[IntentKind::Build.index()]
            && kinds[IntentKind::Defend.index()]
            && kinds[IntentKind::Corridor.index()]
            && (self.owned_deposits.is_empty() || kinds[IntentKind::Gather.index()])
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct DesiredIntent {
    cell: IVec2,
    kind: IntentKind,
}

#[derive(Debug, Default)]
struct ForceEstimate {
    friendly_strength: f32,
    enemy_strength: f32,
}

mod economy;
mod lifecycle;
mod materialize;
mod raid;
mod spending;
mod staging;
mod strategy;

#[cfg(test)]
mod tests;
