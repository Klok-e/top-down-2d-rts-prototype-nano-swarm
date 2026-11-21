use std::sync::Arc;

use super::{
    lifecycle::WorkBudget,
    materialize::{intent_cell_center, query_route, world_to_intent_cell},
    *,
};

use crate::navigation::RouteGoal;

const MAX_STAGING_POINTS: usize = 6;
const PURSUIT_AREA_CELLS: usize = 9;

#[derive(Debug, Clone)]
pub(super) struct StagingFront {
    route: Arc<[Vec2]>,
    points: Vec<StagingPoint>,
    active: usize,
    spacing_cells: u32,
    pub(super) block: DefendBlock,
}

#[derive(Debug, Clone, Copy)]
struct StagingPoint {
    cell: IVec2,
    distance: f32,
}

impl StagingFront {
    pub(super) fn point(&self) -> IVec2 {
        self.points[self.active].cell
    }

    pub(super) fn covers(&self, target: IVec2, owner: SwarmId, state: &GameState<'_>) -> bool {
        self.block.contains(target)
            && state
                .grid
                .cell(target)
                .is_some_and(|cell| cell.has_owned(IntentKind::Defend, owner))
    }

    pub(super) fn destination_contains(
        &self,
        target: IVec2,
        owner: SwarmId,
        state: &GameState<'_>,
    ) -> bool {
        let destination = self.destination();
        target == destination
            || if self.active + 1 == self.points.len() {
                self.covers(target, owner, state)
            } else {
                DefendBlock::new(destination, PURSUIT_AREA_CELLS).contains(target)
            }
    }

    fn destination(&self) -> IVec2 {
        self.points
            .last()
            .expect("a staging front has a destination")
            .cell
    }

    fn advance_if_arrived(
        &mut self,
        owner: SwarmId,
        state: &GameState<'_>,
        fraction: f32,
        budget: &mut WorkBudget,
    ) {
        if self.active + 1 == self.points.len() {
            return;
        }
        let mut arrived = 0;
        let mut living = 0;
        for bot in state.bots {
            if !budget.spend(1) {
                return;
            }
            if bot.owner == owner && bot.kind == NanobotType::Defender && bot.health > 0 {
                living += 1;
                if bot.position.is_finite()
                    && self.covers(world_to_intent_cell(bot.position), owner, state)
                {
                    arrived += 1;
                }
            }
        }
        if living > 0 && arrived as f32 >= living as f32 * fraction {
            self.active += 1;
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct DefendBlock {
    min: IVec2,
    size: IVec2,
}

impl DefendBlock {
    pub(super) fn new(center: IVec2, cell_limit: usize) -> Self {
        let cell_limit = cell_limit.max(1);
        let root = cell_limit.isqrt();
        let width = root + usize::from(root * root < cell_limit);
        let height = cell_limit / width;
        let size = IVec2::new(width as i32, height as i32);
        Self {
            min: center - size / 2,
            size,
        }
    }

    pub(super) fn contains(self, cell: IVec2) -> bool {
        let offset = cell - self.min;
        offset.cmpge(IVec2::ZERO).all() && offset.cmplt(self.size).all()
    }

    pub(super) fn avoiding(mut self, anchor: IVec2, protected: Option<IVec2>) -> Self {
        let Some(cell) = protected.filter(|&cell| self.contains(cell)) else {
            return self;
        };
        let offsets = (0..2)
            .filter(|&axis| anchor[axis] != cell[axis])
            .map(|axis| {
                let mut offset = IVec2::ZERO;
                offset[axis] = if anchor[axis] > cell[axis] {
                    cell[axis] - self.min[axis] + 1
                } else {
                    cell[axis] - self.min[axis] - self.size[axis]
                };
                offset
            });
        if let Some(offset) = offsets.min_by_key(|offset| offset.abs().element_sum()) {
            self.min += offset;
        }
        self
    }

    pub(super) fn cells(self) -> impl Iterator<Item = IVec2> {
        (0..self.size.y)
            .flat_map(move |y| (0..self.size.x).map(move |x| self.min + IVec2::new(x, y)))
    }
}

pub(super) fn plan_staging(
    tuning: &AutonomyTuning,
    candidate: &PlanCandidate,
    current: Option<&IntentPlan>,
    owner: SwarmId,
    state: &GameState<'_>,
    budget: &mut WorkBudget,
) -> Option<StagingFront> {
    let target = candidate.attack_target?;
    let previous = current.and_then(|plan| plan.staging.as_ref());
    let same_objective = current.is_some_and(|plan| plan.objective == candidate.objective);
    let spacing = tuning.strategy_staging_spacing_cells;
    let arrival_fraction = tuning.strategy_staging_arrival_fraction;
    if let Some(previous) = previous
        && same_objective
        && previous.destination_contains(target, owner, state)
        && previous.points[previous.active..]
            .iter()
            .all(|point| stageable(owner, state, candidate.protected_home, point.cell))
    {
        let mut retained = previous.clone();
        if previous.spacing_cells != spacing && previous.active + 1 < previous.points.len() {
            retained.points = staging_points(
                &previous.route,
                Some(previous.points[previous.active]),
                previous.destination(),
                spacing,
                budget,
            )?;
            retained
                .points
                .retain(|point| stageable(owner, state, candidate.protected_home, point.cell));
            retained.active = 0;
        }
        retained.spacing_cells = spacing;
        retained.advance_if_arrived(owner, state, arrival_fraction, budget);
        return Some(retained);
    }

    let start = previous.map_or_else(
        || {
            state
                .swarms
                .iter()
                .find(|swarm| budget.spend(1) && swarm.id == owner)
                .map_or(Vec2::ZERO, |swarm| swarm.home)
        },
        |previous| {
            let cell = previous.point();
            ground(owner, state, cell).unwrap_or_else(|| intent_cell_center(cell))
        },
    );
    let destination = ground(owner, state, target)?;
    let waypoints = if state.route_query.is_some() {
        query_route(
            state,
            start,
            RouteGoal::Range {
                center: destination,
                radius: crate::ZONE_BLOCK_SIZE / 4.0,
            },
            budget,
        )?
        .waypoints
    } else {
        vec![destination]
    };
    let route = Arc::from(std::iter::once(start).chain(waypoints).collect::<Vec<_>>());
    let held_stage = previous.filter(|previous| {
        same_objective
            && previous.active + 1 < previous.points.len()
            && ground(owner, state, previous.point()).is_some()
    });
    let first = held_stage.map(|stage| StagingPoint {
        cell: stage.point(),
        distance: 0.0,
    });
    let mut points = staging_points(&route, first, target, spacing, budget)?;
    points.retain(|point| stageable(owner, state, candidate.protected_home, point.cell));
    let mut staging = StagingFront {
        route,
        points,
        active: 0,
        spacing_cells: spacing,
        block: held_stage.map_or_else(|| DefendBlock::new(target, 1), |stage| stage.block),
    };
    if held_stage.is_some() {
        staging.advance_if_arrived(owner, state, arrival_fraction, budget);
    }
    Some(staging)
}

fn stageable(owner: SwarmId, state: &GameState<'_>, protected: Option<IVec2>, cell: IVec2) -> bool {
    protected != Some(cell) && ground(owner, state, cell).is_some()
}

fn ground(owner: SwarmId, state: &GameState<'_>, cell: IVec2) -> Option<Vec2> {
    if let Some(map) = state.reachability {
        return map.reachable_point(owner, cell, state.grid);
    }
    state.grid.in_bounds(cell).then(|| intent_cell_center(cell))
}

fn staging_points(
    route: &[Vec2],
    first: Option<StagingPoint>,
    target: IVec2,
    spacing_cells: u32,
    budget: &mut WorkBudget,
) -> Option<Vec<StagingPoint>> {
    if !budget.spend(route.len().saturating_mul(2)) {
        return None;
    }
    let segments = route.windows(2);
    let length = segments
        .clone()
        .map(|segment| segment[0].distance(segment[1]))
        .sum::<f32>();
    let already_traveled = first.map_or(0.0, |point| point.distance);
    let spacing = spacing_cells.max(1) as f32 * crate::ZONE_BLOCK_SIZE;
    let limit = MAX_STAGING_POINTS - usize::from(first.is_some());
    let count = (((length - already_traveled) / spacing).ceil() as usize).clamp(1, limit);
    let step = (length - already_traveled) / count as f32;
    let origin = first.map_or_else(|| world_to_intent_cell(route[0]), |point| point.cell);
    let mut points = Vec::with_capacity(count + usize::from(first.is_some()));
    points.extend(first);
    let mut traveled = 0.0;
    let mut sample = 1;
    for segment in segments {
        let [from, to] = [segment[0], segment[1]];
        let segment_length = from.distance(to);
        while sample < count {
            let next = already_traveled + sample as f32 * step;
            if next > traveled + segment_length {
                break;
            }
            let fraction = (next - traveled) / segment_length;
            let cell = world_to_intent_cell(from.lerp(to, fraction));
            if cell != origin
                && cell != target
                && points.last().is_none_or(|point| point.cell != cell)
            {
                points.push(StagingPoint {
                    cell,
                    distance: next,
                });
            }
            sample += 1;
        }
        traveled += segment_length;
    }
    points.push(StagingPoint {
        cell: target,
        distance: length,
    });
    Some(points)
}
