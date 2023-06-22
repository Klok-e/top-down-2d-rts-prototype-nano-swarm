use bevy::prelude::{Component, Vec2};

#[derive(Debug, Component, Default, Clone, Copy)]
pub struct NanobotGroup {
    pub id: u16,
}

#[derive(Debug, Component, Default)]
pub struct Nanobot {}

#[derive(Debug, Component)]
pub struct MoveDestination {
    pub xy: Vec2,
}

#[derive(Debug, Component, Clone, Copy, Default)]
pub struct Velocity {
    pub value: Vec2,
}

#[derive(Debug, Component)]
pub struct ProgressChecker {
    pub last_position: Vec2,
    pub last_update_time: f64,
}
