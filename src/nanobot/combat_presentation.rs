//! Render-time presentation of facts already resolved by Defender combat.

use std::{collections::HashSet, time::Duration};

use bevy::{
    ecs::query::QueryData,
    gizmos::GizmoPlugin,
    prelude::*,
    render::{
        Extract, ExtractSchedule, RenderApp, sync_world::TemporaryRenderEntity,
        view::RenderVisibleEntities,
    },
    sprite_render::{ExtractedSprite, ExtractedSpriteKind, ExtractedSprites, SpriteSystems},
};

use crate::{
    GAMEPLAY_SPRITE_Z,
    nanobot::{
        CombatAppearance, CombatVisualSnapshot, NanobotSprites, NanobotVisual, ResolvedCombatDeath,
        ResolvedCombatFact, ResolvedCombatHit, SwarmId, rotation_for_direction,
    },
};

/// Tunable combat-presentation values shared by windowed and offscreen apps.
#[derive(Debug, Clone, Copy, Resource)]
pub struct CombatPresentationSettings {
    pub pulse_duration: Duration,
    pub recovery_duration: Duration,
    pub death_duration: Duration,
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
            death_duration: Duration::from_millis(320),
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

/// Public render state for one nanobot destroyed by resolved combat.
#[derive(Debug, Clone, PartialEq)]
pub struct NanobotDeathGhost {
    pub victim: CombatVisualSnapshot,
    pub transform: Transform,
    pub color: Color,
    pub image: Handle<Image>,
}

#[derive(Debug)]
struct ActiveNanobotDeathGhost {
    visual: NanobotDeathGhost,
    elapsed: Duration,
    just_started: bool,
}

/// Presentation-only death state kept outside the gameplay entity allocator.
#[derive(Debug, Default, Resource)]
pub struct ActiveNanobotDeathGhosts {
    active: Vec<ActiveNanobotDeathGhost>,
}

impl ActiveNanobotDeathGhosts {
    pub fn len(&self) -> usize {
        self.active.len()
    }

    pub fn is_empty(&self) -> bool {
        self.active.is_empty()
    }

    pub fn iter(&self) -> impl ExactSizeIterator<Item = &NanobotDeathGhost> {
        self.active.iter().map(|ghost| &ghost.visual)
    }

    fn push(&mut self, visual: NanobotDeathGhost) {
        self.active.push(ActiveNanobotDeathGhost {
            visual,
            elapsed: Duration::ZERO,
            just_started: true,
        });
    }
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
            .init_resource::<ActiveCombatPulses>()
            .init_resource::<ActiveNanobotDeathGhosts>();
        if let Some(render_app) = app.get_sub_app_mut(RenderApp) {
            render_app.add_systems(
                ExtractSchedule,
                extract_nanobot_death_ghosts.after(SpriteSystems::ExtractSprites),
            );
        }
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
    sprites: Res<NanobotSprites>,
    settings: Res<CombatPresentationSettings>,
    mut pulses: ResMut<ActiveCombatPulses>,
    mut ghosts: ResMut<ActiveNanobotDeathGhosts>,
) {
    for fact in facts.read() {
        match *fact {
            ResolvedCombatFact::Hit(hit) => {
                present_hit(&mut commands, &visuals, &settings, &mut pulses, hit);
            }
            ResolvedCombatFact::Death(death) => {
                present_death(&sprites, &mut ghosts, death);
            }
        }
    }
}

fn present_death(
    sprites: &NanobotSprites,
    ghosts: &mut ActiveNanobotDeathGhosts,
    death: ResolvedCombatDeath,
) {
    let CombatAppearance::Nanobot(kind) = death.victim.appearance else {
        return;
    };
    ghosts.push(NanobotDeathGhost {
        victim: death.victim,
        transform: Transform::from_translation(death.victim.position.extend(GAMEPLAY_SPRITE_Z)),
        color: Color::WHITE,
        image: sprites.handle(kind, !death.victim.swarm.is_player()),
    });
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

fn transient_progress(
    elapsed: &mut Duration,
    just_started: &mut bool,
    delta: Duration,
    duration: Duration,
) -> f32 {
    if *just_started {
        *just_started = false;
    } else {
        *elapsed += delta;
    }
    (elapsed.as_secs_f32() / duration.as_secs_f32()).clamp(0.0, 1.0)
}

fn animate_combat(
    mut commands: Commands,
    time: Option<Res<Time>>,
    settings: Res<CombatPresentationSettings>,
    roots: Query<&Transform, Without<NanobotVisual>>,
    mut visuals: Query<AnimatedCombatVisual, With<NanobotVisual>>,
    mut ghosts: ResMut<ActiveNanobotDeathGhosts>,
    mut pulses: ResMut<ActiveCombatPulses>,
) {
    let delta = time.as_ref().map_or(Duration::ZERO, |time| time.delta());
    for mut visual in &mut visuals {
        let pose = &mut *visual.pose;
        let progress = transient_progress(
            &mut pose.elapsed,
            &mut pose.just_started,
            delta,
            settings.recovery_duration,
        );
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

    ghosts.active.retain_mut(|ghost| {
        let progress = transient_progress(
            &mut ghost.elapsed,
            &mut ghost.just_started,
            delta,
            settings.death_duration,
        );
        if progress >= 1.0 {
            return false;
        }

        ghost.visual.transform.scale = Vec3::new(1.0 - 0.25 * progress, 1.0 - 0.9 * progress, 1.0);
        let faction = faction_color(ghost.visual.victim.swarm).to_srgba();
        let flash = 0.75 * (1.0 - progress / 0.25).clamp(0.0, 1.0);
        let alpha = (1.0 - (progress - 0.2).max(0.0) / 0.8).clamp(0.0, 1.0);
        ghost.visual.color = Color::srgba(
            faction.red + (1.0 - faction.red) * flash,
            faction.green + (1.0 - faction.green) * flash,
            faction.blue + (1.0 - faction.blue) * flash,
            alpha,
        );
        true
    });

    pulses.active.retain_mut(|pulse| {
        transient_progress(
            &mut pulse.elapsed,
            &mut pulse.just_started,
            delta,
            settings.pulse_duration,
        ) < 1.0
    });
}

fn extract_nanobot_death_ghosts(
    mut commands: Commands,
    ghosts: Extract<Res<ActiveNanobotDeathGhosts>>,
    views: Query<&RenderVisibleEntities>,
    mut extracted_sprites: ResMut<ExtractedSprites>,
) {
    let visible_sprite_anchors = views
        .iter()
        .filter_map(|visible| visible.iter::<Sprite>().next().map(|(_, main)| **main))
        .collect::<HashSet<_>>();
    for main_entity in visible_sprite_anchors {
        for ghost in ghosts.iter() {
            extracted_sprites.sprites.push(ExtractedSprite {
                main_entity,
                render_entity: commands.spawn(TemporaryRenderEntity).id(),
                transform: GlobalTransform::from(ghost.transform),
                color: ghost.color.into(),
                image_handle_id: ghost.image.id(),
                flip_x: false,
                flip_y: false,
                kind: ExtractedSpriteKind::Single {
                    anchor: Vec2::ZERO,
                    rect: None,
                    scaling_mode: None,
                    custom_size: None,
                },
            });
        }
    }
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
