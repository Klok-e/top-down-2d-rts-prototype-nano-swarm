use bevy::{
    prelude::{Component, Query, Res, ResMut, Resource, With, Without},
    ui::{Node, RelativeCursorPosition},
};

use super::production_priority_panel::ProductionPriorityDragState;

#[derive(Resource, Default, Debug)]
pub struct UiHandling {
    pub is_pointer_over_ui: bool,
}
#[derive(Component)]
pub struct NoPointerCapture;

#[allow(clippy::type_complexity)]
pub fn check_ui_interaction(
    mut ui_handling: ResMut<UiHandling>,
    drag: Option<Res<ProductionPriorityDragState>>,
    interaction_query: Query<&RelativeCursorPosition, (With<Node>, Without<NoPointerCapture>)>,
) {
    let any = drag.is_some_and(|drag| drag.is_active())
        || interaction_query.iter().any(|x| x.cursor_over());
    if ui_handling.is_pointer_over_ui != any {
        ui_handling.is_pointer_over_ui = any;
    }
}
