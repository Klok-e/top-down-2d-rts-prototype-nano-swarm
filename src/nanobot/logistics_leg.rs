use crate::resources::{ResourceKind, StockpileRole};

use super::{Cargo, SwarmId};

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
    /// False for a Sink that no longer repays its Maintenance: it takes no
    /// new material, though returning cargo may still land there.
    pub accepts_fill: bool,
}

impl Destination {
    /// Minerals a pickup could move here. Sinks are filled only from Sources;
    /// terminals draw from Sources and Sinks alike.
    pub fn projected_amount(
        self,
        source_owner: Option<SwarmId>,
        source_kind: ResourceKind,
        source_role: StockpileRole,
        source_amount: u32,
    ) -> u32 {
        let valid_tier = match self.role {
            DestinationRole::Stockpile(StockpileRole::Source) => false,
            DestinationRole::Stockpile(StockpileRole::Sink) => {
                source_role == StockpileRole::Source && self.accepts_fill
            }
            DestinationRole::ProductionFacility | DestinationRole::Charger => true,
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
        returning: bool,
        incoming: u32,
    ) -> bool {
        let valid_tier = match self.role {
            DestinationRole::Stockpile(role) => {
                returning || (role == StockpileRole::Sink && self.accepts_fill)
            }
            DestinationRole::ProductionFacility | DestinationRole::Charger => true,
        };
        valid_tier
            && cargo.amount > 0
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
        hauler_capacity: u32,
        production_cost: u32,
    ) -> PickupQuote {
        let unclaimed_space =
            unclaimed_space(self.capacity.saturating_sub(self.buffered), incoming);
        let mut amount = hauler_capacity
            .min(source_amount.saturating_sub(source_claims))
            .min(unclaimed_space);
        let base_urgency = match self.role {
            // Startable only when this load is what lets the next cycle
            // start. Loads queued behind an already funded cycle cannot raise
            // output sooner, so they rank as ordinary refill and a starving
            // facility elsewhere outranks them.
            DestinationRole::ProductionFacility
                if self.buffered.saturating_add(incoming) < production_cost
                    && self
                        .buffered
                        .saturating_add(incoming)
                        .saturating_add(amount)
                        >= production_cost =>
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

#[cfg(test)]
mod tests {
    use super::*;

    fn facility(buffered: u32) -> PickupBuffer {
        PickupBuffer {
            role: DestinationRole::ProductionFacility,
            buffered,
            capacity: 25,
        }
    }

    fn urgency(buffer: &PickupBuffer, incoming: u32) -> u8 {
        buffer.quote(100, 0, incoming, None, 10, 5).base_urgency
    }

    #[test]
    fn only_the_load_that_funds_the_next_cycle_is_startable() {
        assert_eq!(
            urgency(&facility(0), 0),
            3,
            "an empty hopper starts a cycle"
        );
        assert_eq!(urgency(&facility(3), 0), 3, "topping up to one cycle");
        assert_eq!(
            urgency(&facility(5), 0),
            4,
            "a funded hopper only queues the load"
        );
        assert_eq!(
            urgency(&facility(0), 10),
            4,
            "a load already on its way funds the cycle"
        );
    }
}
