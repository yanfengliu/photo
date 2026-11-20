//! Keywording and the Keyword List, both about the selected photos (the
//! active photo when none is selected).
//!
//! Keywording shows their keywords as tags (a tag carried by only some of
//! them is marked with an asterisk): a click on a tag's cross takes that
//! keyword off them, and names typed in the field (separated by commas) go
//! onto them with Enter. The Keyword List is the keyword tree with each
//! keyword's count and a tick where every one of them carries it (a dash
//! where only some do); clicking a tick puts the keyword on them all or
//! takes it off.

use std::collections::{HashMap, HashSet};

use egui::{
    Checkbox, CornerRadius, Key, Rect, RichText, Role, Sense, TextEdit, Ui, UiBuilder, Vec2,
    WidgetInfo, pos2, vec2,
};
use photo_core::meta::{KeywordId, PhotoId};
use photo_engine::{Engine, KeywordInfo, KeywordUse, Target};

use crate::app::actions::Action;
use crate::app::{Cx, text};
use crate::theme::{self, palette, radius, size, space};
use crate::widgets::{Icon, IconButton};

/// The most selected photos whose keywords are read; the tags and ticks of
/// a larger selection are those of its first photos.
const MOST_READ: usize = 500;
/// How far each level of the keyword tree is indented.
const INDENT: f32 = 14.0;

/// What the keyword panels remember between frames.
#[derive(Clone, Debug, Default)]
pub(crate) struct KeywordState {
    /// The names being typed into Keywording.
    pub(crate) draft: String,
    /// The keywords of the photos the panels are about, and what they were
    /// read for.
    read: Option<Read>,
}

#[derive(Clone, Debug)]
struct Read {
    /// The selected photos, the active one and the keyword list they were
    /// read against: a keyword put on or taken off a photo changes a count
    /// in that list.
    selection: Vec<PhotoId>,
    active: Option<PhotoId>,
    keywords: Vec<KeywordInfo>,
    /// How many photos the panels are about: those `Target::Selected`
    /// names, the selection or else the active photo.
    subject: usize,
    /// How many of them were read.
    counted: usize,
    /// Each keyword of the photos read, with how many carry it.
    uses: Vec<KeywordUse>,
}

impl KeywordState {
    /// The keywords of the photos the panels are about, read again only
    /// when the selection, the active photo or the keyword list changed.
    fn read(&mut self, engine: &Engine) -> &Read {
        let selection = engine.selection().selected();
        let active = engine.active_photo();
        let stale = self.read.as_ref().is_none_or(|read| {
            read.selection != selection
                || read.active != active
                || read.keywords != engine.keywords()
        });
        if stale {
            self.read = Some(read_keywords(engine));
        }
        self.read.get_or_insert_with(|| read_keywords(engine))
    }
}

fn read_keywords(engine: &Engine) -> Read {
    let selection = engine.selection().selected().to_vec();
    let active = engine.active_photo();
    let subject: Vec<PhotoId> = if selection.is_empty() {
        active.into_iter().collect()
    } else {
        selection.clone()
    };
    let counted = subject.len().min(MOST_READ);
    let uses = engine
        .keywords_of(Target::Photos(subject[..counted].to_vec()))
        .unwrap_or_else(|error| {
            log::warn!("cannot read the keywords of the selected photos: {error}");
            Vec::new()
        });
    Read {
        selection,
        active,
        keywords: engine.keywords().to_vec(),
        subject: subject.len(),
        counted,
        uses,
    }
}

/// The Keywording panel.
pub(super) fn keywording(ui: &mut Ui, cx: &mut Cx<'_>) {
    if cx.engine.selection().selected().is_empty() && cx.engine.active_photo().is_none() {
        ui.label(RichText::new("Select photos to keyword them.").color(palette::TEXT_DIM));
        return;
    }
    let read = cx.ui.right.keywords.read(cx.engine).clone();
    if read.uses.is_empty() {
        ui.label(RichText::new("No keywords yet.").color(palette::TEXT_DIM));
    }
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(space::SM, space::SM);
        for used in &read.uses {
            let partial = used.photos < read.counted;
            if tag(ui, &used.keyword.name, partial) {
                let ids = vec![used.keyword.id];
                cx.act(Action::RemoveKeywordIds(Target::Selected, ids));
            }
        }
    });
    if read.subject > read.counted {
        let note = format!("Keywords of the first {} selected photos.", read.counted);
        ui.label(RichText::new(note).color(palette::TEXT_DIM).small());
    }
    ui.add_space(space::MD);
    let draft = &mut cx.ui.right.keywords.draft;
    let response = ui
        .add(
            TextEdit::singleline(draft)
                .id_salt("add-keywords")
                .hint_text("Add keywords, separated by commas")
                .desired_width(f32::INFINITY),
        )
        .accessible_name("Add keywords");
    let entered = response.lost_focus() && ui.input(|input| input.key_pressed(Key::Enter));
    let set_from_outside = response.changed() && !response.has_focus();
    if entered || set_from_outside {
        let names = text::split_names(draft);
        draft.clear();
        if !names.is_empty() {
            cx.act(Action::AddKeywords(Target::Selected, names));
        }
    }
}

/// One tag: the name, an asterisk if only some photos carry it, and a
/// cross named "Remove keyword {name}", laid out as one piece so a row of
/// tags wraps between tags, never inside one. Returns whether the cross
/// was clicked.
fn tag(ui: &mut Ui, name: &str, partial: bool) -> bool {
    const CROSS: f32 = 16.0;
    const PAD: f32 = 7.0;
    let shown = if partial {
        format!("{name}*")
    } else {
        name.to_owned()
    };
    let galley =
        ui.painter()
            .layout_no_wrap(shown.clone(), theme::body_font(), palette::TEXT_STRONG);
    let size = vec2(PAD + galley.size().x + space::XS + CROSS + 2.0, size::ROW);
    let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
    response.widget_info(|| WidgetInfo::labeled(Role::Label, true, &shown));
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(radius::MD), palette::RAISED);
    let text = pos2(rect.left() + PAD, rect.center().y - galley.size().y / 2.0);
    painter.galley(text, galley, palette::TEXT_STRONG);
    let cross = Rect::from_min_size(
        pos2(rect.right() - CROSS - 2.0, rect.center().y - CROSS / 2.0),
        Vec2::splat(CROSS),
    );
    let label = format!("Remove keyword {name}");
    // A child of its own, so the button does not move the row's cursor.
    ui.new_child(UiBuilder::new().max_rect(cross))
        .add(IconButton::new(Icon::Close, &label).side(CROSS))
        .clicked()
}

/// The Keyword List panel.
pub(super) fn list(ui: &mut Ui, cx: &mut Cx<'_>) {
    let keywords = cx.engine.keywords().to_vec();
    if keywords.is_empty() {
        ui.label(
            RichText::new("Keywords appear here once photos carry them.").color(palette::TEXT_DIM),
        );
        return;
    }
    let read = cx.ui.right.keywords.read(cx.engine);
    let counted = read.counted;
    let carried: HashMap<KeywordId, usize> = read
        .uses
        .iter()
        .map(|used| (used.keyword.id, used.photos))
        .collect();
    for (index, depth) in tree(&keywords) {
        let keyword = &keywords[index];
        let carriers = carried.get(&keyword.id).copied().unwrap_or(0);
        ui.horizontal(|ui| {
            ui.add_space(INDENT * depth as f32);
            let mut ticked = counted > 0 && carriers == counted;
            let checkbox = Checkbox::new(&mut ticked, keyword.name.as_str())
                .indeterminate(carriers > 0 && carriers < counted);
            let response = ui
                .add_enabled(counted > 0, checkbox)
                .accessible_name(format!("Keyword {}", keyword.name));
            if response.changed() {
                let ids = vec![keyword.id];
                cx.act(if ticked {
                    Action::AddKeywordIds(Target::Selected, ids)
                } else {
                    Action::RemoveKeywordIds(Target::Selected, ids)
                });
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    RichText::new(keyword.photo_count.to_string())
                        .color(palette::TEXT_DIM)
                        .small(),
                );
            });
        });
    }
}

/// The keywords in tree order with their depths: each keyword's children
/// follow it, siblings in name order.
fn tree(keywords: &[KeywordInfo]) -> Vec<(usize, usize)> {
    let known: HashSet<KeywordId> = keywords.iter().map(|keyword| keyword.id).collect();
    let mut children: HashMap<Option<KeywordId>, Vec<usize>> = HashMap::new();
    for (index, keyword) in keywords.iter().enumerate() {
        let parent = keyword.parent.filter(|parent| known.contains(parent));
        children.entry(parent).or_default().push(index);
    }
    let mut order = Vec::with_capacity(keywords.len());
    let mut seen = HashSet::new();
    let mut stack: Vec<(usize, usize)> = children
        .get(&None)
        .map(|roots| roots.iter().rev().map(|index| (*index, 0)).collect())
        .unwrap_or_default();
    while let Some((index, depth)) = stack.pop() {
        if !seen.insert(index) {
            continue;
        }
        order.push((index, depth));
        if let Some(below) = children.get(&Some(keywords[index].id)) {
            stack.extend(below.iter().rev().map(|child| (*child, depth + 1)));
        }
    }
    order
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keywords_nest_under_their_parents() {
        let keyword = |id: i64, parent: Option<i64>, name: &str| KeywordInfo {
            id: KeywordId(id),
            parent: parent.map(KeywordId),
            name: name.to_owned(),
            photo_count: 0,
        };
        let keywords = [
            keyword(1, None, "Animals"),
            keyword(2, Some(1), "Birds"),
            keyword(3, None, "Places"),
            keyword(4, Some(3), "Iceland"),
        ];
        let order: Vec<(&str, usize)> = tree(&keywords)
            .into_iter()
            .map(|(index, depth)| (keywords[index].name.as_str(), depth))
            .collect();
        assert_eq!(
            order,
            [("Animals", 0), ("Birds", 1), ("Places", 0), ("Iceland", 1)]
        );
    }
}
