//! How things are written on screen: photo and source names, counts,
//! exposure readings, and text that must fit a width.

use std::path::Path;
use std::sync::Arc;

use egui::text::{LayoutJob, TextWrapping};
use egui::{Color32, FontId, Galley, Painter};
use photo_core::meta::{PhotoMetadata, format_shutter};
use photo_core::time::DateTime;
use photo_engine::{CollectionKind, Engine, PhotoSummary, Source};

/// The name a photo is shown and found by: its file name, and for a
/// virtual copy the copy's name after it.
pub(crate) fn photo_name(summary: &PhotoSummary) -> String {
    match &summary.copy_name {
        Some(copy) if summary.is_virtual_copy() => format!("{} ({copy})", summary.file_name),
        _ => summary.file_name.clone(),
    }
}

/// `count` and the noun, singular or plural: "1 photo", "6 photos".
pub(crate) fn count(count: usize, one: &str, many: &str) -> String {
    if count == 1 {
        format!("1 {one}")
    } else {
        format!("{count} {many}")
    }
}

/// The last component of a path, or the path itself when it has none (a
/// root).
pub(crate) fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

/// What the source line calls the current source.
pub(crate) fn source_name(engine: &Engine) -> String {
    match *engine.source() {
        Source::AllPhotos => "All Photographs".to_owned(),
        Source::QuickCollection => "Quick Collection".to_owned(),
        Source::PreviousImport => "Previous Import".to_owned(),
        Source::Folder { id, .. } => engine
            .folders()
            .iter()
            .find(|folder| folder.id == id)
            .map_or_else(
                || "Folder".to_owned(),
                |folder| format!("Folder {}", file_name(&folder.path)),
            ),
        Source::Collection(id) => engine
            .collections()
            .iter()
            .find(|collection| collection.id == id)
            .map_or_else(
                || "Collection".to_owned(),
                |collection| format!("{} {}", kind_name(collection.kind), collection.name),
            ),
        Source::Keyword(id) => engine
            .keywords()
            .iter()
            .find(|keyword| keyword.id == id)
            .map_or_else(
                || "Keyword".to_owned(),
                |keyword| format!("Keyword {}", keyword.name),
            ),
    }
}

/// What a collection of this kind is called, capitalised.
pub(crate) fn kind_name(kind: CollectionKind) -> &'static str {
    match kind {
        CollectionKind::Set => "Collection Set",
        CollectionKind::Regular => "Collection",
        CollectionKind::Smart => "Smart Collection",
        CollectionKind::Quick => "Quick Collection",
    }
}

/// A capture time as photographers read it: `2024-03-15 07:42:10`.
pub(crate) fn date_time(time: &DateTime) -> String {
    format!(
        "{} {:02}:{:02}:{:02}",
        time.date_string(),
        time.hour,
        time.minute,
        time.second
    )
}

/// Shutter and aperture together: `1/250 s at f/5.6`.
pub(crate) fn exposure(metadata: &PhotoMetadata) -> Option<String> {
    let shutter = metadata
        .shutter_seconds
        .map(format_shutter)
        .filter(|text| !text.is_empty())
        .map(|text| format!("{text} s"));
    let aperture = metadata.aperture.map(|f| format!("f/{}", trim(f, 1)));
    match (shutter, aperture) {
        (Some(shutter), Some(aperture)) => Some(format!("{shutter} at {aperture}")),
        (Some(one), None) | (None, Some(one)) => Some(one),
        (None, None) => None,
    }
}

/// A focal length: `35 mm`.
pub(crate) fn focal_length(metadata: &PhotoMetadata) -> Option<String> {
    metadata
        .focal_length_mm
        .map(|mm| format!("{} mm", trim(mm, 1)))
}

/// `value` with at most `decimals` decimals and no trailing zeros.
fn trim(value: f32, decimals: usize) -> String {
    let text = format!("{value:.decimals$}");
    if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.').to_owned()
    } else {
        text
    }
}

/// Names typed into a field, separated by commas: trimmed, blanks left
/// out.
pub(crate) fn split_names(typed: &str) -> Vec<String> {
    typed
        .split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .collect()
}

/// `text` laid out on one line, cut short with an ellipsis where it would
/// pass `width`.
pub(crate) fn fitted(
    painter: &Painter,
    text: &str,
    font: FontId,
    color: Color32,
    width: f32,
) -> Arc<Galley> {
    let mut job = LayoutJob::simple_singleline(text.to_owned(), font, color);
    job.wrap = TextWrapping::truncate_at_width(width.max(0.0));
    painter.layout_job(job)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposure_reads_like_a_camera() {
        let metadata = PhotoMetadata {
            shutter_seconds: Some(1.0 / 250.0),
            aperture: Some(5.6),
            focal_length_mm: Some(35.0),
            ..PhotoMetadata::default()
        };
        assert_eq!(exposure(&metadata).as_deref(), Some("1/250 s at f/5.6"));
        assert_eq!(focal_length(&metadata).as_deref(), Some("35 mm"));
        let aperture_only = PhotoMetadata {
            aperture: Some(8.0),
            ..PhotoMetadata::default()
        };
        assert_eq!(exposure(&aperture_only).as_deref(), Some("f/8"));
        assert_eq!(exposure(&PhotoMetadata::default()), None);
    }

    #[test]
    fn typed_names_are_split_at_commas_and_trimmed() {
        assert_eq!(split_names(" sunset, beach ,, "), ["sunset", "beach"]);
        assert!(split_names(" , ").is_empty());
    }

    #[test]
    fn counts_are_singular_for_one() {
        assert_eq!(count(1, "photo", "photos"), "1 photo");
        assert_eq!(count(0, "photo", "photos"), "0 photos");
        assert_eq!(count(12, "photo", "photos"), "12 photos");
    }

    #[test]
    fn a_capture_time_has_date_and_clock() {
        let time = DateTime::new(2024, 3, 15, 7, 42, 10).unwrap();
        assert_eq!(date_time(&time), "2024-03-15 07:42:10");
    }
}
