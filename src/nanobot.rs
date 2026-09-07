mod route_following;
mod work_navigation;
pub use work_navigation::*;
pub mod allocation;
mod autonomy;
mod build;
mod cargo;
mod charge;
mod combat;
mod combat_presentation;
mod components;
pub mod construction_access;
mod consts;
mod debug;
mod elimination;
mod gather;
mod haul;
mod interaction;
mod logistics_leg;
mod maintenance;
mod move_system;
mod opponent;
mod placement;
mod planned;
mod population;
mod production;
mod spread;
mod sprites;
mod work_access;

pub use allocation::*;
pub use autonomy::*;
pub use build::*;
pub use cargo::*;
pub use charge::*;
pub use combat::*;
pub use combat_presentation::*;
pub use components::*;
pub use consts::*;
pub use debug::*;
pub use elimination::*;
pub use gather::*;
pub use haul::*;
pub use interaction::*;
pub use maintenance::*;
pub use move_system::*;
pub use opponent::*;
pub use placement::*;
pub use planned::*;
pub use population::*;
pub use production::*;
pub use spread::*;
pub use sprites::*;

use bevy::prelude::*;

use crate::ai::AiStateComponent;

pub use self::components::{Health, Nanobot, SwarmId, SwarmMember, VelocityComponent};

/// Bundle for a freshly spawned nanobot, defaulting to a Worker with zero
/// velocity and a fresh AI state. Spawners override fields to specialise it.
///
/// `swarm_member` defaults to [`SwarmId::PLAYER`]. Opponent spawners and
/// production set it to the owning swarm so each bot acts on its own intent.
#[derive(Debug, Bundle)]
pub struct NanobotBundle {
    pub nanobot: Nanobot,
    pub nanobot_type: NanobotType,
    pub velocity: VelocityComponent,
    pub ai_state: AiStateComponent,
    pub health: Health,
    pub swarm_member: SwarmMember,
}

impl Default for NanobotBundle {
    fn default() -> Self {
        Self {
            nanobot: Nanobot {},
            nanobot_type: NanobotType::Worker,
            velocity: VelocityComponent::default(),
            ai_state: AiStateComponent::new(),
            health: Health::default(),
            swarm_member: SwarmMember::new(SwarmId::PLAYER),
        }
    }
}

/// Attach type-specific lifecycle state whenever a nanobot type is introduced.
/// This is the single authority used by scenario, opponent, production, and tests.
pub fn initialize_nanobot_type_components(
    added: On<Add, NanobotType>,
    mut commands: Commands,
    types: Query<&NanobotType>,
) {
    let Ok(kind) = types.get(added.entity) else {
        return;
    };
    if *kind == NanobotType::Defender {
        commands.entity(added.entity).insert(Charge::default());
    } else {
        commands.entity(added.entity).remove::<Charge>();
    }
}

/// Ordered phases shared across simulation plugins.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SystemSet)]
pub enum NanobotSimulationSet {
    Movement,
    Threat,
    Combat,
    Maintenance,
}

/// Top-level bundle for the player swarm. Holds the [`Swarm`] marker and a
/// transform used as the spawn origin for its nanobots.
///
/// The `SwarmId` is the player identifier; the production chain
/// reads it to stamp `SwarmMember` on freshly produced
/// nanobots, so the player swarm must always carry it.
#[derive(Debug, Bundle, Default)]
pub struct SwarmBundle {
    pub swarm: Swarm,
    pub swarm_id: SwarmId,
    pub transform: Transform,
    pub global_transform: GlobalTransform,
    pub visibility: Visibility,
}

#[derive(Debug, Default)]
pub struct NanobotPlugin {}

impl Plugin for NanobotPlugin {
    fn build(&self, app: &mut App) {
        // Movement intent, local steering, and integration form one deterministic
        // fixed-tick pipeline. Presentation-only debug drawing remains frame-driven.
        if !app.is_plugin_added::<crate::navigation_runtime::NavigationPlugin>() {
            app.add_plugins(crate::navigation_runtime::NavigationPlugin);
        }
        app.add_observer(initialize_nanobot_type_components)
            .configure_sets(
                FixedUpdate,
                (
                    NanobotSimulationSet::Movement,
                    NanobotSimulationSet::Threat,
                    NanobotSimulationSet::Combat,
                    NanobotSimulationSet::Maintenance,
                )
                    .chain(),
            )
            .add_systems(
                FixedUpdate,
                (
                    move_velocity_system,
                    separation_system,
                    idle_spread_system,
                    velocity_system,
                    work_standing_system,
                )
                    .chain()
                    .in_set(NanobotSimulationSet::Movement),
            )
            // Death settlement closes each simulation tick so an entity at zero health
            // cannot act during another fixed tick in the same rendered frame.
            .add_systems(FixedLast, nanobot_death_cleanup_system)
            .add_systems(Update, bot_debug_circle_system);
    }
}

pub mod structure_lifecycle;
pub use structure_lifecycle::*;
