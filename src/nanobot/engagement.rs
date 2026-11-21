use bevy::{ecs::entity::EntityHashMap, prelude::*};

use super::{
    Charge, ChargerAssignment, ChargerProgress, ClearingEvacuation, CombatIdentity, DefenderRout,
    Health, Nanobot, NanobotType, SwarmId, SwarmMember, charge_strength_multiplier,
    combat::combat_hash,
    combat_key,
    move_system::{BODY_DIAMETER, coincident_pair_direction},
};
use crate::{spatial::FixedSpatialBuckets, tuning::Tuning};

const FRIENDLY_DENSITY_RADIUS: f32 = BODY_DIAMETER * 2.0;

#[derive(Clone, Copy)]
pub(super) struct EngagementDefender {
    pub(super) entity: Entity,
    pub(super) key: u64,
    pub(super) position: Vec2,
    pub(super) swarm: SwarmId,
    pub(super) strength: f64,
    pub(super) routing: bool,
    using_charger: bool,
    evacuating: bool,
}

#[derive(Clone, Copy)]
struct Engagement {
    defender: EngagementDefender,
    toward_hostile: Vec2,
    velocity: Vec2,
}

#[derive(Default, Resource)]
pub struct DefenderEngagements {
    contacts: EntityHashMap<Engagement>,
    defenders: Option<FixedSpatialBuckets<EngagementDefender>>,
    bucket_size: f32,
    disengage_velocities: EntityHashMap<Vec2>,
}

impl DefenderEngagements {
    pub(super) fn is_engaged(&self, entity: Entity) -> bool {
        self.contacts.contains_key(&entity)
    }

    pub(super) fn controls_movement(&self, entity: Entity) -> bool {
        self.disengage_velocities.contains_key(&entity)
            || self
                .contacts
                .get(&entity)
                .is_some_and(|engagement| !engagement.defender.evacuating)
    }

    pub(super) fn stop_engaged_movement(&mut self, entity: Entity) {
        self.contacts.remove(&entity);
    }

    pub(super) fn set_disengage_velocity(&mut self, entity: Entity, velocity: Vec2) {
        self.disengage_velocities.insert(entity, velocity);
    }

    pub(super) fn defenders_near(
        &self,
        position: Vec2,
        radius: f32,
    ) -> impl Iterator<Item = &EngagementDefender> {
        let radius_squared = radius * radius;
        let bucket_radius = (radius / self.bucket_size).ceil() as i32;
        self.defenders
            .iter()
            .flat_map(move |buckets| {
                buckets.neighbourhood(buckets.bucket_for_position(position), bucket_radius)
            })
            .flat_map(|(_, entries)| entries)
            .filter(move |defender| position.distance_squared(defender.position) <= radius_squared)
    }

    pub(super) fn hostiles_near(
        &self,
        position: Vec2,
        swarm: SwarmId,
        radius: f32,
    ) -> impl Iterator<Item = &EngagementDefender> {
        self.defenders_near(position, radius)
            .filter(move |defender| defender.swarm != swarm)
    }

    pub(super) fn velocity_with_separation(
        &self,
        entity: Entity,
        current: Vec2,
        push: Vec2,
    ) -> Vec2 {
        if let Some(retreat) = self.disengage_velocities.get(&entity) {
            return *retreat + push;
        }
        let Some(engagement) = self.contacts.get(&entity) else {
            return current + push;
        };
        let movement = if engagement.defender.evacuating {
            current
        } else {
            engagement.velocity
        };
        movement + self.redirect_separation(engagement, push)
    }

    fn redirect_separation(&self, engagement: &Engagement, push: Vec2) -> Vec2 {
        let toward = engagement.toward_hostile;
        let forward = push.dot(toward);
        if forward <= 0.0 {
            return push;
        }

        let defender = engagement.defender;
        let entity = defender.entity;
        let tangent = toward.perp();
        let buckets = self
            .defenders
            .as_ref()
            .expect("engagement indexes its Defender");
        let center = buckets.bucket_for_position(defender.position);
        let mut positive = 0;
        let mut negative = 0;
        for (_, neighbours) in buckets.neighbourhood(center, 1) {
            for other in neighbours {
                let offset = other.position - defender.position;
                if other.entity != entity
                    && other.swarm == defender.swarm
                    && offset.length_squared() <= FRIENDLY_DENSITY_RADIUS.powi(2)
                {
                    let side = offset.dot(tangent);
                    positive += usize::from(side > 0.0);
                    negative += usize::from(side < 0.0);
                }
            }
        }
        let side = match positive.cmp(&negative) {
            std::cmp::Ordering::Less => 1.0,
            std::cmp::Ordering::Greater => -1.0,
            std::cmp::Ordering::Equal => hash_side(defender.key, 0),
        };
        tangent * (push.dot(tangent) + side * forward)
    }
}

pub(super) fn direction_to_hostile(position: Vec2, key: u64, hostile: &EngagementDefender) -> Vec2 {
    let offset = hostile.position - position;
    if let Some(direction) = offset.try_normalize() {
        direction
    } else if key < hostile.key {
        -coincident_pair_direction(key, hostile.key)
    } else {
        coincident_pair_direction(hostile.key, key)
    }
}

fn hash_side(key: u64, tick: u64) -> f32 {
    if combat_hash(key, tick) & 1 == 0 {
        1.0
    } else {
        -1.0
    }
}

#[allow(clippy::type_complexity)]
pub fn defender_engagement_system(
    bots: Query<
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
            Has<DefenderRout>,
            Option<&CombatIdentity>,
        ),
        With<Nanobot>,
    >,
    mut engagements: ResMut<DefenderEngagements>,
    tuning: Res<Tuning>,
    time: Res<Time<Fixed>>,
) {
    engagements.contacts.clear();
    engagements.disengage_velocities.clear();
    engagements.defenders = None;
    let mut defenders: Vec<_> = bots
        .iter()
        .filter(|(_, _, kind, _, health, ..)| **kind == NanobotType::Defender && health.current > 0)
        .map(
            |(
                entity,
                transform,
                _,
                member,
                health,
                charge,
                travelling,
                charging,
                evacuating,
                routing,
                identity,
            )| {
                EngagementDefender {
                    entity,
                    key: combat_key(entity, identity),
                    position: transform.translation.truncate(),
                    swarm: member.0,
                    strength: f64::from(health.current) / f64::from(health.max.max(1))
                        * f64::from(charge_strength_multiplier(
                            charge.map_or(tuning.charge.max_charge, |charge| charge.current),
                            &tuning.charge,
                        )),
                    using_charger: travelling || charging,
                    evacuating,
                    routing,
                }
            },
        )
        .collect();
    let Some(first) = defenders.first() else {
        return;
    };
    if defenders
        .iter()
        .all(|defender| defender.swarm == first.swarm)
    {
        return;
    }
    let range = tuning.combat.attack_range * tuning.combat.engage_range_factor;
    let range_squared = range * range;
    let outer = (tuning.combat.attack_range - 4.0).max(0.0);
    let inner = (tuning.combat.attack_range - tuning.combat.band_depth).clamp(0.0, outer);
    let speed = tuning.logistics.bot_speed;
    let tick = crate::tuning::elapsed_fixed_ticks(&time);
    let bucket_size = range.max(FRIENDLY_DENSITY_RADIUS);
    let mut buckets = FixedSpatialBuckets::new(bucket_size);
    defenders.sort_unstable_by_key(|defender| (defender.key, defender.entity));
    for defender in &defenders {
        buckets.insert(defender.position, *defender);
    }
    for defender in defenders
        .into_iter()
        .filter(|defender| !defender.using_charger && !defender.routing)
    {
        let center = buckets.bucket_for_position(defender.position);
        let Some((distance_squared, _, hostile)) = buckets
            .neighbourhood(center, 1)
            .flat_map(|(_, entries)| entries)
            .filter(|other| other.swarm != defender.swarm)
            .filter_map(|other| {
                let distance = defender.position.distance_squared(other.position);
                (distance <= range_squared).then_some((distance, other.key, other))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.cmp(&b.1)))
        else {
            continue;
        };
        let distance = distance_squared.sqrt();
        let toward_hostile = direction_to_hostile(defender.position, defender.key, hostile);
        let velocity = if distance > outer {
            toward_hostile * speed
        } else if distance < inner {
            -toward_hostile * speed
        } else {
            let period = 60 + combat_hash(defender.key, 0) % 61;
            toward_hostile.perp()
                * (speed
                    * tuning.combat.strafe_speed_fraction
                    * hash_side(defender.key, tick / period))
        };
        engagements.contacts.insert(
            defender.entity,
            Engagement {
                defender,
                toward_hostile,
                velocity,
            },
        );
    }
    engagements.bucket_size = bucket_size;
    engagements.defenders = Some(buckets);
}
