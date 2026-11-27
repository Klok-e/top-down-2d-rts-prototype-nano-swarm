use std::collections::VecDeque;

use crate::{resources::WorkerDeliveryTotals, strategic_controller::MineObservation};

const TICKS_PER_SECOND: u64 = crate::SIMULATION_HZ as u64;
const OBSERVATION_TICKS: u64 = 12 * TICKS_PER_SECOND;
const SERVICE_CYCLE_TICKS: u64 = 3 * TICKS_PER_SECOND;
const MIN_DROPOFF_SPACING_TICKS: u64 = SERVICE_CYCLE_TICKS;

#[derive(Default)]
pub(super) struct MineHistory {
    samples: VecDeque<Sample>,
    worker_ticks: u64,
    deliveries: WorkerDeliveryTotals,
    last_dropoff: Option<u64>,
}

#[derive(Clone, Copy)]
struct Sample {
    tick: u64,
    worker_ticks: u64,
    deliveries: WorkerDeliveryTotals,
}

impl MineHistory {
    pub(super) fn observe(&mut self, tick: u64, workers: u32, deliveries: WorkerDeliveryTotals) {
        if deliveries.completed > self.deliveries.completed {
            self.last_dropoff = Some(tick);
        }
        self.deliveries = deliveries;
        self.worker_ticks += u64::from(workers);
        if self
            .samples
            .back()
            .is_none_or(|sample| tick.saturating_sub(sample.tick) >= TICKS_PER_SECOND)
        {
            self.samples.push_back(Sample {
                tick,
                worker_ticks: self.worker_ticks,
                deliveries,
            });
        }
        while self.samples.len() > 1 && tick - self.samples[1].tick >= OBSERVATION_TICKS {
            self.samples.pop_front();
        }
    }

    pub(super) fn recent(&self, tick: u64) -> (MineObservation, u32) {
        let start = self.samples[0];
        let observation_ticks = tick - start.tick;
        let middle = self.sample_at_least_as_old_as(tick, OBSERVATION_TICKS / 2);
        let cycle_start = self.sample_at_least_as_old_as(tick, SERVICE_CYCLE_TICKS);
        let cycle_deliveries = self.deliveries.minerals - cycle_start.deliveries.minerals;
        let active_workers = (self.worker_ticks - start.worker_ticks + observation_ticks / 2)
            .checked_div(observation_ticks)
            .unwrap_or_default();
        let sustained_dropoffs = observation_ticks >= OBSERVATION_TICKS
            && middle.deliveries.completed > start.deliveries.completed
            && self.last_dropoff.is_some_and(|last| {
                last > middle.tick
                    && self
                        .sample_at_least_as_old_as(last, MIN_DROPOFF_SPACING_TICKS)
                        .deliveries
                        .completed
                        > start.deliveries.completed
            });
        let observation = MineObservation {
            active_workers: active_workers as u32,
            delivered_minerals: self.deliveries.minerals - start.deliveries.minerals,
            completed_dropoffs: (self.deliveries.completed - start.deliveries.completed) as u32,
            observation_ticks,
            ticks_since_dropoff: self.last_dropoff.map(|dropoff| tick - dropoff),
            sustained_dropoffs,
            ..Default::default()
        };
        (
            observation,
            cycle_deliveries.min(u64::from(u32::MAX)) as u32,
        )
    }

    fn sample_at_least_as_old_as(&self, tick: u64, age: u64) -> Sample {
        self.samples
            .iter()
            .rev()
            .find(|sample| tick.saturating_sub(sample.tick) >= age)
            .copied()
            .unwrap_or(self.samples[0])
    }
}
