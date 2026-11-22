use std::collections::BTreeMap;

use bevy::{ecs::entity::EntityHashMap, prelude::*};

use super::{
    Charge, ChargerAssignment, ChargerProgress, ClearingEvacuation, CombatIdentity,
    DefenderEngagements, DefenderResponse, DirectMovementComponent, Health, Nanobot, NanobotType,
    RegionalLease, STOP_THRESHOLD, SwarmId, SwarmMember, combat_key,
    engagement::direction_to_hostile, move_system::BODY_DIAMETER, world_to_cell,
};
use crate::{
    ZONE_BLOCK_SIZE,
    ai::get_world_from_zone,
    intent::{IntentGrid, IntentKind},
    navigation::{CELL_WIDTH, ConnectivityStatus, Navigation, RouteGoal},
    reachability::SwarmReachabilityMap,
    tuning::{Tuning, elapsed_fixed_ticks},
};

#[derive(Component, Debug)]
pub struct DefenderDisengagement;

#[derive(Component, Debug, Clone, Copy)]
pub struct DefenderRout {
    pub cell: IVec2,
    pub started_tick: u64,
    destination: RetreatDestination,
    navigation_revision: u64,
}

#[derive(Debug, Clone, Copy)]
enum RetreatDestination {
    Pending,
    Target(Vec2),
    Unreachable,
}

impl DefenderRout {
    fn destination(
        &mut self,
        position: Vec2,
        navigation: &Navigation,
        ground: Option<(&SwarmReachabilityMap, SwarmId, &IntentGrid)>,
    ) -> Option<Vec2> {
        let revision = navigation.revision();
        if self.navigation_revision != revision
            || matches!(self.destination, RetreatDestination::Pending)
        {
            self.destination = retreat_destination(self.cell, position, navigation, ground);
            self.navigation_revision = revision;
        }
        match self.destination {
            RetreatDestination::Target(point) => Some(point),
            RetreatDestination::Pending | RetreatDestination::Unreachable => None,
        }
    }
}

fn retreat_cells(
    swarm: SwarmId,
    grid: &IntentGrid,
    engagements: &DefenderEngagements,
    radius: f32,
    reachability: Option<&SwarmReachabilityMap>,
) -> Vec<IVec2> {
    let mut cells: Vec<_> = grid
        .iter_active_cells()
        .filter_map(|(cell, intent)| intent.has_owned(IntentKind::Defend, swarm).then_some(cell))
        .collect();
    if cells.is_empty() {
        cells = grid.swarm_tiles(swarm);
    }
    cells.retain(|&cell| {
        reachability.is_none_or(|map| map.contains(swarm, cell, grid))
            && !engagements.hostile_near(get_world_from_zone(cell), swarm, radius)
    });
    cells
}

fn retreat_destination(
    cell: IVec2,
    position: Vec2,
    navigation: &Navigation,
    ground: Option<(&SwarmReachabilityMap, SwarmId, &IntentGrid)>,
) -> RetreatDestination {
    let center = get_world_from_zone(cell);
    let preferred = match ground {
        Some((map, swarm, grid)) => map.reachable_point(swarm, cell, grid),
        None => Some(center),
    };
    if let Some(point) = preferred.filter(|point| navigation.point_clear_for_entry(*point)) {
        if navigation.movement_clear(position, point) {
            return RetreatDestination::Target(point);
        }
        match navigation.query_connectivity(position, RouteGoal::Point(point)) {
            ConnectivityStatus::Connected { .. } => return RetreatDestination::Target(point),
            ConnectivityStatus::Pending => return RetreatDestination::Pending,
            ConnectivityStatus::Unreachable => {}
        }
    }

    let lower = cell.as_vec2() * ZONE_BLOCK_SIZE;
    let min = (lower / CELL_WIDTH).floor().as_ivec2();
    let max = ((lower + Vec2::splat(ZONE_BLOCK_SIZE)) / CELL_WIDTH)
        .ceil()
        .as_ivec2();
    let mut candidates = Vec::new();
    for y in min.y..max.y {
        for x in min.x..max.x {
            let point = (IVec2::new(x, y).as_vec2() + Vec2::splat(0.5)) * CELL_WIDTH;
            if world_to_cell(point) == cell
                && navigation.point_clear_for_entry(point)
                && ground
                    .is_none_or(|(map, swarm, grid)| map.position_is_reachable(swarm, point, grid))
            {
                candidates.push(point);
            }
        }
    }
    candidates.sort_by(|a, b| {
        center
            .distance_squared(*a)
            .total_cmp(&center.distance_squared(*b))
            .then_with(|| a.y.total_cmp(&b.y))
            .then_with(|| a.x.total_cmp(&b.x))
    });
    if let Some(&point) = candidates
        .iter()
        .find(|point| navigation.movement_clear(position, **point))
    {
        return RetreatDestination::Target(point);
    }
    for point in candidates {
        match navigation.query_connectivity(position, RouteGoal::Point(point)) {
            ConnectivityStatus::Connected { .. } => return RetreatDestination::Target(point),
            ConnectivityStatus::Pending => return RetreatDestination::Pending,
            ConnectivityStatus::Unreachable => {}
        }
    }
    RetreatDestination::Unreachable
}

fn retreat_movement(point: Vec2) -> DirectMovementComponent {
    DirectMovementComponent {
        xy: point,
        interaction: None,
        speed: None,
        stop_radius: 0.0,
    }
}

#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub fn defender_retreat_system(
    mut commands: Commands,
    mut engagements: ResMut<DefenderEngagements>,
    mut understrength_ticks: Local<EntityHashMap<u32>>,
    grid: Res<IntentGrid>,
    navigation: Res<Navigation>,
    reachability: Option<Res<SwarmReachabilityMap>>,
    tuning: Res<Tuning>,
    time: Res<Time<Fixed>>,
    mut bots: Query<
        (
            Entity,
            &Transform,
            &NanobotType,
            &SwarmMember,
            &Health,
            Option<&Charge>,
            Has<ChargerAssignment>,
            Has<ChargerProgress>,
            Has<ClearingEvacuation>,
            Has<DefenderDisengagement>,
            Option<&mut DefenderRout>,
            Option<&DirectMovementComponent>,
            Option<&CombatIdentity>,
        ),
        With<Nanobot>,
    >,
) {
    let tick = elapsed_fixed_ticks(&time);
    let radius = tuning.combat.rout_radius;
    let safe_distance = tuning.combat.attack_range + 2.0 * BODY_DIAMETER;
    let speed = tuning.logistics.bot_speed;
    let mut safe_cells = BTreeMap::new();
    understrength_ticks.retain(|entity, _| engagements.is_engaged(*entity));

    let mut ordered = Vec::new();
    for (entity, _, kind, _, health, _, _, _, _, disengaged, rout, _, identity) in &bots {
        if *kind == NanobotType::Defender && health.current > 0 {
            let engaged = engagements.is_engaged(entity);
            if engaged || disengaged || rout.is_some() {
                ordered.push((combat_key(entity, identity), entity, engaged));
            }
        }
    }
    ordered.sort_unstable();
    for (key, entity, engaged) in ordered {
        let (
            _,
            transform,
            _,
            member,
            _,
            charge,
            travelling,
            charging,
            evacuating,
            disengaged,
            rout,
            movement,
            _,
        ) = bots
            .get_mut(entity)
            .expect("Defender was in the retreat snapshot");
        let position = transform.translation.truncate();
        let swarm = member.0;
        let ground = reachability.as_deref().map(|map| (map, swarm, &*grid));
        let charge = charge.map_or(tuning.charge.max_charge, |charge| charge.current);
        let disengage = if disengaged {
            charge < tuning.charge.low_threshold
        } else {
            engaged && charge < tuning.charge.weakened_threshold
        };
        if disengage && !disengaged {
            let mut defender = commands.entity(entity);
            defender
                .remove::<(DefenderResponse, RegionalLease)>()
                .insert(DefenderDisengagement);
            if !evacuating {
                defender.remove::<DirectMovementComponent>();
            }
        } else if disengaged && !disengage {
            commands.entity(entity).remove::<DefenderDisengagement>();
        }

        if let Some(mut rout) = rout {
            if tick.saturating_sub(rout.started_tick) >= u64::from(tuning.combat.rout_min_ticks)
                && !engagements.hostile_near(position, swarm, 2.0 * radius)
            {
                commands
                    .entity(entity)
                    .remove::<(DefenderRout, DirectMovementComponent)>();
            } else if let Some(point) = rout.destination(position, &navigation, ground) {
                if movement.is_none_or(|movement| movement.xy != point) {
                    if position.distance_squared(point) > STOP_THRESHOLD.powi(2) {
                        commands.entity(entity).insert(retreat_movement(point));
                    } else if movement.is_some() && !evacuating {
                        commands.entity(entity).remove::<DirectMovementComponent>();
                    }
                }
            } else if movement.is_some() && !evacuating {
                commands.entity(entity).remove::<DirectMovementComponent>();
            }
            continue;
        }

        if engaged {
            if engagements.outnumbered(position, swarm, radius, tuning.combat.rout_ratio) {
                let ticks = understrength_ticks.entry(entity).or_default();
                *ticks = ticks.saturating_add(1);
                if *ticks >= tuning.combat.rout_confirm_ticks {
                    let cells = safe_cells.entry(swarm).or_insert_with(|| {
                        retreat_cells(swarm, &grid, &engagements, radius, reachability.as_deref())
                    });
                    if let Some(&cell) = cells.iter().min_by(|a, b| {
                        position
                            .distance_squared(get_world_from_zone(**a))
                            .total_cmp(&position.distance_squared(get_world_from_zone(**b)))
                            .then_with(|| (a.y, a.x).cmp(&(b.y, b.x)))
                    }) {
                        engagements.stop_engaged_movement(entity);
                        understrength_ticks.remove(&entity);
                        let mut rout = DefenderRout {
                            cell,
                            started_tick: tick,
                            destination: RetreatDestination::Pending,
                            navigation_revision: navigation.revision(),
                        };
                        let point = rout.destination(position, &navigation, ground);
                        let mut defender = commands.entity(entity);
                        defender
                            .remove::<(DefenderResponse, RegionalLease)>()
                            .insert(rout);
                        if let Some(point) = point {
                            defender.insert(retreat_movement(point));
                        } else if !evacuating {
                            defender.remove::<DirectMovementComponent>();
                        }
                        continue;
                    }
                }
            } else {
                understrength_ticks.remove(&entity);
            }
        }

        if !disengage || travelling || charging || evacuating {
            continue;
        }

        let nearest = engagements.nearest_hostile(position, swarm, safe_distance);
        let velocity = nearest.map_or(Vec2::ZERO, |(squared, hostile)| {
            let distance = squared.sqrt();
            let step = (safe_distance - distance).clamp(0.0, speed);
            -direction_to_hostile(position, key, hostile) * step
        });
        engagements.set_disengage_velocity(entity, velocity);
    }
}
