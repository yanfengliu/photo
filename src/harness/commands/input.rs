//! The user's hands: clicks, keys, typing, drags, hovering, sliders, and
//! waiting for what they started to finish.
//!
//! Coordinates are logical points from the top left of the window, as the
//! rectangles `observe` reports are.

use std::time::Duration;

use egui::accesskit::Role;
use egui::{Pos2, pos2};
use serde_json::{Value, json};

use super::{Session, performed};
use crate::harness::controls::{Control, rect_json, role_name};
use crate::harness::driver::{Driver, finish};
use crate::harness::keys::{parse_key, parse_modifiers};
use crate::harness::protocol::{CmdError, Params};

/// How long `wait_idle` waits unless told otherwise.
const DEFAULT_WAIT: Duration = Duration::from_secs(30);
/// The longest `wait_idle` waits.
const MAX_WAIT: Duration = Duration::from_secs(600);
/// Pointer positions tried while a slider is held, at most.
const SLIDER_PROBES: usize = 40;

/// `click {name, role?}` and `double_click {name, role?}`: the pointer on
/// the middle of the control.
pub fn click(
    params: &Params<'_>,
    session: &mut Session<'_>,
    double: bool,
) -> Result<Value, CmdError> {
    params.only(&["name", "role"])?;
    let name = params.string("name")?;
    let control = session.driver.find(name, params.opt_string("role")?)?;
    within_window(session.driver, &control)?;
    let harness = session.driver.harness();
    let result = if double {
        harness.double_click(control.role, name)
    } else {
        harness.click(control.role, name)
    };
    let warnings = finish(result)?;
    Ok(performed(
        json!({
            "clicked": { "role": role_name(control.role), "name": name, "rect": control.rect.map(rect_json) },
            "double": double,
        }),
        warnings,
    ))
}

/// A control can be used: it is enabled and on screen.
fn within_window(driver: &Driver, control: &Control) -> Result<(), CmdError> {
    let name = control.name.as_deref().unwrap_or_default();
    let role = role_name(control.role);
    if !control.enabled {
        return Err(CmdError::unavailable(format!(
            "the {role} named {name:?} is disabled"
        )));
    }
    let rect = control.rect.ok_or_else(|| {
        CmdError::unavailable(format!(
            "the {role} named {name:?} has no rectangle on screen"
        ))
    })?;
    let [width, height] = driver.look().size;
    let window = egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(width, height));
    if !window.contains(rect.center()) {
        return Err(CmdError::unavailable(format!(
            "the {role} named {name:?} is outside the window (scrolled out of view?)"
        )));
    }
    Ok(())
}

/// `key {name, mods}`: press and release a key with modifiers held.
pub fn key(params: &Params<'_>, session: &mut Session<'_>) -> Result<Value, CmdError> {
    params.only(&["name", "mods"])?;
    let name = params.string("name")?;
    let key = parse_key(name)?;
    let mods = params.opt_names("mods")?;
    let modifiers = parse_modifiers(&mods)?;
    let result = session.driver.harness().press_key(key, modifiers);
    let warnings = finish(result)?;
    Ok(performed(
        json!({ "key": key.name(), "mods": mods }),
        warnings,
    ))
}

/// `type_text {text}`: type into whatever has keyboard focus.
pub fn type_text(params: &Params<'_>, session: &mut Session<'_>) -> Result<Value, CmdError> {
    params.only(&["text"])?;
    let text = params.string("text")?;
    let focused = session
        .driver
        .ctx()
        .memory(|memory| memory.focused().is_some());
    let result = session.driver.harness().type_text(text);
    let mut warnings = finish(result)?;
    if !focused {
        warnings.push(
            "nothing had keyboard focus, so the text went nowhere; click a text field first"
                .to_owned(),
        );
    }
    Ok(performed(
        json!({ "typed": text.chars().count() }),
        warnings,
    ))
}

/// `drag {from, to}`: press at one point, move in steps, release at the
/// other.
pub fn drag(params: &Params<'_>, session: &mut Session<'_>) -> Result<Value, CmdError> {
    params.only(&["from", "to"])?;
    let from = point_in_window(session.driver, params, "from")?;
    let to = point_in_window(session.driver, params, "to")?;
    session.driver.separate_clicks();
    let result = session.driver.harness().drag(from, to);
    let warnings = finish(result)?;
    Ok(performed(
        json!({ "from": [from.x, from.y], "to": [to.x, to.y] }),
        warnings,
    ))
}

/// `hover {at}`: move the pointer there without pressing anything.
pub fn hover(params: &Params<'_>, session: &mut Session<'_>) -> Result<Value, CmdError> {
    params.only(&["at"])?;
    let at = point_in_window(session.driver, params, "at")?;
    let result = session.driver.harness().hover_at(at);
    let warnings = finish(result)?;
    Ok(performed(json!({ "at": [at.x, at.y] }), warnings))
}

fn point_in_window(driver: &Driver, params: &Params<'_>, key: &str) -> Result<Pos2, CmdError> {
    let [x, y] = params.point(key)?;
    let [width, height] = driver.look().size;
    if !(0.0..=width).contains(&x) || !(0.0..=height).contains(&y) {
        return Err(CmdError::invalid(format!(
            "`{key}` [{x}, {y}] is outside the window, which is {width} x {height} points"
        )));
    }
    Ok(pos2(x, y))
}

/// `set_slider {name, value}`: drag the slider's handle to the value and
/// let go, so the slider sees one drag and one release, as from a user.
///
/// The value under the pointer is only known by trying, since a slider may
/// spread its values unevenly: the button is pressed where a linear track
/// would hold the value, then, still held, the pointer is moved by halves
/// along the track to where the slider shows the value (or the nearest it
/// can show), and released there.
pub fn set_slider(params: &Params<'_>, session: &mut Session<'_>) -> Result<Value, CmdError> {
    params.only(&["name", "value"])?;
    let name = params.string("name")?;
    let wanted = params.number("value")?;
    let slider = session.driver.find(name, Some("slider"))?;
    within_window(session.driver, &slider)?;
    let Some(rect) = slider.rect else {
        return Err(CmdError::unavailable("the slider has no rectangle"));
    };
    let target = match (slider.min, slider.max) {
        (Some(min), Some(max)) if min <= max => wanted.clamp(min, max),
        _ => wanted,
    };
    let tolerance = match (slider.step, slider.min, slider.max) {
        (Some(step), _, _) if step > 0.0 => step / 2.0,
        (_, Some(min), Some(max)) if max > min => (max - min) / 10_000.0,
        _ => 1e-3,
    };
    let seek = Seek {
        name,
        track: rect,
        target,
        tolerance,
    };
    let y = rect.center().y;
    let mut at = linear_x(rect, slider.min, slider.max, target);
    let driver = &mut *session.driver;
    driver.separate_clicks();
    let mut warnings = finish(driver.harness().pointer_down(pos2(at, y)))?;
    let sought = seek.run(driver, &mut at, &mut warnings);
    // Let go where the pointer is, whatever happened on the way, so no
    // button is left held down.
    let released = finish(driver.harness().pointer_up(pos2(at, y)));
    sought?;
    warnings.extend(released?);
    let value = slider_value(driver, name);
    Ok(performed(
        json!({
            "name": name,
            "requested": wanted,
            "target": target,
            "before": slider.numeric,
            "value": value,
            "exact": seek.distance(value) <= tolerance,
        }),
        warnings,
    ))
}

/// The search along a held slider for the place that shows a value.
struct Seek<'a> {
    name: &'a str,
    track: egui::Rect,
    target: f64,
    tolerance: f64,
}

impl Seek<'_> {
    fn distance(&self, value: Option<f64>) -> f64 {
        value.map_or(f64::INFINITY, |value| (value - self.target).abs())
    }

    /// With the button held, move the pointer by halves along the track to
    /// where the slider shows the target, or the nearest value it showed;
    /// `at` follows the pointer.
    fn run(
        &self,
        driver: &mut Driver,
        at: &mut f32,
        warnings: &mut Vec<String>,
    ) -> Result<(), CmdError> {
        let y = self.track.center().y;
        let mut best = (*at, slider_value(driver, self.name));
        let (mut low, mut high) = (self.track.left() - 1.0, self.track.right() + 1.0);
        for _ in 0..SLIDER_PROBES {
            if self.distance(best.1) <= self.tolerance || high - low < 0.01 {
                break;
            }
            let middle = (low + high) / 2.0;
            warnings.extend(finish(driver.harness().pointer_move(pos2(middle, y)))?);
            *at = middle;
            let value = slider_value(driver, self.name);
            if self.distance(value) < self.distance(best.1) {
                best = (middle, value);
            }
            match value {
                Some(value) if value < self.target => low = middle,
                Some(_) => high = middle,
                None => break,
            }
        }
        if *at != best.0 {
            warnings.extend(finish(driver.harness().pointer_move(pos2(best.0, y)))?);
            *at = best.0;
        }
        Ok(())
    }
}

/// Where `value` would be on a track running evenly from `min` to `max`
/// across `rect`, with room for the handle at each end.
fn linear_x(rect: egui::Rect, min: Option<f64>, max: Option<f64>, value: f64) -> f32 {
    let inset = (rect.height() / 2.0).min(rect.width() / 4.0);
    let (left, right) = (rect.left() + inset, rect.right() - inset);
    match (min, max) {
        (Some(min), Some(max)) if max > min => {
            let fraction = ((value - min) / (max - min)).clamp(0.0, 1.0) as f32;
            left + fraction * (right - left)
        }
        _ => rect.center().x,
    }
}

fn slider_value(driver: &Driver, name: &str) -> Option<f64> {
    driver.harness_ref().control(Role::Slider, name).ok()?.value
}

/// `reset_slider {name}`: double-click the slider, which puts it back to
/// its default.
pub fn reset_slider(params: &Params<'_>, session: &mut Session<'_>) -> Result<Value, CmdError> {
    params.only(&["name"])?;
    let name = params.string("name")?;
    let slider = session.driver.find(name, Some("slider"))?;
    within_window(session.driver, &slider)?;
    let result = session.driver.harness().double_click(Role::Slider, name);
    let warnings = finish(result)?;
    Ok(performed(
        json!({
            "name": name,
            "before": slider.numeric,
            "value": slider_value(session.driver, name),
        }),
        warnings,
    ))
}

/// `wait_idle {timeout_ms}`: run frames until the engine is idle and the UI
/// has come to rest.
pub fn wait_idle(params: &Params<'_>, session: &mut Session<'_>) -> Result<Value, CmdError> {
    params.only(&["timeout_ms"])?;
    let timeout = params
        .opt_u64("timeout_ms")?
        .map_or(DEFAULT_WAIT, Duration::from_millis);
    if timeout > MAX_WAIT {
        return Err(CmdError::invalid(format!(
            "`timeout_ms` is at most {}",
            MAX_WAIT.as_millis()
        )));
    }
    let rest = session.driver.wait_idle(timeout)?;
    Ok(json!({ "idle": true, "waited_ms": rest.waited_ms }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_value_is_first_tried_where_an_even_track_would_hold_it() {
        let rect = egui::Rect::from_min_max(pos2(100.0, 0.0), pos2(300.0, 20.0));
        assert_eq!(linear_x(rect, Some(0.0), Some(100.0), 0.0), 110.0);
        assert_eq!(linear_x(rect, Some(0.0), Some(100.0), 100.0), 290.0);
        assert_eq!(linear_x(rect, Some(-5.0), Some(5.0), 0.0), 200.0);
        assert_eq!(linear_x(rect, None, None, 3.0), 200.0);
        assert_eq!(
            linear_x(rect, Some(0.0), Some(1.0), 9.0),
            290.0,
            "held to the track"
        );
    }
}
