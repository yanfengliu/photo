//! The app: one [`PhotoApp`] owning the engine and the presentation state.
//!
//! Each frame it polls the engine once, turns the engine's toasts into
//! on-screen ones, takes the keyboard shortcuts out of the input, lays out
//! the shell around the Library's work area and shows the open dialog.
//! Every control that is used queues an action while the frame is drawn;
//! once the frame is laid out each action is handed to the engine, one call
//! per action, and the next frame shows the result.

mod actions;
mod dialogs;
mod dispatch;
mod input;
mod library;
mod shell;
mod state;
mod text;
mod textures;

use std::sync::Arc;
use std::time::Duration;

use photo_engine::Engine;

use self::actions::Action;
use self::state::UiState;
pub use self::state::{Layout, Module, View, Zoom};
use self::textures::Textures;
use crate::AppOptions;
use crate::theme;
use crate::widgets::{PanelGroup, Toasts};

/// The Photo app: the shell and the Library module, driving one engine.
///
/// Holds the engine and only presentation state besides. Every user action
/// becomes one engine call, made after the frame it was asked for in.
pub struct PhotoApp {
    engine: Engine,
    options: AppOptions,
    ui: UiState,
    textures: Textures,
    toasts: Toasts,
    actions: Vec<Action>,
    /// Counts the changes that can make something shown stale: every action
    /// applied and every poll that brought results in.
    generation: u64,
}

/// What a part of the window draws with: the engine, the presentation
/// state, the textures, and the queue its actions go to.
pub(crate) struct Cx<'a> {
    pub(crate) engine: &'a mut Engine,
    pub(crate) ui: &'a mut UiState,
    pub(crate) textures: &'a mut Textures,
    actions: &'a mut Vec<Action>,
    toasts: &'a mut Toasts,
    pub(crate) generation: u64,
}

impl Cx<'_> {
    /// Ask for `action`; it is handed to the engine once the frame is laid
    /// out.
    pub(crate) fn act(&mut self, action: Action) {
        self.actions.push(action);
    }

    /// Tell the user something in a toast.
    pub(crate) fn say(&mut self, message: impl Into<String>) {
        self.toasts.push(message);
    }
}

impl PhotoApp {
    /// The app on an engine that is already open. Installs the theme and
    /// has the engine's workers ask for a frame whenever they finish
    /// something, so background work is shown without a timer.
    pub fn new(cc: &eframe::CreationContext<'_>, mut engine: Engine, options: AppOptions) -> Self {
        theme::apply(&cc.egui_ctx);
        let ctx = cc.egui_ctx.clone();
        engine.set_wake(Arc::new(move || ctx.request_repaint()));
        Self {
            engine,
            options,
            ui: UiState::default(),
            textures: Textures::default(),
            toasts: Toasts::new(),
            actions: Vec::new(),
            generation: 0,
        }
    }

    /// The engine, to read what it holds: the photo list, the selection,
    /// the source and filter, the catalog's panels, running tasks.
    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    /// Where the app keeps its data.
    pub fn options(&self) -> &AppOptions {
        &self.options
    }

    /// The module shown. Only the Library exists so far.
    pub fn module(&self) -> Module {
        Module::Library
    }

    /// The Library view shown in the work area.
    pub fn view(&self) -> View {
        self.ui.view
    }

    /// The loupe's zoom.
    pub fn zoom(&self) -> Zoom {
        self.ui.loupe.zoom
    }

    /// Which edges of the window and the toolbar are shown.
    pub fn layout(&self) -> Layout {
        self.ui.layout
    }

    /// Which panels of the left column are open.
    pub fn left_panels(&self) -> &PanelGroup {
        &self.ui.left_panels
    }

    /// Which panels of the right column are open.
    pub fn right_panels(&self) -> &PanelGroup {
        &self.ui.right_panels
    }

    /// Whether a dialog is up.
    pub fn dialog_open(&self) -> bool {
        self.ui.dialog.is_some()
    }

    /// The messages of the toasts queued or showing, oldest first.
    pub fn toasts(&self) -> impl Iterator<Item = &str> {
        self.toasts.messages()
    }

    /// Wait (blocking, up to `timeout`) until the engine has no background
    /// work left, bringing its results in. For tests and the automation
    /// harness, which then run frames to show what came in. Returns whether
    /// the engine became idle.
    pub fn wait_idle(&mut self, timeout: Duration) -> bool {
        let idle = self.engine.wait_idle(timeout);
        self.generation += 1;
        idle
    }

    fn cx(&mut self) -> Cx<'_> {
        Cx {
            engine: &mut self.engine,
            ui: &mut self.ui,
            textures: &mut self.textures,
            actions: &mut self.actions,
            toasts: &mut self.toasts,
            generation: self.generation,
        }
    }

    /// Hand the queued actions to the engine, in order. A refusal is told
    /// in a toast; the engine has left everything as it was.
    fn apply_actions(&mut self, ctx: &egui::Context) {
        if self.actions.is_empty() {
            return;
        }
        for action in std::mem::take(&mut self.actions) {
            let failure = action.failure();
            if let Err(error) = action.apply(&mut self.engine) {
                self.toasts.push(format!("{failure}: {error}"));
            }
        }
        self.generation += 1;
        ctx.request_repaint();
    }

    /// Notice when the active photo changes, so the grid and filmstrip
    /// scroll to it.
    fn track_active(&mut self) {
        let active = self.engine.active_photo();
        let grid = &mut self.ui.grid;
        if active != grid.last_active {
            grid.last_active = active;
            grid.reveal_active = active.is_some();
        }
    }
}

impl eframe::App for PhotoApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if self.engine.poll() {
            self.generation += 1;
            ctx.request_repaint();
        }
        self.textures.begin_frame(ctx.cumulative_frame_nr());

        // Shortcuts act before anything is drawn, so the frame they are
        // pressed in already shows what they did.
        if self.ui.dialog.is_none() && !ctx.text_edit_focused() {
            let mut cx = self.cx();
            for shortcut in input::take_shortcuts(&ctx) {
                dispatch::shortcut(&mut cx, &ctx, shortcut);
            }
        }
        self.apply_actions(&ctx);
        for toast in self.engine.take_toasts() {
            self.toasts.push(toast.message);
        }
        self.track_active();

        let work_area = {
            let mut cx = self.cx();
            shell::top_bar(ui, &mut cx);
            let work_area = library::show(ui, &mut cx);
            dialogs::show(&ctx, &mut cx);
            work_area
        };
        self.apply_actions(&ctx);
        for toast in self.engine.take_toasts() {
            self.toasts.push(toast.message);
        }
        if self.textures.end_frame() {
            ctx.request_repaint();
        }
        self.toasts.show(&ctx, work_area);
    }
}
