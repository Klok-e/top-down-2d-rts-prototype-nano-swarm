//! Responsive staging and continuous local roaming for unengaged Defenders.

use std::collections::{BTreeMap, HashMap, HashSet};

use bevy::prelude::*;

mod flow;

use flow::BoundedMinCostFlow;

use super::TerritorySnapshot;
use crate::{
    ZONE_BLOCK_SIZE,
    ai::get_world_from_zone,
    intent::{IntentGrid, IntentKind},
    nanobot::{
        BOT_SPREAD_FORCE, ChargerAssignment, ChargerProgress, Commitment, DefenderResponse,
        DirectMovementComponent, Health, Nanobot, NanobotType, ProgressChecker, SwarmId,
        SwarmMember, VelocityComponent, world_to_cell,
    },
};

#[derive(Debug, Clone, Copy, Component)]
pub(super) struct DefenderStaging {
    cell: IVec2,
    waypoint_index: u64,
    waypoint: Vec2,
}

#[derive(Debug, Clone, Copy)]
struct DefenderSnapshot {
    entity: Entity,
    position: Vec2,
    current_cell: IVec2,
    swarm: SwarmId,
    staging: Option<DefenderStaging>,
}

fn owned_defend_cells(grid: &IntentGrid, swarm: SwarmId) -> Vec<IVec2> {
    let mut cells = grid
        .iter_active_cells()
        .filter_map(|(cell, intent)| {
            (intent.owner(IntentKind::Defend) == Some(swarm)).then_some(cell)
        })
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

fn balanced_assignments(
    defenders: &[DefenderSnapshot],
    cells: &[IVec2],
) -> BTreeMap<Entity, IVec2> {
    let mut assignments = BTreeMap::new();
    if cells.is_empty() {
        for defender in defenders {
            assignments.insert(defender.entity, defender.current_cell);
        }
        return assignments;
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

#[allow(clippy::type_complexity)]
pub(super) fn reconcile_defender_staging_system(
    mut commands: Commands,
    grid: Res<IntentGrid>,
    territory: Res<TerritorySnapshot>,
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
            &mut VelocityComponent,
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
        _,
    ) in &mut defenders
    {
        let available = *kind == NanobotType::Defender
            && *commitment == Commitment::Idle
            && health.is_none_or(|health| health.current > 0)
            && response.is_none()
            && charger_assignment.is_none()
            && charger_progress.is_none();
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
    let mut assignments = BTreeMap::new();
    let mut start = 0;
    while start < snapshots.len() {
        let swarm = snapshots[start].swarm;
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
        assignments.extend(balanced_assignments(cohort, cells));
        start = end;
    }

    for (entity, transform, _, _, _, _, _, _, _, staging, movement, mut velocity) in &mut defenders
    {
        let Some(&target_cell) = assignments.get(&entity) else {
            continue;
        };
        let position = transform.translation.truncate();
        let current_cell = world_to_cell(position);
        let (waypoint_index, waypoint) = staging
            .filter(|staging| staging.cell == target_cell)
            .map(|staging| (staging.waypoint_index, staging.waypoint))
            .unwrap_or_else(|| (0, procedural_waypoint(entity, target_cell, 0)));

        if current_cell != target_cell {
            commands.entity(entity).insert((
                DefenderStaging {
                    cell: target_cell,
                    waypoint_index,
                    waypoint,
                },
                DirectMovementComponent {
                    xy: waypoint,
                    stop_radius: 0.0,
                },
            ));
            continue;
        }

        let reached = position.distance_squared(waypoint) <= BOT_SPREAD_FORCE.powi(2);
        let (waypoint_index, waypoint) = if reached {
            let next = waypoint_index.wrapping_add(1);
            (next, procedural_waypoint(entity, target_cell, next))
        } else {
            (waypoint_index, waypoint)
        };
        if movement.is_some() {
            commands
                .entity(entity)
                .remove::<DirectMovementComponent>()
                .remove::<ProgressChecker>();
        }
        let direction = waypoint - position;
        if direction.length_squared() > f32::EPSILON {
            velocity.value += direction.normalize() * BOT_SPREAD_FORCE;
        }
        commands.entity(entity).insert(DefenderStaging {
            cell: target_cell,
            waypoint_index,
            waypoint,
        });
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
            }),
        }
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
