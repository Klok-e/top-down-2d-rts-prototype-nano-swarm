use crate::resources::{ResourceKind, StockpileRole};

use super::{Cargo, HAULER_CARRY_CAPACITY, PRODUCTION_COST_PER_BOT, SwarmId};

#[derive(Clone, Copy)]
pub(super) enum DestinationRole {
    Stockpile(StockpileRole),
    ProductionFacility,
    Charger,
}

#[derive(Clone, Copy)]
pub(super) struct Destination {
    pub owner: Option<SwarmId>,
    pub kind: ResourceKind,
    pub role: DestinationRole,
    pub free_space: u32,
}

impl Destination {
    pub fn projected_amount(
        self,
        source_owner: Option<SwarmId>,
        source_kind: ResourceKind,
        source_role: StockpileRole,
        source_amount: u32,
    ) -> u32 {
        let valid_tier = match self.role {
            DestinationRole::Stockpile(StockpileRole::Source) => false,
            DestinationRole::Stockpile(StockpileRole::Sink) => source_role == StockpileRole::Source,
            DestinationRole::ProductionFacility | DestinationRole::Charger => {
                source_role == StockpileRole::Sink
            }
        };
        if valid_tier && source_owner.is_some_and(|owner| self.accepts(owner, source_kind)) {
            source_amount.min(self.free_space)
        } else {
            0
        }
    }

    pub fn can_receive_load(
        self,
        owner: SwarmId,
        cargo: Cargo,
        source_role: StockpileRole,
        returning: bool,
        incoming: u32,
    ) -> bool {
        let valid_tier = match self.role {
            DestinationRole::Stockpile(role) => returning || role == StockpileRole::Sink,
            DestinationRole::ProductionFacility | DestinationRole::Charger => {
                source_role == StockpileRole::Sink
            }
        };
        valid_tier
            && self.accepts(owner, cargo.kind)
            && unclaimed_space(self.free_space, incoming) >= cargo.amount
    }

    fn accepts(self, owner: SwarmId, kind: ResourceKind) -> bool {
        self.owner == Some(owner) && self.kind == kind
    }
}

pub(super) struct PickupBuffer {
    pub role: DestinationRole,
    pub buffered: u32,
    pub capacity: u32,
}

pub(super) struct PickupQuote {
    pub amount: u32,
    pub unclaimed_space: u32,
    pub base_urgency: u8,
}

impl PickupBuffer {
    pub fn quote(
        &self,
        source_amount: u32,
        source_claims: u32,
        incoming: u32,
        charger_demand: Option<(u8, u32)>,
    ) -> PickupQuote {
        let unclaimed_space =
            unclaimed_space(self.capacity.saturating_sub(self.buffered), incoming);
        let mut amount = HAULER_CARRY_CAPACITY
            .min(source_amount.saturating_sub(source_claims))
            .min(unclaimed_space);
        let base_urgency = match self.role {
            DestinationRole::ProductionFacility
                if self
                    .buffered
                    .saturating_add(incoming)
                    .saturating_add(amount)
                    >= PRODUCTION_COST_PER_BOT =>
            {
                3
            }
            DestinationRole::Charger => {
                let (urgency, total_need) = charger_demand.unwrap_or((4, 0));
                let uncovered = total_need
                    .saturating_sub(self.buffered)
                    .saturating_sub(incoming);
                if urgency < 4 && uncovered > 0 {
                    amount = amount.min(uncovered);
                    urgency
                } else {
                    4
                }
            }
            DestinationRole::Stockpile(_) => 5,
            DestinationRole::ProductionFacility => 4,
        };
        PickupQuote {
            amount,
            unclaimed_space,
            base_urgency,
        }
    }
}

fn unclaimed_space(free_space: u32, incoming: u32) -> u32 {
    free_space.saturating_sub(incoming)
}
