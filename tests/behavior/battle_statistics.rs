use bevy::{prelude::*, time::TimeUpdateStrategy};
use std::time::Duration;
use top_down_2d_rts_prototype_nano_swarm::{
    agent_control::{AgentControlCorePlugin, AgentControlHandle, parse_request_line},
    ai_battle::{AiBattleConfig, AiBattleLayout},
    battle_statistics::{BattleRun, BattleStatisticsConfig, BattleStatisticsPlugin},
    intent::{IntentGrid, IntentKind},
    nanobot::{NanobotType, SwarmId},
    perf::RecorderClock,
    scenario_selection::{Scenario, ScenarioSelection},
};
#[path = "../common/mod.rs"]
mod common;

fn battle() -> App {
    let mut app = common::sim_app_with_elimination();
    record_battle(&mut app);
    let player =
        common::spawn_swarm_with_nanobots(&mut app, Vec2::ZERO, &[(NanobotType::Worker, 1)]);
    common::spawn_home_facility(&mut app, player);
    let opponent = common::spawn_opponent_swarm_with_nanobots(
        &mut app,
        Vec2::X * 500.,
        &[(NanobotType::Defender, 1)],
    );
    common::spawn_home_facility(&mut app, opponent);
    app
}

#[derive(Resource, Clone)]
struct StatsHandle(AgentControlHandle);

/// Query `battle.stats` without advancing the fixed simulation.
fn query_stats(
    app: &mut App,
    after_sample: Option<u64>,
    after_hitch: Option<u64>,
) -> top_down_2d_rts_prototype_nano_swarm::agent_control::AgentResponse {
    let handle = app.world().resource::<StatsHandle>().0.clone();
    let params = serde_json::json!({"after_sample": after_sample, "after_hitch": after_hitch});
    let line = serde_json::json!({"id": 1, "method": "battle.stats", "params": params});
    let reply = handle
        .submit(parse_request_line(line.to_string().as_bytes()).unwrap())
        .unwrap();
    let strategy = std::mem::replace(
        &mut *app.world_mut().resource_mut::<TimeUpdateStrategy>(),
        TimeUpdateStrategy::ManualDuration(Duration::ZERO),
    );
    app.update();
    app.insert_resource(strategy);
    reply
        .try_recv()
        .expect("battle.stats replies in one update")
}

fn stats(app: &mut App) -> serde_json::Value {
    let response = query_stats(app, None, None);
    assert!(response.ok, "{response:?}");
    response.result.unwrap()
}

fn swarm_row(sample: &serde_json::Value, swarm: u64) -> &serde_json::Value {
    sample["swarms"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["swarm"] == swarm)
        .unwrap_or_else(|| panic!("sample has no swarm {swarm}"))
}

/// The newest sample's row for one swarm.
fn latest_row(app: &mut App, swarm: u64) -> serde_json::Value {
    let stats = stats(app);
    swarm_row(stats["samples"].as_array().unwrap().last().unwrap(), swarm).clone()
}

fn record_battle(app: &mut App) {
    record_battle_with(app, BattleStatisticsConfig::default());
}

fn record_battle_with(app: &mut App, config: BattleStatisticsConfig) {
    let (handle, channel) = AgentControlCorePlugin::channel(4);
    app.insert_resource(StatsHandle(handle))
        .add_plugins(channel);
    let mut selection = ScenarioSelection::default();
    selection.current = Scenario::AiBattle;
    app.insert_resource(selection);
    app.insert_resource(top_down_2d_rts_prototype_nano_swarm::fixed_simulation_time());
    app.insert_resource(TimeUpdateStrategy::FixedTimesteps(1));
    app.insert_resource(config);
    app.insert_resource(AiBattleConfig::default());
    app.add_plugins(BattleStatisticsPlugin);
}

#[derive(Resource)]
struct FrameWork {
    update_ms: u64,
    post_update_ms: u64,
}

#[test]
fn battle_stats_serve_one_compact_summary_of_the_whole_hitch_episode() {
    let mut app = battle();
    app.insert_resource(RecorderClock::manual())
        .insert_resource(FrameWork {
            update_ms: 4,
            post_update_ms: 1,
        })
        .add_systems(PreUpdate, |mut clock: ResMut<RecorderClock>| {
            clock.advance(Duration::from_millis(2));
        })
        .add_systems(FixedUpdate, |mut clock: ResMut<RecorderClock>| {
            clock.advance(Duration::from_millis(3));
        })
        .add_systems(
            Update,
            |work: Res<FrameWork>, mut clock: ResMut<RecorderClock>| {
                clock.advance(Duration::from_millis(work.update_ms));
            },
        )
        .add_systems(
            PostUpdate,
            |work: Res<FrameWork>, mut clock: ResMut<RecorderClock>| {
                clock.advance(Duration::from_millis(work.post_update_ms));
            },
        );
    for frame in 1..=770 {
        *app.world_mut().resource_mut::<FrameWork>() = FrameWork {
            update_ms: if (121..=740).contains(&frame) { 34 } else { 4 },
            // Frame 420 falls in the middle discarded by the frame-detail cap.
            post_update_ms: if frame == 420 { 60_000 } else { 1 },
        };
        app.insert_resource(TimeUpdateStrategy::FixedTimesteps(if frame == 420 {
            3
        } else {
            1
        }));
        app.update();
    }
    app.update();

    let payload = stats(&mut app);
    let episodes = payload["hitches"].as_array().unwrap();
    assert_eq!(episodes.len(), 1);
    assert_eq!(
        episodes[0],
        serde_json::json!({
            "start_frame": 1,
            "end_frame": 770,
            "start_fixed_tick": 0,
            "hitch_count": 620,
            "worst_frame_ns": 60_045_000_000u64,
            "catch_up_ticks": 2,
            "top_phases": [
                {"phase": "PostUpdate", "duration_ns": 60_619_000_000u64},
                {"phase": "Update", "duration_ns": 21_080_000_000u64},
                {"phase": "fixed_other", "duration_ns": 1_866_000_000},
            ],
            "nanobots": 2,
        })
    );
    assert_eq!(payload["summary"]["schema_version"], 9);
}

#[test]
fn battle_records_per_second_samples_using_simulation_time() {
    let mut app = battle();
    app.insert_resource(AiBattleConfig {
        layout: AiBattleLayout::Flanks,
    });
    for _ in 0..60 {
        app.update();
    }
    let payload = stats(&mut app);
    assert_eq!(payload["summary"]["status"], "in_progress");
    assert_eq!(payload["samples"].as_array().unwrap().len(), 1);
    let sample = &payload["samples"][0];
    assert_eq!(swarm_row(sample, 0)["workers"], 1);
    assert_eq!(swarm_row(sample, 1)["defenders"], 1);
    assert_eq!(payload["summary"]["ai_battle"]["layout"], "flanks");
    let row = swarm_row(sample, 0);
    assert_eq!(row["tick_count"], 60);
    assert!((row["simulation_seconds"].as_f64().unwrap() - 1.).abs() < 0.001);
    assert!(
        row["frame_count"].as_u64().unwrap() > 0,
        "watchable runs need frame timing"
    );
}

#[test]
fn headless_frame_timing_follows_the_generic_unbounded_option() {
    for (unbounded, expects_frame_timing) in [(true, false), (false, true)] {
        let mut app = battle();
        let mut config = app.world_mut().resource_mut::<BattleStatisticsConfig>();
        config.headless = true;
        config.unbounded = unbounded;

        for _ in 0..60 {
            app.update();
        }

        let row = latest_row(&mut app, 0);
        if expects_frame_timing {
            assert!(row["frame_count"].as_u64().is_some_and(|count| count > 0));
            assert!(row["frame_max_ms"].is_f64());
        } else {
            assert!(row["frame_count"].is_null());
            assert!(row["frame_max_ms"].is_null());
        }
    }
}

#[test]
fn samples_preserve_gross_events_and_stop_counters_after_draw() {
    use top_down_2d_rts_prototype_nano_swarm::{
        battle_statistics::{BattleCounters, BattleEvent},
        nanobot::SwarmId,
    };
    let mut app = battle();
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
    let row = latest_row(&mut app, 0);
    for field in ["births", "deaths", "structures_built", "structures_lost"] {
        assert_eq!(
            row[field], 1,
            "gross {field} must survive a net-zero interval"
        );
    }
    assert_eq!(row["minerals_gathered"], 30);
    assert_eq!(row["minerals_consumed"], 30);
    assert_eq!(row["effective_damage_total"], 0);
    assert_eq!(row["effective_damage_nanobots"], 0);
    assert_eq!(
        row["effective_damage_structures"], 0,
        "non-combat structure loss must not count as hostile combat damage",
    );
    assert_eq!(latest_row(&mut app, 1)["births"], 0);
    common::destroy_structures(&mut app, |_| true);
    app.update();
    let completed = stats(&mut app);
    assert_eq!(completed["summary"]["outcome"], "draw");
    let last = completed["samples"].as_array().unwrap().last().unwrap();
    assert_eq!(
        swarm_row(last, 0)["elimination_seconds"],
        swarm_row(last, 1)["elimination_seconds"]
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
    advance(&mut app, 60);
    assert_eq!(
        stats(&mut app),
        completed,
        "a completed battle's results never change"
    );
}

#[test]
fn effective_damage_is_cumulative_and_freezes_at_match_end() {
    use top_down_2d_rts_prototype_nano_swarm::{
        battle_statistics::{BattleCounters, BattleEvent},
        nanobot::SwarmId,
    };
    let mut app = battle();
    app.update();
    {
        let mut counters = app.world_mut().resource_mut::<BattleCounters>();
        counters.record(SwarmId::PLAYER, BattleEvent::EffectiveNanobotDamage(17));
        counters.record(SwarmId::PLAYER, BattleEvent::EffectiveStructureDamage(5));
    }
    for _ in 1..60 {
        app.update();
    }

    common::destroy_structures(&mut app, |_| true);
    app.update();

    let payload = stats(&mut app);
    let row = swarm_row(payload["samples"].as_array().unwrap().last().unwrap(), 0);
    assert_eq!(payload["summary"]["schema_version"], 9);
    assert_eq!(row["effective_damage_total"], 22);
    assert_eq!(row["effective_damage_nanobots"], 17);
    assert_eq!(row["effective_damage_structures"], 5);
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
fn frame_statistics_include_active_frames_without_fixed_ticks_and_omit_paused_frames() {
    let mut app = battle();
    app.insert_resource(RecorderClock::manual());
    app.add_systems(
        Update,
        |mut frame: Local<usize>,
         mut time: ResMut<Time<Virtual>>,
         mut clock: ResMut<RecorderClock>| {
            *frame += 1;
            match *frame {
                30 => time.pause(),
                36 => time.unpause(),
                _ => {}
            }
            clock.advance(Duration::from_millis(if time.is_paused() { 40 } else { 4 }));
        },
    );
    app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_nanos(
        8_333_334,
    )));
    for _ in 0..126 {
        app.update();
    }
    let row = latest_row(&mut app, 0);
    assert_eq!(row["tick_count"], 60);
    assert_eq!(row["frame_count"], 119);
    assert_eq!(row["frame_max_ms"], 4.);
}

#[test]
fn recording_uses_session_policy_without_a_scenario_selection() {
    use top_down_2d_rts_prototype_nano_swarm::session::SessionRules;
    for enabled in [false, true] {
        let mut app = App::new();
        if enabled {
            app.add_plugins(bevy::time::TimePlugin)
                .insert_resource(top_down_2d_rts_prototype_nano_swarm::fixed_simulation_time())
                .insert_resource(TimeUpdateStrategy::FixedTimesteps(1));
        }
        app.insert_resource(SessionRules {
            scenario_name: "observation",
            record_statistics: enabled,
            ..default()
        })
        .insert_resource(BattleStatisticsConfig::default())
        .add_plugins(BattleStatisticsPlugin);
        for _ in 0..61 {
            app.update();
        }
        let run = app.world().get_resource::<BattleRun>();
        if enabled {
            let summary = &run.expect("recording session has a run").summary;
            assert_eq!(summary.scenario, "observation");
            assert_eq!(summary.samples, 1);
        } else {
            assert!(run.is_none(), "disabled recording must not create a run");
        }
    }
}

#[test]
fn recorded_outcomes_use_actual_swarm_ids_and_session_perspective() {
    use top_down_2d_rts_prototype_nano_swarm::{
        nanobot::{SwarmId, SwarmMember},
        session::{OutcomeMode, SessionRules},
    };
    for (mode, expected) in [
        (OutcomeMode::SwarmRelative, "swarm_42_wins"),
        (OutcomeMode::PlayerRelative, "victory"),
    ] {
        let mut app = battle();
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
        for mut member in app
            .world_mut()
            .query::<&mut SwarmMember>()
            .iter_mut(app.world_mut())
        {
            member.0 = if member.0.0 == 0 {
                SwarmId(7)
            } else {
                SwarmId(42)
            };
        }
        common::destroy_structures(&mut app, |id| id == SwarmId(7));
        app.update();
        let payload = stats(&mut app);
        assert_eq!(payload["summary"]["outcome"], expected);
        let last = payload["samples"].as_array().unwrap().last().unwrap();
        assert!(swarm_row(last, 7)["elimination_seconds"].as_f64().unwrap() > 0.);
        assert!(swarm_row(last, 42)["elimination_seconds"].is_null());
    }
}

/// A scratch balance directory for tests whose tuning edits write the balance file.
struct BalanceDirectory(std::path::PathBuf);

impl Drop for BalanceDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn battle_records_initial_tuning_and_only_its_own_tick_stamped_edits() {
    use top_down_2d_rts_prototype_nano_swarm::tuning::{
        Tuning, TuningJournal, TuningRevision, TuningStore, set_value,
    };
    let directory = BalanceDirectory(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join(".tmp")
            .join(format!("battle-tuning-{}", std::process::id())),
    );
    std::fs::create_dir_all(&directory.0).unwrap();
    let mut app = battle();
    app.init_resource::<Tuning>()
        .init_resource::<TuningJournal>()
        .init_resource::<TuningRevision>()
        .insert_resource(TuningStore::new(directory.0.join("balance.ron")));
    set_value(app.world_mut(), "combat.base_attack", 14.).unwrap();
    app.update();
    for _ in 1..7 {
        app.update();
    }
    set_value(app.world_mut(), "combat.base_attack", 23.).unwrap();
    // Edits join the summary at the next sample.
    advance(&mut app, 53);
    let payload = stats(&mut app);
    assert_eq!(payload["summary"]["tuning"]["combat"]["base_attack"], 14.);
    let edits = payload["summary"]["tuning_edits"].as_array().unwrap();
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

fn latest_stat_columns(app: &mut App, swarm: u64, fields: &[&str]) -> Vec<f64> {
    let row = latest_row(app, swarm);
    fields
        .iter()
        .map(|field| {
            row[field]
                .as_f64()
                .unwrap_or_else(|| panic!("missing {field}"))
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

    let mut app = common::minimal_app();
    app.add_systems(FixedUpdate, worker_gather_extract_system);
    app.world_mut()
        .resource_mut::<Tuning>()
        .logistics
        .material_work_ticks = 24;
    record_battle(&mut app);
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
        latest_stat_columns(&mut app, 0, &fields),
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
    advance(&mut app, 60);
    assert_eq!(
        latest_stat_columns(&mut app, 0, &fields),
        [0.0, 0.0, f64::from(income)],
    );
    assert_eq!(latest_row(&mut app, 0)["minerals_gathered"], income);
    assert_eq!(
        latest_row(&mut app, 0)["deposit_extraction"],
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

    let mut app = common::sim_app_with_population_demand();
    app.insert_resource(IntentGrid::new(32, 32));
    record_battle(&mut app);
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
    let player_sample = latest_stat_columns(&mut app, 0, &fields);
    assert!((player_sample[0] - 1.0).abs() < 1e-6, "{player_sample:?}");
    assert_eq!(player_sample[1], 0.0);
    assert_eq!(latest_stat_columns(&mut app, 1, &fields), [0.0, 1.0]);
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

    let mut app = common::minimal_app();
    record_battle(&mut app);
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
    assert_eq!(latest_stat_columns(&mut app, 0, &columns), [62.0]);

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
    assert_eq!(latest_stat_columns(&mut app, 0, &columns), [22.0]);
}

fn advance(app: &mut App, ticks: usize) {
    for _ in 0..ticks {
        app.update();
    }
}

fn indices(samples: &serde_json::Value) -> Vec<u64> {
    samples
        .as_array()
        .unwrap()
        .iter()
        .map(|sample| sample["swarms"][0]["fixed_tick"].as_u64().unwrap() / 60)
        .collect()
}

#[test]
fn battle_stats_sample_cursors_return_only_newer_samples() {
    let mut app = battle();
    advance(&mut app, 5);

    let empty = stats(&mut app);
    assert_eq!(empty["samples"], serde_json::json!([]));
    assert_eq!(empty["hitches"], serde_json::json!([]));
    assert!(empty["last_sample"].is_null());
    assert!(empty["last_hitch"].is_null());
    assert_eq!(empty["summary"]["samples"], 0);
    assert_eq!(empty["summary"]["status"], "in_progress");
    assert!(empty["summary"]["termination_reason"].is_null());
    assert!(empty["summary"].get("latest").is_none());

    advance(&mut app, 130);
    let all = stats(&mut app);
    assert_eq!(all["samples"].as_array().unwrap().len(), 2);
    assert_eq!(all["last_sample"], 1);
    assert_eq!(all["summary"]["samples"], 2);
    assert_eq!(indices(&all["samples"]), [1, 2]);

    let after_first = query_stats(&mut app, Some(0), None).result.unwrap();
    assert_eq!(
        after_first["samples"],
        serde_json::json!([all["samples"][1]])
    );
    for cursor in [1, 2, 1_000_000] {
        let none = query_stats(&mut app, Some(cursor), None).result.unwrap();
        assert_eq!(none["samples"], serde_json::json!([]));
        assert_eq!(none["last_sample"], 1);
    }

    advance(&mut app, 60);
    let newer = query_stats(&mut app, Some(1), None).result.unwrap();
    assert_eq!(newer["samples"].as_array().unwrap().len(), 1);
    assert_eq!(newer["last_sample"], 2);
    assert_eq!(indices(&newer["samples"]), [3]);
}

#[test]
fn battle_stats_samples_use_typed_json_with_null_for_empty_cells() {
    let mut app = battle();
    advance(&mut app, 60);
    let payload = stats(&mut app);
    let row = swarm_row(&payload["samples"][0], 0);
    assert!(row["swarm"].is_u64());
    assert!(row["simulation_seconds"].is_f64());
    assert!(row["deposit_extraction"].is_array());
    assert!(row["controller_last_explanation"].is_string());
    assert!(row["elimination_seconds"].is_null());
    assert_eq!(
        row["fixed_tick"],
        swarm_row(&payload["samples"][0], 1)["fixed_tick"]
    );
    assert_eq!(payload["summary"]["schema_version"], 9);
}

#[test]
fn battle_stats_hitch_cursors_return_only_newer_hitches() {
    let mut app = battle();
    app.insert_resource(RecorderClock::manual())
        .insert_resource(FrameWork {
            update_ms: 40,
            post_update_ms: 0,
        })
        .add_systems(
            Update,
            |work: Res<FrameWork>, mut clock: ResMut<RecorderClock>| {
                clock.advance(Duration::from_millis(work.update_ms));
            },
        );
    let slow_frame = |app: &mut App, slow: bool| {
        app.world_mut().resource_mut::<FrameWork>().update_ms = if slow { 40 } else { 4 };
        app.update();
    };
    for _ in 0..61 {
        slow_frame(&mut app, false);
    }
    // Each slow frame is its own episode: 30 calm frames close one.
    for _ in 0..3 {
        slow_frame(&mut app, true);
        for _ in 0..40 {
            slow_frame(&mut app, false);
        }
    }
    let all = stats(&mut app);
    let hitches = all["hitches"].as_array().unwrap();
    assert_eq!(hitches.len(), 3);
    assert_eq!(all["last_hitch"], 2);
    assert_eq!(hitches[0]["hitch_count"], 1);
    assert!(hitches[0]["end_frame"].as_u64() < hitches[1]["end_frame"].as_u64());
    assert!(hitches[1]["end_frame"].as_u64() < hitches[2]["end_frame"].as_u64());

    let newer = query_stats(&mut app, None, Some(0)).result.unwrap();
    assert_eq!(
        newer["hitches"],
        serde_json::json!([hitches[1], hitches[2]])
    );
    for cursor in [2, 3] {
        let none = query_stats(&mut app, None, Some(cursor)).result.unwrap();
        assert_eq!(none["hitches"], serde_json::json!([]));
        assert_eq!(none["last_hitch"], 2);
    }
}

#[test]
fn battle_stats_report_status_at_elimination_and_freeze_while_the_world_advances() {
    use top_down_2d_rts_prototype_nano_swarm::nanobot::{Health, SwarmMember};
    let mut app = battle();
    advance(&mut app, 10);
    let running = stats(&mut app);
    assert_eq!(running["summary"]["status"], "in_progress");
    assert!(running["summary"]["termination_reason"].is_null());

    for (owner, mut health) in app
        .world_mut()
        .query::<(&SwarmMember, &mut Health)>()
        .iter_mut(app.world_mut())
    {
        if !owner.0.is_player() {
            health.current = 0;
        }
    }
    common::destroy_structures(&mut app, |id| !id.is_player());
    app.update();
    let terminal = stats(&mut app);
    assert_eq!(terminal["summary"]["status"], "completed");
    assert_eq!(
        terminal["summary"]["termination_reason"],
        "swarm_elimination"
    );
    assert_eq!(terminal["summary"]["outcome"], "swarm_0_wins");
    assert_eq!(
        terminal["last_sample"], 0,
        "final partial sample is recorded"
    );
    assert_eq!(terminal["summary"]["samples"], 1);
    let final_row = swarm_row(&terminal["samples"][0], 1);
    assert_eq!(final_row["fixed_tick"], 11);
    assert_eq!(final_row["population"], 0);
    assert!(final_row["elimination_seconds"].as_f64().unwrap() > 0.);

    let elapsed = app.world().resource::<Time<Fixed>>().elapsed();
    advance(&mut app, 120);
    assert!(
        app.world().resource::<Time<Fixed>>().elapsed() > elapsed,
        "a windowed battle keeps its world running"
    );
    assert_eq!(stats(&mut app), terminal);
}

#[test]
fn battle_stats_report_interruption_at_the_tick_limit_and_freeze() {
    let mut app = common::sim_app_with_elimination();
    record_battle_with(
        &mut app,
        BattleStatisticsConfig {
            max_ticks: std::num::NonZeroU64::new(90),
            ..default()
        },
    );
    let player =
        common::spawn_swarm_with_nanobots(&mut app, Vec2::ZERO, &[(NanobotType::Worker, 1)]);
    common::spawn_home_facility(&mut app, player);
    let opponent = common::spawn_opponent_swarm_with_nanobots(
        &mut app,
        Vec2::X * 500.,
        &[(NanobotType::Defender, 1)],
    );
    common::spawn_home_facility(&mut app, opponent);
    advance(&mut app, 89);
    assert_eq!(stats(&mut app)["summary"]["status"], "in_progress");
    advance(&mut app, 1);
    let terminal = stats(&mut app);
    assert_eq!(terminal["summary"]["status"], "interrupted");
    assert_eq!(terminal["summary"]["termination_reason"], "tick_limit");
    assert!(terminal["summary"]["outcome"].is_null());
    assert_eq!(
        terminal["last_sample"], 1,
        "the partial sample follows tick 60"
    );
    assert_eq!(swarm_row(&terminal["samples"][1], 0)["fixed_tick"], 90);

    let elapsed = app.world().resource::<Time<Fixed>>().elapsed();
    advance(&mut app, 120);
    assert!(
        app.world().resource::<Time<Fixed>>().elapsed() > elapsed,
        "a windowed battle keeps its world running after the tick limit"
    );
    assert_eq!(stats(&mut app), terminal);
}

#[test]
fn non_windowed_battle_freezes_its_simulation_at_the_end_without_exiting() {
    for max_ticks in [Some(90), None] {
        let mut app = common::sim_app_with_elimination();
        record_battle_with(
            &mut app,
            BattleStatisticsConfig {
                headless: true,
                max_ticks: max_ticks.and_then(std::num::NonZeroU64::new),
                ..default()
            },
        );
        let player =
            common::spawn_swarm_with_nanobots(&mut app, Vec2::ZERO, &[(NanobotType::Worker, 1)]);
        common::spawn_home_facility(&mut app, player);
        let opponent = common::spawn_opponent_swarm_with_nanobots(
            &mut app,
            Vec2::X * 500.,
            &[(NanobotType::Defender, 1)],
        );
        common::spawn_home_facility(&mut app, opponent);
        advance(&mut app, 30);
        if max_ticks.is_none() {
            common::destroy_structures(&mut app, |_| true);
        }
        advance(&mut app, 70);
        let terminal = stats(&mut app);
        assert_ne!(terminal["summary"]["status"], "in_progress");
        let frozen_at = app.world().resource::<Time<Fixed>>().elapsed();

        advance(&mut app, 120);

        assert_eq!(app.should_exit(), None, "the process waits for shutdown");
        assert_eq!(app.world().resource::<Time<Fixed>>().elapsed(), frozen_at);
        assert_eq!(stats(&mut app), terminal);
    }
}

#[test]
fn battle_stats_are_unavailable_outside_an_ai_battle_recording() {
    let mut app = common::minimal_app();
    let (handle, channel) = AgentControlCorePlugin::channel(4);
    app.insert_resource(StatsHandle(handle))
        .add_plugins(channel);
    let response = query_stats(&mut app, None, None);
    assert!(!response.ok);
    assert_eq!(response.error.unwrap().code, "battle_unavailable");
}
