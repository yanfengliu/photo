//! Options for building a harness.

use eframe::egui_wgpu::WgpuSetup;
use egui::Vec2;
use egui_kittest::wgpu::WgpuTestRenderer;

use super::{Harness, Host, gpu};

/// Options for building a [`Harness`].
#[must_use]
pub struct HarnessBuilder {
    size: Vec2,
    pixels_per_point: f32,
    step_dt: f32,
    max_steps: u64,
    show_pointer: bool,
    strict_glyphs: bool,
    check_names: bool,
    wgpu_setup: Option<WgpuSetup>,
}

/// The window size a harness has unless told otherwise.
const SIZE: Vec2 = Vec2::new(1280.0, 800.0);
/// The simulated seconds per frame a harness has unless told otherwise.
const STEP_DT: f32 = 1.0 / 60.0;

impl Default for HarnessBuilder {
    fn default() -> Self {
        Self {
            size: SIZE,
            pixels_per_point: 1.0,
            step_dt: STEP_DT,
            max_steps: 120,
            show_pointer: true,
            strict_glyphs: false,
            check_names: true,
            wgpu_setup: None,
        }
    }
}

/// `value` held to `range`, or `fallback` if it is not a number at all.
/// Options are never rejected: egui asserts on a size, scale or time step
/// that is zero, negative or not finite, and the harness must not panic.
fn sane(value: f32, range: std::ops::RangeInclusive<f32>, fallback: f32) -> f32 {
    if value.is_nan() {
        fallback
    } else {
        value.clamp(*range.start(), *range.end())
    }
}

impl HarnessBuilder {
    /// Defaults: 1280 x 800 points at 1 pixel per point, 60 frames a second.
    pub fn new() -> Self {
        Self::default()
    }

    /// The window size in logical points, each side held to 1 to 16384.
    pub fn size(mut self, size: impl Into<Vec2>) -> Self {
        let size = size.into();
        let side = |value: f32, fallback: f32| sane(value, 1.0..=16384.0, fallback);
        self.size = Vec2::new(side(size.x, SIZE.x), side(size.y, SIZE.y));
        self
    }

    /// Physical pixels per logical point (2.0 for a high-density display),
    /// held to 0.25 to 8.
    pub fn pixels_per_point(mut self, pixels_per_point: f32) -> Self {
        self.pixels_per_point = sane(pixels_per_point, 0.25..=8.0, 1.0);
        self
    }

    /// Simulated seconds per frame (1/60 by default), held to between a
    /// millisecond and a twentieth of a second. A press and its release are
    /// a frame apart, and with longer frames egui would no longer take them
    /// for a click, or two clicks for a double-click.
    pub fn step_dt(mut self, seconds: f32) -> Self {
        self.step_dt = sane(seconds, 0.001..=0.05, STEP_DT);
        self
    }

    /// How many frames [`Harness::run`] may take to settle before it gives
    /// up: at least one.
    pub fn max_steps(mut self, max_steps: u64) -> Self {
        self.max_steps = max_steps.max(1);
        self
    }

    /// Whether [`Harness::run`], and with it every action, reports controls
    /// that have no accessible name. On by default: such a control cannot be
    /// reached by a test or by assistive technology. Turn it off to drive an
    /// app that is not there yet.
    pub fn check_names(mut self, check_names: bool) -> Self {
        self.check_names = check_names;
        self
    }

    /// Whether renders show a synthetic pointer where the pointer is.
    pub fn show_pointer(mut self, show_pointer: bool) -> Self {
        self.show_pointer = show_pointer;
        self
    }

    /// Panic when text contains a character no bundled font can draw. Off by
    /// default (a driven app shows user data); tests of our own widgets turn
    /// it on so a stray glyph cannot slip into the chrome.
    pub fn strict_glyphs(mut self, strict_glyphs: bool) -> Self {
        self.strict_glyphs = strict_glyphs;
        self
    }

    /// Replace how the GPU adapter is chosen. The default prefers a software
    /// rasterizer and falls back to any adapter.
    pub fn wgpu_setup(mut self, setup: WgpuSetup) -> Self {
        self.wgpu_setup = Some(setup);
        self
    }

    /// Build the harness around the app that `create` returns. `create` gets
    /// the same creation context `eframe::run_native` would hand it.
    pub fn build<A: eframe::App + 'static>(
        self,
        create: impl FnOnce(&eframe::CreationContext<'_>) -> A,
    ) -> Harness<A> {
        let setup = self
            .wgpu_setup
            .unwrap_or_else(egui_kittest::wgpu::default_wgpu_setup);
        let render_state = gpu::create_render_state(setup);

        let mut builder = egui_kittest::Harness::<'static, A>::builder()
            .with_size(self.size)
            .with_pixels_per_point(self.pixels_per_point)
            .with_step_dt(self.step_dt)
            .with_max_steps(self.max_steps)
            .with_render_cursor(self.show_pointer)
            // Reported by `Harness::run` as an error instead of a panic.
            .with_accessibility_check(false);
        if !self.strict_glyphs {
            builder = builder.allow_missing_glyphs();
        }
        let (builder, gpu) = match render_state {
            Ok(state) => {
                let longest_side = state.device.limits().max_texture_dimension_2d;
                let renderer = WgpuTestRenderer::from_render_state(state);
                (builder.renderer(renderer), Ok(longest_side))
            }
            Err(reason) => (builder.renderer(gpu::NullRenderer), Err(reason)),
        };
        let inner = builder.build_eframe(|cc: &mut eframe::CreationContext<'static>| create(cc));
        Harness {
            inner,
            gpu,
            step_dt: self.step_dt,
            check_names: self.check_names,
            last_click_pos: None,
        }
    }

    /// Build a harness that hosts bare widgets: `ui` is called every frame
    /// with `state`, inside a themed panel. See [`Host`].
    pub fn build_ui<S: 'static>(
        self,
        state: S,
        ui: impl FnMut(&mut egui::Ui, &mut S) + 'static,
    ) -> Harness<Host<S>> {
        self.build(|cc| Host::new(&cc.egui_ctx, state, ui))
    }
}
