use std::{collections::HashMap, time::Duration};

use bevy::{ecs::message::MessageId, prelude::*};

use crate::tuning::{PresentationTuning, Tuning};

use super::{Nanobot, NanobotType, NanobotVisual, ResolvedCombatFact, rotation_for_direction};

#[derive(Clone, Copy, PartialEq)]
struct MotionSample {
    translation: Vec3,
    rotation: Quat,
}

impl From<&Transform> for MotionSample {
    fn from(transform: &Transform) -> Self {
        Self {
            translation: transform.translation,
            rotation: transform.rotation,
        }
    }
}

#[derive(Clone, Copy)]
struct FixedMotion {
    previous: MotionSample,
    current: MotionSample,
}

impl FixedMotion {
    fn new(sample: MotionSample) -> Self {
        Self {
            previous: sample,
            current: sample,
        }
    }

    fn advance(&mut self, sample: MotionSample, max_distance_squared: f32) {
        self.previous = if self
            .current
            .translation
            .distance_squared(sample.translation)
            > max_distance_squared
        {
            sample
        } else {
            self.current
        };
        self.current = sample;
    }

    fn translation(self, overstep: f32) -> Vec3 {
        if self.previous.translation == self.current.translation {
            self.current.translation
        } else {
            self.previous
                .translation
                .lerp(self.current.translation, overstep)
        }
    }

    fn rotation(self, overstep: f32) -> Quat {
        if self.previous.rotation == self.current.rotation {
            self.current.rotation
        } else {
            self.previous
                .rotation
                .slerp(self.current.rotation, overstep)
        }
    }
}

struct Aim {
    rotation: Quat,
    hit_at: Duration,
}

/// The nanobot's base world-space render pose, before jab and recoil offsets.
#[derive(Component, Default)]
pub struct PresentedNanobot {
    fixed: Option<FixedMotion>,
    translation: Vec3,
    rotation: Quat,
    aim: Option<Aim>,
    pub(super) impact_offset: Vec2,
}

impl PresentedNanobot {
    pub fn translation(&self) -> Vec3 {
        self.translation
    }

    /// The drawn sprite position including any active jab or recoil.
    pub fn sprite_translation(&self) -> Vec3 {
        self.translation + self.impact_offset.extend(0.0)
    }

    pub(super) fn fixed_death_position(&self, time: Option<&Time<Fixed>>) -> Vec2 {
        let overstep = time.map_or(1.0, |time| time.overstep_fraction().min(1.0));
        let translation = self
            .fixed
            .map_or(self.translation, |motion| motion.translation(overstep));
        translation.truncate() + self.impact_offset
    }

    pub fn rotation(&self) -> Quat {
        self.rotation
    }

    pub(super) fn new(transform: Option<&Transform>) -> Self {
        let Some(transform) = transform else {
            return Self::default();
        };
        Self {
            fixed: Some(FixedMotion::new(transform.into())),
            translation: transform.translation,
            rotation: transform.rotation,
            ..Self::default()
        }
    }

    pub(super) fn aim_at(&mut self, direction: Vec2, hit_at: Duration) {
        if let Some(rotation) = rotation_for_direction(direction) {
            self.aim = Some(Aim { rotation, hit_at });
        }
    }

    pub(super) fn update_translation(&mut self, transform: &Transform, fixed_overstep: f32) {
        let sample = MotionSample::from(transform);
        let fixed = self.fixed.get_or_insert_with(|| {
            self.rotation = transform.rotation;
            FixedMotion::new(sample)
        });
        // World placement can also change between fixed ticks.
        if fixed.current != sample {
            *fixed = FixedMotion::new(sample);
        }
        self.translation = fixed.translation(fixed_overstep);
    }

    pub(super) fn update_rotation(
        &mut self,
        kind: NanobotType,
        fixed_overstep: f32,
        (now, delta): (Duration, f32),
        settings: &PresentationTuning,
    ) {
        let moving_rotation = self
            .fixed
            .map_or(self.rotation, |fixed| fixed.rotation(fixed_overstep));

        if self.aim.as_ref().is_some_and(|aim| {
            now.saturating_sub(aim.hit_at).as_secs_f32() >= settings.aim_hold_secs
        }) {
            self.aim = None;
        }

        let desired = self
            .aim
            .as_ref()
            .map_or(moving_rotation, |aim| aim.rotation);
        let turn_rate = if kind == NanobotType::Defender {
            settings.defender_turn_rate
        } else {
            settings.nanobot_turn_rate
        };
        if self.rotation != desired && delta > 0.0 && turn_rate > 0.0 {
            let max_angle = turn_rate.to_radians() * delta;
            self.rotation = if self.rotation.angle_between(desired) <= max_angle {
                desired
            } else {
                self.rotation.rotate_towards(desired, max_angle)
            };
        }
    }
}

#[derive(SystemSet, Debug, Hash, PartialEq, Eq, Clone)]
pub(crate) struct UpdateNanobotPresentation;

struct DeathMotion {
    motion: FixedMotion,
    impact_offset: Vec2,
    rotation: Quat,
    scale: Vec3,
    sprite: Option<Sprite>,
}

#[derive(Resource, Default)]
pub(super) struct CombatTickSnapshots {
    hit_times: HashMap<MessageId<ResolvedCombatFact>, Duration>,
    deaths: HashMap<Entity, (Duration, Option<DeathMotion>)>,
    removed_structures: HashMap<Entity, (Transform, Sprite)>,
}

impl CombatTickSnapshots {
    pub(super) fn hit_time(&self, id: MessageId<ResolvedCombatFact>) -> Option<Duration> {
        self.hit_times.get(&id).copied()
    }

    pub(super) fn death_time(&self, entity: Entity) -> Option<Duration> {
        self.deaths.get(&entity).map(|(time, _)| *time)
    }

    pub(super) fn record_structure_death(
        &mut self,
        entity: Entity,
        transform: Transform,
        sprite: Sprite,
    ) {
        self.removed_structures.insert(entity, (transform, sprite));
    }

    pub(super) fn take_death_visual(
        &mut self,
        entity: Entity,
        time: Option<&Time<Fixed>>,
    ) -> Option<(Transform, Option<Sprite>)> {
        if let Some((fixed_time, Some(death))) = self.deaths.remove(&entity) {
            let overstep = time
                .filter(|time| time.elapsed() == fixed_time)
                .map_or(1.0, Time::overstep_fraction);
            let transform = Transform::from_translation(
                death.motion.translation(overstep) + death.impact_offset.extend(0.0),
            )
            .with_rotation(death.rotation)
            .with_scale(death.scale);
            Some((transform, death.sprite))
        } else {
            self.removed_structures
                .remove(&entity)
                .map(|(transform, sprite)| (transform, Some(sprite)))
        }
    }

    pub(super) fn clear(&mut self) {
        self.hit_times.clear();
        self.deaths.clear();
        self.removed_structures.clear();
    }
}

pub(super) fn capture_fixed_motion(
    mut bots: Query<(&Transform, &mut PresentedNanobot, Option<&Children>), With<Nanobot>>,
    visuals: Query<(&Transform, &Sprite), With<NanobotVisual>>,
    tuning: Res<Tuning>,
    time: Option<Res<Time<Fixed>>>,
    mut facts: MessageReader<ResolvedCombatFact>,
    mut snapshots: ResMut<CombatTickSnapshots>,
) {
    let max_distance_squared = (4.0 * tuning.logistics.bot_speed).powi(2);
    for (transform, mut pose, _) in &mut bots {
        let sample = MotionSample::from(transform);
        if let Some(fixed) = &mut pose.fixed {
            fixed.advance(sample, max_distance_squared);
        } else {
            pose.fixed = Some(FixedMotion::new(sample));
            pose.rotation = transform.rotation;
        }
    }

    let fixed_time = time.as_ref().map_or(Duration::ZERO, |time| time.elapsed());
    for (fact, id) in facts.read_with_id() {
        match fact {
            ResolvedCombatFact::Hit(_) => {
                snapshots.hit_times.insert(id, fixed_time);
            }
            ResolvedCombatFact::Death(death) => {
                let motion =
                    bots.get(death.victim.entity)
                        .ok()
                        .and_then(|(transform, pose, children)| {
                            let visual = children.and_then(|children| {
                                children.iter().find_map(|child| visuals.get(child).ok())
                            });
                            Some(DeathMotion {
                                motion: pose.fixed?,
                                impact_offset: pose.impact_offset,
                                rotation: pose.rotation,
                                scale: transform.scale
                                    * visual.map_or(Vec3::ONE, |(local, _)| local.scale),
                                sprite: visual.map(|(_, sprite)| sprite.clone()),
                            })
                        });
                snapshots
                    .deaths
                    .insert(death.victim.entity, (fixed_time, motion));
            }
        }
    }
}
