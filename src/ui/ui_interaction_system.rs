use bevy::{
    prelude::{Changed, Component, Query, ResMut, Resource, With, Without},
    ui::{Interaction, Node},
};

#[derive(Resource, Default)]
pub struct UiHandling {
    pub is_pointer_over_ui: bool,
}
#[derive(Component)]
pub struct NoPointerCapture;

#[allow(clippy::type_complexity)]
pub fn check_ui_interaction(
    mut ui_handling: ResMut<UiHandling>,
    interaction_query: Query<
        &Interaction,
        (With<Node>, Changed<Interaction>, Without<NoPointerCapture>),
    >,
) {
    ui_handling.is_pointer_over_ui = interaction_query
        .iter()
        .any(|i| matches!(i, Interaction::Clicked | Interaction::Hovered));
}
