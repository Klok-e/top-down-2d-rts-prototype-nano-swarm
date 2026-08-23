//! Render-time presentation of facts already resolved by Defender combat.

use std::time::Duration;

use bevy::{ecs::query::QueryData, gizmos::GizmoPlugin, prelude::*};

use crate::nanobot::{
    CombatVisualSnapshot, NanobotVisual, ResolvedCombatFact, ResolvedCombatHit, SwarmId,
    rotation_for_direction,
};

/// Tunable combat-presentation values shared by windowed and offscreen apps.
#[derive(Debug, Clone, Copy, Resource)]
pub struct CombatPresentationSettings {
    pub pulse_duration: Duration,
    pub recovery_duration: Duration,
    pub jab_distance: f32,
    pub recoil_distance: f32,
    pub pulse_thickness: f32,
    pub zoom_cutoff: f32,
    pub max_decorative_effects: usize,
}

impl Default for CombatPresentationSettings {
    fn default() -> Self {
        Self {
            pulse_duration: Duration::from_millis(60),
            recovery_duration: Duration::from_millis(180),
            jab_distance: 12.0,
            recoil_distance: 10.0,
            pulse_thickness: 6.0,
            zoom_cutoff: 8.0,
            max_decorative_effects: 24,
        }
    }
}

/// One visible connection produced by a resolved hit.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CombatPulse {
    pub start: Vec2,
    pub end: Vec2,
    pub color: Color,
}

#[derive(Debug)]
struct ActiveCombatPulse {
    visual: CombatPulse,
    elapsed: Duration,
    just_started: bool,
}

/// Presentation-only pulse state, kept outside the gameplay entity allocator.
#[derive(Debug, Default, Resource)]
pub struct ActiveCombatPulses {
    active: Vec<ActiveCombatPulse>,
}

impl ActiveCombatPulses {
    pub fn len(&self) -> usize {
        self.active.len()
    }

    pub fn is_empty(&self) -> bool {
        self.active.is_empty()
    }

    pub fn iter(&self) -> impl ExactSizeIterator<Item = &CombatPulse> {
        self.active.iter().map(|pulse| &pulse.visual)
    }

    fn push(&mut self, visual: CombatPulse, limit: usize) {
        if limit == 0 {
            return;
        }
        let excess = self.active.len().saturating_add(1).saturating_sub(limit);
        if excess > 0 {
            self.active.drain(..excess);
        }
        self.active.push(ActiveCombatPulse {
            visual,
            elapsed: Duration::ZERO,
            just_started: true,
        });
    }
}

#[derive(Default, Reflect, GizmoConfigGroup)]
struct CombatGizmos;

#[derive(Debug, Clone, Copy)]
enum CombatPoseKind {
    Attack,
    Reaction,
}

#[derive(Debug, Component)]
struct CombatPose {
    kind: CombatPoseKind,
    world_direction: Vec2,
    elapsed: Duration,
    just_started: bool,
}

#[derive(QueryData)]
#[query_data(mutable)]
struct AnimatedCombatVisual {
    entity: Entity,
    parent: &'static ChildOf,
    transform: &'static mut Transform,
    sprite: &'static mut Sprite,
    pose: &'static mut CombatPose,
}

pub(crate) struct CombatPresentationPlugin;

impl Plugin for CombatPresentationPlugin {
    fn build(&self, app: &mut App) {
        if !app
            .world()
            .contains_resource::<Messages<ResolvedCombatFact>>()
        {
            app.add_message::<ResolvedCombatFact>();
        }
        app.init_resource::<CombatPresentationSettings>()
            .init_resource::<ActiveCombatPulses>();
        if app.is_plugin_added::<GizmoPlugin>() {
            app.init_gizmo_group::<CombatGizmos>().add_systems(
                Update,
                (
                    consume_resolved_combat,
                    animate_combat,
                    sync_combat_gizmo_settings,
                    draw_combat_pulses,
                )
                    .chain(),
            );
        } else {
            app.add_systems(Update, (consume_resolved_combat, animate_combat).chain());
        }
    }
}

fn faction_color(swarm: SwarmId) -> Color {
    if swarm.is_player() {
        Color::srgb(0.25, 0.72, 1.0)
    } else {
        Color::srgb(1.0, 0.28, 0.22)
    }
}

fn direction_between(attacker: CombatVisualSnapshot, target: CombatVisualSnapshot) -> Vec2 {
    (target.position - attacker.position).normalize_or(Vec2::X)
}

fn visual_for(
    root: Entity,
    visuals: &Query<(Entity, &ChildOf), With<NanobotVisual>>,
) -> Option<Entity> {
    visuals
        .iter()
        .find_map(|(entity, parent)| (parent.parent() == root).then_some(entity))
}

fn consume_resolved_combat(
    mut commands: Commands,
    mut facts: MessageReader<ResolvedCombatFact>,
    visuals: Query<(Entity, &ChildOf), With<NanobotVisual>>,
    settings: Res<CombatPresentationSettings>,
    mut pulses: ResMut<ActiveCombatPulses>,
) {
    for fact in facts.read() {
        let ResolvedCombatFact::Hit(hit) = *fact;
        present_hit(&mut commands, &visuals, &settings, &mut pulses, hit);
    }
}

fn present_hit(
    commands: &mut Commands,
    visuals: &Query<(Entity, &ChildOf), With<NanobotVisual>>,
    settings: &CombatPresentationSettings,
    pulses: &mut ActiveCombatPulses,
    hit: ResolvedCombatHit,
) {
    let direction = direction_between(hit.attacker, hit.target);
    if let Some(visual) = visual_for(hit.attacker.entity, visuals) {
        commands.entity(visual).insert(CombatPose {
            kind: CombatPoseKind::Attack,
            world_direction: direction,
            elapsed: Duration::ZERO,
            just_started: true,
        });
    }
    if let Some(visual) = visual_for(hit.target.entity, visuals) {
        commands.entity(visual).insert(CombatPose {
            kind: CombatPoseKind::Reaction,
            world_direction: direction,
            elapsed: Duration::ZERO,
            just_started: true,
        });
    }

    pulses.push(
        CombatPulse {
            start: hit.attacker.position,
            end: hit.target.position,
            color: faction_color(hit.attacker.swarm),
        },
        settings.max_decorative_effects,
    );
}

fn animate_combat(
    mut commands: Commands,
    time: Option<Res<Time>>,
    settings: Res<CombatPresentationSettings>,
    roots: Query<&Transform, Without<NanobotVisual>>,
    mut visuals: Query<AnimatedCombatVisual, With<NanobotVisual>>,
    mut pulses: ResMut<ActiveCombatPulses>,
) {
    let delta = time.as_ref().map_or(Duration::ZERO, |time| time.delta());
    for mut visual in &mut visuals {
        if visual.pose.just_started {
            visual.pose.just_started = false;
        } else {
            visual.pose.elapsed += delta;
        }
        let progress = (visual.pose.elapsed.as_secs_f32()
            / settings.recovery_duration.as_secs_f32())
        .clamp(0.0, 1.0);
        if progress >= 1.0 {
            *visual.transform = Transform::IDENTITY;
            visual.sprite.color = Color::WHITE;
            commands.entity(visual.entity).remove::<CombatPose>();
            continue;
        }

        let strength = 1.0 - progress;
        let Ok(root) = roots.get(visual.parent.parent()) else {
            continue;
        };
        let local_direction =
            (root.rotation.inverse() * visual.pose.world_direction.extend(0.0)).truncate();
        match visual.pose.kind {
            CombatPoseKind::Attack => {
                visual.transform.translation =
                    (local_direction * settings.jab_distance * strength).extend(0.0);
                let world_facing =
                    rotation_for_direction(visual.pose.world_direction).unwrap_or(Quat::IDENTITY);
                visual.transform.rotation = root.rotation.inverse() * world_facing;
                visual.sprite.color = Color::WHITE;
            }
            CombatPoseKind::Reaction => {
                visual.transform.translation =
                    (local_direction * settings.recoil_distance * strength).extend(0.0);
                visual.transform.rotation = Quat::IDENTITY;
                visual.sprite.color = Color::srgb(1.0, 1.0, 1.0 - 0.65 * strength);
            }
        }
    }

    pulses.active.retain_mut(|pulse| {
        if pulse.just_started {
            pulse.just_started = false;
        } else {
            pulse.elapsed += delta;
        }
        pulse.elapsed < settings.pulse_duration
    });
}

fn sync_combat_gizmo_settings(
    settings: Res<CombatPresentationSettings>,
    mut configs: ResMut<GizmoConfigStore>,
) {
    let (config, _) = configs.config_mut::<CombatGizmos>();
    config.line.width = settings.pulse_thickness;
}

fn draw_combat_pulses(pulses: Res<ActiveCombatPulses>, mut gizmos: Gizmos<CombatGizmos>) {
    for pulse in pulses.iter() {
        gizmos.line_2d(pulse.start, pulse.end, pulse.color);
    }
}
