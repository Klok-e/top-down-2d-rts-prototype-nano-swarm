//! Offscreen visual evidence for responsive Defender staging.
//!
//! The three captures show the same six idle Defenders clumped in the
//! middle of a three-cell Defend strip, redistributed to two per cell,
//! and then shifted again by continuous subcell roaming. ECS assertions
//! pin the balanced occupancy, absence of cross-cell movement commands,
//! and later within-cell displacement before each corresponding capture.
//!
//! Run: `cargo test --test screenshots -- --ignored defender_staging`
//! Artifacts land under `target/playtest-screenshots/defender_staging_*.png`.

use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    GAMEPLAY_SPRITE_Z, ZONE_BLOCK_SIZE,
    intent::{IntentGrid, IntentKind},
    nanobot::{
        Charge, Commitment, DirectMovementComponent, Health, Nanobot, NanobotType, SwarmId,
        SwarmMember, VelocityComponent, world_to_cell,
    },
};

use crate::harness::{TestContext, TestFlow};

const EVIDENCE_SWARM: SwarmId = SwarmId(99);
const STAGING_CELLS: [IVec2; 3] = [IVec2::new(-1, -5), IVec2::new(0, -5), IVec2::new(1, -5)];
const DEFENDER_COUNT: usize = 6;

#[derive(Debug, Component)]
struct StagingEvidenceDefender;

#[derive(Debug, Component)]
struct BalancedPosition(Vec2);

fn cell_center(cell: IVec2) -> Vec2 {
    Vec2::new(
        (cell.x as f32 + 0.5) * ZONE_BLOCK_SIZE,
        (cell.y as f32 + 0.5) * ZONE_BLOCK_SIZE,
    )
}

fn pin_camera(world: &mut World) {
    let target = cell_center(STAGING_CELLS[1]);
    for (mut transform, mut projection) in world
        .query_filtered::<(&mut Transform, &mut Projection), With<Camera2d>>()
        .iter_mut(world)
    {
        transform.translation.x = target.x;
        transform.translation.y = target.y;
        if let Projection::Orthographic(orthographic) = &mut *projection {
            orthographic.scale = 1.35;
        }
    }
}

fn setup_scene(world: &mut World) {
    {
        let mut grid = world.resource_mut::<IntentGrid>();
        for cell in STAGING_CELLS {
            grid.paint_owned(cell, IntentKind::Defend, Some(EVIDENCE_SWARM));
        }
    }
    let center = cell_center(STAGING_CELLS[1]);
    for offset in [
        Vec2::new(-36.0, -24.0),
        Vec2::new(-12.0, -24.0),
        Vec2::new(12.0, -24.0),
        Vec2::new(36.0, -24.0),
        Vec2::new(-12.0, 24.0),
        Vec2::new(12.0, 24.0),
    ] {
        world.spawn((
            StagingEvidenceDefender,
            Nanobot {},
            NanobotType::Defender,
            Commitment::Idle,
            VelocityComponent::default(),
            Health::default(),
            Charge::default(),
            SwarmMember::new(EVIDENCE_SWARM),
            Transform::from_translation((center + offset).extend(GAMEPLAY_SPRITE_Z)),
        ));
    }
}

fn assert_balanced_and_record(world: &mut World) {
    let mut counts = [0_usize; STAGING_CELLS.len()];
    let positions = world
        .query_filtered::<
            (Entity, &Transform, Option<&DirectMovementComponent>),
            With<StagingEvidenceDefender>,
        >()
        .iter(world)
        .map(|(entity, transform, movement)| {
            assert!(
                movement.is_none(),
                "balanced Defenders should have completed cross-cell travel"
            );
            let position = transform.translation.truncate();
            let cell = world_to_cell(position);
            let index = STAGING_CELLS
                .iter()
                .position(|candidate| *candidate == cell)
                .expect("every evidence Defender should occupy the Defend strip");
            counts[index] += 1;
            (entity, position)
        })
        .collect::<Vec<_>>();
    assert_eq!(counts, [2, 2, 2]);
    assert_eq!(positions.len(), DEFENDER_COUNT);
    for (entity, position) in positions {
        world.entity_mut(entity).insert(BalancedPosition(position));
    }
}

fn assert_roamed_within_staging_cells(world: &mut World) {
    let mut counts = [0_usize; STAGING_CELLS.len()];
    let mut visibly_shifted = 0;
    let mut defenders = 0;
    for (transform, balanced, movement) in world
        .query_filtered::<(
            &Transform,
            &BalancedPosition,
            Option<&DirectMovementComponent>,
        ), With<StagingEvidenceDefender>>()
        .iter(world)
    {
        assert!(movement.is_none());
        let position = transform.translation.truncate();
        let cell = world_to_cell(position);
        let index = STAGING_CELLS
            .iter()
            .position(|candidate| *candidate == cell)
            .expect("local roaming should remain inside the assigned Defend strip");
        counts[index] += 1;
        visibly_shifted += usize::from(position.distance(balanced.0) > 20.0);
        defenders += 1;
    }
    assert_eq!(counts, [2, 2, 2]);
    assert_eq!(defenders, DEFENDER_COUNT);
    assert!(
        visibly_shifted >= 4,
        "continuous local roaming should visibly shift most Defenders"
    );
}

pub fn defender_staging(ctx: &mut TestContext) -> TestFlow {
    pin_camera(ctx.world);
    match ctx.frame {
        0 => {
            setup_scene(ctx.world);
            TestFlow::Continue
        }
        1 => TestFlow::Continue,
        2 => TestFlow::Screenshot("defender_staging_clumped".to_string()),
        3..160 => TestFlow::Continue,
        160 => {
            assert_balanced_and_record(ctx.world);
            TestFlow::Screenshot("defender_staging_balanced".to_string())
        }
        161..320 => TestFlow::Continue,
        320 => {
            assert_roamed_within_staging_cells(ctx.world);
            TestFlow::Screenshot("defender_staging_roaming".to_string())
        }
        _ => TestFlow::Exit,
    }
}
