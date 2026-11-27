use std::sync::Arc;

use super::{
    lifecycle::WorkBudget,
    materialize::{intent_cell_center, query_route, world_to_intent_cell},
    raid::{READINESS_GRACE_TICKS, RaidForces},
    *,
};

use crate::navigation::RouteGoal;

const MAX_STAGING_POINTS: usize = 6;
pub(super) const MINIMUM_STAGING_BLOCK_CELLS: usize = 9;

#[derive(Debug, Clone)]
pub(super) struct StagingFront {
    route: Arc<[Vec2]>,
    points: Vec<StagingPoint>,
    active: usize,
    spacing_cells: u32,
    readiness_shortfall_started: Option<u64>,
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

    pub(super) fn waiting_for_readiness(&self) -> bool {
        self.readiness_shortfall_started.is_some()
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
            || if self.at_destination() {
                self.covers(target, owner, state)
            } else {
                DefendBlock::new(destination, MINIMUM_STAGING_BLOCK_CELLS).contains(target)
            }
    }

    fn at_destination(&self) -> bool {
        self.active + 1 == self.points.len()
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
        if !self.at_destination() && self.defenders_arrived(owner, state, fraction, budget) {
            self.active += 1;
        }
    }

    pub(super) fn destination_secured(
        &self,
        owner: SwarmId,
        state: &GameState<'_>,
        fraction: f32,
        budget: &mut WorkBudget,
    ) -> bool {
        self.at_destination() && self.defenders_arrived(owner, state, fraction, budget)
    }

    fn defenders_arrived(
        &self,
        owner: SwarmId,
        state: &GameState<'_>,
        fraction: f32,
        budget: &mut WorkBudget,
    ) -> bool {
        let mut arrived = 0;
        let mut assigned = 0;
        for bot in state.bots {
            if !budget.spend(1) {
                return false;
            }
            if bot.owner == owner
                && bot.kind == NanobotType::Defender
                && bot.health > 0
                && let DefenderDuty::StagingAt(destination) = bot.defender_duty
                && self.covers(destination, owner, state)
            {
                assigned += 1;
                if bot.position.is_finite()
                    && self.covers(world_to_intent_cell(bot.position), owner, state)
                {
                    arrived += 1;
                }
            }
        }
        assigned > 0 && arrived as f32 >= assigned as f32 * fraction
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    raid_forces: &RaidForces<'_>,
    budget: &mut WorkBudget,
) -> Option<StagingFront> {
    let target = candidate.attack_target?;
    let previous = current.and_then(|plan| plan.staging.as_ref());
    let same_objective = current.is_some_and(|plan| plan.objective == candidate.objective);
    let continuing = previous.filter(|front| {
        same_objective
            && stageable(
                owner,
                state,
                candidate.protected_home,
                front.point(),
                budget,
            )
    });
    let requires_readiness = candidate.objective.requires_raid_readiness();
    if requires_readiness && continuing.is_none() && !raid_forces.can_launch() {
        return None;
    }
    let reusable = continuing.filter(|front| {
        (!requires_readiness || front.at_destination())
            && front.destination_contains(target, owner, state)
            && front.points[front.active + 1..]
                .iter()
                .all(|point| stageable(owner, state, candidate.protected_home, point.cell, budget))
    });

    let route = if requires_readiness || reusable.is_none() {
        let active_raid = requires_readiness && continuing.is_some();
        let route = staging_route(candidate, previous, active_raid, owner, state, budget)?;
        let response_supports_front = |position| {
            continuing
                .is_some_and(|front| front.covers(world_to_intent_cell(position), owner, state))
        };
        if requires_readiness
            && !raid_forces.ready(
                &route,
                intent_cell_center(target),
                response_supports_front,
                budget,
            )?
        {
            let mut held = continuing?.clone();
            let shortfall_started = held.readiness_shortfall_started.unwrap_or(state.tick);
            if state.tick.saturating_sub(shortfall_started) >= READINESS_GRACE_TICKS {
                return None;
            }
            held.readiness_shortfall_started = Some(shortfall_started.min(state.tick));
            return Some(held);
        }
        Some(route)
    } else {
        None
    };

    let spacing = tuning.strategy_staging_spacing_cells;
    let arrival_fraction = tuning.strategy_staging_arrival_fraction;
    if let Some(previous) = reusable {
        let mut retained = previous.clone();
        if previous.spacing_cells != spacing && !previous.at_destination() {
            retained.points = staging_points(
                &previous.route,
                Some(previous.points[previous.active]),
                previous.destination(),
                spacing,
                budget,
            )?;
            retained.points.retain(|point| {
                stageable(owner, state, candidate.protected_home, point.cell, budget)
            });
            if retained.points.is_empty() || budget.exhausted {
                return None;
            }
            retained.active = 0;
        }
        retained.spacing_cells = spacing;
        retained.readiness_shortfall_started = None;
        retained.advance_if_arrived(owner, state, arrival_fraction, budget);
        return Some(retained);
    }

    let route = route?;
    let held_stage = continuing.filter(|old| !old.at_destination());
    let first = held_stage.map(|stage| StagingPoint {
        cell: stage.point(),
        distance: 0.0,
    });
    let mut points = staging_points(&route, first, target, spacing, budget)?;
    points.retain(|point| stageable(owner, state, candidate.protected_home, point.cell, budget));
    if points.is_empty() || budget.exhausted {
        return None;
    }
    let mut staging = StagingFront {
        route,
        points,
        active: 0,
        spacing_cells: spacing,
        readiness_shortfall_started: None,
        block: held_stage.map_or_else(|| DefendBlock::new(target, 1), |stage| stage.block),
    };
    if held_stage.is_some() {
        staging.advance_if_arrived(owner, state, arrival_fraction, budget);
    }
    Some(staging)
}

fn staging_route(
    candidate: &PlanCandidate,
    previous: Option<&StagingFront>,
    active_raid: bool,
    owner: SwarmId,
    state: &GameState<'_>,
    budget: &mut WorkBudget,
) -> Option<Arc<[Vec2]>> {
    let home = state
        .swarms
        .iter()
        .find(|swarm| budget.spend(1) && swarm.id == owner)
        .map_or(Vec2::ZERO, |swarm| swarm.home);
    let start = previous.map_or(home, |previous| {
        let cell = previous.point();
        ground(owner, state, cell, budget).unwrap_or_else(|| intent_cell_center(cell))
    });
    let destination = ground(owner, state, candidate.attack_target?, budget)?;
    if budget.exhausted {
        return None;
    }
    if state.route_query.is_none() {
        return Some(Arc::from([start, destination]));
    }
    let goal = candidate.approach.unwrap_or(RouteGoal::Range {
        center: destination,
        // Leave room to route beside a structure at the tile center.
        radius: crate::ZONE_BLOCK_SIZE / 2.0,
    });
    let (start, route) = if let Some(route) = query_route(state, start, goal, budget) {
        (start, route)
    } else {
        if active_raid || start == home || budget.exhausted {
            return None;
        }
        (home, query_route(state, home, goal, budget)?)
    };
    Some(Arc::from(
        std::iter::once(start)
            .chain(route.waypoints)
            .collect::<Vec<_>>(),
    ))
}

fn stageable(
    owner: SwarmId,
    state: &GameState<'_>,
    protected: Option<IVec2>,
    cell: IVec2,
    budget: &mut WorkBudget,
) -> bool {
    protected != Some(cell) && ground(owner, state, cell, budget).is_some()
}

fn ground(
    owner: SwarmId,
    state: &GameState<'_>,
    cell: IVec2,
    budget: &mut WorkBudget,
) -> Option<Vec2> {
    if let Some(map) = state.reachability {
        return map
            .planning_point_with_budget(owner, cell, state.grid, &mut || budget.spend(1))
            .ok()
            .flatten();
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
