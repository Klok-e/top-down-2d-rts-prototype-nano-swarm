//! Static-world snapshot shared by route consumers and final movement clearance.
use crate::{
    intent::IntentGrid,
    nanobot::{Charger, PlannedStructure, ProductionFacility, Structure},
    navigation::{Navigation, Obstacle},
    resources::{ResourceDeposit, Stockpile},
};
use bevy::prelude::*;

pub struct NavigationPlugin;
impl Plugin for NavigationPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Navigation>()
            .add_systems(FixedFirst, refresh_navigation);
    }
}

#[allow(clippy::type_complexity)]
pub fn refresh_navigation(
    mut navigation: ResMut<Navigation>,
    grid: Res<IntentGrid>,
    objects: Query<
        (
            Entity,
            &Transform,
            Option<&ResourceDeposit>,
            Option<&PlannedStructure>,
        ),
        Or<(
            With<ResourceDeposit>,
            With<Structure>,
            With<Stockpile>,
            With<ProductionFacility>,
            With<Charger>,
        )>,
    >,
) {
    let mut obstacles: Vec<_> = objects
        .iter()
        .filter_map(|(entity, transform, deposit, planned)| {
            if planned.is_some() {
                return None;
            }
            let shape = if let Some(deposit) = deposit {
                Obstacle::deposit(transform.translation.truncate(), deposit.radius)
            } else {
                Obstacle::structure(transform)
            };
            Some((entity, shape))
        })
        .collect();
    obstacles.sort_by_key(|(entity, _)| entity.to_bits());
    navigation.refresh(
        &grid,
        obstacles.into_iter().map(|(_, shape)| shape).collect(),
    );
}
