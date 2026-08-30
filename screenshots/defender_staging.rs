//! Offscreen evidence for responsive Defender staging and territory response.
//!
//! One authored cohort visibly roams a balanced layout, redistributes after a
//! paint edit, intercepts a Threat on owned non-Defend territory, retains the
//! same response through orthogonal and diagonal Pursuit Halo cells, and then
//! returns to a Defend layout painted while the response was active.
//!
//! Run: `cargo test --test screenshots -- --ignored defender_staging`
//! Artifacts land under `target/playtest-screenshots/defender_*.png`.

use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    GAMEPLAY_SPRITE_Z, ZONE_BLOCK_SIZE,
    fly_camera::{CameraZoom2d, FlyCamera2d, set_camera_view},
    intent::{IntentGrid, IntentKind},
    nanobot::{
        Charge, Commitment, DEFENDER_ATTACK_RANGE, DefenderResponse, DirectMovementComponent,
        Health, Nanobot, NanobotType, OpponentIntentController, OpponentSwarm, Swarm, SwarmId,
        SwarmMember, TerritorySnapshot, VelocityComponent, world_to_cell,
    },
};

use crate::harness::{TestContext, TestFlow, clear_nanobots_and_sprite_entities};

const EVIDENCE_SWARM: SwarmId = SwarmId(99);
const INITIAL_STAGING_CELLS: [IVec2; 2] = [IVec2::new(-1, 0), IVec2::new(0, 0)];
const ADDED_STAGING_CELL: IVec2 = IVec2::new(1, 0);
const REDISTRIBUTED_STAGING_CELLS: [IVec2; 3] = [
    INITIAL_STAGING_CELLS[0],
    INITIAL_STAGING_CELLS[1],
    ADDED_STAGING_CELL,
];
const THREAT_CELL: IVec2 = IVec2::new(1, 1);
const ORTHOGONAL_HALO_CELL: IVec2 = IVec2::new(2, 1);
const DIAGONAL_HALO_CELL: IVec2 = IVec2::new(2, 2);
const RETURN_STAGING_CELLS: [IVec2; 3] = [IVec2::new(-1, -1), IVec2::new(0, -1), IVec2::new(1, -1)];
const DEFENDER_COUNT: usize = 6;
const PHASE_UPDATE_LIMIT: u32 = 1_200;

#[derive(Debug, Component)]
struct StagingEvidenceDefender;

#[derive(Debug, Component)]
struct StagingEvidenceHostile;

#[derive(Debug, Clone, Copy)]
enum EvidencePhase {
    AwaitInitialBalance,
    AwaitInitialRoaming,
    ResumeInitialRoaming,
    AwaitRedistribution,
    ResumeRedistribution,
    AwaitRedistributionSettlement,
    AwaitInterception,
    ResumeInterception,
    AwaitOrthogonalHalo,
    ResumeOrthogonalHalo,
    AwaitDiagonalHalo,
    ResumeDiagonalHalo,
    AwaitCurrentLayoutReaction,
    AwaitReturn,
    ResumeReturn,
}

#[derive(Debug, Resource)]
struct StagingEvidence {
    defenders: [Entity; DEFENDER_COUNT],
    phase: EvidencePhase,
    phase_updates: u32,
    initial_positions: [Vec2; DEFENDER_COUNT],
    redistribution_movers: Option<[(Entity, Vec2); 2]>,
    hostile: Option<Entity>,
    responder: Option<Entity>,
    response_start: Vec2,
}

fn cell_center(cell: IVec2) -> Vec2 {
    Vec2::new(
        (cell.x as f32 + 0.5) * ZONE_BLOCK_SIZE,
        (cell.y as f32 + 0.5) * ZONE_BLOCK_SIZE,
    )
}

fn set_phase(evidence: &mut StagingEvidence, phase: EvidencePhase) {
    evidence.phase = phase;
    evidence.phase_updates = 0;
}

fn tick_phase(evidence: &mut StagingEvidence, description: &str) {
    evidence.phase_updates += 1;
    assert!(
        evidence.phase_updates < PHASE_UPDATE_LIMIT,
        "timed out while waiting for {description}",
    );
}

fn pin_camera(world: &mut World, focus: Vec2, zoom: f32) {
    let mut camera = world.query_filtered::<(
        &mut Transform,
        &mut Projection,
        &mut CameraZoom2d,
        &mut FlyCamera2d,
    ), With<Camera2d>>();
    for (mut transform, mut projection, mut camera_zoom, mut movement) in camera.iter_mut(world) {
        set_camera_view(
            &mut transform,
            &mut projection,
            &mut camera_zoom,
            &mut movement,
            focus,
            Some(zoom),
        )
        .expect("the offscreen camera should support an orthographic view");
    }
}

fn hide_ui(world: &mut World) {
    let entities = world
        .query_filtered::<Entity, With<Node>>()
        .iter(world)
        .collect::<Vec<_>>();
    for entity in entities {
        world.entity_mut(entity).insert(Visibility::Hidden);
    }
}

fn clear_intent(world: &mut World) {
    let layers = {
        let grid = world.resource::<IntentGrid>();
        grid.iter_active_cells()
            .flat_map(|(cell, intent)| {
                IntentKind::ALL
                    .into_iter()
                    .filter(move |kind| intent.has(*kind))
                    .map(move |kind| (cell, kind))
            })
            .collect::<Vec<_>>()
    };
    let mut grid = world.resource_mut::<IntentGrid>();
    for (cell, kind) in layers {
        assert!(grid.remove(cell, kind));
    }
}

fn spawn_defender(world: &mut World, position: Vec2) -> Entity {
    world
        .spawn((
            StagingEvidenceDefender,
            Nanobot {},
            NanobotType::Defender,
            Commitment::Idle,
            VelocityComponent::default(),
            Health::default(),
            Charge::default(),
            SwarmMember::new(EVIDENCE_SWARM),
            Transform::from_translation(position.extend(GAMEPLAY_SPRITE_Z)),
        ))
        .id()
}

fn setup_scene(world: &mut World) -> StagingEvidence {
    hide_ui(world);
    clear_nanobots_and_sprite_entities(world);
    clear_intent(world);

    let controllers = world
        .query_filtered::<Entity, With<OpponentIntentController>>()
        .iter(world)
        .collect::<Vec<_>>();
    for entity in controllers {
        world
            .entity_mut(entity)
            .remove::<OpponentIntentController>();
    }

    world.spawn((Swarm {}, EVIDENCE_SWARM));
    {
        let mut grid = world.resource_mut::<IntentGrid>();
        for cell in INITIAL_STAGING_CELLS {
            assert!(grid.paint_owned(cell, IntentKind::Defend, Some(EVIDENCE_SWARM)));
        }
        assert!(grid.paint_owned(THREAT_CELL, IntentKind::Gather, Some(EVIDENCE_SWARM)));
    }

    let left = cell_center(INITIAL_STAGING_CELLS[0]);
    let right = cell_center(INITIAL_STAGING_CELLS[1]);
    let defenders = [
        spawn_defender(world, left + Vec2::new(-112.0, -72.0)),
        spawn_defender(world, left + Vec2::new(0.0, 64.0)),
        spawn_defender(world, left + Vec2::new(112.0, -24.0)),
        spawn_defender(world, right + Vec2::new(-112.0, 72.0)),
        spawn_defender(world, right + Vec2::new(0.0, -64.0)),
        spawn_defender(world, right + Vec2::new(112.0, 24.0)),
    ];

    pin_camera(world, cell_center(IVec2::ZERO), 1.55);
    StagingEvidence {
        defenders,
        phase: EvidencePhase::AwaitInitialBalance,
        phase_updates: 0,
        initial_positions: [Vec2::ZERO; DEFENDER_COUNT],
        redistribution_movers: None,
        hostile: None,
        responder: None,
        response_start: Vec2::ZERO,
    }
}

fn defender_position(world: &World, entity: Entity) -> Vec2 {
    world
        .get::<Transform>(entity)
        .unwrap_or_else(|| panic!("evidence Defender {entity:?} should remain alive"))
        .translation
        .truncate()
}

fn defender_positions(
    world: &World,
    defenders: &[Entity; DEFENDER_COUNT],
) -> [Vec2; DEFENDER_COUNT] {
    std::array::from_fn(|index| defender_position(world, defenders[index]))
}

fn physical_counts<const N: usize>(
    world: &World,
    defenders: &[Entity; DEFENDER_COUNT],
    cells: &[IVec2; N],
) -> Option<[usize; N]> {
    let mut counts = [0; N];
    for entity in defenders {
        let cell = world_to_cell(defender_position(world, *entity));
        let index = cells.iter().position(|candidate| *candidate == cell)?;
        counts[index] += 1;
    }
    Some(counts)
}

fn assert_no_cross_cell_movement(world: &World, defenders: &[Entity; DEFENDER_COUNT]) {
    for entity in defenders {
        assert!(
            world.get::<DirectMovementComponent>(*entity).is_none(),
            "balanced Defender {entity:?} should have completed cross-cell travel",
        );
    }
}

fn assert_balanced<const N: usize>(
    world: &World,
    defenders: &[Entity; DEFENDER_COUNT],
    cells: &[IVec2; N],
    expected: [usize; N],
) {
    assert_eq!(physical_counts(world, defenders, cells), Some(expected));
    assert_no_cross_cell_movement(world, defenders);
}

fn visibly_moved_count(
    world: &World,
    defenders: &[Entity; DEFENDER_COUNT],
    baseline: &[Vec2; DEFENDER_COUNT],
    threshold: f32,
) -> usize {
    defenders
        .iter()
        .zip(baseline)
        .filter(|(entity, start)| defender_position(world, **entity).distance(**start) > threshold)
        .count()
}

fn assert_initial_roaming(world: &World, evidence: &StagingEvidence) {
    assert_balanced(world, &evidence.defenders, &INITIAL_STAGING_CELLS, [3, 3]);
    let moved = visibly_moved_count(
        world,
        &evidence.defenders,
        &evidence.initial_positions,
        20.0,
    );
    assert!(
        moved >= 4,
        "continuous staging should visibly move most Defenders; moved {moved}",
    );
}

fn assert_owned_intent(world: &World, cell: IVec2, kind: IntentKind, owner: SwarmId) {
    let intent = world
        .resource::<IntentGrid>()
        .cell(cell)
        .unwrap_or_else(|| panic!("evidence cell {cell:?} should be in bounds"));
    assert!(intent.has(kind));
    assert_eq!(intent.owner(kind), Some(owner));
}

fn assert_absent_intent(world: &World, cell: IVec2, kind: IntentKind) {
    let intent = world
        .resource::<IntentGrid>()
        .cell(cell)
        .unwrap_or_else(|| panic!("evidence cell {cell:?} should be in bounds"));
    assert!(!intent.has(kind));
}

fn moving_to_cell(world: &World, entity: Entity, cell: IVec2) -> bool {
    world
        .get::<DirectMovementComponent>(entity)
        .is_some_and(|movement| world_to_cell(movement.xy) == cell)
}

fn redistribution_movers(world: &World, defenders: &[Entity; DEFENDER_COUNT]) -> Vec<Entity> {
    let mut movers = defenders
        .iter()
        .copied()
        .filter(|entity| moving_to_cell(world, *entity, ADDED_STAGING_CELL))
        .collect::<Vec<_>>();
    movers.sort_by_key(|entity| entity.to_bits());
    movers
}

fn assert_redistribution_in_motion(world: &World, evidence: &StagingEvidence) {
    assert_owned_intent(
        world,
        ADDED_STAGING_CELL,
        IntentKind::Defend,
        EVIDENCE_SWARM,
    );
    let movers = evidence
        .redistribution_movers
        .expect("redistribution movers should have been recorded");
    for (entity, start) in movers {
        assert!(moving_to_cell(world, entity, ADDED_STAGING_CELL));
        let distance = defender_position(world, entity).distance(start);
        assert!(
            distance > 64.0,
            "redistributing Defender {entity:?} should be visibly displaced; moved {distance}",
        );
    }
    assert_eq!(redistribution_movers(world, &evidence.defenders).len(), 2);
}

fn opponent_swarm_id(world: &mut World) -> SwarmId {
    world
        .query_filtered::<&SwarmId, (With<Swarm>, With<OpponentSwarm>)>()
        .iter(world)
        .next()
        .copied()
        .expect("the default scenario should contain an opponent swarm")
}

fn spawn_hostile(world: &mut World) -> Entity {
    let opponent = opponent_swarm_id(world);
    world
        .spawn((
            StagingEvidenceHostile,
            Nanobot {},
            NanobotType::Worker,
            Commitment::Working,
            VelocityComponent::default(),
            Health::full(10_000),
            SwarmMember::new(opponent),
            Transform::from_translation(cell_center(THREAT_CELL).extend(GAMEPLAY_SPRITE_Z)),
        ))
        .id()
}

fn reset_hostile(world: &mut World, hostile: Entity, cell: IVec2) {
    let position = cell_center(cell);
    let mut entity = world.entity_mut(hostile);
    entity
        .get_mut::<Transform>()
        .expect("the evidence hostile should remain alive")
        .translation = position.extend(GAMEPLAY_SPRITE_Z);
    entity
        .get_mut::<VelocityComponent>()
        .expect("the evidence hostile should have velocity state")
        .value = Vec2::ZERO;
    *entity
        .get_mut::<Health>()
        .expect("the evidence hostile should have health") = Health::full(10_000);
}

fn responders_to(
    world: &World,
    defenders: &[Entity; DEFENDER_COUNT],
    hostile: Entity,
) -> Vec<Entity> {
    defenders
        .iter()
        .copied()
        .filter(|entity| {
            world
                .get::<DefenderResponse>(*entity)
                .is_some_and(|response| response.target == hostile)
        })
        .collect()
}

fn assert_response_tracks(
    world: &World,
    evidence: &StagingEvidence,
    expected_target_cell: IVec2,
    expected_threat_count: u32,
) {
    let hostile = evidence.hostile.expect("the Threat should remain present");
    let responder = evidence
        .responder
        .expect("one Defender should own the response");
    assert_eq!(
        world_to_cell(
            world
                .get::<Transform>(hostile)
                .expect("the Threat should have a transform")
                .translation
                .truncate(),
        ),
        expected_target_cell,
    );
    assert_eq!(
        responders_to(world, &evidence.defenders, hostile),
        [responder]
    );
    let target_position = cell_center(expected_target_cell);
    let movement = world
        .get::<DirectMovementComponent>(responder)
        .expect("the responding Defender should track its target");
    assert!(movement.xy.distance(target_position) <= 1e-4);
    assert!((movement.stop_radius - DEFENDER_ATTACK_RANGE).abs() <= 1e-4);

    let territory = world.resource::<TerritorySnapshot>();
    assert_eq!(
        territory.threat_count(EVIDENCE_SWARM),
        expected_threat_count
    );
    assert!(
        territory.pursuit_claim_is_spatially_valid(EVIDENCE_SWARM, expected_target_cell),
        "the response target should remain on territory or in its Pursuit Halo",
    );
}

fn assert_interception(world: &World, evidence: &StagingEvidence) {
    assert_owned_intent(world, THREAT_CELL, IntentKind::Gather, EVIDENCE_SWARM);
    assert_absent_intent(world, THREAT_CELL, IntentKind::Defend);
    let territory = world.resource::<TerritorySnapshot>();
    assert!(territory.is_swarm_tile(EVIDENCE_SWARM, THREAT_CELL));
    assert_response_tracks(world, evidence, THREAT_CELL, 1);
    let responder = evidence.responder.expect("the response should be recorded");
    assert_eq!(
        world_to_cell(defender_position(world, responder)),
        THREAT_CELL
    );
    assert!(defender_position(world, responder).distance(evidence.response_start) > 128.0);
}

fn assert_halo_response(world: &World, evidence: &StagingEvidence, halo_cell: IVec2) {
    assert_absent_intent(world, halo_cell, IntentKind::Gather);
    assert_absent_intent(world, halo_cell, IntentKind::Defend);
    assert_response_tracks(world, evidence, halo_cell, 0);
    let responder = evidence.responder.expect("the response should be recorded");
    assert_eq!(
        world_to_cell(defender_position(world, responder)),
        halo_cell
    );
}

fn replace_staging_paint(world: &mut World) {
    let mut grid = world.resource_mut::<IntentGrid>();
    for cell in REDISTRIBUTED_STAGING_CELLS {
        assert!(grid.remove(cell, IntentKind::Defend));
    }
    for cell in RETURN_STAGING_CELLS {
        assert!(grid.paint_owned(cell, IntentKind::Defend, Some(EVIDENCE_SWARM)));
    }
}

fn assert_current_staging_paint(world: &World) {
    for cell in REDISTRIBUTED_STAGING_CELLS {
        assert_absent_intent(world, cell, IntentKind::Defend);
    }
    for cell in RETURN_STAGING_CELLS {
        assert_owned_intent(world, cell, IntentKind::Defend, EVIDENCE_SWARM);
    }
}

fn assert_returned_to_current_staging(world: &World, evidence: &StagingEvidence) {
    assert_current_staging_paint(world);
    assert_balanced(world, &evidence.defenders, &RETURN_STAGING_CELLS, [2, 2, 2]);
    let responder = evidence
        .responder
        .expect("the returning Defender should remain recorded");
    assert!(world.get::<DefenderResponse>(responder).is_none());
    assert!(RETURN_STAGING_CELLS.contains(&world_to_cell(defender_position(world, responder))));
    for entity in evidence.defenders {
        assert!(
            !REDISTRIBUTED_STAGING_CELLS.contains(&world_to_cell(defender_position(world, entity)))
        );
    }
}

fn advance_scene(world: &mut World, evidence: &mut StagingEvidence) -> TestFlow {
    match evidence.phase {
        EvidencePhase::AwaitInitialBalance => {
            pin_camera(world, cell_center(IVec2::ZERO), 1.55);
            tick_phase(evidence, "an initial balanced staging layout");
            if evidence.phase_updates >= 30
                && physical_counts(world, &evidence.defenders, &INITIAL_STAGING_CELLS)
                    == Some([3, 3])
                && evidence
                    .defenders
                    .iter()
                    .all(|entity| world.get::<DirectMovementComponent>(*entity).is_none())
            {
                evidence.initial_positions = defender_positions(world, &evidence.defenders);
                set_phase(evidence, EvidencePhase::AwaitInitialRoaming);
            }
            TestFlow::Continue
        }
        EvidencePhase::AwaitInitialRoaming => {
            pin_camera(world, cell_center(IVec2::ZERO), 1.55);
            tick_phase(evidence, "continuous movement in balanced staging");
            assert_balanced(world, &evidence.defenders, &INITIAL_STAGING_CELLS, [3, 3]);
            if visibly_moved_count(
                world,
                &evidence.defenders,
                &evidence.initial_positions,
                20.0,
            ) >= 4
            {
                world.resource_mut::<Time<Virtual>>().pause();
                assert_initial_roaming(world, evidence);
                set_phase(evidence, EvidencePhase::ResumeInitialRoaming);
                return TestFlow::Screenshot(
                    "defender_staging_balanced_moving_before_paint_edit".to_string(),
                );
            }
            TestFlow::Continue
        }
        EvidencePhase::ResumeInitialRoaming => {
            assert!(world.resource::<Time<Virtual>>().is_paused());
            assert_initial_roaming(world, evidence);
            assert!(world.resource_mut::<IntentGrid>().paint_owned(
                ADDED_STAGING_CELL,
                IntentKind::Defend,
                Some(EVIDENCE_SWARM),
            ));
            world.resource_mut::<Time<Virtual>>().unpause();
            set_phase(evidence, EvidencePhase::AwaitRedistribution);
            TestFlow::Continue
        }
        EvidencePhase::AwaitRedistribution => {
            pin_camera(world, cell_center(IVec2::ZERO), 1.55);
            tick_phase(evidence, "visible redistribution after new Defend paint");
            assert_owned_intent(
                world,
                ADDED_STAGING_CELL,
                IntentKind::Defend,
                EVIDENCE_SWARM,
            );
            let movers = redistribution_movers(world, &evidence.defenders);
            if evidence.redistribution_movers.is_none() && movers.len() == 2 {
                evidence.redistribution_movers = Some([
                    (movers[0], defender_position(world, movers[0])),
                    (movers[1], defender_position(world, movers[1])),
                ]);
            }
            if let Some(recorded) = evidence.redistribution_movers {
                let visibly_displaced = recorded.iter().all(|(entity, start)| {
                    moving_to_cell(world, *entity, ADDED_STAGING_CELL)
                        && defender_position(world, *entity).distance(*start) > 64.0
                });
                if visibly_displaced {
                    world.resource_mut::<Time<Virtual>>().pause();
                    assert_redistribution_in_motion(world, evidence);
                    set_phase(evidence, EvidencePhase::ResumeRedistribution);
                    return TestFlow::Screenshot(
                        "defender_staging_redistributing_after_new_defend_paint".to_string(),
                    );
                }
            }
            TestFlow::Continue
        }
        EvidencePhase::ResumeRedistribution => {
            assert!(world.resource::<Time<Virtual>>().is_paused());
            assert_redistribution_in_motion(world, evidence);
            world.resource_mut::<Time<Virtual>>().unpause();
            set_phase(evidence, EvidencePhase::AwaitRedistributionSettlement);
            TestFlow::Continue
        }
        EvidencePhase::AwaitRedistributionSettlement => {
            pin_camera(world, cell_center(IVec2::ZERO), 1.55);
            tick_phase(evidence, "the redistributed cohort to settle");
            if physical_counts(world, &evidence.defenders, &REDISTRIBUTED_STAGING_CELLS)
                == Some([2, 2, 2])
                && evidence
                    .defenders
                    .iter()
                    .all(|entity| world.get::<DirectMovementComponent>(*entity).is_none())
            {
                evidence.hostile = Some(spawn_hostile(world));
                set_phase(evidence, EvidencePhase::AwaitInterception);
            }
            TestFlow::Continue
        }
        EvidencePhase::AwaitInterception => {
            pin_camera(world, Vec2::new(256.0, 512.0), 1.7);
            tick_phase(evidence, "interception on owned non-Defend territory");
            let hostile = evidence.hostile.expect("the Threat should be spawned");
            reset_hostile(world, hostile, THREAT_CELL);
            let responders = responders_to(world, &evidence.defenders, hostile);
            if evidence.responder.is_none() && responders.len() == 1 {
                evidence.responder = Some(responders[0]);
                evidence.response_start = defender_position(world, responders[0]);
            }
            if let Some(responder) = evidence.responder {
                assert_eq!(responders, [responder]);
                let distance =
                    defender_position(world, responder).distance(cell_center(THREAT_CELL));
                if world_to_cell(defender_position(world, responder)) == THREAT_CELL
                    && (120.0..=190.0).contains(&distance)
                {
                    world.resource_mut::<Time<Virtual>>().pause();
                    assert_interception(world, evidence);
                    set_phase(evidence, EvidencePhase::ResumeInterception);
                    return TestFlow::Screenshot(
                        "defender_response_intercepts_owned_non_defend_territory".to_string(),
                    );
                }
            }
            TestFlow::Continue
        }
        EvidencePhase::ResumeInterception => {
            assert!(world.resource::<Time<Virtual>>().is_paused());
            assert_interception(world, evidence);
            let hostile = evidence.hostile.expect("the Threat should remain present");
            reset_hostile(world, hostile, ORTHOGONAL_HALO_CELL);
            world.resource_mut::<Time<Virtual>>().unpause();
            set_phase(evidence, EvidencePhase::AwaitOrthogonalHalo);
            TestFlow::Continue
        }
        EvidencePhase::AwaitOrthogonalHalo => {
            pin_camera(world, Vec2::new(1_024.0, 768.0), 1.1);
            tick_phase(evidence, "orthogonal Pursuit Halo retention");
            let hostile = evidence.hostile.expect("the Threat should remain present");
            reset_hostile(world, hostile, ORTHOGONAL_HALO_CELL);
            let responder = evidence
                .responder
                .expect("the response should remain assigned");
            if world
                .get::<DefenderResponse>(responder)
                .is_some_and(|response| response.target == hostile)
                && world_to_cell(defender_position(world, responder)) == ORTHOGONAL_HALO_CELL
                && (120.0..=190.0).contains(
                    &defender_position(world, responder)
                        .distance(cell_center(ORTHOGONAL_HALO_CELL)),
                )
            {
                world.resource_mut::<Time<Virtual>>().pause();
                assert_halo_response(world, evidence, ORTHOGONAL_HALO_CELL);
                set_phase(evidence, EvidencePhase::ResumeOrthogonalHalo);
                return TestFlow::Screenshot(
                    "defender_response_retained_in_orthogonal_pursuit_halo".to_string(),
                );
            }
            TestFlow::Continue
        }
        EvidencePhase::ResumeOrthogonalHalo => {
            assert!(world.resource::<Time<Virtual>>().is_paused());
            assert_halo_response(world, evidence, ORTHOGONAL_HALO_CELL);
            let hostile = evidence.hostile.expect("the Threat should remain present");
            reset_hostile(world, hostile, DIAGONAL_HALO_CELL);
            world.resource_mut::<Time<Virtual>>().unpause();
            set_phase(evidence, EvidencePhase::AwaitDiagonalHalo);
            TestFlow::Continue
        }
        EvidencePhase::AwaitDiagonalHalo => {
            pin_camera(world, Vec2::new(1_024.0, 1_024.0), 1.45);
            tick_phase(evidence, "diagonal Pursuit Halo retention");
            let hostile = evidence.hostile.expect("the Threat should remain present");
            reset_hostile(world, hostile, DIAGONAL_HALO_CELL);
            let responder = evidence
                .responder
                .expect("the response should remain assigned");
            if world
                .get::<DefenderResponse>(responder)
                .is_some_and(|response| response.target == hostile)
                && world_to_cell(defender_position(world, responder)) == DIAGONAL_HALO_CELL
                && (120.0..=190.0).contains(
                    &defender_position(world, responder).distance(cell_center(DIAGONAL_HALO_CELL)),
                )
            {
                world.resource_mut::<Time<Virtual>>().pause();
                assert_halo_response(world, evidence, DIAGONAL_HALO_CELL);
                set_phase(evidence, EvidencePhase::ResumeDiagonalHalo);
                return TestFlow::Screenshot(
                    "defender_response_retained_in_diagonal_pursuit_halo".to_string(),
                );
            }
            TestFlow::Continue
        }
        EvidencePhase::ResumeDiagonalHalo => {
            assert!(world.resource::<Time<Virtual>>().is_paused());
            assert_halo_response(world, evidence, DIAGONAL_HALO_CELL);
            replace_staging_paint(world);
            world.resource_mut::<Time<Virtual>>().unpause();
            set_phase(evidence, EvidencePhase::AwaitCurrentLayoutReaction);
            TestFlow::Continue
        }
        EvidencePhase::AwaitCurrentLayoutReaction => {
            pin_camera(world, Vec2::new(1_024.0, 1_024.0), 1.45);
            tick_phase(
                evidence,
                "the unengaged cohort to react to current staging paint",
            );
            let hostile = evidence.hostile.expect("the Threat should remain present");
            reset_hostile(world, hostile, DIAGONAL_HALO_CELL);
            assert_current_staging_paint(world);
            assert_response_tracks(world, evidence, DIAGONAL_HALO_CELL, 0);
            let responder = evidence
                .responder
                .expect("the response should remain assigned");
            let return_movers = evidence
                .defenders
                .iter()
                .filter(|entity| {
                    **entity != responder
                        && world
                            .get::<DirectMovementComponent>(**entity)
                            .is_some_and(|movement| {
                                RETURN_STAGING_CELLS.contains(&world_to_cell(movement.xy))
                            })
                })
                .count();
            if return_movers > 0 {
                let _ = world.despawn(hostile);
                evidence.hostile = None;
                set_phase(evidence, EvidencePhase::AwaitReturn);
            }
            TestFlow::Continue
        }
        EvidencePhase::AwaitReturn => {
            pin_camera(world, cell_center(IVec2::new(0, -1)), 1.55);
            tick_phase(evidence, "post-Threat return to current staging");
            let responder = evidence
                .responder
                .expect("the responder should remain recorded");
            if world.get::<DefenderResponse>(responder).is_none()
                && physical_counts(world, &evidence.defenders, &RETURN_STAGING_CELLS)
                    == Some([2, 2, 2])
                && evidence
                    .defenders
                    .iter()
                    .all(|entity| world.get::<DirectMovementComponent>(*entity).is_none())
            {
                world.resource_mut::<Time<Virtual>>().pause();
                assert_returned_to_current_staging(world, evidence);
                set_phase(evidence, EvidencePhase::ResumeReturn);
                return TestFlow::Screenshot(
                    "defender_response_returns_to_current_staging_after_threat".to_string(),
                );
            }
            TestFlow::Continue
        }
        EvidencePhase::ResumeReturn => {
            assert!(world.resource::<Time<Virtual>>().is_paused());
            assert_returned_to_current_staging(world, evidence);
            TestFlow::Exit
        }
    }
}

pub fn defender_staging(ctx: &mut TestContext) -> TestFlow {
    if ctx.frame == 0 {
        let evidence = setup_scene(ctx.world);
        ctx.world.insert_resource(evidence);
        return TestFlow::Continue;
    }

    let mut evidence = ctx
        .world
        .remove_resource::<StagingEvidence>()
        .expect("the screenshot flow should retain its authored evidence state");
    let flow = advance_scene(ctx.world, &mut evidence);
    ctx.world.insert_resource(evidence);
    flow
}
