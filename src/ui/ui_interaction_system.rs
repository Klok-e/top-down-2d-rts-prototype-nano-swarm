use bevy::{
    prelude::{Changed, Component, Query, ResMut, Resource, With, Without},
    ui::{Node, RelativeCursorPosition},
};

#[derive(Resource, Default, Debug)]
pub struct UiHandling {
    pub is_pointer_over_ui: bool,
}
#[derive(Component)]
pub struct NoPointerCapture;

#[allow(clippy::type_complexity)]
pub fn check_ui_interaction(
    mut ui_handling: ResMut<UiHandling>,
    interaction_query: Query<
        &RelativeCursorPosition,
        (
            With<Node>,
            Changed<RelativeCursorPosition>,
            Without<NoPointerCapture>,
        ),
    >,
) {
    let any = interaction_query.iter().any(|x| x.mouse_over());
    if ui_handling.is_pointer_over_ui != any {
        ui_handling.is_pointer_over_ui = any;
    }
}
