use super::CELL_WIDTH;
use crate::nanobot::SwarmId;
use bevy::{platform::collections::HashMap, prelude::*};
use std::sync::Arc;

#[derive(Default)]
pub(crate) struct TrafficSnapshot {
    swarms: HashMap<SwarmId, HashMap<IVec2, CellTraffic>>,
}

#[derive(Default)]
struct CellTraffic {
    travellers: [Option<Entity>; 3],
    saturated: bool,
}

impl CellTraffic {
    fn record(&mut self, entity: Entity) {
        if self.saturated || self.travellers.contains(&Some(entity)) {
            return;
        }
        if let Some(slot) = self.travellers.iter_mut().find(|slot| slot.is_none()) {
            *slot = Some(entity);
        } else {
            // Four distinct travellers leave at least three others for every owner.
            self.saturated = true;
        }
    }

    fn surcharge(&self, owner: Entity) -> f32 {
        let count = if self.saturated {
            3
        } else {
            self.travellers
                .iter()
                .flatten()
                .filter(|&&entity| entity != owner)
                .count()
        };
        count as f32 * 0.05
    }
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
        let cells = self.swarms.entry(swarm).or_default();
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
                cells.entry(cell).or_default().record(entity);
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
        self.swarms
            .get(&swarm)
            .filter(|cells| !cells.is_empty())
            .map(|_| RouteTraffic {
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
        self.snapshot.swarms[&self.swarm]
            .get(&cell)
            .map_or(0.0, |traffic| traffic.surcharge(self.owner))
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
