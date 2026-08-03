use bevy::app::AppExit;
use top_down_2d_rts_prototype_nano_swarm::runtime::{
    RUNTIME_HELP, RuntimeCommand, build_runtime_app,
};

fn main() -> AppExit {
    match RuntimeCommand::parse(std::env::args().skip(1)) {
        Ok(RuntimeCommand::Run(options)) => match build_runtime_app(options) {
            Ok(mut app) => app.run(),
            Err(error) => {
                eprintln!("error: {error}");
                AppExit::from_code(2)
            }
        },
        Ok(RuntimeCommand::Help) => {
            print!("{RUNTIME_HELP}");
            AppExit::Success
        }
        Err(error) => {
            eprintln!("error: {error}\n\n{RUNTIME_HELP}");
            AppExit::from_code(2)
        }
    }
}
