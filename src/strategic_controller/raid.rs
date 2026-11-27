use super::{lifecycle::WorkBudget, *};

const MINIMUM_CHARGED_DEFENDERS: usize = 2;
const MINIMUM_FORCE_STRENGTH: f32 = 1.5;
const FORCE_ADVANTAGE: f32 = 1.15;
const OPPOSITION_RADIUS: f32 = crate::ZONE_BLOCK_SIZE;
pub(super) const READINESS_GRACE_TICKS: u64 = 5 * crate::SIMULATION_HZ as u64;

#[derive(Default)]
pub(super) struct RaidForces<'a> {
    friendly: Vec<(&'a BotState, f32)>,
    hostile: Vec<(&'a BotState, f32)>,
    potential_strength: f32,
}

impl<'a> RaidForces<'a> {
    pub(super) fn observe(owner: SwarmId, state: &GameState<'a>, budget: &mut WorkBudget) -> Self {
        let mut forces = Self::default();
        for bot in state.bots {
            if !budget.spend(1) {
                break;
            }
            if bot.kind != NanobotType::Defender || bot.health == 0 || !bot.position.is_finite() {
                continue;
            }
            let charge = if bot.charge.is_finite() && bot.max_charge > 0.0 {
                (bot.charge / bot.max_charge).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let health = bot.health.min(bot.max_health) as f32 / bot.max_health.max(1) as f32;
            let strength = health * charge;
            if bot.owner == owner {
                if bot.charge > state.charge.low_threshold
                    && strength > 0.0
                    && bot.defender_duty != DefenderDuty::Unavailable
                {
                    forces.friendly.push((bot, strength));
                    forces.potential_strength += strength;
                }
            } else if strength > 0.0 {
                forces.hostile.push((bot, strength));
            }
        }
        forces
    }

    pub(super) fn can_launch(&self) -> bool {
        self.friendly.len() >= MINIMUM_CHARGED_DEFENDERS
            && self.potential_strength >= MINIMUM_FORCE_STRENGTH
    }

    pub(super) fn ready(
        &self,
        approach: &[Vec2],
        target: Vec2,
        contributes: impl Fn(&BotState) -> bool,
        budget: &mut WorkBudget,
    ) -> Option<bool> {
        if !self.can_launch() {
            return Some(false);
        }
        let mut available = 0;
        let mut friendly = 0.0;
        for &(bot, strength) in &self.friendly {
            if !budget.spend(1) {
                return None;
            }
            if !contributes(bot) {
                continue;
            }
            available += 1;
            friendly += strength;
        }
        if available < MINIMUM_CHARGED_DEFENDERS || friendly < MINIMUM_FORCE_STRENGTH {
            return Some(false);
        }
        let mut opposition = 0.0;
        for &(bot, strength) in &self.hostile {
            if !budget.spend(1) {
                return None;
            }
            if near_approach(bot.position, approach, target, budget)? {
                opposition += strength;
                if friendly < opposition * FORCE_ADVANTAGE {
                    return Some(false);
                }
            }
        }
        Some(true)
    }
}

fn near_approach(
    position: Vec2,
    route: &[Vec2],
    target: Vec2,
    budget: &mut WorkBudget,
) -> Option<bool> {
    let radius_squared = OPPOSITION_RADIUS.powi(2);
    if position.distance_squared(target) <= radius_squared {
        return Some(true);
    }
    for segment in route.windows(2) {
        if !budget.spend(1) {
            return None;
        }
        let start = segment[0];
        let offset = segment[1] - start;
        let length_squared = offset.length_squared();
        let closest = if length_squared > 0.0 {
            let fraction = ((position - start).dot(offset) / length_squared).clamp(0.0, 1.0);
            start + fraction * offset
        } else {
            start
        };
        if position.distance_squared(closest) <= radius_squared {
            return Some(true);
        }
    }
    Some(false)
}
