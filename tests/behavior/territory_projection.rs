use std::collections::HashMap;

use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    intent::{IntentGrid, IntentKind},
    nanobot::{
        AllocationRegion, Health, OwnerSwarm, Structure, StructureKind, Swarm, SwarmId,
        SwarmMember, TerritorySnapshot, ThreatKind,
    },
};

#[path = "../common/mod.rs"]
mod common;

#[test]
fn territory_snapshot_counts_each_owned_intent_cell_once_per_swarm() {
    let mut app = common::sim_app();
    app.insert_resource(IntentGrid::new(32, 32));
    let opponent = SwarmId(7);
    app.world_mut().spawn((Swarm {}, SwarmId::PLAYER));
    app.world_mut().spawn((Swarm {}, opponent));
    {
        let mut grid = app.world_mut().resource_mut::<IntentGrid>();
        for (cell, kind) in [
            (IVec2::ZERO, IntentKind::Gather),
            (IVec2::new(1, 0), IntentKind::Build),
            (IVec2::new(2, 0), IntentKind::Defend),
            (IVec2::new(8, 0), IntentKind::Corridor),
        ] {
            grid.paint(cell, kind, SwarmId::PLAYER);
        }
        grid.paint(IVec2::ZERO, IntentKind::Build, SwarmId::PLAYER);
        grid.paint(IVec2::ZERO, IntentKind::Gather, opponent);
    }

    app.update();

    let snapshot = app.world().resource::<TerritorySnapshot>();
    assert_eq!(snapshot.tile_count(SwarmId::PLAYER), 4);
    assert_eq!(snapshot.tile_count(opponent), 1);
    assert_eq!(snapshot.threat_count(SwarmId::PLAYER), 0);
    assert_eq!(snapshot.threat_count(opponent), 0);
    assert_eq!(
        snapshot.regions(SwarmId::PLAYER).collect::<Vec<_>>(),
        vec![
            AllocationRegion::for_cell(IVec2::ZERO),
            AllocationRegion::for_cell(IVec2::new(8, 0)),
        ],
    );
    assert_eq!(
        snapshot.tiles_in_region(SwarmId::PLAYER, AllocationRegion::for_cell(IVec2::ZERO)),
        &[IVec2::ZERO, IVec2::new(1, 0), IVec2::new(2, 0)],
    );
    assert_eq!(
        snapshot.tiles_in_region(
            SwarmId::PLAYER,
            AllocationRegion::for_cell(IVec2::new(8, 0)),
        ),
        &[IVec2::new(8, 0)],
    );
}

#[test]
fn territory_snapshot_counts_living_hostiles_physically_inside_swarm_tiles() {
    let mut app = common::sim_app();
    let opponent = SwarmId(7);
    app.world_mut().spawn((Swarm {}, SwarmId::PLAYER));
    let opponent_swarm = app.world_mut().spawn((Swarm {}, opponent)).id();
    app.world_mut().resource_mut::<IntentGrid>().paint(
        IVec2::ZERO,
        IntentKind::Gather,
        SwarmId::PLAYER,
    );

    let hostile_nanobot = common::spawn_worker_at(&mut app, Vec2::new(64.0, 64.0));
    app.world_mut()
        .entity_mut(hostile_nanobot)
        .insert(SwarmMember::new(opponent));
    let hostile_structure = app
        .world_mut()
        .spawn((
            Structure::new(StructureKind::Basic),
            OwnerSwarm(opponent_swarm),
            Transform::from_xyz(128.0, 128.0, 0.0),
        ))
        .id();
    common::spawn_defender_at(&mut app, Vec2::new(192.0, 192.0));
    let outside_hostile = common::spawn_hauler_at(&mut app, Vec2::new(600.0, 64.0));
    app.world_mut()
        .entity_mut(outside_hostile)
        .insert(SwarmMember::new(opponent));
    let dead_hostile = common::spawn_defender_at(&mut app, Vec2::new(256.0, 256.0));
    app.world_mut()
        .entity_mut(dead_hostile)
        .insert(SwarmMember::new(opponent));
    app.world_mut()
        .entity_mut(dead_hostile)
        .get_mut::<Health>()
        .unwrap()
        .current = 0;

    app.update();

    let first_projection = {
        let snapshot = app.world().resource::<TerritorySnapshot>();
        assert_eq!(snapshot.threat_count(SwarmId::PLAYER), 2);
        let threats =
            snapshot.threats_in_region(SwarmId::PLAYER, AllocationRegion::for_cell(IVec2::ZERO));
        let by_entity = threats
            .iter()
            .map(|threat| (threat.entity, threat.kind))
            .collect::<HashMap<_, _>>();
        assert_eq!(by_entity.len(), 2);
        assert_eq!(
            by_entity.get(&hostile_nanobot),
            Some(&ThreatKind::OtherNanobot),
        );
        assert_eq!(
            by_entity.get(&hostile_structure),
            Some(&ThreatKind::Structure),
        );
        threats.to_vec()
    };

    app.update();

    assert_eq!(
        app.world()
            .resource::<TerritorySnapshot>()
            .threats_in_region(SwarmId::PLAYER, AllocationRegion::for_cell(IVec2::ZERO)),
        first_projection,
        "unchanged physical state must reproduce the same stable projection",
    );
}

#[test]
fn overlapping_gather_territory_projects_hostile_occupants_for_both_swarms() {
    let mut app = common::sim_app();
    let opponent = SwarmId(7);
    app.world_mut().spawn((Swarm {}, SwarmId::PLAYER));
    app.world_mut().spawn((Swarm {}, opponent));
    {
        let mut grid = app.world_mut().resource_mut::<IntentGrid>();
        grid.paint(IVec2::ZERO, IntentKind::Gather, SwarmId::PLAYER);
        grid.paint(IVec2::ZERO, IntentKind::Gather, opponent);
    }
    let player_worker = common::spawn_worker_at(&mut app, Vec2::new(64.0, 64.0));
    let enemy_worker = common::spawn_worker_at(&mut app, Vec2::new(192.0, 192.0));
    app.world_mut()
        .entity_mut(enemy_worker)
        .insert(SwarmMember::new(opponent));

    app.update();

    let snapshot = app.world().resource::<TerritorySnapshot>();
    let region = AllocationRegion::for_cell(IVec2::ZERO);
    assert_eq!(snapshot.tile_count(SwarmId::PLAYER), 1);
    assert_eq!(snapshot.tile_count(opponent), 1);
    assert_eq!(
        snapshot
            .threats_in_region(SwarmId::PLAYER, region)
            .iter()
            .map(|threat| threat.entity)
            .collect::<Vec<_>>(),
        vec![enemy_worker]
    );
    assert_eq!(
        snapshot
            .threats_in_region(opponent, region)
            .iter()
            .map(|threat| threat.entity)
            .collect::<Vec<_>>(),
        vec![player_worker]
    );
}
