//! Fresh scenario sessions inside the persistent application shell.

use bevy::{ecs::system::SystemState, prelude::*};

use crate::{
    MainCamera,
    fly_camera::{CameraZoom2d, FlyCamera2d, set_camera_view},
    intent::{BrushSelection, IntentGrid},
    nanobot::{
        Nanobot, OpponentSwarmIdAlloc, PlannedStructure, ProductionFacility, Structure, Swarm,
    },
    resources::{ResourceDeposit, ResourceLedger, Stockpile},
    scenario_selection::{Scenario, ScenarioSelection},
    session::SessionRules,
    ui::scenario_menu::{MenuInputSet, ScenarioMenu},
};

#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct StartScenario(pub Scenario);

/// Changes whenever a fresh session replaces the simulation state.
#[derive(Resource, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionGeneration(pub u64);

/// Entities whose lifetime belongs to the current match, including their children.
#[derive(Component, Default)]
pub struct SessionEntity;

pub struct SessionLifecyclePlugin;
impl Plugin for SessionLifecyclePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SessionGeneration>()
            .add_message::<StartScenario>()
            .register_required_components::<Nanobot, SessionEntity>()
            .register_required_components::<Swarm, SessionEntity>()
            .register_required_components::<PlannedStructure, SessionEntity>()
            .register_required_components::<ProductionFacility, SessionEntity>()
            .register_required_components::<Structure, SessionEntity>()
            .register_required_components::<Stockpile, SessionEntity>()
            .register_required_components::<crate::nanobot::Charger, SessionEntity>()
            .register_required_components::<crate::nanobot::OwnerSwarm, SessionEntity>()
            .register_required_components::<ResourceDeposit, SessionEntity>()
            .register_required_components::<crate::terrain::RockFormation, SessionEntity>()
            .register_required_components::<crate::terrain_presentation::RockSurfaceRoot, SessionEntity>()
            .register_required_components::<crate::structure_overlay::StructureOverlay, SessionEntity>()
            .register_required_components::<crate::structure_overlay::ConditionOverlay, SessionEntity>()
            .register_required_components::<crate::structure_overlay::CancelledPlanVisual, SessionEntity>()
            .register_required_components::<crate::tactical_overlay::TacticalMarker, SessionEntity>()
            .add_systems(PostStartup, start_initial_session)
            .add_systems(PreUpdate, process_start_requests.after(MenuInputSet::Actions));
    }
}

fn start_initial_session(world: &mut World) {
    let scenario = world.resource::<ScenarioSelection>().current;
    populate_session(world, scenario);
}

fn process_start_requests(world: &mut World) {
    let request = world
        .resource_mut::<Messages<StartScenario>>()
        .drain()
        .last();
    let Some(StartScenario(scenario)) = request else {
        return;
    };
    world.flush();
    if !crate::battle_statistics::finish_session(world) {
        world.resource_mut::<ScenarioSelection>().error =
            Some("Could not save battle results. Start the scenario again to retry.".into());
        return;
    }
    let entities: Vec<_> = world
        .query_filtered::<Entity, With<SessionEntity>>()
        .iter(world)
        .collect();
    for entity in entities {
        if let Ok(entity) = world.get_entity_mut(entity) {
            entity.despawn();
        }
    }
    world.flush();
    world.resource_mut::<IntentGrid>().reset_session();
    world.insert_resource(ResourceLedger::default());
    world.insert_resource(OpponentSwarmIdAlloc::default());
    world.insert_resource(BrushSelection::default());
    crate::nanobot::reset_session(world);
    crate::navigation_runtime::reset_session(world);
    world.resource_mut::<SessionGeneration>().0 += 1;

    // The real clock and agent transport clock remain process-scoped.
    let timestep = world.resource::<Time<Fixed>>().timestep();
    world.insert_resource(Time::<Fixed>::from_duration(timestep));
    let max_delta = world.resource::<Time<Virtual>>().max_delta();
    world.insert_resource(Time::<Virtual>::from_max_delta(max_delta));
    world.insert_resource(Time::<()>::default());
    if let Some(mut keys) = world.get_resource_mut::<ButtonInput<KeyCode>>() {
        keys.reset_all();
    }
    if let Some(mut buttons) = world.get_resource_mut::<ButtonInput<MouseButton>>() {
        buttons.reset_all();
    }
    if let Some(mut wheels) = world.get_resource_mut::<Messages<bevy::input::mouse::MouseWheel>>() {
        wheels.clear();
    }
    for (mut transform, mut projection, mut zoom, mut movement) in world
        .query_filtered::<(
            &mut Transform,
            &mut Projection,
            &mut CameraZoom2d,
            &mut FlyCamera2d,
        ), With<MainCamera>>()
        .iter_mut(world)
    {
        set_camera_view(
            &mut transform,
            &mut projection,
            &mut zoom,
            &mut movement,
            crate::scenario::cell_origin(crate::scenario::PLAYER_CELL),
            Some(crate::DEFAULT_CAMERA_ZOOM),
        )
        .expect("main camera uses an orthographic projection");
    }
    populate_session(world, scenario);
    crate::ui::intent_layer_panel::reset_for_session(world);
    if let Some(mut menu) = world.get_resource_mut::<ScenarioMenu>() {
        menu.finish_scenario_start();
    }
}

fn populate_session(world: &mut World, scenario: Scenario) {
    {
        let mut selection = world.resource_mut::<ScenarioSelection>();
        selection.current = scenario;
        selection.error = None;
    }
    let experiment = world
        .resource::<crate::battle_experiment::BattleExperimentConfig>()
        .clone();
    world.insert_resource::<SessionRules>(crate::scenario::session_rules(scenario, &experiment));
    world.insert_resource(crate::scenario::gameplay_pacing(scenario, &experiment));
    world.insert_resource(crate::strategic_runtime::ControllerTelemetry::default());
    let mut state = SystemState::<(
        Commands,
        Res<AssetServer>,
        ResMut<IntentGrid>,
        ResMut<OpponentSwarmIdAlloc>,
    )>::new(world);
    let (mut commands, assets, mut grid, ids) = state.get_mut(world);
    crate::scenario::spawn_selected_scenario(
        scenario,
        &mut commands,
        &assets,
        &mut grid,
        ids,
        experiment.layout,
    );
    state.apply(world);
    world
        .run_system_cached(crate::scenario::configure_controllers)
        .expect("scenario controllers initialize");
    world.flush();
    crate::battle_statistics::start_session(world);
}
