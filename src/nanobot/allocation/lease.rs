//! Category-neutral regional lease lifecycle and ECS adapter.

use crate::nanobot::work_access::{WorkAccess, WorkReachability};
use crate::nanobot::{Cargo, NanobotType, SwarmMember};
use bevy::prelude::*;

use super::{
    ActionableProjection, AllocationClock, AllocationRegion, OpportunityCategory, OpportunityTarget,
};
use crate::nanobot::{
    DirectMovementComponent, ExtractProgress, GatherAssignment, HaulerAssignment, HaulerLoading,
    LogisticsReservation, MaintenanceAssignment, MaintenanceProgress, MaterialWorkProgress,
    PlannedStructureClaim, PlannedStructureProgress, SwarmId,
};

#[derive(Debug, Clone, Copy, PartialEq)]
struct LeaseTravel {
    waypoint: Vec2,
    best_distance: f32,
    pending_step: bool,
}

/// Temporary ownership of projected capacity, not ownership of authoritative
/// intent or category-specific work state.
#[derive(Debug, Clone, Copy, Component, PartialEq)]
pub struct RegionalLease {
    pub region: AllocationRegion,
    pub category: OpportunityCategory,
    pub target: OpportunityTarget,
    pub owner: Option<SwarmId>,
    progress: u64,
    progress_checkpoint: u64,
    expires_at_tick: u64,
    travel: Option<LeaseTravel>,
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
            progress,
            progress_checkpoint: progress,
            expires_at_tick: now_tick.saturating_add(no_progress_ttl_ticks.max(1)),
            travel: None,
        }
    }

    pub(crate) fn rescale_deadline(&mut self, now_tick: u64, old_ttl: u64, new_ttl: u64) {
        let remaining = self.expires_at_tick.saturating_sub(now_tick);
        let scaled = u128::from(remaining)
            .saturating_mul(u128::from(new_ttl.max(1)))
            .div_ceil(u128::from(old_ttl.max(1)))
            .min(u128::from(u64::MAX)) as u64;
        self.expires_at_tick = now_tick.saturating_add(scaled);
    }

    pub fn progress_checkpoint(self) -> u64 {
        self.progress_checkpoint
    }

    pub fn expires_at_tick(self) -> u64 {
        self.expires_at_tick
    }

    pub(crate) fn record_work(&mut self) {
        self.progress = self.progress.saturating_add(1);
    }

    pub(crate) fn begin_travel_step(&mut self, position: Vec2, waypoint: Vec2) {
        if let Some(travel) = &mut self.travel
            && travel.waypoint == waypoint
        {
            travel.pending_step = true;
        } else {
            self.travel = Some(LeaseTravel {
                waypoint,
                best_distance: position.distance(waypoint),
                pending_step: true,
            });
        }
    }

    pub(crate) fn finish_travel_step(&mut self, start: Vec2, end: Vec2) {
        let Some(travel) = &mut self.travel else {
            return;
        };
        if !std::mem::take(&mut travel.pending_step) || start == end {
            return;
        }
        let distance = end.distance(travel.waypoint);
        if distance + 0.001 < travel.best_distance && distance < start.distance(travel.waypoint) {
            travel.best_distance = distance;
            self.progress = self.progress.saturating_add(1);
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
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub fn maintain_regional_leases_system(
    mut commands: Commands,
    clock: Res<AllocationClock>,
    tuning: Res<crate::tuning::Tuning>,
    projection: Res<ActionableProjection>,
    access: WorkAccess,
    bodies: Query<&Transform, (With<SwarmMember>, With<NanobotType>)>,
    mut leases: Query<(
        Entity,
        &mut RegionalLease,
        Option<&Cargo>,
        Option<&LogisticsReservation>,
    )>,
) {
    for (entity, mut lease, cargo, reservation) in &mut leases {
        // Physical loads outlive task leases; their delivery systems retain custody.
        if cargo.is_some_and(|cargo| cargo.amount > 0) {
            continue;
        }
        let reachability = bodies.get(entity).ok().map(|transform| {
            let target = match lease.target {
                OpportunityTarget::Gather { deposit, .. } => deposit,
                OpportunityTarget::PlannedBuild { structure, .. }
                | OpportunityTarget::Maintenance { structure } => structure,
                OpportunityTarget::Haul { source, .. } => source,
            };
            let Some(region) = access.region(target) else {
                return WorkReachability::Unreachable;
            };
            let approach = access.reachability_from(transform.translation.truncate(), region);
            if approach == WorkReachability::Unreachable {
                return approach;
            }
            if let OpportunityTarget::Haul { sink, .. } = lease.target {
                let Some(destination) = access.region(sink) else {
                    return WorkReachability::Unreachable;
                };
                let delivery =
                    access.chain_from(transform.translation.truncate(), region, destination);
                if delivery == WorkReachability::Unreachable {
                    return delivery;
                }
            }
            approach
        });
        let observed_progress = lease.progress;
        let supported = reachability != Some(WorkReachability::Unreachable)
            && (reservation.is_some() || projection_supports_lease(&projection, &lease));
        let decision = evaluate_lease(
            &mut lease,
            clock.tick(),
            observed_progress,
            supported,
            tuning.autonomy.lease_no_progress_ttl_ticks,
        );
        if decision != LeaseDecision::Keep {
            let category = lease.category;
            let mut entity_commands = commands.entity(entity);
            entity_commands
                .remove::<RegionalLease>()
                .remove::<DirectMovementComponent>()
                .insert(crate::nanobot::Commitment::Idle);
            match category {
                OpportunityCategory::Gather => {
                    entity_commands
                        .remove::<GatherAssignment>()
                        .remove::<ExtractProgress>()
                        .remove::<LogisticsReservation>()
                        .remove::<(Cargo, MaterialWorkProgress)>();
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
                OpportunityCategory::Haul => {
                    entity_commands
                        .remove::<HaulerAssignment>()
                        .remove::<HaulerLoading>()
                        .remove::<LogisticsReservation>()
                        .remove::<(Cargo, MaterialWorkProgress)>();
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
        Option<&HaulerAssignment>,
    )>,
) {
    for (entity, lease, gather, planned, maintenance, haul) in &leases {
        let active = match lease.category {
            OpportunityCategory::Gather => gather.is_some(),
            OpportunityCategory::PlannedBuild => planned.is_some(),
            OpportunityCategory::Maintenance => maintenance.is_some(),
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
