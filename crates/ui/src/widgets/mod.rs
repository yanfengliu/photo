//! The widget kit: every control the Library and Develop screens are built
//! from.
//!
//! Each widget is a plain function or a small builder that edits a value the
//! caller owns and reports what happened. None of them keeps application
//! state, and each interactive part has an accessible name, which is how
//! tests and the automation harness find it.
//!
//! The editing widgets report `changed` on every frame the value moved and
//! `finished` once when the edit ends. A gesture lives only while its widget
//! is shown every frame: if the widget goes away mid-drag (its panel closes),
//! the drag is over and no `finished` follows, and a number typed but not
//! yet committed is dropped. A caller that hides widgets on its own records
//! what changed until then itself.

pub mod adj_slider;
pub mod color_label;
pub mod color_wheel;
pub mod curve_editor;
pub mod dialog;
pub mod flag;
pub mod histogram;
pub mod icons;
pub mod panel;
pub mod rating;
pub mod thumbnail;
pub mod toast;

pub use adj_slider::{AdjSlider, Scale, SliderResponse, adj_slider};
pub use color_label::{color_labels, label_color};
pub use color_wheel::{ColorWheel, WheelResponse};
pub use curve_editor::{CurveEditor, CurveResponse};
pub use dialog::{Dialog, DialogOutcome, DialogResponse};
pub use flag::flag_toggle;
pub use histogram::{ClippingToggles, HistogramData, histogram};
pub use icons::{Icon, IconButton, icon_button};
pub use panel::{Panel, PanelGroup, PanelResponse, panel_column};
pub use rating::rating_stars;
pub use thumbnail::{Badges, SelectionTier, ThumbImage, Thumbnail, ThumbnailResponse};
pub use toast::Toasts;

/// A widget's state between frames, and the frame it was kept in.
#[derive(Clone)]
struct Kept<T> {
    frame: u64,
    state: T,
}

/// Keep `state` for the widget `id` until next frame, or forget it.
///
/// Gestures that span frames (a drag, text being typed) keep their bookkeeping
/// in egui's temporary memory: it lives exactly as long as the gesture and is
/// never persisted. A widget with something to keep calls this every frame.
fn remember<T: Clone + Send + Sync + 'static>(ctx: &egui::Context, id: egui::Id, state: Option<T>) {
    let frame = ctx.cumulative_frame_nr();
    ctx.data_mut(|data| match state {
        Some(state) => {
            data.insert_temp(id, Kept { frame, state });
        }
        None => data.remove::<Kept<T>>(id),
    });
}

/// The state [`remember`] kept for the widget `id` last frame, if any.
///
/// Older state is not returned: the widget was not shown in between (its
/// panel was closed, say), so the gesture it belonged to is over and must not
/// come back to life when the widget reappears.
fn recall<T: Clone + Send + Sync + 'static>(ctx: &egui::Context, id: egui::Id) -> Option<T> {
    let kept: Kept<T> = ctx.data(|data| data.get_temp(id))?;
    (kept.frame + 1 >= ctx.cumulative_frame_nr()).then_some(kept.state)
}

/// When and where a widget acted on a double-click.
#[derive(Clone, Copy)]
struct Burst {
    at: f64,
    pos: egui::Pos2,
}

/// Whether `response` was double-clicked as the user meant it. `burst` is
/// what [`in_click_burst`] says for the widget this frame.
///
/// egui counts the second click of a double-click as a triple-click when the
/// first came right after a click anywhere else: its triple-click test does
/// not look at where the click before last was. Resetting one slider after
/// another by double-clicking each is how that happens, so such a triple is
/// taken for a double-click, unless it is one click too many of a
/// double-click this widget has just acted on.
fn meant_double_click(response: &egui::Response, burst: bool) -> bool {
    response.double_clicked() || (response.triple_clicked() && !burst)
}

/// Note that the widget `id` has just acted on a double-click.
fn end_click_burst(ctx: &egui::Context, id: egui::Id) {
    let burst = ctx.input(|input| {
        let pos = input.pointer.interact_pos()?;
        Some(Burst {
            at: input.time,
            pos,
        })
    });
    remember(ctx, id.with("click burst"), burst);
}

/// Whether a press on the widget `id` now is one click too many of the burst
/// whose double-click it has acted on: on the same spot, right after it.
///
/// egui counts such a click as the third of a triple-click, not as a new
/// one. A widget that resets on a double-click would take it as a fresh edit
/// and undo the reset. Call every frame.
fn in_click_burst(ctx: &egui::Context, id: egui::Id) -> bool {
    let id = id.with("click burst");
    let Some(mut burst) = recall::<Burst>(ctx, id) else {
        return false;
    };
    let (delay, reach) = ctx.options(|options| {
        let input = &options.input_options;
        (input.max_double_click_delay, input.max_click_dist)
    });
    let (fresh, within) = ctx.input(|input| {
        let pointer = input.pointer.interact_pos();
        let near = pointer.is_some_and(|pos| pos.distance(burst.pos) < reach);
        let fresh = input.time - burst.at < delay;
        if fresh && near && input.pointer.primary_pressed() {
            // The burst goes on for as long as the clicks keep coming.
            burst.at = input.time;
        }
        (fresh, fresh && near)
    });
    remember(ctx, id, fresh.then_some(burst));
    within
}
