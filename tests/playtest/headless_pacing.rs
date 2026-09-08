use bevy::{
    prelude::*,
    time::{TimePlugin, TimeUpdateStrategy},
};
use top_down_2d_rts_prototype_nano_swarm::{
    runtime::DynamicHeadlessPacingPlugin, scenario_selection::Scenario, session::SessionRules,
};

#[test]
fn headless_pacing_follows_same_process_scenario_changes() {
    let mut app = App::new();
    app.add_plugins((TimePlugin, DynamicHeadlessPacingPlugin));

    app.update();
    assert!(matches!(
        app.world().resource::<TimeUpdateStrategy>(),
        TimeUpdateStrategy::Automatic,
    ));

    app.insert_resource::<SessionRules>(Scenario::AiBattle.definition().rules);
    app.update();
    assert!(matches!(
        app.world().resource::<TimeUpdateStrategy>(),
        TimeUpdateStrategy::FixedTimesteps(1),
    ));

    app.insert_resource::<SessionRules>(Scenario::Sandbox.definition().rules);
    app.update();
    assert!(matches!(
        app.world().resource::<TimeUpdateStrategy>(),
        TimeUpdateStrategy::Automatic,
    ));
}
