use std::{cmp::Reverse, collections::BinaryHeap};

use crate::tuning::LogisticsTuning;
use crate::{
    nanobot::{
        Cargo, DirectMovementComponent, InteractionRegion, LogisticsReservation,
        MaterialWorkProgress, RemainingTravel,
    },
    navigation::{ConnectivityStatus, Navigation, RouteGoal},
};
use bevy::prelude::*;

#[derive(Clone, Copy)]
pub(in crate::nanobot) struct DeliveryEstimate {
    first_unload: u64,
    amount: u32,
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

    pub fn pickup(
        amount: u32,
        travel_to_pickup: f32,
        travel_to_facility: f32,
        logistics: &LogisticsTuning,
    ) -> Self {
        let arrival = travel_ticks(travel_to_pickup, logistics.bot_speed)
            + transfer_ticks(amount, logistics)
            + travel_ticks(travel_to_facility, logistics.bot_speed)
            + 2;
        Self::new(arrival, amount, 0, logistics)
    }
}

pub(in crate::nanobot) struct RestartForecast {
    cycle_cost: u32,
    cycle_ticks: u64,
    remaining_starts: u32,
    next_start: u64,
    buffered: u32,
    capacity: u32,
    deliveries: Vec<DeliveryEstimate>,
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
            baseline: None,
        }
    }

    pub fn add_delivery(&mut self, delivery: DeliveryEstimate) {
        self.deliveries.push(delivery);
    }

    pub fn refresh_baseline(&mut self, logistics: &LogisticsTuning) {
        self.baseline = self.funding_tick(None, logistics);
    }

    pub fn funding_advancement(
        &self,
        candidate: DeliveryEstimate,
        logistics: &LogisticsTuning,
    ) -> Option<u64> {
        let due = self.next_start + u64::from(self.remaining_starts - 1) * self.cycle_ticks;
        if self.baseline.is_some_and(|funded| funded <= due) {
            return None;
        }
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
        let needed = (self.remaining_starts * self.cycle_cost).saturating_sub(self.buffered);
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
    ) -> Option<DeliveryEstimate> {
        let carried = self.cargo.map_or(0, |cargo| cargo.amount);
        let work_progress = self.work.map_or(0, |work| work.ticks_elapsed);
        let loading = self.loading || self.cargo.is_none();
        let route_to_pickup = pickup
            .filter(|(_, available)| {
                loading && *available > 0 && self.reservation.source_remaining > 0
            })
            .and_then(|(region, amount)| {
                let (endpoint, distance) = self.travel(self.position, region, navigation, true)?;
                Some((region, endpoint, distance, amount))
            });

        let (departure, load_ticks, amount) =
            if let Some((region, position, distance, available)) = route_to_pickup {
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
            return None;
        }
        let (_, distance) = self.travel(departure, facility, navigation, !loading)?;
        let already_unloading =
            !loading && self.movement.is_none() && facility.contains(self.position);
        let arrival = load_ticks
            + travel_ticks(distance, logistics.bot_speed)
            + u64::from(!loading && (!already_unloading || self.work.is_none()));
        Some(DeliveryEstimate::new(
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
    ) -> Option<(Vec2, f32)> {
        if goal.contains(from) {
            return Some((from, 0.0));
        }
        if current_leg
            && let (Some(movement), Some(remaining)) = (self.movement, self.remaining_travel)
            && movement.interaction == Some(goal)
            && goal.contains(movement.xy)
        {
            return Some((movement.xy, remaining.0.max(from.distance(movement.xy))));
        }
        let endpoint = match navigation.query_connectivity(from, RouteGoal::Interaction(goal)) {
            ConnectivityStatus::Connected { endpoint } => endpoint,
            ConnectivityStatus::Pending => goal.approach(from),
            ConnectivityStatus::Unreachable => return None,
        };
        Some((endpoint, from.distance(endpoint)))
    }
}
