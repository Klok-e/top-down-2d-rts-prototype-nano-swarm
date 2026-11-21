use std::{collections::HashMap, time::Duration};

use bevy::prelude::*;

use crate::{
    nanobot::{Health, SwarmId, SwarmMember},
    spatial::FixedSpatialBuckets,
};

use super::{
    CombatTickSnapshots, Nanobot, NanobotType, ResolvedCombatFact, UpdateNanobotPresentation,
    resolve_combat, swarm_presentation_color,
};

const CELL_SIZE: f32 = 512.0;
const QUIET_PERIOD: Duration = Duration::from_secs(2);
const WINNER_RANGE_SQUARED: f32 = 384.0 * 384.0;
const RING_LIFETIME: Duration = Duration::from_millis(500);
const MAX_RING_RADIUS: f32 = 160.0;

#[derive(Debug, Clone, Copy)]
pub struct SkirmishRing {
    pub center: Vec2,
    pub color: Color,
    elapsed: Duration,
}

impl SkirmishRing {
    pub fn radius(&self) -> f32 {
        MAX_RING_RADIUS * self.elapsed.as_secs_f32() / RING_LIFETIME.as_secs_f32()
    }
}

#[derive(Debug)]
struct HitCell {
    first_swarm: SwarmId,
    exchanged_hits: bool,
    last_hit: Duration,
}

#[derive(Debug, Default, Resource)]
pub struct ActiveSkirmishRings {
    hit_cells: HashMap<IVec2, HitCell>,
    rings: Vec<SkirmishRing>,
}

impl ActiveSkirmishRings {
    pub fn iter(&self) -> impl ExactSizeIterator<Item = &SkirmishRing> {
        self.rings.iter()
    }
}

pub(super) struct SkirmishEndPlugin;

impl Plugin for SkirmishEndPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ActiveSkirmishRings>().add_systems(
            Update,
            update_skirmish_rings
                .before(resolve_combat)
                .in_set(UpdateNanobotPresentation),
        );
    }
}

pub(super) fn reset_session(world: &mut World) {
    crate::nanobot::reset_resource::<ActiveSkirmishRings>(world);
}

fn surviving_swarm(
    defenders: &FixedSpatialBuckets<(Vec2, SwarmId)>,
    cell: IVec2,
    center: Vec2,
) -> Option<SwarmId> {
    let mut winner = None;
    for (_, neighbors) in defenders.neighbourhood(cell, 1) {
        for &(position, swarm) in neighbors {
            if position.distance_squared(center) <= WINNER_RANGE_SQUARED {
                if winner.is_some_and(|winner| winner != swarm) {
                    return None;
                }
                winner = Some(swarm);
            }
        }
    }
    winner
}

fn update_skirmish_rings(
    mut facts: MessageReader<ResolvedCombatFact>,
    snapshots: Res<CombatTickSnapshots>,
    time: Option<Res<Time>>,
    bots: Query<(&Transform, &NanobotType, &SwarmMember, &Health), With<Nanobot>>,
    mut state: ResMut<ActiveSkirmishRings>,
) {
    let (now, delta) = time.map_or((Duration::ZERO, Duration::ZERO), |time| {
        (time.elapsed(), time.delta())
    });
    let ActiveSkirmishRings { hit_cells, rings } = &mut *state;
    rings.retain_mut(|ring| {
        ring.elapsed += delta;
        ring.elapsed < RING_LIFETIME
    });

    for (fact, id) in facts.read_with_id() {
        let ResolvedCombatFact::Hit(hit) = fact else {
            continue;
        };
        let coord = (hit.target.position / CELL_SIZE).floor().as_ivec2();
        let hit_at = snapshots.hit_time(id).unwrap_or(now);
        let cell = hit_cells.entry(coord).or_insert(HitCell {
            first_swarm: hit.attacker.swarm,
            exchanged_hits: false,
            last_hit: hit_at,
        });
        cell.exchanged_hits |= cell.first_swarm != hit.attacker.swarm;
        cell.last_hit = hit_at;
    }

    let mut defenders = None;
    hit_cells.retain(|&coord, cell| {
        if now.saturating_sub(cell.last_hit) < QUIET_PERIOD {
            return true;
        }
        if cell.exchanged_hits {
            let defenders = defenders.get_or_insert_with(|| {
                let mut defenders = FixedSpatialBuckets::new(CELL_SIZE);
                for (transform, kind, member, health) in &bots {
                    if *kind == NanobotType::Defender && health.current > 0 {
                        let position = transform.translation.truncate();
                        defenders.insert(position, (position, member.0));
                    }
                }
                defenders
            });
            let center = (coord.as_vec2() + Vec2::splat(0.5)) * CELL_SIZE;
            if let Some(winner) = surviving_swarm(defenders, coord, center) {
                rings.push(SkirmishRing {
                    center,
                    color: swarm_presentation_color(winner).with_alpha(0.4),
                    elapsed: Duration::ZERO,
                });
            }
        }
        false
    });
}
