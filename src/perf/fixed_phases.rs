use std::{any::TypeId, collections::HashMap, time::Duration};

use crate::{
    nanobot::{NanobotSimulationSet, RegionalAllocationSet},
    strategic_runtime::StrategicControllerSet,
};
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

use super::{FixedPhaseTimes, RecorderClock, elapsed_ns, recorder::FixedPhase};

fn measured_set(phase: FixedPhase) -> Option<InternedSystemSet> {
    Some(match phase {
        FixedPhase::Movement => NanobotSimulationSet::Movement.intern(),
        FixedPhase::Threat => NanobotSimulationSet::Threat.intern(),
        FixedPhase::Combat => NanobotSimulationSet::Combat.intern(),
        FixedPhase::Maintenance => NanobotSimulationSet::Maintenance.intern(),
        FixedPhase::AllocationProject => RegionalAllocationSet::Project.intern(),
        FixedPhase::AllocationInvalidate => RegionalAllocationSet::Invalidate.intern(),
        FixedPhase::AllocationAcquire => RegionalAllocationSet::Acquire.intern(),
        FixedPhase::StrategicController => StrategicControllerSet.intern(),
        FixedPhase::Other => return None,
    })
}

#[derive(Resource, Default)]
pub(super) struct FixedMeasurements {
    tick_start: Duration,
    segment_start: Duration,
    phase: FixedPhase,
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

    fn transition(&mut self, next: FixedPhase, now: Duration) {
        self.times
            .add(self.phase, elapsed_ns(self.segment_start, now));
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
                fixed.phase = FixedPhase::Other;
            },
        )
        .add_systems(
            TickBoundary::End,
            |clock: Res<RecorderClock>, mut fixed: ResMut<FixedMeasurements>| {
                let now = clock.now();
                fixed.transition(FixedPhase::Other, now);
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
    markers: HashMap<(FixedPhase, usize), SystemKey>,
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
        let segments = fixed_segments(graph, dependency);
        let mut used = [0; FixedPhase::ALL.len()];
        let mut previous_phase = FixedPhase::Other;
        let mut previous_systems = Vec::new();
        for (phase, systems) in segments
            .into_iter()
            .chain([(FixedPhase::Other, Vec::new())])
        {
            if phase != previous_phase {
                // Reuse the phase's markers when the schedule is rebuilt with a different span order.
                let used = &mut used[phase as usize];
                let marker = *self.markers.entry((phase, *used)).or_insert_with(|| {
                    graph.systems.insert(
                        Box::new(IntoSystem::into_system(
                            move |clock: Res<RecorderClock>, mut fixed: ResMut<FixedMeasurements>| {
                                fixed.transition(phase, clock.now());
                            },
                        )),
                        Vec::new(),
                    )
                });
                *used += 1;
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
) -> Vec<(FixedPhase, Vec<SystemKey>)> {
    let mut phases = HashMap::new();
    for phase in FixedPhase::ALL {
        let Some(set) = measured_set(phase).and_then(|set| graph.system_sets.get_key(set)) else {
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
    // Charge a generated flush to its named consumer phase when no other
    // named phase depends directly on it. This avoids a separate timed span.
    let flush_phases: Vec<_> = dependency
        .nodes()
        .filter(|&system| {
            !graph
                .hierarchy()
                .graph()
                .contains_node(NodeId::System(system))
                && graph.systems[system].type_id() == TypeId::of::<ApplyDeferred>()
        })
        .filter_map(|system| {
            let mut consumers = dependency
                .neighbors(system)
                .filter_map(|node| phases.get(&node).copied());
            let phase = consumers.next()?;
            consumers
                .all(|other| other == phase)
                .then_some((system, phase))
        })
        .collect();
    phases.extend(flush_phases);
    let phase_of = |system: SystemKey| phases.get(&system).copied().unwrap_or_default();

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
    let mut segments: Vec<(FixedPhase, Vec<SystemKey>)> = Vec::new();
    while !ready.is_empty() {
        let phase = segments
            .last()
            .map_or(FixedPhase::Other, |(phase, _)| *phase);
        let next = ready
            .iter()
            .position(|&system| phase_of(system) == phase)
            .or_else(|| {
                ready
                    .iter()
                    .position(|&system| phase_of(system) != FixedPhase::Other)
            })
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
