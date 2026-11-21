static DEFAULT_CHARGE: std::sync::LazyLock<ChargeTuning> =
    std::sync::LazyLock::new(ChargeTuning::default);

use super::*;

fn direct_attack_tuning() -> AutonomyTuning {
    AutonomyTuning {
        strategy_staging_spacing_cells: 64,
        ..AutonomyTuning::default()
    }
}

fn world(cell: IVec2) -> Vec2 {
    (cell.as_vec2() + Vec2::splat(0.5)) * crate::ZONE_BLOCK_SIZE
}

fn apply(grid: &mut IntentGrid, owner: SwarmId, decision: &Decision) {
    for edit in &decision.edits {
        if edit.action == IntentEditAction::Paint {
            grid.paint(edit.cell, edit.kind, owner);
        } else {
            grid.erase(edit.cell, edit.kind, owner);
        }
    }
}

fn empty_state<'a>(grid: &'a IntentGrid, tick: u64) -> GameState<'a> {
    GameState {
        grid,
        swarms: &[],
        bots: &[],
        structures: &[],
        deposits: &[],
        terrain: &[],
        charge: &DEFAULT_CHARGE,
        reachability: None,
        route_query: None,
        tick,
        finished: false,
    }
}

fn corridor_paint(decision: &Decision) -> BTreeSet<(i32, i32)> {
    decision
        .edits
        .iter()
        .filter(|edit| edit.kind == IntentKind::Corridor && edit.action == IntentEditAction::Paint)
        .map(|edit| (edit.cell.x, edit.cell.y))
        .collect()
}

#[test]
fn corridor_paint_follows_the_supplied_deposit_route() {
    let owner = SwarmId::PLAYER;
    let grid = IntentGrid::new(16, 16);
    let swarms = [SwarmState {
        id: owner,
        home: world(IVec2::ZERO),
        minerals: 1_000,
        mineral_income_per_second: 0.0,
        production_saturated: false,
    }];
    let deposits = [DepositState {
        id: 1,
        owner: None,
        position: world(IVec2::new(4, 0)),
        amount: 72_000,
        radius: 80.0,
    }];
    let route = |_, goal, _: &mut dyn FnMut() -> bool| {
        Some(match goal {
            RouteGoal::Interaction(_) => RouteOutcome::Found(crate::navigation::Route {
                waypoints: [(0, 2), (4, 2), (4, 0)]
                    .map(|(x, y)| world(IVec2::new(x, y)))
                    .to_vec(),
                cost: 8.0 * crate::ZONE_BLOCK_SIZE,
            }),
            _ => RouteOutcome::Unreachable,
        })
    };

    let decision = Controller::new(owner).decide(
        &AutonomyTuning::default(),
        &GameState {
            swarms: &swarms,
            deposits: &deposits,
            route_query: Some(&route),
            ..empty_state(&grid, 0)
        },
    );

    assert_eq!(
        corridor_paint(&decision),
        BTreeSet::from([
            (0, 0),
            (0, 1),
            (0, 2),
            (1, 2),
            (2, 2),
            (3, 2),
            (4, 2),
            (4, 1),
            (4, 0),
        ])
    );
}

#[test]
fn corridor_paint_without_routing_follows_a_straight_cell_line() {
    let owner = SwarmId::PLAYER;
    let grid = IntentGrid::new(16, 16);
    let swarms = [SwarmState {
        id: owner,
        home: world(IVec2::ZERO),
        minerals: 1_000,
        mineral_income_per_second: 0.0,
        production_saturated: false,
    }];
    let deposits = [DepositState {
        id: 1,
        owner: None,
        position: world(IVec2::new(6, 2)),
        amount: 72_000,
        radius: 80.0,
    }];
    let decision = Controller::new(owner).decide(
        &AutonomyTuning::default(),
        &GameState {
            swarms: &swarms,
            deposits: &deposits,
            ..empty_state(&grid, 0)
        },
    );

    assert_eq!(
        corridor_paint(&decision),
        BTreeSet::from([(0, 0), (1, 0), (2, 1), (3, 1), (4, 1), (5, 2), (6, 2)])
    );
}

#[test]
fn a_neutral_deposit_is_not_claimed_when_no_corridor_reaches_it() {
    let owner = SwarmId::PLAYER;
    let grid = IntentGrid::new(16, 16);
    let swarms = [SwarmState {
        id: owner,
        home: world(IVec2::ZERO),
        minerals: 1_000,
        mineral_income_per_second: 0.0,
        production_saturated: false,
    }];
    let deposits = [DepositState {
        id: 1,
        owner: None,
        position: world(IVec2::new(4, 0)),
        amount: 72_000,
        radius: 80.0,
    }];
    let terrain = [Obstacle::Circle {
        center: world(IVec2::new(2, 0)),
        radius: 260.0,
    }];
    let decision = Controller::new(owner).decide(
        &AutonomyTuning::default(),
        &GameState {
            swarms: &swarms,
            deposits: &deposits,
            terrain: &terrain,
            charge: &DEFAULT_CHARGE,
            ..empty_state(&grid, 0)
        },
    );

    assert!(!decision.edits.contains(&IntentEdit {
        cell: IVec2::new(4, 0),
        kind: IntentKind::Gather,
        action: IntentEditAction::Paint,
    }));
    assert_eq!(corridor_paint(&decision), BTreeSet::from([(0, 0)]));
}

#[test]
fn corridor_search_spends_planning_work_and_keeps_existing_paint_on_exhaustion() {
    let owner = SwarmId::PLAYER;
    let mut grid = IntentGrid::new(16, 16);
    let swarms = [SwarmState {
        id: owner,
        home: world(IVec2::ZERO),
        minerals: 1_000,
        mineral_income_per_second: 0.0,
        production_saturated: false,
    }];
    let deposit = DepositState {
        id: 1,
        owner: None,
        position: world(IVec2::new(4, 0)),
        amount: 72_000,
        radius: 80.0,
    };
    let tuning = AutonomyTuning::default();
    let mut controller = Controller::new(owner);
    let first = controller.decide(
        &tuning,
        &GameState {
            swarms: &swarms,
            deposits: &[deposit],
            ..empty_state(&grid, 0)
        },
    );
    apply(&mut grid, owner, &first);
    let next_deposit = DepositState {
        position: world(IVec2::new(0, 4)),
        ..deposit
    };
    let route = |_, _, allow_work: &mut dyn FnMut() -> bool| {
        for _ in 0..PLANNING_WORK_BUDGET {
            if !allow_work() {
                return None;
            }
        }
        panic!("route search exceeded the entire planning allowance")
    };
    let next = controller.decide(
        &tuning,
        &GameState {
            swarms: &swarms,
            deposits: &[next_deposit],
            route_query: Some(&route),
            ..empty_state(&grid, tuning.controller_review_period_ticks)
        },
    );

    assert!(next.explanation.contains("planning allowance exhausted"));
    assert!(next.edits.is_empty());
    assert!(next.work_units > first.work_units);
    assert!(next.work_units <= PLANNING_WORK_BUDGET);
}

#[test]
fn expansion_rejects_a_deposit_whose_corridor_exceeds_paint_capacity() {
    let owner = SwarmId::PLAYER;
    let grid = IntentGrid::new(220, 220);
    let swarms = [SwarmState {
        id: owner,
        home: world(IVec2::ZERO),
        minerals: 1_000,
        mineral_income_per_second: 0.0,
        production_saturated: false,
    }];
    let deposit_cell = IVec2::new(98, 0);
    let deposits = [DepositState {
        id: 1,
        owner: None,
        position: world(deposit_cell),
        amount: 72_000,
        radius: 80.0,
    }];
    let tuning = AutonomyTuning {
        strategy_expansion_travel_cost: 0.0,
        strategy_expansion_haul_cost: 0.0,
        ..Default::default()
    };
    let decision = Controller::new(owner).decide(
        &tuning,
        &GameState {
            swarms: &swarms,
            deposits: &deposits,
            ..empty_state(&grid, 0)
        },
    );

    for (cell, kind) in [
        (IVec2::ZERO, IntentKind::Build),
        (IVec2::ZERO, IntentKind::Defend),
    ] {
        assert!(decision.edits.contains(&IntentEdit {
            cell,
            kind,
            action: IntentEditAction::Paint,
        }));
    }
    assert!(
        !decision
            .edits
            .iter()
            .any(|edit| edit.kind == IntentKind::Gather)
    );
    assert_eq!(corridor_paint(&decision), BTreeSet::from([(0, 0)]));
}

#[test]
fn changed_plan_erases_superseded_paint_beyond_the_cleanup_cursor() {
    let mut grid = IntentGrid::new(64, 64);
    let owner = SwarmId::PLAYER;
    let foreign = SwarmId(9);
    for y in -32..-16 {
        for x in -32..32 {
            grid.paint(IVec2::new(x, y), IntentKind::Corridor, foreign);
        }
    }
    let swarms = [SwarmState {
        id: owner,
        home: world(IVec2::ZERO),
        minerals: 100,
        mineral_income_per_second: 0.0,
        production_saturated: false,
    }];
    let target = BotState {
        id: 42,
        owner: foreign,
        kind: NanobotType::Hauler,
        position: world(IVec2::new(12, 5)),
        health: 40,
        max_health: 100,
        charge: 1.0,
        max_charge: 1.0,
        cargo: 0,
    };
    let deposit = DepositState {
        id: 20,
        owner: None,
        position: world(IVec2::new(0, 8)),
        amount: 72_000,
        radius: 80.0,
    };
    let tuning = AutonomyTuning::default();
    let mut controller = Controller::new(owner);
    let initial = controller.decide(
        &tuning,
        &GameState {
            swarms: &swarms,
            bots: &[target],
            deposits: &[deposit],
            ..empty_state(&grid, 0)
        },
    );
    apply(&mut grid, owner, &initial);

    let empty_grid = IntentGrid::new(64, 64);
    let home_plan = Controller::new(owner).decide(
        &tuning,
        &GameState {
            swarms: &swarms,
            ..empty_state(&empty_grid, 0)
        },
    );
    let superseded = initial
        .edits
        .iter()
        .filter(|intent| !home_plan.edits.contains(intent))
        .collect::<Vec<_>>();
    assert!(
        superseded
            .iter()
            .any(|intent| intent.kind == IntentKind::Defend)
    );
    assert!(
        superseded
            .iter()
            .any(|intent| intent.kind == IntentKind::Corridor)
    );

    let changed = controller.decide(
        &tuning,
        &GameState {
            swarms: &swarms,
            ..empty_state(&grid, WORK_WINDOW_TICKS)
        },
    );
    assert!(changed.reviewed);
    assert!(changed.work_units <= PLANNING_WORK_BUDGET);
    for intent in superseded {
        assert!(
            changed.edits.contains(&IntentEdit {
                action: IntentEditAction::Erase,
                ..*intent
            }),
            "plan replacement left stale paint: {intent:?}"
        );
    }
}

#[test]
fn controller_erases_legacy_intent_and_never_paints_across_permanent_terrain() {
    use crate::{navigation::Obstacle, reachability::SwarmReachabilityMap};

    let mut grid = IntentGrid::new(8, 8);
    let owner = SwarmId::PLAYER;
    let blocked = IVec2::new(1, 0);
    grid.paint(blocked, IntentKind::Defend, owner);
    let wall = Obstacle::Rectangle {
        center: Vec2::ZERO,
        half: Vec2::new(36.0, 8_000.0),
    };
    let reachability =
        SwarmReachabilityMap::new(&grid, [wall], [(owner, Vec2::new(-252.0, 36.0))]).unwrap();
    let swarms = [SwarmState {
        id: owner,
        home: Vec2::new(-252.0, 36.0),
        minerals: 0,
        mineral_income_per_second: 0.0,
        production_saturated: false,
    }];
    let terrain = [wall];
    let state = GameState {
        grid: &grid,
        swarms: &swarms,
        bots: &[],
        structures: &[],
        deposits: &[],
        terrain: &terrain,
        charge: &DEFAULT_CHARGE,
        reachability: Some(&reachability),
        route_query: None,
        tick: 0,
        finished: false,
    };

    let decision = Controller::new(owner).decide(&crate::tuning::AutonomyTuning::default(), &state);

    assert!(decision.edits.contains(&IntentEdit {
        cell: blocked,
        kind: IntentKind::Defend,
        action: IntentEditAction::Erase,
    }));
    assert!(decision.edits.iter().all(|edit| {
        edit.action == IntentEditAction::Erase || reachability.contains(owner, edit.cell, &grid)
    }));
}

#[test]
fn controller_stops_hunting_a_target_on_the_isolated_side_of_a_painted_tile() {
    let owner = SwarmId::PLAYER;
    let home = Vec2::new(-252.0, 36.0);
    let mut grid = IntentGrid::new(8, 8);
    let terrain = [Obstacle::Rectangle {
        center: Vec2::new(256.0, 0.0),
        half: Vec2::new(36.0, 8_000.0),
    }];
    let reachability = SwarmReachabilityMap::new(&grid, terrain, [(owner, home)]).unwrap();
    let swarms = [SwarmState {
        id: owner,
        home,
        minerals: 0,
        mineral_income_per_second: 0.0,
        production_saturated: false,
    }];
    let target = BotState {
        id: 1,
        owner: SwarmId(1),
        kind: NanobotType::Hauler,
        position: Vec2::new(128.0, 256.0),
        health: 100,
        max_health: 100,
        charge: 0.0,
        max_charge: 1.0,
        cargo: 0,
    };
    let tuning = direct_attack_tuning();
    let mut controller = Controller::new(owner);
    let initial = controller.decide(
        &tuning,
        &GameState {
            swarms: &swarms,
            bots: &[target],
            terrain: &terrain,
            charge: &DEFAULT_CHARGE,
            reachability: Some(&reachability),
            route_query: None,
            ..empty_state(&grid, 0)
        },
    );
    let hunt = Objective::HuntRemnant(target.id).label();
    assert!(initial.explanation.contains(&hunt));
    apply(&mut grid, owner, &initial);
    assert!(
        grid.cell(IVec2::ZERO)
            .unwrap()
            .has_owned(IntentKind::Defend, owner)
    );

    let moved = BotState {
        position: Vec2::new(416.0, 256.0),
        ..target
    };
    let revised = controller.decide(
        &tuning,
        &GameState {
            swarms: &swarms,
            bots: &[moved],
            terrain: &terrain,
            charge: &DEFAULT_CHARGE,
            reachability: Some(&reachability),
            route_query: None,
            ..empty_state(&grid, 1)
        },
    );
    assert!(revised.reviewed);
    assert!(!revised.explanation.contains(&hunt));
}

fn mature_economy_bots(owner: SwarmId, home: Vec2) -> Vec<BotState> {
    (0..4)
        .map(|id| BotState {
            id,
            owner,
            kind: NanobotType::Worker,
            position: home,
            health: 100,
            max_health: 100,
            charge: 1.0,
            max_charge: 1.0,
            cargo: 0,
        })
        .chain((4..8).map(|id| BotState {
            id,
            owner,
            kind: NanobotType::Hauler,
            position: home,
            health: 100,
            max_health: 100,
            charge: 1.0,
            max_charge: 1.0,
            cargo: 0,
        }))
        .collect()
}

fn funded_primary_chain(owner: SwarmId, home: Vec2, primary: DepositState) -> [StructureState; 2] {
    [
        StructureState {
            id: 30,
            owner,
            position: primary.position + Vec2::new(96.0, 0.0),
            kind: StructureKind::Source,
            health: 100,
            max_health: 100,
            minerals: 40,
        },
        StructureState {
            id: 31,
            owner,
            position: home,
            kind: StructureKind::Facility,
            health: 100,
            max_health: 100,
            minerals: 20,
        },
    ]
}

#[test]
fn controller_coordinates_economy_support_and_combat_on_first_review() {
    let grid = IntentGrid::new(32, 32);
    let owner = SwarmId(4);
    let enemy = SwarmId(9);
    let swarms = [
        SwarmState {
            id: owner,
            home: world(IVec2::ZERO),
            minerals: 40,
            mineral_income_per_second: 0.0,
            production_saturated: false,
        },
        SwarmState {
            id: enemy,
            home: world(IVec2::new(8, 0)),
            minerals: 20,
            mineral_income_per_second: 0.0,
            production_saturated: false,
        },
    ];
    let bots = [
        BotState {
            id: 1,
            owner,
            kind: NanobotType::Worker,
            position: swarms[0].home,
            health: 100,
            max_health: 100,
            charge: 1.0,
            max_charge: 1.0,
            cargo: 0,
        },
        BotState {
            id: 2,
            owner,
            kind: NanobotType::Hauler,
            position: swarms[0].home,
            health: 100,
            max_health: 100,
            charge: 1.0,
            max_charge: 1.0,
            cargo: 0,
        },
        BotState {
            id: 3,
            owner,
            kind: NanobotType::Defender,
            position: swarms[0].home,
            health: 100,
            max_health: 100,
            charge: 0.3,
            max_charge: 1.0,
            cargo: 0,
        },
    ];
    let structures = [
        StructureState {
            id: 10,
            owner,
            position: swarms[0].home,
            kind: StructureKind::Facility,
            health: 100,
            max_health: 100,
            minerals: 0,
        },
        StructureState {
            id: 11,
            owner: enemy,
            position: world(IVec2::new(7, 0)),
            kind: StructureKind::Sink,
            health: 100,
            max_health: 100,
            minerals: 80,
        },
    ];
    let deposits = [DepositState {
        id: 20,
        owner: None,
        position: world(IVec2::new(2, 0)),
        amount: 500,
        radius: 100.0,
    }];
    let state = GameState {
        grid: &grid,
        swarms: &swarms,
        bots: &bots,
        structures: &structures,
        deposits: &deposits,
        terrain: &[],
        charge: &DEFAULT_CHARGE,
        reachability: None,
        route_query: None,
        tick: 0,
        finished: false,
    };

    let decision = Controller::new(owner).decide(&direct_attack_tuning(), &state);

    assert!(decision.reviewed);
    assert!(decision.work_units <= PLANNING_WORK_BUDGET);
    assert!(
        decision
            .edits
            .iter()
            .all(|edit| edit.action == IntentEditAction::Paint)
    );
    assert!(decision.edits.iter().all(|edit| grid.in_bounds(edit.cell)));
    assert!(
        decision
            .edits
            .iter()
            .any(|edit| { edit.kind == IntentKind::Gather && edit.cell == IVec2::new(2, 0) })
    );
    for kind in IntentKind::ALL {
        assert!(
            decision.edits.iter().any(|edit| edit.kind == kind),
            "coordinated plan omitted {kind:?}"
        );
    }
    let defend_tiles = decision
        .edits
        .iter()
        .filter(|edit| edit.kind == IntentKind::Defend)
        .count();
    assert!(
        defend_tiles >= 4,
        "plan needs broad enough territory to create Defender demand"
    );
}

#[test]
fn profitable_neutral_expansion_preempts_a_viable_raid() {
    let mut grid = IntentGrid::new(32, 32);
    let owner = SwarmId(4);
    let enemy = SwarmId(9);
    let home = world(IVec2::ZERO);
    let initial_swarms = [SwarmState {
        id: owner,
        home,
        minerals: 0,
        mineral_income_per_second: 0.0,
        production_saturated: false,
    }];
    let economy_bots = mature_economy_bots(owner, home);
    let deposits = [
        DepositState {
            id: 20,
            owner: None,
            position: world(IVec2::new(-1, 0)),
            amount: 72_000,
            radius: 64.0,
        },
        DepositState {
            id: 21,
            owner: None,
            position: world(IVec2::new(4, 0)),
            amount: 72_000,
            radius: 64.0,
        },
    ];
    let primary_chain = funded_primary_chain(owner, home, deposits[0]);
    let mut controller = Controller::new(owner);
    let initial = controller.decide(
        &direct_attack_tuning(),
        &GameState {
            grid: &grid,
            swarms: &initial_swarms,
            bots: &economy_bots,
            structures: &primary_chain,
            deposits: &deposits,
            terrain: &[],
            charge: &DEFAULT_CHARGE,
            reachability: None,
            route_query: None,
            tick: 0,
            finished: false,
        },
    );
    apply(&mut grid, owner, &initial);
    assert!(
        grid.cell(IVec2::new(-1, 0))
            .is_some_and(|intent| intent.has_owned(IntentKind::Gather, owner))
    );
    assert!(
        !grid
            .cell(IVec2::new(4, 0))
            .is_some_and(|intent| intent.has_owned(IntentKind::Gather, owner))
    );

    let supplied_swarms = [
        SwarmState {
            minerals: 2_000,
            mineral_income_per_second: 0.0,
            production_saturated: false,
            ..initial_swarms[0]
        },
        SwarmState {
            id: enemy,
            home: world(IVec2::new(8, 0)),
            minerals: 0,
            mineral_income_per_second: 0.0,
            production_saturated: false,
        },
    ];
    let defenders = (0..8).map(|id| BotState {
        id: 100 + id,
        owner,
        kind: NanobotType::Defender,
        position: home,
        health: 100,
        max_health: 100,
        charge: 1.0,
        max_charge: 1.0,
        cargo: 0,
    });
    let bots = economy_bots
        .into_iter()
        .chain(defenders)
        .collect::<Vec<_>>();
    let enemy_sink = StructureState {
        id: 40,
        owner: enemy,
        position: supplied_swarms[1].home,
        kind: StructureKind::Sink,
        health: 20,
        max_health: 100,
        minerals: 100,
    };
    let depleted_primary_chain = [
        StructureState {
            minerals: 0,
            ..primary_chain[0]
        },
        StructureState {
            minerals: 0,
            ..primary_chain[1]
        },
    ];
    let expansion = controller.decide(
        &direct_attack_tuning(),
        &GameState {
            grid: &grid,
            swarms: &supplied_swarms,
            bots: &bots,
            structures: &[
                depleted_primary_chain[0],
                depleted_primary_chain[1],
                enemy_sink,
            ],
            deposits: &deposits,
            terrain: &[],
            charge: &DEFAULT_CHARGE,
            reachability: None,
            route_query: None,
            tick: WORK_WINDOW_TICKS,
            finished: false,
        },
    );

    assert!(expansion.edits.iter().any(|edit| {
        edit.action == IntentEditAction::Paint
            && edit.kind == IntentKind::Gather
            && edit.cell == IVec2::new(4, 0)
    }));
    assert!(!expansion.edits.iter().any(|edit| {
        edit.action == IntentEditAction::Paint
            && edit.kind == IntentKind::Defend
            && edit.cell == IVec2::new(8, 0)
    }));
    assert!(!expansion.edits.iter().any(|edit| {
        edit.action == IntentEditAction::Erase
            && edit.kind == IntentKind::Gather
            && edit.cell == IVec2::new(-1, 0)
    }));
}

#[test]
fn advantaged_raid_paints_enemy_logistics_and_keeps_home_economy() {
    let grid = IntentGrid::new(32, 32);
    let owner = SwarmId(4);
    let enemy = SwarmId(9);
    let swarms = [
        SwarmState {
            id: owner,
            home: world(IVec2::ZERO),
            minerals: 1_000,
            mineral_income_per_second: 0.0,
            production_saturated: false,
        },
        SwarmState {
            id: enemy,
            home: world(IVec2::new(8, 0)),
            minerals: 0,
            mineral_income_per_second: 0.0,
            production_saturated: false,
        },
    ];
    let bots = (0..8)
        .map(|id| BotState {
            id,
            owner,
            kind: NanobotType::Defender,
            position: swarms[0].home,
            health: 100,
            max_health: 100,
            charge: 1.0,
            max_charge: 1.0,
            cargo: 0,
        })
        .collect::<Vec<_>>();
    let deposits = [DepositState {
        id: 20,
        owner: Some(owner),
        position: world(IVec2::new(-1, 0)),
        amount: 500,
        radius: 100.0,
    }];
    let structures = [StructureState {
        id: 30,
        owner: enemy,
        position: world(IVec2::new(7, 0)),
        kind: StructureKind::Sink,
        health: 20,
        max_health: 100,
        minerals: 80,
    }];

    let decision = Controller::new(owner).decide(
        &direct_attack_tuning(),
        &GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &bots,
            structures: &structures,
            deposits: &deposits,
            terrain: &[],
            charge: &DEFAULT_CHARGE,
            reachability: None,
            route_query: None,
            tick: 0,
            finished: false,
        },
    );

    assert!(decision.explanation.contains("attack enemy logistics"));
    assert!(decision.edits.iter().any(|edit| {
        edit.action == IntentEditAction::Paint
            && edit.kind == IntentKind::Gather
            && edit.cell == IVec2::new(-1, 0)
    }));
    assert!(decision.edits.iter().any(|edit| {
        edit.action == IntentEditAction::Paint
            && edit.kind == IntentKind::Build
            && edit.cell == IVec2::ZERO
    }));
    assert!(decision.edits.iter().any(|edit| {
        edit.action == IntentEditAction::Paint
            && edit.kind == IntentKind::Defend
            && edit.cell == IVec2::new(7, 0)
    }));
}

#[test]
fn progressing_nanobot_front_is_not_abandoned_for_fresh_targets() {
    let mut grid = IntentGrid::new(32, 32);
    let owner = SwarmId(4);
    let enemy = SwarmId(9);
    let swarms = [
        SwarmState {
            id: owner,
            home: world(IVec2::ZERO),
            minerals: 2_000,
            mineral_income_per_second: 0.0,
            production_saturated: false,
        },
        SwarmState {
            id: enemy,
            home: world(IVec2::new(12, 0)),
            minerals: 0,
            mineral_income_per_second: 0.0,
            production_saturated: false,
        },
    ];
    let friendly_defenders = (0..2)
        .map(|id| BotState {
            id,
            owner,
            kind: NanobotType::Defender,
            position: swarms[0].home,
            health: 100,
            max_health: 100,
            charge: 1.0,
            max_charge: 1.0,
            cargo: 0,
        })
        .collect::<Vec<_>>();
    let rich_charger = StructureState {
        id: 30,
        owner: enemy,
        position: world(IVec2::new(6, 2)),
        kind: StructureKind::Charger,
        health: 1,
        max_health: 100,
        minerals: 1_000,
    };
    let damage_target = BotState {
        id: 50,
        owner: enemy,
        kind: NanobotType::Worker,
        position: world(IVec2::new(6, -2)),
        health: 100,
        max_health: 100,
        charge: 1.0,
        max_charge: 1.0,
        cargo: 0,
    };
    let mut bots = friendly_defenders.clone();
    bots.push(damage_target);
    let mut controller = Controller::new(owner);

    let initial = controller.decide(
        &direct_attack_tuning(),
        &GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &bots,
            structures: &[],
            deposits: &[],
            terrain: &[],
            charge: &DEFAULT_CHARGE,
            reachability: None,
            route_query: None,
            tick: 0,
            finished: false,
        },
    );
    apply(&mut grid, owner, &initial);

    assert!(initial.explanation.contains("Nanobot 50"));
    assert!(
        grid.cell(IVec2::new(6, -2))
            .is_some_and(|intent| { intent.has_owned(IntentKind::Defend, owner) })
    );
    let progressing_target = BotState {
        health: 20,
        ..damage_target
    };
    let fresh_target = BotState {
        id: 51,
        position: world(IVec2::new(4, 4)),
        ..damage_target
    };
    bots.truncate(friendly_defenders.len());
    bots.extend([progressing_target, fresh_target]);
    let revised = controller.decide(
        &direct_attack_tuning(),
        &GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &bots,
            structures: &[rich_charger],
            deposits: &[],
            terrain: &[],
            charge: &DEFAULT_CHARGE,
            reachability: None,
            route_query: None,
            tick: WORK_WINDOW_TICKS,
            finished: false,
        },
    );
    apply(&mut grid, owner, &revised);

    assert!(
        grid.cell(IVec2::new(6, -2))
            .is_some_and(|intent| { intent.has_owned(IntentKind::Defend, owner) })
    );
    assert!(
        !grid
            .cell(IVec2::new(4, 4))
            .is_some_and(|intent| { intent.has_owned(IntentKind::Defend, owner) })
    );
    assert!(
        !grid
            .cell(IVec2::new(6, 2))
            .is_some_and(|intent| { intent.has_owned(IntentKind::Defend, owner) })
    );
    assert!(
        revised
            .explanation
            .contains("retained hunt remaining Nanobot 50")
    );
}

#[test]
fn losing_a_mine_keeps_a_progressing_attack_front() {
    let mut grid = IntentGrid::new(32, 32);
    let owner = SwarmId(4);
    let enemy = SwarmId(9);
    let swarms = [
        SwarmState {
            id: owner,
            home: world(IVec2::ZERO),
            minerals: 2_000,
            mineral_income_per_second: 0.0,
            production_saturated: false,
        },
        SwarmState {
            id: enemy,
            home: world(IVec2::new(12, 0)),
            minerals: 0,
            mineral_income_per_second: 0.0,
            production_saturated: false,
        },
    ];
    let bots = (0..8)
        .map(|id| BotState {
            id,
            owner,
            kind: NanobotType::Defender,
            position: swarms[0].home,
            health: 100,
            max_health: 100,
            charge: 1.0,
            max_charge: 1.0,
            cargo: 0,
        })
        .collect::<Vec<_>>();
    let friendly_charger = StructureState {
        id: 20,
        owner,
        position: world(IVec2::new(1, 0)),
        kind: StructureKind::Charger,
        health: 100,
        max_health: 100,
        minerals: 100,
    };
    let incumbent = StructureState {
        id: 30,
        owner: enemy,
        position: world(IVec2::new(12, 0)),
        kind: StructureKind::Sink,
        health: 100,
        max_health: 100,
        minerals: 0,
    };
    let mine_cell = IVec2::new(-4, 0);
    let mine = DepositState {
        id: 21,
        owner: Some(owner),
        position: world(mine_cell),
        amount: 72_000,
        radius: 80.0,
    };
    let mut controller = Controller::new(owner);
    let initial = controller.decide(
        &direct_attack_tuning(),
        &GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &bots,
            structures: &[friendly_charger, incumbent],
            deposits: &[mine],
            terrain: &[],
            charge: &DEFAULT_CHARGE,
            reachability: None,
            route_query: None,
            tick: 0,
            finished: false,
        },
    );
    apply(&mut grid, owner, &initial);

    assert!(
        grid.cell(mine_cell)
            .unwrap()
            .has_owned(IntentKind::Gather, owner)
    );

    let progressing_incumbent = StructureState {
        health: 20,
        ..incumbent
    };
    let new_forward_target = StructureState {
        id: 31,
        owner: enemy,
        position: world(IVec2::new(3, 5)),
        kind: StructureKind::Charger,
        health: 100,
        max_health: 100,
        minerals: 1_000,
    };
    let revised = controller.decide(
        &direct_attack_tuning(),
        &GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &bots,
            structures: &[friendly_charger, progressing_incumbent, new_forward_target],
            deposits: &[DepositState { amount: 0, ..mine }],
            terrain: &[],
            charge: &DEFAULT_CHARGE,
            reachability: None,
            route_query: None,
            tick: 1,
            finished: false,
        },
    );
    apply(&mut grid, owner, &revised);

    assert!(
        revised
            .explanation
            .contains("retained attack enemy logistics structure 30"),
        "{}",
        revised.explanation
    );
    assert!(revised.reviewed);
    assert!(
        !grid
            .cell(mine_cell)
            .unwrap()
            .has_owned(IntentKind::Gather, owner)
    );
    assert!(
        grid.cell(IVec2::new(12, 0))
            .is_some_and(|intent| { intent.has_owned(IntentKind::Defend, owner) })
    );
    assert!(
        !grid
            .cell(IVec2::new(3, 5))
            .is_some_and(|intent| { intent.has_owned(IntentKind::Defend, owner) })
    );
}

#[test]
fn viable_plan_is_quiet_between_reviews_and_retained_at_regular_review() {
    let mut grid = IntentGrid::new(32, 32);
    let owner = SwarmId(4);
    let swarms = [SwarmState {
        id: owner,
        home: world(IVec2::ZERO),
        minerals: 20,
        mineral_income_per_second: 0.0,
        production_saturated: false,
    }];
    let deposits = [DepositState {
        id: 20,
        owner: Some(owner),
        position: world(IVec2::new(2, 0)),
        amount: 500,
        radius: 100.0,
    }];
    let mut controller = Controller::new(owner);
    let initial = controller.decide(
        &crate::tuning::AutonomyTuning::default(),
        &GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &[],
            structures: &[],
            deposits: &deposits,
            terrain: &[],
            charge: &DEFAULT_CHARGE,
            reachability: None,
            route_query: None,
            tick: 0,
            finished: false,
        },
    );
    apply(&mut grid, owner, &initial);

    let between = controller.decide(
        &crate::tuning::AutonomyTuning::default(),
        &GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &[],
            structures: &[],
            deposits: &deposits,
            terrain: &[],
            charge: &DEFAULT_CHARGE,
            reachability: None,
            route_query: None,
            tick: WORK_WINDOW_TICKS - 1,
            finished: false,
        },
    );
    assert!(!between.reviewed);
    assert!(between.edits.is_empty());

    let regular = controller.decide(
        &crate::tuning::AutonomyTuning::default(),
        &GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &[],
            structures: &[],
            deposits: &deposits,
            terrain: &[],
            charge: &DEFAULT_CHARGE,
            reachability: None,
            route_query: None,
            tick: WORK_WINDOW_TICKS,
            finished: false,
        },
    );
    assert!(regular.reviewed);
    assert!(regular.edits.is_empty());
    assert!(regular.explanation.contains("retained viable"));
    assert!(regular.work_units <= PLANNING_WORK_BUDGET);
}

#[test]
fn new_neutral_deposit_keeps_the_existing_mine_supported() {
    let owner = SwarmId::PLAYER;
    let mut grid = IntentGrid::new(32, 32);
    let swarms = [SwarmState {
        id: owner,
        home: world(IVec2::ZERO),
        minerals: 0,
        mineral_income_per_second: 0.0,
        production_saturated: false,
    }];
    let deposits = [
        DepositState {
            id: 20,
            owner: None,
            position: world(IVec2::new(4, 0)),
            amount: 72_000,
            radius: 80.0,
        },
        DepositState {
            id: 21,
            owner: None,
            position: world(IVec2::new(0, 6)),
            amount: 72_000,
            radius: 80.0,
        },
    ];
    let tuning = AutonomyTuning::default();
    let mut controller = Controller::new(owner);
    let initial = controller.decide(
        &tuning,
        &GameState {
            swarms: &swarms,
            deposits: &deposits,
            ..empty_state(&grid, 0)
        },
    );
    apply(&mut grid, owner, &initial);
    let expanded = controller.decide(
        &tuning,
        &GameState {
            swarms: &swarms,
            deposits: &deposits,
            ..empty_state(&grid, WORK_WINDOW_TICKS)
        },
    );
    apply(&mut grid, owner, &expanded);

    for (gather, build) in [
        (IVec2::new(4, 0), IVec2::new(3, 0)),
        (IVec2::new(0, 6), IVec2::new(0, 5)),
    ] {
        for (cell, kind) in [
            (gather, IntentKind::Gather),
            (gather, IntentKind::Defend),
            (build, IntentKind::Build),
            (build, IntentKind::Defend),
            (build, IntentKind::Corridor),
        ] {
            assert!(
                grid.cell(cell)
                    .is_some_and(|intent| intent.has_owned(kind, owner)),
                "mine at {gather} lacks {kind:?} at {cell}"
            );
        }
    }
}

#[test]
fn depleted_and_unreachable_deposits_are_removed_without_abandoning_the_others() {
    use crate::{nanobot::InteractionRegion, navigation::Route};

    let owner = SwarmId::PLAYER;
    let mut grid = IntentGrid::new(32, 32);
    let home = world(IVec2::ZERO);
    let swarms = [SwarmState {
        id: owner,
        home,
        minerals: 0,
        mineral_income_per_second: 0.0,
        production_saturated: false,
    }];
    let cells = [IVec2::new(4, 0), IVec2::new(0, 4), IVec2::new(-4, 0)];
    let mut deposits = cells.map(|cell| DepositState {
        id: (cell.x + 4) as u64,
        owner: None,
        position: world(cell),
        amount: 72_000,
        radius: 80.0,
    });
    let tuning = AutonomyTuning::default();
    let mut controller = Controller::new(owner);
    for cell in cells {
        grid.paint(cell, IntentKind::Gather, owner);
    }
    let initial = controller.decide(
        &tuning,
        &GameState {
            swarms: &swarms,
            deposits: &deposits,
            ..empty_state(&grid, 0)
        },
    );
    apply(&mut grid, owner, &initial);
    for cell in cells {
        assert!(
            grid.cell(cell)
                .unwrap()
                .has_owned(IntentKind::Gather, owner)
        );
    }
    deposits[0].amount = 0;
    let blocked = InteractionRegion::deposit(
        &bevy::prelude::Transform::from_translation(deposits[1].position.extend(0.0)),
        deposits[1].radius,
    )
    .within_cell(cells[1]);
    let route = |start: Vec2, goal: RouteGoal, _: &mut dyn FnMut() -> bool| {
        Some(match goal {
            RouteGoal::Interaction(region) if region != blocked => {
                let end = region.approach(start);
                RouteOutcome::Found(Route {
                    waypoints: vec![end],
                    cost: start.distance(end),
                })
            }
            _ => RouteOutcome::Unreachable,
        })
    };
    let revised = controller.decide(
        &tuning,
        &GameState {
            swarms: &swarms,
            deposits: &deposits,
            route_query: Some(&route),
            ..empty_state(&grid, WORK_WINDOW_TICKS)
        },
    );
    apply(&mut grid, owner, &revised);
    for cell in [cells[0], cells[1]] {
        assert!(
            !grid
                .cell(cell)
                .unwrap()
                .has_owned(IntentKind::Gather, owner),
            "unavailable deposit at {cell} retained Gather"
        );
    }
    assert!(
        grid.cell(cells[2])
            .unwrap()
            .has_owned(IntentKind::Gather, owner)
    );
}

#[test]
fn accumulated_supply_and_a_logistics_target_replace_the_economy_plan() {
    let mut grid = IntentGrid::new(64, 64);
    let owner = SwarmId(4);
    let enemy = SwarmId(9);
    let initial_swarms = [
        SwarmState {
            id: owner,
            home: world(IVec2::ZERO),
            minerals: 0,
            mineral_income_per_second: 0.0,
            production_saturated: false,
        },
        SwarmState {
            id: enemy,
            home: world(IVec2::new(24, 24)),
            minerals: 0,
            mineral_income_per_second: 0.0,
            production_saturated: false,
        },
    ];
    let bots = [owner, enemy]
        .into_iter()
        .flat_map(|bot_owner| {
            (0..3).map(move |id| BotState {
                id: bot_owner.0 as u64 * 10 + id,
                owner: bot_owner,
                kind: NanobotType::Defender,
                position: if bot_owner == owner {
                    initial_swarms[0].home
                } else {
                    initial_swarms[1].home
                },
                health: 100,
                max_health: 100,
                charge: 1.0,
                max_charge: 1.0,
                cargo: 0,
            })
        })
        .collect::<Vec<_>>();
    let deposit = [DepositState {
        id: 20,
        owner: None,
        position: world(IVec2::new(-1, -1)),
        amount: 72_000,
        radius: 64.0,
    }];
    let facility = StructureState {
        id: 30,
        owner: enemy,
        position: initial_swarms[1].home,
        kind: StructureKind::Facility,
        health: 100,
        max_health: 100,
        minerals: 0,
    };
    let mut controller = Controller::new(owner);
    let initial = controller.decide(
        &direct_attack_tuning(),
        &GameState {
            grid: &grid,
            swarms: &initial_swarms,
            bots: &bots,
            structures: &[facility],
            deposits: &deposit,
            terrain: &[],
            charge: &DEFAULT_CHARGE,
            reachability: None,
            route_query: None,
            tick: 0,
            finished: false,
        },
    );
    assert!(initial.explanation.contains("secure Resource Deposit"));
    apply(&mut grid, owner, &initial);

    let supplied_swarms = [
        SwarmState {
            minerals: 2_000,
            mineral_income_per_second: 0.0,
            production_saturated: false,
            ..initial_swarms[0]
        },
        initial_swarms[1],
    ];
    let sink = StructureState {
        id: 31,
        kind: StructureKind::Sink,
        minerals: 100,
        ..facility
    };
    let pressure = controller.decide(
        &direct_attack_tuning(),
        &GameState {
            grid: &grid,
            swarms: &supplied_swarms,
            bots: &bots,
            structures: &[facility, sink],
            deposits: &deposit,
            terrain: &[],
            charge: &DEFAULT_CHARGE,
            reachability: None,
            route_query: None,
            tick: WORK_WINDOW_TICKS,
            finished: false,
        },
    );

    assert!(pressure.explanation.contains("attack enemy logistics"));
    assert!(pressure.edits.iter().any(|edit| {
        edit.action == IntentEditAction::Paint
            && edit.kind == IntentKind::Defend
            && edit.cell == IVec2::new(24, 24)
    }));
    assert!(!pressure.edits.iter().any(|edit| {
        edit.action == IntentEditAction::Erase
            && edit.kind == IntentKind::Gather
            && edit.cell == IVec2::new(-1, -1)
    }));
}

#[test]
fn expansion_prefers_income_after_hauling_and_exposure_costs() {
    use crate::{nanobot::InteractionRegion, navigation::Route};

    let grid = IntentGrid::new(64, 64);
    let owner = SwarmId::PLAYER;
    let home = world(IVec2::ZERO);
    let swarms = [SwarmState {
        id: owner,
        home,
        minerals: 0,
        mineral_income_per_second: 0.0,
        production_saturated: false,
    }];
    let poor = IVec2::new(0, -2);
    let best = IVec2::new(0, 5);
    let exposed = IVec2::new(5, 0);
    let costly = IVec2::new(0, 8);
    let deposits = [
        (poor, 35),
        (best, 12_000),
        (exposed, 12_000),
        (costly, 40_000),
    ]
    .into_iter()
    .enumerate()
    .map(|(id, (cell, amount))| DepositState {
        id: id as u64,
        owner: None,
        position: world(cell),
        amount,
        radius: 80.0,
    })
    .collect::<Vec<_>>();
    let terrain = [Obstacle::Circle {
        center: world(IVec2::new(2, 0)),
        radius: 230.0,
    }];
    let exposed_region = InteractionRegion::deposit(
        &bevy::prelude::Transform::from_translation(world(exposed).extend(0.0)),
        80.0,
    )
    .within_cell(exposed);
    let route = |start: Vec2, goal: RouteGoal, _: &mut dyn FnMut() -> bool| {
        Some(match goal {
            RouteGoal::Interaction(region) => {
                let end = region.approach(start);
                let mut waypoints = Vec::new();
                if region == exposed_region {
                    waypoints.extend([world(IVec2::new(2, -1)), world(IVec2::new(3, -1))]);
                }
                waypoints.push(end);
                RouteOutcome::Found(Route {
                    waypoints,
                    cost: start.distance(end),
                })
            }
            _ => RouteOutcome::Unreachable,
        })
    };
    let decision = Controller::new(owner).decide(
        &AutonomyTuning::default(),
        &GameState {
            swarms: &swarms,
            deposits: &deposits,
            terrain: &terrain,
            charge: &DEFAULT_CHARGE,
            route_query: Some(&route),
            ..empty_state(&grid, 0)
        },
    );

    let gather = decision
        .edits
        .iter()
        .filter(|edit| edit.kind == IntentKind::Gather)
        .map(|edit| edit.cell)
        .collect::<Vec<_>>();
    assert_eq!(gather, [best]);
}

#[test]
fn authored_terrain_leaves_budget_for_a_coordinated_plan() {
    let grid = IntentGrid::new(64, 64);
    let owner = SwarmId::PLAYER;
    let enemy = SwarmId(1);
    let swarms = [
        SwarmState {
            id: owner,
            home: crate::scenario::cell_origin(crate::scenario::PLAYER_CELL),
            minerals: 0,
            mineral_income_per_second: 0.0,
            production_saturated: false,
        },
        SwarmState {
            id: enemy,
            home: crate::scenario::cell_origin(crate::scenario::OPPONENT_CELL),
            minerals: 0,
            mineral_income_per_second: 0.0,
            production_saturated: false,
        },
    ];
    let bots = NanobotType::ALL
        .into_iter()
        .flat_map(|kind| {
            (0..3).map(move |id| BotState {
                id: kind as u64 * 3 + id,
                owner,
                kind,
                position: swarms[0].home,
                health: 100,
                max_health: 100,
                charge: 1.0,
                max_charge: 1.0,
                cargo: 0,
            })
        })
        .collect::<Vec<_>>();
    let deposit_cells = [
        crate::scenario::PLAYER_DEPOSIT_CELL,
        crate::scenario::OPPONENT_DEPOSIT_CELL,
        crate::scenario::NEUTRAL_DEPOSIT_CELLS[0],
        crate::scenario::NEUTRAL_DEPOSIT_CELLS[1],
        crate::scenario::NEUTRAL_DEPOSIT_CELLS[2],
        crate::scenario::NEUTRAL_DEPOSIT_CELLS[3],
    ];
    let deposits = deposit_cells
        .into_iter()
        .enumerate()
        .map(|(id, cell)| DepositState {
            id: id as u64,
            owner: None,
            position: crate::scenario::cell_origin(cell),
            amount: crate::scenario::STARTING_DEPOSIT_AMOUNT,
            radius: crate::scenario::STARTING_WORK_RADIUS,
        })
        .collect::<Vec<_>>();
    let terrain = crate::scenario::default_rock_geometry()
        .into_iter()
        .map(|(rock, transform)| rock.obstacle(&transform))
        .collect::<Vec<_>>();

    let decision = Controller::new(owner).decide(
        &crate::tuning::AutonomyTuning::default(),
        &GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &bots,
            structures: &[],
            deposits: &deposits,
            terrain: &terrain,
            charge: &DEFAULT_CHARGE,
            reachability: None,
            route_query: None,
            tick: 0,
            finished: false,
        },
    );

    assert!(decision.work_units <= PLANNING_WORK_BUDGET);
    for kind in IntentKind::ALL {
        assert!(
            decision
                .edits
                .iter()
                .any(|edit| edit.action == IntentEditAction::Paint && edit.kind == kind),
            "authored terrain plan omitted {kind:?} at {} work units with {} rocks",
            decision.work_units,
            terrain.len()
        );
    }
}

#[test]
fn authored_terrain_advantaged_raid_reaches_target_and_preserves_gather() {
    let mut grid = IntentGrid::new(64, 64);
    let owner = SwarmId::PLAYER;
    let enemy = SwarmId(1);
    let swarms = [
        SwarmState {
            id: owner,
            home: crate::scenario::cell_origin(crate::scenario::PLAYER_CELL),
            minerals: 2_000,
            mineral_income_per_second: 0.0,
            production_saturated: false,
        },
        SwarmState {
            id: enemy,
            home: crate::scenario::cell_origin(crate::scenario::OPPONENT_CELL),
            minerals: 0,
            mineral_income_per_second: 0.0,
            production_saturated: false,
        },
    ];
    let bots = (0..10)
        .map(|id| BotState {
            id,
            owner,
            kind: NanobotType::Defender,
            position: swarms[0].home,
            health: 100,
            max_health: 100,
            charge: 1.0,
            max_charge: 1.0,
            cargo: 0,
        })
        .collect::<Vec<_>>();
    let deposit_cells = [
        crate::scenario::PLAYER_DEPOSIT_CELL,
        crate::scenario::OPPONENT_DEPOSIT_CELL,
        crate::scenario::NEUTRAL_DEPOSIT_CELLS[0],
        crate::scenario::NEUTRAL_DEPOSIT_CELLS[1],
        crate::scenario::NEUTRAL_DEPOSIT_CELLS[2],
        crate::scenario::NEUTRAL_DEPOSIT_CELLS[3],
    ];
    for cell in crate::scenario::NEUTRAL_DEPOSIT_CELLS {
        grid.paint(cell, IntentKind::Gather, enemy);
    }
    let deposits = deposit_cells
        .into_iter()
        .enumerate()
        .map(|(id, cell)| DepositState {
            id: 20 + id as u64,
            owner: match id {
                0 => Some(owner),
                1 => Some(enemy),
                _ => None,
            },
            position: crate::scenario::cell_origin(cell),
            amount: crate::scenario::STARTING_DEPOSIT_AMOUNT,
            radius: crate::scenario::STARTING_WORK_RADIUS,
        })
        .collect::<Vec<_>>();
    let structures = [StructureState {
        id: 30,
        owner: enemy,
        position: swarms[1].home,
        kind: StructureKind::Sink,
        health: 20,
        max_health: 100,
        minerals: 100,
    }];
    let terrain = crate::scenario::default_rock_geometry()
        .into_iter()
        .map(|(rock, transform)| rock.obstacle(&transform))
        .collect::<Vec<_>>();

    let decision = Controller::new(owner).decide(
        &direct_attack_tuning(),
        &GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &bots,
            structures: &structures,
            deposits: &deposits,
            terrain: &terrain,
            charge: &DEFAULT_CHARGE,
            reachability: None,
            route_query: None,
            tick: 0,
            finished: false,
        },
    );

    assert!(decision.explanation.contains("attack enemy logistics"));
    assert!(decision.edits.iter().any(|edit| {
        edit.action == IntentEditAction::Paint
            && edit.kind == IntentKind::Gather
            && edit.cell == crate::scenario::PLAYER_DEPOSIT_CELL
    }));
    assert!(decision.edits.iter().any(|edit| {
        edit.action == IntentEditAction::Paint
            && edit.kind == IntentKind::Defend
            && edit.cell == crate::scenario::OPPONENT_CELL
    }));
    assert!(decision.work_units < PLANNING_WORK_BUDGET);
}

#[test]
fn progressing_target_within_owned_defend_coverage_keeps_existing_paint() {
    let mut grid = IntentGrid::new(32, 32);
    let owner = SwarmId(4);
    let enemy = SwarmId(9);
    let target_cell = IVec2::new(6, 0);
    let covered_cell = IVec2::new(5, -1);
    let swarms = [
        SwarmState {
            id: owner,
            home: world(IVec2::ZERO),
            minerals: 2_000,
            mineral_income_per_second: 0.0,
            production_saturated: false,
        },
        SwarmState {
            id: enemy,
            home: world(IVec2::new(12, 0)),
            minerals: 0,
            mineral_income_per_second: 0.0,
            production_saturated: false,
        },
    ];
    let friendly_defenders = (0..2).map(|id| BotState {
        id,
        owner,
        kind: NanobotType::Defender,
        position: swarms[0].home,
        health: 100,
        max_health: 100,
        charge: 1.0,
        max_charge: 1.0,
        cargo: 0,
    });
    let target = BotState {
        id: 50,
        owner: enemy,
        kind: NanobotType::Worker,
        position: world(target_cell),
        health: 100,
        max_health: 100,
        charge: 1.0,
        max_charge: 1.0,
        cargo: 0,
    };
    let bots = friendly_defenders.chain([target]).collect::<Vec<_>>();
    let mut controller = Controller::new(owner);
    let initial = controller.decide(
        &direct_attack_tuning(),
        &GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &bots,
            structures: &[],
            deposits: &[],
            terrain: &[],
            charge: &DEFAULT_CHARGE,
            reachability: None,
            route_query: None,
            tick: 0,
            finished: false,
        },
    );
    apply(&mut grid, owner, &initial);
    assert!(
        grid.cell(covered_cell)
            .is_some_and(|intent| intent.has_owned(IntentKind::Defend, owner)),
        "the moved target cell must be covered by actual owned Defend intent"
    );

    let moved_progressing = BotState {
        position: world(covered_cell),
        health: 80,
        ..target
    };
    let mut bots = bots;
    *bots.last_mut().unwrap() = moved_progressing;
    let immediate = controller.decide(
        &direct_attack_tuning(),
        &GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &bots,
            structures: &[],
            deposits: &[],
            terrain: &[],
            charge: &DEFAULT_CHARGE,
            reachability: None,
            route_query: None,
            tick: 1,
            finished: false,
        },
    );
    assert!(!immediate.reviewed);
    assert!(immediate.edits.is_empty());

    let regular = controller.decide(
        &direct_attack_tuning(),
        &GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &bots,
            structures: &[],
            deposits: &[],
            terrain: &[],
            charge: &DEFAULT_CHARGE,
            reachability: None,
            route_query: None,
            tick: WORK_WINDOW_TICKS,
            finished: false,
        },
    );
    assert!(regular.reviewed);
    assert!(regular.edits.is_empty());
    assert!(
        grid.cell(target_cell)
            .is_some_and(|intent| intent.has_owned(IntentKind::Defend, owner))
    );
    assert!(
        grid.cell(covered_cell)
            .is_some_and(|intent| intent.has_owned(IntentKind::Defend, owner))
    );
}

#[test]
fn covered_movement_without_damage_gets_one_grace_before_reassessment() {
    let mut grid = IntentGrid::new(32, 32);
    let owner = SwarmId(4);
    let enemy = SwarmId(9);
    let target_cell = IVec2::new(6, 0);
    let covered_cell = IVec2::new(5, -1);
    let alternative_cell = IVec2::new(8, 4);
    let swarms = [
        SwarmState {
            id: owner,
            home: world(IVec2::ZERO),
            minerals: 2_000,
            mineral_income_per_second: 0.0,
            production_saturated: false,
        },
        SwarmState {
            id: enemy,
            home: world(IVec2::new(12, 0)),
            minerals: 0,
            mineral_income_per_second: 0.0,
            production_saturated: false,
        },
    ];
    let friendly_defenders = (0..2).map(|id| BotState {
        id,
        owner,
        kind: NanobotType::Defender,
        position: swarms[0].home,
        health: 100,
        max_health: 100,
        charge: 1.0,
        max_charge: 1.0,
        cargo: 0,
    });
    let target = BotState {
        id: 50,
        owner: enemy,
        kind: NanobotType::Worker,
        position: world(target_cell),
        health: 100,
        max_health: 100,
        charge: 1.0,
        max_charge: 1.0,
        cargo: 0,
    };
    let mut bots = friendly_defenders.chain([target]).collect::<Vec<_>>();
    let alternative = StructureState {
        id: 30,
        owner: enemy,
        position: world(alternative_cell),
        kind: StructureKind::Charger,
        health: 1,
        max_health: 100,
        minerals: 1_000,
    };
    let mut controller = Controller::new(owner);
    let initial = controller.decide(
        &direct_attack_tuning(),
        &GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &bots,
            structures: &[],
            deposits: &[],
            terrain: &[],
            charge: &DEFAULT_CHARGE,
            reachability: None,
            route_query: None,
            tick: 0,
            finished: false,
        },
    );
    apply(&mut grid, owner, &initial);
    assert!(
        grid.cell(covered_cell)
            .is_some_and(|intent| intent.has_owned(IntentKind::Defend, owner))
    );

    *bots.last_mut().unwrap() = BotState {
        position: world(covered_cell),
        ..target
    };
    let immediate = controller.decide(
        &direct_attack_tuning(),
        &GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &bots,
            structures: &[],
            deposits: &[],
            terrain: &[],
            charge: &DEFAULT_CHARGE,
            reachability: None,
            route_query: None,
            tick: 1,
            finished: false,
        },
    );
    assert!(!immediate.reviewed);
    assert!(immediate.edits.is_empty());
    let grace = controller.decide(
        &direct_attack_tuning(),
        &GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &bots,
            structures: &[alternative],
            deposits: &[],
            terrain: &[],
            charge: &DEFAULT_CHARGE,
            reachability: None,
            route_query: None,
            tick: WORK_WINDOW_TICKS,
            finished: false,
        },
    );
    assert!(grace.reviewed);
    assert!(grace.edits.is_empty());
    let reassessed = controller.decide(
        &direct_attack_tuning(),
        &GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &bots,
            structures: &[alternative],
            deposits: &[],
            terrain: &[],
            charge: &DEFAULT_CHARGE,
            reachability: None,
            route_query: None,
            tick: WORK_WINDOW_TICKS * 2,
            finished: false,
        },
    );
    assert!(reassessed.edits.iter().any(|edit| {
        edit.action == IntentEditAction::Paint
            && edit.kind == IntentKind::Defend
            && edit.cell == alternative_cell
    }));
}

#[test]
fn progressing_target_that_exits_owned_defend_coverage_is_followed() {
    let mut grid = IntentGrid::new(32, 32);
    let owner = SwarmId(4);
    let enemy = SwarmId(9);
    let target_cell = IVec2::new(6, 0);
    let covered_cell = IVec2::new(5, -1);
    let uncovered_cell = IVec2::new(10, 0);
    let swarms = [
        SwarmState {
            id: owner,
            home: world(IVec2::ZERO),
            minerals: 2_000,
            mineral_income_per_second: 0.0,
            production_saturated: false,
        },
        SwarmState {
            id: enemy,
            home: world(IVec2::new(12, 0)),
            minerals: 0,
            mineral_income_per_second: 0.0,
            production_saturated: false,
        },
    ];
    let friendly_defenders = (0..2).map(|id| BotState {
        id,
        owner,
        kind: NanobotType::Defender,
        position: swarms[0].home,
        health: 100,
        max_health: 100,
        charge: 1.0,
        max_charge: 1.0,
        cargo: 0,
    });
    let target = BotState {
        id: 50,
        owner: enemy,
        kind: NanobotType::Worker,
        position: world(target_cell),
        health: 100,
        max_health: 100,
        charge: 1.0,
        max_charge: 1.0,
        cargo: 0,
    };
    let mut bots = friendly_defenders.chain([target]).collect::<Vec<_>>();
    let mut controller = Controller::new(owner);
    let initial = controller.decide(
        &direct_attack_tuning(),
        &GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &bots,
            structures: &[],
            deposits: &[],
            terrain: &[],
            charge: &DEFAULT_CHARGE,
            reachability: None,
            route_query: None,
            tick: 0,
            finished: false,
        },
    );
    apply(&mut grid, owner, &initial);
    assert!(
        grid.cell(covered_cell)
            .is_some_and(|intent| intent.has_owned(IntentKind::Defend, owner))
    );
    assert!(
        !grid
            .cell(uncovered_cell)
            .is_some_and(|intent| intent.has_owned(IntentKind::Defend, owner)),
        "the exit must begin outside actual owned Defend coverage"
    );

    *bots.last_mut().unwrap() = BotState {
        position: world(covered_cell),
        health: 80,
        ..target
    };
    let covered = controller.decide(
        &direct_attack_tuning(),
        &GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &bots,
            structures: &[],
            deposits: &[],
            terrain: &[],
            charge: &DEFAULT_CHARGE,
            reachability: None,
            route_query: None,
            tick: 1,
            finished: false,
        },
    );
    assert!(!covered.reviewed);
    assert!(covered.edits.is_empty());

    *bots.last_mut().unwrap() = BotState {
        position: world(uncovered_cell),
        health: 60,
        ..target
    };
    let competing_charger = StructureState {
        id: 30,
        owner: enemy,
        position: world(IVec2::new(2, 7)),
        kind: StructureKind::Charger,
        health: 1,
        max_health: 100,
        minerals: 1_000,
    };
    let exited = controller.decide(
        &direct_attack_tuning(),
        &GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &bots,
            structures: &[competing_charger],
            deposits: &[],
            terrain: &[],
            charge: &DEFAULT_CHARGE,
            reachability: None,
            route_query: None,
            tick: 2,
            finished: false,
        },
    );
    assert!(exited.reviewed);
    assert!(
        exited
            .explanation
            .contains("retained hunt remaining Nanobot 50"),
        "exiting the block must preserve a progressing hunt: {}",
        exited.explanation
    );
    assert!(exited.edits.iter().any(|edit| {
        edit.action == IntentEditAction::Paint
            && edit.kind == IntentKind::Defend
            && edit.cell == uncovered_cell
    }));
}

#[test]
fn renewed_covered_progress_resets_the_stall_grace() {
    let mut grid = IntentGrid::new(32, 32);
    let owner = SwarmId(4);
    let enemy = SwarmId(9);
    let target_cell = IVec2::new(6, 0);
    let covered_cell = IVec2::new(5, -1);
    let alternative_cell = IVec2::new(8, 4);
    let swarms = [
        SwarmState {
            id: owner,
            home: world(IVec2::ZERO),
            minerals: 2_000,
            mineral_income_per_second: 0.0,
            production_saturated: false,
        },
        SwarmState {
            id: enemy,
            home: world(IVec2::new(12, 0)),
            minerals: 0,
            mineral_income_per_second: 0.0,
            production_saturated: false,
        },
    ];
    let friendly_defenders = (0..2).map(|id| BotState {
        id,
        owner,
        kind: NanobotType::Defender,
        position: swarms[0].home,
        health: 100,
        max_health: 100,
        charge: 1.0,
        max_charge: 1.0,
        cargo: 0,
    });
    let target = BotState {
        id: 50,
        owner: enemy,
        kind: NanobotType::Worker,
        position: world(target_cell),
        health: 100,
        max_health: 100,
        charge: 1.0,
        max_charge: 1.0,
        cargo: 0,
    };
    let mut bots = friendly_defenders.chain([target]).collect::<Vec<_>>();
    let alternative = StructureState {
        id: 30,
        owner: enemy,
        position: world(alternative_cell),
        kind: StructureKind::Charger,
        health: 1,
        max_health: 100,
        minerals: 1_000,
    };
    let mut controller = Controller::new(owner);
    let initial = controller.decide(
        &direct_attack_tuning(),
        &GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &bots,
            structures: &[],
            deposits: &[],
            terrain: &[],
            charge: &DEFAULT_CHARGE,
            reachability: None,
            route_query: None,
            tick: 0,
            finished: false,
        },
    );
    apply(&mut grid, owner, &initial);
    assert!(
        grid.cell(covered_cell)
            .is_some_and(|intent| intent.has_owned(IntentKind::Defend, owner))
    );

    *bots.last_mut().unwrap() = BotState {
        position: world(covered_cell),
        health: 80,
        ..target
    };
    let immediate = controller.decide(
        &direct_attack_tuning(),
        &GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &bots,
            structures: &[],
            deposits: &[],
            terrain: &[],
            charge: &DEFAULT_CHARGE,
            reachability: None,
            route_query: None,
            tick: 1,
            finished: false,
        },
    );
    assert!(!immediate.reviewed);
    assert!(immediate.edits.is_empty());
    let progress_review = controller.decide(
        &direct_attack_tuning(),
        &GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &bots,
            structures: &[],
            deposits: &[],
            terrain: &[],
            charge: &DEFAULT_CHARGE,
            reachability: None,
            route_query: None,
            tick: WORK_WINDOW_TICKS,
            finished: false,
        },
    );
    assert!(progress_review.edits.is_empty());
    let first_grace = controller.decide(
        &direct_attack_tuning(),
        &GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &bots,
            structures: &[alternative],
            deposits: &[],
            terrain: &[],
            charge: &DEFAULT_CHARGE,
            reachability: None,
            route_query: None,
            tick: WORK_WINDOW_TICKS * 2,
            finished: false,
        },
    );
    assert!(first_grace.edits.is_empty());

    bots.last_mut().unwrap().health = 60;
    let renewed_progress = controller.decide(
        &direct_attack_tuning(),
        &GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &bots,
            structures: &[alternative],
            deposits: &[],
            terrain: &[],
            charge: &DEFAULT_CHARGE,
            reachability: None,
            route_query: None,
            tick: WORK_WINDOW_TICKS * 3,
            finished: false,
        },
    );
    assert!(renewed_progress.reviewed);
    assert!(renewed_progress.edits.is_empty());
    let reset_grace = controller.decide(
        &direct_attack_tuning(),
        &GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &bots,
            structures: &[alternative],
            deposits: &[],
            terrain: &[],
            charge: &DEFAULT_CHARGE,
            reachability: None,
            route_query: None,
            tick: WORK_WINDOW_TICKS * 4,
            finished: false,
        },
    );
    assert!(reset_grace.reviewed);
    assert!(reset_grace.edits.is_empty());
    let reassessed = controller.decide(
        &direct_attack_tuning(),
        &GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &bots,
            structures: &[alternative],
            deposits: &[],
            terrain: &[],
            charge: &DEFAULT_CHARGE,
            reachability: None,
            route_query: None,
            tick: WORK_WINDOW_TICKS * 5,
            finished: false,
        },
    );
    assert!(reassessed.edits.iter().any(|edit| {
        edit.action == IntentEditAction::Paint
            && edit.kind == IntentKind::Defend
            && edit.cell == alternative_cell
    }));
}

#[test]
fn hunt_paint_holds_until_the_target_leaves_its_block() {
    let mut grid = IntentGrid::new(64, 64);
    let owner = SwarmId(4);
    let enemy = SwarmId(9);
    let swarms = [
        SwarmState {
            id: owner,
            home: world(IVec2::ZERO),
            minerals: 100,
            mineral_income_per_second: 0.0,
            production_saturated: false,
        },
        SwarmState {
            id: enemy,
            home: world(IVec2::new(20, 20)),
            minerals: 0,
            mineral_income_per_second: 0.0,
            production_saturated: false,
        },
    ];
    let deposit = [DepositState {
        id: 20,
        owner: Some(owner),
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
        max_health: 100,
        charge: 1.0,
        max_charge: 1.0,
        cargo: 0,
    };
    let mut controller = Controller::new(owner);
    let first = controller.decide(
        &direct_attack_tuning(),
        &GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &[remnant],
            structures: &[],
            deposits: &deposit,
            terrain: &[],
            charge: &DEFAULT_CHARGE,
            reachability: None,
            route_query: None,
            tick: 0,
            finished: false,
        },
    );
    assert!(first.explanation.contains("hunt remaining Nanobot 50"));
    assert!(first.edits.iter().any(|edit| {
        edit.action == IntentEditAction::Paint
            && edit.kind == IntentKind::Defend
            && edit.cell == IVec2::new(12, 3)
    }));
    apply(&mut grid, owner, &first);

    let first_block = owned_defend_cells(&grid, owner);
    for (review, cell) in [IVec2::new(13, 3), IVec2::new(12, 4), IVec2::new(11, 3)]
        .into_iter()
        .enumerate()
    {
        assert!(first_block.contains(&(cell.x, cell.y)));
        let inside = controller.decide(
            &direct_attack_tuning(),
            &GameState {
                swarms: &swarms,
                bots: &[BotState {
                    position: world(cell),
                    ..remnant
                }],
                deposits: &deposit,
                ..empty_state(&grid, (review as u64 + 1) * WORK_WINDOW_TICKS)
            },
        );
        assert!(inside.reviewed);
        assert!(inside.edits.is_empty(), "{inside:?}");
    }
    let moved = BotState {
        position: world(IVec2::new(14, 3)),
        ..remnant
    };
    let relocated = controller.decide(
        &direct_attack_tuning(),
        &GameState {
            swarms: &swarms,
            bots: &[moved],
            deposits: &deposit,
            ..empty_state(&grid, 3 * WORK_WINDOW_TICKS + 1)
        },
    );
    assert!(relocated.reviewed);
    assert!(relocated.edits.iter().any(|edit| {
        edit.action == IntentEditAction::Paint
            && edit.kind == IntentKind::Defend
            && edit.cell == IVec2::new(14, 3)
    }));
    apply(&mut grid, owner, &relocated);
    assert!(!owned_defend_cells(&grid, owner).contains(&(11, 3)));

    let disappeared = controller.decide(
        &direct_attack_tuning(),
        &GameState {
            swarms: &swarms,
            deposits: &deposit,
            ..empty_state(&grid, 3 * WORK_WINDOW_TICKS + 2)
        },
    );
    assert!(disappeared.reviewed);
    assert!(disappeared.edits.iter().any(|edit| {
        edit.action == IntentEditAction::Erase
            && edit.kind == IntentKind::Defend
            && edit.cell == IVec2::new(14, 3)
    }));
}

#[test]
fn controllers_stop_planning_after_the_match_finishes() {
    let grid = IntentGrid::new(16, 16);
    let state = GameState {
        finished: true,
        ..empty_state(&grid, 0)
    };

    let decision =
        Controller::new(SwarmId(4)).decide(&crate::tuning::AutonomyTuning::default(), &state);
    assert!(!decision.reviewed);
    assert!(decision.edits.is_empty());
    assert_eq!(decision.work_units, 0);
}

#[test]
fn mature_funded_force_retains_attack_and_mining_after_expansion() {
    let mut grid = IntentGrid::new(96, 96);
    let owner = SwarmId(4);
    let enemy = SwarmId(9);
    let home = world(IVec2::ZERO);
    let enemy_home = world(IVec2::new(32, 0));
    let swarms = [
        SwarmState {
            id: owner,
            home,
            minerals: 0,
            mineral_income_per_second: 0.0,
            production_saturated: false,
        },
        SwarmState {
            id: enemy,
            home: enemy_home,
            minerals: 0,
            mineral_income_per_second: 0.0,
            production_saturated: false,
        },
    ];
    let economy_bots = mature_economy_bots(owner, home);
    let deposits = [
        DepositState {
            id: 20,
            owner: None,
            position: world(IVec2::new(-1, 0)),
            amount: 72_000,
            radius: 64.0,
        },
        DepositState {
            id: 21,
            owner: None,
            position: world(IVec2::new(4, 0)),
            amount: 72_000,
            radius: 64.0,
        },
    ];
    let primary_chain = funded_primary_chain(owner, home, deposits[0]);
    let enemy_facility = StructureState {
        id: 40,
        owner: enemy,
        position: enemy_home,
        kind: StructureKind::Facility,
        health: 100,
        max_health: 100,
        minerals: 0,
    };
    let mut controller = Controller::new(owner);
    let initial = controller.decide(
        &direct_attack_tuning(),
        &GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &economy_bots,
            structures: &[primary_chain[0], primary_chain[1], enemy_facility],
            deposits: &deposits,
            terrain: &[],
            charge: &DEFAULT_CHARGE,
            reachability: None,
            route_query: None,
            tick: 0,
            finished: false,
        },
    );
    apply(&mut grid, owner, &initial);
    assert!(
        grid.cell(IVec2::new(-1, 0))
            .is_some_and(|intent| intent.has_owned(IntentKind::Gather, owner))
    );
    assert!(
        !grid
            .cell(IVec2::new(4, 0))
            .is_some_and(|intent| intent.has_owned(IntentKind::Gather, owner))
    );

    let supplied_swarms = [
        SwarmState {
            minerals: 2_000,
            mineral_income_per_second: 0.0,
            production_saturated: false,
            ..swarms[0]
        },
        swarms[1],
    ];
    let bots = economy_bots
        .into_iter()
        .chain((0..10).map(|id| BotState {
            id: 100 + id,
            owner,
            kind: NanobotType::Defender,
            position: home,
            health: 100,
            max_health: 100,
            charge: 1.0,
            max_charge: 1.0,
            cargo: 0,
        }))
        .collect::<Vec<_>>();
    let expansion = controller.decide(
        &AutonomyTuning::default(),
        &GameState {
            grid: &grid,
            swarms: &supplied_swarms,
            bots: &bots,
            structures: &[primary_chain[0], primary_chain[1], enemy_facility],
            deposits: &deposits,
            ..empty_state(&grid, WORK_WINDOW_TICKS)
        },
    );
    apply(&mut grid, owner, &expansion);
    for review in 2..=120 {
        let pressure = controller.decide(
            &direct_attack_tuning(),
            &GameState {
                grid: &grid,
                swarms: &supplied_swarms,
                bots: &bots,
                structures: &[primary_chain[0], primary_chain[1], enemy_facility],
                deposits: &deposits,
                terrain: &[],
                charge: &DEFAULT_CHARGE,
                reachability: None,
                route_query: None,
                tick: review * WORK_WINDOW_TICKS,
                finished: false,
            },
        );
        assert!(pressure.reviewed);
        assert!(
            pressure.explanation.contains("staging point [32, 0]"),
            "pressure decision lacked diagnostic coordinates: {}",
            pressure.explanation
        );
        apply(&mut grid, owner, &pressure);
        assert!(grid.iter_active_cells().any(|(cell, intent)| {
            cell == IVec2::ZERO && intent.has_owned(IntentKind::Build, owner)
        }));
        assert!(
            grid.cell(IVec2::new(32, 0))
                .is_some_and(|intent| intent.has_owned(IntentKind::Defend, owner)),
            "pressure disappeared at review {review}"
        );
        assert!(
            grid.cell(IVec2::new(-1, 0))
                .is_some_and(|intent| intent.has_owned(IntentKind::Gather, owner)),
            "pressure abandoned the original mine"
        );
        assert!(
            grid.cell(IVec2::new(4, 0))
                .is_some_and(|intent| intent.has_owned(IntentKind::Gather, owner)),
            "pressure abandoned the expanded mine"
        );
    }
}

#[test]
fn live_scoring_review_changes_intent_without_refilling_the_planning_allowance() {
    let owner = SwarmId::PLAYER;
    let target = IVec2::new(6, 0);
    let enemy = BotState {
        id: 9,
        owner: SwarmId(2),
        kind: NanobotType::Worker,
        position: world(target),
        health: 100,
        max_health: 100,
        charge: 1.0,
        max_charge: 1.0,
        cargo: 0,
    };
    let swarms = [SwarmState {
        id: owner,
        home: world(IVec2::ZERO),
        minerals: 0,
        mineral_income_per_second: 0.0,
        production_saturated: false,
    }];
    let bots = [enemy];
    let mut grid = IntentGrid::new(32, 32);
    let mut tuning = direct_attack_tuning();
    let mut controller = Controller::new(owner);
    let initial = controller.decide(
        &tuning,
        &GameState {
            swarms: &swarms,
            bots: &bots,
            ..empty_state(&grid, 0)
        },
    );
    assert!(initial.explanation.contains("hunt remaining Nanobot"));
    apply(&mut grid, owner, &initial);
    assert!(
        grid.cell(target)
            .unwrap()
            .has_owned(IntentKind::Defend, owner)
    );

    tuning.strategy_hold_base = 10_000.0;
    controller.request_tuning_review();
    let revised = controller.decide(
        &tuning,
        &GameState {
            swarms: &swarms,
            bots: &bots,
            ..empty_state(&grid, 0)
        },
    );
    assert!(revised.reviewed);
    assert!(revised.explanation.contains("hold the home economy"));
    apply(&mut grid, owner, &revised);
    assert!(
        !grid
            .cell(target)
            .unwrap()
            .has_owned(IntentKind::Defend, owner)
    );

    let mut work = initial.work_units + revised.work_units;
    let mut deferred = false;
    for _ in 0..PLANNING_WORK_BUDGET {
        controller.request_tuning_review();
        let decision = controller.decide(
            &tuning,
            &GameState {
                swarms: &swarms,
                bots: &bots,
                ..empty_state(&grid, 0)
            },
        );
        work += decision.work_units;
        if decision.explanation.contains("allowance exhausted") {
            deferred = true;
            break;
        }
    }
    assert!(deferred, "repeated edits must eventually defer planning");
    assert!(
        work <= PLANNING_WORK_BUDGET,
        "tuning reviews share the existing bounded allowance"
    );
}

#[test]
fn wounded_target_priority_uses_the_edited_health_maximum() {
    let owner = SwarmId::PLAYER;
    let enemy = SwarmId(2);
    let grid = IntentGrid::new(32, 32);
    let swarms = [SwarmState {
        id: owner,
        home: world(IVec2::ZERO),
        minerals: 0,
        mineral_income_per_second: 0.0,
        production_saturated: false,
    }];
    let mut structure = StructureState {
        id: 9,
        owner: enemy,
        position: world(IVec2::new(6, 0)),
        kind: StructureKind::Facility,
        health: 100,
        max_health: 100,
        minerals: 0,
    };
    let mut tuning = AutonomyTuning {
        strategy_hold_base: 135.0,
        ..AutonomyTuning::default()
    };
    let decide_structure = |target| {
        Controller::new(owner).decide(
            &tuning,
            &GameState {
                swarms: &swarms,
                structures: &[target],
                ..empty_state(&grid, 0)
            },
        )
    };
    assert!(
        decide_structure(structure)
            .explanation
            .contains("hold the home economy")
    );
    structure.max_health = 200;
    assert!(
        decide_structure(structure)
            .explanation
            .contains("attack enemy logistics structure 9"),
        "half-health logistics must receive its wound priority with a larger maximum"
    );

    let mut bot = BotState {
        id: 12,
        owner: enemy,
        kind: NanobotType::Worker,
        position: world(IVec2::new(6, 0)),
        health: 100,
        max_health: 100,
        charge: 1.0,
        max_charge: 1.0,
        cargo: 0,
    };
    tuning.strategy_hold_base = 520.0;
    let decide_bot = |target| {
        Controller::new(owner).decide(
            &tuning,
            &GameState {
                swarms: &swarms,
                bots: &[target],
                ..empty_state(&grid, 0)
            },
        )
    };
    assert!(
        decide_bot(bot)
            .explanation
            .contains("hold the home economy")
    );
    bot.max_health = 200;
    assert!(
        decide_bot(bot)
            .explanation
            .contains("hunt remaining Nanobot 12"),
        "half-health Nanobots must receive their wound priority with a larger maximum"
    );
}

fn owned_defend_cells(grid: &IntentGrid, owner: SwarmId) -> BTreeSet<(i32, i32)> {
    grid.iter_active_cells()
        .filter(|(_, cell)| cell.has_owned(IntentKind::Defend, owner))
        .map(|(cell, _)| (cell.x, cell.y))
        .collect()
}

#[test]
fn a_compact_front_leads_the_army_and_waits_for_most_defenders() {
    let owner = SwarmId::PLAYER;
    let mut grid = IntentGrid::new(24, 24);
    let swarms = [SwarmState {
        id: owner,
        home: world(IVec2::ZERO),
        minerals: 2_000,
        mineral_income_per_second: 0.0,
        production_saturated: false,
    }];
    let target = StructureState {
        id: 10,
        owner: SwarmId(9),
        position: world(IVec2::new(9, 0)),
        kind: StructureKind::Sink,
        health: 100,
        max_health: 100,
        minerals: 0,
    };
    let defenders = (300..304).map(|id| BotState {
        id,
        owner,
        kind: NanobotType::Defender,
        position: swarms[0].home,
        health: 100,
        max_health: 100,
        charge: 1.0,
        max_charge: 1.0,
        cargo: 0,
    });
    let mut defenders = defenders.collect::<Vec<_>>();
    let mut controller = Controller::new(owner);
    let mut tuning = AutonomyTuning {
        controller_review_period_ticks: WORK_WINDOW_TICKS,
        ..AutonomyTuning::default()
    };
    let workers = (0..300)
        .map(|id| BotState {
            id,
            kind: NanobotType::Worker,
            ..defenders[0]
        })
        .collect::<Vec<_>>();
    let review = |controller: &mut Controller,
                  grid: &mut IntentGrid,
                  tuning: &AutonomyTuning,
                  defenders: &[BotState],
                  tick| {
        let bots = workers.iter().chain(defenders).copied().collect::<Vec<_>>();
        let decision = controller.decide(
            tuning,
            &GameState {
                swarms: &swarms,
                bots: &bots,
                structures: &[target],
                ..empty_state(grid, tick)
            },
        );
        assert!(decision.reviewed);
        apply(grid, owner, &decision);
    };

    review(&mut controller, &mut grid, &tuning, &defenders, 0);
    let first = owned_defend_cells(&grid, owner);
    assert!(
        first.iter().all(|&(x, _)| (1..9).contains(&x)) && !first.is_empty(),
        "the intermediate front must lead the army: {first:?}"
    );
    let min_x = first.iter().map(|&(x, _)| x).min().unwrap();
    let max_x = first.iter().map(|&(x, _)| x).max().unwrap();
    let min_y = first.iter().map(|&(_, y)| y).min().unwrap();
    let max_y = first.iter().map(|&(_, y)| y).max().unwrap();
    assert!(min_y < max_y && min_x < max_x);
    assert_eq!(
        first.len(),
        ((max_x - min_x + 1) * (max_y - min_y + 1)) as usize,
        "the intermediate Defend block must have no holes: {first:?}"
    );

    for defender in &mut defenders {
        defender.position = world(IVec2::new(1, 3));
    }
    review(
        &mut controller,
        &mut grid,
        &tuning,
        &defenders,
        WORK_WINDOW_TICKS,
    );
    assert_eq!(owned_defend_cells(&grid, owner), first);

    for defender in &mut defenders[..2] {
        defender.position = world(IVec2::new(min_x, min_y));
    }
    for defender in &mut defenders[2..] {
        defender.position = world(IVec2::new(1, -3));
    }
    review(
        &mut controller,
        &mut grid,
        &tuning,
        &defenders,
        2 * WORK_WINDOW_TICKS,
    );
    assert_eq!(
        owned_defend_cells(&grid, owner),
        first,
        "half the force has arrived"
    );

    defenders[2].position = world(IVec2::new(max_x, max_y));
    tuning.strategy_staging_arrival_fraction = 0.9;
    controller.request_tuning_review();
    review(
        &mut controller,
        &mut grid,
        &tuning,
        &defenders,
        2 * WORK_WINDOW_TICKS + 1,
    );
    assert_eq!(
        owned_defend_cells(&grid, owner),
        first,
        "the new threshold still requires a fourth arrival"
    );

    tuning.strategy_staging_arrival_fraction = 0.6;
    controller.request_tuning_review();
    review(
        &mut controller,
        &mut grid,
        &tuning,
        &defenders,
        2 * WORK_WINDOW_TICKS + 2,
    );
    let next = owned_defend_cells(&grid, owner);
    assert!(next.iter().any(|&(x, _)| x > max_x));
    assert!(
        !next.contains(&(min_x, min_y)),
        "the old front must be erased"
    );
    assert_eq!(next.iter().map(|&(_, y)| y).min(), Some(min_y));
}

#[test]
fn staging_follows_the_supplied_approach_and_keeps_its_flank() {
    use std::cell::Cell;

    use crate::navigation::Route;

    let owner = SwarmId::PLAYER;
    let mut grid = IntentGrid::new(32, 32);
    let swarms = [SwarmState {
        id: owner,
        home: world(IVec2::ZERO),
        minerals: 2_000,
        mineral_income_per_second: 0.0,
        production_saturated: false,
    }];
    let target = StructureState {
        id: 10,
        owner: SwarmId(9),
        position: world(IVec2::new(12, 0)),
        kind: StructureKind::Sink,
        health: 100,
        max_health: 100,
        minerals: 0,
    };
    let side = Cell::new(-6);
    let route = |_, goal, _: &mut dyn FnMut() -> bool| {
        let waypoints = match goal {
            RouteGoal::Range { center, .. } => vec![
                world(IVec2::new(0, side.get())),
                world(IVec2::new(12, side.get())),
                center,
            ],
            RouteGoal::Point(center) => vec![center],
            RouteGoal::Interaction(_) => unreachable!(),
        };
        Some(RouteOutcome::Found(Route {
            waypoints,
            cost: 0.0,
        }))
    };
    let defender = BotState {
        id: 1,
        owner,
        kind: NanobotType::Defender,
        position: swarms[0].home,
        health: 100,
        max_health: 100,
        charge: 1.0,
        max_charge: 1.0,
        cargo: 0,
    };
    let mut controller = Controller::new(owner);
    let mut tuning = AutonomyTuning::default();
    let review = |controller: &mut Controller,
                  grid: &mut IntentGrid,
                  tuning: &AutonomyTuning,
                  defender,
                  tick| {
        let decision = controller.decide(
            tuning,
            &GameState {
                swarms: &swarms,
                bots: &[defender],
                structures: &[target],
                route_query: Some(&route),
                ..empty_state(grid, tick)
            },
        );
        assert!(decision.reviewed);
        apply(grid, owner, &decision);
    };

    review(&mut controller, &mut grid, &tuning, defender, 0);
    let first = owned_defend_cells(&grid, owner);
    assert!(
        first.contains(&(0, -4)),
        "the block must follow the reachable approach: {first:?}"
    );

    side.set(6);
    review(
        &mut controller,
        &mut grid,
        &tuning,
        BotState {
            position: world(IVec2::new(1, 2)),
            ..defender
        },
        WORK_WINDOW_TICKS,
    );
    assert_eq!(owned_defend_cells(&grid, owner), first);

    review(
        &mut controller,
        &mut grid,
        &tuning,
        BotState {
            position: world(IVec2::new(0, -4)),
            ..defender
        },
        2 * WORK_WINDOW_TICKS,
    );
    let next = owned_defend_cells(&grid, owner);
    assert!(
        next.contains(&(2, -6)) && !next.contains(&(0, -4)),
        "{next:?}"
    );

    tuning.strategy_staging_spacing_cells = 64;
    controller.request_tuning_review();
    review(
        &mut controller,
        &mut grid,
        &tuning,
        defender,
        2 * WORK_WINDOW_TICKS + 1,
    );
    assert_eq!(
        owned_defend_cells(&grid, owner),
        next,
        "tuning must keep the active block"
    );

    review(
        &mut controller,
        &mut grid,
        &tuning,
        BotState {
            position: world(IVec2::new(2, -6)),
            ..defender
        },
        3 * WORK_WINDOW_TICKS + 1,
    );
    let final_block = owned_defend_cells(&grid, owner);
    assert!(
        final_block.contains(&(12, 0)),
        "the new spacing must apply to the remaining route: {final_block:?}"
    );
}

#[test]
fn defend_paint_follows_income_and_preserves_chargers_when_reserves_fall() {
    let owner = SwarmId::PLAYER;
    let home = IVec2::ZERO;
    let charger_cell = IVec2::new(6, 4);
    let mut grid = IntentGrid::new(24, 24);
    let mut controller = Controller::new(owner);
    let tuning = AutonomyTuning::default();
    let mut swarms = [SwarmState {
        id: owner,
        home: world(home),
        minerals: tuning.strategy_mineral_reserve,
        mineral_income_per_second: 0.0,
        production_saturated: false,
    }];
    let primary = StructureState {
        id: 27,
        owner,
        position: world(charger_cell),
        kind: StructureKind::Charger,
        health: 100,
        max_health: 100,
        minerals: tuning.strategy_charger_reserve,
    };
    let mut chargers = [
        primary,
        StructureState {
            id: primary.id + 1,
            position: world(charger_cell + IVec2::X),
            ..primary
        },
    ];
    let mut review =
        |grid: &mut IntentGrid, swarms: &[SwarmState], chargers: &[StructureState], tick| {
            let decision = controller.decide(
                &tuning,
                &GameState {
                    swarms,
                    structures: chargers,
                    ..empty_state(grid, tick)
                },
            );
            apply(grid, owner, &decision);
            assert!(
                grid.cell(charger_cell)
                    .is_some_and(|cell| cell.has_owned(IntentKind::Defend, owner)),
                "the operational Charger remains accessible through the review"
            );
            grid.iter_active_cells()
                .filter(|(_, cell)| cell.has_owned(IntentKind::Defend, owner))
                .count()
        };
    let initial = review(&mut grid, &swarms, &chargers, 0);
    swarms[0].mineral_income_per_second = 12.0;
    let supplied = review(&mut grid, &swarms, &chargers, WORK_WINDOW_TICKS);
    assert!(supplied > initial, "sustained income grows Defend paint");
    swarms[0].minerals = 0;
    assert!(
        review(&mut grid, &swarms, &chargers, 2 * WORK_WINDOW_TICKS) < supplied,
        "bank depletion shrinks Defend paint even while extraction continues"
    );
    swarms[0].minerals = tuning.strategy_mineral_reserve;
    chargers.iter_mut().for_each(|charger| charger.minerals = 0);
    assert!(
        review(&mut grid, &swarms, &chargers, 3 * WORK_WINDOW_TICKS) < supplied,
        "a depleted Charger shrinks Defend paint even with minerals banked elsewhere"
    );
}

#[test]
fn production_room_requires_saturation_and_mineral_surplus() {
    let owner = SwarmId::PLAYER;
    let home = IVec2::new(1, 2);
    let grid = IntentGrid::new(16, 16);
    let tuning = AutonomyTuning::default();
    let builds = |production_saturated, minerals| {
        let swarms = [SwarmState {
            id: owner,
            home: world(home),
            minerals,
            mineral_income_per_second: 0.0,
            production_saturated,
        }];
        let decision = Controller::new(owner).decide(
            &tuning,
            &GameState {
                swarms: &swarms,
                ..empty_state(&grid, 0)
            },
        );
        decision
            .edits
            .into_iter()
            .filter(|edit| edit.kind == IntentKind::Build && edit.action == IntentEditAction::Paint)
            .map(|edit| edit.cell)
            .collect::<Vec<_>>()
    };
    let threshold = tuning.strategy_production_mineral_threshold;
    assert_eq!(builds(false, threshold + 1), [home]);
    assert_eq!(builds(true, threshold), [home]);
    let expanded = builds(true, threshold + 1);
    assert!(
        expanded.len() > 1,
        "saturation with surplus minerals adds Build room"
    );
    assert!(
        expanded
            .iter()
            .all(|&cell| (cell - home).abs().element_sum() <= 1)
    );
}
