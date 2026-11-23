//! Responsive staging and continuous local roaming for Defenders without a response.
//!
//! Layouts retain exact balance and minimize incumbent and returner relocation.
//! Small cohorts then minimize total travel exactly; large cohorts assign the
//! remaining travel through deterministic bounded-nearest matching.

use std::collections::{BTreeMap, BTreeSet};

use bevy::{
    ecs::entity::{EntityHashMap, EntityHashSet},
    // Only looked up, never iterated, so the faster hasher cannot reorder anything.
    platform::collections::{HashMap, HashSet},
    prelude::*,
};

mod flow;

use flow::BoundedMinCostFlow;

const EXACT_STAGING_MAX_DEFENDERS: usize = 128;
const EXACT_STAGING_MAX_ASSIGNMENT_EDGES: usize = 16_384;
const BOUNDED_STAGING_AXIS_CANDIDATES: usize = 32;

use super::TerritorySnapshot;
use crate::{
    ZONE_BLOCK_SIZE,
    ai::get_world_from_zone,
    intent::{IntentGrid, IntentKind},
    nanobot::{
        ChargerAssignment, ChargerProgress, ClearingEvacuation, Commitment, DefenderDisengagement,
        DefenderResponse, DefenderRout, DirectMovementComponent, Health, Nanobot, NanobotType,
        SwarmId, SwarmMember, world_to_cell,
    },
    navigation::{CELL_WIDTH, ConnectivityStatus, Navigation, RouteGoal},
    reachability::SwarmReachabilityMap,
};

#[derive(Debug, Clone, Copy, Component)]
pub(super) struct DefenderStaging {
    cell: IVec2,
    waypoint_index: u64,
    waypoint: Vec2,
    navigation_revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StagingLayoutInput {
    defenders: Vec<Entity>,
    cells: Vec<IVec2>,
}

impl DefenderStaging {
    /// Exact identity, so a stored staging is replaced unless it is bit for bit the same.
    fn bits(&self) -> (IVec2, u64, [u32; 2], u64) {
        (
            self.cell,
            self.waypoint_index,
            [self.waypoint.x.to_bits(), self.waypoint.y.to_bits()],
            self.navigation_revision,
        )
    }
}

/// Cache of the current owner-scoped Defender staging layout.
#[derive(Debug, Default, Resource)]
pub(super) struct DefenderStagingLayouts {
    by_swarm: BTreeMap<SwarmId, StagingLayoutInput>,
}

#[derive(Debug, Clone, Copy)]
struct DefenderSnapshot {
    entity: Entity,
    position: Vec2,
    current_cell: IVec2,
    swarm: SwarmId,
    staging: Option<DefenderStaging>,
}

struct BoundedStagingTargets {
    by_x: BTreeSet<(i32, i32, usize)>,
    by_y: BTreeSet<(i32, i32, usize)>,
}

impl BoundedStagingTargets {
    fn new(cells: &[IVec2], capacities: &[usize]) -> Self {
        let mut targets = Self {
            by_x: BTreeSet::new(),
            by_y: BTreeSet::new(),
        };
        for (index, (cell, capacity)) in cells.iter().zip(capacities).enumerate() {
            if *capacity > 0 {
                targets.by_x.insert((cell.x, cell.y, index));
                targets.by_y.insert((cell.y, cell.x, index));
            }
        }
        targets
    }

    fn nearest(&self, cells: &[IVec2], position: Vec2) -> Option<usize> {
        let current = world_to_cell(position);
        let x_pivot = (current.x, current.y, 0);
        let y_pivot = (current.y, current.x, 0);
        let mut candidates = self
            .by_x
            .range(x_pivot..)
            .take(BOUNDED_STAGING_AXIS_CANDIDATES)
            .chain(
                self.by_x
                    .range(..x_pivot)
                    .rev()
                    .take(BOUNDED_STAGING_AXIS_CANDIDATES),
            )
            .map(|(_, _, index)| *index)
            .chain(
                self.by_y
                    .range(y_pivot..)
                    .take(BOUNDED_STAGING_AXIS_CANDIDATES)
                    .chain(
                        self.by_y
                            .range(..y_pivot)
                            .rev()
                            .take(BOUNDED_STAGING_AXIS_CANDIDATES),
                    )
                    .map(|(_, _, index)| *index),
            )
            .collect::<Vec<_>>();
        candidates.sort_unstable();
        candidates.dedup();
        candidates.into_iter().min_by(|left, right| {
            position
                .distance_squared(get_world_from_zone(cells[*left]))
                .total_cmp(&position.distance_squared(get_world_from_zone(cells[*right])))
                .then_with(|| cells[*left].y.cmp(&cells[*right].y))
                .then_with(|| cells[*left].x.cmp(&cells[*right].x))
        })
    }

    fn remove(&mut self, cell: IVec2, index: usize) {
        self.by_x.remove(&(cell.x, cell.y, index));
        self.by_y.remove(&(cell.y, cell.x, index));
    }
}

/// The `count` Defenders nearest `cell`'s center, nearest first, ties by entity: the
/// first `count` of a stable sort by that comparison.
fn nearest_first<'a>(
    defenders: &[&'a DefenderSnapshot],
    cell: IVec2,
    count: usize,
) -> Vec<&'a DefenderSnapshot> {
    let center = get_world_from_zone(cell);
    // Entity bits make the order total, so unstable selection and sorting agree with it.
    let order = |left: &(f32, u64, usize), right: &(f32, u64, usize)| {
        left.0.total_cmp(&right.0).then(left.1.cmp(&right.1))
    };
    let mut keyed = defenders
        .iter()
        .enumerate()
        .map(|(index, defender)| {
            (
                defender.position.distance_squared(center),
                defender.entity.to_bits(),
                index,
            )
        })
        .collect::<Vec<_>>();
    if count < keyed.len() {
        keyed.select_nth_unstable_by(count, order);
        keyed.truncate(count);
    }
    keyed.sort_unstable_by(order);
    keyed.iter().map(|&(.., index)| defenders[index]).collect()
}

fn owned_defend_cells(grid: &IntentGrid, swarm: SwarmId) -> Vec<IVec2> {
    let mut cells = grid
        .iter_active_cells()
        .filter_map(|(cell, intent)| (intent.has_owned(IntentKind::Defend, swarm)).then_some(cell))
        .collect::<Vec<_>>();
    cells.sort_by_key(|cell| (cell.y, cell.x));
    cells
}

fn territory_cells(territory: &TerritorySnapshot, swarm: SwarmId) -> Vec<IVec2> {
    let mut cells = territory
        .regions(swarm)
        .flat_map(|region| territory.tiles_in_region(swarm, region).iter().copied())
        .collect::<Vec<_>>();
    cells.sort_by_key(|cell| (cell.y, cell.x));
    cells.dedup();
    cells
}

fn connected_components(cells: &[IVec2]) -> Vec<Vec<IVec2>> {
    let all = cells.iter().copied().collect::<HashSet<_>>();
    let mut visited = HashSet::new();
    let mut components = Vec::new();
    for seed in cells {
        if !visited.insert(*seed) {
            continue;
        }
        let mut component = Vec::new();
        let mut pending = vec![*seed];
        while let Some(cell) = pending.pop() {
            component.push(cell);
            for dy in -1..=1 {
                for dx in -1..=1 {
                    if dx == 0 && dy == 0 {
                        continue;
                    }
                    let neighbour = cell + IVec2::new(dx, dy);
                    if all.contains(&neighbour) && visited.insert(neighbour) {
                        pending.push(neighbour);
                    }
                }
            }
        }
        component.sort_by_key(|cell| (cell.y, cell.x));
        components.push(component);
    }
    components
}

fn component_extra_bounds(
    components: &[Vec<IVec2>],
    remainder: usize,
    cell_count: usize,
) -> Vec<(usize, usize)> {
    let floors = components
        .iter()
        .map(|component| remainder * component.len() / cell_count)
        .collect::<Vec<_>>();
    let unapportioned = remainder - floors.iter().sum::<usize>();
    if unapportioned == 0 {
        return floors.into_iter().map(|extra| (extra, extra)).collect();
    }

    let mut fractions = components
        .iter()
        .map(|component| remainder * component.len() % cell_count)
        .collect::<Vec<_>>();
    fractions.sort_unstable_by(|left, right| right.cmp(left));
    let cutoff = fractions[unapportioned - 1];
    let cutoff_slots = unapportioned
        - fractions
            .iter()
            .filter(|fraction| **fraction > cutoff)
            .count();
    let cutoff_components = fractions
        .iter()
        .filter(|fraction| **fraction == cutoff)
        .count();
    components
        .iter()
        .zip(floors)
        .map(|(component, floor)| {
            let fraction = remainder * component.len() % cell_count;
            if fraction > cutoff || (fraction == cutoff && cutoff_slots == cutoff_components) {
                (floor + 1, floor + 1)
            } else if fraction == cutoff {
                (floor, floor + 1)
            } else {
                (floor, floor)
            }
        })
        .collect()
}

fn bounded_balanced_assignments(
    defenders: &[DefenderSnapshot],
    cells: &[IVec2],
) -> EntityHashMap<IVec2> {
    let base = defenders.len() / cells.len();
    let remainder = defenders.len() % cells.len();
    let cell_indexes = cells
        .iter()
        .enumerate()
        .map(|(index, cell)| (*cell, index))
        .collect::<HashMap<_, _>>();
    let components = connected_components(cells);
    let extra_bounds = component_extra_bounds(&components, remainder, cells.len());
    let component_by_cell = components
        .iter()
        .enumerate()
        .flat_map(|(component, cells)| cells.iter().map(move |cell| (*cell, component)))
        .collect::<HashMap<_, _>>();

    let mut incumbent_counts = vec![0_usize; cells.len()];
    let mut returner_counts = vec![0_usize; cells.len()];
    for defender in defenders {
        let valid_staging = defender
            .staging
            .and_then(|staging| cell_indexes.get(&staging.cell).copied());
        if let Some(index) = valid_staging {
            incumbent_counts[index] += 1;
        } else if let Some(index) = cell_indexes.get(&defender.current_cell).copied() {
            returner_counts[index] += 1;
        }
    }

    let extra_preference = |left: &usize, right: &usize| {
        let score = |index: usize| {
            let incumbent_saved = incumbent_counts[index] > base;
            let returner_saved = !incumbent_saved
                && returner_counts[index] > base.saturating_sub(incumbent_counts[index]);
            (incumbent_saved, returner_saved)
        };
        let left_score = score(*left);
        let right_score = score(*right);
        right_score
            .cmp(&left_score)
            .then_with(|| cells[*left].y.cmp(&cells[*right].y))
            .then_with(|| cells[*left].x.cmp(&cells[*right].x))
    };

    let mut capacities = vec![base; cells.len()];
    let mut has_extra = vec![false; cells.len()];
    let mut component_extras = vec![0_usize; components.len()];
    for (component_index, component) in components.iter().enumerate() {
        let mut candidates = component
            .iter()
            .map(|cell| cell_indexes[cell])
            .collect::<Vec<_>>();
        candidates.sort_by(extra_preference);
        for cell_index in candidates.into_iter().take(extra_bounds[component_index].0) {
            capacities[cell_index] += 1;
            has_extra[cell_index] = true;
            component_extras[component_index] += 1;
        }
    }

    let mut extras_remaining = remainder.saturating_sub(component_extras.iter().sum());
    let mut candidates = (0..cells.len())
        .filter(|index| !has_extra[*index])
        .collect::<Vec<_>>();
    candidates.sort_by(extra_preference);
    for cell_index in candidates {
        if extras_remaining == 0 {
            break;
        }
        let component = component_by_cell[&cells[cell_index]];
        if component_extras[component] >= extra_bounds[component].1 {
            continue;
        }
        capacities[cell_index] += 1;
        component_extras[component] += 1;
        extras_remaining -= 1;
    }
    assert_eq!(
        extras_remaining, 0,
        "component bounds must admit every balanced staging remainder",
    );

    let mut assignments = EntityHashMap::default();
    let mut assigned = EntityHashSet::default();
    let mut incumbents_by_cell = vec![Vec::new(); cells.len()];
    for defender in defenders {
        if let Some(index) = defender
            .staging
            .and_then(|staging| cell_indexes.get(&staging.cell).copied())
        {
            incumbents_by_cell[index].push(defender);
        }
    }
    for (cell_index, incumbents) in incumbents_by_cell.iter().enumerate() {
        for defender in nearest_first(incumbents, cells[cell_index], capacities[cell_index]) {
            assignments.insert(defender.entity, cells[cell_index]);
            assigned.insert(defender.entity);
            capacities[cell_index] -= 1;
        }
    }
    let mut returners_by_cell = vec![Vec::new(); cells.len()];
    for defender in defenders {
        if !assigned.contains(&defender.entity)
            && let Some(index) = cell_indexes.get(&defender.current_cell).copied()
        {
            returners_by_cell[index].push(defender);
        }
    }
    for (cell_index, returners) in returners_by_cell.iter().enumerate() {
        for defender in nearest_first(returners, cells[cell_index], capacities[cell_index]) {
            assignments.insert(defender.entity, cells[cell_index]);
            assigned.insert(defender.entity);
            capacities[cell_index] -= 1;
        }
    }

    let mut remaining_defenders = defenders
        .iter()
        .filter(|defender| !assigned.contains(&defender.entity))
        .collect::<Vec<_>>();
    // Entity bits make the order total, so an unstable sort gives the same order.
    remaining_defenders.sort_unstable_by(|left, right| {
        left.current_cell
            .y
            .cmp(&right.current_cell.y)
            .then_with(|| left.current_cell.x.cmp(&right.current_cell.x))
            .then_with(|| left.position.y.total_cmp(&right.position.y))
            .then_with(|| left.position.x.total_cmp(&right.position.x))
            .then_with(|| left.entity.to_bits().cmp(&right.entity.to_bits()))
    });
    let remaining_capacity = capacities.iter().sum::<usize>();
    assert_eq!(
        remaining_defenders.len(),
        remaining_capacity,
        "balanced staging capacity must cover the full cohort",
    );
    let active_cells = capacities
        .iter()
        .enumerate()
        .filter_map(|(index, capacity)| (*capacity > 0).then_some(index))
        .collect::<Vec<_>>();
    if let [cell_index] = active_cells.as_slice() {
        assignments.extend(
            remaining_defenders
                .into_iter()
                .map(|defender| (defender.entity, cells[*cell_index])),
        );
        return assignments;
    }

    let mut targets = BoundedStagingTargets::new(cells, &capacities);
    for defender in remaining_defenders {
        let cell_index = targets
            .nearest(cells, defender.position)
            .expect("remaining staging capacity provides a bounded-nearest target");
        assignments.insert(defender.entity, cells[cell_index]);
        capacities[cell_index] -= 1;
        if capacities[cell_index] == 0 {
            targets.remove(cells[cell_index], cell_index);
        }
    }
    assignments
}

fn balanced_assignments(defenders: &[DefenderSnapshot], cells: &[IVec2]) -> EntityHashMap<IVec2> {
    let mut assignments = EntityHashMap::default();
    if cells.is_empty() {
        for defender in defenders {
            assignments.insert(defender.entity, defender.current_cell);
        }
        return assignments;
    }
    if let [cell] = cells {
        for defender in defenders {
            assignments.insert(defender.entity, *cell);
        }
        return assignments;
    }
    let assignment_edges = defenders.len().saturating_mul(cells.len());
    if defenders.len() > EXACT_STAGING_MAX_DEFENDERS
        || assignment_edges > EXACT_STAGING_MAX_ASSIGNMENT_EDGES
    {
        return bounded_balanced_assignments(defenders, cells);
    }

    let cell_set = cells.iter().copied().collect::<HashSet<_>>();
    let base = defenders.len() / cells.len();
    let remainder = defenders.len() % cells.len();
    let components = connected_components(cells);
    let extra_bounds = component_extra_bounds(&components, remainder, cells.len());
    let component_by_cell = components
        .iter()
        .enumerate()
        .flat_map(|(index, component)| component.iter().map(move |cell| (*cell, index)))
        .collect::<HashMap<_, _>>();

    let source = 0;
    let defender_start = source + 1;
    let cell_start = defender_start + defenders.len();
    let component_start = cell_start + cells.len();
    let sink = component_start + components.len();
    let mut flow = BoundedMinCostFlow::new(sink + 1);

    let maximum_travel = defenders
        .iter()
        .flat_map(|defender| {
            cells
                .iter()
                .map(|cell| defender.position.distance(get_world_from_zone(*cell)) as f64)
        })
        .max_by(f64::total_cmp)
        .unwrap_or_default();
    let travel_bound = maximum_travel * defenders.len() as f64 + 1.0;
    let returner_move_penalty = travel_bound;
    let incumbent_move_penalty = travel_bound * (defenders.len() + 1) as f64;

    let mut assignment_edges = vec![vec![0_usize; cells.len()]; defenders.len()];
    for (defender_index, defender) in defenders.iter().enumerate() {
        let defender_node = defender_start + defender_index;
        flow.add_edge(source, defender_node, 1, 1, 0.0);
        let valid_staging = defender
            .staging
            .map(|staging| staging.cell)
            .filter(|cell| cell_set.contains(cell));
        let valid_current = cell_set.contains(&defender.current_cell);
        for (cell_index, cell) in cells.iter().enumerate() {
            let travel = defender.position.distance(get_world_from_zone(*cell)) as f64;
            let preservation = if valid_staging.is_some_and(|staging| staging != *cell) {
                incumbent_move_penalty
            } else if valid_staging.is_none() && valid_current && defender.current_cell != *cell {
                returner_move_penalty
            } else {
                0.0
            };
            assignment_edges[defender_index][cell_index] = flow.add_edge(
                defender_node,
                cell_start + cell_index,
                0,
                1,
                preservation + travel,
            );
        }
    }
    for (cell_index, cell) in cells.iter().enumerate() {
        let component = component_by_cell[cell];
        flow.add_edge(
            cell_start + cell_index,
            component_start + component,
            base,
            base + usize::from(remainder > 0),
            0.0,
        );
    }
    for (component_index, component) in components.iter().enumerate() {
        let (minimum_extra, maximum_extra) = extra_bounds[component_index];
        flow.add_edge(
            component_start + component_index,
            sink,
            base * component.len() + minimum_extra,
            base * component.len() + maximum_extra,
            0.0,
        );
    }
    flow.add_edge(sink, source, defenders.len(), defenders.len(), 0.0);
    flow.solve();

    for (defender_index, defender) in defenders.iter().enumerate() {
        let defender_node = defender_start + defender_index;
        let cell = cells
            .iter()
            .enumerate()
            .find_map(|(cell_index, cell)| {
                flow.edge_is_saturated(defender_node, assignment_edges[defender_index][cell_index])
                    .then_some(*cell)
            })
            .expect("each Defender has one balanced staging destination");
        assignments.insert(defender.entity, cell);
    }
    assignments
}

fn splitmix64(mut value: u64) -> u64 {
    value = value.wrapping_add(0x9E37_79B9_7F4A_7C15);
    value = (value ^ (value >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    value ^ (value >> 31)
}

fn procedural_waypoint(entity: Entity, cell: IVec2, index: u64) -> Vec2 {
    let cell_bits = (cell.x as u32 as u64) << 32 | cell.y as u32 as u64;
    let first = splitmix64(entity.to_bits() ^ cell_bits.rotate_left(17) ^ index);
    let second = splitmix64(first);
    let unit = |bits: u64| (bits >> 40) as f32 / ((1_u32 << 24) - 1) as f32;
    let offset =
        (Vec2::new(unit(first), unit(second)) - Vec2::splat(0.5)) * (ZONE_BLOCK_SIZE * 0.7);
    get_world_from_zone(cell) + offset
}

/// The first clear roaming waypoint for `entity` in `cell`, trying procedural points
/// from `index`, then the cell's lattice, then `anchor`.
///
/// The procedural and lattice candidates depend only on the entity, cell and index, and
/// whether they lie in the cell and are clear for entry only on the navigation geometry.
/// `known_misses` of them are already known to fail that test and are skipped. Also
/// returns how many leading procedural and lattice candidates fail it.
#[allow(clippy::too_many_arguments)]
fn clear_roaming_waypoint(
    entity: Entity,
    cell: IVec2,
    index: u64,
    position: Vec2,
    navigation: &Navigation,
    anchor: Option<Vec2>,
    reachable: impl Fn(Vec2) -> bool,
    known_misses: u64,
) -> (Option<(u64, Vec2)>, u64) {
    const PROCEDURAL: u64 = 64;
    let lower = cell.as_vec2() * ZONE_BLOCK_SIZE;
    let min = (lower / CELL_WIDTH).floor().as_ivec2();
    let max = ((lower + Vec2::splat(ZONE_BLOCK_SIZE)) / CELL_WIDTH)
        .ceil()
        .as_ivec2();
    let size = max - min;
    let count = (size.x * size.y) as u64;
    let fixed = PROCEDURAL + count;
    let known_misses = known_misses.min(fixed);
    let procedural = (known_misses.min(PROCEDURAL)..PROCEDURAL).map(|offset| {
        let index = index.wrapping_add(offset);
        (index, procedural_waypoint(entity, cell, index))
    });
    let lattice = (known_misses.saturating_sub(PROCEDURAL)..count).map(|offset| {
        let candidate = index.wrapping_add(offset) % count;
        let tile = min + IVec2::new(candidate as i32 % size.x, candidate as i32 / size.x);
        (index, (tile.as_vec2() + Vec2::splat(0.5)) * CELL_WIDTH)
    });
    let mut misses = None;
    for (offset, (index, waypoint)) in (known_misses..).zip(
        procedural
            .chain(lattice)
            .chain(anchor.map(|point| (index, point))),
    ) {
        if world_to_cell(waypoint) != cell || !navigation.point_clear_for_entry(waypoint) {
            continue;
        }
        let misses = *misses.get_or_insert(offset.min(fixed));
        if !reachable(waypoint) {
            continue;
        }
        if navigation.movement_clear(position, waypoint) {
            return (Some((index, waypoint)), misses);
        }
        match navigation.query_connectivity(position, RouteGoal::Point(waypoint)) {
            ConnectivityStatus::Connected { .. } => return (Some((index, waypoint)), misses),
            ConnectivityStatus::Pending => return (None, misses),
            ConnectivityStatus::Unreachable => {}
        }
    }
    (None, misses.unwrap_or(fixed))
}

/// Everything a roaming search's geometry test depends on, besides the Defender.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct RoamingSearch {
    cell: IVec2,
    index: u64,
    clearance: u64,
}

#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub(super) fn reconcile_defender_staging_system(
    mut commands: Commands,
    grid: Res<IntentGrid>,
    navigation: Res<Navigation>,
    territory: Res<TerritorySnapshot>,
    reachability: Option<Res<SwarmReachabilityMap>>,
    mut layouts: ResMut<DefenderStagingLayouts>,
    // Per Defender, its last search and how many leading candidates failed the geometry test.
    mut roaming_misses: Local<EntityHashMap<(RoamingSearch, u64)>>,
    mut defenders: Query<
        (
            Entity,
            &Transform,
            &NanobotType,
            &SwarmMember,
            &Commitment,
            Option<&Health>,
            Option<&DefenderResponse>,
            Option<&ChargerAssignment>,
            Option<&ChargerProgress>,
            Option<&DefenderStaging>,
            Option<&DirectMovementComponent>,
            Has<DefenderDisengagement>,
            Has<DefenderRout>,
            Has<ClearingEvacuation>,
        ),
        With<Nanobot>,
    >,
) {
    let mut snapshots = Vec::new();
    for (
        entity,
        transform,
        kind,
        member,
        commitment,
        health,
        response,
        charger_assignment,
        charger_progress,
        staging,
        _,
        disengaged,
        routing,
        evacuating,
    ) in &mut defenders
    {
        let available = *kind == NanobotType::Defender
            && *commitment == Commitment::Idle
            && health.is_none_or(|health| health.current > 0)
            && response.is_none()
            && charger_assignment.is_none()
            && charger_progress.is_none()
            && !disengaged
            && !routing
            && !evacuating;
        if !available {
            if staging.is_some() {
                commands.entity(entity).remove::<DefenderStaging>();
            }
            continue;
        }

        let position = transform.translation.truncate();
        snapshots.push(DefenderSnapshot {
            entity,
            position,
            current_cell: world_to_cell(position),
            swarm: member.0,
            staging: staging.copied(),
        });
    }

    snapshots.sort_by_key(|defender| (defender.swarm, defender.entity.to_bits()));
    let mut assignments = EntityHashMap::default();
    let mut seen_swarms = HashSet::new();
    let mut start = 0;
    while start < snapshots.len() {
        let swarm = snapshots[start].swarm;
        seen_swarms.insert(swarm);
        let end = snapshots[start..]
            .iter()
            .position(|defender| defender.swarm != swarm)
            .map_or(snapshots.len(), |offset| start + offset);
        let cohort = &snapshots[start..end];
        let defend_cells = owned_defend_cells(&grid, swarm);
        let fallback_cells = if defend_cells.is_empty() {
            territory_cells(&territory, swarm)
        } else {
            Vec::new()
        };
        let cells = if defend_cells.is_empty() {
            &fallback_cells
        } else {
            &defend_cells
        };
        let input = StagingLayoutInput {
            defenders: cohort.iter().map(|defender| defender.entity).collect(),
            cells: cells.clone(),
        };
        let reusable = layouts.by_swarm.get(&swarm) == Some(&input)
            && cohort.iter().all(|defender| {
                defender.staging.is_some_and(|staging| {
                    if cells.is_empty() {
                        staging.cell == defender.current_cell
                    } else {
                        cells.contains(&staging.cell)
                    }
                })
            });
        if reusable {
            assignments.extend(cohort.iter().map(|defender| {
                (
                    defender.entity,
                    defender
                        .staging
                        .expect("a reusable staging layout has every assignment")
                        .cell,
                )
            }));
        } else {
            assignments.extend(balanced_assignments(cohort, cells));
            layouts.by_swarm.insert(swarm, input);
        }
        start = end;
    }
    layouts
        .by_swarm
        .retain(|swarm, _| seen_swarms.contains(swarm));

    let navigation_revision = navigation.revision();
    roaming_misses.retain(|entity, _| assignments.contains_key(entity));
    for (entity, transform, _, member, _, _, _, _, _, staging, movement, ..) in &mut defenders {
        let Some(&target_cell) = assignments.get(&entity) else {
            continue;
        };
        let position = transform.translation.truncate();
        let current_cell = world_to_cell(position);
        let previous = staging.filter(|staging| staging.cell == target_cell);
        let arrived = previous.is_some_and(|staging| {
            current_cell == target_cell
                && position.distance(staging.waypoint) <= crate::nanobot::STOP_THRESHOLD
        });
        let selected = if let Some(previous) = previous
            && !arrived
            && previous.navigation_revision == navigation_revision
        {
            Some((previous.waypoint_index, previous.waypoint))
        } else {
            let index = previous.map_or(0, |staging| staging.waypoint_index);
            let anchor = reachability
                .as_ref()
                .and_then(|map| map.reachable_point(member.0, target_cell, &grid));
            let index = index.wrapping_add(u64::from(arrived));
            // A search repeated against the same geometry skips the candidates it rejected.
            let search = RoamingSearch {
                cell: target_cell,
                index,
                clearance: navigation.clearance_identity(),
            };
            let known_misses = roaming_misses
                .get(&entity)
                .filter(|(known, _)| *known == search)
                .map_or(0, |&(_, misses)| misses);
            let (selected, misses) = clear_roaming_waypoint(
                entity,
                target_cell,
                index,
                position,
                &navigation,
                anchor,
                |point| {
                    reachability
                        .as_ref()
                        .is_none_or(|map| map.position_is_reachable(member.0, point, &grid))
                },
                known_misses,
            );
            roaming_misses.insert(entity, (search, misses));
            selected
        };
        let Some((waypoint_index, waypoint)) = selected else {
            if staging.is_some_and(|staging| staging.cell != target_cell) {
                // An older waypoint cannot stand in for the pending layout assignment.
                commands.entity(entity).remove::<DefenderStaging>();
            }
            if movement.is_some() {
                commands.entity(entity).remove::<DirectMovementComponent>();
            }
            continue;
        };
        let speed = (current_cell == target_cell).then_some(crate::nanobot::BOT_SPREAD_FORCE);
        if movement.is_none_or(|movement| movement.xy != waypoint || movement.speed != speed) {
            commands.entity(entity).insert(DirectMovementComponent {
                xy: waypoint,
                stop_radius: 0.0,
                interaction: None,
                speed,
            });
        }
        let selected = DefenderStaging {
            cell: target_cell,
            waypoint_index,
            waypoint,
            navigation_revision,
        };
        // Nothing reads when staging changed, so an identical staging needs no command.
        if staging.is_none_or(|staging| staging.bits() != selected.bits()) {
            commands.entity(entity).insert(selected);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(id: u64, position: Vec2, staging: Option<IVec2>) -> DefenderSnapshot {
        DefenderSnapshot {
            entity: Entity::from_bits(id),
            position,
            current_cell: world_to_cell(position),
            swarm: SwarmId::PLAYER,
            staging: staging.map(|cell| DefenderStaging {
                cell,
                waypoint_index: 0,
                waypoint: Vec2::ZERO,
                navigation_revision: 0,
            }),
        }
    }

    #[test]
    fn nearest_first_takes_the_front_of_the_stable_distance_sort() {
        use rand::{RngExt, SeedableRng, rngs::StdRng};

        let mut rng = StdRng::seed_from_u64(0x0005_7A6E);
        for _ in 0..300 {
            let cell = IVec2::new(rng.random_range(-3..3), rng.random_range(-3..3));
            let center = get_world_from_zone(cell);
            let defenders = (0..rng.random_range(0..200))
                .map(|_| {
                    // Mirrored offsets give equal distances, so ties must break by entity.
                    let offset = Vec2::new(
                        rng.random_range(0..8) as f32 * 64.0,
                        rng.random_range(0..8) as f32 * 64.0,
                    ) * if rng.random_bool(0.5) { 1.0 } else { -1.0 };
                    snapshot(rng.random_range(1..10_000), center + offset, None)
                })
                .collect::<Vec<_>>();
            let references = defenders.iter().collect::<Vec<_>>();
            let mut expected = references.clone();
            expected.sort_by(|left, right| {
                left.position
                    .distance_squared(center)
                    .total_cmp(&right.position.distance_squared(center))
                    .then_with(|| left.entity.to_bits().cmp(&right.entity.to_bits()))
            });
            let count = rng.random_range(0..=defenders.len() + 2);
            expected.truncate(count);
            let entities = |defenders: Vec<&DefenderSnapshot>| {
                defenders
                    .iter()
                    .map(|defender| defender.entity)
                    .collect::<Vec<_>>()
            };
            assert_eq!(
                entities(nearest_first(&references, cell, count)),
                entities(expected)
            );
        }
    }

    #[test]
    fn skipping_known_roaming_misses_finds_the_same_waypoint() {
        use rand::{RngExt, SeedableRng, rngs::StdRng};

        use crate::navigation::Obstacle;

        let mut rng = StdRng::seed_from_u64(0x0A11_5EED);
        let grid = IntentGrid::new(8, 8);
        let mut skipped = 0;
        for layout in 0..20 {
            // Dense rocks leave few clear points, so many searches begin with misses.
            let obstacles = (0..rng.random_range(20..400))
                .map(|_| Obstacle::Circle {
                    center: Vec2::new(
                        rng.random_range(-1_100.0..1_100.0),
                        rng.random_range(-1_100.0..1_100.0),
                    ),
                    radius: rng.random_range(20.0..160.0),
                })
                .collect();
            let navigation = Navigation::new(&grid, obstacles);
            for search in 0..40 {
                let entity = Entity::from_bits(1 + layout * 100 + search);
                let cell = IVec2::new(rng.random_range(-2..2), rng.random_range(-2..2));
                let index = rng.random_range(0..200);
                let position = Vec2::new(
                    rng.random_range(-1_000.0..1_000.0),
                    rng.random_range(-1_000.0..1_000.0),
                );
                let anchor = rng.random_bool(0.5).then(|| get_world_from_zone(cell));
                let unreachable = Vec2::new(rng.random_range(-1_000.0..1_000.0), 0.0);
                let reachable = |point: Vec2| point.x < unreachable.x;
                let search = |known_misses| {
                    clear_roaming_waypoint(
                        entity,
                        cell,
                        index,
                        position,
                        &navigation,
                        anchor,
                        reachable,
                        known_misses,
                    )
                };
                let (expected, misses) = search(0);
                for known_misses in [misses, rng.random_range(0..=misses)] {
                    assert_eq!(search(known_misses), (expected, misses));
                }
                skipped += misses;
            }
        }
        assert!(
            skipped > 1_000,
            "the searches must skip rejected candidates"
        );
    }

    #[test]
    fn equal_component_remainders_stay_flexible_for_global_cost() {
        let components = vec![vec![IVec2::new(-2, 0)], vec![IVec2::new(2, 0)]];

        assert_eq!(component_extra_bounds(&components, 1, 2), [(0, 1), (0, 1)]);
    }

    #[test]
    fn larger_component_receives_the_strict_proportional_remainder() {
        let components = vec![
            vec![IVec2::new(-2, 0)],
            vec![IVec2::new(1, 0), IVec2::new(2, 0), IVec2::new(3, 0)],
        ];

        assert_eq!(component_extra_bounds(&components, 1, 4), [(0, 0), (1, 1)]);
    }

    #[test]
    fn capacity_choice_and_matching_share_one_global_travel_minimum() {
        let west = IVec2::new(-2, 0);
        let east = IVec2::new(2, 0);
        let defenders = [
            snapshot(1, Vec2::new(-512.0, 256.0), None),
            snapshot(2, Vec2::new(1023.9, 256.0), None),
            snapshot(3, Vec2::new(1000.0, 156.0), None),
        ];

        let assignments = balanced_assignments(&defenders, &[west, east]);

        assert_eq!(assignments[&Entity::from_bits(1)], west);
        assert_eq!(assignments[&Entity::from_bits(2)], east);
        assert_eq!(assignments[&Entity::from_bits(3)], east);
    }

    #[test]
    fn returning_defender_moves_before_a_steady_incumbent() {
        let west = IVec2::ZERO;
        let east = IVec2::X;
        let position = get_world_from_zone(west);
        let incumbent = snapshot(1, position, Some(west));
        let returner = snapshot(2, position, None);

        let assignments = balanced_assignments(&[incumbent, returner], &[west, east]);

        assert_eq!(assignments[&incumbent.entity], west);
        assert_eq!(assignments[&returner.entity], east);
    }
}
