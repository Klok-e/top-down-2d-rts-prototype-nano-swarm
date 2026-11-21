use std::any::TypeId;

use bevy::{
    prelude::*,
    render::{Extract, sync_world::RenderEntity, view::RenderVisibleEntities},
};

use crate::MainCamera;

pub(super) type CombatCameras<'w, 's> =
    Extract<'w, 's, Query<'static, 'static, (Entity, &'static RenderEntity), With<MainCamera>>>;

// Call after camera extraction updates visibility. The camera stays valid even
// when the view contains no ordinary sprites.
pub(super) fn queue_camera_sprite_anchor(
    commands: &mut Commands,
    camera: Entity,
    render: &RenderEntity,
) {
    let render = render.id();
    commands.queue(move |world: &mut World| {
        if let Some(mut visible) = world.get_mut::<RenderVisibleEntities>(render) {
            let sprites = visible.entities.entry(TypeId::of::<Sprite>()).or_default();
            let anchor = (render, camera.into());
            if sprites.last() != Some(&anchor) {
                sprites.push(anchor);
            }
        }
    });
}
