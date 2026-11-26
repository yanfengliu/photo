//! A headless driver for any [`eframe::App`]: run it without a window, feed
//! it keys and pointer gestures, find controls through the accessibility tree
//! by role and name, and render what it would show to an image.
//!
//! UI tests use this directly, and it is the engine room of the planned
//! `photo harness` subcommand, so nothing here panics on bad input: a control
//! that cannot be found, a UI that never settles and a machine without a GPU
//! are all reported as errors.
//!
//! Every action settles the UI before it returns (it runs frames until no
//! repaint is requested), so the next query sees the result. Time is
//! simulated: it passes only as frames run, one step per frame.

mod builder;
mod controls;
mod error;
mod gpu;
mod host;

use std::path::Path;

use egui::accesskit::{self, Role};
use egui::{Event, Key, Modifiers, PointerButton, Pos2, Rect, Vec2};
use egui_kittest::kittest::{NodeT as _, Queryable as _};

pub use builder::HarnessBuilder;
pub use controls::ListedControl;
pub use error::{HarnessError, RenderError};
pub use host::Host;

/// What the accessibility tree says about one control.
#[derive(Clone, Debug, PartialEq)]
pub struct Control {
    /// Where it is, in logical points.
    pub rect: Rect,
    /// Its number, for sliders and the like.
    pub value: Option<f64>,
    /// The least number it takes, for sliders and the like.
    pub min: Option<f64>,
    /// The greatest number it takes, for sliders and the like.
    pub max: Option<f64>,
    /// Its text: the content of a text field, or a widget's own description
    /// of its state.
    pub text: Option<String>,
    /// Whether it is on, for switches and toggle buttons.
    pub toggled: Option<bool>,
    /// Whether it is selected, for items of a list or grid.
    pub selected: Option<bool>,
    /// Whether it is expanded, for disclosure headers.
    pub expanded: Option<bool>,
    /// Whether it has keyboard focus.
    pub focused: bool,
    /// Whether it is disabled.
    pub disabled: bool,
}

/// The longest stretch of simulated time [`Harness::advance`] passes at once.
const MAX_ADVANCE: f32 = 60.0;

/// A running, windowless app. See the [module documentation](self).
pub struct Harness<A> {
    inner: egui_kittest::Harness<'static, A>,
    /// The longest side, in pixels, the GPU can render; `Err` holds the
    /// reason there is nothing to render with.
    gpu: Result<u32, String>,
    /// Simulated seconds per frame.
    step_dt: f32,
    /// Whether [`Self::run`] reports controls without an accessible name.
    check_names: bool,
    /// Where the last click egui counted was made, so the next one can be
    /// kept from merging with it into a double-click. (When it was, egui
    /// tells; where, it does not.)
    last_click_pos: Option<Pos2>,
}

impl<A: eframe::App + 'static> Harness<A> {
    /// A harness of `size` logical points at `pixels_per_point`, with default
    /// options otherwise. See [`HarnessBuilder`] for the rest.
    pub fn new(
        size: impl Into<Vec2>,
        pixels_per_point: f32,
        create: impl FnOnce(&eframe::CreationContext<'_>) -> A,
    ) -> Self {
        HarnessBuilder::new()
            .size(size)
            .pixels_per_point(pixels_per_point)
            .build(create)
    }

    /// The app being driven.
    pub fn app(&self) -> &A {
        self.inner.state()
    }

    /// The app being driven, mutably (to arrange state before an action).
    pub fn app_mut(&mut self) -> &mut A {
        self.inner.state_mut()
    }

    /// The egui context the app runs in.
    pub fn ctx(&self) -> &egui::Context {
        &self.inner.ctx
    }

    /// Run frames until the UI stops asking for repaints, and check that
    /// every interactive control has an accessible name (unless the harness
    /// was built with [`HarnessBuilder::check_names`] off). Returns the
    /// number of frames run.
    ///
    /// Every action ends with this, so on an app with an unnamed control an
    /// action returns [`HarnessError::UnnamedControls`] although it was
    /// carried out.
    pub fn run(&mut self) -> Result<u64, HarnessError> {
        let steps = self
            .inner
            .try_run()
            .map_err(|err| HarnessError::NotSettled(err.to_string()))?;
        let unnamed = if self.check_names {
            self.inner.unnamed_widgets()
        } else {
            Vec::new()
        };
        if unnamed.is_empty() {
            Ok(steps)
        } else {
            Err(HarnessError::UnnamedControls(unnamed))
        }
    }

    /// Run exactly `frames` frames, settled or not.
    pub fn step(&mut self, frames: usize) {
        self.inner.run_steps(frames);
    }

    /// Let `seconds` of simulated time pass (for tooltips, toasts, timers):
    /// at least one frame, and at most a minute at a time.
    pub fn advance(&mut self, seconds: f32) {
        let seconds = if seconds.is_nan() {
            0.0
        } else {
            seconds.clamp(0.0, MAX_ADVANCE)
        };
        let frames = (seconds / self.step_dt).ceil().max(1.0);
        self.step(frames as usize);
    }

    /// Run frames until `done` says so, for work that finishes on its own
    /// time (a background job, an engine going idle). Gives up when `done`
    /// is still false after `max_frames` frames. Returns the frames run.
    pub fn run_until(
        &mut self,
        max_frames: u64,
        mut done: impl FnMut(&A) -> bool,
    ) -> Result<u64, HarnessError> {
        for frame in 0..=max_frames {
            if done(self.inner.state()) {
                return Ok(frame);
            }
            if frame < max_frames {
                self.inner.step();
            }
        }
        Err(HarnessError::NotSettled(format!(
            "the condition was still false after {max_frames} frames"
        )))
    }

    // ---- Keyboard ----

    /// Press and release `key` while `modifiers` are held.
    pub fn press_key(&mut self, key: Key, modifiers: Modifiers) -> Result<(), HarnessError> {
        self.inner.key_press_modifiers(modifiers, key);
        self.run().map(drop)
    }

    /// Type `text` into whatever has keyboard focus.
    pub fn type_text(&mut self, text: &str) -> Result<(), HarnessError> {
        self.inner.event(Event::Text(text.to_owned()));
        self.run().map(drop)
    }

    // ---- Pointer ----

    /// Move the pointer to `pos` without pressing anything.
    pub fn hover_at(&mut self, pos: Pos2) -> Result<(), HarnessError> {
        self.inner.event(Event::PointerMoved(pos));
        self.run().map(drop)
    }

    /// Take the pointer off the window, so nothing is hovered.
    pub fn pointer_leave(&mut self) -> Result<(), HarnessError> {
        self.inner.event(Event::PointerGone);
        self.run().map(drop)
    }

    /// Click the primary button at `pos`.
    ///
    /// Every click made through the harness is a click of its own: if the
    /// one before was at the same spot a moment ago, enough simulated time
    /// is let pass first that the two do not merge into a double-click. Use
    /// [`Self::double_click_at`] for that.
    pub fn click_at(&mut self, pos: Pos2) -> Result<(), HarnessError> {
        self.click_at_with(pos, PointerButton::Primary, Modifiers::NONE)
    }

    /// Double-click the primary button at `pos`.
    pub fn double_click_at(&mut self, pos: Pos2) -> Result<(), HarnessError> {
        // egui takes two quick clicks after a third as a triple-click, even
        // if the earlier one was somewhere else.
        self.separate_from_last_click(None);
        let started = self.time();
        self.queue_click(pos, PointerButton::Primary, Modifiers::NONE);
        self.queue_click(pos, PointerButton::Primary, Modifiers::NONE);
        self.settle_release(pos, started)
    }

    /// Click the secondary (context menu) button at `pos`.
    pub fn secondary_click_at(&mut self, pos: Pos2) -> Result<(), HarnessError> {
        self.click_at_with(pos, PointerButton::Secondary, Modifiers::NONE)
    }

    /// Click `button` at `pos` while `modifiers` are held.
    pub fn click_at_with(
        &mut self,
        pos: Pos2,
        button: PointerButton,
        modifiers: Modifiers,
    ) -> Result<(), HarnessError> {
        self.separate_from_last_click(Some(pos));
        let started = self.time();
        self.queue_click(pos, button, modifiers);
        self.settle_release(pos, started)
    }

    /// Press the primary button at `from`, move to `to` in a few steps and
    /// release there. The button is released even if a step on the way
    /// reports an error; the first error is the one returned.
    pub fn drag(&mut self, from: Pos2, to: Pos2) -> Result<(), HarnessError> {
        self.drag_with(from, to, Modifiers::NONE)
    }

    /// [`Self::drag`] with `modifiers` held from the press to the release
    /// (Alt to erase with a brush, say).
    pub fn drag_with(
        &mut self,
        from: Pos2,
        to: Pos2,
        modifiers: Modifiers,
    ) -> Result<(), HarnessError> {
        const STEPS: usize = 5;
        self.inner.event(Event::PointerMoved(from));
        if modifiers.any() {
            self.inner.event(Event::ModifiersChanged(modifiers));
        }
        self.inner.event(pointer_button(
            from,
            PointerButton::Primary,
            true,
            modifiers,
        ));
        let mut result = self.run().map(drop);
        for step in 1..=STEPS {
            let moved = self.pointer_move(from.lerp(to, step as f32 / STEPS as f32));
            result = result.and(moved);
        }
        let started = self.time();
        self.inner
            .event(pointer_button(to, PointerButton::Primary, false, modifiers));
        if modifiers.any() {
            self.inner.event(Event::ModifiersChanged(Modifiers::NONE));
        }
        let released = self.settle_release(to, started);
        result.and(released)
    }

    /// Press the primary button at `pos` and hold it. Follow with
    /// [`Self::pointer_move`] and [`Self::pointer_up`] to script a gesture
    /// frame by frame.
    pub fn pointer_down(&mut self, pos: Pos2) -> Result<(), HarnessError> {
        self.inner.event(Event::PointerMoved(pos));
        self.inner.event(pointer_button(
            pos,
            PointerButton::Primary,
            true,
            Modifiers::NONE,
        ));
        self.run().map(drop)
    }

    /// Move the pointer to `pos`, keeping any pressed button down.
    pub fn pointer_move(&mut self, pos: Pos2) -> Result<(), HarnessError> {
        self.hover_at(pos)
    }

    /// Release the primary button at `pos`.
    pub fn pointer_up(&mut self, pos: Pos2) -> Result<(), HarnessError> {
        let started = self.time();
        self.inner.event(pointer_button(
            pos,
            PointerButton::Primary,
            false,
            Modifiers::NONE,
        ));
        // A press and a release on one spot is a click like any other.
        self.settle_release(pos, started)
    }

    /// The simulated time, in seconds.
    fn time(&self) -> f64 {
        self.inner.ctx.input(|input| input.time)
    }

    /// If the last click was a moment ago, let enough simulated time pass
    /// that the next one does not count as its double-click. With `near`,
    /// only a last click on that spot matters.
    fn separate_from_last_click(&mut self, near: Option<Pos2>) {
        let ctx = &self.inner.ctx;
        let (delay, reach) = ctx.options(|options| {
            let input = &options.input_options;
            (input.max_double_click_delay, input.max_click_dist)
        });
        let since = f64::from(ctx.input(|input| input.pointer.time_since_last_click()));
        let close = match (near, self.last_click_pos) {
            (Some(pos), Some(last)) => last.distance(pos) <= reach,
            // Anywhere counts, or it is not known where the last click was.
            _ => true,
        };
        if since <= delay && close {
            self.advance((delay - since) as f32 + self.step_dt);
        }
    }

    /// Settle the UI after a button was released at `pos` in an action begun
    /// at `started`. If egui counted the release as a click (it does not at
    /// the end of a drag), note where the click was.
    fn settle_release(&mut self, pos: Pos2, started: f64) -> Result<(), HarnessError> {
        let result = self.run().map(drop);
        let (now, since) = self.inner.ctx.input(|input| {
            let since = input.pointer.time_since_last_click();
            (input.time, f64::from(since))
        });
        // A click counted during the action is younger than the action, by
        // a frame at least; an older click is not.
        if since < now - started - 0.5 * f64::from(self.step_dt) {
            self.last_click_pos = Some(pos);
        }
        result
    }

    fn queue_click(&self, pos: Pos2, button: PointerButton, modifiers: Modifiers) {
        self.inner.event(Event::PointerMoved(pos));
        if modifiers.any() {
            self.inner.event(Event::ModifiersChanged(modifiers));
        }
        self.inner
            .event(pointer_button(pos, button, true, modifiers));
        self.inner
            .event(pointer_button(pos, button, false, modifiers));
        if modifiers.any() {
            self.inner.event(Event::ModifiersChanged(Modifiers::NONE));
        }
    }

    // ---- Controls, by role and accessible name ----

    /// Describe the one control with this role and name.
    pub fn control(&self, role: Role, name: &str) -> Result<Control, HarnessError> {
        let node = self.node(role, name)?;
        let access = node.accesskit_node();
        Ok(Control {
            rect: self.rect_of(&node, role, name)?,
            value: access.numeric_value(),
            min: access.min_numeric_value(),
            max: access.max_numeric_value(),
            text: access.value(),
            toggled: access
                .toggled()
                .map(|toggled| toggled == accesskit::Toggled::True),
            selected: access.is_selected(),
            expanded: access.data().is_expanded(),
            focused: access.is_focused(),
            disabled: access.is_disabled(),
        })
    }

    /// How many controls have this role and name (0 when it is not shown).
    pub fn count(&self, role: Role, name: &str) -> usize {
        self.inner.query_all_by_role_and_label(role, name).count()
    }

    /// Click the centre of a control with the pointer.
    pub fn click(&mut self, role: Role, name: &str) -> Result<(), HarnessError> {
        let center = self.control(role, name)?.rect.center();
        self.click_at(center)
    }

    /// Double-click the centre of a control with the pointer.
    pub fn double_click(&mut self, role: Role, name: &str) -> Result<(), HarnessError> {
        let center = self.control(role, name)?.rect.center();
        self.double_click_at(center)
    }

    /// Activate a control through the accessibility tree, the way assistive
    /// technology does. Unlike [`Self::click`] this works when the control is
    /// scrolled out of view or covered.
    pub fn invoke(&mut self, role: Role, name: &str) -> Result<(), HarnessError> {
        self.node(role, name)?.click_accesskit();
        self.run().map(drop)
    }

    /// Give a control keyboard focus.
    pub fn focus(&mut self, role: Role, name: &str) -> Result<(), HarnessError> {
        self.node(role, name)?.focus();
        self.run().map(drop)
    }

    /// Set the number of a slider-like control.
    pub fn set_value(&mut self, role: Role, name: &str, value: f64) -> Result<(), HarnessError> {
        self.node(role, name)?.set_value_accesskit(value);
        self.run().map(drop)
    }

    /// Replace the content of a text field.
    pub fn set_text(&mut self, role: Role, name: &str, text: &str) -> Result<(), HarnessError> {
        self.node(role, name)?.set_text_value_accesskit(text);
        self.run().map(drop)
    }

    fn node<'a>(
        &'a self,
        role: Role,
        name: &'a str,
    ) -> Result<egui_kittest::Node<'a>, HarnessError> {
        let mut matches = self.inner.query_all_by_role_and_label(role, name);
        let first = matches.next().ok_or_else(|| HarnessError::NotFound {
            role,
            name: name.to_owned(),
        })?;
        match matches.count() {
            0 => Ok(first),
            more => Err(HarnessError::Ambiguous {
                role,
                name: name.to_owned(),
                count: more + 1,
            }),
        }
    }

    /// Where `node` is, in logical points. Containers have no rectangle.
    fn rect_of(
        &self,
        node: &egui_kittest::Node<'_>,
        role: Role,
        name: &str,
    ) -> Result<Rect, HarnessError> {
        let bounds =
            node.accesskit_node()
                .bounding_box()
                .ok_or_else(|| HarnessError::NoBounds {
                    role,
                    name: name.to_owned(),
                })?;
        // The tree is in physical pixels.
        let scale = self.inner.ctx.pixels_per_point();
        let point = |x: f64, y: f64| Pos2::new(x as f32 / scale, y as f32 / scale);
        Ok(Rect::from_min_max(
            point(bounds.x0, bounds.y0),
            point(bounds.x1, bounds.y1),
        ))
    }

    // ---- Rendering ----

    /// Whether this machine can render. When `false`, [`Self::render`]
    /// returns [`RenderError::NoAdapter`].
    pub fn can_render(&self) -> bool {
        self.gpu.is_ok()
    }

    /// Render the current frame to an image of `size * pixels_per_point`
    /// pixels.
    pub fn render(&mut self) -> Result<image::RgbaImage, RenderError> {
        let longest = match &self.gpu {
            Ok(longest) => *longest as f32,
            Err(reason) => return Err(RenderError::NoAdapter(reason.clone())),
        };
        // wgpu panics when asked for a texture of a size it cannot make.
        let ctx = &self.inner.ctx;
        let pixels = (ctx.content_rect().size() * ctx.pixels_per_point()).round();
        if pixels.min_elem() < 1.0 || pixels.max_elem() > longest {
            return Err(RenderError::Failed(format!(
                "a frame of {} x {} pixels cannot be rendered (at most {longest} a side)",
                pixels.x, pixels.y
            )));
        }
        self.inner.render().map_err(RenderError::Failed)
    }

    /// Render the current frame and save it as a PNG, creating the directory
    /// if needed.
    pub fn save_png(&mut self, path: impl AsRef<Path>) -> Result<(), RenderError> {
        let image = self.render()?;
        let path = path.as_ref();
        if let Some(directory) = path.parent() {
            std::fs::create_dir_all(directory).map_err(|err| RenderError::Save(err.to_string()))?;
        }
        image
            .save_with_format(path, image::ImageFormat::Png)
            .map_err(|err| RenderError::Save(err.to_string()))
    }
}

fn pointer_button(pos: Pos2, button: PointerButton, pressed: bool, modifiers: Modifiers) -> Event {
    Event::PointerButton {
        pos,
        button,
        pressed,
        modifiers,
    }
}
