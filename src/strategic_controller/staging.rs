use std::sync::Arc;

use super::{
    lifecycle::WorkBudget,
    materialize::{intent_cell_center, query_route, world_to_intent_cell},
    raid::{READINESS_GRACE_TICKS, RaidForces, distance_squared_to_segment},
    *,
};

use crate::navigation::{RouteGoal, segment_box};

const MAX_STAGING_POINTS: usize = 6;
pub(super) const MINIMUM_STAGING_BLOCK_CELLS: usize = 9;

#[derive(Debug, Clone)]
pub(super) struct StagingFront {
    route: Arc<[Vec2]>,
    points: Vec<StagingPoint>,
    active: usize,
    spacing_cells: u32,
    readiness_shortfall_started: Option<u64>,
    pub(super) defend_cells: Arc<HashSet<IVec2>>,
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

    pub(super) fn observe_readiness_from(&mut self, reviewed: &Self) {
        self.readiness_shortfall_started = reviewed.readiness_shortfall_started;
    }

    pub(super) fn covers(&self, target: IVec2, owner: SwarmId, state: &GameState<'_>) -> bool {
        self.defend_cells.contains(&target)
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

    fn reusable_route(&self, route: &[Vec2], budget: &mut WorkBudget) -> Option<Vec<Vec2>> {
        if !budget.spend(self.route.len()) {
            return None;
        }
        let distance = self.points[self.active].distance;
        let mut traveled = 0.0;
        for (index, segment) in self.route.windows(2).enumerate() {
            let length = segment[0].distance(segment[1]);
            if length > 0.0 && distance <= traveled + length {
                let start = segment[0].lerp(segment[1], (distance - traveled) / length);
                let mut remaining = Vec::with_capacity(self.route.len() - index + 1);
                remaining.push(start);
                remaining.extend_from_slice(&self.route[index + 1..]);
                let remaining_stages = &self.points[self.active + 1..self.points.len() - 1];
                let reusable = follows_route(&remaining, route, budget)
                    && follows_route(route, &remaining, budget)
                    && remaining_stages
                        .iter()
                        .all(|point| route_crosses_cell(route, point.cell, budget));
                if reusable && route[0] != start {
                    remaining.insert(0, route[0]);
                }
                return reusable.then_some(remaining);
            }
            traveled += length;
        }
        None
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

fn follows_route(checkpoints: &[Vec2], route: &[Vec2], budget: &mut WorkBudget) -> bool {
    // Allow the offset between a sample and its cell center when the next search
    // starts from the active staging cell.
    let radius_squared = crate::ZONE_BLOCK_SIZE.powi(2) / 2.0;
    let mut segments = route.windows(2).peekable();
    for &point in checkpoints {
        loop {
            let Some(segment) = segments.peek() else {
                return false;
            };
            if !budget.spend(1) {
                return false;
            }
            if distance_squared_to_segment(point, segment[0], segment[1]) <= radius_squared {
                break;
            }
            segments.next();
        }
    }
    true
}

fn route_crosses_cell(route: &[Vec2], cell: IVec2, budget: &mut WorkBudget) -> bool {
    let min = cell.as_vec2() * crate::ZONE_BLOCK_SIZE;
    let max = (cell + IVec2::ONE).as_vec2() * crate::ZONE_BLOCK_SIZE;
    // Intent cells include their lower edge. The segment test uses an open box.
    let min = Vec2::new(min.x.next_down(), min.y.next_down());
    route
        .windows(2)
        .take_while(|_| budget.spend(1))
        .any(|segment| segment_box(segment[0], segment[1], min, max))
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

#[derive(Debug, PartialEq, Eq)]
pub(super) enum StageFailure {
    Ineligible,
    WorkExhausted,
    SustainedReadinessLoss,
}

impl StageFailure {
    fn unavailable(budget: &WorkBudget) -> Self {
        if budget.exhausted {
            Self::WorkExhausted
        } else {
            Self::Ineligible
        }
    }
}

pub(super) fn review_staging(
    tuning: &AutonomyTuning,
    candidate: &PlanCandidate,
    current: Option<&IntentPlan>,
    owner: SwarmId,
    state: &GameState<'_>,
    raid_forces: &RaidForces<'_>,
    budget: &mut WorkBudget,
) -> Result<StagingFront, StageFailure> {
    let target = candidate.attack_target.ok_or(StageFailure::Ineligible)?;
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
    let requires_readiness = candidate.objective.needs_ready_force();
    let spacing = tuning.strategy_staging_spacing_cells;
    if requires_readiness && continuing.is_none() && !raid_forces.can_launch() {
        return Err(StageFailure::unavailable(budget));
    }
    let mut reusable = continuing.filter(|front| {
        (!requires_readiness || front.at_destination() || front.spacing_cells == spacing)
            && front.destination_contains(target, owner, state)
            && front.points[front.active + 1..]
                .iter()
                .all(|point| stageable(owner, state, candidate.protected_home, point.cell, budget))
    });

    let route = if requires_readiness || reusable.is_none() {
        let active_raid = requires_readiness && continuing.is_some();
        let route_start = if requires_readiness {
            continuing
        } else {
            previous
        };
        let route = staging_route(candidate, route_start, active_raid, owner, state, budget)
            .ok_or_else(|| StageFailure::unavailable(budget))?;
        if requires_readiness {
            let target_position = match candidate.approach {
                Some(RouteGoal::Range { center, .. }) => center,
                _ => intent_cell_center(target),
            };
            let attack_ready = |route: &[Vec2], budget: &mut WorkBudget| {
                attack_route_is_ready(
                    continuing,
                    owner,
                    state,
                    raid_forces,
                    route,
                    target_position,
                    budget,
                )
                .ok_or_else(|| StageFailure::unavailable(budget))
            };
            if !attack_ready(&route, budget)? {
                let mut held = continuing.ok_or(StageFailure::Ineligible)?.clone();
                let shortfall_started = held.readiness_shortfall_started.unwrap_or(state.tick);
                if state.tick.saturating_sub(shortfall_started) >= READINESS_GRACE_TICKS {
                    return Err(StageFailure::SustainedReadinessLoss);
                }
                held.readiness_shortfall_started = Some(shortfall_started.min(state.tick));
                return Ok(held);
            }
            // A fresh route can clear guards even where it crosses the old staging cells.
            if let Some(front) = reusable.filter(|front| !front.at_destination()) {
                reusable = match front.reusable_route(&route, budget) {
                    Some(prior_route) => (prior_route.as_slice() == route.as_ref()
                        || attack_ready(&prior_route, budget)?)
                    .then_some(front),
                    None => None,
                };
            }
        }
        Some(route)
    } else {
        None
    };

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
            )
            .ok_or_else(|| StageFailure::unavailable(budget))?;
            retained.points.retain(|point| {
                stageable(owner, state, candidate.protected_home, point.cell, budget)
            });
            if retained.points.is_empty() || budget.exhausted {
                return Err(StageFailure::unavailable(budget));
            }
            retained.active = 0;
        }
        retained.spacing_cells = spacing;
        retained.readiness_shortfall_started = None;
        retained.advance_if_arrived(owner, state, arrival_fraction, budget);
        return Ok(retained);
    }

    let route = route.ok_or_else(|| StageFailure::unavailable(budget))?;
    let held_stage = continuing.filter(|old| !old.at_destination());
    let first = held_stage.map(|stage| StagingPoint {
        cell: stage.point(),
        distance: 0.0,
    });
    let mut points = staging_points(&route, first, target, spacing, budget)
        .ok_or_else(|| StageFailure::unavailable(budget))?;
    points.retain(|point| stageable(owner, state, candidate.protected_home, point.cell, budget));
    if points.is_empty() || budget.exhausted {
        return Err(StageFailure::unavailable(budget));
    }
    let mut staging = StagingFront {
        route,
        points,
        active: 0,
        spacing_cells: spacing,
        readiness_shortfall_started: None,
        defend_cells: held_stage.map_or_else(Arc::default, |stage| Arc::clone(&stage.defend_cells)),
    };
    if held_stage.is_some() {
        staging.advance_if_arrived(owner, state, arrival_fraction, budget);
    }
    Ok(staging)
}

fn attack_route_is_ready(
    continuing: Option<&StagingFront>,
    owner: SwarmId,
    state: &GameState<'_>,
    raid_forces: &RaidForces<'_>,
    route: &[Vec2],
    target_position: Vec2,
    budget: &mut WorkBudget,
) -> Option<bool> {
    if !raid_forces.can_launch() {
        return Some(false);
    }
    let covers = |position| {
        continuing.is_some_and(|front| front.covers(world_to_intent_cell(position), owner, state))
    };
    let contributes = |bot: &BotState| match bot.defender_duty {
        DefenderDuty::RespondingTo(position) => covers(position),
        _ => true,
    };
    if continuing.is_none() {
        return raid_forces.ready(route, target_position, contributes, budget);
    }
    if raid_forces.ready(
        route,
        target_position,
        |bot| contributes(bot) && covers(bot.position),
        budget,
    )? {
        return Some(true);
    }

    let home = staging_home(owner, state, budget);
    let stage = *route.first()?;
    let mut approach = vec![home];
    if state.route_query.is_some() {
        let goal = RouteGoal::Range {
            center: stage,
            radius: crate::ZONE_BLOCK_SIZE / 2.0,
        };
        let Some(prefix) = query_route(state, home, goal, budget) else {
            return (!budget.exhausted).then_some(false);
        };
        approach.extend(prefix.waypoints);
    }
    if !budget.spend(route.len()) {
        return None;
    }
    approach.extend_from_slice(route);
    raid_forces.ready(&approach, target_position, contributes, budget)
}

fn staging_home(owner: SwarmId, state: &GameState<'_>, budget: &mut WorkBudget) -> Vec2 {
    state
        .swarms
        .iter()
        .find(|swarm| budget.spend(1) && swarm.id == owner)
        .map_or(Vec2::ZERO, |swarm| swarm.home)
}

fn staging_route(
    candidate: &PlanCandidate,
    previous: Option<&StagingFront>,
    active_raid: bool,
    owner: SwarmId,
    state: &GameState<'_>,
    budget: &mut WorkBudget,
) -> Option<Arc<[Vec2]>> {
    let home = staging_home(owner, state, budget);
    let start = previous.map_or(home, |previous| {
        let cell = previous.point();
        ground(owner, state, cell, budget).unwrap_or_else(|| intent_cell_center(cell))
    });
    let destination = ground(owner, state, candidate.attack_target?, budget)?;
    if budget.exhausted {
        return None;
    }
    let goal = candidate.approach.unwrap_or(RouteGoal::Range {
        center: destination,
        // Leave room to route beside a structure at the tile center.
        radius: crate::ZONE_BLOCK_SIZE / 2.0,
    });
    if state.route_query.is_none() {
        let destination = match goal {
            RouteGoal::Range { center, .. } => center,
            _ => destination,
        };
        return Some(Arc::from([start, destination]));
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_nearby_reroute_bypasses_the_old_staging_cell() {
        let tile = crate::ZONE_BLOCK_SIZE;
        let target = IVec2::new(11, 0);
        let destination = Vec2::new(11.0, 0.5) * tile;
        let route = Arc::from([Vec2::new(0.5, 0.5) * tile, destination]);
        let mut budget = WorkBudget::planning(0);
        let points = staging_points(&route, None, target, 3, &mut budget).unwrap();
        assert_eq!(points[1].cell, IVec2::new(5, 0));
        let front = StagingFront {
            route,
            points,
            active: 0,
            spacing_cells: 3,
            readiness_shortfall_started: None,
            defend_cells: Arc::default(),
        };
        let reroute = [
            Vec2::new(3.5, 0.5) * tile,
            Vec2::new(4.5, 1.17) * tile,
            Vec2::new(9.5, 1.17) * tile,
            destination,
        ];
        assert!(front.reusable_route(&reroute, &mut budget).is_none());
    }
}
