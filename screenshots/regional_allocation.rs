//! Visual evidence for exhausted persistent Gather intent under regional allocation.

use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    ZONE_BLOCK_SIZE,
    intent::{IntentGrid, IntentKind},
    nanobot::{
        Commitment, GatherAssignment, Health, Nanobot, NanobotSprites, NanobotType,
        OpportunityCategory, RegionalLease, SwarmId, SwarmMember, VelocityComponent,
    },
    resources::{ResourceDeposit, ResourceKind, Stockpile},
};

use crate::harness::{TestContext, TestFlow};

#[derive(Component)]
struct ExhaustionTestWorker;

const CELL: IVec2 = IVec2::new(0, -5);
const TEST_SWARM: SwarmId = SwarmId(99);

fn cell_center(cell: IVec2) -> Vec2 {
    Vec2::new(
        (cell.x as f32 + 0.5) * ZONE_BLOCK_SIZE,
        (cell.y as f32 + 0.5) * ZONE_BLOCK_SIZE,
    )
}

pub fn regional_allocation(ctx: &mut TestContext) -> TestFlow {
    let world = &mut *ctx.world;
    let center = cell_center(CELL);
    for mut transform in world
        .query_filtered::<&mut Transform, With<Camera2d>>()
        .iter_mut(world)
    {
        transform.translation.x = center.x;
        transform.translation.y = center.y;
    }

    if ctx.frame == 0 {
        world
            .resource_mut::<IntentGrid>()
            .add_owned(CELL, IntentKind::Gather, Some(TEST_SWARM));
        world.spawn((
            ResourceDeposit {
                kind: ResourceKind::Minerals,
                amount: 1,
                capacity: 1,
                radius: 32.0,
            },
            Transform::from_translation(center.extend(0.0)),
        ));
        world.spawn((
            Stockpile {
                kind: ResourceKind::Minerals,
                amount: 0,
                capacity: 100,
                radius: 32.0,
            },
            Transform::from_translation((center + Vec2::new(96.0, 0.0)).extend(0.0)),
        ));
        let sprite = world
            .resource::<NanobotSprites>()
            .handle(NanobotType::Worker, false);
        world.spawn((
            ExhaustionTestWorker,
            Nanobot {},
            NanobotType::Worker,
            Commitment::Idle,
            VelocityComponent::default(),
            Health::default(),
            SwarmMember::new(TEST_SWARM),
            Transform::from_translation(center.extend(0.0)),
            Sprite::from_image(sprite),
        ));
        return TestFlow::Continue;
    }

    if ctx.frame < 60 {
        return TestFlow::Continue;
    }

    if ctx.frame == 60 {
        let grid = world.resource::<IntentGrid>();
        assert!(
            grid.cell(CELL)
                .is_some_and(|cell| cell.has(IntentKind::Gather))
        );
        for (assignment, lease) in world
            .query_filtered::<(Option<&GatherAssignment>, Option<&RegionalLease>), With<ExhaustionTestWorker>>()
            .iter(world)
        {
            assert!(assignment.is_none());
            assert!(lease.is_none_or(|lease| lease.category != OpportunityCategory::Gather));
        }
        return TestFlow::Screenshot("regional_allocation".to_string());
    }

    TestFlow::Exit
}
