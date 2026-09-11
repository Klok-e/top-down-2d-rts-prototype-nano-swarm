//! Per-battle measurements and durable run results.

use crate::{
    battle_experiment::BattleExperimentConfig,
    gameplay_pacing::GameplayPacing,
    nanobot::{
        MatchOutcome, Nanobot, NanobotType, Swarm, SwarmEliminationSet, SwarmEliminationState,
        SwarmId, SwarmMember,
    },
    session::SessionRules,
    strategic_runtime::{ControllerProfile, ControllerTelemetry},
};
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    fs::{self, File, OpenOptions},
    io::{self, BufWriter, Write},
    path::PathBuf,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

#[derive(Resource, Debug, Clone)]
pub struct BattleStatisticsConfig {
    pub output_root: PathBuf,
    pub seed: u64,
    pub headless: bool,
}

impl Default for BattleStatisticsConfig {
    fn default() -> Self {
        Self {
            output_root: PathBuf::from("target/battle-runs"),
            seed: 0,
            headless: false,
        }
    }
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
    pub scored_damage_total: u64,
    pub scored_damage_nanobots: u64,
    pub scored_damage_structures: u64,
}

pub enum BattleEvent {
    Birth,
    Death,
    Gathered(u32),
    Consumed(u32),
    StructureBuilt,
    StructureLost,
    EffectiveNanobotDamage(u32),
    EffectiveStructureDamage(u32),
    ScoredNanobotDamage(u32),
    ScoredStructureDamage(u32),
}

#[derive(Resource, Debug, Default)]
pub struct BattleCounters {
    totals: BTreeMap<u32, BattleTotals>,
    scored_damage_by_target: HashMap<Entity, u32>,
    frozen: bool,
}

impl BattleCounters {
    pub fn record(&mut self, swarm: SwarmId, event: BattleEvent) {
        if self.frozen {
            return;
        }
        let total = self.totals.entry(swarm.0).or_default();
        match event {
            BattleEvent::Birth => total.births += 1,
            BattleEvent::Death => total.deaths += 1,
            BattleEvent::Gathered(amount) => total.minerals_gathered += u64::from(amount),
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
            BattleEvent::ScoredNanobotDamage(amount) => {
                total.scored_damage_total += u64::from(amount);
                total.scored_damage_nanobots += u64::from(amount);
            }
            BattleEvent::ScoredStructureDamage(amount) => {
                total.scored_damage_total += u64::from(amount);
                total.scored_damage_structures += u64::from(amount);
            }
        }
    }

    pub(crate) fn claim_scored_damage(
        &mut self,
        target: Entity,
        target_max_health: u32,
        effective_damage: u32,
    ) -> u32 {
        if self.frozen {
            return 0;
        }
        let credited = self.scored_damage_by_target.entry(target).or_default();
        let scored = effective_damage.min(target_max_health.saturating_sub(*credited));
        *credited += scored;
        scored
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
            )
            .add_systems(First, collect_frame.run_if(resource_exists::<BattleRun>))
            .add_systems(Last, finish_on_exit.run_if(resource_exists::<BattleRun>));
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
    pub elimination_seconds: Option<f64>,
    #[serde(flatten)]
    pub totals: BattleTotals,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct BattleSample {
    pub simulation_seconds: f64,
    pub fixed_tick: u64,
    pub tick_timing: Timing,
    pub frame_timing: Option<Timing>,
    pub controllers: BTreeMap<u32, ControllerProfile>,
    pub observation_max_ms: f64,
    pub swarms: BTreeMap<u32, SwarmSample>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    InProgress,
    Completed,
    Unresolved,
    Interrupted,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BattleSummary {
    pub schema_version: u32,
    pub scenario: String,
    pub seed: u64,
    pub timestep_seconds: f64,
    pub headless: bool,
    pub code_revision: String,
    pub dirty_worktree: bool,
    pub experiment: BattleExperimentConfig,
    pub gameplay_pacing: GameplayPacing,
    pub status: RunStatus,
    pub outcome: Option<String>,
    pub samples: u64,
    pub latest: BattleSample,
}

impl BattleSummary {
    fn records_frame_timing(&self) -> bool {
        !self.headless || self.experiment.realtime
    }
}

#[derive(Resource)]
pub struct BattleRun {
    pub directory: PathBuf,
    pub summary: BattleSummary,
    pub error: Option<String>,
    csv: BufWriter<File>,
    tick_start: Instant,
    ticks: u64,
    elimination_ticks: BTreeMap<SwarmId, u64>,
    next_sample_seconds: f64,
    tick_samples: Vec<f64>,
    frame_samples: Vec<f64>,
    frame_start: Option<Instant>,
}

fn start_run(world: &mut World) {
    start_session(world);
}

pub(crate) fn start_session(world: &mut World) {
    if !world.resource::<SessionRules>().record_statistics {
        world.remove_resource::<BattleRun>();
        world.remove_resource::<BattleCounters>();
        return;
    }
    let config = world.resource::<BattleStatisticsConfig>().clone();
    let experiment = world
        .get_resource::<BattleExperimentConfig>()
        .cloned()
        .unwrap_or_default();
    let gameplay_pacing = world
        .get_resource::<GameplayPacing>()
        .cloned()
        .unwrap_or_default();
    let result = create_run(
        &config,
        experiment,
        gameplay_pacing,
        world.resource::<Time<Fixed>>().timestep().as_secs_f64(),
        world.resource::<SessionRules>().scenario_name,
    );
    match result {
        Ok(run) => {
            world.insert_resource(run);
            world.insert_resource(BattleCounters::default());
        }
        Err(error) => {
            error!("Cannot create battle results: {error}");
            world.write_message(AppExit::error());
        }
    }
}

/// Finalize an active recording before its ECS session is replaced.
///
/// A failed durable write retains the run so the menu action can retry without
/// tearing down the session it describes.
pub(crate) fn finish_session(world: &mut World) -> bool {
    let Some(mut run) = world.remove_resource::<BattleRun>() else {
        world.remove_resource::<BattleCounters>();
        return true;
    };
    if run.summary.status == RunStatus::InProgress {
        run.summary.status = RunStatus::Interrupted;
        if let Some(mut counters) = world.get_resource_mut::<BattleCounters>() {
            counters.frozen = true;
        } else {
            world.insert_resource(BattleCounters::default());
        }
        let result = if run.error.is_some() {
            run.persist()
        } else {
            let sample = snapshot(world, &mut run);
            run.write_sample(sample)
        };
        if let Err(error) = result {
            error!("Cannot save interrupted battle: {error}");
            run.error = Some(error.to_string());
            world.insert_resource(run);
            return false;
        }
    } else if run.error.is_some()
        && let Err(error) = run.persist()
    {
        error!("Cannot finish battle results: {error}");
        run.error = Some(error.to_string());
        world.insert_resource(run);
        return false;
    }
    run.error = None;
    world.insert_resource(run);
    true
}

fn create_run(
    config: &BattleStatisticsConfig,
    experiment: BattleExperimentConfig,
    gameplay_pacing: GameplayPacing,
    timestep_seconds: f64,
    scenario_name: &str,
) -> io::Result<BattleRun> {
    fs::create_dir_all(&config.output_root)?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(io::Error::other)?
        .as_nanos();
    let directory = config
        .output_root
        .join(format!("battle-{stamp}-{}", std::process::id()));
    fs::create_dir(&directory)?;
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(directory.join("samples.csv"))?;
    let mut csv = BufWriter::new(file);
    writeln!(
        csv,
        "simulation_seconds,fixed_tick,swarm,population,workers,haulers,defenders,births,deaths,minerals_gathered,minerals_consumed,structures_built,structures_lost,effective_damage_total,effective_damage_nanobots,effective_damage_structures,scored_damage_total,scored_damage_nanobots,scored_damage_structures,elimination_seconds,tick_count,tick_mean_ms,tick_p50_ms,tick_p95_ms,tick_p99_ms,tick_max_ms,frame_count,frame_mean_ms,frame_p95_ms,frame_max_ms,controller_reviews,controller_intent_edits,controller_work_units,controller_mean_ms,controller_p95_ms,controller_p95_window_samples,controller_max_ms,controller_last_explanation,observation_max_ms"
    )?;
    let summary = BattleSummary {
        schema_version: 3,
        scenario: scenario_name.into(),
        seed: config.seed,
        timestep_seconds,
        headless: config.headless,
        code_revision: env!("NANO_SWARM_CODE_REVISION").into(),
        dirty_worktree: env!("NANO_SWARM_DIRTY_WORKTREE") == "true",
        experiment,
        gameplay_pacing,
        status: RunStatus::InProgress,
        outcome: None,
        samples: 0,
        latest: BattleSample::default(),
    };
    let mut run = BattleRun {
        directory,
        summary,
        error: None,
        csv,
        tick_start: Instant::now(),
        ticks: 0,
        elimination_ticks: BTreeMap::new(),
        next_sample_seconds: 1.,
        tick_samples: Vec::with_capacity(64),
        frame_samples: Vec::with_capacity(64),
        frame_start: None,
    };
    run.persist()?;
    info!("AI Battle results: {}", run.directory.display());
    Ok(run)
}

impl BattleRun {
    fn persist(&mut self) -> io::Result<()> {
        self.csv.flush()?;
        self.csv.get_ref().sync_data()?;
        let temporary = self.directory.join("summary.json.tmp");
        let mut file = File::create(&temporary)?;
        serde_json::to_writer_pretty(&mut file, &self.summary)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        fs::rename(temporary, self.directory.join("summary.json"))
    }

    fn write_sample(&mut self, sample: BattleSample) -> io::Result<()> {
        let t = &sample.tick_timing;
        for (swarm, s) in &sample.swarms {
            let c = s.totals;
            let frame_count = sample
                .frame_timing
                .as_ref()
                .map_or(String::new(), |t| t.count.to_string());
            let frame_mean = sample
                .frame_timing
                .as_ref()
                .map_or(String::new(), |t| t.mean_ms.to_string());
            let frame_p95 = sample
                .frame_timing
                .as_ref()
                .map_or(String::new(), |t| t.p95_ms.to_string());
            let frame_max = sample
                .frame_timing
                .as_ref()
                .map_or(String::new(), |t| t.max_ms.to_string());
            let elimination = s
                .elimination_seconds
                .map_or(String::new(), |n| n.to_string());
            let controller = sample.controllers.get(swarm).cloned().unwrap_or_default();
            let explanation = csv_field(&controller.last_explanation);
            writeln!(
                self.csv,
                "{},{},{swarm},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{elimination},{},{},{},{},{},{},{frame_count},{frame_mean},{frame_p95},{frame_max},{},{},{},{},{},{},{},{explanation},{}",
                sample.simulation_seconds,
                sample.fixed_tick,
                s.population,
                s.workers,
                s.haulers,
                s.defenders,
                c.births,
                c.deaths,
                c.minerals_gathered,
                c.minerals_consumed,
                c.structures_built,
                c.structures_lost,
                c.effective_damage_total,
                c.effective_damage_nanobots,
                c.effective_damage_structures,
                c.scored_damage_total,
                c.scored_damage_nanobots,
                c.scored_damage_structures,
                t.count,
                t.mean_ms,
                t.p50_ms,
                t.p95_ms,
                t.p99_ms,
                t.max_ms,
                controller.reviews,
                controller.intent_edits,
                controller.work_units,
                controller.mean_ms,
                controller.p95_ms,
                controller.p95_window_samples,
                controller.max_ms,
                sample.observation_max_ms,
            )?;
        }
        self.summary.samples += 1;
        self.summary.latest = sample;
        self.persist()
    }
}

fn csv_field(value: &str) -> String {
    if value
        .chars()
        .any(|character| matches!(character, ',' | '"' | '\n' | '\r'))
    {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.into()
    }
}

fn start_tick(mut run: ResMut<BattleRun>) {
    if run.summary.status == RunStatus::InProgress {
        run.tick_start = Instant::now();
    }
}

fn snapshot(world: &mut World, run: &mut BattleRun) -> BattleSample {
    world.resource_scope(|world, mut counters: Mut<BattleCounters>| {
        counters
            .scored_damage_by_target
            .retain(|entity, _| world.entities().contains(*entity));
    });
    let mut swarms = BTreeMap::new();
    for id in world.query_filtered::<&SwarmId, With<Swarm>>().iter(world) {
        swarms.insert(
            id.0,
            SwarmSample {
                totals: world.resource::<BattleCounters>().totals_for(*id),
                ..default()
            },
        );
    }
    for (owner, kind) in world
        .query_filtered::<(&SwarmMember, &NanobotType), With<Nanobot>>()
        .iter(world)
    {
        let s = swarms.entry(owner.0.0).or_default();
        s.population += 1;
        match kind {
            NanobotType::Worker => s.workers += 1,
            NanobotType::Hauler => s.haulers += 1,
            NanobotType::Defender => s.defenders += 1,
        }
    }
    let simulation_seconds = run.ticks as f64 * run.summary.timestep_seconds;
    for (id, swarm) in &mut swarms {
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
    let (controllers, observation_max_ms) = world
        .get_resource::<ControllerTelemetry>()
        .map(|telemetry| (telemetry.profiles(), telemetry.observation_max_ms))
        .unwrap_or_default();
    BattleSample {
        simulation_seconds,
        fixed_tick: run.ticks,
        tick_timing,
        frame_timing,
        controllers,
        observation_max_ms,
        swarms,
    }
}

fn collect_tick(world: &mut World) {
    let mut run = world.remove_resource::<BattleRun>().unwrap();
    if run.summary.status != RunStatus::InProgress || run.error.is_some() {
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
    if let Some(ref outcome) = outcome {
        run.summary.status = RunStatus::Completed;
        run.summary.outcome = Some(outcome.clone());
        world.resource_mut::<BattleCounters>().frozen = true;
    } else if run
        .summary
        .experiment
        .cutoff_seconds
        .is_some_and(|cutoff| run.ticks as f64 * run.summary.timestep_seconds >= f64::from(cutoff))
    {
        run.summary.status = RunStatus::Unresolved;
        world.resource_mut::<BattleCounters>().frozen = true;
    }
    let terminal = run.summary.status != RunStatus::InProgress;
    if terminal || run.ticks as f64 * run.summary.timestep_seconds >= run.next_sample_seconds {
        let sample = snapshot(world, &mut run);
        if let Err(error) = run.write_sample(sample) {
            error!("Cannot save battle results: {error}");
            run.error = Some(error.to_string());
            world.write_message(AppExit::error());
        }
        run.next_sample_seconds += 1.;
    }
    if terminal && run.error.is_none() && run.summary.headless {
        world.write_message(AppExit::Success);
    }
    world.insert_resource(run);
}

fn finish_on_exit(world: &mut World) {
    if world.resource::<Messages<AppExit>>().is_empty() {
        return;
    }
    let mut run = world.remove_resource::<BattleRun>().unwrap();
    if run.summary.status == RunStatus::InProgress && run.error.is_none() {
        run.summary.status = RunStatus::Interrupted;
        world.resource_mut::<BattleCounters>().frozen = true;
        let sample = snapshot(world, &mut run);
        if let Err(error) = run.write_sample(sample) {
            error!("Cannot save interrupted battle: {error}");
            run.error = Some(error.to_string());
            world.write_message(AppExit::error());
        }
    }
    world.insert_resource(run);
}

fn collect_frame(mut run: ResMut<BattleRun>, time: Res<Time<Virtual>>) {
    let now = Instant::now();
    if run.summary.records_frame_timing()
        && run.summary.status == RunStatus::InProgress
        && !time.is_paused()
        && let Some(previous) = run.frame_start
    {
        run.frame_samples
            .push(now.duration_since(previous).as_secs_f64() * 1000.);
    }
    run.frame_start = Some(now);
}
