use std::{collections::HashMap, time::Duration};

use bevy::{
    app::FixedMainScheduleOrder,
    ecs::schedule::{
        InternedSystemSet, NodeId, ScheduleBuildError, ScheduleBuildPass, ScheduleGraph,
        ScheduleLabel, SystemKey, SystemSetKey,
        graph::{Dag, DiGraph, Direction},
    },
    platform::hash::FixedHasher,
    prelude::*,
};
use indexmap::IndexSet;
use serde::{Serialize, Serializer, ser::SerializeMap};

use crate::{
    nanobot::{NanobotSimulationSet, RegionalAllocationSet},
    strategic_runtime::StrategicControllerSet,
};

use super::{RecorderClock, elapsed_ns};

const PHASE_NAMES: [&str; 9] = [
    "movement",
    "threat",
    "combat",
    "maintenance",
    "allocation_project",
    "allocation_invalidate",
    "allocation_acquire",
    "strategic_controller",
    "fixed_other",
];
const FIXED_OTHER: usize = PHASE_NAMES.len() - 1;

fn named_phases() -> [InternedSystemSet; FIXED_OTHER] {
    [
        NanobotSimulationSet::Movement.intern(),
        NanobotSimulationSet::Threat.intern(),
        NanobotSimulationSet::Combat.intern(),
        NanobotSimulationSet::Maintenance.intern(),
        RegionalAllocationSet::Project.intern(),
        RegionalAllocationSet::Invalidate.intern(),
        RegionalAllocationSet::Acquire.intern(),
        StrategicControllerSet.intern(),
    ]
}

#[derive(Debug, Default, Clone, Copy)]
pub struct FixedPhaseTimes([u64; Self::COUNT]);

impl FixedPhaseTimes {
    pub(crate) const COUNT: usize = PHASE_NAMES.len();

    pub(crate) fn iter(&self) -> impl ExactSizeIterator<Item = (&'static str, u64)> + '_ {
        PHASE_NAMES.into_iter().zip(self.0.iter().copied())
    }

    pub fn total(self) -> u64 {
        self.iter().map(|(_, time)| time).sum()
    }
}

impl Serialize for FixedPhaseTimes {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(Self::COUNT))?;
        for (phase, time) in self.iter() {
            map.serialize_entry(phase, &time)?;
        }
        map.end()
    }
}

#[derive(Resource, Default)]
pub(super) struct FixedMeasurements {
    tick_start: Duration,
    segment_start: Duration,
    phase: usize,
    times: FixedPhaseTimes,
    longest_tick_ns: u64,
}

impl FixedMeasurements {
    pub(super) fn take_frame(&mut self) -> (FixedPhaseTimes, u64) {
        (
            std::mem::take(&mut self.times),
            std::mem::take(&mut self.longest_tick_ns),
        )
    }

    fn transition(&mut self, next: usize, now: Duration) {
        self.times.0[self.phase] += elapsed_ns(self.segment_start, now);
        self.phase = next;
        self.segment_start = now;
    }
}

#[derive(ScheduleLabel, Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum TickBoundary {
    Start,
    End,
}

pub(super) fn install(app: &mut App) {
    app.init_resource::<FixedMeasurements>()
        .add_systems(
            TickBoundary::Start,
            |clock: Res<RecorderClock>, mut fixed: ResMut<FixedMeasurements>| {
                let now = clock.now();
                fixed.tick_start = now;
                fixed.segment_start = now;
                fixed.phase = FIXED_OTHER;
            },
        )
        .add_systems(
            TickBoundary::End,
            |clock: Res<RecorderClock>, mut fixed: ResMut<FixedMeasurements>| {
                let now = clock.now();
                fixed.transition(FIXED_OTHER, now);
                let tick_ns = elapsed_ns(fixed.tick_start, now);
                fixed.longest_tick_ns = fixed.longest_tick_ns.max(tick_ns);
            },
        )
        .edit_schedule(FixedUpdate, |schedule| {
            schedule.add_build_pass(PhaseBoundaries::default());
        });
    let mut order = app.world_mut().resource_mut::<FixedMainScheduleOrder>();
    order.labels.insert(0, TickBoundary::Start.intern());
    order.labels.push(TickBoundary::End.intern());
}

#[derive(Debug, Default)]
struct PhaseBoundaries {
    markers: HashMap<(usize, usize), SystemKey>,
}

impl ScheduleBuildPass for PhaseBoundaries {
    type EdgeOptions = ();

    fn add_dependency(&mut self, _: NodeId, _: NodeId, _: Option<&()>) {}

    fn collapse_set(
        &mut self,
        _: SystemSetKey,
        _: &IndexSet<SystemKey, FixedHasher>,
        _: &DiGraph<NodeId>,
    ) -> impl Iterator<Item = (NodeId, NodeId)> {
        std::iter::empty()
    }

    fn build(
        &mut self,
        world: &mut World,
        graph: &mut ScheduleGraph,
        dependency: &mut Dag<SystemKey>,
    ) -> Result<(), ScheduleBuildError> {
        // Keep the gameplay dependencies and Bevy's deferred command flushes.
        // Adjacent systems in a span retain their existing ability to run in parallel.
        let segments = fixed_segments(graph, dependency);
        let mut previous_phase = FIXED_OTHER;
        let mut previous_systems = Vec::new();
        for (index, (phase, systems)) in segments
            .into_iter()
            .chain([(FIXED_OTHER, Vec::new())])
            .enumerate()
        {
            if phase != previous_phase {
                let marker = *self.markers.entry((index, phase)).or_insert_with(|| {
                    graph.systems.insert(
                        Box::new(IntoSystem::into_system(
                            move |clock: Res<RecorderClock>, mut fixed: ResMut<FixedMeasurements>| {
                                fixed.transition(phase, clock.now());
                            },
                        )),
                        Vec::new(),
                    )
                });
                for system in previous_systems {
                    dependency.add_edge(system, marker);
                }
                for &system in &systems {
                    dependency.add_edge(marker, system);
                }
            }
            previous_phase = phase;
            previous_systems = systems;
        }
        graph.systems.initialize(world);
        Ok(())
    }
}

fn fixed_segments(
    graph: &ScheduleGraph,
    dependency: &Dag<SystemKey>,
) -> Vec<(usize, Vec<SystemKey>)> {
    let mut phases = HashMap::new();
    for (phase, set) in named_phases().into_iter().enumerate() {
        let Some(set) = graph.system_sets.get_key(set) else {
            continue;
        };
        let mut nodes = vec![NodeId::Set(set)];
        while let Some(node) = nodes.pop() {
            match node {
                NodeId::System(system) => {
                    phases.insert(system, phase);
                }
                NodeId::Set(_) => nodes.extend(graph.hierarchy().neighbors(node)),
            }
        }
    }
    let phase_of = |system: SystemKey| phases.get(&system).copied().unwrap_or(FIXED_OTHER);

    // Topologically group ready systems by phase. Work without a phase can run
    // on either side of a boundary, subject to its existing prerequisites.
    let mut waiting: HashMap<_, _> = dependency
        .nodes()
        .map(|system| {
            let count = dependency
                .neighbors_directed(system, Direction::Incoming)
                .count();
            (system, count)
        })
        .collect();
    let mut ready: Vec<_> = dependency
        .nodes()
        .filter(|system| waiting[system] == 0)
        .collect();
    let mut segments: Vec<(usize, Vec<SystemKey>)> = Vec::new();
    while !ready.is_empty() {
        let phase = segments.last().map_or(FIXED_OTHER, |(phase, _)| *phase);
        let next = ready
            .iter()
            .position(|&system| phase_of(system) == phase)
            .unwrap_or(0);
        let system = ready.remove(next);
        let phase = phase_of(system);
        if segments.last().is_none_or(|(current, _)| *current != phase) {
            segments.push((phase, Vec::new()));
        }
        segments.last_mut().unwrap().1.push(system);
        for successor in dependency.neighbors(system) {
            let count = waiting.get_mut(&successor).unwrap();
            *count -= 1;
            if *count == 0 {
                ready.push(successor);
            }
        }
    }
    segments
}
