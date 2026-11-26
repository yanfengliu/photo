//! `ping`, `observe` and `observe_library`: what the app shows and holds,
//! read without touching it.

use photo_engine::{
    DevelopLoad, Engine, Filter, LoadState, PreviewOrigin, PreviewSize, Source, TaskState,
};
use serde_json::{Value, json};

use super::Session;
use crate::harness::controls;
use crate::harness::protocol::{CmdError, PROTOCOL_VERSION, Params};

/// The most photos one `observe_library` page holds.
const MAX_PAGE: u64 = 1000;
/// The photos a page holds unless asked otherwise.
const DEFAULT_PAGE: u64 = 100;
/// The most selected ids `observe` lists.
const MAX_SELECTED: usize = 200;

/// Liveness and versions.
pub fn ping(params: &Params<'_>, session: &mut Session<'_>) -> Result<Value, CmdError> {
    params.only(&[])?;
    Ok(json!({
        "protocol_version": PROTOCOL_VERSION,
        "app_version": env!("CARGO_PKG_VERSION"),
        "pid": std::process::id(),
        "run_id": session.run.id(),
        "run_dir": session.run.path(),
        "uptime_ms": session.run.elapsed_ms(),
        "sandboxed": session.info.sandboxed,
        "storage_root": session.info.storage_root,
        "gpu": session.driver.gpu_json(),
        "window": window_json(session),
    }))
}

fn window_json(session: &Session<'_>) -> Value {
    let look = session.driver.look();
    json!({ "size": look.size, "pixels_per_point": look.pixels_per_point })
}

/// Everything the app shows and holds, and every control on screen.
pub fn observe(params: &Params<'_>, session: &mut Session<'_>) -> Result<Value, CmdError> {
    params.only(&["controls"])?;
    // The controls come from a frame run now, so what follows is read from
    // that same frame.
    let listed = match params.opt_bool("controls")?.unwrap_or(true) {
        true => Some(session.driver.controls()),
        false => None,
    };
    let driver = &*session.driver;
    let app = driver.app();
    let engine = app.engine();
    let layout = app.layout();
    let pointer = driver
        .ctx()
        .input(|input| input.pointer.latest_pos())
        .map(|pos| json!([pos.x, pos.y]));
    let recent: Vec<Value> = driver
        .observed()
        .toast_history()
        .map(|toast| json!({ "message": toast.message, "seq": toast.seq, "frame": toast.frame }))
        .collect();
    let selected = engine.selection().selected();
    let counts = engine.source_counts();
    let sort = engine.sort();
    let mut data = json!({
        "module": module_name(app.module()),
        "view": view_name(app.view()),
        "zoom": snake(&format!("{:?}", app.zoom())),
        "develop": develop_json(app),
        "dialog_open": app.dialog_open(),
        "layout": {
            "top_bar": layout.top_bar,
            "left": layout.left,
            "right": layout.right,
            "filmstrip": layout.filmstrip,
            "toolbar": layout.toolbar,
        },
        "panels": {
            "left": app.left_panels().open_panels().collect::<Vec<_>>(),
            "right": app.right_panels().open_panels().collect::<Vec<_>>(),
        },
        "source": source_json(engine),
        "filtered": *engine.filter() != Filter::default(),
        "sort": {
            "key": snake(&format!("{:?}", sort.key)),
            "direction": snake(&format!("{:?}", sort.direction)),
        },
        "photos": {
            "shown": engine.photo_count(),
            "in_source": engine.source_photo_count(),
            "all": counts.all_photos,
            "quick_collection": counts.quick_collection,
            "previous_import": counts.previous_import,
        },
        "selection": {
            "active": engine.active_photo().map(|id| id.0),
            "count": selected.len(),
            "selected": selected.iter().take(MAX_SELECTED).map(|id| id.0).collect::<Vec<_>>(),
        },
        "active_photo": active_json(engine),
        "idle": engine.is_idle(),
        "pending_work": {
            "jobs": engine.pending_work().jobs,
            "queued": engine.pending_work().queued,
            "previews": engine.pending_work().previews,
            "tasks": engine.pending_work().tasks,
        },
        "tasks": engine.tasks().iter().map(|task| {
            let (state, reason) = match &task.state {
                TaskState::Running => ("running", None),
                TaskState::Finished => ("finished", None),
                TaskState::Cancelled => ("cancelled", None),
                TaskState::Failed(reason) => ("failed", Some(reason.clone())),
            };
            json!({
                "id": task.id.to_string().parse::<u64>().ok(),
                "kind": snake(&format!("{:?}", task.kind)),
                "label": task.label,
                "done": task.done,
                "total": task.total,
                "errors": task.errors,
                "state": state,
                "reason": reason,
            })
        }).collect::<Vec<_>>(),
        "toasts": {
            "showing": app.toasts().collect::<Vec<_>>(),
            "recent": recent,
        },
        "undo": { "undo": engine.undo_label(), "redo": engine.redo_label() },
        "read_only": engine.is_read_only(),
        "pointer": pointer,
        "window": window_json(session),
    });
    if let Some(listed) = listed {
        data["control_count"] = json!(listed.len());
        data["controls"] = listed.iter().map(controls::to_json).collect();
    }
    Ok(data)
}

/// The Develop canvas: its zoom, where the photo is on screen, and the tool
/// that is open, so that a gesture can be aimed at the photo. `null`
/// outside Develop.
fn develop_json(app: &photo_ui::PhotoApp) -> Value {
    if app.module() != photo_ui::Module::Develop {
        return Value::Null;
    }
    json!({
        "zoom": snake(&format!("{:?}", app.develop_zoom())),
        "photo_rect": app.canvas_photo_rect().map(controls::rect_json),
        "tool": app.tool().map(|tool| snake(&format!("{tool:?}"))),
        "crop_rect": app.crop_rect().map(controls::rect_json),
        "selected_spot": app.selected_spot(),
        "selected_mask": app.selected_mask(),
        "mask_overlay": app.mask_overlay(),
    })
}

/// A module's name as `observe` and `open` report it: `library`, `develop`.
pub(super) fn module_name(module: photo_ui::Module) -> String {
    snake(&format!("{module:?}"))
}

/// A view's name as `observe` and `open` report it: `grid`, `loupe`.
pub(super) fn view_name(view: photo_ui::View) -> String {
    snake(&format!("{view:?}"))
}

/// A page of the photo list, in the order the grid shows it.
pub fn observe_library(params: &Params<'_>, session: &mut Session<'_>) -> Result<Value, CmdError> {
    params.only(&["offset", "limit"])?;
    let offset = params.opt_u64("offset")?.unwrap_or(0);
    let limit = params.opt_u64("limit")?.unwrap_or(DEFAULT_PAGE);
    if limit > MAX_PAGE {
        return Err(CmdError::invalid(format!("`limit` is at most {MAX_PAGE}")));
    }
    let engine = session.driver.engine();
    let photos = engine.photos();
    let active = engine.active_photo();
    let page: Vec<Value> = photos
        .iter()
        .enumerate()
        .skip(offset as usize)
        .take(limit as usize)
        .map(|(index, photo)| {
            json!({
                "index": index,
                "id": photo.id.0,
                "file_name": photo.file_name,
                "kind": photo.kind,
                "rating": photo.rating.stars(),
                "flag": photo.flag,
                "label": photo.label,
                "thumbnail_ready": matches!(
                    engine.load_state(photo.id, PreviewSize::Thumbnail),
                    LoadState::Ready { .. }
                ),
                "selected": engine.selection().is_selected(photo.id),
                "active": active == Some(photo.id),
                "virtual_copy": photo.is_virtual_copy(),
                "has_edits": photo.has_edits,
                "missing": photo.missing,
            })
        })
        .collect();
    Ok(json!({
        "total": photos.len(),
        "offset": offset,
        "limit": limit,
        "photos": page,
    }))
}

/// The active photo: what it is and how far its pixels have got.
fn active_json(engine: &Engine) -> Value {
    let Some(id) = engine.active_photo() else {
        return Value::Null;
    };
    let develop = engine
        .develop()
        .filter(|session| session.photo() == id)
        .map(|session| match session.load_state() {
            DevelopLoad::Nothing => json!({ "state": "nothing" }),
            DevelopLoad::EmbeddedPreview => json!({ "state": "embedded_preview" }),
            DevelopLoad::Decoded => json!({ "state": "decoded" }),
            DevelopLoad::Rendered => json!({ "state": "rendered" }),
            DevelopLoad::Failed(reason) => json!({ "state": "failed", "reason": reason }),
        });
    let load = json!({
        "thumbnail": load_json(engine.load_state(id, PreviewSize::Thumbnail)),
        "standard": load_json(engine.load_state(id, PreviewSize::Standard)),
        "develop": develop,
    });
    match engine.summary(id) {
        Some(photo) => json!({
            "id": id.0,
            "file_name": photo.file_name,
            "path": photo.path,
            "kind": photo.kind,
            "rating": photo.rating.stars(),
            "flag": photo.flag,
            "label": photo.label,
            "size": [photo.width, photo.height],
            "has_edits": photo.has_edits,
            "virtual_copy": photo.is_virtual_copy(),
            "missing": photo.missing,
            "develop_version": photo.develop_version,
            "load": load,
        }),
        None => json!({ "id": id.0, "load": load }),
    }
}

/// Where a preview stands.
fn load_json(state: LoadState) -> Value {
    match state {
        LoadState::Nothing => json!({ "state": "nothing" }),
        LoadState::Pending => json!({ "state": "pending" }),
        LoadState::Ready { origin } => json!({
            "state": "ready",
            "origin": match origin {
                PreviewOrigin::EmbeddedPreview => "embedded_preview",
                PreviewOrigin::Rendered => "rendered",
            },
        }),
        LoadState::Failed(reason) => json!({ "state": "failed", "reason": reason }),
    }
}

/// The source the list shows, with the name the panels give it.
fn source_json(engine: &Engine) -> Value {
    match *engine.source() {
        Source::AllPhotos => json!({ "kind": "all_photos" }),
        Source::QuickCollection => json!({ "kind": "quick_collection" }),
        Source::PreviousImport => json!({ "kind": "previous_import" }),
        Source::Folder {
            id,
            include_subfolders,
        } => json!({
            "kind": "folder",
            "id": id.0,
            "include_subfolders": include_subfolders,
            "path": engine.folders().iter().find(|folder| folder.id == id).map(|folder| &folder.path),
        }),
        Source::Collection(id) => json!({
            "kind": "collection",
            "id": id.0,
            "name": engine.collections().iter().find(|c| c.id == id).map(|c| &c.name),
        }),
        Source::Keyword(id) => json!({
            "kind": "keyword",
            "id": id.0,
            "name": engine.keywords().iter().find(|k| k.id == id).map(|k| &k.name),
        }),
    }
}

/// `CamelCase` as `snake_case`: `OneToOne` is `one_to_one`.
fn snake(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 4);
    for (index, c) in name.chars().enumerate() {
        if c.is_ascii_uppercase() {
            if index > 0 {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_written_in_snake_case() {
        assert_eq!(snake("OneToOne"), "one_to_one");
        assert_eq!(snake("Library"), "library");
        assert_eq!(snake("CaptureTime"), "capture_time");
    }
}
