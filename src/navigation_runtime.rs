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
            .init_resource::<NavigationBudget>()
            .add_systems(FixedFirst, (refresh_navigation, advance_navigation).chain());
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
            Option<&crate::nanobot::StructureClearing>,
        ),
        Or<(
            With<ResourceDeposit>,
            With<Structure>,
            With<Stockpile>,
            With<ProductionFacility>,
            With<Charger>,
            With<crate::nanobot::StructureClearing>,
        )>,
    >,
) {
    let mut clearing: Vec<_> = objects
        .iter()
        .filter(|(_, _, _, _, clearing)| {
            clearing.is_some_and(|clearing| clearing.validated_layout.is_some())
        })
        .map(|(entity, transform, _, _, _)| (entity, Obstacle::structure(transform)))
        .collect();
    clearing.sort_by_key(|(entity, _)| entity.to_bits());
    navigation.refresh_clearing(clearing.into_iter().map(|(_, shape)| shape).collect());
    let mut obstacles: Vec<_> = objects
        .iter()
        .filter_map(|(entity, transform, deposit, planned, _)| {
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

/// Shared deterministic allowance for hierarchy maintenance and route searches.
#[derive(Resource)]
pub struct NavigationBudget(pub usize);
impl Default for NavigationBudget {
    fn default() -> Self {
        Self(32_768)
    }
}
fn advance_navigation(
    navigation: Res<Navigation>,
    grid: Res<IntentGrid>,
    budget: Res<NavigationBudget>,
) {
    navigation.advance(&grid, budget.0);
}
