#[path = "../common/mod.rs"]
mod common;

use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    intent::{IntentGrid, IntentKind},
    nanobot::{
        CombatPlugin, DefendHold, DefendPressure, DirectMovementComponent, Health, OwnerSwarm,
        Structure, StructureKind, Swarm, SwarmId, SwarmMember,
    },
};

#[test]
fn opposing_holding_defenders_exchange_damage_and_raise_pressure() {
    let mut app = common::sim_app_with_defend();
    app.add_plugins(CombatPlugin);
    let cell = IVec2::ZERO;
    app.world_mut().resource_mut::<IntentGrid>().paint_owned(
        cell,
        IntentKind::Defend,
        Some(SwarmId::PLAYER),
    );
    let center = common::cell_world_center(cell);
    let player = common::spawn_defender_at(&mut app, center + Vec2::new(-8.0, 0.0));
    app.world_mut()
        .entity_mut(player)
        .insert(DefendHold { cell });
    let enemy = common::spawn_defender_at(&mut app, center + Vec2::new(8.0, 0.0));
    app.world_mut()
        .entity_mut(enemy)
        .insert((SwarmMember::new(SwarmId(11)), DefendHold { cell }));

    let player_before = app.world().entity(player).get::<Health>().unwrap().current;
    let enemy_before = app.world().entity(enemy).get::<Health>().unwrap().current;
    app.update();

    assert!(app.world().entity(player).get::<Health>().unwrap().current < player_before);
    assert!(app.world().entity(enemy).get::<Health>().unwrap().current < enemy_before);
    assert!(
        app.world().resource::<DefendPressure>().get(cell) > 1.0,
        "hostile presence in owned Defend paint raises pressure",
    );
}

#[test]
fn threat_pressure_observes_post_integration_cell() {
    let mut app = common::sim_app_with_defend();
    app.add_plugins(CombatPlugin);
    let cell = IVec2::ZERO;
    let center = common::cell_world_center(cell);
    app.world_mut().resource_mut::<IntentGrid>().paint_owned(
        cell,
        IntentKind::Defend,
        Some(SwarmId::PLAYER),
    );
    let enemy = common::spawn_defender_at(&mut app, Vec2::new(-1.0, center.y));
    app.world_mut().entity_mut(enemy).insert((
        SwarmMember::new(SwarmId(11)),
        DirectMovementComponent {
            xy: center,
            stop_radius: 0.0,
        },
    ));

    app.update();

    assert!(
        app.world()
            .entity(enemy)
            .get::<Transform>()
            .unwrap()
            .translation
            .x
            > 0.0
    );
    assert!(
        app.world()
            .resource::<DefendPressure>()
            .get_for(SwarmId::PLAYER, cell)
            > 1.0,
        "threat projection reads the cell reached during this fixed tick",
    );
}

#[test]
fn holding_defender_damages_hostile_support_structure() {
    let mut app = common::sim_app_with_defend();
    app.add_plugins(CombatPlugin);
    let cell = IVec2::ZERO;
    let center = common::cell_world_center(cell);
    app.world_mut().resource_mut::<IntentGrid>().paint_owned(
        cell,
        IntentKind::Defend,
        Some(SwarmId::PLAYER),
    );
    let defender = common::spawn_defender_at(&mut app, center);
    app.world_mut()
        .entity_mut(defender)
        .insert(DefendHold { cell });
    let enemy_swarm = app.world_mut().spawn((Swarm {}, SwarmId(11))).id();
    let structure = app
        .world_mut()
        .spawn((
            Structure::new(StructureKind::Basic),
            OwnerSwarm(enemy_swarm),
            Transform::from_translation((center + Vec2::new(16.0, 0.0)).extend(0.0)),
        ))
        .id();
    let before = app
        .world()
        .entity(structure)
        .get::<Structure>()
        .unwrap()
        .health;

    app.update();

    assert!(
        app.world()
            .entity(structure)
            .get::<Structure>()
            .unwrap()
            .health
            < before,
        "hostile support structure is a secondary combat target",
    );
}

#[test]
fn lethal_combat_despawns_support_structure_immediately() {
    let mut app = common::sim_app_with_defend();
    app.add_plugins(CombatPlugin);
    let cell = IVec2::ZERO;
    let center = common::cell_world_center(cell);
    app.world_mut().resource_mut::<IntentGrid>().paint_owned(
        cell,
        IntentKind::Defend,
        Some(SwarmId::PLAYER),
    );
    let defender = common::spawn_defender_at(&mut app, center);
    app.world_mut()
        .entity_mut(defender)
        .insert(DefendHold { cell });
    let enemy_swarm = app.world_mut().spawn((Swarm {}, SwarmId(11))).id();
    let mut condition = Structure::new(StructureKind::Basic);
    condition.health = 1;
    let structure = app
        .world_mut()
        .spawn((
            condition,
            OwnerSwarm(enemy_swarm),
            Transform::from_translation((center + Vec2::new(16.0, 0.0)).extend(0.0)),
        ))
        .id();

    app.update();

    assert!(
        !app.world().entities().contains(structure),
        "zero-health support structure must not remain repairable",
    );
}

#[test]
fn shared_defend_paint_does_not_assign_self_hostility_to_player() {
    let mut app = common::sim_app_with_defend();
    app.add_plugins(CombatPlugin);
    let cell = IVec2::ZERO;
    app.world_mut()
        .resource_mut::<IntentGrid>()
        .paint(cell, IntentKind::Defend);
    let enemy = common::spawn_defender_at(&mut app, common::cell_world_center(cell));
    app.world_mut()
        .entity_mut(enemy)
        .insert(SwarmMember::new(SwarmId(11)));

    app.update();

    let pressure = app.world().resource::<DefendPressure>();
    assert_eq!(pressure.get_for(SwarmId::PLAYER, cell), 1.0);
    assert_eq!(pressure.get_for(SwarmId(11), cell), 1.0);
}
