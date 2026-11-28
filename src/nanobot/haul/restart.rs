use std::{cmp::Reverse, collections::BinaryHeap};

use crate::tuning::LogisticsTuning;
use crate::{
    nanobot::{
        Cargo, DirectMovementComponent, InteractionRegion, LogisticsReservation,
        MaterialWorkProgress, RemainingTravel,
    },
    navigation::{Navigation, PhysicalRouteStatus},
};
use bevy::prelude::*;

#[derive(Clone, Copy)]
pub(in crate::nanobot) struct DeliveryEstimate {
    first_unload: u64,
    pub amount: u32,
}

pub(in crate::nanobot) enum InboundDelivery {
    Estimated(DeliveryEstimate),
    Pending,
    Unavailable,
}

fn travel_ticks(distance: f32, speed: f32) -> u64 {
    (distance / speed).ceil() as u64
}

fn transfer_ticks(amount: u32, logistics: &LogisticsTuning) -> u64 {
    u64::from(amount.div_ceil(logistics.hauler_transfer_per_pulse))
        * u64::from(logistics.material_work_ticks)
}

impl DeliveryEstimate {
    pub fn new(
        arrival: u64,
        amount: u32,
        unload_progress: u32,
        logistics: &LogisticsTuning,
    ) -> Self {
        Self {
            first_unload: arrival
                + u64::from(
                    logistics
                        .material_work_ticks
                        .saturating_sub(unload_progress)
                        .max(1),
                ),
            amount,
        }
    }

    fn unloaded_by(self, tick: u64, interval: u64, pulse: u32) -> u32 {
        tick.checked_sub(self.first_unload).map_or(0, |elapsed| {
            (1 + elapsed / interval)
                .saturating_mul(u64::from(pulse))
                .min(u64::from(self.amount)) as u32
        })
    }
}

fn first_tick(limit: u64, reached: impl Fn(u64) -> bool) -> Option<u64> {
    if !reached(limit) {
        return None;
    }
    let (mut lower, mut upper) = (0, limit);
    while lower < upper {
        let mid = lower + (upper - lower) / 2;
        if reached(mid) {
            upper = mid;
        } else {
            lower = mid + 1;
        }
    }
    Some(lower)
}

pub(in crate::nanobot) struct RestartForecast {
    cycle_cost: u32,
    cycle_ticks: u64,
    remaining_starts: u32,
    next_start: u64,
    buffered: u32,
    capacity: u32,
    deliveries: Vec<DeliveryEstimate>,
    pending_delivery: bool,
    baseline: Option<u64>,
}

impl RestartForecast {
    pub fn new(
        facility: &super::ProductionFacility,
        remaining_starts: u32,
        tuning: &crate::tuning::Tuning,
    ) -> Self {
        let cycle_ticks = tuning.production.ticks_per_bot;
        let next_start = facility
            .current_target
            .map_or(0, |_| cycle_ticks.saturating_sub(facility.progress));
        Self {
            cycle_cost: tuning.production.cost_per_bot,
            cycle_ticks: u64::from(cycle_ticks),
            remaining_starts,
            next_start: u64::from(next_start),
            buffered: facility.input_amount,
            capacity: facility.input_capacity,
            deliveries: Vec::new(),
            pending_delivery: false,
            baseline: None,
        }
    }

    pub fn add_delivery(&mut self, delivery: InboundDelivery) {
        match delivery {
            InboundDelivery::Estimated(delivery) => self.deliveries.push(delivery),
            InboundDelivery::Pending => self.pending_delivery = true,
            InboundDelivery::Unavailable => (),
        }
    }

    pub fn has_pending_delivery(&self) -> bool {
        self.pending_delivery
    }

    pub fn refresh_baseline(&mut self, logistics: &LogisticsTuning) {
        self.baseline = self.funding_tick(None, logistics);
    }

    pub fn best_pickup(
        &self,
        usual_amount: u32,
        travel_to_pickup: f32,
        travel_to_facility: f32,
        logistics: &LogisticsTuning,
    ) -> (DeliveryEstimate, Option<u64>) {
        let travel = travel_ticks(travel_to_pickup, logistics.bot_speed)
            + travel_ticks(travel_to_facility, logistics.bot_speed)
            + 2;
        let pickup = |amount| {
            DeliveryEstimate::new(
                travel + transfer_ticks(amount, logistics),
                amount,
                0,
                logistics,
            )
        };
        let usual = pickup(usual_amount);
        if self.has_pending_delivery() || self.funded_by_due_tick() {
            return (usual, None);
        }
        let mut best = (usual, self.funding_advancement(usual, logistics));
        let pulse = logistics.hauler_transfer_per_pulse;
        let interval = u64::from(logistics.material_work_ticks);
        let needed = self.shortfall();
        let short_limit = needed.min((usual_amount.div_ceil(pulse) - 1) * pulse);
        if short_limit == 0 {
            return best;
        }

        let incoming = |tick| {
            self.deliveries.iter().fold(0u32, |sum, delivery| {
                sum.saturating_add(delivery.unloaded_by(tick, interval, pulse))
            })
        };
        let latest_unload = self
            .deliveries
            .iter()
            .map(|delivery| {
                delivery.first_unload + transfer_ticks(delivery.amount, logistics) - interval
            })
            .fold(
                travel + 2 * transfer_ticks(short_limit, logistics),
                u64::max,
            );
        let deadline = best
            .1
            .into_iter()
            .chain(self.baseline)
            .fold(latest_unload, |limit, tick| {
                limit.min(tick.saturating_sub(1))
            });
        let Some(earliest) = first_tick(deadline, |tick| {
            // Each full pulse delivered by this trip takes one load pulse and
            // one unload pulse. Balance those phases to maximize its delivery.
            let transfer_budget = tick.saturating_sub(travel) / interval;
            let delivered =
                ((transfer_budget / 2) * u64::from(pulse)).min(u64::from(short_limit)) as u32;
            incoming(tick).saturating_add(delivered) >= needed
        }) else {
            return best;
        };
        let amount = needed.saturating_sub(incoming(earliest));
        if amount > 0 {
            let candidate = pickup(amount);
            if let Some(tick) = self.funding_advancement(candidate, logistics)
                && best.1.is_none_or(|best_tick| tick < best_tick)
            {
                best = (candidate, Some(tick));
                if tick == earliest {
                    return best;
                }
            }
        }

        // Unloads can lose pulses to a full hopper before the first already
        // funded cycle starts. Search the remaining load pulse counts using the
        // physical forecast whenever the unconstrained deadline cannot be met.
        let mut ranges = vec![(1, short_limit.div_ceil(pulse))];
        while let Some((first, last)) = ranges.pop() {
            let first_unload = pickup((first - 1) * pulse + 1).first_unload;
            let amount = short_limit.min(last.saturating_mul(pulse));
            // Give the upper bound enough extra cargo to cover every pulse
            // before the latest possible arrival in the range.
            let upper_bound = DeliveryEstimate {
                first_unload,
                amount: amount.saturating_add((last - first).saturating_mul(pulse)),
            };
            let Some(tick) = self.funding_advancement(upper_bound, logistics) else {
                continue;
            };
            if best.1.is_some_and(|best_tick| tick >= best_tick) {
                continue;
            }
            if first == last {
                best = (upper_bound, Some(tick));
                if tick == earliest {
                    break;
                }
            } else {
                let mid = first + (last - first) / 2;
                ranges.extend([(mid + 1, last), (first, mid)]);
            }
        }
        best
    }

    fn shortfall(&self) -> u32 {
        (self.remaining_starts * self.cycle_cost).saturating_sub(self.buffered)
    }

    pub fn funded_by_due_tick(&self) -> bool {
        let due = self.next_start + u64::from(self.remaining_starts - 1) * self.cycle_ticks;
        self.baseline.is_some_and(|funded| funded <= due)
    }

    fn funding_advancement(
        &self,
        candidate: DeliveryEstimate,
        logistics: &LogisticsTuning,
    ) -> Option<u64> {
        let funded = self.funding_tick(Some(candidate), logistics)?;
        self.baseline
            .is_none_or(|baseline| funded < baseline)
            .then_some(funded)
    }

    fn funding_tick(
        &self,
        candidate: Option<DeliveryEstimate>,
        logistics: &LogisticsTuning,
    ) -> Option<u64> {
        let needed = self.shortfall();
        let deliveries = self.deliveries.iter().copied().chain(candidate);
        if needed == 0 {
            return Some(0);
        }
        if deliveries
            .clone()
            .fold(0u32, |sum, trip| sum.saturating_add(trip.amount))
            < needed
        {
            return None;
        }
        let mut amounts = Vec::new();
        let mut pulses = BinaryHeap::new();
        for (index, delivery) in deliveries.enumerate() {
            amounts.push(delivery.amount);
            pulses.push(Reverse((delivery.first_unload, index)));
        }

        let mut starts = self.remaining_starts;
        let mut next_start = self.next_start;
        let mut buffered = self.buffered;
        let interval = u64::from(logistics.material_work_ticks);
        while let Some(Reverse((tick, index))) = pulses.pop() {
            // Hopper stock already covers the starts before the restart cycle.
            // Those starts free physical space for incoming unload pulses.
            if tick >= next_start {
                let elapsed_starts =
                    ((tick - next_start) / self.cycle_ticks + 1).min(u64::from(starts - 1)) as u32;
                buffered -= elapsed_starts * self.cycle_cost;
                next_start += u64::from(elapsed_starts) * self.cycle_ticks;
                starts -= elapsed_starts;
            }
            let amount = amounts[index]
                .min(logistics.hauler_transfer_per_pulse)
                .min(self.capacity.saturating_sub(buffered));
            buffered += amount;
            amounts[index] -= amount;
            if buffered >= starts * self.cycle_cost {
                return Some(tick);
            }

            if amounts[index] > 0 {
                let next_pulse = if buffered < self.capacity {
                    tick + interval
                } else if starts > 1 {
                    tick + (next_start - tick).div_ceil(interval) * interval
                } else {
                    return None;
                };
                pulses.push(Reverse((next_pulse, index)));
            }
        }
        None
    }
}

pub(in crate::nanobot) struct InboundHauler<'a> {
    pub position: Vec2,
    pub reservation: &'a LogisticsReservation,
    pub cargo: Option<&'a Cargo>,
    pub loading: bool,
    pub work: Option<&'a MaterialWorkProgress>,
    pub movement: Option<&'a DirectMovementComponent>,
    pub remaining_travel: Option<&'a RemainingTravel>,
}

impl InboundHauler<'_> {
    pub fn delivery(
        &self,
        pickup: Option<(InteractionRegion, u32)>,
        facility: InteractionRegion,
        navigation: &Navigation,
        logistics: &LogisticsTuning,
    ) -> InboundDelivery {
        let carried = self.cargo.map_or(0, |cargo| cargo.amount);
        let work_progress = self.work.map_or(0, |work| work.ticks_elapsed);
        let loading = self.loading || self.cargo.is_none();
        let pickup = pickup.filter(|(_, available)| {
            loading
                && *available > 0
                && self.reservation.source_remaining > 0
                && carried < logistics.hauler_carry_capacity
        });
        let pickup_travel = if let Some((region, available)) = pickup {
            match self.travel(self.position, region, navigation, true) {
                PhysicalRouteStatus::Found { endpoint, distance } => {
                    Some((region, endpoint, distance, available))
                }
                PhysicalRouteStatus::Pending => return InboundDelivery::Pending,
                PhysicalRouteStatus::Unreachable => None,
            }
        } else {
            None
        };
        let (departure, load_ticks, amount) =
            if let Some((region, position, distance, available)) = pickup_travel {
                let load = self
                    .reservation
                    .source_remaining
                    .min(available)
                    .min(logistics.hauler_carry_capacity.saturating_sub(carried));
                let progress = if load > 0 && self.loading && region.contains(self.position) {
                    u64::from(work_progress)
                } else {
                    0
                };
                let ticks = travel_ticks(distance, logistics.bot_speed)
                    + transfer_ticks(load, logistics).saturating_sub(progress)
                    + 2;
                (position, ticks, carried + load)
            } else {
                (self.position, if self.loading { 2 } else { 0 }, carried)
            };
        let amount = amount.min(self.reservation.destination_remaining);
        if amount == 0 {
            return InboundDelivery::Unavailable;
        }
        let distance = match self.travel(departure, facility, navigation, !loading) {
            PhysicalRouteStatus::Found { distance, .. } => distance,
            PhysicalRouteStatus::Pending => return InboundDelivery::Pending,
            PhysicalRouteStatus::Unreachable => return InboundDelivery::Unavailable,
        };
        let already_unloading =
            !loading && self.movement.is_none() && facility.contains(self.position);
        let arrival = load_ticks
            + travel_ticks(distance, logistics.bot_speed)
            + u64::from(!loading && (!already_unloading || self.work.is_none()));
        InboundDelivery::Estimated(DeliveryEstimate::new(
            arrival,
            amount,
            if already_unloading { work_progress } else { 0 },
            logistics,
        ))
    }

    fn travel(
        &self,
        from: Vec2,
        goal: InteractionRegion,
        navigation: &Navigation,
        current_leg: bool,
    ) -> PhysicalRouteStatus {
        if goal.contains(from) {
            return PhysicalRouteStatus::Found {
                endpoint: from,
                distance: 0.0,
            };
        }
        if current_leg
            && let (Some(movement), Some(remaining)) = (self.movement, self.remaining_travel)
            && movement.interaction == Some(goal)
            && goal.contains(movement.xy)
            && navigation.point_clear(movement.xy)
        {
            return PhysicalRouteStatus::Found {
                endpoint: movement.xy,
                distance: remaining.0.max(from.distance(movement.xy)),
            };
        }
        navigation.query_physical_route(from, goal)
    }
}
