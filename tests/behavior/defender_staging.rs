//! Observable behavior coverage for issue #55's responsive Defender staging.

use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    intent::{IntentGrid, IntentKind},
    nanobot::{
        ChargerAssignment, DefenderResponse, DirectMovementComponent, SwarmId, SwarmMember,
        world_to_cell,
    },
};

#[path = "../common/mod.rs"]
mod common;

#[test]
fn adding_owned_defend_paint_retargets_on_the_next_fixed_step() {
    let mut app = common::sim_app();
    let start = common::cell_world_center(IVec2::ZERO);
    let target_cell = IVec2::X;
    let defender = common::spawn_defender_at(&mut app, start);

    app.update();
    app.world_mut().resource_mut::<IntentGrid>().paint_owned(
        target_cell,
        IntentKind::Defend,
        Some(SwarmId::PLAYER),
    );

    app.update();

    let movement = app
        .world()
        .entity(defender)
        .get::<DirectMovementComponent>()
        .expect("new Defend paint should create normal-speed cross-cell travel");
    assert_eq!(world_to_cell(movement.xy), target_cell);
    assert!(
        movement.xy.x > start.x,
        "the cross-cell destination should physically lead toward the new paint",
    );
}

#[test]
fn clumped_defenders_redistribute_to_balanced_painted_cells() {
    let mut app = common::sim_app();
    let crowded = IVec2::ZERO;
    let empty = IVec2::X;
    for cell in [crowded, empty] {
        app.world_mut().resource_mut::<IntentGrid>().paint_owned(
            cell,
            IntentKind::Defend,
            Some(SwarmId::PLAYER),
        );
    }
    let center = common::cell_world_center(crowded);
    let defenders = [
        common::spawn_defender_at(&mut app, center + Vec2::new(-20.0, 0.0)),
        common::spawn_defender_at(&mut app, center),
        common::spawn_defender_at(&mut app, center + Vec2::new(20.0, 0.0)),
    ];

    app.update();

    let moving_to_empty = defenders
        .iter()
        .filter(|defender| {
            app.world()
                .entity(**defender)
                .get::<DirectMovementComponent>()
                .is_some_and(|movement| world_to_cell(movement.xy) == empty)
        })
        .count();
    assert_eq!(
        moving_to_empty, 1,
        "a 3-to-0 clump across two painted cells should rebalance to 2-to-1",
    );

    for _ in 0..180 {
        app.update();
    }
    let crowded_count = defenders
        .iter()
        .filter(|defender| {
            let position = app
                .world()
                .entity(**defender)
                .get::<Transform>()
                .unwrap()
                .translation
                .truncate();
            world_to_cell(position) == crowded
        })
        .count();
    let empty_count = defenders.len() - crowded_count;
    assert_eq!((crowded_count, empty_count), (2, 1));
}

#[test]
fn disconnected_zones_receive_defenders_in_proportion_to_painted_area() {
    let mut app = common::sim_app();
    let small_zone = IVec2::new(-2, 0);
    let large_zone = [IVec2::new(1, 0), IVec2::new(2, 0), IVec2::new(3, 0)];
    for cell in std::iter::once(small_zone).chain(large_zone) {
        app.world_mut().resource_mut::<IntentGrid>().paint_owned(
            cell,
            IntentKind::Defend,
            Some(SwarmId::PLAYER),
        );
    }
    let center = common::cell_world_center(small_zone);
    let defenders = (0..5)
        .map(|index| {
            common::spawn_defender_at(&mut app, center + Vec2::new(0.0, index as f32 * 8.0))
        })
        .collect::<Vec<_>>();

    app.update();

    let moving_to_large_zone = defenders
        .iter()
        .filter(|defender| {
            app.world()
                .entity(**defender)
                .get::<DirectMovementComponent>()
                .is_some_and(|movement| large_zone.contains(&world_to_cell(movement.xy)))
        })
        .count();
    assert_eq!(
        moving_to_large_zone, 4,
        "one painted cell should stage one Defender while three cells stage four",
    );
}

#[test]
fn balanced_layout_chooses_the_extra_slot_that_minimizes_travel() {
    let mut app = common::sim_app();
    let west = IVec2::new(-2, 0);
    let east = IVec2::new(2, 0);
    for cell in [west, east] {
        app.world_mut().resource_mut::<IntentGrid>().paint_owned(
            cell,
            IntentKind::Defend,
            Some(SwarmId::PLAYER),
        );
    }
    let west_defender = common::spawn_defender_at(&mut app, common::cell_world_center(west));
    let east_defender = common::spawn_defender_at(&mut app, common::cell_world_center(east));
    let returning =
        common::spawn_defender_at(&mut app, common::cell_world_center(IVec2::new(1, 0)));

    app.update();

    let movement = app
        .world()
        .entity(returning)
        .get::<DirectMovementComponent>()
        .expect("the off-paint Defender should join one balanced staging cell");
    assert_eq!(
        world_to_cell(movement.xy),
        east,
        "the equally balanced 1-to-2 layout should choose the nearby eastern extra slot",
    );
    for defender in [west_defender, east_defender] {
        assert!(
            app.world()
                .entity(defender)
                .get::<DirectMovementComponent>()
                .is_none(),
            "Defenders already filling required cells should not be displaced",
        );
    }
}

#[test]
fn remaining_cohort_uses_the_global_minimum_travel_assignment() {
    let mut app = common::sim_app();
    let west = IVec2::new(-2, 0);
    let east = IVec2::new(2, 0);
    for cell in [west, east] {
        app.world_mut().resource_mut::<IntentGrid>().paint_owned(
            cell,
            IntentKind::Defend,
            Some(SwarmId::PLAYER),
        );
    }
    let first = common::spawn_defender_at(&mut app, Vec2::ZERO);
    let second = common::spawn_defender_at(&mut app, Vec2::ZERO);
    let (eastern_defender, western_defender) = if first.to_bits() < second.to_bits() {
        (first, second)
    } else {
        (second, first)
    };
    app.world_mut()
        .entity_mut(eastern_defender)
        .get_mut::<Transform>()
        .unwrap()
        .translation = Vec2::new(200.0, 0.0).extend(0.0);
    app.world_mut()
        .entity_mut(western_defender)
        .get_mut::<Transform>()
        .unwrap()
        .translation = Vec2::new(-200.0, 0.0).extend(0.0);

    app.update();

    let target_cell = |defender: Entity| {
        world_to_cell(
            app.world()
                .entity(defender)
                .get::<DirectMovementComponent>()
                .expect("an off-paint Defender should travel to its staging slot")
                .xy,
        )
    };
    assert_eq!(target_cell(eastern_defender), east);
    assert_eq!(target_cell(western_defender), west);
}

#[test]
fn remainder_capacity_and_matching_minimize_travel_together() {
    let mut app = common::sim_app();
    let west = IVec2::new(-2, 0);
    let east = IVec2::new(2, 0);
    for cell in [west, east] {
        app.world_mut().resource_mut::<IntentGrid>().paint_owned(
            cell,
            IntentKind::Defend,
            Some(SwarmId::PLAYER),
        );
    }
    let defenders = [
        common::spawn_defender_at(&mut app, Vec2::new(-512.0, 256.0)),
        common::spawn_defender_at(&mut app, Vec2::new(1023.9, 256.0)),
        common::spawn_defender_at(&mut app, Vec2::new(1000.0, 156.0)),
    ];

    app.update();

    let east_count = defenders
        .into_iter()
        .filter(|defender| {
            app.world()
                .entity(*defender)
                .get::<DirectMovementComponent>()
                .is_some_and(|movement| world_to_cell(movement.xy) == east)
        })
        .count();
    assert_eq!(
        east_count, 2,
        "the extra eastern slot lowers total cohort travel despite a slightly cheaper western first match",
    );
}

#[test]
fn erasing_defend_paint_retargets_only_the_now_displaced_defender() {
    let mut app = common::sim_app();
    let west = IVec2::ZERO;
    let east = IVec2::X;
    for cell in [west, east] {
        app.world_mut().resource_mut::<IntentGrid>().paint_owned(
            cell,
            IntentKind::Defend,
            Some(SwarmId::PLAYER),
        );
    }
    let west_defender = common::spawn_defender_at(&mut app, common::cell_world_center(west));
    let east_defender = common::spawn_defender_at(&mut app, common::cell_world_center(east));
    app.update();

    app.world_mut()
        .resource_mut::<IntentGrid>()
        .remove(east, IntentKind::Defend);
    app.update();

    let east_movement = app
        .world()
        .entity(east_defender)
        .get::<DirectMovementComponent>()
        .expect("the Defender on erased paint should join the remaining staging cell");
    assert_eq!(world_to_cell(east_movement.xy), west);
    assert!(
        app.world()
            .entity(west_defender)
            .get::<DirectMovementComponent>()
            .is_none(),
        "the Defender already filling the remaining cell should keep its post",
    );
}

#[test]
fn cross_cell_redistribution_uses_normal_travel_speed() {
    let mut app = common::sim_app();
    let start_cell = IVec2::ZERO;
    let target_cell = IVec2::X;
    app.world_mut().resource_mut::<IntentGrid>().paint_owned(
        target_cell,
        IntentKind::Defend,
        Some(SwarmId::PLAYER),
    );
    let defender = common::spawn_defender_at(&mut app, common::cell_world_center(start_cell));
    app.update();
    let before = app
        .world()
        .entity(defender)
        .get::<Transform>()
        .unwrap()
        .translation
        .truncate();

    app.update();

    let after = app
        .world()
        .entity(defender)
        .get::<Transform>()
        .unwrap()
        .translation
        .truncate();
    let distance = before.distance(after);
    assert!(
        (distance - common::default_game_settings().bot_speed).abs() <= 1e-4,
        "cross-cell travel should use the normal speed cap; moved {distance}",
    );
}

#[test]
fn swarm_tiles_stage_defenders_when_owned_defend_paint_is_absent() {
    let mut app = common::sim_app();
    let crowded = IVec2::ZERO;
    let empty = IVec2::X;
    for cell in [crowded, empty] {
        app.world_mut().resource_mut::<IntentGrid>().paint_owned(
            cell,
            IntentKind::Gather,
            Some(SwarmId::PLAYER),
        );
    }
    let center = common::cell_world_center(crowded);
    let defenders = [
        common::spawn_defender_at(&mut app, center + Vec2::new(-20.0, 0.0)),
        common::spawn_defender_at(&mut app, center),
        common::spawn_defender_at(&mut app, center + Vec2::new(20.0, 0.0)),
    ];

    app.update();

    let moving_to_empty = defenders
        .iter()
        .filter(|defender| {
            app.world()
                .entity(**defender)
                .get::<DirectMovementComponent>()
                .is_some_and(|movement| world_to_cell(movement.xy) == empty)
        })
        .count();
    assert_eq!(moving_to_empty, 1);
}

#[test]
fn engaged_defenders_do_not_consume_staging_slots() {
    let mut app = common::sim_app();
    let crowded = IVec2::ZERO;
    let empty = IVec2::X;
    for cell in [crowded, empty] {
        app.world_mut().resource_mut::<IntentGrid>().paint_owned(
            cell,
            IntentKind::Defend,
            Some(SwarmId::PLAYER),
        );
    }
    let crowded_center = common::cell_world_center(crowded);
    let staged = [
        common::spawn_defender_at(&mut app, crowded_center + Vec2::new(-20.0, 0.0)),
        common::spawn_defender_at(&mut app, crowded_center),
        common::spawn_defender_at(&mut app, crowded_center + Vec2::new(20.0, 0.0)),
    ];
    let empty_center = common::cell_world_center(empty);
    let hostile = common::spawn_worker_at(&mut app, empty_center + Vec2::new(20.0, 0.0));
    app.world_mut()
        .entity_mut(hostile)
        .insert(SwarmMember::new(SwarmId(1)));
    let responder = common::spawn_defender_at(&mut app, empty_center);
    app.world_mut()
        .entity_mut(responder)
        .insert(DefenderResponse { target: hostile });
    let charging = common::spawn_defender_at(&mut app, empty_center + Vec2::new(-20.0, 0.0));
    app.world_mut().entity_mut(charging).insert((
        ChargerAssignment {
            charger: Entity::PLACEHOLDER,
        },
        DirectMovementComponent {
            xy: empty_center,
            stop_radius: 0.0,
        },
    ));

    app.update();

    let moving_to_empty = staged
        .iter()
        .filter(|defender| {
            app.world()
                .entity(**defender)
                .get::<DirectMovementComponent>()
                .is_some_and(|movement| world_to_cell(movement.xy) == empty)
        })
        .count();
    assert_eq!(
        moving_to_empty, 1,
        "response and Charge duties should not satisfy the empty staging slot",
    );
    assert_eq!(
        app.world()
            .entity(responder)
            .get::<DefenderResponse>()
            .map(|response| response.target),
        Some(hostile),
    );
    assert!(
        app.world()
            .entity(charging)
            .get::<ChargerAssignment>()
            .is_some(),
    );
}

#[test]
fn returning_response_and_charge_defenders_join_the_current_layout() {
    let mut app = common::sim_app();
    let old_west = IVec2::ZERO;
    let old_east = IVec2::X;
    for cell in [old_west, old_east] {
        app.world_mut().resource_mut::<IntentGrid>().paint_owned(
            cell,
            IntentKind::Defend,
            Some(SwarmId::PLAYER),
        );
    }
    let old_west_center = common::cell_world_center(old_west);
    let old_east_center = common::cell_world_center(old_east);
    let responder = common::spawn_defender_at(&mut app, old_west_center);
    let charging = common::spawn_defender_at(&mut app, old_east_center);
    app.update();

    let hostile = common::spawn_worker_at(&mut app, old_east_center + Vec2::new(20.0, 0.0));
    app.world_mut()
        .entity_mut(hostile)
        .insert(SwarmMember::new(SwarmId(1)));
    app.world_mut()
        .entity_mut(responder)
        .insert(DefenderResponse { target: hostile });
    app.world_mut().entity_mut(charging).insert((
        ChargerAssignment {
            charger: Entity::PLACEHOLDER,
        },
        DirectMovementComponent {
            xy: old_east_center,
            stop_radius: 0.0,
        },
    ));
    app.update();

    let new_west = IVec2::Y;
    let new_east = IVec2::ONE;
    {
        let mut grid = app.world_mut().resource_mut::<IntentGrid>();
        grid.remove(old_west, IntentKind::Defend);
        grid.remove(old_east, IntentKind::Defend);
        for cell in [new_west, new_east] {
            grid.paint_owned(cell, IntentKind::Defend, Some(SwarmId::PLAYER));
        }
    }
    let west_incumbent = common::spawn_defender_at(&mut app, common::cell_world_center(new_west));
    let east_incumbent = common::spawn_defender_at(&mut app, common::cell_world_center(new_east));
    app.update();

    app.world_mut().despawn(hostile);
    app.world_mut()
        .entity_mut(charging)
        .remove::<ChargerAssignment>()
        .remove::<DirectMovementComponent>();
    app.update();

    for (defender, expected_cell) in [(responder, new_west), (charging, new_east)] {
        let movement = app
            .world()
            .entity(defender)
            .get::<DirectMovementComponent>()
            .expect("returning duty should travel to the current staging layout");
        assert_eq!(world_to_cell(movement.xy), expected_cell,);
        assert_ne!(world_to_cell(movement.xy), old_west);
        assert_ne!(world_to_cell(movement.xy), old_east);
    }
    for incumbent in [west_incumbent, east_incumbent] {
        assert!(
            app.world()
                .entity(incumbent)
                .get::<DirectMovementComponent>()
                .is_none(),
            "returning duty should not displace a valid incumbent",
        );
    }
}

#[test]
fn unowned_defend_paint_does_not_replace_current_cell_fallback() {
    let mut app = common::sim_app();
    let current = IVec2::ZERO;
    let unowned = IVec2::X;
    app.world_mut()
        .resource_mut::<IntentGrid>()
        .paint(unowned, IntentKind::Defend);
    let defender = common::spawn_defender_at(&mut app, common::cell_world_center(current));

    app.update();

    assert!(
        app.world()
            .entity(defender)
            .get::<DirectMovementComponent>()
            .is_none(),
        "shared unowned paint is neither owned Defend staging nor a Swarm Tile",
    );
}

#[test]
fn lone_defender_roams_continuously_and_replays_deterministically_without_territory() {
    let cell = IVec2::ZERO;
    let mut first_app = common::sim_app();
    let first_defender = common::spawn_defender_at(&mut first_app, common::cell_world_center(cell));
    first_app.update();
    let mut second_app = common::sim_app();
    let second_defender =
        common::spawn_defender_at(&mut second_app, common::cell_world_center(cell));
    second_app.update();

    let mut changed_direction = false;
    let mut previous_delta = None;
    let mut previous_position = common::cell_world_center(cell);
    for index in 0..360 {
        first_app.update();
        second_app.update();
        let position = |app: &App, defender: Entity| {
            app.world()
                .entity(defender)
                .get::<Transform>()
                .unwrap()
                .translation
                .truncate()
        };
        let left = position(&first_app, first_defender);
        let right = position(&second_app, second_defender);
        assert_eq!(world_to_cell(left), cell);
        assert_eq!(world_to_cell(right), cell);
        assert!(
            left.distance(right) <= 1e-5,
            "deterministic replay diverged at fixed step {index}",
        );
        let delta = left - previous_position;
        assert!(
            (delta.length() - 1.5).abs() <= 1e-4,
            "the lone roamer parked or exceeded the gentle scale at step {index}: {delta:?}",
        );
        if previous_delta
            .is_some_and(|previous: Vec2| previous.normalize().dot(delta.normalize()) < 0.99)
        {
            changed_direction = true;
        }
        previous_delta = Some(delta);
        previous_position = left;
    }
    assert!(
        changed_direction,
        "procedural arrival should advance to another subcell destination",
    );
}
