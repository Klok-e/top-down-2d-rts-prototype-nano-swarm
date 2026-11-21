//! Short-lived combat effects and bounded battlefield decals.

mod render;

use std::{
    collections::{BTreeMap, HashMap},
    time::Duration,
};

use bevy::prelude::*;
use rand::{RngExt, SeedableRng, rngs::SmallRng};

use crate::{
    nanobot::{
        CombatAppearance, CombatVisualSnapshot, NonCombatDestruction, OwnerSwarm, Structure, Swarm,
        SwarmId, swarm_presentation_color,
    },
    tuning::Tuning,
};

use super::{CombatView, CombatZoomTier};

const MAX_PARTICLES_PER_FRAME: usize = 600;
const MAX_POPS_PER_FRAME: usize = 40;
const MAX_DECALS: usize = 2_000;
const DECAL_CELL_SIZE: f32 = 64.0;
const MUZZLE_LIFETIME: f32 = 0.05;
const POP_LIFETIME: f32 = 0.25;
const STRUCTURE_FLASH_LIFETIME: f32 = 0.15;
const CASUALTY_LIFETIME: Duration = Duration::from_millis(400);

#[derive(Debug, Clone, Copy, Message)]
pub(super) enum CombatEffectRequest {
    Muzzle {
        attacker: Entity,
        position: Vec2,
        color: Color,
    },
    Impact {
        position: Vec2,
        incoming: Vec2,
        color: Color,
    },
    Death {
        victim: CombatVisualSnapshot,
    },
    Casualty {
        victim: CombatVisualSnapshot,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CombatParticleKind {
    ImpactSpark,
    ImpactFlash,
    Fragment,
    StructureSpark,
    Smoke,
}

#[derive(Debug, Clone, Copy)]
pub struct CombatParticle {
    pub position: Vec2,
    pub color: Color,
    pub kind: CombatParticleKind,
    velocity: Vec2,
    elapsed: f32,
    lifetime: f32,
    angle: f32,
    spin: f32,
    size: f32,
    close_only: bool,
}

impl CombatParticle {
    pub fn elapsed_secs(&self) -> f32 {
        self.elapsed
    }

    pub fn lifetime_secs(&self) -> f32 {
        self.lifetime
    }

    fn advance(&mut self, delta: f32) -> bool {
        self.elapsed += delta;
        if self.elapsed >= self.lifetime {
            return false;
        }

        let drag = match self.kind {
            CombatParticleKind::ImpactSpark
            | CombatParticleKind::ImpactFlash
            | CombatParticleKind::StructureSpark => 8.0,
            CombatParticleKind::Fragment => 4.2,
            CombatParticleKind::Smoke => 0.85,
        };
        let remaining_velocity = (-drag * delta).exp();
        self.position += self.velocity * ((1.0 - remaining_velocity) / drag);
        self.velocity *= remaining_velocity;
        self.angle += self.spin * delta;
        true
    }
}

#[derive(Debug, Clone, Copy)]
pub struct CombatCasualtyFlash {
    pub position: Vec2,
    pub color: Color,
    elapsed: Duration,
}

impl CombatCasualtyFlash {
    fn new(victim: CombatVisualSnapshot) -> Self {
        Self {
            position: victim.position,
            color: swarm_presentation_color(victim.swarm),
            elapsed: Duration::ZERO,
        }
    }

    pub fn diameter_px(&self) -> f32 {
        const POP_PEAK: f32 = 0.08;
        let elapsed = self.elapsed.as_secs_f32();
        let lifetime = CASUALTY_LIFETIME.as_secs_f32();
        let strength = if elapsed < POP_PEAK {
            elapsed / POP_PEAK
        } else {
            (lifetime - elapsed) / (lifetime - POP_PEAK)
        };
        5.0 + 3.0 * strength
    }

    pub fn opacity(&self) -> f32 {
        1.0 - self.elapsed.as_secs_f32() / CASUALTY_LIFETIME.as_secs_f32()
    }
}

#[derive(Debug, Clone, Copy)]
pub struct CombatDeathPop {
    pub victim: CombatVisualSnapshot,
    pub color: Color,
    elapsed: f32,
    ring_progress: f32,
}

impl CombatDeathPop {
    pub fn elapsed_secs(&self) -> f32 {
        self.elapsed
    }

    fn is_structure(&self) -> bool {
        matches!(self.victim.appearance, CombatAppearance::Structure(_))
    }
}

#[derive(Debug, Clone, Copy)]
struct MuzzleFlash {
    attacker: Entity,
    position: Vec2,
    color: Color,
    elapsed: f32,
}

#[derive(Debug, Clone, Copy)]
struct CombatDecal {
    position: Vec2,
    structure: bool,
    born: f64,
}

#[derive(Debug)]
struct StructureEmission {
    next: f64,
    particle_credit: f32,
    next_is_smoke: bool,
}

impl CombatDecal {
    fn cell(&self) -> IVec2 {
        (self.position / DECAL_CELL_SIZE).floor().as_ivec2()
    }

    fn lifetime(&self) -> f64 {
        if self.structure { 45.0 } else { 30.0 }
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct CombatEffectSpawns {
    pub particles: usize,
    pub death_pops: usize,
}

/// Visual state owned by presentation. Effects never allocate gameplay entities.
#[derive(Debug, Resource)]
pub struct ActiveCombatEffects {
    particles: Vec<CombatParticle>,
    death_pops: Vec<CombatDeathPop>,
    muzzles: Vec<MuzzleFlash>,
    casualties: Vec<CombatCasualtyFlash>,
    decals: BTreeMap<u64, CombatDecal>,
    decals_by_cell: HashMap<IVec2, u64>,
    structure_emissions: HashMap<Entity, StructureEmission>,
    next_decal: u64,
    clock: f64,
    rng: SmallRng,
    frame_spawns: CombatEffectSpawns,
}

impl Default for ActiveCombatEffects {
    fn default() -> Self {
        Self {
            particles: Vec::new(),
            death_pops: Vec::new(),
            muzzles: Vec::new(),
            casualties: Vec::new(),
            decals: BTreeMap::new(),
            decals_by_cell: HashMap::new(),
            structure_emissions: HashMap::new(),
            next_decal: 0,
            clock: 0.0,
            rng: SmallRng::seed_from_u64(0),
            frame_spawns: CombatEffectSpawns::default(),
        }
    }
}

impl ActiveCombatEffects {
    pub fn particles(&self) -> impl ExactSizeIterator<Item = &CombatParticle> {
        self.particles.iter()
    }

    pub fn death_pops(&self) -> impl ExactSizeIterator<Item = &CombatDeathPop> {
        self.death_pops.iter()
    }

    pub fn decal_count(&self) -> usize {
        self.decals.len()
    }

    pub fn muzzle_count(&self) -> usize {
        self.muzzles.len()
    }

    pub fn casualty_flashes(&self) -> &[CombatCasualtyFlash] {
        &self.casualties
    }

    pub fn frame_spawns(&self) -> CombatEffectSpawns {
        self.frame_spawns
    }

    fn refresh_decal(&mut self, decal: CombatDecal) {
        let order = self.next_decal;
        self.next_decal += 1;
        if let Some(previous) = self.decals_by_cell.insert(decal.cell(), order) {
            self.decals.remove(&previous);
        }
        self.decals.insert(order, decal);
        if self.decals.len() > MAX_DECALS
            && let Some((_, oldest)) = self.decals.pop_first()
        {
            self.decals_by_cell.remove(&oldest.cell());
        }
    }

    fn advance(&mut self, frame_time: Duration, structure_ring_lifetime: f32) {
        let delta = frame_time.as_secs_f32();
        self.clock += f64::from(delta);
        self.frame_spawns = CombatEffectSpawns::default();
        self.particles
            .retain_mut(|particle| particle.advance(delta));
        self.muzzles.retain_mut(|muzzle| {
            muzzle.elapsed += delta;
            muzzle.elapsed < MUZZLE_LIFETIME
        });
        self.casualties.retain_mut(|casualty| {
            casualty.elapsed += frame_time;
            casualty.elapsed < CASUALTY_LIFETIME
        });
        self.death_pops.retain_mut(|pop| {
            pop.elapsed += delta;
            if pop.is_structure() {
                pop.ring_progress += delta / structure_ring_lifetime;
                pop.elapsed < STRUCTURE_FLASH_LIFETIME || pop.ring_progress < 1.0
            } else {
                pop.elapsed < POP_LIFETIME
            }
        });
        let now = self.clock;
        self.decals.retain(|_, decal| {
            let alive = now - decal.born < decal.lifetime();
            if !alive {
                self.decals_by_cell.remove(&decal.cell());
            }
            alive
        });
    }

    fn spawn_death_pop(&mut self, victim: CombatVisualSnapshot, color: Color) {
        if self.frame_spawns.death_pops < MAX_POPS_PER_FRAME {
            self.frame_spawns.death_pops += 1;
            self.death_pops.push(CombatDeathPop {
                victim,
                color,
                elapsed: 0.0,
                ring_progress: 0.0,
            });
        }
    }

    fn spawn_particles(&mut self, emission: Emission, intensity: f32, tier: CombatZoomTier) {
        let remaining = MAX_PARTICLES_PER_FRAME - self.frame_spawns.particles;
        let count = |nominal| particle_count(nominal, intensity).min(remaining);
        let particles = &mut self.particles;
        let rng = &mut self.rng;
        let start = particles.len();
        match emission.kind {
            EmissionKind::Impact { .. } if tier == CombatZoomTier::Mid => {
                if remaining > 0 && intensity > 0.0 {
                    particles.push(impact_flash(emission));
                }
            }
            EmissionKind::Impact { incoming } => {
                for _ in 0..count(6) {
                    particles.push(impact_spark(emission, incoming, rng));
                }
            }
            EmissionKind::Death(victim) => {
                let structure = matches!(victim.appearance, CombatAppearance::Structure(_));
                let full_count = if structure { 16 } else { 8 };
                let nominal = if tier == CombatZoomTier::Mid {
                    full_count / 2
                } else {
                    full_count
                };
                for index in 0..count(nominal) {
                    let mut particle = fragment(emission, structure, rng);
                    particle.close_only = tier == CombatZoomTier::Close && index % 2 == 1;
                    particles.push(particle);
                }
            }
            EmissionKind::DamagedStructure {
                count,
                starts_with_smoke,
            } => {
                for index in 0..count.min(remaining) {
                    let make_smoke = (index % 2 == 0) == starts_with_smoke;
                    let particle = if make_smoke {
                        smoke(emission, rng)
                    } else {
                        structure_spark(emission, rng)
                    };
                    particles.push(particle);
                }
            }
        }
        self.frame_spawns.particles += particles.len() - start;
    }
}

fn particle_count(nominal: u8, intensity: f32) -> usize {
    (f32::from(nominal) * intensity).round() as usize
}

pub(super) struct CombatEffectsPlugin;

impl Plugin for CombatEffectsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ActiveCombatEffects>()
            .add_message::<CombatEffectRequest>()
            .add_systems(PostUpdate, update_combat_effects);
        render::configure(app);
    }
}

pub(super) fn reset_session(world: &mut World) {
    if let Some(mut effects) = world.get_resource_mut::<ActiveCombatEffects>() {
        *effects = ActiveCombatEffects::default();
    }
    if let Some(mut requests) = world.get_resource_mut::<Messages<CombatEffectRequest>>() {
        requests.clear();
    }
}

#[derive(Clone, Copy)]
enum EmissionKind {
    Impact {
        incoming: Vec2,
    },
    Death(CombatVisualSnapshot),
    DamagedStructure {
        count: usize,
        starts_with_smoke: bool,
    },
}

#[derive(Clone, Copy)]
struct Emission {
    position: Vec2,
    color: Color,
    kind: EmissionKind,
}

impl Emission {
    fn is_death(&self) -> bool {
        matches!(self.kind, EmissionKind::Death(_))
    }
}

#[allow(clippy::too_many_arguments)]
fn update_combat_effects(
    mut requests: MessageReader<CombatEffectRequest>,
    mut other_deaths: MessageReader<NonCombatDestruction>,
    mut effects: ResMut<ActiveCombatEffects>,
    view: Res<CombatView>,
    tuning: Res<Tuning>,
    time: Option<Res<Time>>,
    structures: Query<(Entity, &Structure, &Transform, &OwnerSwarm)>,
    swarms: Query<&SwarmId, With<Swarm>>,
    mut emissions: Local<Vec<Emission>>,
) {
    let settings = &tuning.presentation;
    effects.advance(
        time.as_ref().map_or(Duration::ZERO, |time| time.delta()),
        settings.combat_death_duration_secs,
    );

    effects
        .casualties
        .extend(other_deaths.read().filter_map(|event| match *event {
            NonCombatDestruction::Nanobot { victim, .. } => Some(CombatCasualtyFlash::new(victim)),
            NonCombatDestruction::Structure { .. } => None,
        }));

    for request in requests.read() {
        let emission = match *request {
            CombatEffectRequest::Casualty { victim } => {
                effects.casualties.push(CombatCasualtyFlash::new(victim));
                continue;
            }
            _ if view.tier == CombatZoomTier::Far => continue,
            CombatEffectRequest::Muzzle {
                attacker,
                position,
                color,
            } => {
                effects.muzzles.push(MuzzleFlash {
                    attacker,
                    position,
                    color,
                    elapsed: 0.0,
                });
                continue;
            }
            CombatEffectRequest::Impact {
                position,
                incoming,
                color,
            } => Emission {
                position,
                color,
                kind: EmissionKind::Impact { incoming },
            },
            CombatEffectRequest::Death { victim } => {
                let born = effects.clock;
                effects.refresh_decal(CombatDecal {
                    position: victim.position,
                    structure: matches!(victim.appearance, CombatAppearance::Structure(_)),
                    born,
                });
                Emission {
                    position: victim.position,
                    color: swarm_presentation_color(victim.swarm),
                    kind: EmissionKind::Death(victim),
                }
            }
        };
        emissions.push(emission);
    }

    match view.tier {
        CombatZoomTier::Close => collect_structure_emissions(
            &mut effects,
            &view,
            &tuning,
            &structures,
            &swarms,
            &mut emissions,
        ),
        CombatZoomTier::Mid => effects.structure_emissions.clear(),
        CombatZoomTier::Far => {
            effects.structure_emissions.clear();
            effects.muzzles.clear();
            effects.particles.clear();
            effects.death_pops.clear();
            return;
        }
    }

    let center = view.center();
    emissions.sort_by(|a, b| {
        b.is_death().cmp(&a.is_death()).then_with(|| {
            a.position
                .distance_squared(center)
                .total_cmp(&b.position.distance_squared(center))
        })
    });
    for emission in emissions.drain(..) {
        if let EmissionKind::Death(victim) = emission.kind {
            effects.spawn_death_pop(victim, emission.color);
        }
        effects.spawn_particles(emission, settings.effects_intensity, view.tier);
    }
}

fn collect_structure_emissions(
    effects: &mut ActiveCombatEffects,
    view: &CombatView,
    tuning: &Tuning,
    structures: &Query<(Entity, &Structure, &Transform, &OwnerSwarm)>,
    swarms: &Query<&SwarmId, With<Swarm>>,
    emissions: &mut Vec<Emission>,
) {
    let now = effects.clock;
    let max_health = tuning.construction.max_health;
    effects.structure_emissions.retain(|entity, _| {
        structures
            .get(*entity)
            .is_ok_and(|(_, structure, transform, _)| {
                structure_emits(structure, max_health)
                    && view.contains(transform.translation.truncate())
            })
    });
    for (entity, structure, transform, owner) in structures {
        let position = transform.translation.truncate();
        if !structure_emits(structure, max_health) || !view.contains(position) {
            continue;
        }
        let schedule = effects
            .structure_emissions
            .entry(entity)
            .or_insert_with(|| StructureEmission {
                next: now + emission_interval(entity, now),
                particle_credit: 0.0,
                next_is_smoke: false,
            });
        if now < schedule.next {
            continue;
        }
        schedule.next = now + emission_interval(entity, now);
        let Ok(&swarm) = swarms.get(owner.0) else {
            continue;
        };
        // Carry fractional particles across intervals and alternate the two cues.
        schedule.particle_credit += 2.0 * tuning.presentation.effects_intensity;
        let count = schedule.particle_credit as usize;
        schedule.particle_credit -= count as f32;
        if count == 0 {
            continue;
        }
        let starts_with_smoke = schedule.next_is_smoke;
        schedule.next_is_smoke ^= count % 2 != 0;
        emissions.push(Emission {
            position,
            color: swarm_presentation_color(swarm),
            kind: EmissionKind::DamagedStructure {
                count,
                starts_with_smoke,
            },
        });
    }
}

fn structure_emits(structure: &Structure, max_health: u32) -> bool {
    structure.health > 0 && structure.health < max_health.div_ceil(2)
}

fn emission_interval(entity: Entity, now: f64) -> f64 {
    let mut rng = SmallRng::seed_from_u64(entity.to_bits() ^ now.to_bits());
    rng.random_range(0.3..=0.6)
}

fn impact_spark(emission: Emission, incoming: Vec2, rng: &mut SmallRng) -> CombatParticle {
    let angle = (-incoming).normalize_or(Vec2::X).to_angle()
        + rng.random_range(-50.0_f32..=50.0).to_radians();
    CombatParticle {
        position: emission.position,
        color: emission.color,
        kind: CombatParticleKind::ImpactSpark,
        velocity: Vec2::from_angle(angle) * rng.random_range(120.0..=260.0),
        elapsed: 0.0,
        lifetime: 0.18,
        angle,
        spin: 0.0,
        size: rng.random_range(4.0..=8.0),
        close_only: true,
    }
}

fn impact_flash(emission: Emission) -> CombatParticle {
    CombatParticle {
        position: emission.position,
        color: emission.color,
        kind: CombatParticleKind::ImpactFlash,
        velocity: Vec2::ZERO,
        elapsed: 0.0,
        lifetime: 0.12,
        angle: 0.0,
        spin: 0.0,
        size: 16.0,
        close_only: false,
    }
}

fn fragment(emission: Emission, structure: bool, rng: &mut SmallRng) -> CombatParticle {
    let angle = rng.random_range(0.0..std::f32::consts::TAU);
    let direction = Vec2::from_angle(angle);
    CombatParticle {
        position: emission.position + direction * if structure { 7.0 } else { 3.0 },
        color: emission.color,
        kind: CombatParticleKind::Fragment,
        velocity: direction * rng.random_range(75.0..=165.0),
        elapsed: 0.0,
        lifetime: if structure { 0.4 } else { 0.6 },
        angle,
        spin: rng.random_range(-15.0..=15.0),
        size: rng.random_range(3.5..=8.0),
        close_only: false,
    }
}

fn structure_spark(emission: Emission, rng: &mut SmallRng) -> CombatParticle {
    let angle = rng.random_range(0.0..std::f32::consts::TAU);
    let direction = Vec2::from_angle(angle);
    CombatParticle {
        position: emission.position + direction * rng.random_range(2.0..=8.0),
        color: emission.color,
        kind: CombatParticleKind::StructureSpark,
        velocity: direction * rng.random_range(120.0..=190.0),
        elapsed: 0.0,
        lifetime: 0.18,
        angle,
        spin: 0.0,
        size: 7.0,
        close_only: true,
    }
}

fn smoke(emission: Emission, rng: &mut SmallRng) -> CombatParticle {
    CombatParticle {
        position: emission.position,
        color: Color::srgb(0.45, 0.49, 0.53),
        kind: CombatParticleKind::Smoke,
        velocity: Vec2::new(
            rng.random_range(-12.0..=12.0),
            rng.random_range(24.0..=36.0),
        ),
        elapsed: 0.0,
        lifetime: 1.1,
        angle: rng.random_range(0.0..std::f32::consts::TAU),
        spin: rng.random_range(-1.0..=1.0),
        size: 12.0,
        close_only: true,
    }
}
