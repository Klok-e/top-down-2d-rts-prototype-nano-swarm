use super::{materialize::*, *};

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
        tick,
        finished: false,
    }
}

fn mature_economy_bots(owner: SwarmId, home: Vec2) -> Vec<BotState> {
    (0..4)
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
        },
        SwarmState {
            id: enemy,
            home: world(IVec2::new(8, 0)),
            minerals: 20,
        },
    ];
    let bots = [
        BotState {
            id: 1,
            owner,
            kind: NanobotType::Worker,
            position: swarms[0].home,
            health: 100,
            charge: 1.0,
            cargo: 0,
        },
        BotState {
            id: 2,
            owner,
            kind: NanobotType::Hauler,
            position: swarms[0].home,
            health: 100,
            charge: 1.0,
            cargo: 0,
        },
        BotState {
            id: 3,
            owner,
            kind: NanobotType::Defender,
            position: swarms[0].home,
            health: 100,
            charge: 0.3,
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
            minerals: 0,
        },
        StructureState {
            id: 11,
            owner: enemy,
            position: world(IVec2::new(7, 0)),
            kind: StructureKind::Sink,
            health: 100,
            minerals: 80,
        },
    ];
    let deposits = [DepositState {
        id: 20,
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
        tick: 0,
        finished: false,
    };

    let decision = Controller::new(owner).decide(&state);

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
fn raid_retains_the_established_primary() {
    let mut grid = IntentGrid::new(32, 32);
    let owner = SwarmId(4);
    let enemy = SwarmId(9);
    let home = world(IVec2::ZERO);
    let initial_swarms = [SwarmState {
        id: owner,
        home,
        minerals: 0,
    }];
    let economy_bots = mature_economy_bots(owner, home);
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
    ];
    let primary_chain = funded_primary_chain(owner, home, deposits[0]);
    let mut controller = Controller::new(owner);
    let initial = controller.decide(&GameState {
        grid: &grid,
        swarms: &initial_swarms,
        bots: &economy_bots,
        structures: &primary_chain,
        deposits: &deposits,
        terrain: &[],
        tick: 0,
        finished: false,
    });
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
            ..initial_swarms[0]
        },
        SwarmState {
            id: enemy,
            home: world(IVec2::new(8, 0)),
            minerals: 0,
        },
    ];
    let defenders = (0..8).map(|id| BotState {
        id: 100 + id,
        owner,
        kind: NanobotType::Defender,
        position: home,
        health: 100,
        charge: 1.0,
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
    let raid = controller.decide(&GameState {
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
        tick: REVIEW_PERIOD_TICKS,
        finished: false,
    });

    assert!(raid.explanation.contains("attack enemy logistics"));
    assert!(!raid.edits.iter().any(|edit| {
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
        },
        SwarmState {
            id: enemy,
            home: world(IVec2::new(8, 0)),
            minerals: 0,
        },
    ];
    let bots = (0..8)
        .map(|id| BotState {
            id,
            owner,
            kind: NanobotType::Defender,
            position: swarms[0].home,
            health: 100,
            charge: 1.0,
            cargo: 0,
        })
        .collect::<Vec<_>>();
    let deposits = [DepositState {
        id: 20,
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
        minerals: 80,
    }];

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

    assert!(decision.explanation.contains("attack enemy logistics"));
    assert!(decision.edits.iter().any(|edit| {
        edit.action == IntentEditAction::Paint
            && edit.kind == IntentKind::Gather
            && edit.cell == IVec2::new(-1, 0)
    }));
    assert!(decision.edits.iter().any(|edit| {
        edit.action == IntentEditAction::Paint
            && edit.kind == IntentKind::Build
            && edit.cell != IVec2::ZERO
            && edit.cell.abs().max_element() <= 1
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
        },
        SwarmState {
            id: enemy,
            home: world(IVec2::new(12, 0)),
            minerals: 0,
        },
    ];
    let friendly_defenders = (0..2)
        .map(|id| BotState {
            id,
            owner,
            kind: NanobotType::Defender,
            position: swarms[0].home,
            health: 100,
            charge: 1.0,
            cargo: 0,
        })
        .collect::<Vec<_>>();
    let rich_charger = StructureState {
        id: 30,
        owner: enemy,
        position: world(IVec2::new(6, 2)),
        kind: StructureKind::Charger,
        health: 1,
        minerals: 1_000,
    };
    let damage_target = BotState {
        id: 50,
        owner: enemy,
        kind: NanobotType::Worker,
        position: world(IVec2::new(6, -2)),
        health: 100,
        charge: 1.0,
        cargo: 0,
    };
    let mut bots = friendly_defenders.clone();
    bots.push(damage_target);
    let mut controller = Controller::new(owner);

    let initial = controller.decide(&GameState {
        grid: &grid,
        swarms: &swarms,
        bots: &bots,
        structures: &[],
        deposits: &[],
        terrain: &[],
        tick: 0,
        finished: false,
    });
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
    let revised = controller.decide(&GameState {
        grid: &grid,
        swarms: &swarms,
        bots: &bots,
        structures: &[rich_charger],
        deposits: &[],
        terrain: &[],
        tick: REVIEW_PERIOD_TICKS,
        finished: false,
    });
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
fn progressing_live_attack_remains_when_a_supported_new_target_appears() {
    let mut grid = IntentGrid::new(64, 64);
    let owner = SwarmId(4);
    let enemy = SwarmId(9);
    let swarms = [
        SwarmState {
            id: owner,
            home: world(IVec2::ZERO),
            minerals: 2_000,
        },
        SwarmState {
            id: enemy,
            home: world(IVec2::new(24, 24)),
            minerals: 0,
        },
    ];
    let bots = (0..10)
        .map(|id| BotState {
            id,
            owner,
            kind: NanobotType::Defender,
            position: swarms[0].home,
            health: 100,
            charge: 1.0,
            cargo: 0,
        })
        .collect::<Vec<_>>();
    let friendly_charger = StructureState {
        id: 20,
        owner,
        position: world(IVec2::new(1, 0)),
        kind: StructureKind::Charger,
        health: 100,
        minerals: 100,
    };
    let incumbent = StructureState {
        id: 30,
        owner: enemy,
        position: world(IVec2::new(24, 24)),
        kind: StructureKind::Sink,
        health: 100,
        minerals: 0,
    };
    let mut controller = Controller::new(owner);
    let initial = controller.decide(&GameState {
        grid: &grid,
        swarms: &swarms,
        bots: &bots,
        structures: &[friendly_charger, incumbent],
        deposits: &[],
        terrain: &[],
        tick: 0,
        finished: false,
    });
    apply(&mut grid, owner, &initial);
    assert!(
        grid.cell(IVec2::new(24, 24))
            .is_some_and(|intent| { intent.has_owned(IntentKind::Defend, owner) })
    );

    let progressing_incumbent = StructureState {
        health: 80,
        ..incumbent
    };
    let new_forward_target = StructureState {
        id: 31,
        owner: enemy,
        position: world(IVec2::new(10, 10)),
        kind: StructureKind::Charger,
        health: 5,
        minerals: 1_000,
    };
    let revised = controller.decide(&GameState {
        grid: &grid,
        swarms: &swarms,
        bots: &bots,
        structures: &[friendly_charger, progressing_incumbent, new_forward_target],
        deposits: &[],
        terrain: &[],
        tick: REVIEW_PERIOD_TICKS,
        finished: false,
    });
    apply(&mut grid, owner, &revised);

    assert!(
        grid.cell(IVec2::new(24, 24))
            .is_some_and(|intent| { intent.has_owned(IntentKind::Defend, owner) })
    );
    assert!(
        grid.cell(IVec2::new(10, 10))
            .is_some_and(|intent| { intent.has_owned(IntentKind::Defend, owner) })
    );
    assert!(
        grid.iter_active_cells()
            .filter(|(_, intent)| IntentKind::ALL
                .into_iter()
                .any(|kind| intent.has_owned(kind, owner)))
            .count()
            <= MAX_PLAN_CELLS
    );

    let depleted_defenders = bots
        .iter()
        .map(|bot| BotState {
            charge: 0.1,
            ..*bot
        })
        .collect::<Vec<_>>();
    let empty_charger = StructureState {
        minerals: 0,
        ..friendly_charger
    };
    let unsupported = controller.decide(&GameState {
        grid: &grid,
        swarms: &swarms,
        bots: &depleted_defenders,
        structures: &[empty_charger, progressing_incumbent, new_forward_target],
        deposits: &[],
        terrain: &[],
        tick: REVIEW_PERIOD_TICKS * 2,
        finished: false,
    });
    apply(&mut grid, owner, &unsupported);

    let retained_fronts = [IVec2::new(24, 24), IVec2::new(10, 10)]
        .into_iter()
        .filter(|cell| {
            grid.cell(*cell)
                .is_some_and(|intent| intent.has_owned(IntentKind::Defend, owner))
        })
        .count();
    assert_eq!(retained_fronts, 1);
}

#[test]
fn supported_attack_targets_a_recovery_bot_while_enemy_structures_remain() {
    let grid = IntentGrid::new(32, 32);
    let owner = SwarmId(4);
    let enemy = SwarmId(9);
    let swarms = [
        SwarmState {
            id: owner,
            home: world(IVec2::ZERO),
            minerals: 2_000,
        },
        SwarmState {
            id: enemy,
            home: world(IVec2::new(12, 0)),
            minerals: 0,
        },
    ];
    let mut bots = (0..8)
        .map(|id| BotState {
            id,
            owner,
            kind: NanobotType::Defender,
            position: swarms[0].home,
            health: 100,
            charge: 1.0,
            cargo: 0,
        })
        .collect::<Vec<_>>();
    bots.push(BotState {
        id: 50,
        owner: enemy,
        kind: NanobotType::Worker,
        position: world(IVec2::new(3, 5)),
        health: 100,
        charge: 1.0,
        cargo: 0,
    });
    let structures = [
        StructureState {
            id: 20,
            owner,
            position: world(IVec2::new(1, 0)),
            kind: StructureKind::Charger,
            health: 100,
            minerals: 100,
        },
        StructureState {
            id: 30,
            owner: enemy,
            position: world(IVec2::new(12, 0)),
            kind: StructureKind::Sink,
            health: 100,
            minerals: 0,
        },
    ];

    let decision = Controller::new(owner).decide(&GameState {
        grid: &grid,
        swarms: &swarms,
        bots: &bots,
        structures: &structures,
        deposits: &[],
        terrain: &[],
        tick: 0,
        finished: false,
    });

    assert!(decision.edits.iter().any(|edit| {
        edit.action == IntentEditAction::Paint
            && edit.kind == IntentKind::Defend
            && edit.cell == IVec2::new(12, 0)
    }));
    assert!(decision.edits.iter().any(|edit| {
        edit.action == IntentEditAction::Paint
            && edit.kind == IntentKind::Defend
            && edit.cell == IVec2::new(3, 5)
    }));
}

#[test]
fn progressing_incumbent_wins_redeployment_when_only_one_front_is_supportable() {
    let mut grid = IntentGrid::new(32, 32);
    let owner = SwarmId(4);
    let enemy = SwarmId(9);
    let swarms = [
        SwarmState {
            id: owner,
            home: world(IVec2::ZERO),
            minerals: 2_000,
        },
        SwarmState {
            id: enemy,
            home: world(IVec2::new(12, 0)),
            minerals: 0,
        },
    ];
    let bots = (0..2)
        .map(|id| BotState {
            id,
            owner,
            kind: NanobotType::Defender,
            position: swarms[0].home,
            health: 100,
            charge: 1.0,
            cargo: 0,
        })
        .collect::<Vec<_>>();
    let friendly_charger = StructureState {
        id: 20,
        owner,
        position: world(IVec2::new(1, 0)),
        kind: StructureKind::Charger,
        health: 100,
        minerals: 100,
    };
    let incumbent = StructureState {
        id: 30,
        owner: enemy,
        position: world(IVec2::new(12, 0)),
        kind: StructureKind::Sink,
        health: 100,
        minerals: 0,
    };
    let mut controller = Controller::new(owner);
    let initial = controller.decide(&GameState {
        grid: &grid,
        swarms: &swarms,
        bots: &bots,
        structures: &[friendly_charger, incumbent],
        deposits: &[],
        terrain: &[],
        tick: 0,
        finished: false,
    });
    apply(&mut grid, owner, &initial);

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
        minerals: 1_000,
    };
    let revised = controller.decide(&GameState {
        grid: &grid,
        swarms: &swarms,
        bots: &bots,
        structures: &[friendly_charger, progressing_incumbent, new_forward_target],
        deposits: &[],
        terrain: &[],
        tick: REVIEW_PERIOD_TICKS,
        finished: false,
    });
    apply(&mut grid, owner, &revised);

    assert!(
        revised
            .explanation
            .contains("retained attack enemy logistics structure 30"),
        "{}",
        revised.explanation
    );
    assert!(!revised.explanation.contains("structure 31"));
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
    }];
    let deposits = [DepositState {
        id: 20,
        position: world(IVec2::new(2, 0)),
        amount: 500,
        radius: 100.0,
    }];
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
    apply(&mut grid, owner, &initial);

    let between = controller.decide(&GameState {
        grid: &grid,
        swarms: &swarms,
        bots: &[],
        structures: &[],
        deposits: &deposits,
        terrain: &[],
        tick: REVIEW_PERIOD_TICKS - 1,
        finished: false,
    });
    assert!(!between.reviewed);
    assert!(between.edits.is_empty());

    let regular = controller.decide(&GameState {
        grid: &grid,
        swarms: &swarms,
        bots: &[],
        structures: &[],
        deposits: &deposits,
        terrain: &[],
        tick: REVIEW_PERIOD_TICKS,
        finished: false,
    });
    assert!(regular.reviewed);
    assert!(regular.edits.is_empty());
    assert!(regular.explanation.contains("retained viable"));
    assert!(regular.work_units <= PLANNING_WORK_BUDGET);
}

#[test]
fn clearly_better_resource_opportunity_replaces_a_viable_plan() {
    let mut grid = IntentGrid::new(32, 32);
    let owner = SwarmId(4);
    let swarms = [SwarmState {
        id: owner,
        home: world(IVec2::ZERO),
        minerals: 0,
    }];
    let original = DepositState {
        id: 20,
        position: world(IVec2::new(2, 0)),
        amount: 100,
        radius: 100.0,
    };
    let mut controller = Controller::new(owner);
    let initial = controller.decide(&GameState {
        grid: &grid,
        swarms: &swarms,
        bots: &[],
        structures: &[],
        deposits: &[original],
        terrain: &[],
        tick: 0,
        finished: false,
    });
    apply(&mut grid, owner, &initial);

    let opportunities = [
        original,
        DepositState {
            id: 21,
            position: world(IVec2::new(4, 0)),
            amount: 1_000_000,
            radius: 100.0,
        },
    ];
    let changed = controller.decide(&GameState {
        grid: &grid,
        swarms: &swarms,
        bots: &[],
        structures: &[],
        deposits: &opportunities,
        terrain: &[],
        tick: REVIEW_PERIOD_TICKS,
        finished: false,
    });

    assert!(changed.reviewed);
    assert!(changed.explanation.contains("clearly better"));
    assert!(changed.edits.iter().any(|edit| {
        edit.action == IntentEditAction::Paint
            && edit.kind == IntentKind::Gather
            && edit.cell == IVec2::new(4, 0)
    }));
    assert!(changed.edits.iter().any(|edit| {
        edit.action == IntentEditAction::Erase
            && edit.kind == IntentKind::Gather
            && edit.cell == IVec2::new(2, 0)
    }));
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
        },
        SwarmState {
            id: enemy,
            home: world(IVec2::new(24, 24)),
            minerals: 0,
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
                charge: 1.0,
                cargo: 0,
            })
        })
        .collect::<Vec<_>>();
    let deposit = [DepositState {
        id: 20,
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
        minerals: 0,
    };
    let mut controller = Controller::new(owner);
    let initial = controller.decide(&GameState {
        grid: &grid,
        swarms: &initial_swarms,
        bots: &bots,
        structures: &[facility],
        deposits: &deposit,
        terrain: &[],
        tick: 0,
        finished: false,
    });
    assert!(initial.explanation.contains("secure Resource Deposit"));
    apply(&mut grid, owner, &initial);

    let supplied_swarms = [
        SwarmState {
            minerals: 2_000,
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
    let pressure = controller.decide(&GameState {
        grid: &grid,
        swarms: &supplied_swarms,
        bots: &bots,
        structures: &[facility, sink],
        deposits: &deposit,
        terrain: &[],
        tick: REVIEW_PERIOD_TICKS,
        finished: false,
    });

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
fn terrain_exposure_changes_which_equal_deposit_is_secured() {
    let grid = IntentGrid::new(32, 32);
    let owner = SwarmId(4);
    let swarms = [SwarmState {
        id: owner,
        home: world(IVec2::ZERO),
        minerals: 0,
    }];
    let exposed = DepositState {
        id: 20,
        position: world(IVec2::new(4, 0)),
        amount: 500,
        radius: 100.0,
    };
    let clear = DepositState {
        id: 21,
        position: world(IVec2::new(0, 4)),
        amount: 500,
        radius: 100.0,
    };
    let deposits = [clear, exposed];
    let terrain = [Obstacle::Circle {
        center: world(IVec2::new(2, 0)),
        radius: crate::ZONE_BLOCK_SIZE * 0.45,
    }];

    let decision = Controller::new(owner).decide(&GameState {
        grid: &grid,
        swarms: &swarms,
        bots: &[],
        structures: &[],
        deposits: &deposits,
        terrain: &terrain,
        tick: 0,
        finished: false,
    });

    assert!(decision.edits.iter().any(|edit| {
        edit.action == IntentEditAction::Paint
            && edit.kind == IntentKind::Gather
            && edit.cell == IVec2::new(0, 4)
    }));
    assert!(!decision.edits.iter().any(|edit| {
        edit.action == IntentEditAction::Paint
            && edit.kind == IntentKind::Gather
            && edit.cell == IVec2::new(4, 0)
    }));
}

#[test]
fn irrelevant_far_rocks_do_not_dilute_an_obstructed_deposit_approach() {
    let grid = IntentGrid::new(64, 64);
    let owner = SwarmId(4);
    let swarms = [SwarmState {
        id: owner,
        home: world(IVec2::ZERO),
        minerals: 0,
    }];
    let obstructed = DepositState {
        id: 20,
        position: world(IVec2::new(4, 0)),
        amount: 500,
        radius: 100.0,
    };
    let clear = DepositState {
        id: 21,
        position: world(IVec2::new(0, 4)) + Vec2::new(0.0, 10.0),
        amount: 500,
        radius: 100.0,
    };
    let deposits = [clear, obstructed];
    let approach_rock = Obstacle::Circle {
        center: world(IVec2::new(2, 0)),
        radius: crate::ZONE_BLOCK_SIZE * 0.45,
    };
    let mut terrain = vec![approach_rock];
    terrain.extend((0..200).map(|index| Obstacle::Circle {
        center: Vec2::new(50_000.0 + index as f32 * 100.0, 50_000.0),
        radius: 10.0,
    }));

    for terrain in [&[approach_rock][..], terrain.as_slice()] {
        let decision = Controller::new(owner).decide(&GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &[],
            structures: &[],
            deposits: &deposits,
            terrain,
            tick: 0,
            finished: false,
        });
        assert!(
            decision.edits.iter().any(|edit| {
                edit.action == IntentEditAction::Paint
                    && edit.kind == IntentKind::Gather
                    && edit.cell == IVec2::new(0, 4)
            }),
            "the clear approach lost preference after {} irrelevant rocks",
            terrain.len().saturating_sub(1)
        );
        assert!(!decision.edits.iter().any(|edit| {
            edit.action == IntentEditAction::Paint
                && edit.kind == IntentKind::Gather
                && edit.cell == IVec2::new(4, 0)
        }));
    }
}

#[test]
fn combat_support_build_follows_the_living_defender_cohort_without_outlier_drift() {
    let mut grid = IntentGrid::new(64, 64);
    let owner = SwarmId(4);
    let enemy = SwarmId(9);
    let home_cell = IVec2::ZERO;
    let cohort_cell = IVec2::new(4, 0);
    let midpoint = IVec2::new(6, 0);
    let target_cell = IVec2::new(12, 0);
    let home = world(home_cell);
    let swarms = [
        SwarmState {
            id: owner,
            home,
            minerals: 1_000,
        },
        SwarmState {
            id: enemy,
            home: world(target_cell),
            minerals: 0,
        },
    ];
    let mut bots = (0..5)
        .map(|id| BotState {
            id,
            owner,
            kind: NanobotType::Defender,
            position: home,
            health: 100,
            charge: 1.0,
            cargo: 0,
        })
        .collect::<Vec<_>>();
    bots.extend([
        BotState {
            id: 5,
            owner,
            kind: NanobotType::Defender,
            position: world(IVec2::new(11, 0)),
            health: 100,
            charge: 1.0,
            cargo: 0,
        },
        BotState {
            id: 6,
            owner,
            kind: NanobotType::Worker,
            position: world(IVec2::new(10, 0)),
            health: 100,
            charge: 1.0,
            cargo: 0,
        },
        BotState {
            id: 7,
            owner: enemy,
            kind: NanobotType::Defender,
            position: world(IVec2::new(-10, 0)),
            health: 100,
            charge: 1.0,
            cargo: 0,
        },
        BotState {
            id: 8,
            owner,
            kind: NanobotType::Defender,
            position: world(IVec2::new(10, 0)),
            health: 0,
            charge: 1.0,
            cargo: 0,
        },
    ]);
    let deposit = [DepositState {
        id: 20,
        position: world(IVec2::new(-2, 0)),
        amount: 500,
        radius: 100.0,
    }];
    let structures = [
        StructureState {
            id: 29,
            owner,
            position: world(IVec2::new(-1, 0)),
            kind: StructureKind::Charger,
            health: 100,
            minerals: 0,
        },
        StructureState {
            id: 30,
            owner: enemy,
            position: world(target_cell),
            kind: StructureKind::Sink,
            health: 20,
            minerals: 100,
        },
    ];
    let mut controller = Controller::new(owner);

    let home_decision = controller.decide(&GameState {
        grid: &grid,
        swarms: &swarms,
        bots: &bots,
        structures: &structures,
        deposits: &deposit,
        terrain: &[],
        tick: 0,
        finished: false,
    });
    assert!(
        home_decision
            .explanation
            .contains("attack enemy logistics structure 30")
    );
    assert!(home_decision.edits.iter().any(|edit| {
        edit.action == IntentEditAction::Paint
            && edit.kind == IntentKind::Gather
            && edit.cell == IVec2::new(-2, 0)
    }));
    assert!(home_decision.edits.iter().any(|edit| {
        edit.action == IntentEditAction::Paint
            && edit.kind == IntentKind::Defend
            && edit.cell == target_cell
    }));
    assert!(
        home_decision
            .edits
            .iter()
            .filter(|edit| {
                edit.action == IntentEditAction::Paint
                    && edit.kind == IntentKind::Build
                    && edit.cell != home_cell
            })
            .all(|edit| (edit.cell - home_cell).abs().max_element() <= 1),
        "support must stay with the home cohort rather than open at midpoint {midpoint}: {:?}",
        home_decision.edits
    );
    assert!(!home_decision.edits.iter().any(|edit| {
        edit.action == IntentEditAction::Paint
            && edit.kind == IntentKind::Build
            && edit.cell == midpoint
    }));
    apply(&mut grid, owner, &home_decision);

    for bot in bots.iter_mut().filter(|bot| bot.id < 5) {
        bot.position = world(cohort_cell);
    }
    let obstruction = Obstacle::Circle {
        center: world(cohort_cell),
        radius: crate::ZONE_BLOCK_SIZE * 0.6,
    };
    let advanced = controller.decide(&GameState {
        grid: &grid,
        swarms: &swarms,
        bots: &bots,
        structures: &structures,
        deposits: &deposit,
        terrain: &[obstruction],
        tick: REVIEW_PERIOD_TICKS,
        finished: false,
    });
    apply(&mut grid, owner, &advanced);

    let forward_builds = grid
        .iter_active_cells()
        .filter_map(|(cell, intent)| {
            (cell != home_cell && intent.has_owned(IntentKind::Build, owner)).then_some(cell)
        })
        .collect::<Vec<_>>();
    assert!(
        forward_builds.iter().any(|cell| {
            (*cell - cohort_cell).abs().max_element() <= 1 && obstruction.admits_body(world(*cell))
        }),
        "an open support Build must follow the advanced cohort: {forward_builds:?}; {}",
        advanced.explanation
    );
    assert!(
        forward_builds
            .iter()
            .all(|cell| (*cell - cohort_cell).abs().max_element() <= 1),
        "outliers or remote obstacle alternatives dragged support away: {forward_builds:?}"
    );
    assert!(!forward_builds.contains(&midpoint));
    assert!(
        grid.cell(target_cell)
            .is_some_and(|intent| { intent.has_owned(IntentKind::Defend, owner) })
    );
    assert!(
        grid.cell(IVec2::new(-2, 0))
            .is_some_and(|intent| { intent.has_owned(IntentKind::Gather, owner) })
    );
}

#[test]
fn combat_support_retains_a_nearby_open_build_during_small_cohort_drift() {
    let mut grid = IntentGrid::new(32, 32);
    let owner = SwarmId(4);
    let enemy = SwarmId(9);
    let home_cell = IVec2::ZERO;
    let home = world(home_cell);
    let swarms = [
        SwarmState {
            id: owner,
            home,
            minerals: 1_000,
        },
        SwarmState {
            id: enemy,
            home: world(IVec2::new(8, 0)),
            minerals: 0,
        },
    ];
    let mut bots = (0..6)
        .map(|id| BotState {
            id,
            owner,
            kind: NanobotType::Defender,
            position: home,
            health: 100,
            charge: 1.0,
            cargo: 0,
        })
        .collect::<Vec<_>>();
    let deposits = [DepositState {
        id: 20,
        position: world(IVec2::new(-1, 0)),
        amount: 500,
        radius: 100.0,
    }];
    let structures = [StructureState {
        id: 30,
        owner: enemy,
        position: world(IVec2::new(8, 0)),
        kind: StructureKind::Sink,
        health: 20,
        minerals: 100,
    }];
    let mut controller = Controller::new(owner);
    let initial = controller.decide(&GameState {
        grid: &grid,
        swarms: &swarms,
        bots: &bots,
        structures: &structures,
        deposits: &deposits,
        terrain: &[],
        tick: 0,
        finished: false,
    });
    assert!(
        initial
            .explanation
            .contains("attack enemy logistics structure 30")
    );
    apply(&mut grid, owner, &initial);
    let incumbent = grid
        .iter_active_cells()
        .find_map(|(cell, intent)| {
            (cell != home_cell && intent.has_owned(IntentKind::Build, owner)).then_some(cell)
        })
        .expect("the attack plan needs one non-home support Build");

    for bot in &mut bots {
        bot.position = world(IVec2::new(1, 0));
    }
    let drift = controller.decide(&GameState {
        grid: &grid,
        swarms: &swarms,
        bots: &bots,
        structures: &structures,
        deposits: &deposits,
        terrain: &[],
        tick: REVIEW_PERIOD_TICKS,
        finished: false,
    });
    assert!(
        !drift.edits.iter().any(|edit| {
            edit.kind == IntentKind::Build
                && (edit.cell == incumbent || edit.action == IntentEditAction::Paint)
        }),
        "an open support Build one cell from the cohort must not churn: {:?}",
        drift.edits
    );
    apply(&mut grid, owner, &drift);
    assert!(
        grid.cell(incumbent)
            .is_some_and(|intent| intent.has_owned(IntentKind::Build, owner))
    );

    let obstruction = Obstacle::Circle {
        center: world(incumbent),
        radius: crate::ZONE_BLOCK_SIZE * 0.6,
    };
    let obstructed = controller.decide(&GameState {
        grid: &grid,
        swarms: &swarms,
        bots: &bots,
        structures: &structures,
        deposits: &deposits,
        terrain: &[obstruction],
        tick: REVIEW_PERIOD_TICKS * 2,
        finished: false,
    });
    apply(&mut grid, owner, &obstructed);
    assert!(
        grid.cell(incumbent)
            .is_none_or(|intent| !intent.has_owned(IntentKind::Build, owner)),
        "an obstructed incumbent must be released"
    );
    assert!(grid.iter_active_cells().any(|(cell, intent)| {
        cell != home_cell
            && (cell - IVec2::new(1, 0)).abs().max_element() <= 1
            && obstruction.admits_body(world(cell))
            && intent.has_owned(IntentKind::Build, owner)
    }));
}

#[test]
fn even_split_cohort_support_uses_an_occupied_flank_instead_of_the_empty_median_gap() {
    let grid = IntentGrid::new(32, 32);
    let owner = SwarmId(4);
    let enemy = SwarmId(9);
    let home = world(IVec2::ZERO);
    let swarms = [
        SwarmState {
            id: owner,
            home,
            minerals: 1_000,
        },
        SwarmState {
            id: enemy,
            home: world(IVec2::new(12, 0)),
            minerals: 0,
        },
    ];
    let bots = (0..6)
        .map(|id| BotState {
            id,
            owner,
            kind: NanobotType::Defender,
            position: world(if id < 3 {
                IVec2::ZERO
            } else {
                IVec2::new(6, 0)
            }),
            health: 100,
            charge: 1.0,
            cargo: 0,
        })
        .collect::<Vec<_>>();
    let deposits = [DepositState {
        id: 20,
        position: world(IVec2::new(-1, 0)),
        amount: 500,
        radius: 100.0,
    }];
    let structures = [StructureState {
        id: 30,
        owner: enemy,
        position: world(IVec2::new(12, 0)),
        kind: StructureKind::Sink,
        health: 20,
        minerals: 100,
    }];

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
    assert!(
        decision
            .explanation
            .contains("attack enemy logistics structure 30")
    );
    let forward_builds = decision
        .edits
        .iter()
        .filter(|edit| {
            edit.action == IntentEditAction::Paint
                && edit.kind == IntentKind::Build
                && edit.cell != IVec2::ZERO
        })
        .map(|edit| edit.cell)
        .collect::<Vec<_>>();
    assert!(
        forward_builds
            .iter()
            .all(|cell| cell.abs().max_element() <= 1),
        "equal cohort flanks must choose the home-nearest occupied flank, not the empty median gap: {forward_builds:?}"
    );
    assert!(
        forward_builds
            .iter()
            .all(|cell| { (*cell - IVec2::new(3, 0)).abs().max_element() > 1 })
    );
}

#[test]
fn pressure_support_uses_the_open_side_of_a_swapped_obstruction() {
    let owner = SwarmId(4);
    let enemy = SwarmId(9);
    let home = world(IVec2::ZERO);
    let cohort_cell = IVec2::new(4, 0);
    let enemy_home = world(IVec2::new(12, 0));
    let swarms = [
        SwarmState {
            id: owner,
            home,
            minerals: 1_000,
        },
        SwarmState {
            id: enemy,
            home: enemy_home,
            minerals: 0,
        },
    ];
    let bots = [
        Vec2::new(-250.0, -250.0),
        Vec2::new(250.0, -250.0),
        Vec2::new(-250.0, 250.0),
        Vec2::new(250.0, 250.0),
    ]
    .into_iter()
    .enumerate()
    .map(|(id, offset)| BotState {
        id: id as u64,
        owner,
        kind: NanobotType::Defender,
        position: world(cohort_cell) + offset,
        health: 100,
        charge: 1.0,
        cargo: 0,
    })
    .collect::<Vec<_>>();
    let deposit = [DepositState {
        id: 20,
        position: world(IVec2::new(-1, 0)),
        amount: 500,
        radius: 100.0,
    }];
    let enemy_sink = [StructureState {
        id: 30,
        owner: enemy,
        position: enemy_home,
        kind: StructureKind::Sink,
        health: 20,
        minerals: 100,
    }];

    for (blocked_side, expected_open_side) in [(1, -1), (-1, 1)] {
        let grid = IntentGrid::new(64, 64);
        let terrain = [
            Obstacle::Circle {
                center: world(cohort_cell),
                radius: crate::ZONE_BLOCK_SIZE * 0.6,
            },
            Obstacle::Circle {
                center: world(cohort_cell + IVec2::new(0, blocked_side)),
                radius: crate::ZONE_BLOCK_SIZE * 0.6,
            },
        ];
        assert!(bots.iter().all(|bot| {
            terrain
                .iter()
                .all(|obstacle| obstacle.admits_body(bot.position))
        }));
        let decision = Controller::new(owner).decide(&GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &bots,
            structures: &enemy_sink,
            deposits: &deposit,
            terrain: &terrain,
            tick: 0,
            finished: false,
        });

        assert!(
            decision.edits.iter().any(|edit| {
                edit.action == IntentEditAction::Paint
                    && edit.kind == IntentKind::Build
                    && edit.cell.x == cohort_cell.x
                    && (edit.cell - cohort_cell).abs().max_element() == 1
                    && edit.cell.y.signum() == expected_open_side
            }),
            "expected open side {expected_open_side}: {}; edits: {:?}",
            decision.explanation,
            decision.edits
        );
        assert!(decision.edits.iter().any(|edit| {
            edit.action == IntentEditAction::Paint
                && edit.kind == IntentKind::Defend
                && edit.cell == IVec2::new(12, 0)
        }));
        assert!(decision.edits.iter().any(|edit| {
            edit.action == IntentEditAction::Paint
                && edit.kind == IntentKind::Gather
                && edit.cell == IVec2::new(-1, 0)
        }));
    }
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
        },
        SwarmState {
            id: enemy,
            home: crate::scenario::cell_origin(crate::scenario::OPPONENT_CELL),
            minerals: 0,
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
                charge: 1.0,
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
            position: crate::scenario::cell_origin(cell),
            amount: crate::scenario::STARTING_DEPOSIT_AMOUNT,
            radius: crate::scenario::STARTING_WORK_RADIUS,
        })
        .collect::<Vec<_>>();
    let terrain = crate::scenario::default_rock_geometry()
        .into_iter()
        .map(|(rock, transform)| rock.obstacle(&transform))
        .collect::<Vec<_>>();

    let decision = Controller::new(owner).decide(&GameState {
        grid: &grid,
        swarms: &swarms,
        bots: &bots,
        structures: &[],
        deposits: &deposits,
        terrain: &terrain,
        tick: 0,
        finished: false,
    });

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
    let grid = IntentGrid::new(64, 64);
    let owner = SwarmId::PLAYER;
    let enemy = SwarmId(1);
    let swarms = [
        SwarmState {
            id: owner,
            home: crate::scenario::cell_origin(crate::scenario::PLAYER_CELL),
            minerals: 2_000,
        },
        SwarmState {
            id: enemy,
            home: crate::scenario::cell_origin(crate::scenario::OPPONENT_CELL),
            minerals: 0,
        },
    ];
    let bots = (0..10)
        .map(|id| BotState {
            id,
            owner,
            kind: NanobotType::Defender,
            position: swarms[0].home,
            health: 100,
            charge: 1.0,
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
    let deposits = deposit_cells
        .into_iter()
        .enumerate()
        .map(|(id, cell)| DepositState {
            id: 20 + id as u64,
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
        minerals: 100,
    }];
    let terrain = crate::scenario::default_rock_geometry()
        .into_iter()
        .map(|(rock, transform)| rock.obstacle(&transform))
        .collect::<Vec<_>>();

    let decision = Controller::new(owner).decide(&GameState {
        grid: &grid,
        swarms: &swarms,
        bots: &bots,
        structures: &structures,
        deposits: &deposits,
        terrain: &terrain,
        tick: 0,
        finished: false,
    });

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
        },
        SwarmState {
            id: enemy,
            home: world(IVec2::new(12, 0)),
            minerals: 0,
        },
    ];
    let friendly_defenders = (0..2).map(|id| BotState {
        id,
        owner,
        kind: NanobotType::Defender,
        position: swarms[0].home,
        health: 100,
        charge: 1.0,
        cargo: 0,
    });
    let target = BotState {
        id: 50,
        owner: enemy,
        kind: NanobotType::Worker,
        position: world(target_cell),
        health: 100,
        charge: 1.0,
        cargo: 0,
    };
    let bots = friendly_defenders.chain([target]).collect::<Vec<_>>();
    let mut controller = Controller::new(owner);
    let initial = controller.decide(&GameState {
        grid: &grid,
        swarms: &swarms,
        bots: &bots,
        structures: &[],
        deposits: &[],
        terrain: &[],
        tick: 0,
        finished: false,
    });
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
    let immediate = controller.decide(&GameState {
        grid: &grid,
        swarms: &swarms,
        bots: &bots,
        structures: &[],
        deposits: &[],
        terrain: &[],
        tick: 1,
        finished: false,
    });
    assert!(!immediate.reviewed);
    assert!(immediate.edits.is_empty());

    let regular = controller.decide(&GameState {
        grid: &grid,
        swarms: &swarms,
        bots: &bots,
        structures: &[],
        deposits: &[],
        terrain: &[],
        tick: REVIEW_PERIOD_TICKS,
        finished: false,
    });
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
        },
        SwarmState {
            id: enemy,
            home: world(IVec2::new(12, 0)),
            minerals: 0,
        },
    ];
    let friendly_defenders = (0..2).map(|id| BotState {
        id,
        owner,
        kind: NanobotType::Defender,
        position: swarms[0].home,
        health: 100,
        charge: 1.0,
        cargo: 0,
    });
    let target = BotState {
        id: 50,
        owner: enemy,
        kind: NanobotType::Worker,
        position: world(target_cell),
        health: 100,
        charge: 1.0,
        cargo: 0,
    };
    let mut bots = friendly_defenders.chain([target]).collect::<Vec<_>>();
    let alternative = StructureState {
        id: 30,
        owner: enemy,
        position: world(alternative_cell),
        kind: StructureKind::Charger,
        health: 1,
        minerals: 1_000,
    };
    let mut controller = Controller::new(owner);
    let initial = controller.decide(&GameState {
        grid: &grid,
        swarms: &swarms,
        bots: &bots,
        structures: &[],
        deposits: &[],
        terrain: &[],
        tick: 0,
        finished: false,
    });
    apply(&mut grid, owner, &initial);
    assert!(
        grid.cell(covered_cell)
            .is_some_and(|intent| intent.has_owned(IntentKind::Defend, owner))
    );

    *bots.last_mut().unwrap() = BotState {
        position: world(covered_cell),
        ..target
    };
    let immediate = controller.decide(&GameState {
        grid: &grid,
        swarms: &swarms,
        bots: &bots,
        structures: &[],
        deposits: &[],
        terrain: &[],
        tick: 1,
        finished: false,
    });
    assert!(!immediate.reviewed);
    assert!(immediate.edits.is_empty());
    let grace = controller.decide(&GameState {
        grid: &grid,
        swarms: &swarms,
        bots: &bots,
        structures: &[alternative],
        deposits: &[],
        terrain: &[],
        tick: REVIEW_PERIOD_TICKS,
        finished: false,
    });
    assert!(grace.reviewed);
    assert!(grace.edits.is_empty());
    let reassessed = controller.decide(&GameState {
        grid: &grid,
        swarms: &swarms,
        bots: &bots,
        structures: &[alternative],
        deposits: &[],
        terrain: &[],
        tick: REVIEW_PERIOD_TICKS * 2,
        finished: false,
    });
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
        },
        SwarmState {
            id: enemy,
            home: world(IVec2::new(12, 0)),
            minerals: 0,
        },
    ];
    let friendly_defenders = (0..2).map(|id| BotState {
        id,
        owner,
        kind: NanobotType::Defender,
        position: swarms[0].home,
        health: 100,
        charge: 1.0,
        cargo: 0,
    });
    let target = BotState {
        id: 50,
        owner: enemy,
        kind: NanobotType::Worker,
        position: world(target_cell),
        health: 100,
        charge: 1.0,
        cargo: 0,
    };
    let mut bots = friendly_defenders.chain([target]).collect::<Vec<_>>();
    let mut controller = Controller::new(owner);
    let initial = controller.decide(&GameState {
        grid: &grid,
        swarms: &swarms,
        bots: &bots,
        structures: &[],
        deposits: &[],
        terrain: &[],
        tick: 0,
        finished: false,
    });
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
    let covered = controller.decide(&GameState {
        grid: &grid,
        swarms: &swarms,
        bots: &bots,
        structures: &[],
        deposits: &[],
        terrain: &[],
        tick: 1,
        finished: false,
    });
    assert!(!covered.reviewed);
    assert!(covered.edits.is_empty());

    *bots.last_mut().unwrap() = BotState {
        position: world(uncovered_cell),
        health: 60,
        ..target
    };
    let exited = controller.decide(&GameState {
        grid: &grid,
        swarms: &swarms,
        bots: &bots,
        structures: &[],
        deposits: &[],
        terrain: &[],
        tick: 2,
        finished: false,
    });
    assert!(exited.reviewed);
    assert!(exited.edits.iter().any(|edit| {
        edit.action == IntentEditAction::Paint
            && edit.kind == IntentKind::Defend
            && edit.cell == uncovered_cell
    }));
}

#[test]
fn stalled_covered_target_yields_to_a_clearly_better_plan_after_one_grace_review() {
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
        },
        SwarmState {
            id: enemy,
            home: world(IVec2::new(12, 0)),
            minerals: 0,
        },
    ];
    let friendly_defenders = (0..2).map(|id| BotState {
        id,
        owner,
        kind: NanobotType::Defender,
        position: swarms[0].home,
        health: 100,
        charge: 1.0,
        cargo: 0,
    });
    let target = BotState {
        id: 50,
        owner: enemy,
        kind: NanobotType::Worker,
        position: world(target_cell),
        health: 100,
        charge: 1.0,
        cargo: 0,
    };
    let mut bots = friendly_defenders.chain([target]).collect::<Vec<_>>();
    let alternative = StructureState {
        id: 30,
        owner: enemy,
        position: world(alternative_cell),
        kind: StructureKind::Charger,
        health: 1,
        minerals: 1_000,
    };
    let mut controller = Controller::new(owner);
    let initial = controller.decide(&GameState {
        grid: &grid,
        swarms: &swarms,
        bots: &bots,
        structures: &[],
        deposits: &[],
        terrain: &[],
        tick: 0,
        finished: false,
    });
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
    let immediate = controller.decide(&GameState {
        grid: &grid,
        swarms: &swarms,
        bots: &bots,
        structures: &[],
        deposits: &[],
        terrain: &[],
        tick: 1,
        finished: false,
    });
    assert!(!immediate.reviewed);
    assert!(immediate.edits.is_empty());
    let progress_review = controller.decide(&GameState {
        grid: &grid,
        swarms: &swarms,
        bots: &bots,
        structures: &[],
        deposits: &[],
        terrain: &[],
        tick: REVIEW_PERIOD_TICKS,
        finished: false,
    });
    assert!(progress_review.reviewed);
    assert!(progress_review.edits.is_empty());

    let grace = controller.decide(&GameState {
        grid: &grid,
        swarms: &swarms,
        bots: &bots,
        structures: &[alternative],
        deposits: &[],
        terrain: &[],
        tick: REVIEW_PERIOD_TICKS * 2,
        finished: false,
    });
    assert!(grace.reviewed);
    assert!(grace.edits.is_empty());
    let reassessed = controller.decide(&GameState {
        grid: &grid,
        swarms: &swarms,
        bots: &bots,
        structures: &[alternative],
        deposits: &[],
        terrain: &[],
        tick: REVIEW_PERIOD_TICKS * 3,
        finished: false,
    });
    assert!(reassessed.reviewed);
    assert!(reassessed.edits.iter().any(|edit| {
        edit.action == IntentEditAction::Paint
            && edit.kind == IntentKind::Defend
            && edit.cell == alternative_cell
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
        },
        SwarmState {
            id: enemy,
            home: world(IVec2::new(12, 0)),
            minerals: 0,
        },
    ];
    let friendly_defenders = (0..2).map(|id| BotState {
        id,
        owner,
        kind: NanobotType::Defender,
        position: swarms[0].home,
        health: 100,
        charge: 1.0,
        cargo: 0,
    });
    let target = BotState {
        id: 50,
        owner: enemy,
        kind: NanobotType::Worker,
        position: world(target_cell),
        health: 100,
        charge: 1.0,
        cargo: 0,
    };
    let mut bots = friendly_defenders.chain([target]).collect::<Vec<_>>();
    let alternative = StructureState {
        id: 30,
        owner: enemy,
        position: world(alternative_cell),
        kind: StructureKind::Charger,
        health: 1,
        minerals: 1_000,
    };
    let mut controller = Controller::new(owner);
    let initial = controller.decide(&GameState {
        grid: &grid,
        swarms: &swarms,
        bots: &bots,
        structures: &[],
        deposits: &[],
        terrain: &[],
        tick: 0,
        finished: false,
    });
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
    let immediate = controller.decide(&GameState {
        grid: &grid,
        swarms: &swarms,
        bots: &bots,
        structures: &[],
        deposits: &[],
        terrain: &[],
        tick: 1,
        finished: false,
    });
    assert!(!immediate.reviewed);
    assert!(immediate.edits.is_empty());
    let progress_review = controller.decide(&GameState {
        grid: &grid,
        swarms: &swarms,
        bots: &bots,
        structures: &[],
        deposits: &[],
        terrain: &[],
        tick: REVIEW_PERIOD_TICKS,
        finished: false,
    });
    assert!(progress_review.edits.is_empty());
    let first_grace = controller.decide(&GameState {
        grid: &grid,
        swarms: &swarms,
        bots: &bots,
        structures: &[alternative],
        deposits: &[],
        terrain: &[],
        tick: REVIEW_PERIOD_TICKS * 2,
        finished: false,
    });
    assert!(first_grace.edits.is_empty());

    bots.last_mut().unwrap().health = 60;
    let renewed_progress = controller.decide(&GameState {
        grid: &grid,
        swarms: &swarms,
        bots: &bots,
        structures: &[alternative],
        deposits: &[],
        terrain: &[],
        tick: REVIEW_PERIOD_TICKS * 3,
        finished: false,
    });
    assert!(renewed_progress.reviewed);
    assert!(renewed_progress.edits.is_empty());
    let reset_grace = controller.decide(&GameState {
        grid: &grid,
        swarms: &swarms,
        bots: &bots,
        structures: &[alternative],
        deposits: &[],
        terrain: &[],
        tick: REVIEW_PERIOD_TICKS * 4,
        finished: false,
    });
    assert!(reset_grace.reviewed);
    assert!(reset_grace.edits.is_empty());
    let reassessed = controller.decide(&GameState {
        grid: &grid,
        swarms: &swarms,
        bots: &bots,
        structures: &[alternative],
        deposits: &[],
        terrain: &[],
        tick: REVIEW_PERIOD_TICKS * 5,
        finished: false,
    });
    assert!(reassessed.edits.iter().any(|edit| {
        edit.action == IntentEditAction::Paint
            && edit.kind == IntentKind::Defend
            && edit.cell == alternative_cell
    }));
}

#[test]
fn covered_progress_restores_owned_defend_lost_by_another_target() {
    let mut grid = IntentGrid::new(32, 32);
    let owner = SwarmId(4);
    let enemy = SwarmId(9);
    let swarms = [
        SwarmState {
            id: owner,
            home: world(IVec2::ZERO),
            minerals: 2_000,
        },
        SwarmState {
            id: enemy,
            home: world(IVec2::new(12, 0)),
            minerals: 0,
        },
    ];
    let bots = (0..10)
        .map(|id| BotState {
            id,
            owner,
            kind: NanobotType::Defender,
            position: swarms[0].home,
            health: 100,
            charge: 1.0,
            cargo: 0,
        })
        .collect::<Vec<_>>();
    let friendly_charger = StructureState {
        id: 20,
        owner,
        position: world(IVec2::new(1, 0)),
        kind: StructureKind::Charger,
        health: 100,
        minerals: 100,
    };
    let moving_target = StructureState {
        id: 30,
        owner: enemy,
        position: world(IVec2::new(6, 4)),
        kind: StructureKind::Charger,
        health: 100,
        minerals: 1_000,
    };
    let other_target = StructureState {
        id: 31,
        owner: enemy,
        position: world(IVec2::new(12, 0)),
        kind: StructureKind::Sink,
        health: 100,
        minerals: 0,
    };
    let other_cell = world_to_intent_cell(other_target.position);
    let mut controller = Controller::new(owner);
    let initial = controller.decide(&GameState {
        grid: &grid,
        swarms: &swarms,
        bots: &bots,
        structures: &[friendly_charger, moving_target, other_target],
        deposits: &[],
        terrain: &[],
        tick: 0,
        finished: false,
    });
    apply(&mut grid, owner, &initial);
    assert!(
        grid.cell(other_cell)
            .is_some_and(|intent| intent.has_owned(IntentKind::Defend, owner)),
        "the fixture must establish the second attack obligation"
    );
    let moving_cell = world_to_intent_cell(moving_target.position);
    let covered_cell = grid
        .iter_active_cells()
        .map(|(cell, _)| cell)
        .find(|cell| {
            *cell != moving_cell
                && (*cell - moving_cell).abs().max_element() <= 2
                && grid
                    .cell(*cell)
                    .is_some_and(|intent| intent.has_owned(IntentKind::Defend, owner))
        })
        .expect("the moving target needs another actually covered cell");
    let moving_target = StructureState {
        position: world(covered_cell),
        health: 80,
        ..moving_target
    };
    let immediate = controller.decide(&GameState {
        grid: &grid,
        swarms: &swarms,
        bots: &bots,
        structures: &[friendly_charger, moving_target, other_target],
        deposits: &[],
        terrain: &[],
        tick: 1,
        finished: false,
    });
    assert!(!immediate.reviewed);
    assert!(immediate.edits.is_empty());

    grid.erase(other_cell, IntentKind::Defend, owner);
    let replanned = controller.decide(&GameState {
        grid: &grid,
        swarms: &swarms,
        bots: &bots,
        structures: &[friendly_charger, moving_target, other_target],
        deposits: &[],
        terrain: &[],
        tick: 2,
        finished: false,
    });
    assert!(replanned.reviewed);
    assert!(replanned.edits.iter().any(|edit| {
        edit.action == IntentEditAction::Paint
            && edit.kind == IntentKind::Defend
            && edit.cell == other_cell
    }));
}

#[test]
fn covered_progress_reduces_an_attack_front_when_ready_force_falls() {
    let mut grid = IntentGrid::new(32, 32);
    let owner = SwarmId(4);
    let enemy = SwarmId(9);
    let swarms = [
        SwarmState {
            id: owner,
            home: world(IVec2::ZERO),
            minerals: 2_000,
        },
        SwarmState {
            id: enemy,
            home: world(IVec2::new(12, 0)),
            minerals: 0,
        },
    ];
    let bots = (0..10)
        .map(|id| BotState {
            id,
            owner,
            kind: NanobotType::Defender,
            position: swarms[0].home,
            health: 100,
            charge: 1.0,
            cargo: 0,
        })
        .collect::<Vec<_>>();
    let friendly_charger = StructureState {
        id: 20,
        owner,
        position: world(IVec2::new(1, 0)),
        kind: StructureKind::Charger,
        health: 100,
        minerals: 100,
    };
    let moving_target = StructureState {
        id: 30,
        owner: enemy,
        position: world(IVec2::new(6, 4)),
        kind: StructureKind::Charger,
        health: 100,
        minerals: 1_000,
    };
    let other_target = StructureState {
        id: 31,
        owner: enemy,
        position: world(IVec2::new(12, 0)),
        kind: StructureKind::Sink,
        health: 100,
        minerals: 0,
    };
    let other_cell = world_to_intent_cell(other_target.position);
    let mut controller = Controller::new(owner);
    let initial = controller.decide(&GameState {
        grid: &grid,
        swarms: &swarms,
        bots: &bots,
        structures: &[friendly_charger, moving_target, other_target],
        deposits: &[],
        terrain: &[],
        tick: 0,
        finished: false,
    });
    apply(&mut grid, owner, &initial);
    let moving_cell = world_to_intent_cell(moving_target.position);
    let covered_cell = grid
        .iter_active_cells()
        .map(|(cell, _)| cell)
        .find(|cell| {
            *cell != moving_cell
                && (*cell - moving_cell).abs().max_element() <= 2
                && grid
                    .cell(*cell)
                    .is_some_and(|intent| intent.has_owned(IntentKind::Defend, owner))
        })
        .expect("the moving target needs another actually covered cell");
    let moving_target = StructureState {
        position: world(covered_cell),
        health: 80,
        ..moving_target
    };
    let immediate = controller.decide(&GameState {
        grid: &grid,
        swarms: &swarms,
        bots: &bots,
        structures: &[friendly_charger, moving_target, other_target],
        deposits: &[],
        terrain: &[],
        tick: 1,
        finished: false,
    });
    assert!(!immediate.reviewed);
    assert!(immediate.edits.is_empty());

    let depleted_defenders = bots
        .iter()
        .map(|bot| BotState {
            charge: 0.1,
            ..*bot
        })
        .collect::<Vec<_>>();
    let empty_charger = StructureState {
        minerals: 0,
        ..friendly_charger
    };
    let regular = controller.decide(&GameState {
        grid: &grid,
        swarms: &swarms,
        bots: &depleted_defenders,
        structures: &[empty_charger, moving_target, other_target],
        deposits: &[],
        terrain: &[],
        tick: REVIEW_PERIOD_TICKS,
        finished: false,
    });
    assert!(regular.reviewed);
    assert!(!regular.edits.is_empty());
    apply(&mut grid, owner, &regular);
    assert!(
        grid.cell(covered_cell)
            .is_some_and(|intent| intent.has_owned(IntentKind::Defend, owner))
    );
    assert!(
        !grid
            .cell(other_cell)
            .is_some_and(|intent| intent.has_owned(IntentKind::Defend, owner))
    );
}

#[test]
fn covered_progress_opens_a_new_front_when_ready_force_grows() {
    let mut grid = IntentGrid::new(32, 32);
    let owner = SwarmId(4);
    let enemy = SwarmId(9);
    let target_cell = IVec2::new(6, 0);
    let covered_cell = IVec2::new(5, -1);
    let new_target_cell = IVec2::new(10, 4);
    let swarms = [
        SwarmState {
            id: owner,
            home: world(IVec2::ZERO),
            minerals: 2_000,
        },
        SwarmState {
            id: enemy,
            home: world(IVec2::new(12, 0)),
            minerals: 0,
        },
    ];
    let target = BotState {
        id: 50,
        owner: enemy,
        kind: NanobotType::Worker,
        position: world(target_cell),
        health: 100,
        charge: 1.0,
        cargo: 0,
    };
    let mut bots = (0..2)
        .map(|id| BotState {
            id,
            owner,
            kind: NanobotType::Defender,
            position: swarms[0].home,
            health: 100,
            charge: 1.0,
            cargo: 0,
        })
        .chain([target])
        .collect::<Vec<_>>();
    let mut controller = Controller::new(owner);
    let initial = controller.decide(&GameState {
        grid: &grid,
        swarms: &swarms,
        bots: &bots,
        structures: &[],
        deposits: &[],
        terrain: &[],
        tick: 0,
        finished: false,
    });
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
    let immediate = controller.decide(&GameState {
        grid: &grid,
        swarms: &swarms,
        bots: &bots,
        structures: &[],
        deposits: &[],
        terrain: &[],
        tick: 1,
        finished: false,
    });
    assert!(!immediate.reviewed);
    assert!(immediate.edits.is_empty());

    bots.truncate(2);
    bots.extend((2..8).map(|id| BotState {
        id,
        owner,
        kind: NanobotType::Defender,
        position: swarms[0].home,
        health: 100,
        charge: 1.0,
        cargo: 0,
    }));
    bots.push(BotState {
        position: world(covered_cell),
        health: 60,
        ..target
    });
    let supplied_charger = StructureState {
        id: 20,
        owner,
        position: world(IVec2::new(1, 0)),
        kind: StructureKind::Charger,
        health: 100,
        minerals: 100,
    };
    let no_fresh_target = controller.decide(&GameState {
        grid: &grid,
        swarms: &swarms,
        bots: &bots,
        structures: &[supplied_charger],
        deposits: &[],
        terrain: &[],
        tick: REVIEW_PERIOD_TICKS,
        finished: false,
    });
    assert!(no_fresh_target.reviewed);
    assert!(no_fresh_target.edits.is_empty());

    bots.last_mut().unwrap().health = 40;
    let new_target = StructureState {
        id: 30,
        owner: enemy,
        position: world(new_target_cell),
        kind: StructureKind::Charger,
        health: 20,
        minerals: 1_000,
    };
    let expanded = controller.decide(&GameState {
        grid: &grid,
        swarms: &swarms,
        bots: &bots,
        structures: &[supplied_charger, new_target],
        deposits: &[],
        terrain: &[],
        tick: REVIEW_PERIOD_TICKS * 2,
        finished: false,
    });
    apply(&mut grid, owner, &expanded);
    assert!(
        grid.cell(covered_cell)
            .is_some_and(|intent| intent.has_owned(IntentKind::Defend, owner))
    );
    assert!(
        grid.cell(new_target_cell)
            .is_some_and(|intent| intent.has_owned(IntentKind::Defend, owner))
    );
}

#[test]
fn structure_free_remnant_is_followed_across_cells_and_after_disappearance() {
    let mut grid = IntentGrid::new(64, 64);
    let owner = SwarmId(4);
    let enemy = SwarmId(9);
    let swarms = [
        SwarmState {
            id: owner,
            home: world(IVec2::ZERO),
            minerals: 100,
        },
        SwarmState {
            id: enemy,
            home: world(IVec2::new(20, 20)),
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
    let mut controller = Controller::new(owner);
    let first = controller.decide(&GameState {
        grid: &grid,
        swarms: &swarms,
        bots: &[remnant],
        structures: &[],
        deposits: &deposit,
        terrain: &[],
        tick: 0,
        finished: false,
    });
    assert!(first.explanation.contains("hunt remaining Nanobot 50"));
    assert!(first.edits.iter().any(|edit| {
        edit.action == IntentEditAction::Paint
            && edit.kind == IntentKind::Defend
            && edit.cell == IVec2::new(12, 3)
    }));
    apply(&mut grid, owner, &first);

    let moved = BotState {
        position: world(IVec2::new(13, 3)),
        ..remnant
    };
    let relocated = controller.decide(&GameState {
        grid: &grid,
        swarms: &swarms,
        bots: &[moved],
        structures: &[],
        deposits: &deposit,
        terrain: &[],
        tick: 1,
        finished: false,
    });
    assert!(relocated.reviewed);
    assert!(relocated.explanation.contains("moved"));
    assert!(relocated.edits.iter().any(|edit| {
        edit.action == IntentEditAction::Paint
            && edit.kind == IntentKind::Defend
            && edit.cell == IVec2::new(13, 3)
    }));
    apply(&mut grid, owner, &relocated);

    let disappeared = controller.decide(&GameState {
        grid: &grid,
        swarms: &swarms,
        bots: &[],
        structures: &[],
        deposits: &deposit,
        terrain: &[],
        tick: 2,
        finished: false,
    });
    assert!(disappeared.reviewed);
    assert!(disappeared.explanation.contains("disappeared"));
    assert!(disappeared.edits.iter().any(|edit| {
        edit.action == IntentEditAction::Erase
            && edit.kind == IntentKind::Defend
            && edit.cell == IVec2::new(13, 3)
    }));
}

#[test]
fn controllers_stop_planning_after_the_match_finishes() {
    let grid = IntentGrid::new(16, 16);
    let state = GameState {
        finished: true,
        ..empty_state(&grid, 0)
    };

    let decision = Controller::new(SwarmId(4)).decide(&state);
    assert!(!decision.reviewed);
    assert!(decision.edits.is_empty());
    assert_eq!(decision.work_units, 0);
}

#[test]
fn mature_funded_force_retains_attack_and_primary_mining_for_a_minute() {
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
        },
        SwarmState {
            id: enemy,
            home: enemy_home,
            minerals: 0,
        },
    ];
    let economy_bots = mature_economy_bots(owner, home);
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
    ];
    let primary_chain = funded_primary_chain(owner, home, deposits[0]);
    let enemy_facility = StructureState {
        id: 40,
        owner: enemy,
        position: enemy_home,
        kind: StructureKind::Facility,
        health: 100,
        minerals: 0,
    };
    let mut controller = Controller::new(owner);
    let initial = controller.decide(&GameState {
        grid: &grid,
        swarms: &swarms,
        bots: &economy_bots,
        structures: &[primary_chain[0], primary_chain[1], enemy_facility],
        deposits: &deposits,
        terrain: &[],
        tick: 0,
        finished: false,
    });
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
            charge: 1.0,
            cargo: 0,
        }))
        .collect::<Vec<_>>();
    for review in 1..=120 {
        let pressure = controller.decide(&GameState {
            grid: &grid,
            swarms: &supplied_swarms,
            bots: &bots,
            structures: &[primary_chain[0], primary_chain[1], enemy_facility],
            deposits: &deposits,
            terrain: &[],
            tick: review * REVIEW_PERIOD_TICKS,
            finished: false,
        });
        assert!(pressure.reviewed);
        assert!(
            pressure.explanation.contains("target [32, 0]"),
            "pressure decision lacked diagnostic coordinates: {}",
            pressure.explanation
        );
        apply(&mut grid, owner, &pressure);
        assert!(grid.iter_active_cells().any(|(cell, intent)| {
            cell != IVec2::ZERO
                && cell.abs().max_element() <= 1
                && intent.has_owned(IntentKind::Build, owner)
        }));
        assert!(
            grid.cell(IVec2::new(32, 0))
                .is_some_and(|intent| intent.has_owned(IntentKind::Defend, owner)),
            "pressure disappeared at review {review}"
        );
        assert!(
            grid.cell(IVec2::new(-1, 0))
                .is_some_and(|intent| intent.has_owned(IntentKind::Gather, owner)),
            "pressure abandoned the working primary deposit"
        );
        assert!(
            !grid
                .cell(IVec2::new(4, 0))
                .is_some_and(|intent| intent.has_owned(IntentKind::Gather, owner)),
            "pressure opened a second resource site"
        );
    }
}
