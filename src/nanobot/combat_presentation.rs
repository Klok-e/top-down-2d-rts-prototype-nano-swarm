//! Render-time presentation of facts already resolved by Defender combat.

mod effects;
mod heat;
mod indicators;
mod render_anchor;
mod shake;
mod skirmish_end;

use std::{collections::HashMap, time::Duration};

use bevy::{
    camera::CameraProjection,
    ecs::{query::QueryData, system::SystemParam},
    math::I64Vec2,
    prelude::*,
};

use crate::{
    GAMEPLAY_SPRITE_Z, MainCamera,
    fly_camera::camera_2d_movement_system,
    structure_sprites::StructureVisual,
    tuning::{PresentationTuning, Tuning},
};

use super::{
    CombatAppearance, CombatVisualSnapshot, Nanobot, NanobotType, NanobotVisual, PresentedNanobot,
    ResolvedCombatFact, ResolvedCombatHit, SwarmMember, UpdateNanobotPresentation,
    motion_presentation::{CombatTickSnapshots, capture_fixed_motion},
    swarm_presentation_color,
};

pub use effects::{
    ActiveCombatEffects, CombatCasualtyFlash, CombatDeathPop, CombatEffectSpawns, CombatParticle,
    CombatParticleKind,
};
use effects::{CombatEffectRequest, CombatEffectsPlugin};
pub use heat::{BattleMarker, CombatHeat};
use indicators::CombatIndicatorsPlugin;
pub use indicators::{ActiveCombatIndicators, CombatIndicator, CombatIndicatorKind};
pub use skirmish_end::{ActiveSkirmishRings, SkirmishRing};

/// Destruction from the gameplay lifecycle outside resolved combat.
#[derive(Debug, Clone, Copy, Message)]
pub enum NonCombatDestruction {
    Nanobot {
        victim: CombatVisualSnapshot,
        occurred_at: Duration,
    },
    Structure {
        position: Vec2,
    },
}

#[derive(EntityEvent)]
pub(super) struct NanobotDied {
    pub entity: Entity,
}

#[derive(EntityEvent)]
pub(super) struct StructureCollapsed {
    pub entity: Entity,
}

const MAX_BOLTS_PER_FRAME: usize = 300;
const BOLT_SPEED: f32 = 1600.0;
const MIN_FLIGHT_SECS: f32 = 0.04;
const MAX_FLIGHT_SECS: f32 = 0.09;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CombatBolt {
    pub attacker: Entity,
    pub target: Entity,
    pub start: Vec2,
    pub position: Vec2,
    pub end: Vec2,
    pub direction: Vec2,
    pub color: Color,
    pub flight_time: Duration,
    pub elapsed: Duration,
    pub target_destroyed: bool,
}

impl CombatBolt {
    fn aim_at(&mut self, end: Vec2) {
        self.end = end;
        self.direction = (end - self.start).normalize_or(Vec2::X);
        self.flight_time = Duration::from_secs_f32(
            (self.start.distance(end) / BOLT_SPEED).clamp(MIN_FLIGHT_SECS, MAX_FLIGHT_SECS),
        );
    }

    fn move_towards(&mut self, end: Vec2) {
        self.end = end;
        let progress = (self.elapsed.as_secs_f32() / self.flight_time.as_secs_f32()).min(1.0);
        let position = self.start.lerp(end, progress);
        self.direction = (position - self.position).normalize_or(self.direction);
        self.position = position;
    }
}

/// The frozen render state of a victim whose death is awaiting bolt arrival.
#[derive(Debug)]
pub struct PendingCombatVictim {
    pub snapshot: CombatVisualSnapshot,
    pub transform: Transform,
    pub sprite: Option<Sprite>,
}

#[derive(Debug)]
struct PendingDeath {
    visual: PendingCombatVictim,
    active_bolts: usize,
    has_killing_bolt: bool,
    played: bool,
}

impl PendingDeath {
    fn play(&mut self, effects: &mut MessageWriter<CombatEffectRequest>) {
        if !self.played {
            effects.write(CombatEffectRequest::Death {
                victim: self.visual.snapshot,
            });
            self.played = true;
        }
    }
}

/// Presentation state lives outside the gameplay entity allocator.
#[derive(Debug, Default, Resource)]
pub struct ActiveCombatBolts {
    active: Vec<CombatBolt>,
    deaths: HashMap<Entity, PendingDeath>,
    spawned_this_frame: usize,
}

impl ActiveCombatBolts {
    pub fn len(&self) -> usize {
        self.active.len()
    }

    pub fn is_empty(&self) -> bool {
        self.active.is_empty()
    }

    pub fn iter(&self) -> impl ExactSizeIterator<Item = &CombatBolt> {
        self.active.iter()
    }

    pub fn pending_victims(&self) -> impl Iterator<Item = &PendingCombatVictim> {
        self.deaths
            .values()
            .filter(|death| !death.played && death.active_bolts > 0)
            .map(|death| &death.visual)
    }

    pub fn spawned_this_frame(&self) -> usize {
        self.spawned_this_frame
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CombatZoomTier {
    Close,
    Mid,
    Far,
}

impl CombatZoomTier {
    fn at_zoom(zoom: f32, settings: &PresentationTuning) -> Self {
        if zoom >= settings.effects_far_zoom {
            Self::Far
        } else if zoom >= settings.effects_mid_zoom {
            Self::Mid
        } else {
            Self::Close
        }
    }
}

#[derive(Debug, Resource)]
struct CombatView {
    bounds: Option<Rect>,
    tier: CombatZoomTier,
    zoom: f32,
    world_per_pixel: f32,
    minimum_effect_size: f32,
}

impl Default for CombatView {
    fn default() -> Self {
        Self {
            bounds: None,
            tier: CombatZoomTier::Close,
            zoom: 1.0,
            world_per_pixel: 1.0,
            minimum_effect_size: 0.0,
        }
    }
}

impl CombatView {
    fn effect_size(&self, size: Vec2) -> Vec2 {
        size.max(Vec2::splat(self.minimum_effect_size))
    }

    fn line_width(&self, width: f32) -> f32 {
        width.clamp(1.5 * self.world_per_pixel, 3.0 * self.world_per_pixel)
    }

    fn center(&self) -> Vec2 {
        self.bounds.map_or(Vec2::ZERO, |bounds| bounds.center())
    }

    fn contains(&self, point: Vec2) -> bool {
        self.contains_with_margin(point, 0.0)
    }

    fn contains_with_margin(&self, point: Vec2, margin: f32) -> bool {
        self.bounds
            .is_none_or(|bounds| bounds.inflate(margin).contains(point))
    }

    fn hit_distance(&self, start: Vec2, end: Vec2) -> Option<f32> {
        [start, end]
            .into_iter()
            .filter(|position| self.contains(*position))
            .map(|position| position.distance_squared(self.center()))
            .reduce(f32::min)
    }
}

pub(super) fn combat_hdr(color: Color, intensity: f32) -> Color {
    let color = color.to_linear();
    let brightness = 4.0 * intensity;
    Color::linear_rgba(
        color.red * brightness,
        color.green * brightness,
        color.blue * brightness,
        color.alpha,
    )
}

#[derive(Debug, Component)]
struct CombatPose {
    attack_direction: Option<Vec2>,
    reaction_direction: Option<Vec2>,
    reaction_hits: f32,
    blend_from: Option<Vec2>,
    offset: Vec2,
    elapsed: Duration,
}

impl CombatPose {
    fn progress(&self, settings: &PresentationTuning) -> f32 {
        (self.elapsed.as_secs_f32() / settings.combat_recovery_duration_secs).min(1.0)
    }

    fn eased_offset(&self, strength: f32, settings: &PresentationTuning) -> Vec2 {
        let jab = strength.powi(3);
        // One minus easeOutBack, with a 10% overshoot before returning to zero.
        let recoil = strength.powi(2) * (2.70158 * strength - 1.70158);
        let offset =
            self.attack_direction.unwrap_or(Vec2::ZERO) * settings.combat_jab_distance * jab
                + self.reaction_direction.unwrap_or(Vec2::ZERO)
                    * settings.combat_recoil_distance
                    * recoil;
        self.blend_from
            .map_or(offset, |previous| offset.lerp(previous * jab, jab))
    }
}

#[derive(Debug, Default)]
struct PendingCombatPose {
    attack_directions: ImpactDirections,
    reaction_directions: ImpactDirections,
}

#[derive(Debug, Default)]
struct ImpactDirections {
    sum: I64Vec2,
    fallback: Option<Vec2>,
    count: usize,
}

impl ImpactDirections {
    fn push(&mut self, direction: Vec2) {
        // Fixed-point addition keeps simultaneous hits independent of fact order.
        const DIRECTION_SCALE: f32 = (1 << 24) as f32;
        self.sum += (direction * DIRECTION_SCALE).round().as_i64vec2();
        self.count += 1;

        if self.fallback.is_none_or(|min| {
            direction
                .x
                .total_cmp(&min.x)
                .then_with(|| direction.y.total_cmp(&min.y))
                .is_lt()
        }) {
            self.fallback = Some(direction);
        }
    }

    fn direction(&self) -> Option<Vec2> {
        self.sum.as_vec2().try_normalize().or_else(|| {
            self.fallback
                .map(|direction| direction.normalize_or(Vec2::X))
        })
    }
}

#[derive(Debug, Component)]
struct StructureCombatFlash {
    neutral_color: Color,
    hit_count: f32,
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
    pose: Option<&'static mut CombatPose>,
}

#[derive(QueryData)]
#[query_data(mutable)]
struct AnimatedStructureVisual {
    entity: Entity,
    sprite: &'static mut Sprite,
    flash: &'static mut StructureCombatFlash,
}

#[derive(SystemParam)]
struct CombatParticipants<'w, 's> {
    roots: Query<'w, 's, (Option<&'static Children>, &'static mut PresentedNanobot), With<Nanobot>>,
    visuals: Query<'w, 's, Option<&'static CombatPose>, With<NanobotVisual>>,
    structures: Query<
        'w,
        's,
        (
            &'static Transform,
            &'static Sprite,
            Option<&'static StructureCombatFlash>,
        ),
        With<StructureVisual>,
    >,
}

impl CombatParticipants<'_, '_> {
    fn position(&self, entity: Entity) -> Option<Vec2> {
        self.roots
            .get(entity)
            .ok()
            .map(|(_, presented)| presented.sprite_translation().truncate())
            .or_else(|| {
                self.structures
                    .get(entity)
                    .ok()
                    .map(|(transform, ..)| transform.translation.truncate())
            })
    }

    fn visual(&self, entity: Entity) -> Option<Entity> {
        let (children, _) = self.roots.get(entity).ok()?;
        children?.iter().find(|child| self.visuals.contains(*child))
    }
}

#[derive(Debug)]
struct ShotCandidate {
    hit: ResolvedCombatHit,
    start: Vec2,
    end: Vec2,
    view_distance: f32,
    order: usize,
}

pub(crate) struct CombatPresentationPlugin;

pub fn reconcile_combat_presentation_tuning(world: &mut World, old: &Tuning, new: &Tuning) {
    let previous = old.presentation.combat_recovery_duration_secs;
    let next = new.presentation.combat_recovery_duration_secs;
    if previous == next {
        return;
    }
    let rescale = |elapsed: &mut Duration| {
        *elapsed = Duration::from_secs_f32(elapsed.as_secs_f32() / previous * next);
    };
    for mut pose in world.query::<&mut CombatPose>().iter_mut(world) {
        rescale(&mut pose.elapsed);
    }
    for mut flash in world.query::<&mut StructureCombatFlash>().iter_mut(world) {
        rescale(&mut flash.elapsed);
    }
}

pub(super) fn reset_session(world: &mut World) {
    super::reset_resource::<ActiveCombatBolts>(world);
    super::reset_resource::<CombatView>(world);
    super::reset_resource::<CombatTickSnapshots>(world);
    if let Some(mut facts) = world.get_resource_mut::<Messages<ResolvedCombatFact>>() {
        facts.clear();
    }
    if let Some(mut destruction) = world.get_resource_mut::<Messages<NonCombatDestruction>>() {
        destruction.clear();
    }
    effects::reset_session(world);
    heat::reset_session(world);
    indicators::reset_session(world);
    shake::reset_session(world);
    skirmish_end::reset_session(world);
}

impl Plugin for CombatPresentationPlugin {
    fn build(&self, app: &mut App) {
        if !app
            .world()
            .contains_resource::<Messages<ResolvedCombatFact>>()
        {
            app.add_message::<ResolvedCombatFact>();
        }
        app.init_resource::<Tuning>()
            .add_message::<NonCombatDestruction>()
            .init_resource::<ActiveCombatBolts>()
            .init_resource::<CombatView>()
            .init_resource::<CombatTickSnapshots>()
            .add_plugins((
                CombatEffectsPlugin,
                CombatIndicatorsPlugin,
                heat::CombatHeatPlugin,
                shake::CombatShakePlugin,
                skirmish_end::SkirmishEndPlugin,
            ))
            .add_observer(capture_despawning_structure)
            .add_observer(report_noncombat_nanobot_death)
            .add_observer(report_structure_collapse)
            // Combat deaths still have their gameplay transforms before FixedLast settlement.
            .add_systems(FixedPostUpdate, capture_fixed_motion)
            .add_systems(
                Update,
                (
                    update_combat_view.after(camera_2d_movement_system),
                    update_nanobot_positions,
                    resolve_combat,
                    animate_nanobot_visuals,
                    align_flying_bolts,
                    animate_structure_flashes,
                )
                    .chain()
                    .in_set(UpdateNanobotPresentation),
            );
    }
}

fn capture_despawning_structure(
    despawned: On<Despawn, StructureVisual>,
    structures: Query<(&Transform, &Sprite), With<StructureVisual>>,
    mut snapshots: ResMut<CombatTickSnapshots>,
) {
    if let Ok((transform, sprite)) = structures.get(despawned.entity) {
        snapshots.record_structure_death(despawned.entity, *transform, sprite.clone());
    }
}

fn report_noncombat_nanobot_death(
    death: On<NanobotDied>,
    bots: Query<(
        &Transform,
        &NanobotType,
        &SwarmMember,
        Option<&PresentedNanobot>,
    )>,
    snapshots: Res<CombatTickSnapshots>,
    fixed: Option<Res<Time<Fixed>>>,
    mut destruction: MessageWriter<NonCombatDestruction>,
) {
    if snapshots.death_time(death.entity).is_some() {
        return;
    }
    let Ok((transform, kind, member, presented)) = bots.get(death.entity) else {
        return;
    };
    let victim = CombatVisualSnapshot {
        entity: death.entity,
        position: presented.map_or(transform.translation.truncate(), |pose| {
            pose.fixed_death_position(fixed.as_deref())
        }),
        swarm: member.0,
        appearance: CombatAppearance::Nanobot(*kind),
    };
    destruction.write(NonCombatDestruction::Nanobot {
        victim,
        occurred_at: fixed.as_ref().map_or(Duration::ZERO, |time| time.elapsed()),
    });
}

fn report_structure_collapse(
    collapse: On<StructureCollapsed>,
    structures: Query<&Transform>,
    mut destruction: MessageWriter<NonCombatDestruction>,
) {
    if let Ok(transform) = structures.get(collapse.entity) {
        destruction.write(NonCombatDestruction::Structure {
            position: transform.translation.truncate(),
        });
    }
}

fn update_combat_view(
    camera: Query<(&Transform, &Projection, Option<&Camera>), With<MainCamera>>,
    tuning: Res<Tuning>,
    mut view: ResMut<CombatView>,
) {
    let settings = &tuning.presentation;
    let mut next = CombatView::default();
    if let Ok((transform, Projection::Orthographic(ortho), camera)) = camera.single() {
        let mut ortho = ortho.clone();
        next.zoom = ortho.scale.max(f32::EPSILON);
        next.world_per_pixel = next.zoom;
        if let Some(size) = camera.and_then(Camera::logical_viewport_size)
            && size.x > 0.0
            && size.y > 0.0
        {
            ortho.update(size.x, size.y);
            if let Some(pixels) = camera.and_then(Camera::physical_viewport_size)
                && pixels.x > 0
            {
                next.world_per_pixel = ortho.area.width() / pixels.x as f32;
            }
        }
        let translation = transform.translation.truncate();
        next.bounds = Some(Rect::from_corners(
            ortho.area.min + translation,
            ortho.area.max + translation,
        ));
    }
    next.tier = CombatZoomTier::at_zoom(next.zoom, settings);
    next.minimum_effect_size = settings.effect_min_screen_px * next.world_per_pixel;
    *view = next;
}

fn update_nanobot_positions(
    fixed: Option<Res<Time<Fixed>>>,
    time: Option<Res<Time>>,
    tuning: Res<Tuning>,
    view: Res<CombatView>,
    mut roots: Query<(&Transform, &mut PresentedNanobot), With<Nanobot>>,
    mut poses: Query<(&ChildOf, &mut CombatPose)>,
) {
    let overstep = fixed.as_ref().map_or(1.0, |time| time.overstep_fraction());
    for (transform, mut presented) in &mut roots {
        presented.update_translation(transform, overstep);
    }
    let settings = &tuning.presentation;
    let show_poses = view.tier == CombatZoomTier::Close;
    let delta = time.as_ref().map_or(Duration::ZERO, |time| time.delta());
    for (parent, mut pose) in &mut poses {
        // Resolve this frame's impacts at the drawn position before applying the new recoil.
        pose.elapsed += delta;
        pose.offset = if show_poses {
            pose.eased_offset(1.0 - pose.progress(settings), settings)
        } else {
            Vec2::ZERO
        };
        if let Ok((_, mut presented)) = roots.get_mut(parent.parent()) {
            presented.impact_offset = pose.offset;
        }
    }
}

fn effect_position(
    snapshot: CombatVisualSnapshot,
    deaths: &HashMap<Entity, PendingDeath>,
    participants: &CombatParticipants,
) -> Vec2 {
    deaths.get(&snapshot.entity).map_or_else(
        || {
            participants
                .position(snapshot.entity)
                .unwrap_or(snapshot.position)
        },
        |death| death.visual.snapshot.position,
    )
}

#[allow(clippy::too_many_arguments)]
fn resolve_combat(
    mut commands: Commands,
    mut facts: MessageReader<ResolvedCombatFact>,
    mut effects: MessageWriter<CombatEffectRequest>,
    mut participants: CombatParticipants,
    mut tick_snapshots: ResMut<CombatTickSnapshots>,
    mut bolts: ResMut<ActiveCombatBolts>,
    view: Res<CombatView>,
    time: Option<Res<Time>>,
    fixed: Option<Res<Time<Fixed>>>,
) {
    let (now, delta) = time
        .as_ref()
        .map_or((Duration::ZERO, Duration::ZERO), |time| {
            (time.elapsed(), time.delta())
        });
    let mut hits = Vec::new();
    let mut dead_victims = Vec::new();
    for (fact, id) in facts.read_with_id() {
        match *fact {
            ResolvedCombatFact::Hit(hit) => {
                hits.push((hit, tick_snapshots.hit_time(id).unwrap_or(now)))
            }
            ResolvedCombatFact::Death(death) => dead_victims.push(death.victim),
        }
    }
    // A destroyed target can appear in several hits from the same simulation tick.
    dead_victims.extend(
        hits.iter()
            .filter_map(|(hit, _)| hit.target_destroyed.then_some(hit.target)),
    );
    for mut victim in dead_victims {
        bolts.deaths.entry(victim.entity).or_insert_with(|| {
            let (transform, sprite) = tick_snapshots
                .take_death_visual(victim.entity, fixed.as_deref())
                .unwrap_or_else(|| {
                    let position = participants
                        .position(victim.entity)
                        .unwrap_or(victim.position);
                    (
                        Transform::from_translation(position.extend(GAMEPLAY_SPRITE_Z)),
                        None,
                    )
                });
            victim.position = transform.translation.truncate();
            if matches!(victim.appearance, CombatAppearance::Nanobot(_)) {
                effects.write(CombatEffectRequest::Casualty { victim });
            }
            PendingDeath {
                visual: PendingCombatVictim {
                    snapshot: victim,
                    transform,
                    sprite,
                },
                active_bolts: 0,
                has_killing_bolt: false,
                played: false,
            }
        });
    }
    tick_snapshots.clear();

    let mut shots = hits
        .into_iter()
        .enumerate()
        .filter_map(|(order, (hit, hit_at))| {
            let start = effect_position(hit.attacker, &bolts.deaths, &participants);
            let end = effect_position(hit.target, &bolts.deaths, &participants);
            if hit.attacker.appearance == CombatAppearance::Nanobot(NanobotType::Defender)
                && let Ok((_, mut presented)) = participants.roots.get_mut(hit.attacker.entity)
            {
                presented.aim_at((end - start).normalize_or(Vec2::X), hit_at);
            }
            if view.tier == CombatZoomTier::Far {
                return None;
            }
            let view_distance = view.hit_distance(start, end)?;
            Some(ShotCandidate {
                hit,
                start,
                end,
                view_distance,
                order,
            })
        })
        .collect::<Vec<_>>();
    if shots.len() > MAX_BOLTS_PER_FRAME {
        shots.select_nth_unstable_by(MAX_BOLTS_PER_FRAME, |left, right| {
            right
                .hit
                .target_destroyed
                .cmp(&left.hit.target_destroyed)
                .then_with(|| left.view_distance.total_cmp(&right.view_distance))
                .then_with(|| left.order.cmp(&right.order))
        });
        shots.truncate(MAX_BOLTS_PER_FRAME);
        shots.sort_unstable_by_key(|shot| shot.order);
    }

    let mut poses = HashMap::<Entity, PendingCombatPose>::new();
    let mut structure_hits = HashMap::<Entity, usize>::new();
    let ActiveCombatBolts {
        active,
        deaths,
        spawned_this_frame,
    } = &mut *bolts;
    if view.tier == CombatZoomTier::Far {
        active.clear();
    }
    let flying_before_launch = active.len();
    *spawned_this_frame = shots.len();
    for shot in shots {
        let direction = (shot.end - shot.start).normalize_or(Vec2::X);
        let attacker = shot.hit.attacker.entity;
        let target = shot.hit.target.entity;
        if view.tier == CombatZoomTier::Close
            && let Some(visual) = participants.visual(attacker)
        {
            poses
                .entry(visual)
                .or_default()
                .attack_directions
                .push(direction);
        }

        let color = swarm_presentation_color(shot.hit.attacker.swarm);
        active.push(CombatBolt {
            attacker,
            target,
            start: shot.start,
            position: shot.start,
            end: shot.end,
            direction,
            color,
            elapsed: Duration::ZERO,
            flight_time: Duration::ZERO,
            target_destroyed: shot.hit.target_destroyed,
        });
    }
    for death in deaths.values_mut() {
        death.active_bolts = 0;
        death.has_killing_bolt = false;
    }
    for bolt in active.iter() {
        if let Some(death) = deaths.get_mut(&bolt.target) {
            death.active_bolts += 1;
            death.has_killing_bolt |= bolt.target_destroyed;
        }
    }

    let mut existing_bolts = flying_before_launch;
    active.retain_mut(|bolt| {
        if existing_bolts == 0 {
            return true;
        }
        existing_bolts -= 1;
        bolt.elapsed += delta;
        if bolt.elapsed < bolt.flight_time {
            return true;
        }
        let death = deaths.get_mut(&bolt.target);
        let impact = match &death {
            Some(death) => death.visual.snapshot.position,
            None => participants.position(bolt.target).unwrap_or(bolt.end),
        };
        bolt.move_towards(impact);
        effects.write(CombatEffectRequest::Impact {
            position: bolt.end,
            incoming: bolt.direction,
            color: bolt.color,
        });
        if let Some(death) = death {
            death.active_bolts -= 1;
            if bolt.target_destroyed || !death.has_killing_bolt {
                death.play(&mut effects);
            }
        } else if view.tier == CombatZoomTier::Close {
            if let Some(visual) = participants.visual(bolt.target) {
                poses
                    .entry(visual)
                    .or_default()
                    .reaction_directions
                    .push(bolt.direction);
            } else if participants.structures.contains(bolt.target) {
                *structure_hits.entry(bolt.target).or_default() += 1;
            }
        }
        false
    });

    deaths.retain(|_, death| {
        if death.active_bolts > 0 {
            return true;
        }
        if view.tier != CombatZoomTier::Far && view.contains(death.visual.snapshot.position) {
            death.play(&mut effects);
        }
        false
    });

    for (visual, pending) in poses {
        commands.entity(visual).insert(CombatPose {
            attack_direction: pending.attack_directions.direction(),
            reaction_direction: pending.reaction_directions.direction(),
            reaction_hits: pending.reaction_directions.count as f32,
            blend_from: participants
                .visuals
                .get(visual)
                .ok()
                .flatten()
                .filter(|pose| pose.offset != Vec2::ZERO)
                .map(|pose| pose.offset),
            offset: Vec2::ZERO,
            elapsed: Duration::ZERO,
        });
    }
    for (structure, hit_count) in structure_hits {
        let Ok((_, sprite, active_flash)) = participants.structures.get(structure) else {
            continue;
        };
        commands.entity(structure).insert(StructureCombatFlash {
            neutral_color: active_flash.map_or(sprite.color, |flash| flash.neutral_color),
            hit_count: hit_count as f32,
            elapsed: Duration::ZERO,
            just_started: true,
        });
    }
}

fn align_flying_bolts(
    mut bolts: ResMut<ActiveCombatBolts>,
    participants: CombatParticipants,
    mut effects: MessageWriter<CombatEffectRequest>,
    view: Res<CombatView>,
) {
    let ActiveCombatBolts {
        active,
        deaths,
        spawned_this_frame,
    } = &mut *bolts;
    let first_new = active.len() - *spawned_this_frame;
    for (index, bolt) in active.iter_mut().enumerate() {
        let end = deaths.get(&bolt.target).map_or_else(
            || participants.position(bolt.target).unwrap_or(bolt.end),
            |death| death.visual.snapshot.position,
        );
        if index >= first_new {
            if let Some(start) = participants.position(bolt.attacker) {
                bolt.start = start;
                bolt.position = start;
            }
            bolt.aim_at(end);
            if view.tier == CombatZoomTier::Close {
                effects.write(CombatEffectRequest::Muzzle {
                    attacker: bolt.attacker,
                    position: bolt.start,
                    color: bolt.color,
                });
            }
        } else {
            bolt.move_towards(end);
        }
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

#[allow(clippy::type_complexity)]
fn animate_nanobot_visuals(
    mut commands: Commands,
    time: Option<Res<Time>>,
    fixed: Option<Res<Time<Fixed>>>,
    tuning: Res<Tuning>,
    view: Res<CombatView>,
    mut roots: Query<(&Transform, &NanobotType, &mut PresentedNanobot), With<Nanobot>>,
    mut visuals: Query<AnimatedCombatVisual, (With<NanobotVisual>, Without<Nanobot>)>,
) {
    let settings = &tuning.presentation;
    let (now, delta) = time
        .as_ref()
        .map_or((Duration::ZERO, Duration::ZERO), |time| {
            (time.elapsed(), time.delta())
        });
    let overstep = fixed.as_ref().map_or(1.0, |time| time.overstep_fraction());
    let show_poses = view.tier == CombatZoomTier::Close;
    let frame = (now, delta.as_secs_f32());

    for mut visual in &mut visuals {
        let Ok((root, kind, mut presented)) = roots.get_mut(visual.parent.parent()) else {
            continue;
        };
        presented.update_rotation(*kind, overstep, frame, settings);
        let mut offset = Vec2::ZERO;
        if let Some(mut pose) = visual.pose {
            let pose = &mut *pose;
            let progress = pose.progress(settings);
            if progress >= 1.0 {
                visual.sprite.color = Color::WHITE;
                commands.entity(visual.entity).remove::<CombatPose>();
            } else if show_poses {
                let strength = 1.0 - progress;
                offset = pose.eased_offset(strength, settings);
                let flash_gain = (settings.combat_reaction_flash_per_hit * pose.reaction_hits)
                    .min(settings.combat_reaction_flash_cap)
                    .clamp(0.0, 1.0)
                    * strength;
                visual.sprite.color =
                    Color::linear_rgb(1.0 + flash_gain, 1.0 + flash_gain, 1.0 + flash_gain);
            } else {
                visual.sprite.color = Color::WHITE;
            }
            pose.offset = offset;
        }

        let inverse_rotation = root.rotation.inverse();
        presented.impact_offset = offset;
        let position_delta = presented.sprite_translation() - root.translation;
        let local_translation = inverse_rotation * position_delta / root.scale;
        let local_rotation = if presented.rotation() == root.rotation {
            Quat::IDENTITY
        } else {
            inverse_rotation * presented.rotation()
        };
        if visual.transform.translation != local_translation {
            visual.transform.translation = local_translation;
        }
        if visual.transform.rotation != local_rotation {
            visual.transform.rotation = local_rotation;
        }
    }
}

fn animate_structure_flashes(
    mut commands: Commands,
    time: Option<Res<Time>>,
    tuning: Res<Tuning>,
    view: Res<CombatView>,
    mut structures: Query<AnimatedStructureVisual, With<StructureVisual>>,
) {
    let settings = &tuning.presentation;
    let show_poses = view.tier == CombatZoomTier::Close;
    let delta = time.as_ref().map_or(Duration::ZERO, |time| time.delta());
    for mut structure in &mut structures {
        let flash = &mut *structure.flash;
        let progress = transient_progress(
            &mut flash.elapsed,
            &mut flash.just_started,
            delta,
            Duration::from_secs_f32(settings.combat_recovery_duration_secs),
        );
        if progress >= 1.0 {
            structure.sprite.color = flash.neutral_color;
            commands
                .entity(structure.entity)
                .remove::<StructureCombatFlash>();
            continue;
        }
        if !show_poses {
            structure.sprite.color = flash.neutral_color;
            continue;
        }
        let neutral = flash.neutral_color.to_srgba();
        let strength = (settings.combat_reaction_flash_per_hit * flash.hit_count)
            .min(settings.combat_reaction_flash_cap)
            .clamp(0.0, 1.0)
            * (1.0 - progress);
        structure.sprite.color = Color::srgba(
            neutral.red + (1.0 - neutral.red) * strength,
            neutral.green + (1.0 - neutral.green) * strength,
            neutral.blue + (1.0 - neutral.blue) * strength,
            neutral.alpha,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn aggregate_direction(directions: &[Vec2]) -> Option<Vec2> {
        let mut aggregate = ImpactDirections::default();
        for &direction in directions {
            aggregate.push(direction);
        }
        aggregate.direction()
    }

    #[test]
    fn aggregate_direction_handles_empty_single_and_cancelling_inputs() {
        assert_eq!(aggregate_direction(&[]), None);
        assert!(
            aggregate_direction(&[Vec2::X])
                .unwrap()
                .abs_diff_eq(Vec2::X, 1e-5)
        );
        assert!(
            aggregate_direction(&[Vec2::X, Vec2::NEG_X])
                .unwrap()
                .abs_diff_eq(Vec2::NEG_X, 1e-5)
        );
    }

    #[test]
    fn aggregate_direction_is_independent_of_input_order() {
        let forward = [Vec2::new(0.8, 0.6), Vec2::new(0.8, -0.6), Vec2::Y];
        let reverse = [Vec2::Y, Vec2::new(0.8, -0.6), Vec2::new(0.8, 0.6)];

        assert_eq!(aggregate_direction(&forward), aggregate_direction(&reverse),);
    }
}
