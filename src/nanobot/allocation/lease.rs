//! Category-neutral regional lease lifecycle and ECS adapter.

use bevy::prelude::*;

use super::{
    ActionableProjection, AllocationClock, AllocationRegion, OpportunityCategory, OpportunityTarget,
};
use crate::nanobot::{
    ChargerAssignment, ChargerProgress, DefendAssignment, DefendHold, DirectMovementComponent,
    ExtractProgress, GatherAssignment, HaulerAssignment, HaulerLoading, HaulerRoute,
    LogisticsReservation, MaintenanceAssignment, MaintenanceProgress, PlannedStructureClaim,
    PlannedStructureProgress, SwarmId,
};

/// Charge override state for a regional lease.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegionalLeaseState {
    Active,
    SuspendedForCharge,
    ResumePending,
}

/// Temporary ownership of projected capacity, not ownership of authoritative
/// intent or category-specific work state.
#[derive(Debug, Clone, Copy, Component, PartialEq, Eq)]
pub struct RegionalLease {
    pub region: AllocationRegion,
    pub category: OpportunityCategory,
    pub target: OpportunityTarget,
    pub owner: Option<SwarmId>,
    pub state: RegionalLeaseState,
    progress_checkpoint: u64,
    expires_at_tick: u64,
}

impl RegionalLease {
    pub fn new(
        region: AllocationRegion,
        category: OpportunityCategory,
        target: OpportunityTarget,
        owner: Option<SwarmId>,
        now_tick: u64,
        progress: u64,
        no_progress_ttl_ticks: u64,
    ) -> Self {
        Self {
            region,
            category,
            target,
            owner,
            state: RegionalLeaseState::Active,
            progress_checkpoint: progress,
            expires_at_tick: now_tick.saturating_add(no_progress_ttl_ticks.max(1)),
        }
    }

    pub fn progress_checkpoint(self) -> u64 {
        self.progress_checkpoint
    }

    pub fn expires_at_tick(self) -> u64 {
        self.expires_at_tick
    }

    /// Suspended and resume-pending leases permit temporary replacement.
    pub fn counts_toward_capacity(self) -> bool {
        self.state == RegionalLeaseState::Active
    }

    pub fn suspend_for_charge(&mut self) {
        self.state = RegionalLeaseState::SuspendedForCharge;
    }

    pub fn request_resume(&mut self) {
        if self.state == RegionalLeaseState::SuspendedForCharge {
            self.state = RegionalLeaseState::ResumePending;
        }
    }

    /// Resume only after the allocator confirms category capacity remains.
    pub fn activate_if_capacity(&mut self, capacity_available: bool) -> bool {
        if self.state == RegionalLeaseState::ResumePending && capacity_available {
            self.state = RegionalLeaseState::Active;
            true
        } else {
            false
        }
    }
}

/// Monotonic progress supplied by category-specific adapters.
#[derive(Debug, Clone, Copy, Component, Default, PartialEq, Eq)]
pub struct LeaseProgress(pub u64);

/// No-progress lifetime measured in allocation ticks.
#[derive(Debug, Clone, Copy, Resource, PartialEq, Eq)]
pub struct RegionalLeaseConfig {
    pub no_progress_ttl_ticks: u64,
}

impl Default for RegionalLeaseConfig {
    fn default() -> Self {
        Self {
            no_progress_ttl_ticks: 30,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeaseDecision {
    Keep,
    RevokeUnsupported,
    RevokeNoProgress,
}

/// Evaluate one lease. Invalid support always wins, including between 10 Hz
/// allocation ticks. Measurable monotonic progress renews its deadline.
pub fn evaluate_lease(
    lease: &mut RegionalLease,
    now_tick: u64,
    observed_progress: u64,
    supported: bool,
    no_progress_ttl_ticks: u64,
) -> LeaseDecision {
    if !supported {
        return LeaseDecision::RevokeUnsupported;
    }
    if observed_progress > lease.progress_checkpoint {
        lease.progress_checkpoint = observed_progress;
        lease.expires_at_tick = now_tick.saturating_add(no_progress_ttl_ticks.max(1));
        return LeaseDecision::Keep;
    }
    if now_tick >= lease.expires_at_tick {
        LeaseDecision::RevokeNoProgress
    } else {
        LeaseDecision::Keep
    }
}

/// Projection support check used by pure and ECS adapters.
pub fn projection_supports_lease(projection: &ActionableProjection, lease: &RegionalLease) -> bool {
    projection
        .opportunities(lease.region)
        .iter()
        .any(|opportunity| {
            opportunity.category == lease.category
                && opportunity.target == lease.target
                && owners_compatible(opportunity.owner, lease.owner)
                && opportunity.available_work > 0
        })
}

/// Revoke unsupported or stalled leases after projection refresh.
#[allow(clippy::type_complexity)]
pub fn maintain_regional_leases_system(
    mut commands: Commands,
    clock: Res<AllocationClock>,
    config: Res<RegionalLeaseConfig>,
    projection: Res<ActionableProjection>,
    mut leases: Query<(
        Entity,
        &mut RegionalLease,
        Option<&LeaseProgress>,
        Option<&GatherAssignment>,
        Option<&PlannedStructureClaim>,
        Option<&MaintenanceAssignment>,
        Option<&DefendAssignment>,
        Option<&DefendHold>,
        Option<&HaulerAssignment>,
        Option<&LogisticsReservation>,
    )>,
) {
    for (
        entity,
        mut lease,
        progress,
        gather,
        planned,
        maintenance,
        defend,
        hold,
        haul,
        reservation,
    ) in &mut leases
    {
        let lifecycle_active = gather.is_some()
            || planned.is_some()
            || maintenance.is_some()
            || defend.is_some()
            || hold.is_some()
            || haul.is_some();
        let observed_progress = progress.map_or_else(
            || {
                lease
                    .progress_checkpoint
                    .saturating_add(u64::from(lifecycle_active))
            },
            |value| value.0,
        );
        let supported = reservation.is_some() || projection_supports_lease(&projection, &lease);
        let decision = evaluate_lease(
            &mut lease,
            clock.tick(),
            observed_progress,
            supported,
            config.no_progress_ttl_ticks,
        );
        if decision != LeaseDecision::Keep {
            let category = lease.category;
            let mut entity_commands = commands.entity(entity);
            entity_commands
                .remove::<RegionalLease>()
                .remove::<DirectMovementComponent>();
            match category {
                OpportunityCategory::Gather => {
                    entity_commands
                        .remove::<GatherAssignment>()
                        .remove::<ExtractProgress>()
                        .remove::<LogisticsReservation>();
                }
                OpportunityCategory::PlannedBuild => {
                    entity_commands
                        .remove::<PlannedStructureClaim>()
                        .remove::<PlannedStructureProgress>();
                }
                OpportunityCategory::Maintenance => {
                    entity_commands
                        .remove::<MaintenanceAssignment>()
                        .remove::<MaintenanceProgress>();
                }
                OpportunityCategory::Defend => {
                    entity_commands
                        .remove::<DefendAssignment>()
                        .remove::<DefendHold>();
                }
                OpportunityCategory::Haul => {
                    entity_commands
                        .remove::<HaulerAssignment>()
                        .remove::<HaulerLoading>()
                        .remove::<LogisticsReservation>()
                        .remove::<HaulerRoute>();
                }
            }
        }
    }
}

/// Release capacity as soon as category lifecycle markers finish.
#[allow(clippy::type_complexity)]
pub fn release_finished_regional_leases_system(
    mut commands: Commands,
    leases: Query<(
        Entity,
        &RegionalLease,
        Option<&GatherAssignment>,
        Option<&PlannedStructureClaim>,
        Option<&MaintenanceAssignment>,
        Option<&DefendAssignment>,
        Option<&DefendHold>,
        Option<&HaulerAssignment>,
        Option<&ChargerAssignment>,
        Option<&ChargerProgress>,
    )>,
) {
    for (entity, lease, gather, planned, maintenance, defend, hold, haul, charger, charging) in
        &leases
    {
        if lease.state != RegionalLeaseState::Active {
            continue;
        }
        let active = match lease.category {
            OpportunityCategory::Gather => gather.is_some(),
            OpportunityCategory::PlannedBuild => planned.is_some(),
            OpportunityCategory::Maintenance => maintenance.is_some(),
            OpportunityCategory::Defend => {
                defend.is_some() || hold.is_some() || charger.is_some() || charging.is_some()
            }
            OpportunityCategory::Haul => haul.is_some(),
        };
        if !active {
            commands.entity(entity).remove::<RegionalLease>();
        }
    }
}

fn owners_compatible(left: Option<SwarmId>, right: Option<SwarmId>) -> bool {
    left.is_none() || right.is_none() || left == right
}
