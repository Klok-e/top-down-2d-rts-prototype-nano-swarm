use bevy::prelude::{Component, Vec2};

#[derive(Debug, Component, Default)]
pub struct Nanobot {}

/// Marker for the swarm that owns a population of nanobots. Replaces the old
/// per-group entity that owned nanobots and their zones.
#[derive(Debug, Component, Default)]
pub struct Swarm {}

/// Stable identifier for one swarm. Carried on every
/// [`Swarm`] entity via the [`SwarmId`] component, and on every
/// nanobot via [`SwarmMember`].
///
/// The player swarm is reserved the constant
/// [`SwarmId::PLAYER`]. Opponent swarms are assigned fresh ids
/// when they are spawned so the player and each opponent can be
/// told apart by the intent scoring and the production chain.
///
/// The id is a plain `u32` because the only thing the rest of
/// the code does with it is compare and store; a richer handle
/// would just be ceremony around equality.
#[derive(Debug, Clone, Copy, Component, PartialEq, Eq, Hash, Default)]
pub struct SwarmId(pub u32);

impl SwarmId {
    /// Identifier reserved for the player swarm. Every
    /// player-owned entity uses this id so a single
    /// `SwarmMember(SwarmId::PLAYER)` is enough to route
    /// player behaviour without per-spawn bookkeeping.
    pub const PLAYER: SwarmId = SwarmId(0);

    /// True when this id matches the player swarm.
    pub fn is_player(self) -> bool {
        self == Self::PLAYER
    }
}

/// Marks a nanobot as belonging to a specific [`SwarmId`]. The
/// autonomy scoring uses this to filter out intent layers owned
/// by other swarms; the production chain uses it to stamp the
/// right id on newly produced children.
///
/// The component sits on every nanobot. The default
/// [`SwarmId::PLAYER`] is the right value for every test seam
/// helper and every unowned scenario nanobot, so the bundle
/// default falls through to "player" without any explicit
/// assignment.
#[derive(Debug, Clone, Component, Copy, PartialEq, Eq, Default)]
pub struct SwarmMember(pub SwarmId);

impl SwarmMember {
    /// Build a member marker for `swarm`.
    pub const fn new(swarm: SwarmId) -> Self {
        Self(swarm)
    }
}

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
