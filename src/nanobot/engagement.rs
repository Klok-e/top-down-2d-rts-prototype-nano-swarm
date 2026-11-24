use bevy::{ecs::entity::EntityHashMap, prelude::*};

use super::{
    Charge, ChargerAssignment, ChargerProgress, ClearingEvacuation, CombatIdentity, DefenderRout,
    Health, Nanobot, NanobotType, SwarmId, SwarmMember, charge_strength_multiplier,
    combat::combat_hash,
    combat_key,
    move_system::{BODY_DIAMETER, coincident_pair_direction},
};
use crate::tuning::Tuning;

mod index;

use index::DefenderIndex;

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
    defenders: Option<DefenderIndex>,
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

    /// Whether a Defender outside `swarm` is within `radius` of `position`.
    pub(super) fn hostile_near(&self, position: Vec2, swarm: SwarmId, radius: f32) -> bool {
        self.defenders
            .as_ref()
            .is_some_and(|defenders| defenders.any_hostile(position, swarm, radius))
    }

    /// The closest Defender outside `swarm` within `radius`, with its squared
    /// distance; equal distances break by combat key.
    pub(super) fn nearest_hostile(
        &self,
        position: Vec2,
        swarm: SwarmId,
        radius: f32,
    ) -> Option<(f32, &EngagementDefender)> {
        self.defenders
            .as_ref()?
            .nearest_hostile(position, swarm, radius, None)
    }

    /// Whether Defender strength within `radius` leaves `swarm` below `ratio` of
    /// the hostile strength there. Routing friends add no strength.
    pub(super) fn outnumbered(
        &self,
        position: Vec2,
        swarm: SwarmId,
        radius: f32,
        ratio: f32,
    ) -> bool {
        self.defenders
            .as_ref()
            .is_some_and(|defenders| defenders.outnumbered(position, swarm, radius, ratio))
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
        let tangent = toward.perp();
        let (positive, negative) = self
            .defenders
            .as_ref()
            .expect("engagement indexes its Defender")
            .friendly_sides(&defender, FRIENDLY_DENSITY_RADIUS, tangent);
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
    let outer = (tuning.combat.attack_range - 4.0).max(0.0);
    let inner = (tuning.combat.attack_range - tuning.combat.band_depth).clamp(0.0, outer);
    let speed = tuning.logistics.bot_speed;
    let tick = crate::tuning::elapsed_fixed_ticks(&time);
    let bucket_size = range.max(FRIENDLY_DENSITY_RADIUS);
    defenders.sort_unstable_by_key(|defender| (defender.key, defender.entity));
    let index = DefenderIndex::new(defenders, bucket_size);
    for &defender in index
        .defenders()
        .iter()
        .filter(|defender| !defender.using_charger && !defender.routing)
    {
        let Some((distance_squared, hostile)) =
            index.nearest_hostile(defender.position, defender.swarm, range, Some(1))
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
    engagements.defenders = Some(index);
}
