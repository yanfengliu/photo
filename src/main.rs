#![windows_subsystem = "windows"]

mod app;
mod collection;
mod decode;
mod detail_load;
mod edit;
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

    iced::application(App::title, App::update, App::view)
        .subscription(App::subscription)
        .theme(App::theme)
        .window_size(Size::new(1200.0, 800.0))
        .antialiasing(true)
        .run_with(App::new)
}
