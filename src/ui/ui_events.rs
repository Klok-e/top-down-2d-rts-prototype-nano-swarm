use bevy::prelude::{Component, Entity};

/// Event for communicating with UI
#[derive(Debug)]
pub enum SelectedGroupsChanged {
    Selected(Entity),
    Deselected(Entity),
}

#[derive(Debug, Component)]
pub enum NanobotGroupAction {
    Merge,
    Split,
}
