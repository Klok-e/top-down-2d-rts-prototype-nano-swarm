use top_down_2d_rts_prototype_nano_swarm::{
    agent_control::{
        AgentCommand, AgentControlCorePlugin, AgentControlHandle, AgentRequest, RequestId,
        parse_request_line,
    },
    nanobot::{
        BuildAssignment, BuildProgress, BuildSite, Cargo, Charge, Charger, ChargerAssignment,
        ChargerProgress, ChargerPulseProgress, Commitment, DefenderAttackCooldown,
        DefenderResponse, DirectMovementComponent, ExtractProgress, GatherAssignment,
        HaulerAssignment, HaulerLoading, Health, LogisticsReservation, MaintenanceAssignment,
        MaintenanceProgress, MatchOutcome, Nanobot, NanobotType, OwnerSwarm, PlannedKind,
        PlannedStructure, PlannedStructureClaim, PlannedStructureProgress, ProductionFacility,
        ReturningToStockpile, Structure, StructureKind, Swarm, SwarmEliminationState, SwarmId,
        SwarmMember, VelocityComponent, WorkBlocked,
    },
    resources::{ResourceDeposit, ResourceKind, Stockpile, StockpileRole},
};

fn state_get(
    app: &mut bevy::prelude::App,
    control: &AgentControlHandle,
    id: u64,
    cell_offset: u32,
    map_revision: Option<u64>,
    details: bool,
) -> serde_json::Value {
    let response = control
        .submit(AgentRequest {
            id: RequestId::Number(id),
            command: AgentCommand::StateGet {
                cell_offset,
                cell_limit: 1,
                map_revision,
                details,
            },
        })
        .unwrap();
    app.update();
    response.recv().unwrap().result.unwrap()
}

#[test]
fn detailed_state_get_reports_a_nanobots_observable_execution_facts() {
    use bevy::prelude::*;

    let mut app = App::new();
    let bot = app
        .world_mut()
        .spawn((
            Nanobot::default(),
            NanobotType::Defender,
            SwarmMember::new(SwarmId(7)),
            Transform::from_xyz(12.5, -8.0, 0.0),
            Health {
                current: 73,
                max: 120,
            },
            Charge {
                current: 19.5,
                max: 80.0,
            },
            Commitment::Working,
        ))
        .id();
    let (control, plugin) = AgentControlCorePlugin::channel(4);
    app.add_plugins(plugin);

    let response = control
        .submit(
            parse_request_line(br#"{"id":221,"method":"state.get","params":{"details":true}}"#)
                .expect("details is a valid optional state.get parameter"),
        )
        .unwrap();
    app.update();
    let state = response.recv().unwrap().result.unwrap();

    assert_eq!(state["execution"]["nanobots"]["total"], 1);
    assert_eq!(state["execution"]["nanobots"]["truncated"], false);
    let bot_state = &state["execution"]["nanobots"]["items"][0];
    assert_eq!(bot_state["id"], bot.to_bits());
    assert_eq!(bot_state["owner"], 7);
    assert_eq!(bot_state["type"], "defender");
    assert_eq!(
        bot_state["position"],
        serde_json::json!({"x": 12.5, "y": -8.0})
    );
    assert_eq!(
        bot_state["health"],
        serde_json::json!({"current": 73, "max": 120})
    );
    assert_eq!(
        bot_state["charge"],
        serde_json::json!({"current": 19.5, "max": 80.0})
    );
    assert_eq!(bot_state["commitment"], "working");
    assert!(bot_state["movement"].is_null());
}

#[test]
fn detailed_state_get_reports_defender_attack_and_charging_progress() {
    use bevy::prelude::*;

    let mut app = App::new();
    let charger = app.world_mut().spawn_empty().id();
    let defender = app
        .world_mut()
        .spawn((
            Nanobot::default(),
            NanobotType::Defender,
            SwarmMember::new(SwarmId(4)),
            Charge {
                current: 14.0,
                max: 100.0,
            },
            ChargerAssignment { charger },
            ChargerProgress { charger },
            ChargerPulseProgress { ticks_elapsed: 17 },
            DefenderAttackCooldown { ticks_remaining: 9 },
        ))
        .id();
    let (control, plugin) = AgentControlCorePlugin::channel(4);
    app.add_plugins(plugin);

    let state = state_get(&mut app, &control, 228, 0, None, true);
    let defender_state = state["execution"]["nanobots"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == defender.to_bits())
        .unwrap();

    assert_eq!(defender_state["type"], "defender");
    assert_eq!(
        defender_state["charger_progress"],
        serde_json::json!({"charger_id": charger.to_bits()})
    );
    assert_eq!(
        defender_state["charger_pulse_progress"],
        serde_json::json!({"ticks_elapsed": 17})
    );
    assert_eq!(
        defender_state["defender_attack_cooldown"],
        serde_json::json!({"ticks_remaining": 9})
    );
}

#[test]
fn detailed_state_get_reports_distinct_owners_and_overlapping_component_facts() {
    use bevy::prelude::*;

    let mut app = App::new();
    let dead_target = app.world_mut().spawn_empty().id();
    assert!(app.world_mut().despawn(dead_target));
    let deposit = app
        .world_mut()
        .spawn((
            ResourceDeposit {
                kind: ResourceKind::Minerals,
                amount: 41,
                capacity: 90,
                radius: 32.0,
            },
            Transform::from_xyz(400.0, -200.0, 0.0),
        ))
        .id();
    let player_swarm = app
        .world_mut()
        .spawn((Swarm::default(), SwarmId::PLAYER))
        .id();
    let opponent_swarm = app.world_mut().spawn((Swarm::default(), SwarmId(9))).id();
    let charger = app
        .world_mut()
        .spawn((
            Charger {
                cell: IVec2::new(3, 4),
                kind: ResourceKind::Minerals,
                amount: 17,
                capacity: 60,
                radius: 84.0,
            },
            Structure {
                kind: StructureKind::Basic,
                health: 64,
                ticks_since_maintained: 123,
            },
            OwnerSwarm(player_swarm),
            Transform::from_xyz(30.0, 40.0, 0.0),
        ))
        .id();
    let stockpile = app
        .world_mut()
        .spawn((
            Stockpile {
                kind: ResourceKind::Minerals,
                amount: 23,
                capacity: 70,
                radius: 50.0,
            },
            StockpileRole::Sink,
            Structure::new(StructureKind::Basic),
            OwnerSwarm(player_swarm),
            Transform::from_xyz(50.0, 60.0, 0.0),
        ))
        .id();
    let mut facility = ProductionFacility::new();
    facility.input_amount = 31;
    facility.input_capacity = 75;
    facility.progress = 44;
    facility.finished_at = Some(555);
    facility.current_target = Some(NanobotType::Hauler);
    let facility = app
        .world_mut()
        .spawn((
            facility,
            Structure::new(StructureKind::Basic),
            OwnerSwarm(opponent_swarm),
            Transform::from_xyz(-70.0, 80.0, 0.0),
        ))
        .id();
    let build_site = app
        .world_mut()
        .spawn((
            BuildSite {
                cell: IVec2::new(-2, 5),
                kind: StructureKind::Basic,
                required_materials: 20,
                consumed_materials: 7,
            },
            OwnerSwarm(player_swarm),
            Transform::from_xyz(-20.0, 50.0, 0.0),
        ))
        .id();
    let planned = app
        .world_mut()
        .spawn((
            PlannedStructure::new(PlannedKind::Charger, IVec2::new(6, -3))
                .with_work_budget(40)
                .with_work_remaining(11),
            OwnerSwarm(opponent_swarm),
            Transform::from_xyz(60.0, -30.0, 0.0),
        ))
        .id();
    let worker = app
        .world_mut()
        .spawn((
            Nanobot::default(),
            NanobotType::Worker,
            SwarmMember::new(SwarmId::PLAYER),
            Transform::from_xyz(1.0, 2.0, 0.0),
            VelocityComponent {
                value: Vec2::new(3.0, -4.0),
            },
            DirectMovementComponent {
                xy: Vec2::new(90.0, 100.0),
                stop_radius: 2.5,
                speed: Some(1.25),
                interaction: None,
            },
            WorkBlocked,
            Commitment::Carrying,
            DefenderResponse {
                target: dead_target,
            },
            ChargerAssignment { charger },
            ChargerProgress { charger },
            Cargo {
                kind: ResourceKind::Minerals,
                amount: 6,
            },
        ))
        .insert((
            GatherAssignment::new(IVec2::new(1, 2), deposit),
            ExtractProgress { collected: 4 },
            ReturningToStockpile { stockpile },
            BuildAssignment {
                cell: IVec2::new(2, 3),
                target: build_site,
            },
            BuildProgress {
                cell: IVec2::new(2, 3),
                target: build_site,
            },
            PlannedStructureClaim {
                cell: IVec2::new(6, -3),
                target: planned,
            },
            PlannedStructureProgress {
                cell: IVec2::new(6, -3),
                target: planned,
            },
            MaintenanceAssignment {
                cell: IVec2::new(3, 4),
                target: charger,
            },
            MaintenanceProgress {
                cell: IVec2::new(3, 4),
                target: charger,
                ticks_worked: 8,
            },
            HaulerAssignment {
                source: stockpile,
                sink: facility,
            },
            HaulerLoading,
            LogisticsReservation {
                source: stockpile,
                destination: facility,
                kind: ResourceKind::Minerals,
                amount: 12,
                source_remaining: 5,
                destination_remaining: 9,
            },
        ))
        .id();
    let opponent = app
        .world_mut()
        .spawn((
            Nanobot::default(),
            NanobotType::Hauler,
            SwarmMember::new(SwarmId(9)),
        ))
        .id();
    let (control, plugin) = AgentControlCorePlugin::channel(4);
    app.add_plugins(plugin);

    let state = state_get(&mut app, &control, 222, 0, None, true);
    let execution = &state["execution"];
    let bots = execution["nanobots"]["items"].as_array().unwrap();
    let worker_state = bots
        .iter()
        .find(|item| item["id"] == worker.to_bits())
        .unwrap();
    let opponent_state = bots
        .iter()
        .find(|item| item["id"] == opponent.to_bits())
        .unwrap();
    assert_eq!(worker_state["owner"], 0);
    assert_eq!(opponent_state["owner"], 9);
    assert_eq!(
        worker_state["velocity"],
        serde_json::json!({"x": 3.0, "y": -4.0})
    );
    assert_eq!(
        worker_state["movement"]["target"],
        serde_json::json!({"x": 90.0, "y": 100.0})
    );
    assert_eq!(worker_state["work_blocked"], true);
    assert_eq!(
        worker_state["defender_response"]["target_id"],
        dead_target.to_bits()
    );
    assert_eq!(
        worker_state["charger_assignment"]["charger_id"],
        charger.to_bits()
    );
    assert_eq!(
        worker_state["charger_progress"]["charger_id"],
        charger.to_bits()
    );
    assert_eq!(
        worker_state["gather_assignment"]["deposit_id"],
        deposit.to_bits()
    );
    assert_eq!(worker_state["extract_progress"]["collected"], 4);
    assert_eq!(
        worker_state["returning_to_stockpile"]["stockpile_id"],
        stockpile.to_bits()
    );
    assert_eq!(
        worker_state["build_progress"]["target_id"],
        build_site.to_bits()
    );
    assert_eq!(
        worker_state["planned_structure_progress"]["target_id"],
        planned.to_bits()
    );
    assert_eq!(worker_state["maintenance_progress"]["ticks_worked"], 8);
    assert_eq!(worker_state["hauler_loading"], true);
    assert_eq!(worker_state["logistics_reservation"]["source_remaining"], 5);
    assert_eq!(
        worker_state["logistics_reservation"]["destination_remaining"],
        9
    );

    let structures = execution["structures"]["items"].as_array().unwrap();
    let by_id = |id: Entity| {
        structures
            .iter()
            .find(|item| item["id"] == id.to_bits())
            .unwrap()
    };
    assert_eq!(by_id(charger)["owner"], 0);
    assert_eq!(by_id(charger)["owner_entity_id"], player_swarm.to_bits());
    assert_eq!(
        by_id(charger)["health"],
        serde_json::json!({"current": 64, "max": 100})
    );
    assert_eq!(by_id(charger)["maintenance_age_ticks"], 123);
    assert_eq!(by_id(charger)["charger"]["amount"], 17);
    assert_eq!(by_id(stockpile)["stockpile"]["role"], "sink");
    assert_eq!(by_id(stockpile)["stockpile"]["amount"], 23);
    assert_eq!(by_id(facility)["owner"], 9);
    assert_eq!(by_id(facility)["production_facility"]["input_amount"], 31);
    assert_eq!(by_id(facility)["production_facility"]["progress"], 44);
    assert_eq!(by_id(facility)["production_facility"]["finished_at"], 555);
    assert_eq!(
        by_id(facility)["production_facility"]["current_target"],
        "hauler"
    );
    assert_eq!(by_id(build_site)["build_site"]["consumed_materials"], 7);
    assert_eq!(by_id(build_site)["build_site"]["required_materials"], 20);
    assert_eq!(by_id(planned)["planned_structure"]["completed_work"], 29);
    assert_eq!(by_id(planned)["planned_structure"]["total_work"], 40);

    assert_eq!(execution["deposits"]["items"][0]["id"], deposit.to_bits());
    assert_eq!(
        execution["deposits"]["items"][0]["position"],
        serde_json::json!({"x": 400.0, "y": -200.0})
    );
    assert_eq!(execution["deposits"]["items"][0]["amount"], 41);

    let retained = app.world().get::<LogisticsReservation>(worker).unwrap();
    assert_eq!(
        retained.source_remaining, 5,
        "observation must not consume a reservation"
    );
    assert_eq!(app.world().get::<Cargo>(worker).unwrap().amount, 6);
    assert!(app.world().get::<WorkBlocked>(worker).is_some());
}

#[test]
fn execution_details_are_opt_in_and_only_appear_on_page_zero() {
    use bevy::prelude::*;
    use top_down_2d_rts_prototype_nano_swarm::intent::{IntentGrid, IntentKind};

    let mut grid = IntentGrid::new(5, 5);
    grid.paint(IVec2::new(0, 0), IntentKind::Gather, SwarmId::PLAYER);
    grid.paint(IVec2::new(1, 0), IntentKind::Gather, SwarmId::PLAYER);
    let mut app = App::new();
    app.insert_resource(grid).world_mut().spawn((
        Nanobot::default(),
        NanobotType::Worker,
        SwarmMember::new(SwarmId::PLAYER),
    ));
    let (control, plugin) = AgentControlCorePlugin::channel(4);
    app.add_plugins(plugin);

    let default_state = control
        .submit(parse_request_line(br#"{"id":223,"method":"state.get"}"#).unwrap())
        .unwrap();
    app.update();
    let default_state = default_state.recv().unwrap().result.unwrap();
    assert!(default_state.get("execution").is_none());

    let first = state_get(&mut app, &control, 224, 0, None, true);
    assert!(first.get("execution").is_some());
    let revision = first["map"]["map_revision"].as_u64().unwrap();
    let continuation = state_get(&mut app, &control, 225, 1, Some(revision), true);
    assert_eq!(continuation.as_object().unwrap().len(), 1);
    assert!(continuation.get("map").is_some());
    assert!(continuation.get("execution").is_none());
}

#[test]
fn state_get_rejects_non_boolean_details() {
    assert!(
        parse_request_line(br#"{"id":226,"method":"state.get","params":{"details":"yes"}}"#,)
            .is_err()
    );
}

#[test]
fn execution_collections_are_capped_and_sorted_by_stable_entity_id() {
    use bevy::prelude::*;

    let mut app = App::new();
    let mut all_ids = Vec::new();
    let mut all_structure_ids = Vec::new();
    let mut all_deposit_ids = Vec::new();
    for index in 0..260 {
        if index % 11 == 0 {
            app.world_mut().spawn_empty();
        }
        all_ids.push(
            app.world_mut()
                .spawn((
                    Nanobot::default(),
                    NanobotType::Worker,
                    SwarmMember::new(SwarmId::PLAYER),
                ))
                .id()
                .to_bits(),
        );
        all_structure_ids.push(
            app.world_mut()
                .spawn(Structure::new(StructureKind::Basic))
                .id()
                .to_bits(),
        );
        all_deposit_ids.push(
            app.world_mut()
                .spawn(ResourceDeposit {
                    kind: ResourceKind::Minerals,
                    amount: index,
                    capacity: 260,
                    radius: 32.0,
                })
                .id()
                .to_bits(),
        );
    }
    all_ids.sort_unstable();
    all_structure_ids.sort_unstable();
    all_deposit_ids.sort_unstable();
    let (control, plugin) = AgentControlCorePlugin::channel(4);
    app.add_plugins(plugin);

    let state = state_get(&mut app, &control, 227, 0, None, true);
    let nanobots = &state["execution"]["nanobots"];
    let actual_ids = nanobots["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["id"].as_u64().unwrap())
        .collect::<Vec<_>>();
    let structures = &state["execution"]["structures"];
    let actual_structure_ids = structures["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["id"].as_u64().unwrap())
        .collect::<Vec<_>>();
    let deposits = &state["execution"]["deposits"];
    let actual_deposit_ids = deposits["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["id"].as_u64().unwrap())
        .collect::<Vec<_>>();

    assert_eq!(nanobots["total"], 260);
    assert_eq!(nanobots["truncated"], true);
    assert_eq!(actual_ids.len(), 256);
    assert_eq!(actual_ids, all_ids[..256]);
    assert_eq!(structures["total"], 260);
    assert_eq!(structures["truncated"], true);
    assert_eq!(actual_structure_ids, all_structure_ids[..256]);
    assert_eq!(deposits["total"], 260);
    assert_eq!(deposits["truncated"], true);
    assert_eq!(actual_deposit_ids, all_deposit_ids[..256]);
    assert_eq!(
        app.world_mut()
            .query_filtered::<Entity, With<Nanobot>>()
            .iter(app.world())
            .count(),
        260,
        "read-only observation must not despawn overflow entities"
    );
}

#[test]
fn state_get_reports_draw_and_both_eliminated_swarms() {
    use bevy::prelude::*;

    let mut app = App::new();
    app.insert_resource(MatchOutcome::Draw)
        .insert_resource(SwarmEliminationState {
            eliminated: [SwarmId::PLAYER, SwarmId(1)].into_iter().collect(),
        });
    app.world_mut().spawn((Swarm::default(), SwarmId::PLAYER));
    app.world_mut().spawn((Swarm::default(), SwarmId(1)));
    let (control, plugin) = AgentControlCorePlugin::channel(4);
    app.add_plugins(plugin);
    let response = control
        .submit(AgentRequest {
            id: RequestId::Number(220),
            command: AgentCommand::StateGet {
                cell_offset: 0,
                cell_limit: 1000,
                map_revision: None,
                details: false,
            },
        })
        .unwrap();
    app.update();
    let response = response.recv().unwrap();
    assert!(response.ok, "terminal matches must remain inspectable");
    let state = response.result.unwrap();
    assert_eq!(
        state["match"],
        serde_json::json!({
            "outcome": "draw",
            "player_eliminated": true,
            "opponent_eliminated": true,
        })
    );
    let swarms = state["swarms"].as_array().unwrap();
    assert_eq!(swarms.len(), 2);
    for swarm in swarms {
        assert_eq!(swarm["eliminated"], true);
        assert!(swarm.get("collapsed").is_none());
    }
}
