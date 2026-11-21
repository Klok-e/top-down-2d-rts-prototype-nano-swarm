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
    points: Vec<IVec2>,
    active: usize,
    pub(super) block: DefendBlock,
}

impl StagingFront {
    pub(super) fn point(&self) -> IVec2 {
        self.points[self.active]
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
        let destination = *self
            .points
            .last()
            .expect("a staging front has a destination");
        target == destination
            || if self.active + 1 == self.points.len() {
                self.covers(target, owner, state)
            } else {
                DefendBlock::new(destination, PURSUIT_AREA_CELLS).contains(target)
            }
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
        for bot in state.bots.iter().take(MAX_BOTS) {
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
    if let Some(previous) = previous
        && current.is_some_and(|plan| plan.objective == candidate.objective)
        && previous.destination_contains(target, owner, state)
        && previous.points[previous.active..]
            .iter()
            .all(|&point| ground(owner, state, point).is_some())
    {
        let mut retained = previous.clone();
        retained.advance_if_arrived(
            owner,
            state,
            tuning.strategy_staging_arrival_fraction,
            budget,
        );
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
    let start_cell = world_to_intent_cell(start);
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
    let mut points = staging_points(
        start,
        &waypoints,
        tuning.strategy_staging_spacing_cells,
        budget,
    )?;
    points.retain(|&cell| {
        cell != start_cell && cell != target && ground(owner, state, cell).is_some()
    });
    points.push(target);
    let mut staging = StagingFront {
        points,
        active: 0,
        block: DefendBlock::new(target, 1),
    };
    if let Some(previous) = previous
        && current.is_some_and(|plan| plan.objective == candidate.objective)
        && !previous.destination_contains(target, owner, state)
        && budget.spend(1)
        && ground(owner, state, start_cell).is_some()
        && previous.active + 1 != previous.points.len()
    {
        staging.points.insert(0, start_cell);
        staging.block = previous.block;
        staging.advance_if_arrived(
            owner,
            state,
            tuning.strategy_staging_arrival_fraction,
            budget,
        );
    }
    Some(staging)
}

fn ground(owner: SwarmId, state: &GameState<'_>, cell: IVec2) -> Option<Vec2> {
    if let Some(map) = state.reachability {
        return map.reachable_point(owner, cell, state.grid);
    }
    state.grid.in_bounds(cell).then(|| intent_cell_center(cell))
}

fn staging_points(
    start: Vec2,
    waypoints: &[Vec2],
    spacing_cells: u32,
    budget: &mut WorkBudget,
) -> Option<Vec<IVec2>> {
    if !budget.spend(waypoints.len()) {
        return None;
    }
    let segments = std::iter::once(start)
        .chain(waypoints.iter().copied())
        .zip(waypoints.iter().copied());
    let length = segments.clone().map(|(a, b)| a.distance(b)).sum::<f32>();
    let spacing = spacing_cells.max(1) as f32 * crate::ZONE_BLOCK_SIZE;
    let count = ((length / spacing).ceil() as usize).clamp(1, MAX_STAGING_POINTS);
    let step = length / count as f32;
    let mut points = Vec::with_capacity(count);
    let mut traveled = 0.0;
    for (from, to) in segments {
        let segment_length = from.distance(to);
        let mut next = (points.len() + 1) as f32 * step;
        while points.len() + 1 < count && next <= traveled + segment_length {
            let fraction = (next - traveled) / segment_length;
            points.push(world_to_intent_cell(from.lerp(to, fraction)));
            next += step;
        }
        traveled += segment_length;
    }
    points.dedup();
    Some(points)
}
