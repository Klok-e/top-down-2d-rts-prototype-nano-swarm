use bevy::{
    ecs::{lifecycle::HookContext, world::DeferredWorld},
    prelude::*,
};

use super::{Nanobot, PlannedStructure, Structure};

/// Session-local identity assigned when a combat participant enters the simulation.
#[derive(Component, Debug, Default)]
#[component(on_add = assign_combat_identity)]
pub struct CombatIdentity(u64);

#[derive(Resource, Default)]
pub(super) struct NextCombatIdentity(u64);

fn assign_combat_identity(mut world: DeferredWorld, context: HookContext) {
    let mut next = world.resource_mut::<NextCombatIdentity>();
    let identity = next.0;
    next.0 += 1;
    *world
        .get_mut::<CombatIdentity>(context.entity)
        .expect("identity was just added") = CombatIdentity(identity);
}

pub(crate) fn combat_key(entity: Entity, identity: Option<&CombatIdentity>) -> u64 {
    identity.map_or_else(|| entity.to_bits(), |identity| identity.0)
}

pub struct CombatIdentityPlugin;

impl Plugin for CombatIdentityPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<NextCombatIdentity>()
            .register_required_components::<Nanobot, CombatIdentity>()
            .register_required_components::<PlannedStructure, CombatIdentity>()
            .register_required_components::<Structure, CombatIdentity>();
    }
}
