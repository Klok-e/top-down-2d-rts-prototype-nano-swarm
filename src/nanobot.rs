mod route_following;
mod work_navigation;
pub use work_navigation::*;
pub mod allocation;
mod autonomy;
mod build;
mod cargo;
mod charge;
mod combat;
mod combat_identity;
mod combat_presentation;
mod components;
pub mod construction_access;
mod consts;
mod debug;
mod elimination;
mod engagement;
mod gather;
mod gather_quota;
mod haul;
mod haul_quota;
mod interaction;
mod logistics_leg;
mod maintenance;
mod motion_presentation;
mod move_system;
mod opponent;
mod placement;
mod planned;
mod population;
mod production;
mod retreat;
mod spread;
mod sprites;
mod work_access;

pub use crate::strategic_runtime::{StrategicController, StrategicControllerPlugin};
pub use allocation::*;
pub use autonomy::*;
pub use build::*;
pub use cargo::*;
pub use charge::*;
pub use combat::*;
pub(crate) use combat_identity::combat_key;
pub use combat_identity::{CombatIdentity, CombatIdentityPlugin};
pub use combat_presentation::*;
pub use components::*;
pub use consts::*;
pub use debug::*;
pub use elimination::*;
pub use engagement::*;
pub use gather::*;
pub use gather_quota::*;
pub use haul::*;
pub use haul_quota::*;
pub use interaction::*;
pub use maintenance::*;
pub use motion_presentation::PresentedNanobot;
pub(crate) use motion_presentation::UpdateNanobotPresentation;
pub use move_system::*;
pub use opponent::*;
pub use placement::*;
pub use planned::*;
pub use population::*;
pub use production::*;
pub use retreat::*;
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
    tuning: Res<crate::tuning::Tuning>,
) {
    let Ok(kind) = types.get(added.entity) else {
        return;
    };
    if *kind == NanobotType::Defender {
        commands
            .entity(added.entity)
            .insert(Charge::with_tuning(&tuning));
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
        if !app.is_plugin_added::<CombatIdentityPlugin>() {
            app.add_plugins(CombatIdentityPlugin);
        }
        app.init_resource::<DefenderEngagements>()
            .add_observer(initialize_nanobot_type_components)
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
                    defender_engagement_system,
                    defender_retreat_system,
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
        run_fixed_schedules_on_one_thread(app);
    }
}

/// The fixed simulation runs its systems in order on the thread that runs the
/// fixed loop. Its systems are short, so handing each one to a worker thread costs
/// more than it saves, and woken workers often land on slow or idle cores. Single
/// threading made the fixed tick about three times faster at 55 Nanobots and
/// about a quarter faster at 5,000.
fn run_fixed_schedules_on_one_thread(app: &mut App) {
    use bevy::ecs::schedule::{ExecutorKind, ScheduleLabel};
    for label in [
        FixedFirst.intern(),
        FixedPreUpdate.intern(),
        FixedUpdate.intern(),
        FixedPostUpdate.intern(),
        FixedLast.intern(),
    ] {
        app.edit_schedule(label, |schedule| {
            schedule.set_executor_kind(ExecutorKind::SingleThreaded);
        });
    }
}

fn reset_resource<T: Resource + Default>(world: &mut World) {
    if let Some(mut resource) = world.get_resource_mut::<T>() {
        *resource = T::default();
    }
}

/// Clear simulation state retained outside session-owned entities.
pub(crate) fn reset_session(world: &mut World) {
    reset_resource::<combat_identity::NextCombatIdentity>(world);
    reset_resource::<DefenderEngagements>(world);
    reset_resource::<SoftWorkSlots>(world);
    reset_resource::<OpponentSwarmIdAlloc>(world);
    reset_resource::<PopulationDemand>(world);
    reset_resource::<GatherQuotas>(world);
    reset_resource::<HaulQuotas>(world);
    reset_resource::<ProductionPressure>(world);
    reset_resource::<construction_access::CancelledSites>(world);
    allocation::reset_session(world);
    reset_resource::<SwarmEliminationState>(world);
    reset_resource::<MatchOutcome>(world);
    combat_presentation::reset_session(world);
}

pub mod structure_lifecycle;
pub use structure_lifecycle::*;
