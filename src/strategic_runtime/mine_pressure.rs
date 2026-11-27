use std::collections::{BTreeMap, HashMap, VecDeque};

use bevy::prelude::{Entity, IVec2, Vec2};

use crate::{
    nanobot::{DropOffArea, ResolvedCombatHit, STOP_THRESHOLD, SwarmId, world_to_cell},
    navigation::BODY_RADIUS,
    spatial::PackedSpatialBuckets,
    strategic_controller::{DepositState, MinePressure},
    tuning::Tuning,
};

const DAMAGE_WINDOW_TICKS: u64 = 2 * crate::SIMULATION_HZ as u64;
const WORK_RING_CLEARANCE: f32 = BODY_RADIUS + 2.0 * STOP_THRESHOLD;

#[derive(Default)]
pub(super) struct MinePressureHistory {
    owners: BTreeMap<Entity, SwarmId>,
    damage: HashMap<(SwarmId, DamageSite), VecDeque<Damage>>,
    mine_links: HashMap<Entity, Vec<Entity>>,
    defenders: HashMap<Entity, (SwarmId, Vec2)>,
    presence: HashMap<u64, HashMap<SwarmId, DefenderPresence>>,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum DamageSite {
    Cell(IVec2),
    Deposit(Entity),
}

#[derive(Clone, Copy)]
struct Damage {
    tick: u64,
    attacker: Entity,
    target: Vec2,
    amount: u32,
}

struct DefenderPresence {
    first_tick: u64,
    last_tick: u64,
    count: u32,
}

impl MinePressureHistory {
    pub(super) fn begin_tick(&mut self, owners: &BTreeMap<Entity, SwarmId>, tick: u64) -> bool {
        let session_changed = &self.owners != owners;
        if session_changed {
            *self = Self::default();
            self.owners.clone_from(owners);
        }
        self.defenders.clear();
        self.damage.retain(|_, hits| {
            while hits
                .front()
                .is_some_and(|hit| tick.saturating_sub(hit.tick) >= DAMAGE_WINDOW_TICKS)
            {
                hits.pop_front();
            }
            !hits.is_empty()
        });
        session_changed
    }

    pub(super) fn record_hit(&mut self, tick: u64, hit: ResolvedCombatHit) {
        if hit.attacker.swarm == hit.target.swarm || hit.damage == 0 {
            return;
        }
        let damage = Damage {
            tick,
            attacker: hit.attacker.entity,
            target: hit.target.position,
            amount: hit.damage,
        };
        if let Some(deposits) = self.mine_links.get(&hit.target.entity) {
            for &deposit in deposits {
                self.damage
                    .entry((hit.target.swarm, DamageSite::Deposit(deposit)))
                    .or_default()
                    .push_back(damage);
            }
        } else {
            let cell = world_to_cell(hit.target.position);
            self.damage
                .entry((hit.target.swarm, DamageSite::Cell(cell)))
                .or_default()
                .push_back(damage);
        }
    }

    pub(super) fn clear_links(&mut self) {
        self.mine_links.clear();
    }

    pub(super) fn link_mine_work(&mut self, target: Entity, deposit: Entity) {
        let mines = self.mine_links.entry(target).or_default();
        if !mines.contains(&deposit) {
            mines.push(deposit);
        }
    }

    pub(super) fn record_defender(&mut self, entity: Entity, owner: SwarmId, position: Vec2) {
        self.defenders.insert(entity, (owner, position));
    }

    pub(super) fn update_presence(&mut self, tick: u64, deposits: &[DepositState], range: f32) {
        if self.defenders.is_empty() || deposits.is_empty() {
            self.presence.clear();
            return;
        }
        let ring_reach = range + WORK_RING_CLEARANCE;
        let max_radius = deposits
            .iter()
            .map(|deposit| deposit.radius)
            .fold(0.0, f32::max);
        let defenders = PackedSpatialBuckets::new(
            ring_reach + max_radius,
            self.defenders
                .values()
                .map(|&(owner, point)| (point, (owner, point))),
        );
        for deposit in deposits.iter().filter(|deposit| deposit.amount > 0) {
            let radius_squared = (deposit.radius + ring_reach).powi(2);
            let bucket = defenders.bucket_for_position(deposit.position);
            for &(owner, point) in defenders.neighbourhood_runs(bucket, 1).flatten() {
                if point.distance_squared(deposit.position) > radius_squared {
                    continue;
                }
                let presence = self
                    .presence
                    .entry(deposit.id)
                    .or_default()
                    .entry(owner)
                    .or_insert(DefenderPresence {
                        first_tick: tick,
                        last_tick: tick,
                        count: 0,
                    });
                if presence.last_tick != tick {
                    if presence.last_tick.saturating_add(1) != tick {
                        presence.first_tick = tick;
                    }
                    presence.last_tick = tick;
                    presence.count = 0;
                }
                presence.count += 1;
            }
        }
        self.presence.retain(|_, owners| {
            owners.retain(|_, presence| presence.last_tick == tick);
            !owners.is_empty()
        });
    }

    pub(super) fn observe(
        &self,
        tick: u64,
        owner: SwarmId,
        deposit: &DepositState,
        cells: [IVec2; 2],
        tuning: &Tuning,
        allow_work: &mut dyn FnMut() -> bool,
    ) -> Option<MinePressure> {
        let mut pressure = MinePressure::default();
        if let Some(presence) = self.presence.get(&deposit.id) {
            for (&swarm, defenders) in presence {
                if !allow_work() {
                    return None;
                }
                if swarm != owner {
                    pressure.armed_defenders += defenders.count;
                    pressure.armed_presence_ticks = pressure
                        .armed_presence_ticks
                        .max(tick.saturating_sub(defenders.first_tick) + 1);
                }
            }
        }
        let area = DropOffArea::for_deposit(deposit.position, deposit.radius, tuning);
        let nearby_attack = (tuning.combat.attack_range + 2.0 * BODY_RADIUS).powi(2);
        let sites = cells.map(DamageSite::Cell);
        let linked = DamageSite::Deposit(Entity::from_bits(deposit.id));
        for site in sites.into_iter().chain([linked]) {
            if !allow_work() {
                return None;
            }
            let Some(hits) = self.damage.get(&(owner, site)) else {
                continue;
            };
            for hit in hits {
                if !allow_work() {
                    return None;
                }
                if site == linked && !area.contains(hit.target) {
                    continue;
                }
                let index = usize::from(world_to_cell(hit.target) == cells[1]);
                let amount = u64::from(hit.amount);
                pressure.recent_damage[index] += amount;
                if self
                    .defenders
                    .get(&hit.attacker)
                    .is_some_and(|&(_, point)| point.distance_squared(hit.target) <= nearby_attack)
                {
                    pressure.ongoing_damage += amount;
                }
            }
        }
        Some(pressure)
    }
}
