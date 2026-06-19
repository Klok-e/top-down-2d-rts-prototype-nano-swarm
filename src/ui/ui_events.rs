use bevy::prelude::{Component, Entity, Message};

/// Event for communicating with UI
#[derive(Debug, Message)]
pub enum SelectedGroupsChanged {
    Selected(Entity),
    Deselected(Entity),
}

#[derive(Debug, Component, Message)]
pub enum NanobotGroupAction {
    Merge,
    Split,
}
