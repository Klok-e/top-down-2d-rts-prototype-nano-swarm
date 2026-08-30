use std::collections::BTreeSet;

use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    intent::{IntentGrid, IntentKind},
    nanobot::{
        DefenderResponse, OwnerSwarm, Structure, StructureKind, Swarm, SwarmId, SwarmMember,
        world_to_cell,
    },
};

#[path = "../common/mod.rs"]
mod common;

#[test]
fn distinct_threats_receive_one_response_before_surplus_defenders() {
    let mut app = common::sim_app();
    let opponent = SwarmId(9);
    app.world_mut().spawn((Swarm {}, SwarmId::PLAYER));
    let opponent_entity = app.world_mut().spawn((Swarm {}, opponent)).id();
    let threatened_cell = IVec2::new(2, 2);
    app.world_mut().resource_mut::<IntentGrid>().paint_owned(
        threatened_cell,
        IntentKind::Gather,
        Some(SwarmId::PLAYER),
    );

    let center = common::cell_world_center(threatened_cell);
    let hostile_nanobot = common::spawn_worker_at(&mut app, center + Vec2::new(-70.0, 0.0));
    app.world_mut()
        .entity_mut(hostile_nanobot)
        .insert(SwarmMember::new(opponent));
    let hostile_structure = app
        .world_mut()
        .spawn((
            Structure::new(StructureKind::Basic),
            OwnerSwarm(opponent_entity),
            Transform::from_translation((center + Vec2::new(70.0, 0.0)).extend(0.0)),
        ))
        .id();

    let defenders = [
        common::spawn_defender_at(&mut app, center + Vec2::new(-700.0, -120.0)),
        common::spawn_defender_at(&mut app, center + Vec2::new(-700.0, 0.0)),
        common::spawn_defender_at(&mut app, center + Vec2::new(-700.0, 120.0)),
    ];
    assert!(defenders.iter().all(|defender| {
        app.world()
            .entity(*defender)
            .get::<DefenderResponse>()
            .is_none()
    }));

    app.update();

    let responses = defenders
        .into_iter()
        .filter_map(|defender| {
            app.world()
                .entity(defender)
                .get::<DefenderResponse>()
                .map(|response| response.target)
        })
        .collect::<Vec<_>>();
    assert_eq!(
        responses.len(),
        2,
        "one Defender should remain unengaged after both Threats are covered",
    );
    assert_eq!(
        responses
            .iter()
            .filter(|target| **target == hostile_nanobot)
            .count(),
        1,
        "the hostile nanobot should accept exactly one response",
    );
    assert_eq!(
        responses
            .iter()
            .filter(|target| **target == hostile_structure)
            .count(),
        1,
        "the hostile structure should accept exactly one response",
    );
}

#[test]
fn response_coverage_pages_beyond_the_per_defender_candidate_bound() {
    let mut app = common::sim_app();
    let opponent = SwarmId(9);
    app.world_mut().spawn((Swarm {}, SwarmId::PLAYER));
    app.world_mut().spawn((Swarm {}, opponent));
    let threatened_cell = IVec2::new(2, 2);
    app.world_mut().resource_mut::<IntentGrid>().paint_owned(
        threatened_cell,
        IntentKind::Gather,
        Some(SwarmId::PLAYER),
    );
    let center = common::cell_world_center(threatened_cell);
    let threats = (0..129)
        .map(|index| {
            let entity =
                common::spawn_worker_at(&mut app, center + Vec2::new(index as f32 * 0.25, 0.0));
            app.world_mut()
                .entity_mut(entity)
                .insert(SwarmMember::new(opponent));
            entity
        })
        .collect::<BTreeSet<_>>();
    let defenders = (0..129)
        .map(|index| {
            common::spawn_defender_at(&mut app, center + Vec2::new(-200.0, index as f32 * 0.25))
        })
        .collect::<Vec<_>>();

    app.update();

    let responses = defenders
        .into_iter()
        .map(|defender| {
            app.world()
                .entity(defender)
                .get::<DefenderResponse>()
                .expect("every bounded page should produce a response")
                .target
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(responses, threats);
}

#[test]
fn responder_crosses_neutral_and_hostile_gaps_without_releasing_its_claim() {
    let mut app = common::sim_app();
    let opponent = SwarmId(9);
    app.world_mut().spawn((Swarm {}, SwarmId::PLAYER));
    app.world_mut().spawn((Swarm {}, opponent));
    let start_cell = IVec2::ZERO;
    let threatened_cell = IVec2::new(3, 0);
    {
        let mut grid = app.world_mut().resource_mut::<IntentGrid>();
        grid.paint_owned(start_cell, IntentKind::Corridor, Some(SwarmId::PLAYER));
        grid.paint_owned(IVec2::new(2, 0), IntentKind::Build, Some(opponent));
        grid.paint_owned(threatened_cell, IntentKind::Gather, Some(SwarmId::PLAYER));
    }
    let defender = common::spawn_defender_at(&mut app, common::cell_world_center(start_cell));
    let target = common::spawn_worker_at(&mut app, common::cell_world_center(threatened_cell));
    app.world_mut()
        .entity_mut(target)
        .insert(SwarmMember::new(opponent));

    app.update();
    assert_eq!(
        app.world()
            .entity(defender)
            .get::<DefenderResponse>()
            .map(|response| response.target),
        Some(target),
    );

    for _ in 0..180 {
        app.update();
    }

    let defender_cell = world_to_cell(
        app.world()
            .entity(defender)
            .get::<Transform>()
            .unwrap()
            .translation
            .truncate(),
    );
    assert_eq!(
        defender_cell,
        IVec2::new(2, 0),
        "the responder should cross neutral space and enter the hostile-owned gap",
    );
    assert_eq!(
        app.world()
            .entity(defender)
            .get::<DefenderResponse>()
            .map(|response| response.target),
        Some(target),
        "crossing unowned space must not invalidate a target that remains on a Swarm Tile",
    );
}

#[test]
fn higher_danger_preempts_while_same_tier_response_stays_stable() {
    let mut app = common::sim_app();
    let opponent = SwarmId(9);
    app.world_mut().spawn((Swarm {}, SwarmId::PLAYER));
    let opponent_entity = app.world_mut().spawn((Swarm {}, opponent)).id();
    let threatened_cell = IVec2::new(2, 2);
    app.world_mut().resource_mut::<IntentGrid>().paint_owned(
        threatened_cell,
        IntentKind::Build,
        Some(SwarmId::PLAYER),
    );
    let center = common::cell_world_center(threatened_cell);
    let structure = app
        .world_mut()
        .spawn((
            Structure::new(StructureKind::Basic),
            OwnerSwarm(opponent_entity),
            Transform::from_translation((center + Vec2::new(120.0, 0.0)).extend(0.0)),
        ))
        .id();
    let defender = common::spawn_defender_at(&mut app, center + Vec2::new(-700.0, 0.0));

    app.update();
    assert_eq!(
        app.world()
            .entity(defender)
            .get::<DefenderResponse>()
            .map(|response| response.target),
        Some(structure),
    );

    let first_worker = common::spawn_worker_at(&mut app, center + Vec2::new(80.0, 0.0));
    app.world_mut()
        .entity_mut(first_worker)
        .insert(SwarmMember::new(opponent));
    app.update();
    assert_eq!(
        app.world()
            .entity(defender)
            .get::<DefenderResponse>()
            .map(|response| response.target),
        Some(first_worker),
        "a hostile nanobot must preempt a structure response",
    );

    let closer_worker = common::spawn_worker_at(&mut app, center + Vec2::new(-180.0, 0.0));
    app.world_mut()
        .entity_mut(closer_worker)
        .insert(SwarmMember::new(opponent));
    app.update();
    assert_eq!(
        app.world()
            .entity(defender)
            .get::<DefenderResponse>()
            .map(|response| response.target),
        Some(first_worker),
        "a valid same-tier response must not churn toward a newer closer target",
    );

    let hostile_defender = common::spawn_defender_at(&mut app, center + Vec2::new(180.0, 0.0));
    app.world_mut()
        .entity_mut(hostile_defender)
        .insert(SwarmMember::new(opponent));
    app.update();
    assert_eq!(
        app.world()
            .entity(defender)
            .get::<DefenderResponse>()
            .map(|response| response.target),
        Some(hostile_defender),
        "a hostile Defender must preempt a lower-tier nanobot response",
    );
}

#[test]
fn existing_claim_crosses_diagonal_halo_but_halo_does_not_create_work() {
    let mut app = common::sim_app();
    let opponent = SwarmId(9);
    app.world_mut().spawn((Swarm {}, SwarmId::PLAYER));
    app.world_mut().spawn((Swarm {}, opponent));
    let territory_cell = IVec2::new(2, 2);
    app.world_mut().resource_mut::<IntentGrid>().paint_owned(
        territory_cell,
        IntentKind::Corridor,
        Some(SwarmId::PLAYER),
    );
    let target = common::spawn_worker_at(&mut app, common::cell_world_center(territory_cell));
    app.world_mut()
        .entity_mut(target)
        .insert(SwarmMember::new(opponent));
    let defender = common::spawn_defender_at(
        &mut app,
        common::cell_world_center(territory_cell) + Vec2::new(-700.0, 0.0),
    );

    app.update();
    assert_eq!(
        app.world()
            .entity(defender)
            .get::<DefenderResponse>()
            .map(|response| response.target),
        Some(target),
    );

    let diagonal_halo_cell = territory_cell + IVec2::ONE;
    app.world_mut()
        .entity_mut(target)
        .get_mut::<Transform>()
        .unwrap()
        .translation = common::cell_world_center(diagonal_halo_cell).extend(0.0);
    app.update();
    assert_eq!(
        app.world()
            .entity(defender)
            .get::<DefenderResponse>()
            .map(|response| response.target),
        Some(target),
        "the existing response must survive a diagonal Pursuit Halo move",
    );

    let beyond_halo_cell = territory_cell + IVec2::splat(2);
    app.world_mut()
        .entity_mut(target)
        .get_mut::<Transform>()
        .unwrap()
        .translation = common::cell_world_center(beyond_halo_cell).extend(0.0);
    app.update();
    assert!(
        app.world()
            .entity(defender)
            .get::<DefenderResponse>()
            .is_none(),
        "pursuit must end beyond the one-cell halo",
    );

    let halo_only =
        common::spawn_worker_at(&mut app, common::cell_world_center(diagonal_halo_cell));
    app.world_mut()
        .entity_mut(halo_only)
        .insert(SwarmMember::new(opponent));
    app.update();
    assert!(
        app.world()
            .entity(defender)
            .get::<DefenderResponse>()
            .is_none(),
        "a hostile that starts in the halo must not originate response work",
    );
}
