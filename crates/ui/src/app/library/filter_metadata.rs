//! The Metadata tab of the filter bar: columns of the camera, lens, ISO and
//! capture date values among the photos in view, each with its count.
//!
//! Each column lists the values of the photos that pass every other
//! column's choice (the engine's facet values), so choosing in one column
//! narrows the others. Cameras and lenses can be chosen several at a time;
//! an ISO or a day is one at a time.

use egui::{Button, RichText, ScrollArea, Ui, vec2};
use photo_core::time::DateTime;
use photo_engine::{DateRange, Facet, FacetValue, Filter, IsoRange};

use crate::app::Cx;
use crate::theme::{palette, space};

/// Height of the tab's row of columns.
pub(super) const HEIGHT: f32 = 150.0;
/// Height of a column's header.
const HEADER: f32 = 18.0;

/// The columns, in order, with their headers.
const COLUMNS: [(Facet, &str); 4] = [
    (Facet::CaptureDate, "Date"),
    (Facet::Camera, "Camera"),
    (Facet::Lens, "Lens"),
    (Facet::Iso, "ISO"),
];

/// Show the columns, editing `filter`.
pub(super) fn show(ui: &mut Ui, cx: &mut Cx<'_>, filter: &mut Filter) {
    let width = ((ui.available_width() - space::LG * 3.0) / COLUMNS.len() as f32).floor();
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = space::LG;
        for (facet, header) in COLUMNS {
            let values = match cx.engine.facet_values(facet) {
                Ok(values) => values,
                Err(error) => {
                    log::warn!("cannot list the {header} values: {error}");
                    Vec::new()
                }
            };
            ui.allocate_ui(vec2(width, HEIGHT - space::SM), |ui| {
                ui.set_width(width);
                column(ui, facet, header, &values, filter);
            });
        }
    });
}

/// One column: its header and its values.
fn column(ui: &mut Ui, facet: Facet, header: &str, values: &[FacetValue], filter: &mut Filter) {
    ui.vertical(|ui| {
        ui.add_sized(
            vec2(ui.available_width(), HEADER),
            egui::Label::new(
                RichText::new(header.to_uppercase())
                    .small()
                    .color(palette::TEXT_DIM),
            ),
        );
        ScrollArea::vertical()
            .id_salt(("facet", header))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                let all = values.iter().map(|value| value.count).sum::<u64>();
                let none_chosen = !chosen_any(facet, filter);
                let row = Button::selectable(none_chosen, format!("All ({all})"))
                    .min_size(vec2(ui.available_width(), 18.0));
                if ui
                    .add(row)
                    .accessible_name(format!("{header} all"))
                    .clicked()
                {
                    clear(facet, filter);
                }
                for value in values {
                    let shown = if value.value.is_empty() {
                        "Unknown"
                    } else {
                        value.value.as_str()
                    };
                    let selected = is_chosen(facet, &value.value, filter);
                    let usable = choosable(facet, &value.value);
                    // A long lens name is cut short with an ellipsis rather
                    // than run under the next column.
                    let row = Button::selectable(selected, format!("{shown} ({})", value.count))
                        .min_size(vec2(ui.available_width(), 18.0))
                        .truncate();
                    let response = ui
                        .add_enabled(usable, row)
                        .accessible_name(format!("{header} {shown}"));
                    if response.clicked() {
                        toggle(facet, &value.value, filter);
                    }
                }
            });
    });
}

/// Whether a column's value can be chosen: a photo with no ISO or no
/// capture date is never inside a range, so "Unknown" there cannot be.
fn choosable(facet: Facet, value: &str) -> bool {
    match facet {
        Facet::Camera | Facet::Lens => true,
        Facet::Iso | Facet::CaptureDate => !value.is_empty(),
    }
}

/// Whether anything is chosen in the column of `facet`.
fn chosen_any(facet: Facet, filter: &Filter) -> bool {
    match facet {
        Facet::Camera => !filter.cameras.is_empty(),
        Facet::Lens => !filter.lenses.is_empty(),
        Facet::Iso => filter.iso.is_some(),
        Facet::CaptureDate => filter.capture.is_some(),
    }
}

/// Clear the column of `facet`.
fn clear(facet: Facet, filter: &mut Filter) {
    match facet {
        Facet::Camera => filter.cameras.clear(),
        Facet::Lens => filter.lenses.clear(),
        Facet::Iso => filter.iso = None,
        Facet::CaptureDate => filter.capture = None,
    }
}

/// Whether `value` is chosen in the column of `facet`.
fn is_chosen(facet: Facet, value: &str, filter: &Filter) -> bool {
    match facet {
        Facet::Camera => filter.cameras.iter().any(|camera| camera == value),
        Facet::Lens => filter.lenses.iter().any(|lens| lens == value),
        Facet::Iso => value.parse::<u32>().ok().is_some_and(|iso| {
            filter.iso
                == Some(IsoRange {
                    min: Some(iso),
                    max: Some(iso),
                })
        }),
        Facet::CaptureDate => day(value).is_some_and(|range| filter.capture == Some(range)),
    }
}

/// Choose `value` in its column, or take it back.
fn toggle(facet: Facet, value: &str, filter: &mut Filter) {
    let chosen = is_chosen(facet, value, filter);
    match facet {
        Facet::Camera => toggle_in(&mut filter.cameras, value),
        Facet::Lens => toggle_in(&mut filter.lenses, value),
        Facet::Iso => {
            filter.iso = (!chosen)
                .then(|| value.parse::<u32>().ok())
                .flatten()
                .map(|iso| IsoRange {
                    min: Some(iso),
                    max: Some(iso),
                });
        }
        Facet::CaptureDate => filter.capture = if chosen { None } else { day(value) },
    }
}

fn toggle_in(list: &mut Vec<String>, value: &str) {
    if list.iter().any(|item| item == value) {
        list.retain(|item| item != value);
    } else {
        list.push(value.to_owned());
    }
}

/// The whole calendar day `YYYY-MM-DD` as a capture range.
fn day(value: &str) -> Option<DateRange> {
    let mut parts = value.split('-');
    let year = parts.next()?.parse().ok()?;
    let month = parts.next()?.parse().ok()?;
    let date = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some(DateRange {
        from: DateTime::new(year, month, date, 0, 0, 0),
        to: Some(DateTime::new(year, month, date, 23, 59, 59)?),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_day_covers_its_whole_date_and_nonsense_is_no_day() {
        let range = day("2024-03-15").unwrap();
        assert_eq!(range.from, DateTime::new(2024, 3, 15, 0, 0, 0));
        assert_eq!(range.to, DateTime::new(2024, 3, 15, 23, 59, 59));
        assert!(day("2024-13-01").is_none());
        assert!(day("yesterday").is_none());
        assert!(day("2024-03-15-01").is_none());
    }

    #[test]
    fn choosing_toggles_and_iso_and_dates_are_one_at_a_time() {
        let mut filter = Filter::default();
        toggle(Facet::Camera, "Z8", &mut filter);
        toggle(Facet::Camera, "X-T5", &mut filter);
        assert_eq!(filter.cameras, ["Z8", "X-T5"]);
        toggle(Facet::Camera, "Z8", &mut filter);
        assert_eq!(filter.cameras, ["X-T5"]);

        toggle(Facet::Iso, "200", &mut filter);
        toggle(Facet::Iso, "400", &mut filter);
        assert!(is_chosen(Facet::Iso, "400", &filter) && !is_chosen(Facet::Iso, "200", &filter));
        toggle(Facet::Iso, "400", &mut filter);
        assert_eq!(filter.iso, None);

        toggle(Facet::CaptureDate, "2024-03-15", &mut filter);
        assert!(is_chosen(Facet::CaptureDate, "2024-03-15", &filter));
        clear(Facet::CaptureDate, &mut filter);
        assert!(!chosen_any(Facet::CaptureDate, &filter));
        assert!(!choosable(Facet::Iso, "") && choosable(Facet::Camera, ""));
    }
}
