//! The app: one [`PhotoApp`] owning the engine and the presentation state.
//!
//! Each frame it polls the engine once, turns the engine's toasts into
//! on-screen ones, takes the keyboard shortcuts out of the input, lays out
//! the shell around the module shown (Library or Develop) and shows the
//! open dialog. Every control that is used queues an action while the frame
//! is drawn; once the frame is laid out each action is handed to the
//! engine, one call per action, and the next frame shows the result.

mod actions;
pub(crate) mod develop;
mod dialogs;
mod dispatch;
mod input;
mod library;
mod open;
mod reveal;
mod shell;
mod state;
mod text;
mod textures;

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use photo_core::geom::Point;
use photo_engine::Engine;

use self::actions::Action;
use self::develop::drafts::Pending;
pub use self::develop::{
    BeforeAfter, CanvasImage, CanvasShows, CanvasZoom, Lights, MaskKind, Tool,
};
use self::develop::{DevelopState, Frames};
pub use self::open::{OpenError, Opened};
use self::state::UiState;
pub use self::state::{Layout, Module, View, Zoom};
use self::textures::Textures;
use crate::AppOptions;
use crate::theme;
use crate::widgets::{ClippingToggles, PanelGroup, Toasts};

/// The Photo app: the shell around the Library and Develop modules, driving
/// one engine.
///
/// Holds the engine and only presentation state besides. Every user action
/// becomes one engine call, made after the frame it was asked for in.
pub struct PhotoApp {
    engine: Engine,
    options: AppOptions,
    ui: UiState,
    textures: Textures,
    frames: Frames,
    toasts: Toasts,
    actions: Vec<Action>,
    /// Counts the changes that can make something shown stale: every action
    /// applied and every poll that brought results in.
    generation: u64,
    /// Whether the engine's background results are held back (see
    /// [`PhotoApp::hold_results`]).
    holding: bool,
    /// How a folder is shown to the user (see [`PhotoApp::set_reveal`]).
    reveal: Box<dyn Fn(&Path)>,
}

/// What a part of the window draws with: the engine, the presentation
/// state, the textures, and the queue its actions go to.
pub(crate) struct Cx<'a> {
    pub(crate) engine: &'a mut Engine,
    pub(crate) ui: &'a mut UiState,
    pub(crate) textures: &'a mut Textures,
    pub(crate) frames: &'a mut Frames,
    actions: &'a mut Vec<Action>,
    toasts: &'a mut Toasts,
    pub(crate) generation: u64,
    /// Shows a folder to the user, in the system's file manager.
    pub(crate) reveal: &'a dyn Fn(&Path),
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
        let ui = UiState {
            develop: DevelopState::with_stored_panels(&engine),
            ..UiState::default()
        };
        Self {
            engine,
            options,
            ui,
            textures: Textures::default(),
            frames: Frames::default(),
            toasts: Toasts::new(),
            actions: Vec::new(),
            generation: 0,
            holding: false,
            reveal: Box::new(reveal::in_file_manager),
        }
    }

    /// Start in `module` showing `view`, for an app opened on a photo:
    /// call before the first frame. Develop opens the active photo, or the
    /// first one.
    pub fn starting_in(mut self, module: Module, view: View) -> Self {
        self.ui.view = view;
        let wants_photo = module == Module::Develop || view != View::Grid;
        if wants_photo && self.engine.active_photo().is_none() {
            self.engine
                .move_active(photo_engine::Movement::First, false);
        }
        // Develop has nothing to open in an empty catalog.
        if module == Module::Library || self.engine.photo_count() > 0 {
            self.ui.module = module;
        }
        self
    }

    /// Replace how a folder is shown to the user (by default, in the
    /// system's file manager): the automation harness and tests record the
    /// folder instead of opening a window.
    pub fn set_reveal(&mut self, reveal: impl Fn(&Path) + 'static) {
        self.reveal = Box::new(reveal);
    }

    /// The engine, to read what it holds: the photo list, the selection,
    /// the source and filter, the catalog's panels, running tasks, the
    /// develop session.
    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    /// Where the app keeps its data.
    pub fn options(&self) -> &AppOptions {
        &self.options
    }

    /// The module shown.
    pub fn module(&self) -> Module {
        self.ui.module
    }

    /// The Library view shown in the work area.
    pub fn view(&self) -> View {
        self.ui.view
    }

    /// The loupe's zoom.
    pub fn zoom(&self) -> Zoom {
        self.ui.loupe.zoom
    }

    /// The Develop canvas's zoom.
    pub fn develop_zoom(&self) -> CanvasZoom {
        self.ui.develop.canvas.zoom
    }

    /// The point of the photo at the middle of the Develop canvas, as
    /// fractions of its width and height.
    pub fn develop_center(&self) -> egui::Vec2 {
        self.ui.develop.canvas.center
    }

    /// How the Develop canvas shows the before view.
    pub fn before_after(&self) -> BeforeAfter {
        self.ui.develop.before_after
    }

    /// Which clipping overlays the Develop canvas shows.
    pub fn clipping(&self) -> ClippingToggles {
        self.ui.develop.clipping
    }

    /// Whether the white-balance picker waits for a click on the photo.
    pub fn picking_white_balance(&self) -> bool {
        self.ui.develop.picker
    }

    /// How much of the window around the photo is darkened in Develop.
    pub fn lights(&self) -> Lights {
        self.ui.develop.lights
    }

    /// What the Develop canvas showed in the last frame drawn.
    pub fn canvas_shows(&self) -> CanvasShows {
        self.frames.shows
    }

    /// The Develop tool that is open.
    pub fn tool(&self) -> Option<Tool> {
        self.ui.develop.tools.open
    }

    /// Where the crop rectangle is on the Develop canvas, in points, while
    /// the crop tool is open, as of the last frame drawn.
    pub fn crop_rect(&self) -> Option<egui::Rect> {
        self.ui.develop.tools.crop_rect
    }

    /// The healing spot selected in the healing tool: its place in the
    /// settings' list.
    pub fn selected_spot(&self) -> Option<usize> {
        self.ui.develop.tools.heal.selected
    }

    /// The mask selected in the masking tool: its place in the settings'
    /// list.
    pub fn selected_mask(&self) -> Option<usize> {
        self.ui.develop.tools.mask.selected
    }

    /// Whether the masking tool's red overlay is switched on.
    pub fn mask_overlay(&self) -> bool {
        self.ui.develop.tools.mask.overlay
    }

    /// Where a point of the open photo's source (fractions of its width
    /// and height, as spots and masks are stored) is on the Develop canvas,
    /// in points, as of the last frame drawn. `None` until the photo is
    /// decoded.
    pub fn canvas_position(&self, source: Point) -> Option<egui::Pos2> {
        self.ui.develop.map.as_ref()?.to_screen(source)
    }

    /// The point of the open photo's source (fractions of its width and
    /// height) under a position of the Develop canvas, in points, as of the
    /// last frame drawn.
    pub fn canvas_source(&self, position: egui::Pos2) -> Option<Point> {
        Some(self.ui.develop.map.as_ref()?.to_normalized(position))
    }

    /// Where the whole photo is on the Develop canvas (in points, part of
    /// it maybe out of view), as of the last frame drawn: a point of the
    /// photo at fractions `(u, v)` of its size is at
    /// `rect.min + (u * width, v * height)`.
    pub fn canvas_photo_rect(&self) -> Option<egui::Rect> {
        self.ui.develop.photo_rect
    }

    /// The develop version and origin of the preview the screen holds for
    /// `photo` at `size` (a grid or filmstrip cell, the loupe), if any.
    pub fn preview_shown(
        &self,
        photo: photo_core::meta::PhotoId,
        size: photo_engine::PreviewSize,
    ) -> Option<(u64, photo_engine::PreviewOrigin)> {
        self.textures.shown(photo, size)
    }

    /// Which edges of the window and the toolbar are shown.
    pub fn layout(&self) -> Layout {
        self.ui.layout
    }

    /// Which panels of the left column are open (the module's own).
    pub fn left_panels(&self) -> &PanelGroup {
        match self.ui.module {
            Module::Library => &self.ui.left_panels,
            Module::Develop => &self.ui.develop.left_panels,
        }
    }

    /// Which panels of the right column are open (the module's own).
    pub fn right_panels(&self) -> &PanelGroup {
        match self.ui.module {
            Module::Library => &self.ui.right_panels,
            Module::Develop => &self.ui.develop.right_panels,
        }
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

    /// Hold back (or let in again) what the engine's background work
    /// finishes. While held, frames do not poll the engine: the workers go
    /// on, but none of their results reaches the catalog or the screen, so
    /// no task can progress or end; actions such as Cancel still go to the
    /// engine at once. For tests and the automation harness, to look at and
    /// act on work in progress at a moment of their choosing.
    /// [`Self::wait_idle`] brings results in whether held or not.
    pub fn hold_results(&mut self, hold: bool) {
        self.holding = hold;
    }

    fn cx(&mut self) -> Cx<'_> {
        Cx {
            engine: &mut self.engine,
            ui: &mut self.ui,
            textures: &mut self.textures,
            frames: &mut self.frames,
            actions: &mut self.actions,
            toasts: &mut self.toasts,
            generation: self.generation,
            reveal: &*self.reveal,
        }
    }

    /// Hand the queued actions to the engine, in order. A refusal is told
    /// in a toast; the engine has left everything as it was.
    fn apply_actions(&mut self, ctx: &egui::Context) {
        if self.actions.is_empty() {
            return;
        }
        for action in std::mem::take(&mut self.actions) {
            if action.edits_outside_tools() {
                self.record_tool_step();
            }
            let failure = action.failure();
            if let Err(error) = action.apply(&mut self.engine) {
                self.toasts.push(format!("{failure}: {error}"));
            }
            // The layout of every version an action makes is kept, so a
            // frame rendered for it can be placed.
            if let Some(session) = self.engine.develop() {
                self.frames.layouts.note(session);
            }
        }
        self.generation += 1;
        ctx.request_repaint();
    }

    /// Before a change from outside the tools: record what the open tool
    /// has changed as a step of its own, a draft of it that still waits for
    /// its turn included.
    fn record_tool_step(&mut self) {
        let develop = &mut self.ui.develop;
        let waiting = develop.pending.as_ref().is_some_and(Pending::is_tool);
        if !waiting && !develop.tools.dirty {
            return;
        }
        develop.tools.dirty = false;
        let draft = develop.pending.take_if(|pending| pending.is_tool());
        let sent = match draft {
            Some(draft) => Action::Develop(draft.into_action()).apply(&mut self.engine),
            None => Ok(()),
        };
        if let Err(error) = sent.and_then(|()| self.engine.commit_edit().map(drop)) {
            self.toasts
                .push(format!("Could not change the photo: {error}"));
        }
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
        if !self.holding && self.engine.poll() {
            self.generation += 1;
            ctx.request_repaint();
        }
        self.textures.begin_frame(ctx.cumulative_frame_nr());

        // Shortcuts act before anything is drawn, so the frame they are
        // pressed in already shows what they did.
        if self.ui.dialog.is_none() && !ctx.text_edit_focused() {
            let mut cx = self.cx();
            let scope = input::Scope {
                module: cx.ui.module,
                tool: cx.ui.develop.tools.open,
            };
            for shortcut in input::take_shortcuts(&ctx, scope) {
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
            let work_area = match cx.ui.module {
                Module::Library => library::show(ui, &mut cx),
                Module::Develop => develop::show(ui, &mut cx),
            };
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
