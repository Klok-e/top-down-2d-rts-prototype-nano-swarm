use bevy::prelude::*;
use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};
use top_down_2d_rts_prototype_nano_swarm::{
    agent_control::{
        AgentCommand, AgentControlCorePlugin, AgentControlHandle, AgentRequest, RequestId,
    },
    nanobot::{MatchOutcome, SwarmId},
    tuning::{Tuning, TuningJournal, TuningRevision, TuningStore},
    ui::{scenario_menu::ScenarioMenu, tuning_panel::TuningEditor},
};

struct ConfigDirectory(PathBuf);
impl ConfigDirectory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join(".tmp")
            .join(format!(
                "agent-tuning-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}
impl Drop for ConfigDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn command(
    app: &mut App,
    control: &AgentControlHandle,
    command: AgentCommand,
) -> top_down_2d_rts_prototype_nano_swarm::agent_control::AgentResponse {
    let response = control
        .submit(AgentRequest {
            id: RequestId::Number(1),
            command,
        })
        .unwrap();
    app.update();
    response
        .try_recv()
        .expect("immediate command responds in one update")
}

#[test]
fn agent_tuning_edits_persist_and_report_unsaved_changes_without_mutating_the_match() {
    let directory = ConfigDirectory::new();
    let path = directory.0.join("balance.ron");
    let (control, plugin) = AgentControlCorePlugin::channel(4);
    let mut app = App::new();
    app.add_plugins(plugin)
        .init_resource::<Tuning>()
        .init_resource::<TuningJournal>()
        .init_resource::<TuningRevision>()
        .init_resource::<TuningEditor>()
        .insert_resource(TuningStore::new(&path))
        .insert_resource(MatchOutcome::Winner(SwarmId::PLAYER));
    let mut menu = ScenarioMenu::default();
    menu.open = true;
    menu.blocks_world_input = true;
    app.insert_resource(menu);
    let response = command(
        &mut app,
        &control,
        AgentCommand::TuningSet {
            key: "combat.base_attack".into(),
            value: 27.,
        },
    );
    assert!(response.ok, "{response:?}");
    assert_eq!(response.result.unwrap()["revision"], 1);
    assert_eq!(TuningStore::load(&path).unwrap().1.combat.base_attack, 27.);
    assert_eq!(app.world().resource::<Tuning>().combat.base_attack, 27.);
    assert_eq!(
        *app.world().resource::<MatchOutcome>(),
        MatchOutcome::Winner(SwarmId::PLAYER)
    );
    let saved = std::fs::read(&path).unwrap();
    app.insert_resource(TuningStore::new(directory.0.join("missing/balance.ron")));
    let response = command(
        &mut app,
        &control,
        AgentCommand::TuningSet {
            key: "combat.base_attack".into(),
            value: 39.,
        },
    );
    assert!(!response.ok);
    assert_eq!(response.error.unwrap().code, "invalid_tuning");
    assert_eq!(std::fs::read(&path).unwrap(), saved);
    assert_eq!(app.world().resource::<Tuning>().combat.base_attack, 27.);
    assert_eq!(app.world().resource::<TuningJournal>().entries.len(), 1);
    assert_eq!(app.world().resource::<TuningRevision>().revision, 1);
    let toggled = command(
        &mut app,
        &control,
        AgentCommand::TuningToggle {
            group: Some("combat".into()),
        },
    );
    assert_eq!(toggled.result.unwrap()["open"], true);
    let state = command(&mut app, &control, AgentCommand::TuningGet)
        .result
        .unwrap();
    assert_eq!(state["revision"], 1);
    assert_eq!(state["editor"]["open"], true);
    assert_eq!(state["editor"]["expanded"], serde_json::json!(["combat"]));
    let attack = state["fields"]
        .as_array()
        .unwrap()
        .iter()
        .find(|field| field["key"] == "combat.base_attack")
        .unwrap();
    assert_eq!(attack["value"], 27.);
    assert_eq!(attack["group"], "combat");
}
