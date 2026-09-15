use bevy::{
    prelude::*,
    time::{TimePlugin, TimeUpdateStrategy},
};
use top_down_2d_rts_prototype_nano_swarm::runtime::{HeadlessPacing, HeadlessRunnerPlugin};

#[test]
fn headless_pacing_follows_the_startup_option_not_the_active_scenario() {
    for unbounded in [false, true] {
        let mut app = App::new();
        app.insert_resource(HeadlessPacing { unbounded })
            .add_plugins((TimePlugin, HeadlessRunnerPlugin));

        app.update();
        if unbounded {
            assert!(matches!(
                app.world().resource::<TimeUpdateStrategy>(),
                TimeUpdateStrategy::FixedTimesteps(1),
            ));
        } else {
            assert!(matches!(
                app.world().resource::<TimeUpdateStrategy>(),
                TimeUpdateStrategy::Automatic,
            ));
        }
    }
}
