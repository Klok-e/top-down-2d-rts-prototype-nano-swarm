use super::CELL_WIDTH;
use crate::nanobot::SwarmId;
use bevy::{
    platform::collections::{HashMap, HashSet},
    prelude::*,
};
use std::sync::Arc;

#[derive(Default)]
pub(crate) struct TrafficSnapshot {
    swarms: HashMap<SwarmId, HashMap<IVec2, usize>>,
    visits: HashSet<(Entity, IVec2)>,
}

impl TrafficSnapshot {
    pub(crate) fn record_route(
        &mut self,
        entity: Entity,
        swarm: SwarmId,
        mut start: Vec2,
        waypoints: &[Vec2],
        mut remaining: f32,
        mut segment_clear: impl FnMut(Vec2, Vec2) -> bool,
    ) {
        for &waypoint in waypoints {
            let length = start.distance(waypoint);
            if length <= 0.0 {
                continue;
            }
            let end = start.lerp(waypoint, (remaining / length).min(1.0));
            if !segment_clear(start, end) {
                break;
            }
            for (cell, _) in segment_cells(start, end) {
                if self.visits.insert((entity, cell)) {
                    *self
                        .swarms
                        .entry(swarm)
                        .or_default()
                        .entry(cell)
                        .or_default() += 1;
                }
            }
            remaining -= length;
            if remaining <= 0.0 {
                break;
            }
            start = waypoint;
        }
    }

    pub(crate) fn for_route(
        self: &Arc<Self>,
        owner: Entity,
        swarm: SwarmId,
    ) -> Option<RouteTraffic> {
        self.swarms.contains_key(&swarm).then(|| RouteTraffic {
            snapshot: self.clone(),
            owner,
            swarm,
        })
    }
}

#[derive(Clone)]
pub(crate) struct RouteTraffic {
    snapshot: Arc<TrafficSnapshot>,
    owner: Entity,
    swarm: SwarmId,
}

impl RouteTraffic {
    pub(super) fn surcharge(&self, cell: IVec2) -> f32 {
        let count = self.snapshot.swarms[&self.swarm]
            .get(&cell)
            .copied()
            .unwrap_or(0);
        let own = usize::from(self.snapshot.visits.contains(&(self.owner, cell)));
        count.saturating_sub(own).min(3) as f32 * 0.05
    }
}

/// Enumerate positive lengths in fine cells. On a grid line, the half-open cell
/// convention assigns the entire segment to one side in either travel direction.
pub(super) fn segment_cells(start: Vec2, end: Vec2) -> impl Iterator<Item = (IVec2, f32)> {
    let origin = start.as_dvec2();
    let delta = end.as_dvec2() - origin;
    let length = delta.length();
    let width = f64::from(CELL_WIDTH);
    let mut cell = (origin / width).floor().as_ivec2();
    let step = delta.signum().as_ivec2();
    let mut fraction = 0.0;
    std::iter::from_fn(move || {
        while fraction < 1.0 && length > 0.0 {
            let upper = (cell + step.max(IVec2::ZERO)).as_dvec2() * width;
            let offset = (upper - origin) / delta;
            let x = if delta.x == 0.0 {
                f64::INFINITY
            } else {
                offset.x
            };
            let y = if delta.y == 0.0 {
                f64::INFINITY
            } else {
                offset.y
            };
            let crossing = x.min(y).min(1.0);
            let travelled = ((crossing - fraction) * length) as f32;
            let traversed_cell = cell;
            fraction = crossing;
            if x <= crossing {
                cell.x += step.x;
            }
            if y <= crossing {
                cell.y += step.y;
            }
            if travelled > 0.0 {
                return Some((traversed_cell, travelled));
            }
        }
        None
    })
}
