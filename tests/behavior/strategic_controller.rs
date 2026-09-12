#[path = "../common/mod.rs"]
mod common;

use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    intent::{IntentGrid, IntentKind},
    nanobot::{MatchOutcome, Swarm, SwarmId},
    resources::{ResourceKind, ResourceLedger},
    strategic_controller::{Decision, IntentEditAction},
    strategic_runtime::{StrategicController, StrategicControllerPlugin},
};

fn apply_decision(grid: &mut IntentGrid, owner: SwarmId, decision: &Decision) {
    for edit in &decision.edits {
        match edit.action {
            IntentEditAction::Paint => grid.paint(edit.cell, edit.kind, owner),
            IntentEditAction::Erase => grid.erase(edit.cell, edit.kind, owner),
        };
    }
}

#[test]
fn strategic_adapter_applies_only_its_owners_intent_without_mutating_resources() {
    let mut app = common::minimal_app();
    app.add_plugins(StrategicControllerPlugin);
    let owner = SwarmId(7);
    let old = IVec2::new(2, 0);
    let next = IVec2::ZERO;
    app.world_mut().spawn((
        Swarm {},
        owner,
        Transform::default(),
        StrategicController::new(owner),
    ));
    for swarm in [SwarmId::PLAYER, owner] {
        app.world_mut()
            .resource_mut::<IntentGrid>()
            .paint(old, IntentKind::Defend, swarm);
        app.world_mut()
            .resource_mut::<ResourceLedger>()
            .add_for(swarm, ResourceKind::Minerals, 37);
    }
    app.update();
    let grid = app.world().resource::<IntentGrid>();
    assert!(
        grid.cell(next)
            .unwrap()
            .has_owned(IntentKind::Defend, owner)
    );
    assert!(!grid.cell(old).unwrap().has_owned(IntentKind::Defend, owner));
    assert!(
        grid.cell(old)
            .unwrap()
            .has_owned(IntentKind::Defend, SwarmId::PLAYER)
    );
    assert!(
        !grid
            .cell(next)
            .unwrap()
            .has_owned(IntentKind::Defend, SwarmId::PLAYER)
    );
    for swarm in [SwarmId::PLAYER, owner] {
        assert_eq!(
            app.world()
                .resource::<ResourceLedger>()
                .total_for(swarm, ResourceKind::Minerals),
            37
        );
    }
    let revision = grid.revision();
    app.insert_resource(MatchOutcome::Winner(owner));
    for _ in 0..4 {
        app.update();
    }
    assert_eq!(app.world().resource::<IntentGrid>().revision(), revision);
}

#[test]
fn controller_telemetry_counts_invalidation_checks_between_reviews() {
    use top_down_2d_rts_prototype_nano_swarm::strategic_runtime::ControllerTelemetry;

    let mut app = common::minimal_app();
    app.add_plugins(StrategicControllerPlugin);
    let owner = SwarmId(7);
    app.world_mut().spawn((
        Swarm {},
        owner,
        Transform::default(),
        StrategicController::new(owner),
    ));
    common::spawn_deposit(
        &mut app,
        common::DepositFixture {
            world_pos: common::cell_world_center(IVec2::new(-1, -1)),
            amount: 72_000,
            capacity: 72_000,
            radius: 80.0,
        },
    );
    app.update();
    let initial = app.world().resource::<ControllerTelemetry>().profiles()[&owner.0].clone();
    assert_eq!(initial.reviews, 1);
    app.update();
    let next = app.world().resource::<ControllerTelemetry>().profiles()[&owner.0].clone();
    assert_eq!(next.reviews, initial.reviews);
    assert_eq!(next.intent_edits, initial.intent_edits);
    assert!(
        next.work_units > initial.work_units,
        "checking the retained deposit consumes work even without a strategy review"
    );
}

#[test]
fn controller_eventually_erases_stale_owned_intent_after_large_foreign_prefix() {
    use top_down_2d_rts_prototype_nano_swarm::strategic_controller::{
        Controller, GameState, REVIEW_PERIOD_TICKS, SwarmState,
    };

    let owner = SwarmId(4);
    let foreign = SwarmId(9);
    let foreign_sentinel = IVec2::new(-32, -32);
    let stale = IVec2::new(31, 31);
    let mut grid = IntentGrid::new(64, 64);
    for y in -32..-27 {
        for x in -32..32 {
            grid.paint(IVec2::new(x, y), IntentKind::Gather, foreign);
        }
    }
    grid.paint(stale, IntentKind::Defend, owner);

    let swarms = [SwarmState {
        id: owner,
        home: Vec2::ZERO,
        minerals: 0,
    }];
    let mut controller = Controller::new(owner);
    for review in 0..3 {
        let decision = controller.decide(&GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &[],
            structures: &[],
            deposits: &[],
            terrain: &[],
            tick: review * REVIEW_PERIOD_TICKS,
            finished: false,
        });
        apply_decision(&mut grid, owner, &decision);
    }

    assert!(
        !grid
            .cell(stale)
            .unwrap()
            .has_owned(IntentKind::Defend, owner),
        "bounded cleanup must eventually pass foreign cells and erase stale owned intent"
    );
    assert!(
        grid.cell(foreign_sentinel)
            .unwrap()
            .has_owned(IntentKind::Gather, foreign),
        "cleanup must preserve foreign intent"
    );
}

#[test]
fn controller_retargets_when_its_primary_resource_is_exhausted() {
    use top_down_2d_rts_prototype_nano_swarm::strategic_controller::{
        Controller, DepositState, GameState, SwarmState,
    };

    let mut grid = IntentGrid::new(32, 32);
    let owner = SwarmId(4);
    let world = |cell: IVec2| {
        (cell.as_vec2() + Vec2::splat(0.5)) * top_down_2d_rts_prototype_nano_swarm::ZONE_BLOCK_SIZE
    };
    let swarms = [SwarmState {
        id: owner,
        home: world(IVec2::ZERO),
        minerals: 0,
    }];
    let deposits = [
        DepositState {
            id: 20,
            position: world(IVec2::new(2, 0)),
            amount: 1_000,
            radius: 100.0,
        },
        DepositState {
            id: 21,
            position: world(IVec2::new(4, 0)),
            amount: 50,
            radius: 100.0,
        },
    ];
    let mut controller = Controller::new(owner);
    let initial = controller.decide(&GameState {
        grid: &grid,
        swarms: &swarms,
        bots: &[],
        structures: &[],
        deposits: &deposits,
        terrain: &[],
        tick: 0,
        finished: false,
    });
    apply_decision(&mut grid, owner, &initial);

    let depleted = [
        DepositState {
            amount: 0,
            ..deposits[0]
        },
        deposits[1],
    ];
    let urgent = controller.decide(&GameState {
        grid: &grid,
        swarms: &swarms,
        bots: &[],
        structures: &[],
        deposits: &depleted,
        terrain: &[],
        tick: 1,
        finished: false,
    });

    assert!(urgent.reviewed, "depletion must bypass the regular cadence");
    assert!(urgent.explanation.contains("exhausted"));
    assert!(urgent.edits.iter().any(|edit| {
        edit.action == IntentEditAction::Paint
            && edit.kind == IntentKind::Gather
            && edit.cell == IVec2::new(4, 0)
    }));
    assert!(urgent.edits.iter().any(|edit| {
        edit.action == IntentEditAction::Erase
            && edit.kind == IntentKind::Gather
            && edit.cell == IVec2::new(2, 0)
    }));
}

#[test]
fn controller_urgently_replans_when_key_support_is_lost() {
    use top_down_2d_rts_prototype_nano_swarm::strategic_controller::{
        Controller, DepositState, GameState, StructureKind, StructureState, SwarmState,
    };

    let mut grid = IntentGrid::new(32, 32);
    let owner = SwarmId(4);
    let world = |cell: IVec2| {
        (cell.as_vec2() + Vec2::splat(0.5)) * top_down_2d_rts_prototype_nano_swarm::ZONE_BLOCK_SIZE
    };
    let swarms = [SwarmState {
        id: owner,
        home: world(IVec2::ZERO),
        minerals: 20,
    }];
    let deposits = [DepositState {
        id: 20,
        position: world(IVec2::new(2, 0)),
        amount: 500,
        radius: 100.0,
    }];
    let charger = [StructureState {
        id: 42,
        owner,
        position: world(IVec2::new(1, 0)),
        kind: StructureKind::Charger,
        health: 100,
        minerals: 25,
    }];
    let mut controller = Controller::new(owner);
    let initial = controller.decide(&GameState {
        grid: &grid,
        swarms: &swarms,
        bots: &[],
        structures: &charger,
        deposits: &deposits,
        terrain: &[],
        tick: 0,
        finished: false,
    });
    apply_decision(&mut grid, owner, &initial);

    let urgent = controller.decide(&GameState {
        grid: &grid,
        swarms: &swarms,
        bots: &[],
        structures: &[],
        deposits: &deposits,
        terrain: &[],
        tick: 1,
        finished: false,
    });

    assert!(urgent.reviewed);
    assert!(urgent.explanation.contains("key Charger 42 was lost"));
}

#[test]
fn regular_and_urgent_reviews_share_one_public_work_allowance() {
    use top_down_2d_rts_prototype_nano_swarm::{
        nanobot::NanobotType,
        strategic_controller::{
            BotState, Controller, DepositState, GameState, PLANNING_WORK_BUDGET, StructureKind,
            StructureState, SwarmState,
        },
    };

    let mut grid = IntentGrid::new(64, 64);
    assert_eq!(PLANNING_WORK_BUDGET, 100_000);
    let owner = SwarmId(4);
    let enemy = SwarmId(9);
    let world = |cell: IVec2| {
        (cell.as_vec2() + Vec2::splat(0.5)) * top_down_2d_rts_prototype_nano_swarm::ZONE_BLOCK_SIZE
    };
    let swarms = [
        SwarmState {
            id: owner,
            home: world(IVec2::ZERO),
            minerals: 100,
        },
        SwarmState {
            id: enemy,
            home: world(IVec2::new(24, 24)),
            minerals: 0,
        },
    ];
    let deposit = [DepositState {
        id: 20,
        position: world(IVec2::new(-1, 0)),
        amount: 500,
        radius: 100.0,
    }];
    let remnant = BotState {
        id: 50,
        owner: enemy,
        kind: NanobotType::Worker,
        position: world(IVec2::new(12, 3)),
        health: 20,
        charge: 1.0,
        cargo: 0,
    };
    let charger = StructureState {
        id: 42,
        owner,
        position: world(IVec2::new(1, 0)),
        kind: StructureKind::Charger,
        health: 100,
        minerals: 25,
    };
    let mut controller = Controller::new(owner);
    let initial = controller.decide(&GameState {
        grid: &grid,
        swarms: &swarms,
        bots: &[remnant],
        structures: &[charger],
        deposits: &deposit,
        terrain: &[],
        tick: 0,
        finished: false,
    });
    apply_decision(&mut grid, owner, &initial);

    let crowded_structures = (0..60_000)
        .map(|id| StructureState {
            id: 1_000 + id,
            owner,
            position: swarms[0].home,
            kind: StructureKind::Planned,
            health: 100,
            minerals: 0,
        })
        .collect::<Vec<_>>();
    let support_loss = controller.decide(&GameState {
        grid: &grid,
        swarms: &swarms,
        bots: &[remnant],
        structures: &crowded_structures,
        deposits: &deposit,
        terrain: &[],
        tick: 1,
        finished: false,
    });
    apply_decision(&mut grid, owner, &support_loss);

    let moved = BotState {
        position: world(IVec2::new(13, 3)),
        ..remnant
    };
    let mut crowded_bots = (0..60_000)
        .map(|id| BotState {
            id: 1_000 + id,
            owner,
            kind: NanobotType::Worker,
            position: swarms[0].home,
            health: 100,
            charge: 1.0,
            cargo: 0,
        })
        .collect::<Vec<_>>();
    crowded_bots.push(moved);
    let target_move = controller.decide(&GameState {
        grid: &grid,
        swarms: &swarms,
        bots: &crowded_bots,
        structures: &[],
        deposits: &deposit,
        terrain: &[],
        tick: 2,
        finished: false,
    });

    assert!(support_loss.reviewed);
    assert!(
        initial.work_units + support_loss.work_units + target_move.work_units
            <= PLANNING_WORK_BUDGET
    );
    assert!(!target_move.reviewed);
    assert!(target_move.edits.is_empty());
}

#[test]
fn mature_economy_keeps_one_active_resource_site() {
    use top_down_2d_rts_prototype_nano_swarm::{
        nanobot::NanobotType,
        strategic_controller::{
            BotState, Controller, DepositState, GameState, StructureKind, StructureState,
            SwarmState,
        },
    };

    let mut grid = IntentGrid::new(32, 32);
    let owner = SwarmId(4);
    let world = |cell: IVec2| {
        (cell.as_vec2() + Vec2::splat(0.5)) * top_down_2d_rts_prototype_nano_swarm::ZONE_BLOCK_SIZE
    };
    let home = world(IVec2::ZERO);
    grid.paint(IVec2::new(0, 7), IntentKind::Gather, owner);
    let swarms = [SwarmState {
        id: owner,
        home,
        minerals: 100,
    }];
    let bots = (0..4)
        .map(|id| BotState {
            id,
            owner,
            kind: NanobotType::Worker,
            position: home,
            health: 100,
            charge: 1.0,
            cargo: 0,
        })
        .chain((4..8).map(|id| BotState {
            id,
            owner,
            kind: NanobotType::Hauler,
            position: home,
            health: 100,
            charge: 1.0,
            cargo: 0,
        }))
        .collect::<Vec<_>>();
    let deposits = [
        DepositState {
            id: 20,
            position: world(IVec2::new(-1, 0)),
            amount: 72_000,
            radius: 64.0,
        },
        DepositState {
            id: 21,
            position: world(IVec2::new(4, 0)),
            amount: 72_000,
            radius: 64.0,
        },
        DepositState {
            id: 22,
            position: world(IVec2::new(0, 7)),
            amount: 72_000,
            radius: 64.0,
        },
    ];
    let structures = [
        StructureState {
            id: 30,
            owner,
            position: deposits[0].position + Vec2::new(96.0, 0.0),
            kind: StructureKind::Source,
            health: 100,
            minerals: 40,
        },
        StructureState {
            id: 31,
            owner,
            position: home,
            kind: StructureKind::Facility,
            health: 100,
            minerals: 20,
        },
    ];

    let decision = Controller::new(owner).decide(&GameState {
        grid: &grid,
        swarms: &swarms,
        bots: &bots,
        structures: &structures,
        deposits: &deposits,
        terrain: &[],
        tick: 0,
        finished: false,
    });
    let mut painted_gather = decision
        .edits
        .iter()
        .filter(|edit| edit.action == IntentEditAction::Paint && edit.kind == IntentKind::Gather)
        .map(|edit| edit.cell)
        .collect::<Vec<_>>();
    painted_gather.sort_by_key(|cell| (cell.y, cell.x));

    assert_eq!(painted_gather, vec![IVec2::new(-1, 0)]);
    assert!(decision.edits.iter().any(|edit| {
        edit.action == IntentEditAction::Erase
            && edit.kind == IntentKind::Gather
            && edit.cell == IVec2::new(0, 7)
    }));
}

#[test]
fn controller_primary_site_extracts_and_hauls_under_normal_pacing() {
    use top_down_2d_rts_prototype_nano_swarm::{
        gameplay_pacing::GameplayPacing,
        nanobot::{Cargo, HaulerAssignment, NanobotType, OwnerSwarm, world_to_cell},
        resources::ResourceDeposit,
    };

    let mut app = common::sim_app_with_gather_haul();
    app.add_plugins(StrategicControllerPlugin);
    let home = common::cell_world_center(IVec2::ZERO);
    let own = common::spawn_swarm_with_nanobots(
        &mut app,
        home,
        &[(NanobotType::Worker, 4), (NanobotType::Hauler, 2)],
    );
    app.world_mut()
        .entity_mut(own)
        .insert(StrategicController::new(SwarmId::PLAYER));
    let positions =
        [IVec2::new(-1, 0), IVec2::new(4, 0), IVec2::new(0, 7)].map(common::cell_world_center);
    let deposits = positions.map(|world_pos| {
        common::spawn_deposit(
            &mut app,
            common::DepositFixture {
                world_pos,
                amount: 72_000,
                capacity: 72_000,
                radius: 80.0,
            },
        )
    });
    let source = common::spawn_stockpile(&mut app, positions[0] - Vec2::X * 100.0, 0, 200);
    let sink = common::spawn_sink_stockpile(&mut app, home + Vec2::X * 180.0, 0, 200);
    for entity in [source, sink] {
        app.world_mut().entity_mut(entity).insert(OwnerSwarm(own));
    }

    let mut observed_loaded_primary_haul = false;
    for _ in 0..1800 {
        app.update();
        assert_eq!(
            *app.world().resource::<GameplayPacing>(),
            GameplayPacing::default(),
            "the flow must run with normal gameplay pacing"
        );
        let grid = app.world().resource::<IntentGrid>();
        let active = deposits
            .iter()
            .copied()
            .zip(positions)
            .filter(|(_, position)| {
                grid.cell(world_to_cell(*position))
                    .is_some_and(|cell| cell.has_owned(IntentKind::Gather, SwarmId::PLAYER))
            })
            .map(|(deposit, _)| deposit)
            .collect::<Vec<_>>();
        assert!(
            active.len() <= 1,
            "the novice policy must keep at most one active resource site"
        );
        if active == vec![deposits[0]] {
            let world = app.world_mut();
            observed_loaded_primary_haul |= world
                .query::<(&HaulerAssignment, &Cargo)>()
                .iter(world)
                .any(|(assignment, cargo)| assignment.source == source && cargo.amount > 0);
            if observed_loaded_primary_haul
                && world.get::<ResourceDeposit>(deposits[0]).unwrap().amount < 72_000
            {
                assert_eq!(
                    world.get::<ResourceDeposit>(deposits[1]).unwrap().amount,
                    72_000
                );
                assert_eq!(
                    world.get::<ResourceDeposit>(deposits[2]).unwrap().amount,
                    72_000
                );
                return;
            }
        }
    }
    panic!("the single primary site did not produce extraction and a loaded physical haul");
}

#[test]
fn controller_new_front_keeps_the_progressing_fronts_shared_defender_response() {
    use top_down_2d_rts_prototype_nano_swarm::nanobot::{
        DefenderResponse, OwnerSwarm, PlannedKind, Structure, StructureKind,
    };

    let mut app = common::sim_app();
    app.insert_resource(IntentGrid::new(64, 64));
    app.add_plugins(StrategicControllerPlugin);
    let home = common::cell_world_center(IVec2::ZERO);
    let own = common::spawn_swarm_at(&mut app, home);
    app.world_mut()
        .entity_mut(own)
        .insert(StrategicController::new(SwarmId::PLAYER));
    let enemy_cell = IVec2::new(12, 0);
    let enemy = app
        .world_mut()
        .spawn((
            Swarm {},
            SwarmId(9),
            Transform::from_translation(common::cell_world_center(enemy_cell).extend(0.0)),
        ))
        .id();
    app.world_mut().resource_mut::<ResourceLedger>().add_for(
        SwarmId::PLAYER,
        ResourceKind::Minerals,
        2_000,
    );
    let support = common::spawn_charger(
        &mut app,
        common::ChargerFixture {
            cell: IVec2::ZERO,
            amount: 100,
            ticks_since_maintained: 0,
        },
    );
    app.world_mut().entity_mut(support).insert(OwnerSwarm(own));
    let old_front =
        common::spawn_empty_completed_structure(&mut app, enemy, PlannedKind::SinkStockpile);
    app.world_mut().entity_mut(old_front).insert((
        Structure::new(StructureKind::Basic),
        Transform::from_translation(common::cell_world_center(enemy_cell).extend(0.0)),
    ));
    let defenders = (0..8)
        .map(|index| {
            common::spawn_defender_at(
                &mut app,
                common::cell_world_center(IVec2::new(9, 0)) + Vec2::new(0.0, index as f32 * 80.0),
            )
        })
        .collect::<Vec<_>>();
    let mut original_responder = None;
    for _ in 0..5 {
        app.update();
        original_responder = defenders.iter().copied().find(|defender| {
            app.world()
                .get::<DefenderResponse>(*defender)
                .is_some_and(|response| response.target == old_front)
        });
        if original_responder.is_some() {
            break;
        }
    }
    let original_responder = original_responder.unwrap_or_else(|| {
        panic!(
            "the existing front must have a real shared-autonomy response; profiles={:?}",
            app.world()
                .resource::<top_down_2d_rts_prototype_nano_swarm::strategic_runtime::ControllerTelemetry>()
                .profiles()
        )
    });

    app.world_mut()
        .get_mut::<Structure>(old_front)
        .unwrap()
        .health = 80;
    let new_front = common::spawn_charger(
        &mut app,
        common::ChargerFixture {
            cell: IVec2::new(3, 5),
            amount: 100,
            ticks_since_maintained: 0,
        },
    );
    app.world_mut()
        .entity_mut(new_front)
        .insert(OwnerSwarm(enemy));
    app.world_mut()
        .get_mut::<Structure>(new_front)
        .unwrap()
        .health = 20;
    for _ in 0..5 {
        app.update();
    }

    assert!(
        defenders.iter().any(|defender| {
            app.world()
                .get::<DefenderResponse>(*defender)
                .is_some_and(|response| response.target == new_front)
        }),
        "the new front should receive an additional shared-autonomy response"
    );
    assert_eq!(
        app.world()
            .get::<DefenderResponse>(original_responder)
            .map(|response| response.target),
        Some(old_front),
        "adding a supportable front must not cancel the progressing front's response"
    );
}
