#![allow(dead_code)]

use std::collections::HashMap;

use bevy::{ecs::message::MessageCursor, prelude::*, time::TimeUpdateStrategy};
use top_down_2d_rts_prototype_nano_swarm::{
    GAMEPLAY_SPRITE_Z, ZONE_BLOCK_SIZE,
    intent::{IntentGrid, IntentKind},
    nanobot::{
        Charge, CombatPlugin, Commitment, DefenderRout, Health, Nanobot, NanobotBundle,
        NanobotPlugin, NanobotType, OpponentSwarm, RegionalAllocationPlugin, ResolvedCombatFact,
        SwarmBundle, SwarmId, SwarmMember,
    },
    navigation::BODY_RADIUS,
    reachability::SwarmReachabilityMap,
    resources::ResourceLedger,
    spatial::FixedSpatialBuckets,
    tuning::Tuning,
};

const SAMPLE_TICKS: u32 = 30;
const FIRING_WINDOW: u32 = 60;
const NEAREST_NEIGHBORS: usize = 8;
const PILE_UP_DISTANCE: f32 = 0.6 * 2.0 * BODY_RADIUS;

#[derive(Clone, Copy)]
pub enum Scene {
    Skirmish,
    Clash,
    Battle,
    BenchBattle,
}

impl Scene {
    pub fn name(self) -> &'static str {
        match self {
            Self::Skirmish => "skirmish",
            Self::Clash => "clash",
            Self::Battle => "battle",
            Self::BenchBattle => "bench_battle",
        }
    }

    pub fn per_side(self) -> usize {
        match self {
            Self::Skirmish => 4,
            Self::Clash => 40,
            Self::Battle => 300,
            Self::BenchBattle => 1_000,
        }
    }

    pub fn center(self) -> Vec2 {
        let y = match self {
            Self::Skirmish => 10.0,
            Self::Clash | Self::Battle => 5.0,
            Self::BenchBattle => 6.0,
        };
        (Vec2::new(0.0, y) + Vec2::splat(0.5)) * ZONE_BLOCK_SIZE
    }

    fn offset(self) -> f32 {
        match self {
            Self::Skirmish => 260.0,
            Self::Clash => 450.0,
            Self::Battle => 920.0,
            Self::BenchBattle => 1_550.0,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Variant {
    Standard,
    Mirrored,
    ReversedSpawn,
}

impl Variant {
    pub fn name(self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::Mirrored => "mirrored",
            Self::ReversedSpawn => "reversed_spawn",
        }
    }
}

pub fn spawn_scene(world: &mut World, scene: Scene, variant: Variant) {
    let opponent = world
        .query_filtered::<&SwarmId, With<OpponentSwarm>>()
        .single(world)
        .copied()
        .expect("the combat scene needs exactly one opponent swarm");
    let mut grid = IntentGrid::new(64, 64);
    for y in 3..=12 {
        for x in -6..=6 {
            let cell = IVec2::new(x, y);
            grid.paint(cell, IntentKind::Defend, SwarmId::PLAYER);
            grid.paint(cell, IntentKind::Defend, opponent);
        }
    }

    let offset = Vec2::X * scene.offset();
    let offset = if variant == Variant::Mirrored {
        -offset
    } else {
        offset
    };
    let mut armies = [(SwarmId::PLAYER, -offset), (opponent, offset)];
    let homes = armies.map(|(swarm, delta)| (swarm, scene.center() + delta));
    let reachability = SwarmReachabilityMap::new(&grid, [], homes)
        .expect("both army centers are reachable in the empty arena");
    world.insert_resource(grid);
    world.insert_resource(reachability);
    if variant == Variant::ReversedSpawn {
        armies.reverse();
    }
    let health = Health::with_tuning(world.resource::<Tuning>());
    for (swarm, offset) in armies {
        let center = scene.center() + offset;
        for index in 0..scene.per_side() {
            let index = index as f32;
            let radius = 44.0 * (index + 0.5).sqrt();
            let angle = index * 2.399_963;
            let position = center + Vec2::from_angle(angle) * radius;
            world.spawn((
                NanobotBundle {
                    nanobot_type: NanobotType::Defender,
                    health,
                    swarm_member: SwarmMember::new(swarm),
                    ..Default::default()
                },
                Charge {
                    current: 1.0,
                    max: 1.0,
                },
                Commitment::Idle,
                Transform::from_translation(position.extend(GAMEPLAY_SPRITE_Z)),
            ));
        }
    }
}

pub fn headless_scene(scene: Scene, variant: Variant) -> App {
    let mut app = scene_app();
    spawn_scene(app.world_mut(), scene, variant);
    app
}

pub fn scene_app() -> App {
    let mut app = App::new();
    let fixed_time = Time::<Fixed>::from_hz(60.0);
    app.add_plugins(bevy::time::TimePlugin)
        .insert_resource(TimeUpdateStrategy::ManualDuration(fixed_time.timestep()))
        .insert_resource(fixed_time)
        .init_resource::<Tuning>()
        .init_resource::<ResourceLedger>()
        .add_plugins((
            NanobotPlugin::default(),
            CombatPlugin,
            RegionalAllocationPlugin,
        ));
    app.world_mut().spawn(SwarmBundle::default());
    app.world_mut().spawn((
        SwarmBundle {
            swarm_id: SwarmId(1),
            ..Default::default()
        },
        OpponentSwarm {},
    ));
    app
}

type DefenderQuery = (
    Entity,
    &'static Transform,
    &'static SwarmMember,
    &'static NanobotType,
    &'static Health,
    Has<DefenderRout>,
);

#[derive(Clone, Copy)]
struct Defender {
    entity: Entity,
    position: Vec2,
    swarm: SwarmId,
    routing: bool,
}

#[derive(Default)]
struct DefenderHistory {
    last_hit: Option<u32>,
    in_range_ticks: u32,
    has_routed: bool,
}

impl DefenderHistory {
    fn hit_in_window(&self, tick: u32, window: u32) -> bool {
        self.last_hit.is_some_and(|hit| tick - hit < window)
    }
}

#[derive(Debug)]
pub struct CombatSample {
    pub tick: u32,
    pub alive: [usize; 2],
    pub firing_fraction: f64,
    pub idle_in_range: usize,
    pub mixing_fraction: f64,
    pub pile_ups: usize,
}

#[derive(Debug)]
pub struct CombatSummary {
    pub contact_tick: Option<u32>,
    pub resolution_tick: Option<u32>,
    pub losing_sides: [bool; 2],
    pub final_alive: [usize; 2],
    pub routed_before_resolution: [usize; 2],
    pub median_firing: Option<f64>,
    pub peak_mixing: f64,
    pub median_mixing: Option<f64>,
    pub peak_mixing_both_at_30_percent: Option<f64>,
    pub peak_idle_in_range: usize,
    pub peak_pile_ups: usize,
}

fn median(values: impl Iterator<Item = f64>) -> Option<f64> {
    let mut values = values.collect::<Vec<_>>();
    values.sort_unstable_by(f64::total_cmp);
    let upper_middle = values.len() / 2;
    let lower_middle = values.len().saturating_sub(1) / 2;
    Some((values.get(lower_middle)? + values.get(upper_middle)?) / 2.0)
}

pub struct CombatMetrics {
    defenders: QueryState<DefenderQuery, With<Nanobot>>,
    hits: MessageCursor<ResolvedCombatFact>,
    history: HashMap<Entity, DefenderHistory>,
    bodies: Vec<Defender>,
    range_buckets: FixedSpatialBuckets<Defender>,
    starting_per_side: usize,
    alive: [usize; 2],
    losing_sides: [bool; 2],
    routed_before_resolution: [usize; 2],
    pub contact_tick: Option<u32>,
    pub resolution_tick: Option<u32>,
    pub samples: Vec<CombatSample>,
}

impl CombatMetrics {
    pub fn new(world: &mut World, scene: Scene) -> Self {
        Self {
            defenders: world.query_filtered(),
            hits: MessageCursor::default(),
            history: HashMap::new(),
            bodies: Vec::new(),
            range_buckets: FixedSpatialBuckets::new(world.resource::<Tuning>().combat.attack_range),
            starting_per_side: scene.per_side(),
            alive: [scene.per_side(); 2],
            losing_sides: [false; 2],
            routed_before_resolution: [0; 2],
            contact_tick: None,
            resolution_tick: None,
            samples: Vec::new(),
        }
    }

    pub fn observe_tick(&mut self, world: &World, tick: u32, label: &str) {
        for fact in self
            .hits
            .read(world.resource::<Messages<ResolvedCombatFact>>())
        {
            if let ResolvedCombatFact::Hit(hit) = fact {
                self.contact_tick.get_or_insert(tick);
                self.history
                    .entry(hit.attacker.entity)
                    .or_default()
                    .last_hit = Some(tick);
            }
        }

        self.bodies.clear();
        self.range_buckets.clear();
        let mut alive = [0; 2];
        let mut routing = [0; 2];
        for (entity, transform, member, kind, health, is_routing) in self.defenders.iter(world) {
            assert!(
                transform.translation.is_finite()
                    && transform.rotation.is_finite()
                    && transform.scale.is_finite(),
                "{label}: non-finite transform for {entity} at tick {tick}: {transform:?}",
            );
            if *kind == NanobotType::Defender && health.current > 0 {
                let position = transform.translation.truncate();
                let defender = Defender {
                    entity,
                    position,
                    swarm: member.0,
                    routing: is_routing,
                };
                let side = usize::from(!member.0.is_player());
                alive[side] += 1;
                routing[side] += usize::from(is_routing);
                self.bodies.push(defender);
                self.range_buckets.insert(position, defender);
            }
        }
        assert!(
            alive
                .into_iter()
                .zip(self.alive)
                .all(|(now, previous)| now <= previous),
            "{label}: alive count increased at tick {tick} from {:?} to {alive:?}",
            self.alive,
        );
        self.alive = alive;
        if self.resolution_tick.is_none() {
            let losing_sides = std::array::from_fn(|side| {
                (alive[side] - routing[side]) * 10 < self.starting_per_side
            });
            if losing_sides.into_iter().any(|lost| lost) {
                self.resolution_tick = Some(tick);
                self.losing_sides = losing_sides;
            }
        }

        let combat = &world.resource::<Tuning>().combat;
        let range_squared = combat.attack_range.powi(2);
        for defender in &self.bodies {
            let bucket = self.range_buckets.bucket_for_position(defender.position);
            let in_range = self
                .range_buckets
                .neighbourhood(bucket, 1)
                .flat_map(|(_, neighbors)| neighbors)
                .any(|other| {
                    other.swarm != defender.swarm
                        && defender.position.distance_squared(other.position) <= range_squared
                });
            let history = self.history.entry(defender.entity).or_default();
            if self.resolution_tick.is_none() && defender.routing && !history.has_routed {
                self.routed_before_resolution[usize::from(!defender.swarm.is_player())] += 1;
                history.has_routed = true;
            }
            history.in_range_ticks = if in_range {
                history.in_range_ticks + 1
            } else {
                0
            };
        }
        if tick.is_multiple_of(SAMPLE_TICKS) {
            self.sample(tick, 2 * u32::from(combat.attack_interval_ticks));
        }
    }

    fn sample(&mut self, tick: u32, idle_window: u32) {
        let (mut firing, mut idle_in_range, mut mixed, mut pile_ups) = (0, 0, 0, 0);
        let mut neighbors = Vec::with_capacity(self.bodies.len());
        for defender in &self.bodies {
            let history = &self.history[&defender.entity];
            firing += usize::from(history.hit_in_window(tick, FIRING_WINDOW));
            idle_in_range += usize::from(
                history.in_range_ticks >= idle_window && !history.hit_in_window(tick, idle_window),
            );

            neighbors.clear();
            for other in &self.bodies {
                if other.entity == defender.entity {
                    continue;
                }
                let distance = defender.position.distance_squared(other.position);
                if self.contact_tick.is_some()
                    && distance < PILE_UP_DISTANCE.powi(2)
                    && defender.entity.to_bits() < other.entity.to_bits()
                {
                    pile_ups += 1;
                }
                neighbors.push((distance, other.entity.to_bits(), other.swarm));
            }
            if neighbors.len() > NEAREST_NEIGHBORS {
                neighbors.select_nth_unstable_by(NEAREST_NEIGHBORS, |left, right| {
                    left.0
                        .total_cmp(&right.0)
                        .then_with(|| left.1.cmp(&right.1))
                });
                neighbors.truncate(NEAREST_NEIGHBORS);
            }
            let hostile_neighbors = neighbors
                .iter()
                .filter(|(_, _, swarm)| *swarm != defender.swarm)
                .count();
            mixed += usize::from(hostile_neighbors * 2 > neighbors.len());
        }
        let total = self.bodies.len().max(1) as f64;
        self.samples.push(CombatSample {
            tick,
            alive: self.alive,
            firing_fraction: firing as f64 / total,
            idle_in_range,
            mixing_fraction: mixed as f64 / total,
            pile_ups,
        });
    }

    pub fn summary(&self) -> CombatSummary {
        let mixing = self.samples.iter().map(|sample| sample.mixing_fraction);
        let firing_after_contact = self.samples.iter().filter_map(|sample| {
            self.contact_tick
                .is_some_and(|contact| sample.tick >= contact)
                .then_some(sample.firing_fraction)
        });
        CombatSummary {
            contact_tick: self.contact_tick,
            resolution_tick: self.resolution_tick,
            losing_sides: self.losing_sides,
            final_alive: self.alive,
            routed_before_resolution: self.routed_before_resolution,
            median_firing: median(firing_after_contact),
            peak_mixing: mixing.clone().fold(0.0, f64::max),
            median_mixing: median(mixing),
            peak_mixing_both_at_30_percent: self
                .samples
                .iter()
                .filter(|sample| {
                    sample
                        .alive
                        .into_iter()
                        .all(|count| count * 10 >= self.starting_per_side * 3)
                })
                .map(|sample| sample.mixing_fraction)
                .reduce(f64::max),
            peak_idle_in_range: self
                .samples
                .iter()
                .map(|sample| sample.idle_in_range)
                .max()
                .unwrap_or_default(),
            peak_pile_ups: self
                .samples
                .iter()
                .map(|sample| sample.pile_ups)
                .max()
                .unwrap_or_default(),
        }
    }
}
