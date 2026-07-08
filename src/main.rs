#![windows_subsystem = "windows"]

mod app;
mod collection;
mod decode;
mod detail_load;
mod edit;
mod harness;
mod launch;
mod lens;
mod library;
mod loading;
mod local_edits;
mod nav;
mod repo;
mod session_cache;
mod theme;
mod viewer;
mod widgets;

use app::App;
use iced::Size;

fn main() -> iced::Result {
    env_logger::init();

    let args: Vec<String> = std::env::args().skip(1).collect();
    let options = launch::parse_cli_args(&args);
    if let Some(harness_launch) = &options.harness {
        // A failed harness setup degrades to a normal app launch: the control
        // channel is a dev tool, never a reason to refuse the user their app.
        if let Err(e) = harness::prepare_runtime(harness_launch) {
            log::error!("harness setup failed, continuing without harness: {e}");
        }
    }
    launch::set_options(options);

    iced::application(App::title, App::update, App::view)
        .subscription(App::subscription)
        .theme(App::theme)
        .window_size(Size::new(1200.0, 800.0))
        .antialiasing(true)
        .run_with(App::new)
}
