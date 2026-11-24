//! Per-battle measurements, kept in memory and served by `battle.stats`.

use crate::{
    ai_battle::AiBattleConfig,
    nanobot::{
        Cargo, Charge, Charger, Health, MatchOutcome, Nanobot, NanobotType, OwnerSwarm,
        PlannedStructure, ProductionFacility, Structure, Swarm, SwarmEliminationSet,
        SwarmEliminationState, SwarmId, SwarmMember, world_to_cell,
    },
    perf::{HitchEpisodeSummary, HitchRecorderPlugin},
    resources::{ResourceDeposit, ResourceKind, Stockpile, buffered_minerals},
    session::SessionRules,
    strategic_runtime::{ControllerProfile, ControllerTelemetry},
    tuning::{Tuning, TuningEdit, TuningJournal},
};
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    time::Instant,
};

#[derive(Resource, Debug, Clone, Default)]
pub struct BattleStatisticsConfig {
    pub seed: u64,
    pub headless: bool,
    pub unbounded: bool,
    pub simulation_only: bool,
    pub max_ticks: Option<std::num::NonZeroU64>,
}

#[derive(Debug, Default, Clone, Copy, Serialize, Deserialize)]
pub struct BattleTotals {
    pub births: u64,
    pub deaths: u64,
    pub minerals_gathered: u64,
    pub minerals_consumed: u64,
    pub structures_built: u64,
    pub structures_lost: u64,
    pub effective_damage_total: u64,
    pub effective_damage_nanobots: u64,
    pub effective_damage_structures: u64,
}

pub enum BattleEvent {
    Birth,
    Death,
    Gathered {
        amount: u32,
        deposit: Entity,
        painted: bool,
    },
    MineralStarvedFacilityTick,
    Consumed(u32),
    StructureBuilt,
    StructureLost,
    EffectiveNanobotDamage(u32),
    EffectiveStructureDamage(u32),
}

#[derive(Debug, Default)]
struct EconomyWindow {
    extracted_deposits: BTreeSet<Entity>,
    mineral_starved_facility_ticks: u64,
}

#[derive(Resource, Debug, Default)]
pub struct BattleCounters {
    totals: BTreeMap<u32, BattleTotals>,
    minerals_by_deposit: BTreeMap<(SwarmId, Entity), u64>,
    economy_windows: BTreeMap<u32, EconomyWindow>,
    frozen: bool,
}

impl BattleCounters {
    /// Whether the battle has ended and its statistics no longer change.
    pub fn is_frozen(&self) -> bool {
        self.frozen
    }

    pub fn record(&mut self, swarm: SwarmId, event: BattleEvent) {
        if self.frozen {
            return;
        }
        let total = self.totals.entry(swarm.0).or_default();
        match event {
            BattleEvent::Birth => total.births += 1,
            BattleEvent::Death => total.deaths += 1,
            BattleEvent::Gathered {
                amount,
                deposit,
                painted,
            } => {
                total.minerals_gathered += u64::from(amount);
                if amount > 0 {
                    *self
                        .minerals_by_deposit
                        .entry((swarm, deposit))
                        .or_default() += u64::from(amount);
                    if painted {
                        self.economy_windows
                            .entry(swarm.0)
                            .or_default()
                            .extracted_deposits
                            .insert(deposit);
                    }
                }
            }
            BattleEvent::MineralStarvedFacilityTick => {
                self.economy_windows
                    .entry(swarm.0)
                    .or_default()
                    .mineral_starved_facility_ticks += 1;
            }
            BattleEvent::Consumed(amount) => total.minerals_consumed += u64::from(amount),
            BattleEvent::StructureBuilt => total.structures_built += 1,
            BattleEvent::StructureLost => total.structures_lost += 1,
            BattleEvent::EffectiveNanobotDamage(amount) => {
                total.effective_damage_total += u64::from(amount);
                total.effective_damage_nanobots += u64::from(amount);
            }
            BattleEvent::EffectiveStructureDamage(amount) => {
                total.effective_damage_total += u64::from(amount);
                total.effective_damage_structures += u64::from(amount);
            }
        }
    }

    pub fn totals_for(&self, swarm: SwarmId) -> BattleTotals {
        self.totals.get(&swarm.0).copied().unwrap_or_default()
    }
}

pub struct BattleStatisticsPlugin;
impl Plugin for BattleStatisticsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<BattleStatisticsConfig>()
            .init_resource::<SessionRules>()
            .init_resource::<Tuning>()
            .init_resource::<TuningJournal>()
            .add_message::<AppExit>()
            .add_systems(
                PostStartup,
                start_run.run_if(not(resource_exists::<
                    crate::session_lifecycle::SessionGeneration,
                >)),
            )
            .add_systems(
                FixedFirst,
                start_tick
                    .before(crate::navigation_runtime::refresh_navigation)
                    .run_if(resource_exists::<BattleRun>),
            )
            .add_systems(
                FixedLast,
                collect_tick
                    .after(SwarmEliminationSet)
                    .run_if(resource_exists::<BattleRun>),
            );
        if !app
            .world()
            .resource::<BattleStatisticsConfig>()
            .simulation_only
            && !app.is_plugin_added::<HitchRecorderPlugin>()
        {
            app.add_plugins(HitchRecorderPlugin);
        }
    }
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Timing {
    pub count: usize,
    pub mean_ms: f64,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub p99_ms: f64,
    pub max_ms: f64,
}

impl Timing {
    fn from_samples(samples: &mut [f64]) -> Self {
        if samples.is_empty() {
            return Self::default();
        }
        samples.sort_unstable_by(f64::total_cmp);
        let quantile = |fraction: f64| {
            samples[((samples.len() as f64 * fraction).ceil() as usize).saturating_sub(1)]
        };
        Self {
            count: samples.len(),
            mean_ms: samples.iter().sum::<f64>() / samples.len() as f64,
            p50_ms: quantile(0.5),
            p95_ms: quantile(0.95),
            p99_ms: quantile(0.99),
            max_ms: *samples.last().unwrap(),
        }
    }
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct SwarmSample {
    pub population: u64,
    pub workers: u64,
    pub haulers: u64,
    pub defenders: u64,
    pub owned_deposits: usize,
    pub mineral_income: u64,
    pub mineral_starved_facility_seconds: f64,
    pub unspent_minerals: u32,
    pub empty_charge_defenders: u64,
    pub elimination_seconds: Option<f64>,
    #[serde(default)]
    pub deposit_extraction: Vec<DepositExtraction>,
    #[serde(flatten)]
    pub totals: BattleTotals,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DepositExtraction {
    pub cell: [i32; 2],
    pub owner: Option<u32>,
    pub minerals_extracted: u64,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct BattleSample {
    pub simulation_seconds: f64,
    pub fixed_tick: u64,
    pub tick_timing: Timing,
    pub frame_timing: Option<Timing>,
    pub controllers: BTreeMap<u32, ControllerProfile>,
    pub swarms: BTreeMap<u32, SwarmSample>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    InProgress,
    Completed,
    Interrupted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BattleTerminationReason {
    SwarmElimination,
    TickLimit,
}

/// The most compact hitch summaries a run keeps; later ones are only counted.
const HITCH_SUMMARY_LIMIT: usize = 1_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BattleSummary {
    pub schema_version: u32,
    pub scenario: String,
    pub seed: u64,
    pub timestep_seconds: f64,
    pub headless: bool,
    pub unbounded: bool,
    #[serde(default)]
    pub simulation_only: bool,
    #[serde(default)]
    pub max_ticks: Option<std::num::NonZeroU64>,
    pub code_revision: String,
    pub dirty_worktree: bool,
    pub ai_battle: AiBattleConfig,
    pub tuning: Tuning,
    pub tuning_edits: Vec<TuningEdit>,
    pub status: RunStatus,
    pub outcome: Option<String>,
    #[serde(default)]
    pub termination_reason: Option<BattleTerminationReason>,
    pub samples: u64,
    #[serde(default)]
    pub hitch_episodes_suppressed: u64,
    pub latest: BattleSample,
}

impl BattleSummary {
    fn records_frame_timing(&self) -> bool {
        !self.headless || !self.unbounded
    }
}

#[derive(Resource)]
pub struct BattleRun {
    pub summary: BattleSummary,
    samples: Vec<serde_json::Value>,
    hitch_summaries: Vec<HitchEpisodeSummary>,
    tick_start: Instant,
    ticks: u64,
    tuning_journal_offset: usize,
    tuning_start_tick: u64,
    elimination_ticks: BTreeMap<SwarmId, u64>,
    next_sample_seconds: f64,
    tick_samples: Vec<f64>,
    frame_samples: Vec<f64>,
}

fn start_run(world: &mut World) {
    start_session(world);
}

/// Begin recording the session just populated, replacing any earlier run.
pub(crate) fn start_session(world: &mut World) {
    if !world.resource::<SessionRules>().record_statistics {
        world.remove_resource::<BattleRun>();
        world.remove_resource::<BattleCounters>();
        return;
    }
    let config = world.resource::<BattleStatisticsConfig>().clone();
    let run = BattleRun::new(
        &config,
        world
            .get_resource::<AiBattleConfig>()
            .cloned()
            .unwrap_or_default(),
        world.resource::<Tuning>().clone(),
        world.resource::<Time<Fixed>>().timestep().as_secs_f64(),
        world.resource::<SessionRules>().scenario_name,
        world
            .get_resource::<TuningJournal>()
            .map_or(0, |journal| journal.entries.len()),
        crate::tuning::fixed_tick(world),
    );
    world.insert_resource(run);
    world.insert_resource(BattleCounters::default());
}

impl BattleRun {
    fn new(
        config: &BattleStatisticsConfig,
        ai_battle: AiBattleConfig,
        tuning: Tuning,
        timestep_seconds: f64,
        scenario_name: &str,
        tuning_journal_offset: usize,
        tuning_start_tick: u64,
    ) -> Self {
        Self {
            summary: BattleSummary {
                schema_version: 9,
                scenario: scenario_name.into(),
                seed: config.seed,
                timestep_seconds,
                headless: config.headless,
                unbounded: config.unbounded,
                simulation_only: config.simulation_only,
                max_ticks: config.max_ticks,
                code_revision: env!("NANO_SWARM_CODE_REVISION").into(),
                dirty_worktree: env!("NANO_SWARM_DIRTY_WORKTREE") == "true",
                ai_battle,
                tuning,
                tuning_edits: Vec::new(),
                status: RunStatus::InProgress,
                outcome: None,
                termination_reason: None,
                samples: 0,
                hitch_episodes_suppressed: 0,
                latest: BattleSample::default(),
            },
            samples: Vec::new(),
            hitch_summaries: Vec::new(),
            tick_start: Instant::now(),
            ticks: 0,
            tuning_journal_offset,
            tuning_start_tick,
            elimination_ticks: BTreeMap::new(),
            next_sample_seconds: 1.,
            tick_samples: Vec::with_capacity(64),
            frame_samples: Vec::with_capacity(64),
        }
    }

    pub(crate) fn record_frame(
        &mut self,
        frame_ns: u64,
        paused: bool,
        closed: Option<&HitchEpisodeSummary>,
    ) {
        if self.summary.status != RunStatus::InProgress {
            return;
        }
        if self.summary.records_frame_timing() && !paused {
            self.frame_samples.push(frame_ns as f64 / 1_000_000.);
        }
        if let Some(episode) = closed {
            if self.hitch_summaries.len() < HITCH_SUMMARY_LIMIT {
                self.hitch_summaries.push(episode.clone());
            } else {
                self.summary.hitch_episodes_suppressed += 1;
            }
        }
    }

    /// The `battle.stats` result: summary plus the samples and hitch summaries
    /// whose indices exceed the optional cursors.
    pub(crate) fn stats_payload(
        &self,
        after_sample: Option<u64>,
        after_hitch: Option<u64>,
    ) -> serde_json::Value {
        fn newer<T>(list: &[T], after: Option<u64>) -> &[T] {
            let start = after.map_or(0, |after| {
                usize::try_from(after).map_or(list.len(), |n| n.saturating_add(1))
            });
            &list[start.min(list.len())..]
        }
        let mut summary = serde_json::to_value(&self.summary).expect("summary serializes");
        if let Some(object) = summary.as_object_mut() {
            object.remove("latest");
        }
        serde_json::json!({
            "summary": summary,
            "samples": newer(&self.samples, after_sample),
            "hitches": newer(&self.hitch_summaries, after_hitch),
            "last_sample": self.samples.len().checked_sub(1),
            "last_hitch": self.hitch_summaries.len().checked_sub(1),
        })
    }

    fn record_sample(&mut self, sample: BattleSample) {
        self.samples.push(sample_json(&sample));
        self.summary.samples += 1;
        self.summary.latest = sample;
    }
}

/// One sample as typed JSON: a `swarms` array with one object per swarm,
/// repeating the sample-level fields in each swarm object.
fn sample_json(sample: &BattleSample) -> serde_json::Value {
    let t = &sample.tick_timing;
    let frame = sample.frame_timing.as_ref();
    let swarms: Vec<_> = sample
        .swarms
        .iter()
        .map(|(swarm, s)| {
            let c = s.totals;
            let controller = sample.controllers.get(swarm).cloned().unwrap_or_default();
            serde_json::json!({
                "simulation_seconds": sample.simulation_seconds,
                "fixed_tick": sample.fixed_tick,
                "swarm": swarm,
                "population": s.population,
                "workers": s.workers,
                "haulers": s.haulers,
                "defenders": s.defenders,
                "births": c.births,
                "deaths": c.deaths,
                "minerals_gathered": c.minerals_gathered,
                "minerals_consumed": c.minerals_consumed,
                "structures_built": c.structures_built,
                "structures_lost": c.structures_lost,
                "effective_damage_total": c.effective_damage_total,
                "effective_damage_nanobots": c.effective_damage_nanobots,
                "effective_damage_structures": c.effective_damage_structures,
                "elimination_seconds": s.elimination_seconds,
                "tick_count": t.count,
                "tick_mean_ms": t.mean_ms,
                "tick_p50_ms": t.p50_ms,
                "tick_p95_ms": t.p95_ms,
                "tick_p99_ms": t.p99_ms,
                "tick_max_ms": t.max_ms,
                "frame_count": frame.map(|f| f.count),
                "frame_mean_ms": frame.map(|f| f.mean_ms),
                "frame_p95_ms": frame.map(|f| f.p95_ms),
                "frame_max_ms": frame.map(|f| f.max_ms),
                "controller_reviews": controller.reviews,
                "controller_intent_edits": controller.intent_edits,
                "controller_work_units": controller.work_units,
                "owned_deposits": s.owned_deposits,
                "mineral_income": s.mineral_income,
                "mineral_starved_facility_seconds": s.mineral_starved_facility_seconds,
                "unspent_minerals": s.unspent_minerals,
                "empty_charge_defenders": s.empty_charge_defenders,
                "controller_last_explanation": controller.last_explanation,
                "deposit_extraction": s.deposit_extraction,
            })
        })
        .collect();
    serde_json::json!({ "swarms": swarms })
}

fn start_tick(mut run: ResMut<BattleRun>) {
    if run.summary.status == RunStatus::InProgress {
        run.tick_start = Instant::now();
    }
}

fn snapshot(world: &mut World, run: &mut BattleRun) -> BattleSample {
    if let Some(journal) = world.get_resource::<TuningJournal>() {
        run.summary.tuning_edits.extend(
            journal.entries[run.tuning_journal_offset..]
                .iter()
                .cloned()
                .map(|mut edit| {
                    edit.fixed_tick = edit.fixed_tick.saturating_sub(run.tuning_start_tick);
                    edit
                }),
        );
        run.tuning_journal_offset = journal.entries.len();
    }
    let mut swarms = BTreeMap::new();
    let mut owners = world.query_filtered::<&SwarmId, With<Swarm>>();
    for id in owners.iter(world) {
        swarms.insert(id.0, SwarmSample::default());
    }
    for (owner, kind, charge, cargo, health) in world
        .query_filtered::<(
            &SwarmMember,
            &NanobotType,
            Option<&Charge>,
            Option<&Cargo>,
            Option<&Health>,
        ), With<Nanobot>>()
        .iter(world)
    {
        let s = swarms.entry(owner.0.0).or_default();
        if let Some(cargo) = cargo
            && cargo.kind == ResourceKind::Minerals
            && health.is_none_or(|health| health.current > 0)
        {
            s.unspent_minerals = s.unspent_minerals.saturating_add(cargo.amount);
        }
        s.population += 1;
        match kind {
            NanobotType::Worker => s.workers += 1,
            NanobotType::Hauler => s.haulers += 1,
            NanobotType::Defender => {
                s.defenders += 1;
                s.empty_charge_defenders += u64::from(charge.is_some_and(Charge::is_empty));
            }
        }
    }
    for (owner, condition, facility, stockpile, charger) in world
        .query_filtered::<(
            &OwnerSwarm,
            Option<&Structure>,
            Option<&ProductionFacility>,
            Option<&Stockpile>,
            Option<&Charger>,
        ), (
            Without<PlannedStructure>,
            Or<(With<ProductionFacility>, With<Stockpile>, With<Charger>)>,
        )>()
        .iter(world)
    {
        if condition.is_none_or(Structure::is_operational)
            && let Ok(id) = owners.get(world, owner.0)
        {
            let swarm = swarms.entry(id.0).or_default();
            swarm.unspent_minerals = swarm
                .unspent_minerals
                .saturating_add(buffered_minerals(facility, stockpile, charger));
        }
    }
    let simulation_seconds = run.ticks as f64 * run.summary.timestep_seconds;
    let economy_windows =
        std::mem::take(&mut world.resource_mut::<BattleCounters>().economy_windows);
    let mut deposits: Vec<_> = world
        .query_filtered::<(Entity, &Transform, Option<&OwnerSwarm>), With<ResourceDeposit>>()
        .iter(world)
        .map(|(deposit, transform, owner)| {
            let cell = world_to_cell(transform.translation.truncate()).to_array();
            let owner = owner.and_then(|owner| owners.get(world, owner.0).ok());
            (cell, owner.map(|id| id.0), deposit)
        })
        .collect();
    deposits.sort_unstable();
    let counters = world.resource::<BattleCounters>();
    for (id, swarm) in &mut swarms {
        swarm.totals = counters.totals_for(SwarmId(*id));
        swarm.deposit_extraction = deposits
            .iter()
            .map(|&(cell, owner, deposit)| DepositExtraction {
                cell,
                owner,
                minerals_extracted: counters
                    .minerals_by_deposit
                    .get(&(SwarmId(*id), deposit))
                    .copied()
                    .unwrap_or_default(),
            })
            .collect();
        let previously_gathered = run
            .summary
            .latest
            .swarms
            .get(id)
            .map_or(0, |previous| previous.totals.minerals_gathered);
        swarm.mineral_income = swarm.totals.minerals_gathered - previously_gathered;
        if let Some(window) = economy_windows.get(id) {
            swarm.owned_deposits = window.extracted_deposits.len();
            swarm.mineral_starved_facility_seconds =
                window.mineral_starved_facility_ticks as f64 * run.summary.timestep_seconds;
        }
        swarm.elimination_seconds = run
            .elimination_ticks
            .get(&SwarmId(*id))
            .map(|tick| *tick as f64 * run.summary.timestep_seconds);
    }
    let tick_timing = Timing::from_samples(&mut run.tick_samples);
    run.tick_samples.clear();
    let frame_timing = run
        .summary
        .records_frame_timing()
        .then(|| Timing::from_samples(&mut run.frame_samples));
    run.frame_samples.clear();
    let controllers = world
        .get_resource::<ControllerTelemetry>()
        .map(ControllerTelemetry::profiles)
        .unwrap_or_default();
    BattleSample {
        simulation_seconds,
        fixed_tick: run.ticks,
        tick_timing,
        frame_timing,
        controllers,
        swarms,
    }
}

fn collect_tick(world: &mut World) {
    let mut run = world.remove_resource::<BattleRun>().unwrap();
    if run.summary.status != RunStatus::InProgress {
        world.insert_resource(run);
        return;
    }
    run.tick_samples
        .push(run.tick_start.elapsed().as_secs_f64() * 1000.);
    run.ticks += 1;
    if let Some(state) = world.get_resource::<SwarmEliminationState>() {
        for id in &state.eliminated {
            run.elimination_ticks.entry(*id).or_insert(run.ticks);
        }
    }
    let outcome = match world
        .get_resource::<MatchOutcome>()
        .copied()
        .unwrap_or_default()
    {
        MatchOutcome::InProgress => None,
        terminal => Some(world.resource::<SessionRules>().outcome_name(terminal)),
    };
    if let Some(outcome) = outcome {
        run.summary.status = RunStatus::Completed;
        run.summary.outcome = Some(outcome);
        run.summary.termination_reason = Some(BattleTerminationReason::SwarmElimination);
    } else if run
        .summary
        .max_ticks
        .is_some_and(|limit| run.ticks >= limit.get())
    {
        run.summary.status = RunStatus::Interrupted;
        run.summary.termination_reason = Some(BattleTerminationReason::TickLimit);
    }
    let terminal = run.summary.status != RunStatus::InProgress;
    if terminal {
        world.resource_mut::<BattleCounters>().frozen = true;
    }
    if terminal || run.ticks as f64 * run.summary.timestep_seconds >= run.next_sample_seconds {
        let sample = snapshot(world, &mut run);
        run.record_sample(sample);
        run.next_sample_seconds += 1.;
    }
    if terminal && run.summary.headless {
        // A non-windowed battle freezes its simulation and waits for `process.shutdown`.
        world.resource_mut::<Time<Virtual>>().pause();
        let mut clock = world.resource_mut::<Time<Fixed>>();
        let overstep = clock.overstep();
        clock.discard_overstep(overstep);
    }
    world.insert_resource(run);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn battle_stats_keep_the_first_thousand_hitch_summaries_and_count_the_rest() {
        let mut run = BattleRun::new(
            &BattleStatisticsConfig::default(),
            AiBattleConfig::default(),
            Tuning::default(),
            1. / 60.,
            "ai-battle",
            0,
            0,
        );
        let episode = HitchEpisodeSummary::default();
        for _ in 0..HITCH_SUMMARY_LIMIT + 2 {
            run.record_frame(0, false, Some(&episode));
        }

        let stats = run.stats_payload(None, None);

        assert_eq!(stats["hitches"].as_array().map(Vec::len), Some(1_000));
        assert_eq!(stats["last_hitch"], 999);
        assert_eq!(stats["summary"]["hitch_episodes_suppressed"], 2);
    }
}
