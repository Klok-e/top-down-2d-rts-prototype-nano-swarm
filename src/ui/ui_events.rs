use bevy::prelude::{Component, Entity, Event};

/// Event for communicating with UI
#[derive(Debug, Event)]
pub enum SelectedGroupsChanged {
    Selected(Entity),
    Deselected(Entity),
}

#[derive(Debug, Component, Event)]
pub enum NanobotGroupAction {
    Merge,
    Split,
}
