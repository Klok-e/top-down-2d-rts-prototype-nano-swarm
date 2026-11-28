//! Visual evidence for persistent Gather intent and Spare Gathering.

use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    GAMEPLAY_SPRITE_Z, ZONE_BLOCK_SIZE,
    fly_camera::CameraZoom2d,
    intent::{IntentGrid, IntentKind},
    nanobot::{
        Charger, Commitment, GatherAssignment, GatherQuotas, Health, LogisticsReservation, Nanobot,
        NanobotType, OpportunityCategory, OwnerSwarm, PlannedStructure, PopulationDemand,
        ProductionFacility, RegionalLease, SpareGathering, Swarm, SwarmId, SwarmMember,
        VelocityComponent,
    },
    resources::{ResourceDeposit, ResourceKind, Stockpile, StockpileRole},
    session::{OutcomeMode, SessionRules},
    structure_sprites::StructureSprites,
    tuning::Tuning,
};

use crate::harness::{TestContext, TestFlow, clear_nanobots_and_sprite_entities};

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
            .paint(CELL, IntentKind::Gather, TEST_SWARM);
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
        world.spawn((
            ExhaustionTestWorker,
            Nanobot {},
            NanobotType::Worker,
            Commitment::Idle,
            VelocityComponent::default(),
            Health::default(),
            SwarmMember::new(TEST_SWARM),
            Transform::from_translation(center.extend(0.0)),
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

    if ctx.frame > 61 && !world.contains_resource::<SpareGatherScene>() {
        return TestFlow::Exit;
    }
    spare_gathering(world, center, ctx.frame - 61)
}

#[derive(Resource)]
struct SpareGatherScene {
    source: Entity,
    captured_reservation: bool,
}

fn spare_gathering(world: &mut World, center: Vec2, frame: u32) -> TestFlow {
    if frame == 0 {
        world.resource_mut::<SessionRules>().outcomes = OutcomeMode::Disabled;
        clear_nanobots_and_sprite_entities(world);
        let structures = world
            .query_filtered::<Entity, Or<(
                With<Stockpile>,
                With<ProductionFacility>,
                With<Charger>,
                With<PlannedStructure>,
            )>>()
            .iter(world)
            .collect::<Vec<_>>();
        for structure in structures {
            world.despawn(structure);
        }
        let owner = world
            .query_filtered::<(Entity, &SwarmId), With<Swarm>>()
            .iter(world)
            .find_map(|(entity, id)| (*id == SwarmId::PLAYER).then_some(entity))
            .unwrap();
        let grid = world.resource::<IntentGrid>();
        let mut grid = IntentGrid::new(grid.width(), grid.height());
        grid.paint(CELL, IntentKind::Gather, SwarmId::PLAYER);
        world.insert_resource(grid);

        for (mut projection, mut zoom) in world
            .query::<(&mut Projection, &mut CameraZoom2d)>()
            .iter_mut(world)
        {
            zoom.zoom = 0.8;
            if let Projection::Orthographic(ortho) = &mut *projection {
                ortho.scale = 0.8;
            }
        }
        world.resource_mut::<Tuning>().logistics.material_work_ticks = 12;
        let sprites = world.resource::<StructureSprites>().clone();
        world.spawn((
            ResourceDeposit {
                kind: ResourceKind::Minerals,
                amount: 1000,
                capacity: 1000,
                radius: 48.0,
            },
            Transform::from_translation(center.extend(GAMEPLAY_SPRITE_Z)),
        ));
        let source = world
            .spawn((
                Stockpile {
                    kind: ResourceKind::Minerals,
                    amount: 0,
                    capacity: 60,
                    radius: 32.0,
                },
                StockpileRole::Source,
                OwnerSwarm(owner),
                Sprite::from_image(sprites.source_stockpile),
                Transform::from_translation((center + Vec2::X * 160.0).extend(GAMEPLAY_SPRITE_Z)),
            ))
            .id();
        world.spawn((
            Stockpile {
                kind: ResourceKind::Minerals,
                amount: 1000,
                capacity: 1100,
                radius: 32.0,
            },
            StockpileRole::Sink,
            OwnerSwarm(owner),
            Sprite::from_image(sprites.sink_stockpile),
            Transform::from_translation((center - Vec2::Y * 200.0).extend(GAMEPLAY_SPRITE_Z)),
        ));
        world.spawn((
            ProductionFacility::new(),
            OwnerSwarm(owner),
            Sprite::from_image(sprites.production_facility),
            Transform::from_translation(
                (center - Vec2::Y * 200.0 + Vec2::X * 170.0).extend(GAMEPLAY_SPRITE_Z),
            ),
        ));
        for offset in [-Vec2::X * 170.0, Vec2::Y * 90.0] {
            world.spawn((
                Nanobot {},
                NanobotType::Worker,
                Commitment::Idle,
                VelocityComponent::default(),
                Health::default(),
                SwarmMember::new(SwarmId::PLAYER),
                Transform::from_translation((center + offset).extend(GAMEPLAY_SPRITE_Z)),
            ));
        }
        world.insert_resource(SpareGatherScene {
            source,
            captured_reservation: false,
        });
        return TestFlow::Continue;
    }
    let scene = world.resource::<SpareGatherScene>();
    let source = scene.source;
    let captured_reservation = scene.captured_reservation;
    let carry = world.resource::<Tuning>().logistics.worker_carry_capacity;
    let stock = world.get::<Stockpile>(source).unwrap().amount;
    if stock > carry && captured_reservation {
        world.remove_resource::<SpareGatherScene>();
        return TestFlow::Screenshot("regional_allocation_spare_dropoffs".into());
    }
    assert!(
        frame < 240,
        "Spare Gathering did not add drop-offs: Source has {stock} minerals"
    );
    if !captured_reservation
        && world
            .query_filtered::<&LogisticsReservation, With<SpareGathering>>()
            .iter(world)
            .any(|claim| claim.destination == source && claim.destination_remaining == carry)
    {
        assert_eq!(
            world
                .resource::<GatherQuotas>()
                .economy(SwarmId::PLAYER)
                .unwrap()
                .gather_quota,
            1
        );
        assert_eq!(
            world
                .resource::<PopulationDemand>()
                .desired_for(SwarmId::PLAYER, NanobotType::Worker),
            1,
        );
        world
            .resource_mut::<SpareGatherScene>()
            .captured_reservation = true;
        return TestFlow::Screenshot("regional_allocation_spare_reservation".into());
    }
    TestFlow::Continue
}
