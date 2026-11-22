use bevy::{prelude::*, time::TimeUpdateStrategy};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};
use top_down_2d_rts_prototype_nano_swarm::{
    ai_battle::{AiBattleConfig, AiBattleLayout},
    battle_statistics::{BattleStatisticsConfig, BattleStatisticsPlugin},
    intent::{IntentGrid, IntentKind},
    nanobot::{NanobotType, SwarmId},
    scenario_selection::{Scenario, ScenarioSelection},
};
#[path = "../common/mod.rs"]
mod common;

struct Output(PathBuf);
impl Output {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        Self(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join(".tmp")
                .join(format!(
                    "nano-battle-test-{}-{}",
                    std::process::id(),
                    NEXT.fetch_add(1, Ordering::Relaxed)
                )),
        )
    }
    fn summary(&self) -> serde_json::Value {
        let directory = std::fs::read_dir(&self.0)
            .expect("battle must create a run directory")
            .find(|entry| entry.as_ref().is_ok_and(|entry| entry.path().is_dir()))
            .unwrap()
            .unwrap()
            .path();
        serde_json::from_slice(&std::fs::read(directory.join("summary.json")).unwrap()).unwrap()
    }

    fn samples_csv(&self) -> String {
        let directory = std::fs::read_dir(&self.0)
            .expect("battle must create a run directory")
            .find(|entry| entry.as_ref().is_ok_and(|entry| entry.path().is_dir()))
            .unwrap()
            .unwrap()
            .path();
        std::fs::read_to_string(directory.join("samples.csv")).unwrap()
    }
}

impl Drop for Output {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn battle(output: &Output) -> App {
    let mut app = common::sim_app_with_elimination();
    record_battle(&mut app, output);
    common::spawn_swarm_with_nanobots(&mut app, Vec2::ZERO, &[(NanobotType::Worker, 1)]);
    common::spawn_opponent_swarm_with_nanobots(
        &mut app,
        Vec2::X * 500.,
        &[(NanobotType::Defender, 1)],
    );
    app
}

fn record_battle(app: &mut App, output: &Output) {
    let mut selection = ScenarioSelection::default();
    selection.current = Scenario::AiBattle;
    app.insert_resource(selection);
    app.insert_resource(top_down_2d_rts_prototype_nano_swarm::fixed_simulation_time());
    app.insert_resource(TimeUpdateStrategy::FixedTimesteps(1));
    app.insert_resource(BattleStatisticsConfig {
        output_root: output.0.clone(),
        ..default()
    });
    app.insert_resource(AiBattleConfig::default());
    app.add_plugins(BattleStatisticsPlugin);
}

#[test]
fn battle_persists_per_second_samples_using_simulation_time() {
    let output = Output::new();
    let mut app = battle(&output);
    app.insert_resource(AiBattleConfig {
        layout: AiBattleLayout::Flanks,
    });
    for _ in 0..60 {
        app.update();
    }
    let summary = output.summary();
    assert_eq!(summary["status"], "in_progress");
    assert_eq!(summary["latest"]["swarms"]["0"]["workers"], 1);
    assert_eq!(summary["latest"]["swarms"]["1"]["defenders"], 1);
    assert_eq!(summary["ai_battle"]["layout"], "flanks");
    assert!((summary["latest"]["simulation_seconds"].as_f64().unwrap() - 1.).abs() < 0.001);
    assert_eq!(summary["latest"]["tick_timing"]["count"], 60);
    assert!(
        summary["latest"]["frame_timing"]["count"]
            .as_u64()
            .unwrap_or(0)
            > 0,
        "watchable runs need frame timing"
    );
}

#[test]
fn headless_frame_timing_follows_the_generic_unbounded_option() {
    for (unbounded, expects_frame_timing) in [(true, false), (false, true)] {
        let output = Output::new();
        let mut app = battle(&output);
        let mut config = app.world_mut().resource_mut::<BattleStatisticsConfig>();
        config.headless = true;
        config.unbounded = unbounded;

        for _ in 0..60 {
            app.update();
        }

        let summary = output.summary();
        let csv = output.samples_csv();
        let frame_peak_column = csv
            .lines()
            .next()
            .unwrap()
            .split(',')
            .position(|name| name == "frame_max_ms")
            .expect("frame peaks must remain available after later samples replace the summary");
        let frame_peak = csv
            .lines()
            .last()
            .unwrap()
            .split(',')
            .nth(frame_peak_column)
            .unwrap();
        if expects_frame_timing {
            assert!(
                summary["latest"]["frame_timing"]["count"]
                    .as_u64()
                    .is_some_and(|count| count > 0)
            );
            let recorded_peak = frame_peak.parse::<f64>().unwrap();
            let latest_peak = summary["latest"]["frame_timing"]["max_ms"]
                .as_f64()
                .unwrap();
            assert!((recorded_peak - latest_peak).abs() <= 1e-9 * latest_peak.abs().max(1.0));
        } else {
            assert!(summary["latest"]["frame_timing"].is_null());
            assert!(frame_peak.is_empty());
        }
    }
}

#[test]
fn battle_end_flushes_partial_interval_and_freezes_results_while_world_advances() {
    use top_down_2d_rts_prototype_nano_swarm::nanobot::{Health, SwarmMember};
    let output = Output::new();
    let mut app = battle(&output);
    for _ in 0..10 {
        app.update();
    }
    for (owner, mut health) in app
        .world_mut()
        .query::<(&SwarmMember, &mut Health)>()
        .iter_mut(app.world_mut())
    {
        if !owner.0.is_player() {
            health.current = 0;
        }
    }
    app.update();
    let terminal = output.summary();
    assert_eq!(terminal["status"], "completed");
    assert_eq!(terminal["outcome"], "swarm_0_wins");
    assert_eq!(terminal["latest"]["fixed_tick"], 11);
    assert_eq!(terminal["latest"]["swarms"]["1"]["population"], 0);
    assert!(
        terminal["latest"]["swarms"]["1"]["elimination_seconds"]
            .as_f64()
            .unwrap()
            > 0.
    );
    let elapsed = app.world().resource::<Time<Fixed>>().elapsed();
    for _ in 0..120 {
        app.update();
    }
    assert!(app.world().resource::<Time<Fixed>>().elapsed() > elapsed);
    assert_eq!(
        output.summary(),
        terminal,
        "post-battle updates must not rewrite results"
    );
}

#[test]
fn interruption_saves_partial_statistics_without_inventing_an_outcome() {
    let output = Output::new();
    let mut app = battle(&output);
    for _ in 0..7 {
        app.update();
    }
    app.world_mut().write_message(AppExit::from_code(130));
    app.update();
    let summary = output.summary();
    assert_eq!(summary["status"], "interrupted");
    assert!(summary["outcome"].is_null());
    assert_eq!(summary["latest"]["fixed_tick"], 8);
    assert_eq!(summary["latest"]["swarms"]["0"]["population"], 1);
}

#[test]
fn samples_preserve_gross_events_and_stop_counters_after_draw() {
    use top_down_2d_rts_prototype_nano_swarm::{
        battle_statistics::{BattleCounters, BattleEvent},
        nanobot::{Health, SwarmId},
    };
    let output = Output::new();
    let mut app = battle(&output);
    app.update();
    {
        let mut counters = app.world_mut().resource_mut::<BattleCounters>();
        for event in [
            BattleEvent::Birth,
            BattleEvent::Death,
            BattleEvent::Gathered {
                amount: 30,
                deposit: Entity::PLACEHOLDER,
                painted: false,
            },
            BattleEvent::Consumed(30),
            BattleEvent::StructureBuilt,
            BattleEvent::StructureLost,
        ] {
            counters.record(SwarmId::PLAYER, event);
        }
    }
    for _ in 1..60 {
        app.update();
    }
    let summary = output.summary();
    for field in ["births", "deaths", "structures_built", "structures_lost"] {
        assert_eq!(
            summary["latest"]["swarms"]["0"][field], 1,
            "gross {field} must survive a net-zero interval"
        );
    }
    assert_eq!(summary["latest"]["swarms"]["0"]["minerals_gathered"], 30);
    assert_eq!(summary["latest"]["swarms"]["0"]["minerals_consumed"], 30);
    assert_eq!(summary["schema_version"], 8);
    assert_eq!(
        summary["latest"]["swarms"]["0"]["effective_damage_total"],
        0
    );
    assert_eq!(
        summary["latest"]["swarms"]["0"]["effective_damage_nanobots"],
        0
    );
    assert_eq!(
        summary["latest"]["swarms"]["0"]["effective_damage_structures"], 0,
        "non-combat structure loss must not count as hostile combat damage",
    );
    assert_eq!(summary["latest"]["swarms"]["1"]["births"], 0);
    for mut health in app
        .world_mut()
        .query::<&mut Health>()
        .iter_mut(app.world_mut())
    {
        health.current = 0;
    }
    app.update();
    let completed = output.summary();
    assert_eq!(completed["outcome"], "draw");
    assert_eq!(
        completed["latest"]["swarms"]["0"]["elimination_seconds"],
        completed["latest"]["swarms"]["1"]["elimination_seconds"]
    );
    app.world_mut()
        .resource_mut::<BattleCounters>()
        .record(SwarmId::PLAYER, BattleEvent::Birth);
    assert_eq!(
        app.world()
            .resource::<BattleCounters>()
            .totals_for(SwarmId::PLAYER)
            .births,
        1
    );
    let frozen = serde_json::to_value(
        app.world()
            .resource::<BattleCounters>()
            .totals_for(SwarmId::PLAYER),
    )
    .unwrap();
    assert_eq!(frozen["effective_damage_total"], 0);
    app.world_mut().write_message(AppExit::from_code(130));
    app.update();
    assert_eq!(
        output.summary(),
        completed,
        "interrupting a completed battle preserves its result"
    );
}

#[test]
fn effective_damage_is_cumulative_in_json_and_csv_and_freezes_at_match_end() {
    use top_down_2d_rts_prototype_nano_swarm::{
        battle_statistics::{BattleCounters, BattleEvent},
        nanobot::SwarmId,
    };
    let output = Output::new();
    let mut app = battle(&output);
    app.update();
    {
        let mut counters = app.world_mut().resource_mut::<BattleCounters>();
        counters.record(SwarmId::PLAYER, BattleEvent::EffectiveNanobotDamage(17));
        counters.record(SwarmId::PLAYER, BattleEvent::EffectiveStructureDamage(5));
    }
    for _ in 1..60 {
        app.update();
    }

    for mut health in app
        .world_mut()
        .query::<&mut top_down_2d_rts_prototype_nano_swarm::nanobot::Health>()
        .iter_mut(app.world_mut())
    {
        health.current = 0;
    }
    app.update();

    let terminal = output.summary();
    assert_eq!(terminal["schema_version"], 8);
    let swarm = &terminal["latest"]["swarms"]["0"];
    assert_eq!(swarm["effective_damage_total"], 22);
    assert_eq!(swarm["effective_damage_nanobots"], 17);
    assert_eq!(swarm["effective_damage_structures"], 5);

    let csv = output.samples_csv();
    let header = csv.lines().next().unwrap().split(',').collect::<Vec<_>>();
    let row = csv
        .lines()
        .skip(1)
        .map(|line| line.split(',').collect::<Vec<_>>())
        .find(|columns| columns.get(2) == Some(&"0"))
        .expect("the terminal sample must contain the player swarm");
    for (field, expected) in [
        ("effective_damage_total", "22"),
        ("effective_damage_nanobots", "17"),
        ("effective_damage_structures", "5"),
    ] {
        let index = header.iter().position(|name| *name == field).unwrap();
        assert_eq!(row[index], expected);
    }

    app.world_mut()
        .resource_mut::<BattleCounters>()
        .record(SwarmId::PLAYER, BattleEvent::EffectiveNanobotDamage(100));
    let frozen = serde_json::to_value(
        app.world()
            .resource::<BattleCounters>()
            .totals_for(SwarmId::PLAYER),
    )
    .unwrap();
    assert_eq!(frozen["effective_damage_total"], 22);
}

#[test]
fn frame_statistics_include_frames_without_fixed_ticks() {
    let output = Output::new();
    let mut app = battle(&output);
    app.insert_resource(TimeUpdateStrategy::ManualDuration(
        std::time::Duration::from_nanos(8_333_334),
    ));
    for _ in 0..120 {
        app.update();
    }
    let summary = output.summary();
    assert_eq!(summary["latest"]["tick_timing"]["count"], 60);
    assert!(
        summary["latest"]["frame_timing"]["count"].as_u64().unwrap() > 100,
        "120 application frames must not be reduced to the 60 frames containing fixed ticks"
    );
}

#[test]
fn recording_uses_session_policy_without_a_scenario_selection() {
    use top_down_2d_rts_prototype_nano_swarm::session::SessionRules;
    for enabled in [false, true] {
        let output = Output::new();
        let mut app = App::new();
        app.add_plugins(bevy::time::TimePlugin)
            .insert_resource(top_down_2d_rts_prototype_nano_swarm::fixed_simulation_time())
            .insert_resource(TimeUpdateStrategy::FixedTimesteps(1))
            .insert_resource(SessionRules {
                scenario_name: "observation",
                record_statistics: enabled,
                ..default()
            })
            .insert_resource(BattleStatisticsConfig {
                output_root: output.0.clone(),
                ..default()
            })
            .add_plugins(BattleStatisticsPlugin);
        for _ in 0..61 {
            app.update();
        }
        if enabled {
            assert_eq!(output.summary()["scenario"], "observation");
            assert_eq!(output.summary()["samples"], 1);
        } else {
            assert!(
                !output.0.exists(),
                "disabled recording must not create output"
            );
        }
    }
}

#[test]
fn failed_summary_write_signals_failure_and_preserves_last_flushed_summary() {
    let output = Output::new();
    let mut app = battle(&output);
    app.update();
    let previous = output.summary();
    let directory = std::fs::read_dir(&output.0)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    std::fs::create_dir(directory.join("summary.json.tmp")).unwrap();
    for _ in 1..60 {
        app.update();
    }
    assert_eq!(app.should_exit(), Some(AppExit::error()));
    assert_eq!(
        output.summary(),
        previous,
        "a failed atomic replacement must preserve the preceding summary"
    );
}

#[test]
fn recorded_outcomes_use_actual_swarm_ids_and_session_perspective() {
    use top_down_2d_rts_prototype_nano_swarm::{
        nanobot::{Health, SwarmId, SwarmMember},
        session::{OutcomeMode, SessionRules},
    };
    for (mode, expected) in [
        (OutcomeMode::SwarmRelative, "swarm_42_wins"),
        (OutcomeMode::PlayerRelative, "victory"),
    ] {
        let output = Output::new();
        let mut app = battle(&output);
        app.update();
        {
            let mut rules = app.world_mut().resource_mut::<SessionRules>();
            rules.player_swarm = Some(SwarmId(42));
            rules.outcomes = mode;
        }
        for mut id in app
            .world_mut()
            .query::<&mut SwarmId>()
            .iter_mut(app.world_mut())
        {
            id.0 = if id.0 == 0 { 7 } else { 42 };
        }
        for (mut member, mut health) in app
            .world_mut()
            .query::<(&mut SwarmMember, &mut Health)>()
            .iter_mut(app.world_mut())
        {
            if member.0.0 == 0 {
                member.0 = SwarmId(7);
                health.current = 0;
            } else {
                member.0 = SwarmId(42);
            }
        }
        app.update();
        let saved = output.summary();
        assert_eq!(saved["outcome"], expected);
        assert!(
            saved["latest"]["swarms"]["7"]["elimination_seconds"]
                .as_f64()
                .unwrap()
                > 0.
        );
        assert!(saved["latest"]["swarms"]["42"]["elimination_seconds"].is_null());
    }
}

#[test]
fn battle_records_initial_tuning_and_only_its_own_tick_stamped_edits() {
    use top_down_2d_rts_prototype_nano_swarm::tuning::{
        Tuning, TuningJournal, TuningRevision, TuningStore, set_value,
    };
    let output = Output::new();
    let mut app = battle(&output);
    app.init_resource::<Tuning>()
        .init_resource::<TuningJournal>()
        .init_resource::<TuningRevision>()
        .insert_resource(TuningStore::new(output.0.join("balance.ron")));
    std::fs::create_dir_all(&output.0).unwrap();
    set_value(app.world_mut(), "combat.base_attack", 14.).unwrap();
    app.update();
    for _ in 1..7 {
        app.update();
    }
    set_value(app.world_mut(), "combat.base_attack", 23.).unwrap();
    app.world_mut().write_message(AppExit::from_code(130));
    app.update();
    let summary = output.summary();
    assert_eq!(summary["tuning"]["combat"]["base_attack"], 14.);
    let edits = summary["tuning_edits"].as_array().unwrap();
    assert_eq!(
        edits.len(),
        1,
        "edits before the run are already in its initial tuning"
    );
    assert_eq!(edits[0]["key"], "combat.base_attack");
    assert_eq!(edits[0]["value"], 23.);
    assert_eq!(edits[0]["revision"], 2);
    assert_eq!(edits[0]["fixed_tick"], 7);
}

fn latest_csv_columns(output: &Output, swarm: &str, fields: &[&str]) -> Vec<f64> {
    let csv = output.samples_csv();
    let mut lines = csv.lines();
    let headers: Vec<_> = lines.next().unwrap().split(',').collect();
    let row = lines
        .rev()
        .map(|line| line.split(',').collect::<Vec<_>>())
        .find(|columns| columns[2] == swarm)
        .unwrap();
    fields
        .iter()
        .map(|field| {
            let column = headers
                .iter()
                .position(|header| header == field)
                .unwrap_or_else(|| panic!("missing CSV column {field}"));
            row[column].parse().unwrap()
        })
        .collect()
}

#[test]
fn battle_samples_distinguish_painted_deposits_that_yield_minerals() {
    use top_down_2d_rts_prototype_nano_swarm::{
        nanobot::{
            Cargo, ExtractProgress, GatherAssignment, LogisticsReservation, MaterialWorkProgress,
            worker_gather_extract_system,
        },
        resources::{ResourceDeposit, ResourceKind},
        tuning::Tuning,
    };

    let output = Output::new();
    let mut app = common::minimal_app();
    app.add_systems(FixedUpdate, worker_gather_extract_system);
    app.world_mut()
        .resource_mut::<Tuning>()
        .logistics
        .material_work_ticks = 24;
    record_battle(&mut app, &output);
    let position = Vec2::splat(128.0);
    common::spawn_swarm_at(&mut app, position);
    let worker = common::spawn_worker_at(&mut app, position + Vec2::X * 68.0);
    let source = common::spawn_stockpile(&mut app, position + Vec2::Y * 172.0, 0, 100);
    let mined = common::spawn_deposit(
        &mut app,
        common::DepositFixture {
            world_pos: position,
            amount: 100,
            capacity: 100,
            radius: 32.0,
        },
    );
    app.world_mut().entity_mut(worker).insert((
        GatherAssignment::new(IVec2::ZERO, mined),
        ExtractProgress::default(),
        Cargo::empty(ResourceKind::Minerals),
        LogisticsReservation::new(mined, source, ResourceKind::Minerals, 4),
        MaterialWorkProgress::default(),
    ));
    let distant_cell = IVec2::new(3, 0);
    let distant = common::spawn_deposit(
        &mut app,
        common::DepositFixture {
            world_pos: common::cell_world_center(distant_cell),
            amount: 100,
            capacity: 100,
            radius: 32.0,
        },
    );
    for cell in [IVec2::ZERO, distant_cell] {
        app.world_mut().resource_mut::<IntentGrid>().paint(
            cell,
            IntentKind::Gather,
            SwarmId::PLAYER,
        );
    }

    for _ in 0..60 {
        app.update();
    }
    let income = 100 - app.world().get::<ResourceDeposit>(mined).unwrap().amount;
    assert!(
        income > 1,
        "multiple extraction pulses should share one deposit"
    );
    assert_eq!(
        app.world().get::<ResourceDeposit>(distant).unwrap().amount,
        100
    );
    let fields = ["owned_deposits", "mineral_income", "unspent_minerals"];
    assert_eq!(
        latest_csv_columns(&output, "0", &fields),
        [1.0, f64::from(income), f64::from(income)],
    );

    app.world_mut().resource_mut::<IntentGrid>().erase(
        IVec2::ZERO,
        IntentKind::Gather,
        SwarmId::PLAYER,
    );
    app.world_mut()
        .entity_mut(worker)
        .remove::<GatherAssignment>();
    for _ in 0..20 {
        app.update();
    }
    app.world_mut().write_message(AppExit::Success);
    app.update();
    assert_eq!(
        latest_csv_columns(&output, "0", &fields),
        [0.0, 0.0, f64::from(income)],
    );
    assert_eq!(
        output.summary()["latest"]["swarms"]["0"]["minerals_gathered"],
        income
    );
    assert_eq!(
        output.summary()["latest"]["swarms"]["0"]["deposit_extraction"],
        serde_json::json!([
            {"cell": [0, 0], "owner": null, "minerals_extracted": income},
            {"cell": [3, 0], "owner": null, "minerals_extracted": 0},
        ])
    );
}

#[test]
fn battle_samples_measure_unfunded_demand_and_empty_charge_per_swarm() {
    use top_down_2d_rts_prototype_nano_swarm::nanobot::{
        Charge, OwnerSwarm, PopulationDemand, ProductionFacility, SwarmMember,
        production_facility_pick_target_system,
    };

    let output = Output::new();
    let mut app = common::sim_app_with_population_demand();
    app.insert_resource(IntentGrid::new(32, 32));
    record_battle(&mut app, &output);
    app.add_systems(FixedUpdate, production_facility_pick_target_system);
    let player = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let opponent = common::spawn_opponent_swarm_with_nanobots(
        &mut app,
        Vec2::X * 2500.0,
        &[(NanobotType::Defender, 1)],
    );
    common::spawn_defender_at(&mut app, Vec2::splat(128.0));
    for (member, mut charge) in app
        .world_mut()
        .query::<(&SwarmMember, &mut Charge)>()
        .iter_mut(app.world_mut())
    {
        if !member.0.is_player() {
            charge.current = 0.0;
        }
    }
    for x in 0..8 {
        app.world_mut().resource_mut::<IntentGrid>().paint(
            IVec2::new(x, 0),
            IntentKind::Defend,
            SwarmId::PLAYER,
        );
    }
    let funded = common::spawn_facility_at(&mut app, player, Vec2::splat(256.0));
    for owner in [player, opponent] {
        app.world_mut()
            .spawn((ProductionFacility::new(), OwnerSwarm(owner)));
    }

    for _ in 0..60 {
        app.update();
    }
    assert_eq!(
        app.world()
            .resource::<PopulationDemand>()
            .desired_for(SwarmId::PLAYER, NanobotType::Defender,),
        4
    );
    assert_eq!(
        app.world()
            .get::<ProductionFacility>(funded)
            .unwrap()
            .current_target,
        Some(NanobotType::Defender),
    );
    let fields = ["mineral_starved_facility_seconds", "empty_charge_defenders"];
    let player_sample = latest_csv_columns(&output, "0", &fields);
    assert!((player_sample[0] - 1.0).abs() < 1e-6, "{player_sample:?}");
    assert_eq!(player_sample[1], 0.0);
    assert_eq!(latest_csv_columns(&output, "1", &fields), [0.0, 1.0]);
}

#[test]
fn battle_unspent_minerals_drop_when_a_stockpile_collapses() {
    use top_down_2d_rts_prototype_nano_swarm::{
        nanobot::{
            Cargo, Charger, OwnerSwarm, ProductionFacility, Structure, StructureKind,
            structure_degradation_system,
        },
        resources::{ResourceKind, ResourceLedger},
        tuning::Tuning,
    };

    let output = Output::new();
    let mut app = common::minimal_app();
    record_battle(&mut app, &output);
    app.add_systems(FixedUpdate, structure_degradation_system);

    let owner = OwnerSwarm(common::spawn_swarm_at(&mut app, Vec2::ZERO));
    let stockpile = common::spawn_stockpile(&mut app, Vec2::X * 128.0, 40, 100);
    let worker = common::spawn_worker_at(&mut app, Vec2::Y * 128.0);
    let world = app.world_mut();
    world.entity_mut(stockpile).insert(owner);
    world.entity_mut(worker).insert(Cargo {
        kind: ResourceKind::Minerals,
        amount: 3,
    });
    let mut facility = ProductionFacility::new();
    facility.input_amount = 12;
    world.spawn((facility, owner));
    let mut charger = Charger::new(IVec2::ZERO);
    charger.amount = 7;
    world.spawn((charger, owner));
    world
        .resource_mut::<ResourceLedger>()
        .add_for(SwarmId::PLAYER, ResourceKind::Minerals, 62);

    let columns = ["unspent_minerals"];
    for _ in 0..60 {
        app.update();
    }
    assert_eq!(latest_csv_columns(&output, "0", &columns), [62.0]);

    let world = app.world_mut();
    let maintenance = &world.resource::<Tuning>().maintenance;
    let condition = Structure {
        kind: StructureKind::Basic,
        health: 1,
        ticks_since_maintained: maintenance.buffer_ticks + maintenance.degradation_interval_ticks
            - 1,
    };
    world.entity_mut(stockpile).insert(condition);
    for _ in 0..60 {
        app.update();
    }
    assert!(app.world().get_entity(stockpile).is_err());
    assert_eq!(latest_csv_columns(&output, "0", &columns), [22.0]);
}
