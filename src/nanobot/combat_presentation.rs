//! Render-time presentation of facts already resolved by Defender combat.

use std::{
    collections::{HashMap, HashSet},
    time::Duration,
};

use bevy::{
    ecs::{query::QueryData, system::SystemParam},
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
    fly_camera::CameraZoom2d,
    nanobot::{
        CombatAppearance, CombatVisualSnapshot, NanobotSprites, NanobotVisual, ResolvedCombatDeath,
        ResolvedCombatFact, ResolvedCombatHit, SwarmId, completed_visual_color,
        rotation_for_direction,
    },
    structure_sprites::{StructureSprites, StructureVisual},
};

/// Tunable combat-presentation values shared by windowed and offscreen apps.
#[derive(Debug, Clone, Copy, Resource)]
pub struct CombatPresentationSettings {
    pub pulse_duration: Duration,
    pub recovery_duration: Duration,
    pub death_duration: Duration,
    pub jab_distance: f32,
    pub recoil_distance: f32,
    /// Minimum visible jab and recoil displacement in screen pixels.
    pub minimum_impact_screen_distance: f32,
    pub reaction_flash_per_hit: f32,
    pub reaction_flash_cap: f32,
    /// Pulse width in screen pixels.
    pub pulse_thickness: f32,
    pub decoration_length: f32,
    /// Impact-decoration width in screen pixels.
    pub decoration_thickness: f32,
    pub structure_ring_start_radius: f32,
    pub structure_ring_end_radius: f32,
    /// Structure-destruction ring width in screen pixels.
    pub structure_ring_thickness: f32,
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
            minimum_impact_screen_distance: 2.0,
            reaction_flash_per_hit: 0.65,
            reaction_flash_cap: 0.9,
            pulse_thickness: 3.0,
            decoration_length: 10.0,
            decoration_thickness: 2.0,
            structure_ring_start_radius: 18.0,
            structure_ring_end_radius: 42.0,
            structure_ring_thickness: 3.0,
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

/// One bounded secondary impact mark produced by a resolved hit.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CombatDecoration {
    pub start: Vec2,
    pub end: Vec2,
    pub color: Color,
}

#[derive(Debug)]
struct ActiveCombatDecoration {
    visual: CombatDecoration,
    elapsed: Duration,
    just_started: bool,
}

/// Presentation-only secondary combat effects, bounded independently of pulses.
#[derive(Debug, Default, Resource)]
pub struct ActiveCombatDecorations {
    active: Vec<ActiveCombatDecoration>,
}

/// Public render state for one nanobot destroyed by resolved combat.
#[derive(Debug, Clone, PartialEq)]
pub struct NanobotDeathGhost {
    pub victim: CombatVisualSnapshot,
    pub transform: Transform,
    pub color: Color,
    pub image: Handle<Image>,
}

/// Public render state for one support structure destroyed by resolved combat.
#[derive(Debug, Clone, PartialEq)]
pub struct StructureDeathGhost {
    pub victim: CombatVisualSnapshot,
    pub transform: Transform,
    pub color: Color,
    pub image: Handle<Image>,
    pub ring_radius: f32,
    pub ring_color: Color,
}

#[derive(Debug)]
struct ActiveDeathGhost<T> {
    visual: T,
    elapsed: Duration,
    just_started: bool,
}

/// Presentation-only death state kept outside gameplay entity allocators.
#[derive(Debug, Resource)]
pub struct ActiveDeathGhosts<T: Send + Sync + 'static> {
    active: Vec<ActiveDeathGhost<T>>,
}

pub type ActiveNanobotDeathGhosts = ActiveDeathGhosts<NanobotDeathGhost>;
pub type ActiveStructureDeathGhosts = ActiveDeathGhosts<StructureDeathGhost>;

impl<T: Send + Sync + 'static> Default for ActiveDeathGhosts<T> {
    fn default() -> Self {
        Self { active: Vec::new() }
    }
}

impl<T: Send + Sync + 'static> ActiveDeathGhosts<T> {
    pub fn len(&self) -> usize {
        self.active.len()
    }

    pub fn is_empty(&self) -> bool {
        self.active.is_empty()
    }

    pub fn iter(&self) -> impl ExactSizeIterator<Item = &T> {
        self.active.iter().map(|ghost| &ghost.visual)
    }

    fn push(&mut self, visual: T) {
        self.active.push(ActiveDeathGhost {
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
        self.len() == 0
    }

    pub fn iter(&self) -> impl ExactSizeIterator<Item = &CombatPulse> {
        self.active.iter().map(|pulse| &pulse.visual)
    }

    fn push(&mut self, visual: CombatPulse) {
        self.active.push(ActiveCombatPulse {
            visual,
            elapsed: Duration::ZERO,
            just_started: true,
        });
    }
}

impl ActiveCombatDecorations {
    pub fn len(&self) -> usize {
        self.active.len()
    }

    pub fn is_empty(&self) -> bool {
        self.active.is_empty()
    }

    pub fn iter(&self) -> impl ExactSizeIterator<Item = &CombatDecoration> {
        self.active.iter().map(|decoration| &decoration.visual)
    }

    fn push(&mut self, visual: CombatDecoration, limit: usize) {
        if limit == 0 {
            return;
        }
        let excess = self.active.len().saturating_add(1).saturating_sub(limit);
        if excess > 0 {
            self.active.drain(..excess);
        }
        self.active.push(ActiveCombatDecoration {
            visual,
            elapsed: Duration::ZERO,
            just_started: true,
        });
    }
}

#[derive(Debug, Resource)]
struct CombatPresentationView {
    zoom: f32,
    visible: bool,
}

impl Default for CombatPresentationView {
    fn default() -> Self {
        Self {
            zoom: 1.0,
            visible: true,
        }
    }
}

#[derive(Default, Reflect, GizmoConfigGroup)]
struct CombatGizmos;

#[derive(Default, Reflect, GizmoConfigGroup)]
struct CombatDecorationGizmos;

#[derive(Default, Reflect, GizmoConfigGroup)]
struct StructureDeathRingGizmos;

#[derive(Debug, Component)]
struct CombatPose {
    attack_direction: Option<Vec2>,
    reaction_direction: Option<Vec2>,
    reaction_flash: f32,
    elapsed: Duration,
    just_started: bool,
}

#[derive(Debug, Default)]
struct PendingCombatPose {
    attack_directions: Vec<Vec2>,
    reaction_directions: Vec<Vec2>,
}

#[derive(Debug, Component)]
struct StructureCombatFlash {
    neutral_color: Color,
    intensity: f32,
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

#[derive(QueryData)]
#[query_data(mutable)]
struct AnimatedStructureVisual {
    entity: Entity,
    sprite: &'static mut Sprite,
    flash: &'static mut StructureCombatFlash,
}

#[derive(SystemParam)]
struct CombatFactParticipants<'w, 's> {
    visuals: Query<'w, 's, (Entity, &'static ChildOf), With<NanobotVisual>>,
    structures: Query<
        'w,
        's,
        (&'static Sprite, Option<&'static StructureCombatFlash>),
        With<StructureVisual>,
    >,
    nanobot_sprites: Res<'w, NanobotSprites>,
    structure_sprites: Option<Res<'w, StructureSprites>>,
}

#[derive(SystemParam)]
struct AnimatedCombatEntities<'w, 's> {
    roots: Query<'w, 's, &'static Transform, Without<NanobotVisual>>,
    visuals: Query<'w, 's, AnimatedCombatVisual, (With<NanobotVisual>, Without<StructureVisual>)>,
    structures:
        Query<'w, 's, AnimatedStructureVisual, (With<StructureVisual>, Without<NanobotVisual>)>,
}

#[derive(SystemParam)]
struct CombatTransients<'w> {
    pulses: ResMut<'w, ActiveCombatPulses>,
    decorations: ResMut<'w, ActiveCombatDecorations>,
    ghosts: ResMut<'w, ActiveNanobotDeathGhosts>,
    structure_ghosts: ResMut<'w, ActiveStructureDeathGhosts>,
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
            .init_resource::<ActiveCombatDecorations>()
            .init_resource::<CombatPresentationView>()
            .init_resource::<ActiveNanobotDeathGhosts>()
            .init_resource::<ActiveStructureDeathGhosts>();
        if let Some(render_app) = app.get_sub_app_mut(RenderApp) {
            render_app.add_systems(
                ExtractSchedule,
                extract_combat_death_ghosts.after(SpriteSystems::ExtractSprites),
            );
        }
        if app.is_plugin_added::<GizmoPlugin>() {
            app.init_gizmo_group::<CombatGizmos>()
                .init_gizmo_group::<CombatDecorationGizmos>()
                .init_gizmo_group::<StructureDeathRingGizmos>()
                .add_systems(
                    Update,
                    (
                        update_combat_view,
                        consume_resolved_combat,
                        animate_combat,
                        sync_combat_gizmo_settings,
                        draw_combat_pulses,
                        draw_combat_decorations,
                        draw_structure_death_rings,
                    )
                        .chain(),
                );
        } else {
            app.add_systems(
                Update,
                (update_combat_view, consume_resolved_combat, animate_combat).chain(),
            );
        }
    }
}

fn update_combat_view(
    settings: Res<CombatPresentationSettings>,
    zooms: Query<&CameraZoom2d>,
    mut view: ResMut<CombatPresentationView>,
) {
    view.zoom = zooms.iter().next().map_or(1.0, |zoom| zoom.zoom);
    view.visible = view.zoom < settings.zoom_cutoff;
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
    participants: CombatFactParticipants,
    settings: Res<CombatPresentationSettings>,
    transients: CombatTransients,
) {
    let CombatFactParticipants {
        visuals,
        structures,
        nanobot_sprites,
        structure_sprites,
    } = participants;
    let CombatTransients {
        mut pulses,
        mut decorations,
        mut ghosts,
        mut structure_ghosts,
    } = transients;
    let mut hits = Vec::new();
    let mut deaths = Vec::new();
    for fact in facts.read() {
        match *fact {
            ResolvedCombatFact::Hit(hit) => hits.push(hit),
            ResolvedCombatFact::Death(death) => deaths.push(death),
        }
    }
    present_hits(
        &mut commands,
        &visuals,
        &structures,
        &settings,
        &mut pulses,
        &mut decorations,
        hits,
    );
    for death in deaths {
        present_death(
            &nanobot_sprites,
            structure_sprites.as_deref(),
            &settings,
            &mut ghosts,
            &mut structure_ghosts,
            death,
        );
    }
}

fn present_death(
    sprites: &NanobotSprites,
    structure_sprites: Option<&StructureSprites>,
    settings: &CombatPresentationSettings,
    ghosts: &mut ActiveNanobotDeathGhosts,
    structure_ghosts: &mut ActiveStructureDeathGhosts,
    death: ResolvedCombatDeath,
) {
    match death.victim.appearance {
        CombatAppearance::Nanobot(kind) => ghosts.push(NanobotDeathGhost {
            victim: death.victim,
            transform: Transform::from_translation(death.victim.position.extend(GAMEPLAY_SPRITE_Z)),
            color: Color::WHITE,
            image: sprites.handle(kind, !death.victim.swarm.is_player()),
        }),
        CombatAppearance::Structure(appearance) => {
            let (Some(structure_sprites), Some(visual)) = (structure_sprites, appearance.visual)
            else {
                return;
            };
            structure_ghosts.push(StructureDeathGhost {
                victim: death.victim,
                transform: Transform::from_translation(
                    death.victim.position.extend(GAMEPLAY_SPRITE_Z),
                ),
                color: Color::WHITE,
                image: structure_sprites.handle(visual.kind, visual.state),
                ring_radius: settings.structure_ring_start_radius.max(0.0),
                ring_color: faction_color(death.victim.swarm),
            });
        }
    }
}

fn aggregate_direction(directions: &mut [Vec2]) -> Option<Vec2> {
    directions.first()?;
    directions.sort_by(|left, right| {
        left.x
            .total_cmp(&right.x)
            .then_with(|| left.y.total_cmp(&right.y))
    });
    directions
        .iter()
        .copied()
        .fold(Vec2::ZERO, |sum, direction| sum + direction)
        .try_normalize()
}

fn impact_world_distance(configured: f32, zoom: f32, minimum_screen_distance: f32) -> f32 {
    configured
        .max(0.0)
        .max(zoom.max(0.0) * minimum_screen_distance.max(0.0))
}

fn present_hits(
    commands: &mut Commands,
    visuals: &Query<(Entity, &ChildOf), With<NanobotVisual>>,
    structures: &Query<(&Sprite, Option<&StructureCombatFlash>), With<StructureVisual>>,
    settings: &CombatPresentationSettings,
    pulses: &mut ActiveCombatPulses,
    decorations: &mut ActiveCombatDecorations,
    hits: Vec<ResolvedCombatHit>,
) {
    let mut poses = HashMap::<Entity, PendingCombatPose>::new();
    let mut structure_hit_counts = HashMap::<Entity, usize>::new();
    for hit in hits {
        let direction = direction_between(hit.attacker, hit.target);
        if let Some(visual) = visual_for(hit.attacker.entity, visuals) {
            poses
                .entry(visual)
                .or_default()
                .attack_directions
                .push(direction);
        }
        if let Some(visual) = visual_for(hit.target.entity, visuals) {
            poses
                .entry(visual)
                .or_default()
                .reaction_directions
                .push(direction);
        } else if matches!(hit.target.appearance, CombatAppearance::Structure(_))
            && structures.get(hit.target.entity).is_ok()
        {
            *structure_hit_counts.entry(hit.target.entity).or_default() += 1;
        }

        pulses.push(CombatPulse {
            start: hit.attacker.position,
            end: hit.target.position,
            color: faction_color(hit.attacker.swarm),
        });
        let decoration_half_extent = direction.perp() * settings.decoration_length.max(0.0) * 0.5;
        decorations.push(
            CombatDecoration {
                start: hit.target.position - decoration_half_extent,
                end: hit.target.position + decoration_half_extent,
                color: faction_color(hit.attacker.swarm),
            },
            settings.max_decorative_effects,
        );
    }

    let mut poses = poses.into_iter().collect::<Vec<_>>();
    poses.sort_by_key(|(visual, _)| visual.to_bits());
    for (visual, mut pending) in poses {
        let reaction_count = pending.reaction_directions.len() as f32;
        commands.entity(visual).insert(CombatPose {
            attack_direction: aggregate_direction(&mut pending.attack_directions),
            reaction_direction: aggregate_direction(&mut pending.reaction_directions),
            reaction_flash: (settings.reaction_flash_per_hit * reaction_count)
                .min(settings.reaction_flash_cap)
                .clamp(0.0, 1.0),
            elapsed: Duration::ZERO,
            just_started: true,
        });
    }

    let mut structure_hit_counts = structure_hit_counts.into_iter().collect::<Vec<_>>();
    structure_hit_counts.sort_by_key(|(structure, _)| structure.to_bits());
    for (structure, hit_count) in structure_hit_counts {
        let Ok((sprite, active_flash)) = structures.get(structure) else {
            continue;
        };
        commands.entity(structure).insert(StructureCombatFlash {
            neutral_color: active_flash.map_or(sprite.color, |flash| flash.neutral_color),
            intensity: (settings.reaction_flash_per_hit * hit_count as f32)
                .min(settings.reaction_flash_cap)
                .clamp(0.0, 1.0),
            elapsed: Duration::ZERO,
            just_started: true,
        });
    }
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
    view: Res<CombatPresentationView>,
    entities: AnimatedCombatEntities,
    transients: CombatTransients,
) {
    let AnimatedCombatEntities {
        roots,
        mut visuals,
        mut structures,
    } = entities;
    let CombatTransients {
        mut pulses,
        mut decorations,
        mut ghosts,
        mut structure_ghosts,
    } = transients;
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

        if !view.visible {
            *visual.transform = Transform::IDENTITY;
            visual.sprite.color = Color::WHITE;
            continue;
        }

        let strength = 1.0 - progress;
        let Ok(root) = roots.get(visual.parent.parent()) else {
            continue;
        };
        let inverse_root_rotation = root.rotation.inverse();
        let jab_distance = impact_world_distance(
            settings.jab_distance,
            view.zoom,
            settings.minimum_impact_screen_distance,
        );
        let recoil_distance = impact_world_distance(
            settings.recoil_distance,
            view.zoom,
            settings.minimum_impact_screen_distance,
        );
        let attack_offset = visual
            .pose
            .attack_direction
            .map_or(Vec2::ZERO, |direction| {
                (inverse_root_rotation * direction.extend(0.0)).truncate() * jab_distance * strength
            });
        let reaction_offset = visual
            .pose
            .reaction_direction
            .map_or(Vec2::ZERO, |direction| {
                (inverse_root_rotation * direction.extend(0.0)).truncate()
                    * recoil_distance
                    * strength
            });
        visual.transform.translation = (attack_offset + reaction_offset).extend(0.0);
        visual.transform.rotation =
            visual
                .pose
                .attack_direction
                .map_or(Quat::IDENTITY, |direction| {
                    let world_facing = rotation_for_direction(direction).unwrap_or(Quat::IDENTITY);
                    inverse_root_rotation * world_facing
                });
        visual.sprite.color = Color::srgb(1.0, 1.0, 1.0 - visual.pose.reaction_flash * strength);
    }

    for mut structure in &mut structures {
        let flash = &mut *structure.flash;
        let progress = transient_progress(
            &mut flash.elapsed,
            &mut flash.just_started,
            delta,
            settings.recovery_duration,
        );
        if progress >= 1.0 {
            structure.sprite.color = flash.neutral_color;
            commands
                .entity(structure.entity)
                .remove::<StructureCombatFlash>();
            continue;
        }
        if !view.visible {
            structure.sprite.color = flash.neutral_color;
            continue;
        }
        let neutral = flash.neutral_color.to_srgba();
        let strength = flash.intensity * (1.0 - progress);
        structure.sprite.color = Color::srgba(
            neutral.red + (1.0 - neutral.red) * strength,
            neutral.green + (1.0 - neutral.green) * strength,
            neutral.blue + (1.0 - neutral.blue) * strength,
            neutral.alpha,
        );
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

    structure_ghosts.active.retain_mut(|ghost| {
        let progress = transient_progress(
            &mut ghost.elapsed,
            &mut ghost.just_started,
            delta,
            settings.death_duration,
        );
        if progress >= 1.0 {
            return false;
        }

        let neutral = completed_visual_color().to_srgba();
        let flash = (1.0 - progress / 0.25).clamp(0.0, 1.0);
        let alpha = (1.0 - (progress - 0.2).max(0.0) / 0.8).clamp(0.0, 1.0);
        ghost.visual.color = Color::srgba(
            neutral.red + (1.0 - neutral.red) * flash,
            neutral.green + (1.0 - neutral.green) * flash,
            neutral.blue + (1.0 - neutral.blue) * flash,
            alpha,
        );
        ghost.visual.ring_radius = settings
            .structure_ring_start_radius
            .max(0.0)
            .lerp(settings.structure_ring_end_radius.max(0.0), progress);
        let ring = faction_color(ghost.visual.victim.swarm).to_srgba();
        ghost.visual.ring_color = Color::srgba(ring.red, ring.green, ring.blue, 1.0 - progress);
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
    decorations.active.retain_mut(|decoration| {
        transient_progress(
            &mut decoration.elapsed,
            &mut decoration.just_started,
            delta,
            settings.pulse_duration,
        ) < 1.0
    });
}

fn extract_combat_death_ghosts(
    mut commands: Commands,
    nanobot_ghosts: Extract<Res<ActiveNanobotDeathGhosts>>,
    structure_ghosts: Extract<Res<ActiveStructureDeathGhosts>>,
    view: Extract<Res<CombatPresentationView>>,
    views: Query<&RenderVisibleEntities>,
    mut extracted_sprites: ResMut<ExtractedSprites>,
) {
    if !view.visible {
        return;
    }
    let visible_sprite_anchors = views
        .iter()
        .filter_map(|visible| visible.iter::<Sprite>().next().map(|(_, main)| **main))
        .collect::<HashSet<_>>();
    for main_entity in visible_sprite_anchors {
        let ghosts = nanobot_ghosts
            .iter()
            .map(|ghost| (&ghost.transform, ghost.color, ghost.image.id()))
            .chain(
                structure_ghosts
                    .iter()
                    .map(|ghost| (&ghost.transform, ghost.color, ghost.image.id())),
            );
        for (transform, color, image_handle_id) in ghosts {
            extracted_sprites.sprites.push(ExtractedSprite {
                main_entity,
                render_entity: commands.spawn(TemporaryRenderEntity).id(),
                transform: GlobalTransform::from(*transform),
                color: color.into(),
                image_handle_id,
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
    configs.config_mut::<CombatGizmos>().0.line.width = settings.pulse_thickness;
    configs.config_mut::<CombatDecorationGizmos>().0.line.width = settings.decoration_thickness;
    configs
        .config_mut::<StructureDeathRingGizmos>()
        .0
        .line
        .width = settings.structure_ring_thickness;
}

fn draw_combat_pulses(
    view: Res<CombatPresentationView>,
    pulses: Res<ActiveCombatPulses>,
    mut gizmos: Gizmos<CombatGizmos>,
) {
    if !view.visible {
        return;
    }
    for pulse in pulses.iter() {
        gizmos.line_2d(pulse.start, pulse.end, pulse.color);
    }
}

fn draw_combat_decorations(
    view: Res<CombatPresentationView>,
    decorations: Res<ActiveCombatDecorations>,
    mut gizmos: Gizmos<CombatDecorationGizmos>,
) {
    if !view.visible {
        return;
    }
    for decoration in decorations.iter() {
        gizmos.line_2d(decoration.start, decoration.end, decoration.color);
    }
}

fn draw_structure_death_rings(
    view: Res<CombatPresentationView>,
    ghosts: Res<ActiveStructureDeathGhosts>,
    mut gizmos: Gizmos<StructureDeathRingGizmos>,
) {
    if !view.visible {
        return;
    }
    for ghost in ghosts.iter() {
        gizmos.circle_2d(
            Isometry2d::from_translation(ghost.victim.position),
            ghost.ring_radius,
            ghost.ring_color,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aggregate_direction_handles_empty_single_and_cancelling_inputs() {
        assert_eq!(aggregate_direction(&mut []), None);
        assert_eq!(aggregate_direction(&mut [Vec2::X]), Some(Vec2::X));
        assert_eq!(aggregate_direction(&mut [Vec2::X, Vec2::NEG_X]), None);
    }

    #[test]
    fn aggregate_direction_is_independent_of_input_order() {
        let mut forward = [Vec2::new(0.8, 0.6), Vec2::new(0.8, -0.6), Vec2::Y];
        let mut reverse = [Vec2::Y, Vec2::new(0.8, -0.6), Vec2::new(0.8, 0.6)];

        assert_eq!(
            aggregate_direction(&mut forward),
            aggregate_direction(&mut reverse),
        );
    }

    #[test]
    fn impact_distance_keeps_its_configured_and_screen_space_minimums() {
        assert_eq!(impact_world_distance(10.0, 1.0, 2.0), 10.0);
        assert!((impact_world_distance(10.0, 7.99, 2.0) - 15.98).abs() < 0.001);
        assert_eq!(impact_world_distance(-1.0, -1.0, -1.0), 0.0);
    }
}
