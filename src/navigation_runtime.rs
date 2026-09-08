//! Static-world snapshot shared by route consumers and final movement clearance.
use crate::{intent::IntentGrid, navigation::Navigation, physical_world::PhysicalWorld};
use bevy::prelude::*;

pub struct NavigationPlugin;
impl Plugin for NavigationPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Navigation>()
            .init_resource::<NavigationBudget>()
            .add_systems(FixedFirst, (refresh_navigation, advance_navigation).chain());
    }
}

/// Clear route searches and derived geometry retained between fixed ticks.
pub(crate) fn reset_session(world: &mut World) {
    if let Some(mut navigation) = world.get_resource_mut::<Navigation>() {
        *navigation = Navigation::default();
    }
}

pub fn refresh_navigation(
    mut navigation: ResMut<Navigation>,
    grid: Res<IntentGrid>,
    physical: PhysicalWorld,
) {
    physical
        .snapshot()
        .refresh_navigation(&mut navigation, &grid);
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
