use bevy::prelude::{Component, Vec2};

#[derive(Debug, Component, Default)]
pub struct Nanobot {}

/// Marker for the swarm that owns a population of nanobots. Replaces the old
/// per-group entity that owned nanobots and their zones.
#[derive(Debug, Component, Default)]
pub struct Swarm {}

#[derive(Debug, Component)]
pub struct DirectMovementComponent {
    pub xy: Vec2,
}

#[derive(Debug, Component, Clone, Copy, Default)]
pub struct VelocityComponent {
    pub value: Vec2,
}

#[derive(Debug, Component)]
pub struct ProgressChecker {
    pub last_position: Vec2,
    pub last_update_time: f64,
}

/// Health of a nanobot. All nanobots carry one so future
/// damage sources (combat, accidents, environmental hazards)
/// can drain a common pool. The charge loop is the first
/// consumer: an empty/ignored Charge drains defender health
/// at a per-tick rate (see `src/nanobot/charge.rs`).
///
/// `current` is in `[0, max]`. The first implementation only
/// drains defender health from the charge loop; a future
/// combat layer will share the same component.
#[derive(Debug, Component, Clone, Copy)]
pub struct Health {
    pub current: u32,
    pub max: u32,
}

impl Health {
    /// Build a fresh, full-health bar of `max` HP.
    pub fn full(max: u32) -> Self {
        Self { current: max, max }
    }
}

/// Default health for a freshly spawned nanobot. Shared across
/// the three early types (Worker, Hauler, Defender) per the
/// project's "shared cost/time" decision; differentiated
/// health is a follow-up issue.
pub const NANOBOT_DEFAULT_MAX_HEALTH: u32 = 100;

impl Default for Health {
    fn default() -> Self {
        Self::full(NANOBOT_DEFAULT_MAX_HEALTH)
    }
}
