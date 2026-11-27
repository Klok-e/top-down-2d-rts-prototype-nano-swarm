//! Haul Quota: the Haulers a swarm needs to deliver the minerals it has to
//! its terminals, grown only while added Haulers raise delivery.

use std::collections::BTreeMap;

use bevy::prelude::*;

use crate::nanobot::{
    Cargo, GatherQuotas, HaulerAssignment, HaulerLoading, Nanobot, NanobotType, SwarmId,
    SwarmMember, TerritorySnapshot,
};
use crate::resources::ResourceLedger;
use crate::tuning::{LogisticsTuning, Tuning};

const TICKS_PER_SECOND: u64 = crate::SIMULATION_HZ as u64;
/// Haulers are counted once per this many ticks, each count standing for the
/// whole span; the quota only changes once per window.
const SAMPLE_TICKS: u64 = 6;
/// One measurement window. Long enough to span several Hauler trips, so one
/// delivery does not swing the quota.
pub const HAUL_WINDOW_TICKS: u64 = 20 * TICKS_PER_SECOND;

/// One swarm's hauling as the Haul Quota saw it at the end of the last window.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct HaulEconomy {
    /// Mean Spending Capacity over the same window as the delivery rate.
    pub spending_capacity: f32,
    /// Minerals per second there are to deliver: income plus stock above the
    /// Reserve Target spread over `reserve_seconds`, at most Spending Capacity.
    pub target_flow: f32,
    /// Minerals per second the swarm's terminals consumed.
    pub delivered_rate: f32,
    /// Minerals terminals consumed per busy-Hauler-second; 0 when nothing was
    /// delivered.
    pub haul_rate: f32,
    pub haulers: u32,
    /// Whether Hauler demand may exceed the current Hauler count.
    pub growing: bool,
    pub haul_quota: u32,
}

/// Per-swarm Haul Quotas, recomputed once per [`HAUL_WINDOW_TICKS`].
#[derive(Debug, Default, Resource)]
pub struct HaulQuotas {
    tick: u64,
    swarms: BTreeMap<SwarmId, HaulWindow>,
}

impl HaulQuotas {
    /// The last finished window's figures; `None` before the first one ends.
    pub fn economy(&self, swarm: SwarmId) -> Option<HaulEconomy> {
        self.swarms.get(&swarm).and_then(|window| window.economy)
    }

    pub fn iter(&self) -> impl Iterator<Item = (SwarmId, HaulEconomy)> + '_ {
        self.swarms
            .iter()
            .filter_map(|(swarm, window)| Some((*swarm, window.economy?)))
    }
}

#[derive(Debug)]
struct HaulWindow {
    started: u64,
    consumed_at_start: u64,
    extracted_at_start: u64,
    hauler_ticks: u64,
    busy_hauler_ticks: u64,
    spending_capacity_ticks: f32,
    guard: GrowthGuard,
    economy: Option<HaulEconomy>,
}

impl HaulWindow {
    fn new(tick: u64, consumed: u64, extracted: u64) -> Self {
        Self {
            started: tick,
            consumed_at_start: consumed,
            extracted_at_start: extracted,
            hauler_ticks: 0,
            busy_hauler_ticks: 0,
            spending_capacity_ticks: 0.0,
            guard: GrowthGuard::default(),
            economy: None,
        }
    }

    /// Start the next window where this one ended.
    fn reopen(&mut self, tick: u64, consumed: u64, extracted: u64) {
        self.started = tick;
        self.consumed_at_start = consumed;
        self.extracted_at_start = extracted;
        self.hauler_ticks = 0;
        self.busy_hauler_ticks = 0;
        self.spending_capacity_ticks = 0.0;
    }
}

/// Minerals per second there are to deliver: income plus the stock above the
/// Reserve Target spread over `reserve_seconds`, between 0 and Spending
/// Capacity. Stock below the target lowers it while gathering refills.
pub fn target_flow(
    income: f32,
    stock: u32,
    reserve_target: u32,
    reserve_seconds: f32,
    spending_capacity: f32,
) -> f32 {
    // `reserve_seconds` may be tuned to 0; the Gather Quota spreads its gap
    // over at least a second too.
    let backlog = (stock as f32 - reserve_target as f32) / reserve_seconds.max(1.0);
    (income + backlog).clamp(0.0, spending_capacity)
}

/// Haulers needed to deliver `target_flow` at `haul_rate` per Hauler; none
/// until a rate is measured.
pub fn throughput_quota(target_flow: f32, haul_rate: f32) -> u32 {
    if haul_rate > 0.0 && target_flow > 0.0 {
        (target_flow / haul_rate).ceil() as u32
    } else {
        0
    }
}

/// One finished window as the growth guard judges it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WindowSample {
    /// Haulers averaged over the window, so Haulers that arrived late count
    /// only for the time they could deliver.
    pub mean_haulers: f32,
    pub delivered_rate: f32,
    pub haul_rate: f32,
}

/// Lets Hauler demand exceed the current Hauler count only while the last
/// added Haulers raised delivery, so a bottleneck elsewhere cannot make the
/// quota run away.
#[derive(Debug, Clone, Copy, Default)]
pub struct GrowthGuard {
    baseline: Option<WindowSample>,
    /// When added Haulers last failed to raise delivery; growing while `None`.
    stopped_at: Option<u64>,
}

impl GrowthGuard {
    pub fn growing(&self) -> bool {
        self.stopped_at.is_none()
    }

    /// Judge the window that just ended and return the most Haulers demand may
    /// ask for next, given `haulers` now.
    pub fn update(
        &mut self,
        tick: u64,
        sample: WindowSample,
        haulers: u32,
        tuning: &LogisticsTuning,
    ) -> u32 {
        // Half a Hauler more on average is enough evidence to judge; less
        // keeps the current verdict.
        if let Some(before) = self.baseline
            && sample.mean_haulers >= before.mean_haulers + 0.5
        {
            // Added Haulers should deliver at the rate Haulers delivered
            // before they came, not the rate they have already diluted.
            let expected = before.haul_rate * (sample.mean_haulers - before.mean_haulers);
            let gained = sample.delivered_rate - before.delivered_rate;
            self.stopped_at = (gained < tuning.haul_growth_threshold * expected).then_some(tick);
        }
        if self.stopped_at.is_some_and(|stopped| {
            tick.saturating_sub(stopped) >= u64::from(tuning.haul_growth_retry_ticks)
        }) {
            self.stopped_at = None;
        }
        self.baseline = Some(sample);
        if self.growing() {
            let step = ((haulers as f32 * tuning.haul_growth_step).floor() as u32).max(1);
            haulers.saturating_add(step)
        } else {
            haulers
        }
    }
}

#[allow(clippy::type_complexity)]
pub fn haul_quota_system(
    tuning: Res<Tuning>,
    territory: Res<TerritorySnapshot>,
    gather: Res<GatherQuotas>,
    ledger: Option<Res<ResourceLedger>>,
    mut quotas: ResMut<HaulQuotas>,
    bots: Query<
        (
            &NanobotType,
            &SwarmMember,
            Has<HaulerAssignment>,
            Has<Cargo>,
            Has<HaulerLoading>,
        ),
        With<Nanobot>,
    >,
) {
    quotas.tick += 1;
    let tick = quotas.tick;
    if !tick.is_multiple_of(SAMPLE_TICKS) {
        return;
    }
    let mut haulers = BTreeMap::<SwarmId, u32>::new();
    let mut busy = BTreeMap::<SwarmId, u32>::new();
    for (kind, member, assigned, loaded, loading) in &bots {
        if *kind != NanobotType::Hauler {
            continue;
        }
        *haulers.entry(member.0).or_default() += 1;
        if assigned || loaded || loading {
            *busy.entry(member.0).or_default() += 1;
        }
    }

    let live_swarms = territory.swarms().collect::<Vec<_>>();
    quotas.swarms.retain(|swarm, _| live_swarms.contains(swarm));
    let consumed = |swarm| {
        ledger
            .as_ref()
            .map_or(0, |ledger| ledger.consumed_minerals_for(swarm))
    };
    let extracted = |swarm| {
        ledger
            .as_ref()
            .map_or(0, |ledger| ledger.extracted_minerals_for(swarm))
    };
    for swarm in live_swarms {
        let Some(window) = quotas.swarms.get_mut(&swarm) else {
            // Samples cover the span after them, so a new window counts from
            // its next sample.
            quotas.swarms.insert(
                swarm,
                HaulWindow::new(tick, consumed(swarm), extracted(swarm)),
            );
            continue;
        };
        let haulers = haulers.get(&swarm).copied().unwrap_or_default();
        window.hauler_ticks += u64::from(haulers) * SAMPLE_TICKS;
        window.busy_hauler_ticks +=
            u64::from(busy.get(&swarm).copied().unwrap_or_default()) * SAMPLE_TICKS;
        let economy = gather.economy(swarm).unwrap_or_default();
        window.spending_capacity_ticks += economy.spending_capacity * SAMPLE_TICKS as f32;
        let elapsed = tick - window.started;
        if elapsed < HAUL_WINDOW_TICKS {
            continue;
        }
        let (consumed_now, extracted_now) = (consumed(swarm), extracted(swarm));
        let seconds = elapsed as f32 / TICKS_PER_SECOND as f32;
        let delivered = consumed_now.saturating_sub(window.consumed_at_start);
        let income = extracted_now.saturating_sub(window.extracted_at_start) as f32 / seconds;
        let sample = WindowSample {
            mean_haulers: window.hauler_ticks as f32 / elapsed as f32,
            delivered_rate: delivered as f32 / seconds,
            haul_rate: if window.busy_hauler_ticks > 0 {
                delivered as f32 * TICKS_PER_SECOND as f32 / window.busy_hauler_ticks as f32
            } else {
                0.0
            },
        };
        let target_flow = target_flow(
            income,
            economy.reserve_stock,
            economy.reserve_target,
            tuning.production.reserve_seconds,
            economy.spending_capacity,
        );
        let cap = window
            .guard
            .update(tick, sample, haulers, &tuning.logistics);
        window.economy = Some(HaulEconomy {
            spending_capacity: window.spending_capacity_ticks / elapsed as f32,
            target_flow,
            delivered_rate: sample.delivered_rate,
            haul_rate: sample.haul_rate,
            haulers,
            growing: window.guard.growing(),
            haul_quota: throughput_quota(target_flow, sample.haul_rate).min(cap),
        });
        window.reopen(tick, consumed_now, extracted_now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn logistics() -> LogisticsTuning {
        LogisticsTuning {
            haul_growth_step: 0.25,
            haul_growth_threshold: 0.5,
            haul_growth_retry_ticks: 3600,
            ..default()
        }
    }

    fn sample(mean_haulers: f32, delivered_rate: f32, haul_rate: f32) -> WindowSample {
        WindowSample {
            mean_haulers,
            delivered_rate,
            haul_rate,
        }
    }

    #[test]
    fn a_new_session_forgets_the_previous_quota() {
        let mut window = HaulWindow::new(0, 500, 900);
        window.economy = Some(HaulEconomy {
            spending_capacity: 0.0,
            haul_quota: 30,
            ..default()
        });
        let mut world = World::new();
        world.insert_resource(HaulQuotas {
            tick: 9_000,
            swarms: BTreeMap::from([(SwarmId::PLAYER, window)]),
        });
        crate::nanobot::reset_session(&mut world);
        let quotas = world.resource::<HaulQuotas>();
        assert_eq!(quotas.economy(SwarmId::PLAYER), None);
        assert_eq!(quotas.tick, 0);
    }

    #[test]
    fn target_flow_adds_backlog_above_the_reserve_and_caps_at_spending() {
        assert_eq!(target_flow(3.0, 100, 100, 10.0, 15.0), 3.0);
        assert_eq!(target_flow(3.0, 200, 100, 10.0, 15.0), 13.0);
        assert_eq!(target_flow(3.0, 400, 100, 10.0, 15.0), 15.0);
        assert_eq!(
            target_flow(3.0, 0, 100, 10.0, 15.0),
            0.0,
            "refilling the reserve"
        );
    }

    #[test]
    fn throughput_quota_waits_for_a_measured_rate() {
        assert_eq!(throughput_quota(10.0, 0.0), 0);
        assert_eq!(throughput_quota(10.0, 0.3), 34);
        assert_eq!(throughput_quota(0.0, 0.3), 0);
    }

    #[test]
    fn growth_steps_by_a_quarter_of_the_haulers() {
        let tuning = logistics();
        let mut guard = GrowthGuard::default();
        assert_eq!(
            guard.update(1200, sample(2.0, 1.0, 0.5), 2, &tuning),
            3,
            "at least one"
        );
        assert_eq!(
            guard.update(2400, sample(2.0, 1.0, 0.5), 2, &tuning),
            3,
            "no evidence yet"
        );
        assert_eq!(guard.update(3600, sample(19.0, 9.0, 0.5), 20, &tuning), 25);
    }

    #[test]
    fn growth_stops_when_added_haulers_do_not_raise_delivery_and_retries_later() {
        let tuning = logistics();
        let mut guard = GrowthGuard::default();
        guard.update(1200, sample(20.0, 4.0, 0.2), 20, &tuning);
        // Four more Haulers at the earlier 0.2/s should add 0.8/s; 0.3/s is
        // under half.
        assert_eq!(guard.update(2400, sample(24.0, 4.3, 0.18), 24, &tuning), 24);
        assert!(!guard.growing());
        assert_eq!(
            guard.update(3600, sample(24.0, 4.3, 0.18), 24, &tuning),
            24,
            "holding"
        );
        assert_eq!(
            guard.update(2400 + 3600, sample(24.0, 4.3, 0.18), 24, &tuning),
            30,
            "retried after the delay"
        );
    }

    #[test]
    fn late_haulers_count_only_for_the_time_they_were_present() {
        let tuning = logistics();
        let mut guard = GrowthGuard::default();
        guard.update(1200, sample(20.0, 4.0, 0.2), 20, &tuning);
        // Four Haulers arrived halfway through: two more on average, which
        // should add 0.4/s; 0.25/s is enough.
        assert_eq!(
            guard.update(2400, sample(22.0, 4.25, 0.19), 24, &tuning),
            30
        );
        assert!(guard.growing());
    }

    #[test]
    fn growth_resumes_once_added_haulers_raise_delivery_again() {
        let tuning = logistics();
        let mut guard = GrowthGuard::default();
        guard.update(1200, sample(20.0, 4.0, 0.2), 20, &tuning);
        guard.update(2400, sample(24.0, 4.0, 0.17), 24, &tuning);
        assert!(!guard.growing());
        assert_eq!(guard.update(3600, sample(28.0, 5.0, 0.18), 28, &tuning), 35);
        assert!(guard.growing());
    }
}
