use std::collections::{BTreeMap, HashMap, VecDeque};

use bevy::prelude::{Entity, IVec2, Transform, Vec2};

use crate::{
    nanobot::{
        DropOffArea, InteractionRegion, ResolvedCombatHit, STOP_THRESHOLD, SwarmId, world_to_cell,
    },
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
    presence: HashMap<(SwarmId, u64), DefenderPresence>,
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
    gather_cell: IVec2,
    last_attack: Option<u64>,
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

    pub(super) fn update_presence(
        &mut self,
        tick: u64,
        mines: impl Iterator<Item = (SwarmId, u64, [IVec2; 2])>,
        deposits: &[DepositState],
        tuning: &Tuning,
    ) {
        let mut mines = mines.peekable();
        if self.defenders.is_empty() || mines.peek().is_none() {
            self.presence.clear();
            return;
        }
        let range = tuning.combat.attack_range;
        let max_radius = deposits
            .iter()
            .map(|deposit| deposit.radius)
            .fold(0.0, f32::max);
        let defenders = PackedSpatialBuckets::new(
            range + WORK_RING_CLEARANCE + max_radius,
            self.defenders
                .values()
                .map(|&(owner, point)| (point, (owner, point))),
        );
        for (owner, id, cells) in mines {
            let Ok(index) = deposits.binary_search_by_key(&id, |deposit| deposit.id) else {
                continue;
            };
            let deposit = &deposits[index];
            if deposit.amount == 0 {
                continue;
            }
            let band = InteractionRegion::deposit(
                &Transform::from_translation(deposit.position.extend(0.0)),
                deposit.radius,
            )
            .within_cell(cells[0]);
            let bucket = defenders.bucket_for_position(deposit.position);
            let max_distance_squared = (range + WORK_RING_CLEARANCE + deposit.radius).powi(2);
            let count = defenders
                .neighbourhood_runs(bucket, 1)
                .flatten()
                .filter(|&&(swarm, point)| {
                    swarm != owner
                        && point.distance_squared(deposit.position) <= max_distance_squared
                        && band.deposit_work_within_range(point, range)
                })
                .count() as u32;
            if count == 0 {
                continue;
            }

            let key = (owner, id);
            let continuing = self.presence.get(&key).filter(|presence| {
                presence.last_tick.saturating_add(1) == tick && presence.gather_cell == cells[0]
            });
            let first_tick = continuing.map_or(tick, |presence| presence.first_tick);
            let last_attack = continuing.and_then(|presence| presence.last_attack);
            let area = DropOffArea::for_deposit(deposit.position, deposit.radius, tuning);
            let last_attack = damage_sites(deposit, cells)
                .into_iter()
                .filter_map(|site| {
                    self.damage
                        .get(&(owner, site))?
                        .iter()
                        .rev()
                        .take_while(|hit| hit.tick == tick)
                        .find(|hit| {
                            !matches!(site, DamageSite::Deposit(_)) || area.contains(hit.target)
                        })
                        .map(|hit| hit.tick)
                })
                .max()
                .or(last_attack);
            self.presence.insert(
                key,
                DefenderPresence {
                    first_tick,
                    last_tick: tick,
                    count,
                    gather_cell: cells[0],
                    last_attack,
                },
            );
        }
        self.presence
            .retain(|_, presence| presence.last_tick == tick);
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
        if let Some(presence) = self.presence.get(&(owner, deposit.id))
            && presence.gather_cell == cells[0]
        {
            pressure.armed_defenders = presence.count;
            pressure.armed_presence_ticks = tick.saturating_sub(presence.first_tick) + 1;
            pressure.ticks_since_attack_during_presence =
                presence.last_attack.map(|attack| tick - attack);
        }
        let area = DropOffArea::for_deposit(deposit.position, deposit.radius, tuning);
        let nearby_attack = (tuning.combat.attack_range + 2.0 * BODY_RADIUS).powi(2);
        for site in damage_sites(deposit, cells) {
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
                if matches!(site, DamageSite::Deposit(_)) && !area.contains(hit.target) {
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

fn damage_sites(deposit: &DepositState, cells: [IVec2; 2]) -> [DamageSite; 3] {
    [
        DamageSite::Cell(cells[0]),
        DamageSite::Cell(cells[1]),
        DamageSite::Deposit(Entity::from_bits(deposit.id)),
    ]
}
