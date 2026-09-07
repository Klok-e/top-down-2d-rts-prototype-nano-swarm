use top_down_2d_rts_prototype_nano_swarm::{
    agent_control::{AgentCommand, AgentControlCorePlugin, AgentRequest, RequestId},
    nanobot::{MatchOutcome, Swarm, SwarmEliminationState, SwarmId},
};

#[test]
fn state_get_reports_draw_and_both_eliminated_swarms() {
    use bevy::prelude::*;

    let mut app = App::new();
    app.insert_resource(MatchOutcome::Draw)
        .insert_resource(SwarmEliminationState {
            player_eliminated: true,
            opponent_eliminated: true,
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
