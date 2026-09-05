//! Safe evacuation between construction work and operational activation.
use bevy::prelude::*;

/// A finished plan bars entrants while its occupants leave.
#[derive(Component, Debug, Clone, Copy)]
pub struct StructureClearing {
    pub builder_position: Vec2,
    pub validated_layout: Option<u64>,
}

/// Movement temporarily prioritizes leaving a completing footprint.
#[derive(Component, Debug, Clone, Copy)]
pub struct ClearingEvacuation {
    pub structure: Entity,
    pub goal: Vec2,
}

use super::{
    DirectMovementComponent, Nanobot, OwnerSwarm, PlannedProductionTarget, PlannedStructure,
    SwarmId,
    construction_access::{CancelledSites, ConstructionAccess},
};
use crate::{
    intent::IntentGrid,
    navigation::{CELL_WIDTH, Navigation, Obstacle},
    structure_sprites::{StructureSprites, StructureVisualState},
};

/// Visual residue has no structure, reservation, or collision identity.
#[derive(Component)]
pub struct CancelledPlanVisual {
    pub elapsed: u32,
    pub original_scale: Vec3,
}

#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub fn clear_finished_structures_system(
    mut commands: Commands,
    mut access: ParamSet<(ConstructionAccess, ResMut<CancelledSites>)>,
    grid: Res<IntentGrid>,
    navigation: Res<Navigation>,
    sprites: Res<StructureSprites>,
    mut plans: Query<(
        Entity,
        &PlannedStructure,
        &Transform,
        &mut StructureClearing,
        Option<&OwnerSwarm>,
        Option<&PlannedProductionTarget>,
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
    for (bot, _, evacuation, _) in &occupants {
        if evacuation.is_some_and(|evacuation| plans.get(evacuation.structure).is_err()) {
            commands
                .entity(bot)
                .remove::<ClearingEvacuation>()
                .remove::<DirectMovementComponent>();
        }
    }
    let layout = access.p0().snapshot();
    let layout_key = layout.validation_key(&grid);
    for (entity, plan, transform, mut clearing, owner, target) in &mut plans {
        let swarm = owner
            .and_then(|owner| swarms.get(owner.0).ok())
            .copied()
            .unwrap_or(SwarmId::PLAYER);
        let shape = Obstacle::structure(transform);
        let occupied_now = occupants
            .iter()
            .any(|(_, position, _, _)| !shape.admits_body(position.translation.truncate()));
        let rejected = if clearing.validated_layout != Some(layout_key) || !occupied_now {
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
            let mut sprite = sprites.sprite(plan.kind, StructureVisualState::Planned);
            sprite.color = Color::srgba(1.0, 0.05, 0.05, 1.0);
            sprite.custom_size = Some(Vec2::splat(crate::navigation::STRUCTURE_SPRITE_SIZE));
            commands.spawn((
                sprite,
                *transform,
                CancelledPlanVisual {
                    elapsed: 0,
                    original_scale: transform.scale,
                },
            ));
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
        clearing.validated_layout = Some(layout_key);
        let mut occupied = false;
        for (bot, position, evacuation, movement) in &occupants {
            let position = position.translation.truncate();
            if shape.admits_body(position) {
                if evacuation.is_some_and(|evacuation| evacuation.structure == entity) {
                    commands
                        .entity(bot)
                        .remove::<ClearingEvacuation>()
                        .remove::<DirectMovementComponent>();
                }
                continue;
            }
            occupied = true;
            if let Some(evacuation) = evacuation.filter(|evacuation| {
                evacuation.structure == entity
                    && navigation.point_clear(evacuation.goal)
                    && navigation.segment_clear(position, evacuation.goal)
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
                        && navigation.point_clear(goal)
                        && navigation.segment_clear(position, goal)
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
            super::planned::promote_planned_to_completion(
                &mut commands,
                entity,
                plan.kind,
                *transform,
                plan.cell,
                target.map(|target| target.0),
                &sprites,
            );
            commands.entity(entity).remove::<StructureClearing>();
        }
    }
}

pub fn animate_cancelled_plans_system(
    mut commands: Commands,
    mut effects: Query<(
        Entity,
        &mut CancelledPlanVisual,
        &mut Transform,
        &mut Sprite,
    )>,
) {
    for (entity, mut effect, mut transform, mut sprite) in &mut effects {
        effect.elapsed += 1;
        if effect.elapsed >= 30 {
            commands.entity(entity).despawn();
            continue;
        }
        let fade = ((effect.elapsed.saturating_sub(6)) as f32 / 24.0).clamp(0.0, 1.0);
        transform.scale = effect.original_scale * (1.0 - fade);
        sprite.color = Color::srgba(1.0, 0.05, 0.05, 1.0 - fade);
    }
}
