//! Shared tri-state work reachability for assignment, demand, and recovery.
use super::{
    InteractionRegion, Nanobot, NanobotType, OpportunityTarget, OwnerSwarm, ProductionFacility,
    SupportCondition, SwarmId, SwarmMember,
};
use crate::{
    navigation::{ConnectivityStatus, Navigation, RouteGoal},
    resources::ResourceDeposit,
};
use bevy::{ecs::system::SystemParam, prelude::*};
use std::collections::HashMap;

/// A delayed search is possible work until navigation proves otherwise.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum WorkReachability {
    Pending,
    Reachable,
    Unreachable,
}

#[derive(Default)]
struct WorkOrigins {
    tick: Option<u64>,
    by_crew: HashMap<(SwarmId, NanobotType, bool), std::sync::Arc<[Vec2]>>,
}

#[derive(SystemParam)]
#[allow(clippy::type_complexity)]
pub struct WorkAccess<'w, 's> {
    origins: Local<'s, std::cell::RefCell<WorkOrigins>>,
    navigation: Res<'w, Navigation>,
    bots: Query<
        'w,
        's,
        (
            &'static Transform,
            &'static NanobotType,
            &'static SwarmMember,
        ),
        With<Nanobot>,
    >,
    facilities: Query<
        'w,
        's,
        (
            &'static Transform,
            Option<&'static OwnerSwarm>,
            Option<&'static SupportCondition>,
        ),
        With<ProductionFacility>,
    >,
    swarms: Query<'w, 's, &'static SwarmId>,
    targets: Query<'w, 's, (&'static Transform, Option<&'static ResourceDeposit>)>,
}

impl WorkAccess<'_, '_> {
    pub(crate) fn region(&self, entity: Entity) -> Option<InteractionRegion> {
        self.targets.get(entity).ok().map(|(transform, deposit)| {
            deposit.map_or_else(
                || InteractionRegion::structure(transform),
                |deposit| InteractionRegion::deposit(transform, deposit.radius),
            )
        })
    }

    pub(crate) fn reachability_from(
        &self,
        start: Vec2,
        destination: InteractionRegion,
    ) -> WorkReachability {
        match self
            .navigation
            .query_connectivity(start, RouteGoal::Interaction(destination))
        {
            ConnectivityStatus::Pending => WorkReachability::Pending,
            ConnectivityStatus::Connected { .. } => WorkReachability::Reachable,
            ConnectivityStatus::Unreachable => WorkReachability::Unreachable,
        }
    }

    fn starts(
        &self,
        swarm: SwarmId,
        kind: NanobotType,
        include_production: bool,
    ) -> std::sync::Arc<[Vec2]> {
        let mut origins = self.origins.borrow_mut();
        let tick = self.navigation.work().tick;
        if origins.tick != Some(tick) {
            origins.tick = Some(tick);
            origins.by_crew.clear();
        }
        origins
            .by_crew
            .entry((swarm, kind, include_production))
            .or_insert_with(|| {
                let mut starts = self
                    .bots
                    .iter()
                    .filter_map(|(transform, bot_kind, member)| {
                        (*bot_kind == kind && member.0 == swarm)
                            .then_some(transform.translation.truncate())
                    })
                    .collect::<Vec<_>>();
                if include_production {
                    for (transform, owner, condition) in &self.facilities {
                        let owner = owner
                            .and_then(|owner| self.swarms.get(owner.0).ok())
                            .copied()
                            .unwrap_or(SwarmId::PLAYER);
                        if owner == swarm && condition.is_none_or(|condition| condition.health > 0)
                        {
                            starts.extend(
                                InteractionRegion::structure(transform)
                                    .candidates(transform.translation.truncate())
                                    .into_iter()
                                    .filter(|point| self.navigation.point_clear(*point)),
                            );
                        }
                    }
                }
                starts.into()
            })
            .clone()
    }

    /// Potential production exits also count: demand can exist before its crew does.
    pub(crate) fn crew(
        &self,
        swarm: SwarmId,
        kind: NanobotType,
        destination: InteractionRegion,
        include_production: bool,
    ) -> WorkReachability {
        let starts = self.starts(swarm, kind, include_production);
        if starts.is_empty() {
            return WorkReachability::Pending;
        }
        let mut pending = false;
        for &start in starts.iter() {
            match self.reachability_from(start, destination) {
                WorkReachability::Reachable => return WorkReachability::Reachable,
                WorkReachability::Pending => pending = true,
                WorkReachability::Unreachable => {}
            }
        }
        if pending {
            WorkReachability::Pending
        } else {
            WorkReachability::Unreachable
        }
    }

    pub(crate) fn chain_from(
        &self,
        start: Vec2,
        source: InteractionRegion,
        destination: InteractionRegion,
    ) -> WorkReachability {
        match self
            .navigation
            .query_connectivity(start, RouteGoal::Interaction(source))
        {
            ConnectivityStatus::Pending => WorkReachability::Pending,
            ConnectivityStatus::Unreachable => WorkReachability::Unreachable,
            ConnectivityStatus::Connected { endpoint } => {
                self.reachability_from(endpoint, destination)
            }
        }
    }

    pub(crate) fn between(
        &self,
        swarm: SwarmId,
        source: InteractionRegion,
        destination: InteractionRegion,
    ) -> WorkReachability {
        self.haul_chain(swarm, source, destination, false)
    }

    fn haul_chain(
        &self,
        swarm: SwarmId,
        source: InteractionRegion,
        destination: InteractionRegion,
        include_production: bool,
    ) -> WorkReachability {
        let starts = self.starts(swarm, NanobotType::Hauler, include_production);
        if starts.is_empty() {
            return WorkReachability::Pending;
        }
        let mut pending = false;
        for &start in starts.iter() {
            match self.chain_from(start, source, destination) {
                WorkReachability::Reachable => return WorkReachability::Reachable,
                WorkReachability::Pending => pending = true,
                WorkReachability::Unreachable => {}
            }
        }
        if pending {
            WorkReachability::Pending
        } else {
            WorkReachability::Unreachable
        }
    }

    pub(crate) fn opportunity(
        &self,
        swarm: SwarmId,
        target: OpportunityTarget,
    ) -> WorkReachability {
        let (entity, kind) = match target {
            OpportunityTarget::Gather { deposit, .. } => (deposit, NanobotType::Worker),
            OpportunityTarget::PlannedBuild { structure, .. }
            | OpportunityTarget::Maintenance { structure } => (structure, NanobotType::Worker),
            OpportunityTarget::Haul { source, .. } => (source, NanobotType::Hauler),
        };
        let Some(region) = self.region(entity) else {
            return WorkReachability::Unreachable;
        };
        if let OpportunityTarget::Haul { sink, .. } = target {
            let Some(destination) = self.region(sink) else {
                return WorkReachability::Unreachable;
            };
            return self.haul_chain(swarm, region, destination, true);
        }
        self.crew(swarm, kind, region, true)
    }
}
