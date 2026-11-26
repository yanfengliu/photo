//! The running app: Photo inside `photo_ui::harness`, rendering real GPU
//! frames offscreen, and the frames, waits and lookups every command
//! shares.

use std::path::Path;
use std::time::{Duration, Instant};

use photo_engine::Engine;
use photo_ui::harness::{Harness, HarnessBuilder, HarnessError};
use photo_ui::{AppOptions, Module, OpenError, Opened, PhotoApp, View};
use serde_json::{Value, json};

use super::controls::{self, Control, role_matches, role_name};
use super::gpu::{self, ChoiceSlot};
use super::observed::Observed;
use super::protocol::{CmdError, Code};
use crate::launch;

/// The window size and pixel density the app runs at.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
    /// The window in logical points.
    pub size: [f32; 2],
    /// Physical pixels per logical point.
    pub pixels_per_point: f32,
}

impl Default for Look {
    /// The size the real window opens at, at one pixel per point.
    fn default() -> Self {
        Self {
            size: [1600.0, 1000.0],
            pixels_per_point: 1.0,
        }
    }
}

/// How many frames an action may take to come to rest.
const MAX_STEPS: u64 = 240;

/// What `wait_idle` reports when the app has come to rest.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rest {
    /// How long it waited.
    pub waited_ms: u64,
}

/// Photo, running headless.
pub struct Driver {
    harness: Harness<Observed>,
    look: Look,
    adapter: ChoiceSlot,
}

impl Driver {
    /// Open the catalog `options` names and start the app on it; with
    /// `open`, open that photo first and start in the loupe, as `photo
    /// PATH` does.
    pub fn start(
        options: AppOptions,
        look: Look,
        open: Option<&Path>,
    ) -> Result<(Self, Option<Opened>), String> {
        let adapter = ChoiceSlot::default();
        let mut engine = options.open_engine().map_err(|error| {
            format!(
                "the catalog {} cannot be opened: {error}",
                options.catalog_path.display()
            )
        })?;
        let opened = match open {
            Some(path) => Some(
                photo_ui::open_in_engine(&mut engine, path, launch::OPEN_TIMEOUT)
                    .map_err(|error| format!("cannot open {}: {error}", path.display()))?,
            ),
            None => None,
        };
        let view = match opened {
            Some(_) => View::Loupe,
            None => View::Grid,
        };
        let harness = HarnessBuilder::new()
            .size(look.size)
            .pixels_per_point(look.pixels_per_point)
            .max_steps(MAX_STEPS)
            .show_pointer(false)
            .wgpu_setup(gpu::hardware_first(adapter.clone()))
            .build(move |cc| {
                let app = PhotoApp::new(cc, engine, options).starting_in(Module::Library, view);
                Observed::new(app)
            });
        let mut driver = Self {
            harness,
            look,
            adapter,
        };
        driver.settle();
        Ok((driver, opened))
    }

    /// Open the photo at `path` in the running app: imported in place if
    /// the catalog does not hold it, made active and shown, in the loupe
    /// from the Library and in Develop when that is the module. The app is
    /// not started again, so its panels, view settings and undo stack stay.
    /// Returns the photo and what settling the frames said.
    pub fn open(&mut self, path: &Path) -> Result<(Opened, Vec<String>), CmdError> {
        let opened = self
            .harness
            .app_mut()
            .app
            .open_path(path, launch::OPEN_TIMEOUT)
            .map_err(open_error)?;
        Ok((opened, self.settle()))
    }

    /// The app.
    pub fn app(&self) -> &PhotoApp {
        &self.harness.app().app
    }

    /// The app's engine.
    pub fn engine(&self) -> &Engine {
        self.app().engine()
    }

    /// The wrapper around the app, with what it saw.
    pub fn observed(&self) -> &Observed {
        self.harness.app()
    }

    /// The headless driver, for gestures.
    pub fn harness(&mut self) -> &mut Harness<Observed> {
        &mut self.harness
    }

    /// The headless driver, for queries.
    pub fn harness_ref(&self) -> &Harness<Observed> {
        &self.harness
    }

    /// The egui context the app runs in.
    pub fn ctx(&self) -> &egui::Context {
        self.harness.ctx()
    }

    /// The window size and density.
    pub fn look(&self) -> Look {
        self.look
    }

    /// What `ping` says about rendering.
    pub fn gpu_json(&self) -> Value {
        let adapter = self
            .adapter
            .lock()
            .map(|choice| choice.as_ref().map(gpu::AdapterChoice::to_json))
            .unwrap_or_default();
        json!({ "available": self.harness.can_render(), "adapter": adapter })
    }

    /// Run a frame if the app asked for one: a worker finished something,
    /// an animation runs, a toast is due to go. Called between requests, so
    /// background work shows up as it would in the window.
    pub fn tick(&mut self) {
        if self
            .harness
            .ctx()
            .has_requested_repaint_for(&egui::ViewportId::ROOT)
        {
            self.harness.step(1);
        }
    }

    /// Run frames until the UI comes to rest, as far as it will.
    pub fn settle(&mut self) -> Vec<String> {
        let result = self.harness.run().map(drop);
        finish(result).unwrap_or_else(|error| vec![error.message])
    }

    /// Every control on screen, from a frame run now.
    pub fn controls(&mut self) -> Vec<Control> {
        self.harness.step(1);
        controls::worth_listing(self.harness.controls())
    }

    /// The one control called `name`, of the role `role` names if given.
    /// When several roles share the name, an interactive control wins over
    /// text; a tie is an error that lists the roles.
    pub fn find(&mut self, name: &str, role: Option<&str>) -> Result<Control, CmdError> {
        let controls = self.controls();
        let mut matches: Vec<&Control> = controls
            .iter()
            .filter(|control| control.name.as_deref() == Some(name))
            .filter(|control| role.is_none_or(|role| role_matches(control.role, role)))
            .collect();
        if matches.len() > 1 && matches.iter().any(|control| control.interactive) {
            matches.retain(|control| control.interactive);
        }
        match matches.as_slice() {
            [one] => Ok((*one).clone()),
            [] => Err(not_found(name, role, &controls)),
            several => {
                let roles: Vec<String> = several
                    .iter()
                    .map(|control| role_name(control.role))
                    .collect();
                Err(CmdError::invalid(format!(
                    "{} controls are named {name:?} ({}); give `role` to say which",
                    several.len(),
                    roles.join(", ")
                )))
            }
        }
    }

    /// Let enough simulated time pass that the next press is not taken as
    /// the second click of the last one.
    pub fn separate_clicks(&mut self) {
        let ctx = self.harness.ctx();
        let since = ctx.input(|input| input.pointer.time_since_last_click());
        let limit = ctx.options(|options| options.input_options.max_double_click_delay) as f32;
        if since <= limit {
            self.harness.advance(limit - since + 2.0 / 60.0);
        }
    }

    /// Run frames until the engine has no work left and the UI has come to
    /// rest, or `timeout` passes. Between frames it blocks on the engine's
    /// results instead of sleeping.
    pub fn wait_idle(&mut self, timeout: Duration) -> Result<Rest, CmdError> {
        let started = Instant::now();
        let deadline = started + timeout;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let engine_idle = self.harness.app_mut().app.wait_idle(left);
            let settled = !matches!(self.harness.run(), Err(HarnessError::NotSettled(_)));
            if engine_idle && settled && self.engine().is_idle() {
                return Ok(Rest {
                    waited_ms: started.elapsed().as_millis() as u64,
                });
            }
            if Instant::now() >= deadline {
                let reason = if self.engine().is_idle() {
                    "the engine is idle but the UI kept changing".to_owned()
                } else {
                    format!("the engine was still busy: {}", self.pending_text())
                };
                return Err(CmdError::new(
                    Code::Timeout,
                    format!("not idle after {} ms; {reason}", timeout.as_millis()),
                ));
            }
        }
    }

    /// The engine's outstanding work, in words.
    pub fn pending_text(&self) -> String {
        let engine = self.engine();
        let pending = engine.pending_work();
        let running: Vec<&str> = engine
            .tasks()
            .iter()
            .filter(|task| !task.state.is_over())
            .map(|task| task.label.as_str())
            .collect();
        let mut text = format!(
            "{} jobs ({} queued), {} previews, {} tasks",
            pending.jobs, pending.queued, pending.previews, pending.tasks
        );
        if !running.is_empty() {
            text.push_str(&format!(" ({})", running.join("; ")));
        }
        text
    }
}

/// What an action's settling said: nothing, or warnings about a UI that
/// was still changing or has unnamed controls (the action was carried out
/// either way); a control that could not be found or used is an error.
pub fn finish(result: Result<(), HarnessError>) -> Result<Vec<String>, CmdError> {
    match result {
        Ok(()) => Ok(Vec::new()),
        Err(HarnessError::UnnamedControls(controls)) => Ok(vec![format!(
            "{} interactive control(s) have no accessible name: {}",
            controls.len(),
            controls.join("; ")
        )]),
        Err(HarnessError::NotSettled(detail)) => Ok(vec![format!(
            "the UI was still changing when the action ended ({detail}); wait_idle waits for it"
        )]),
        Err(error @ HarnessError::NotFound { .. }) => Err(CmdError::not_found(error.to_string())),
        Err(error @ HarnessError::Ambiguous { .. }) => Err(CmdError::invalid(error.to_string())),
        Err(error @ HarnessError::NoBounds { .. }) => Err(CmdError::unavailable(error.to_string())),
    }
}

/// `not_found` for `name`, suggesting controls whose names contain it.
fn not_found(name: &str, role: Option<&str>, controls: &[Control]) -> CmdError {
    let wanted = name.to_lowercase();
    let near: Vec<String> = controls
        .iter()
        .filter_map(|control| {
            let found = control.name.as_deref()?;
            found
                .to_lowercase()
                .contains(&wanted)
                .then(|| format!("{} {found:?}", role_name(control.role)))
        })
        .take(8)
        .collect();
    let what = match role {
        Some(role) => format!("no {role} named {name:?} is on screen"),
        None => format!("no control named {name:?} is on screen"),
    };
    if near.is_empty() {
        CmdError::not_found(what)
    } else {
        CmdError::not_found(format!("{what}; similar: {}", near.join(", ")))
    }
}

/// A photo that could not be opened, as a request's error.
pub fn open_error(error: OpenError) -> CmdError {
    match error {
        OpenError::Missing(_) => CmdError::not_found(error.to_string()),
        OpenError::Unsupported(_) => CmdError::invalid(error.to_string()),
        OpenError::Timeout(_) => CmdError::new(Code::Timeout, error.to_string()),
        OpenError::Failed(_) => CmdError::new(Code::Io, error.to_string()),
    }
}
