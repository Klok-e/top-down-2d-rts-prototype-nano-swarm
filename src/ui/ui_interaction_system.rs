use bevy::{
    prelude::{Component, Query, Res, ResMut, Resource, With, Without},
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
    interaction_query: Query<&RelativeCursorPosition, (With<Node>, Without<NoPointerCapture>)>,
    editor: Option<Res<super::tuning_panel::TuningEditor>>,
) {
    let any = interaction_query.iter().any(|x| x.cursor_over())
        || editor.is_some_and(|editor| editor.pointer_captured() || editor.keyboard_captured());
    if ui_handling.is_pointer_over_ui != any {
        ui_handling.is_pointer_over_ui = any;
    }
}
