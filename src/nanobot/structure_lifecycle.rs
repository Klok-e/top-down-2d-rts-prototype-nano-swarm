//! Owns construction reservations, validation, evacuation, activation, and cancellation.
use bevy::prelude::*;

use super::{
    DirectMovementComponent, InteractionRegion, Nanobot, NanobotType, OwnerSwarm,
    PlannedProductionTarget, ProductionFacility, SwarmId, SwarmMember,
    construction_access::{CancelledSites, ConstructionAccess},
};
use crate::{
    intent::IntentGrid,
    navigation::{CELL_WIDTH, Navigation, Obstacle},
    structure_sprites::{StructureSprites, StructureVisualState},
};

use super::planned::{
    DEFAULT_PLANNED_WORK_TICKS, DEFAULT_STOCKPILE_CAPACITY, PLANNED_STRUCTURE_FOOTPRINT,
    PlannedKind, completed_visual_color, sink_stockpile_demand_system,
};
use crate::resources::{ResourceKind, Stockpile, StockpileRole};
use crate::structure_sprites::StructureVisual;

/// End-of-tick publication of construction transitions and their spatial consequences.
/// Movement, allocation, and production consume the previously committed lifecycle.
/// This phase applies deferred transitions and refreshes routing before subsequent
/// observers and the next fixed tick can consume newly operational capacity.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StructureLifecycleSet {
    Commit,
}

/// Spatial consequence of a structure's lifecycle, independent of its kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StructurePassage {
    Traversable,
    EntryBarred,
    Solid,
}

pub(crate) fn structure_passage(
    plan: Option<&PlannedStructure>,
    clearing: Option<&StructureClearing>,
) -> StructurePassage {
    if plan.is_none() {
        StructurePassage::Solid
    } else if clearing.is_some_and(StructureClearing::bars_entry) {
        StructurePassage::EntryBarred
    } else {
        StructurePassage::Traversable
    }
}

#[derive(Debug, Clone, Copy)]
enum ClearingValidation {
    AwaitingInitialValidation,
    Clearing { validated_layout: u64 },
}

/// Finished construction waiting for access approval or occupant evacuation.
#[derive(Component, Debug, Clone, Copy)]
pub struct StructureClearing {
    builder_position: Vec2,
    validation: ClearingValidation,
}

impl StructureClearing {
    /// Restore a finished site that has not yet passed its first access check.
    pub fn awaiting_validation(builder_position: Vec2) -> Self {
        Self {
            builder_position,
            validation: ClearingValidation::AwaitingInitialValidation,
        }
    }

    /// Restore an evacuation already approved against the supplied layout.
    pub fn validated(builder_position: Vec2, validated_layout: u64) -> Self {
        Self {
            builder_position,
            validation: ClearingValidation::Clearing { validated_layout },
        }
    }

    pub fn bars_entry(&self) -> bool {
        matches!(self.validation, ClearingValidation::Clearing { .. })
    }

    fn validated_layout(&self) -> Option<u64> {
        match self.validation {
            ClearingValidation::AwaitingInitialValidation => None,
            ClearingValidation::Clearing { validated_layout } => Some(validated_layout),
        }
    }
}

/// Movement temporarily prioritizes leaving a completing footprint.
#[derive(Component, Debug, Clone, Copy)]
pub struct ClearingEvacuation {
    pub structure: Entity,
    pub goal: Vec2,
}

#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub fn clear_finished_structures_system(
    mut commands: Commands,
    mut access: ParamSet<(ConstructionAccess, ResMut<CancelledSites>)>,
    grid: Res<IntentGrid>,
    navigation: Res<Navigation>,
    sprites: Res<StructureSprites>,
    mut world: ParamSet<(
        crate::physical_world::PhysicalWorld,
        Query<(
            Entity,
            &PlannedStructure,
            &Transform,
            &mut StructureClearing,
            Option<&OwnerSwarm>,
        )>,
    )>,
    swarms: Query<&SwarmId>,
    occupants: Query<
        (
            Entity,
            &Transform,
            Option<&ClearingEvacuation>,
            Option<&DirectMovementComponent>,
        ),
        With<Nanobot>,
    >,
) {
    let physical = world.p0().snapshot();
    let mut plans = world.p1();
    for (bot, position, evacuation, movement) in &occupants {
        if let Some(evacuation) = evacuation.filter(|evacuation| {
            plans
                .get(evacuation.structure)
                .map_or(true, |(_, _, transform, _, _)| {
                    Obstacle::structure(transform).admits_body(position.translation.truncate())
                })
        }) {
            commands.entity(bot).remove::<ClearingEvacuation>();
            if movement.is_some_and(|movement| {
                movement.interaction.is_none() && movement.xy == evacuation.goal
            }) {
                commands.entity(bot).remove::<DirectMovementComponent>();
            }
        }
    }
    let layout = access.p0().snapshot();
    let layout_key = layout.validation_key(&grid);
    for (entity, plan, transform, mut clearing, owner) in &mut plans {
        let swarm = owner
            .and_then(|owner| swarms.get(owner.0).ok())
            .copied()
            .unwrap_or(SwarmId::PLAYER);
        let shape = Obstacle::structure(transform);
        let occupied_now = occupants
            .iter()
            .any(|(_, position, _, _)| !shape.admits_body(position.translation.truncate()));
        let rejected = if clearing.validated_layout() != Some(layout_key) || !occupied_now {
            match layout.check(
                &navigation,
                &grid,
                swarm,
                transform,
                Some(entity),
                Some(clearing.builder_position),
            ) {
                crate::navigation::AccessStatus::Pending => continue,
                crate::navigation::AccessStatus::Accepted => false,
                crate::navigation::AccessStatus::Rejected => true,
            }
        } else {
            false
        };
        if rejected {
            access
                .p1()
                .record(&layout, entity, swarm, plan.kind, transform);
            commands.entity(entity).despawn();
            crate::structure_overlay::spawn_cancelled_plan_visual(
                &mut commands,
                &sprites,
                plan.kind,
                *transform,
            );
            for (bot, _, evacuation, _) in &occupants {
                if evacuation.is_some_and(|evacuation| evacuation.structure == entity) {
                    commands
                        .entity(bot)
                        .remove::<ClearingEvacuation>()
                        .remove::<DirectMovementComponent>();
                }
            }
            continue;
        }
        clearing.validation = ClearingValidation::Clearing {
            validated_layout: layout_key,
        };
        let mut occupied = false;
        for (bot, position, evacuation, movement) in &occupants {
            let position = position.translation.truncate();
            if shape.admits_body(position) {
                if let Some(evacuation) =
                    evacuation.filter(|evacuation| evacuation.structure == entity)
                {
                    commands.entity(bot).remove::<ClearingEvacuation>();
                    if movement.is_some_and(|movement| {
                        movement.interaction.is_none() && movement.xy == evacuation.goal
                    }) {
                        commands.entity(bot).remove::<DirectMovementComponent>();
                    }
                }
                continue;
            }
            occupied = true;
            let free_exit = |goal: Vec2| {
                occupants.iter().all(|(other, transform, _, _)| {
                    other == bot
                        || transform.translation.truncate().distance_squared(goal)
                            >= (2.0 * crate::navigation::BODY_RADIUS).powi(2)
                })
            };
            if let Some(evacuation) = evacuation.filter(|evacuation| {
                evacuation.structure == entity
                    && physical.can_occupy(evacuation.goal)
                    && free_exit(evacuation.goal)
                    && physical.movement_clear(position, evacuation.goal)
            }) {
                if movement.is_none() {
                    commands.entity(bot).insert(DirectMovementComponent {
                        xy: evacuation.goal,
                        stop_radius: 0.0,
                        interaction: None,
                        speed: None,
                    });
                }
                continue;
            }
            let Obstacle::Rectangle { center, half } = shape else {
                unreachable!()
            };
            let min = ((center - half) / CELL_WIDTH).floor().as_ivec2() - IVec2::ONE;
            let max = ((center + half) / CELL_WIDTH).ceil().as_ivec2();
            let mut candidates = Vec::new();
            for y in min.y..=max.y {
                for x in min.x..=max.x {
                    let goal = (IVec2::new(x, y).as_vec2() + Vec2::splat(0.5)) * CELL_WIDTH;
                    if shape.admits_body(goal)
                        && free_exit(goal)
                        && physical.can_occupy(goal)
                        && physical.movement_clear(position, goal)
                    {
                        candidates.push(goal);
                    }
                }
            }
            candidates.sort_by(|a, b| {
                a.distance_squared(position)
                    .total_cmp(&b.distance_squared(position))
            });
            if let Some(&goal) = candidates.first() {
                commands.entity(bot).insert((
                    ClearingEvacuation {
                        structure: entity,
                        goal,
                    },
                    DirectMovementComponent {
                        xy: goal,
                        stop_radius: 0.0,
                        interaction: None,
                        speed: None,
                    },
                ));
            }
        }
        if !occupied {
            promote_planned_to_completion(
                &mut commands,
                entity,
                plan.kind,
                *transform,
                plan.cell,
                &sprites,
            );
        }
    }
}

/// A construction commitment that remains present until safe activation or cancellation.
/// Its work budget and Worker reservation are owned by lifecycle transitions.
#[derive(Debug, Component, Clone, Copy)]
pub struct PlannedStructure {
    pub kind: PlannedKind,
    pub cell: IVec2,
    work_remaining: u32,
    active_worker: Option<Entity>,
}

impl PlannedStructure {
    /// Build a fresh planned structure of `kind` in `cell` with
    /// the default work budget and no active worker.
    pub fn new(kind: PlannedKind, cell: IVec2) -> Self {
        Self {
            kind,
            cell,
            work_remaining: DEFAULT_PLANNED_WORK_TICKS,
            active_worker: None,
        }
    }

    /// Construct an authored construction state with an explicit remaining work budget.
    pub fn with_work_remaining(mut self, ticks: u32) -> Self {
        self.work_remaining = ticks;
        if ticks == 0 {
            self.active_worker = None;
        }
        self
    }

    pub fn active_worker(&self) -> Option<Entity> {
        self.active_worker
    }
    pub fn available_work(&self) -> u32 {
        self.work_remaining
    }

    /// Reserve construction only while work remains and no different Worker owns it.
    pub fn try_claim(&mut self, worker: Entity) -> bool {
        if self.available_work() == 0 || self.active_worker.is_some_and(|active| active != worker) {
            return false;
        }
        self.active_worker = Some(worker);
        true
    }

    /// Reserve the site and install the Worker's approach state as one transition.
    pub(crate) fn assign_worker(
        &mut self,
        commands: &mut Commands,
        target: Entity,
        worker: Entity,
        worker_position: Vec2,
        transform: &Transform,
    ) -> bool {
        if !self.try_claim(worker) {
            return false;
        }
        commands.entity(worker).insert((
            PlannedStructureClaim {
                cell: self.cell,
                target,
            },
            InteractionRegion::structure(transform).movement_from(worker_position),
        ));
        true
    }

    /// Completed work and the authored baseline used by the progress display.
    pub fn construction_progress(&self) -> (u32, u32) {
        (
            DEFAULT_PLANNED_WORK_TICKS.saturating_sub(self.work_remaining),
            DEFAULT_PLANNED_WORK_TICKS,
        )
    }

    /// Work remains and no Worker currently holds the construction reservation.
    pub fn can_accept_worker(&self) -> bool {
        self.work_remaining > 0 && self.active_worker.is_none()
    }

    /// True when build progress has finished and the planned
    /// structure is ready to be promoted to the completed
    /// structure for its kind.
    pub fn is_complete(&self) -> bool {
        self.work_remaining == 0
    }
}

/// Release a plan reservation when its Worker no longer exists or no longer
/// carries lifecycle state for that plan. Reconciliation runs before
/// opportunity projection so the same allocation pass can offer the plan to
/// another Worker after death or lease revocation.
pub(crate) fn release_stale_planned_workers_system(
    mut planned: Query<(Entity, &mut PlannedStructure)>,
    workers: Query<
        (
            Option<&PlannedStructureClaim>,
            Option<&PlannedStructureProgress>,
        ),
        With<Nanobot>,
    >,
) {
    for (planned_entity, mut planned) in &mut planned {
        let active_worker_is_valid = planned.active_worker.is_some_and(|worker| {
            workers.get(worker).is_ok_and(|(claim, progress)| {
                claim.is_some_and(|claim| claim.target == planned_entity)
                    || progress.is_some_and(|progress| progress.target == planned_entity)
            })
        });
        if planned.active_worker.is_some() && !active_worker_is_valid {
            planned.active_worker = None;
        }
    }
}

/// Marker on a Worker that has claimed a planned structure.
/// `target` is the [`PlannedStructure`] entity. The arrive
/// system reads the same `target` from this component so the
/// work system does not need to look up the original
/// assignment.
#[derive(Debug, Component, Clone, Copy)]
pub struct PlannedStructureClaim {
    pub cell: IVec2,
    pub target: Entity,
}

/// Marker on a Worker that is at its claimed planned
/// structure and is consuming worker time to build it. The
/// work system decrements `work_remaining` on the planned
/// structure each tick the worker has this marker.
#[derive(Debug, Component, Clone, Copy)]
pub struct PlannedStructureProgress {
    pub cell: IVec2,
    pub target: Entity,
}

/// Assign idle Workers to unfinished, unreserved same-owner plans.
/// Deferred plan writes are tracked locally so each site receives at most one Worker.
#[allow(clippy::type_complexity)]
pub fn worker_planned_structure_claim_system(
    mut commands: Commands,
    planned_structures: Query<(Entity, &Transform, &PlannedStructure, Option<&OwnerSwarm>)>,
    workers: Query<
        (Entity, &Transform, &NanobotType, &SwarmMember),
        (
            With<Nanobot>,
            Without<PlannedStructureClaim>,
            Without<PlannedStructureProgress>,
            Without<DirectMovementComponent>,
        ),
    >,
    swarms: Query<&SwarmId>,
) {
    let mut claimed: std::collections::HashSet<Entity> = std::collections::HashSet::new();
    for (worker_entity, worker_transform, nanobot_type, swarm_member) in &workers {
        // The Planned Structure lifecycle is a Worker job:
        // only Workers carry material to a build site and
        // spend worker time on construction. Defenders and
        // Haulers are filtered out so a Defend cell's
        // defenders do not accidentally claim a planned
        // Charger and a busy Hauler does not get pulled
        // off its run to build a structure.
        if *nanobot_type != NanobotType::Worker {
            continue;
        }
        let worker_pos = worker_transform.translation.truncate();

        let mut best: Option<(f32, Entity, &PlannedStructure, Vec2)> = None;
        for (planned_entity, planned_transform, planned, owner) in &planned_structures {
            if !planned.can_accept_worker() {
                continue;
            }
            if claimed.contains(&planned_entity) {
                continue;
            }
            if !planned_owner_matches_worker(owner, &swarms, swarm_member.0) {
                continue;
            }
            let distance = worker_pos.distance(planned_transform.translation.truncate());
            if best.is_none_or(|(bd, _, _, _)| distance < bd) {
                best = Some((
                    distance,
                    planned_entity,
                    planned,
                    planned_transform.translation.truncate(),
                ));
            }
        }
        let Some((_distance, planned_entity, planned, _planned_pos)) = best else {
            continue;
        };
        let Ok((_, target_transform, _, _)) = planned_structures.get(planned_entity) else {
            continue;
        };
        claimed.insert(planned_entity);

        let mut claimed_plan = *planned;
        if claimed_plan.assign_worker(
            &mut commands,
            planned_entity,
            worker_entity,
            worker_pos,
            target_transform,
        ) {
            commands.entity(planned_entity).insert(claimed_plan);
        }
    }
}

fn planned_owner_matches_worker(
    owner: Option<&OwnerSwarm>,
    swarms: &Query<&SwarmId>,
    worker_swarm: SwarmId,
) -> bool {
    match owner {
        None => true,
        Some(OwnerSwarm(owner_entity)) => swarms
            .get(*owner_entity)
            .is_ok_and(|owner_id| *owner_id == worker_swarm),
    }
}

/// Detect a worker that has arrived at its claimed planned
/// structure and start the work phase. The
/// `Without<PlannedStructureProgress>` filter makes arrival
/// idempotent: the same tick cannot fire twice.
///
/// Arrival requires the same exterior region used for movement and ongoing work.
/// Displaced workers reapproach while retaining their claim.
#[allow(clippy::type_complexity)]
pub fn worker_planned_structure_arrive_system(
    mut commands: Commands,
    workers: Query<
        (Entity, &Transform, &PlannedStructureClaim),
        (
            Without<super::WorkBlocked>,
            With<Nanobot>,
            With<PlannedStructureClaim>,
            Without<DirectMovementComponent>,
            Without<PlannedStructureProgress>,
        ),
    >,
    planned_transforms: Query<&Transform, With<PlannedStructure>>,
) {
    for (worker_entity, worker_transform, claim) in &workers {
        let Ok(planned_transform) = planned_transforms.get(claim.target) else {
            // Target disappeared (e.g. promoted by another
            // worker, or removed by a future cleanup system).
            // Drop the claim; the worker idles.
            commands
                .entity(worker_entity)
                .remove::<PlannedStructureClaim>();
            continue;
        };
        let region = InteractionRegion::structure(planned_transform);
        let position = worker_transform.translation.truncate();
        if region.contains(position) {
            commands
                .entity(worker_entity)
                .insert(PlannedStructureProgress {
                    cell: claim.cell,
                    target: claim.target,
                });
        } else {
            commands
                .entity(worker_entity)
                .insert(region.movement_from(position));
        }
    }
}

/// Apply construction work, then release its Worker while the site clears.
#[allow(clippy::type_complexity)]
pub fn worker_planned_structure_work_system(
    mut commands: Commands,
    workers: Query<
        (Entity, &Transform, &PlannedStructureProgress),
        (With<Nanobot>, Without<super::WorkBlocked>),
    >,
    mut planned: Query<(&mut PlannedStructure, &Transform)>,
) {
    for (worker, transform, progress) in &workers {
        let Ok((mut plan, site)) = planned.get_mut(progress.target) else {
            release_planned_worker(&mut commands, worker);
            continue;
        };
        let position = transform.translation.truncate();
        let region = InteractionRegion::structure(site);
        if !region.contains(position) {
            commands
                .entity(worker)
                .insert(region.movement_from(position));
            continue;
        }
        plan.work_remaining = plan.work_remaining.saturating_sub(1);
        if plan.is_complete() {
            plan.active_worker = None;
            commands
                .entity(progress.target)
                .insert(StructureClearing::awaiting_validation(position));
            release_planned_worker(&mut commands, worker);
        }
    }
}

fn release_planned_worker(commands: &mut Commands, worker: Entity) {
    commands
        .entity(worker)
        .remove::<PlannedStructureClaim>()
        .remove::<PlannedStructureProgress>()
        .remove::<crate::nanobot::RegionalLease>();
}

/// Replace transient construction state with the kind's empty operational payload.
/// Ownership and the aligned transform remain on the same entity.
fn promote_planned_to_completion(
    commands: &mut Commands,
    planned_entity: Entity,
    kind: PlannedKind,
    transform: Transform,
    cell: IVec2,
    structure_sprites: &StructureSprites,
) {
    commands
        .entity(planned_entity)
        .remove::<(PlannedStructure, StructureClearing, PlannedProductionTarget)>();
    let visual = completed_visual_bundle(kind, structure_sprites, transform);
    match kind {
        PlannedKind::SourceStockpile => {
            commands.entity(planned_entity).insert((
                empty_mineral_stockpile(),
                StockpileRole::Source,
                visual,
            ));
        }
        PlannedKind::SinkStockpile => {
            commands.entity(planned_entity).insert((
                empty_mineral_stockpile(),
                StockpileRole::Sink,
                visual,
            ));
        }
        PlannedKind::ProductionFacility => {
            // Completion creates an empty terminal. The normal production picker
            // chooses a type only after the hopper can pay the full cycle cost; a
            // planned target must never become a free first nanobot.
            let facility = ProductionFacility::new();
            // A completed facility is a terminal consumer:
            // it owns its own input hopper (on
            // `ProductionFacility`) and is NOT a `Stockpile`.
            // Haulers fill the hopper via logistics leg 3
            // (sink stockpile -> facility); production
            // consumes exclusively from it. Keeping the
            // `Stockpile` component off the facility means
            // it never enters stockpile queries, so a
            // gather worker cannot dump a gather load into
            // it and a hauler cannot pick it as a
            // stockpile source/sink.
            commands.entity(planned_entity).insert((facility, visual));
        }
        PlannedKind::Charger => {
            // Completed chargers begin empty. OwnerSwarm remains on the
            // entity through Bevy component-merge semantics.
            let charger = crate::nanobot::Charger::new(cell);
            commands.entity(planned_entity).insert((charger, visual));
        }
    }
}

/// The "build finished" visual shared by every completed
/// planned-structure kind. Bevy replaces the planned
/// `Sprite` on `insert`, so the planned visual does not
/// leak through to the completed entity.
fn completed_visual_bundle(
    kind: PlannedKind,
    structure_sprites: &StructureSprites,
    transform: Transform,
) -> (Sprite, Transform, StructureVisual) {
    let mut sprite = structure_sprites.sprite(kind, StructureVisualState::Completed);
    sprite.color = completed_visual_color();
    sprite.custom_size = Some(Vec2::splat(PLANNED_STRUCTURE_FOOTPRINT));
    (sprite, transform, StructureVisual::completed(kind))
}

/// Empty mineral buffer used by every completed planned kind
/// that needs a local `Stockpile`. Source and Sink Stockpiles
/// share capacity; their role marks logistics position, not
/// size.
fn empty_mineral_stockpile() -> Stockpile {
    Stockpile {
        kind: ResourceKind::Minerals,
        amount: 0,
        capacity: DEFAULT_STOCKPILE_CAPACITY,
        radius: 32.0,
    }
}

/// Commits lifecycle transitions after movement and allocation, then refreshes routing.
pub struct PlannedStructurePlugin;

impl Plugin for PlannedStructurePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<crate::nanobot::construction_access::CancelledSites>()
            .add_systems(
                FixedUpdate,
                crate::structure_overlay::animate_cancelled_plans_system,
            )
            .add_systems(
                FixedUpdate,
                release_stale_planned_workers_system
                    .before(crate::nanobot::RegionalAllocationSet::Project),
            )
            .add_systems(
                FixedUpdate,
                (
                    sink_stockpile_demand_system,
                    worker_planned_structure_arrive_system,
                    worker_planned_structure_work_system,
                    clear_finished_structures_system,
                    ApplyDeferred,
                    crate::navigation_runtime::refresh_navigation,
                )
                    .chain()
                    .in_set(StructureLifecycleSet::Commit)
                    .after(crate::nanobot::RegionalAllocationSet::Acquire)
                    .after(crate::nanobot::NanobotSimulationSet::Movement),
            );
    }
}
