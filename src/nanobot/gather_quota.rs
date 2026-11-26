//! Gather Quota: the Workers a swarm assigns to gathering so its income covers
//! its Spending Capacity and refills its Reserve Target.

use std::collections::{BTreeMap, VecDeque};

use bevy::prelude::*;

use super::work_access::{WorkAccess, WorkReachability};
use crate::nanobot::{
    ActionableProjection, Cargo, DirectMovementComponent, ExtractProgress, GatherAssignment,
    Nanobot, NanobotType, OpportunityTarget, OwnerSwarm, PlannedStructure, ProductionFacility,
    ReturningToStockpile, SupportCondition, Swarm, SwarmId, SwarmMember, TerritorySnapshot,
    gather_worker_slots,
};
use crate::resources::{ResourceDeposit, ResourceKind, ResourceLedger, Stockpile, StockpileRole};
use crate::tuning::Tuning;

const TICKS_PER_SECOND: u64 = crate::SIMULATION_HZ as u64;
/// Trailing window for the measured gather rate. Long enough to span several
/// Worker trips, so one delivery does not swing the quota.
const RATE_WINDOW_TICKS: u64 = 20 * TICKS_PER_SECOND;

/// One swarm's mineral economy as the Gather Quota sees it.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct SwarmEconomy {
    /// Minerals per second the swarm would spend: producing facilities plus
    /// Defender charge upkeep.
    pub spending_capacity: f32,
    pub reserve_target: u32,
    /// Minerals in the swarm's operational Source and Sink Stockpiles, each
    /// counted by how soon it could reach a consumer: see
    /// [`reserve_weight`].
    pub reserve_stock: u32,
    /// Minerals one gathering Worker brings in per second while not waiting
    /// for Source Stockpile room.
    pub gather_rate: f32,
    /// Workers gathering now: travelling to, working at, or returning from a deposit.
    pub gathering_workers: u32,
    pub gather_quota: u32,
}

/// Per-swarm Gather Quotas, recomputed every fixed tick.
#[derive(Debug, Default, Resource)]
pub struct GatherQuotas {
    tick: u64,
    swarms: BTreeMap<SwarmId, SwarmEconomy>,
    rates: BTreeMap<SwarmId, GatherRateHistory>,
}

impl GatherQuotas {
    pub fn economy(&self, swarm: SwarmId) -> Option<SwarmEconomy> {
        self.swarms.get(&swarm).copied()
    }

    pub fn iter(&self) -> impl Iterator<Item = (SwarmId, SwarmEconomy)> + '_ {
        self.swarms
            .iter()
            .map(|(swarm, economy)| (*swarm, *economy))
    }
}

/// Minerals extracted per gatherer-second over a trailing window. Dividing by
/// gatherer-time rather than the current count keeps the rate steady while
/// gatherers join or leave.
#[derive(Debug, Default)]
struct GatherRateHistory {
    gatherer_ticks: u64,
    samples: VecDeque<RateSample>,
}

#[derive(Debug, Clone, Copy)]
struct RateSample {
    tick: u64,
    extracted: u64,
    gatherer_ticks: u64,
}

impl GatherRateHistory {
    fn record(&mut self, tick: u64, extracted: u64, gatherers: u32) -> Option<f32> {
        if self
            .samples
            .back()
            .is_some_and(|sample| extracted < sample.extracted)
        {
            *self = Self::default();
        }
        self.gatherer_ticks += u64::from(gatherers);
        if self
            .samples
            .back()
            .is_none_or(|sample| tick.saturating_sub(sample.tick) >= TICKS_PER_SECOND)
        {
            self.samples.push_back(RateSample {
                tick,
                extracted,
                gatherer_ticks: self.gatherer_ticks,
            });
        }
        while self.samples.len() > 1
            && tick.saturating_sub(self.samples[1].tick) >= RATE_WINDOW_TICKS
        {
            self.samples.pop_front();
        }
        let start = *self.samples.front()?;
        let gatherer_ticks = self.gatherer_ticks - start.gatherer_ticks;
        let minerals = extracted - start.extracted;
        (tick.saturating_sub(start.tick) >= RATE_WINDOW_TICKS && gatherer_ticks > 0 && minerals > 0)
            .then_some(minerals as f32 * TICKS_PER_SECOND as f32 / gatherer_ticks.max(1) as f32)
    }
}

/// Minerals per second a swarm would spend with `producing_facilities` running
/// continuously and `defenders` kept charged.
pub fn spending_capacity(producing_facilities: u32, defenders: u32, tuning: &Tuning) -> f32 {
    let hz = crate::SIMULATION_HZ as f32;
    let production = &tuning.production;
    let per_facility = production.cost_per_bot as f32 * hz / production.ticks_per_bot.max(1) as f32;
    let charge = &tuning.charge;
    let per_defender = if charge.per_pulse > 0.0 {
        charge.drain_per_tick * hz / charge.per_pulse * charge.material_per_pulse as f32
    } else {
        0.0
    };
    producing_facilities as f32 * per_facility + defenders as f32 * per_defender
}

/// Share of a stockpile's minerals that counts toward Reserve stock: all of it
/// when a delivery from it to the nearest consumer fits within
/// `reserve_seconds`, otherwise `reserve_seconds` over that delivery's time.
/// Stock with no consumer to reach counts for nothing.
pub fn reserve_weight(tuning: &Tuning, consumer_distance: Option<f32>) -> f32 {
    let Some(distance) = consumer_distance else {
        return 0.0;
    };
    let lead_seconds =
        crate::nanobot::estimated_lead_ticks(tuning, distance) / crate::SIMULATION_HZ as f32;
    let horizon = tuning.production.reserve_seconds.max(1.0);
    if lead_seconds <= horizon {
        1.0
    } else {
        horizon / lead_seconds
    }
}

pub fn reserve_target(spending_capacity: f32, reserve_seconds: f32) -> u32 {
    (spending_capacity * reserve_seconds.max(0.0)).round() as u32
}

/// Workers needed for income to match spending and close the reserve gap over
/// `reserve_seconds`, kept between `floor` and `ceiling`.
pub fn gather_quota(
    spending_capacity: f32,
    stock: u32,
    reserve_seconds: f32,
    gather_rate: f32,
    floor: u32,
    ceiling: u32,
) -> u32 {
    let target = reserve_target(spending_capacity, reserve_seconds);
    let gap = (target as f32 - stock as f32) / reserve_seconds.max(1.0);
    let needed = spending_capacity + gap;
    let wanted = if needed <= 0.0 {
        0
    } else if gather_rate > 0.0 {
        (needed / gather_rate).ceil().min(u32::MAX as f32) as u32
    } else {
        u32::MAX
    };
    wanted.clamp(floor, ceiling.max(floor))
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn gather_quota_system(
    tuning: Res<Tuning>,
    projection: Res<ActionableProjection>,
    territory: Res<TerritorySnapshot>,
    access: WorkAccess,
    ledger: Option<Res<ResourceLedger>>,
    mut quotas: ResMut<GatherQuotas>,
    deposits: Query<&ResourceDeposit>,
    swarm_ids: Query<&SwarmId, With<Swarm>>,
    bots: Query<
        (
            &NanobotType,
            &SwarmMember,
            Has<GatherAssignment>,
            Has<ReturningToStockpile>,
            Has<Cargo>,
            Has<ExtractProgress>,
            Has<DirectMovementComponent>,
        ),
        With<Nanobot>,
    >,
    structures: Query<
        (
            &OwnerSwarm,
            &Transform,
            Option<&SupportCondition>,
            Option<&ProductionFacility>,
            Option<&Stockpile>,
            Option<&StockpileRole>,
            Has<crate::nanobot::Charger>,
        ),
        (
            Without<PlannedStructure>,
            Or<(
                With<ProductionFacility>,
                With<Stockpile>,
                With<crate::nanobot::Charger>,
            )>,
        ),
    >,
) {
    quotas.tick += 1;
    let tick = quotas.tick;
    let live_swarms = territory.swarms().collect::<Vec<_>>();

    // Gather-painted deposits each swarm can reach, with their ring slots.
    let mut gather_deposits = BTreeMap::<SwarmId, BTreeMap<Entity, u32>>::new();
    for (_, opportunities) in projection.iter_regions() {
        for opportunity in opportunities {
            let OpportunityTarget::Gather { deposit, .. } = opportunity.target else {
                continue;
            };
            let Ok(state) = deposits.get(deposit) else {
                continue;
            };
            let owners = opportunity
                .owner
                .map_or_else(|| live_swarms.clone(), |owner| vec![owner]);
            for swarm in owners {
                if access.opportunity(swarm, opportunity.target) == WorkReachability::Unreachable {
                    continue;
                }
                gather_deposits
                    .entry(swarm)
                    .or_default()
                    .insert(deposit, gather_worker_slots(state.radius));
            }
        }
    }

    let mut gathering = BTreeMap::<SwarmId, u32>::new();
    // Gatherers moving, extracting, or returning. One waiting at its deposit
    // for Source Stockpile room holds its quota slot but brings nothing in,
    // so counting it would read a full Source as slow Workers and raise the
    // quota exactly when more gatherers cannot help.
    let mut productive = BTreeMap::<SwarmId, u32>::new();
    let mut defenders = BTreeMap::<SwarmId, u32>::new();
    for (kind, member, gather, returning, loaded, extracting, moving) in &bots {
        match kind {
            // A Worker's cargo only ever comes from a deposit; one holding it
            // between destinations is still on its gather trip.
            NanobotType::Worker if gather || returning || loaded => {
                *gathering.entry(member.0).or_default() += 1;
                if returning || extracting || moving {
                    *productive.entry(member.0).or_default() += 1;
                }
            }
            NanobotType::Defender => *defenders.entry(member.0).or_default() += 1,
            _ => {}
        }
    }

    let operational = structures
        .iter()
        .filter(|(_, _, condition, ..)| {
            condition.is_none_or(|condition| condition.is_operational())
        })
        .filter_map(
            |(owner, transform, _, facility, stockpile, role, charger)| {
                let swarm = swarm_ids.get(owner.0).copied().ok()?;
                Some((
                    swarm,
                    transform.translation.truncate(),
                    facility,
                    stockpile,
                    role,
                    charger,
                ))
            },
        )
        .collect::<Vec<_>>();
    // Where each swarm spends minerals: its facilities and Chargers.
    let mut consumers = BTreeMap::<SwarmId, Vec<Vec2>>::new();
    let mut producing = BTreeMap::<SwarmId, u32>::new();
    for &(swarm, position, facility, _, _, charger) in &operational {
        if facility.is_some() || charger {
            consumers.entry(swarm).or_default().push(position);
        }
        if facility
            .is_some_and(|facility| facility.current_target.is_some() || facility.demand_waiting)
        {
            *producing.entry(swarm).or_default() += 1;
        }
    }
    let mut weighted_stock = BTreeMap::<SwarmId, f32>::new();
    let mut source_room = BTreeMap::<SwarmId, u32>::new();
    for &(swarm, position, _, stockpile, role, _) in &operational {
        let Some(stockpile) =
            stockpile.filter(|stockpile| stockpile.kind == ResourceKind::Minerals)
        else {
            continue;
        };
        let consumer_distance = consumers.get(&swarm).and_then(|consumers| {
            consumers
                .iter()
                .map(|consumer| consumer.distance(position))
                .min_by(f32::total_cmp)
        });
        *weighted_stock.entry(swarm).or_default() +=
            stockpile.amount as f32 * reserve_weight(&tuning, consumer_distance);
        if role != Some(&StockpileRole::Sink) {
            let room = source_room.entry(swarm).or_default();
            *room = room.saturating_add(stockpile.free_space());
        }
    }

    let reserve_seconds = tuning.production.reserve_seconds;
    let GatherQuotas { swarms, rates, .. } = &mut *quotas;
    swarms.clear();
    rates.retain(|swarm, _| live_swarms.contains(swarm));
    for swarm in live_swarms {
        let gathering_workers = gathering.get(&swarm).copied().unwrap_or_default();
        let extracted = ledger
            .as_ref()
            .map_or(0, |ledger| ledger.extracted_minerals_for(swarm));
        let gather_rate = rates
            .entry(swarm)
            .or_default()
            .record(
                tick,
                extracted,
                productive.get(&swarm).copied().unwrap_or_default(),
            )
            .unwrap_or(tuning.production.gather_rate_fallback);
        let spending = spending_capacity(
            producing.get(&swarm).copied().unwrap_or_default(),
            defenders.get(&swarm).copied().unwrap_or_default(),
            &tuning,
        );
        let reserve_stock = weighted_stock
            .get(&swarm)
            .copied()
            .unwrap_or_default()
            .round() as u32;
        let deposits = gather_deposits.remove(&swarm).unwrap_or_default();
        let floor = deposits.len() as u32;
        // Gatherers beyond the full loads Source Stockpiles can still take
        // only split that room into smaller loads.
        let loads = source_room
            .get(&swarm)
            .copied()
            .unwrap_or_default()
            .div_ceil(tuning.logistics.worker_carry_capacity.max(1));
        let ceiling = deposits
            .values()
            .sum::<u32>()
            .min(floor.saturating_add(loads));
        let gather_quota = gather_quota(
            spending,
            reserve_stock,
            reserve_seconds,
            gather_rate,
            floor,
            ceiling,
        );
        swarms.insert(
            swarm,
            SwarmEconomy {
                spending_capacity: spending,
                reserve_target: reserve_target(spending, reserve_seconds),
                reserve_stock,
                gather_rate,
                gathering_workers,
                gather_quota,
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quota_covers_spending_and_closes_the_reserve_gap() {
        // 2/s spending, 10 s reserve: target 20, stock 0 -> needs 2 + 2 = 4/s.
        assert_eq!(gather_quota(2.0, 0, 10.0, 0.5, 1, 100), 8);
        // At the target only spending remains.
        assert_eq!(gather_quota(2.0, 20, 10.0, 0.5, 1, 100), 4);
    }

    #[test]
    fn quota_stays_between_one_per_deposit_and_the_ring_slots() {
        assert_eq!(gather_quota(0.0, 0, 10.0, 0.5, 3, 27), 3);
        assert_eq!(gather_quota(2.0, 500, 10.0, 0.5, 2, 18), 2);
        assert_eq!(gather_quota(100.0, 0, 10.0, 0.5, 1, 9), 9);
        assert_eq!(gather_quota(1.0, 0, 10.0, 0.5, 0, 0), 0);
    }

    #[test]
    fn spending_capacity_counts_production_and_defender_upkeep() {
        let tuning = Tuning::default();
        let hz = crate::SIMULATION_HZ as f32;
        let facility =
            tuning.production.cost_per_bot as f32 * hz / tuning.production.ticks_per_bot as f32;
        let defender = tuning.charge.drain_per_tick * hz / tuning.charge.per_pulse
            * tuning.charge.material_per_pulse as f32;
        let spending = spending_capacity(2, 3, &tuning);
        assert!((spending - (2.0 * facility + 3.0 * defender)).abs() < 1e-4);
    }

    #[test]
    fn rate_is_minerals_per_gatherer_second_after_a_full_window() {
        let mut history = GatherRateHistory::default();
        let mut extracted = 0;
        let mut rate = None;
        // Two gatherers bring in one mineral per second between them.
        for tick in 1..=RATE_WINDOW_TICKS + 1 {
            if tick % TICKS_PER_SECOND == 0 {
                extracted += 1;
            }
            rate = history.record(tick, extracted, 2);
            if tick < RATE_WINDOW_TICKS {
                assert_eq!(rate, None, "no rate before a full window");
            }
        }
        let rate = rate.expect("a full window yields a rate");
        assert!((rate - 0.5).abs() < 0.05, "rate {rate}");
    }
}
