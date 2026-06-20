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
