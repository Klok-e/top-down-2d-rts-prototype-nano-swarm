use bevy::{ecs::message::MessageCursor, prelude::*};
use top_down_2d_rts_prototype_nano_swarm::{
    ai_battle::AiBattleLayout,
    intent::{IntentGrid, IntentKind},
    nanobot::{
        DefenderResponse, Health, ResolvedCombatFact, SwarmId, SwarmMember,
        nanobot_death_cleanup_system, world_to_cell,
    },
    reachability::{IntentWriteAction, SwarmReachabilityMap, apply_reachable_intent},
    scenario::{LayoutSetup, cell_origin, default_rock_geometry},
    terrain::RockFormation,
};

#[path = "../common/mod.rs"]
mod common;

const OPPONENT: SwarmId = SwarmId(1);

fn install_terrain(
    app: &mut App,
    grid: IntentGrid,
    terrain: Vec<(RockFormation, Transform)>,
    homes: [(SwarmId, Vec2); 2],
) -> SwarmReachabilityMap {
    let ground = SwarmReachabilityMap::new(
        &grid,
        terrain
            .iter()
            .map(|(rock, transform)| rock.obstacle(transform)),
        homes,
    )
    .unwrap();
    app.insert_resource(grid).insert_resource(ground.clone());
    app.world_mut().spawn_batch(terrain);
    for (owner, home) in homes {
        let swarm = common::spawn_swarm_at(app, home);
        app.world_mut().entity_mut(swarm).insert(owner);
    }
    ground
}

#[test]
fn defend_paint_on_flanks_edge_ground_kills_the_hauler() {
    let mut app = common::sim_app_with_combat();
    app.add_systems(FixedLast, nanobot_death_cleanup_system);
    let layout = LayoutSetup::for_layout(AiBattleLayout::Flanks);
    let ground = install_terrain(
        &mut app,
        IntentGrid::new(1000, 1000),
        default_rock_geometry(),
        [
            (SwarmId::PLAYER, cell_origin(layout.home)),
            (OPPONENT, cell_origin(LayoutSetup::opposite(layout.home))),
        ],
    );
    // Flanks seed 11 placed the last Hauler on the reachable strip of the west edge tile.
    let target_position = Vec2::new(-520.12854, 3269.0374);
    let west_edge = IVec2::new(-2, 6);
    assert_eq!(world_to_cell(target_position), west_edge);
    let defender = common::spawn_defender_at(&mut app, Vec2::new(-230.0, target_position.y));
    app.world_mut()
        .entity_mut(defender)
        .insert(SwarmMember(OPPONENT));
    let hauler = common::spawn_hauler_at(&mut app, target_position);
    assert_eq!(
        apply_reachable_intent(
            &mut app.world_mut().resource_mut::<IntentGrid>(),
            &ground,
            OPPONENT,
            west_edge,
            IntentKind::Defend,
            IntentWriteAction::Paint,
        ),
        Ok(true)
    );

    let mut saw_pursuit = false;
    let mut hit_hauler = false;
    let mut combat_facts = MessageCursor::<ResolvedCombatFact>::default();
    for _ in 0..400 {
        app.update();
        let world = app.world();
        saw_pursuit |= world
            .get::<DefenderResponse>(defender)
            .is_some_and(|response| response.target == hauler);
        for fact in combat_facts.read(world.resource::<Messages<ResolvedCombatFact>>()) {
            if let ResolvedCombatFact::Hit(hit) = fact
                && hit.attacker.entity == defender
                && hit.target.entity == hauler
            {
                hit_hauler = true;
            }
        }
        if world.get_entity(hauler).is_err() {
            break;
        }
    }

    assert!(
        saw_pursuit,
        "Defend paint must start a response to the Hauler"
    );
    assert!(
        hit_hauler,
        "the responding Defender must hit the Hauler on edge ground"
    );
    assert!(app.world().get_entity(hauler).is_err());
}

#[test]
fn split_tile_defends_only_the_ground_connected_to_its_swarm() {
    let mut app = common::sim_app_with_combat();
    let start = Vec2::new(96.0, -100.0);
    let target = Vec2::new(420.0, 160.0);
    let ground = install_terrain(
        &mut app,
        IntentGrid::new(8, 8),
        vec![(
            RockFormation::Rectangle {
                half: Vec2::new(48.0, 8_000.0),
            },
            Transform::from_xyz(256.0, 0.0, 0.0),
        )],
        [(SwarmId::PLAYER, start), (OPPONENT, target)],
    );
    let cell = IVec2::ZERO;
    assert_eq!(world_to_cell(target), cell);
    let defender = common::spawn_defender_at(&mut app, start);
    let hauler = common::spawn_hauler_at(&mut app, target);
    app.world_mut()
        .entity_mut(hauler)
        .insert(SwarmMember(OPPONENT));
    let initial_health = app.world().get::<Health>(hauler).unwrap().current;
    assert_eq!(
        apply_reachable_intent(
            &mut app.world_mut().resource_mut::<IntentGrid>(),
            &ground,
            SwarmId::PLAYER,
            cell,
            IntentKind::Defend,
            IntentWriteAction::Paint,
        ),
        Ok(true)
    );

    let mut reached_paint = false;
    for _ in 0..240 {
        app.update();
        assert!(
            app.world().get::<DefenderResponse>(defender).is_none(),
            "an enemy in the sealed part of the same tile cannot be a response target"
        );
        let position = app
            .world()
            .get::<Transform>(defender)
            .unwrap()
            .translation
            .truncate();
        reached_paint |= world_to_cell(position) == cell;
        assert!(
            position.x <= 174.0,
            "staging crossed the wall: {position:?}"
        );
    }
    assert!(
        reached_paint,
        "a Defender should stage on the reachable part of the paint"
    );
    assert_eq!(
        app.world().get::<Health>(hauler).unwrap().current,
        initial_health
    );
}
