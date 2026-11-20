//! Text in several languages: an `rdf:Alt` whose items carry `xml:lang`.
//!
//! [`Packet::alternative`] reads the default language (`x-default`), else
//! the first. Changing the text changes the default item and nothing else;
//! clearing it leaves the other languages as they are.

use super::edit::{Site, at_path};
use super::read::{container_in, list_items};
use super::value::Holder;
use super::{Container, DEFAULT_LANGUAGE, Item, LANGUAGE, Packet, Property};
use crate::dom::{Element, Node};

/// A language alternative as it stands in a packet.
struct Alternative {
    /// The way to the list, as positions among children from those of
    /// `rdf:RDF`.
    list: Vec<usize>,
    /// The name a new item of the list takes: the list's own prefix, `li`.
    item_name: String,
    /// The way to the item in the default language, and how it keeps its
    /// text.
    default: Option<(Vec<usize>, Holder)>,
    /// Whether the list holds text in another language too.
    others: bool,
}

impl Packet {
    /// Sets the text of the default language, keeping the other languages
    /// and any qualifiers of the item. Without an item in the default
    /// language, one is added in front of the others; without a list, the
    /// property becomes one.
    pub(crate) fn set_alternative(&mut self, property: Property<'_>, text: &str) {
        let Some(alternative) = self.find_alternative(property) else {
            let item = Item {
                language: Some(DEFAULT_LANGUAGE.to_owned()),
                text: text.to_owned(),
            };
            self.set_list(property, Container::Alt, &[item]);
            return;
        };
        match alternative.default {
            Some((path, holder)) => {
                if let Some(item) = self.element_at_mut(&path) {
                    holder.set_text(item, text);
                }
            }
            None => {
                let mut item = Element::with_text(alternative.item_name, text);
                item.set_attribute(LANGUAGE, DEFAULT_LANGUAGE);
                if let Some(list) = self.element_at_mut(&alternative.list) {
                    list.insert_element(0, item);
                }
            }
        }
    }

    /// Takes the text of the default language away. Where the property
    /// holds other languages too they stay, beside a default that is empty
    /// so that the property reads as no text; otherwise the property goes.
    pub(crate) fn clear_alternative(&mut self, property: Property<'_>) {
        match self.find_alternative(property) {
            Some(alternative) if alternative.others => self.set_alternative(property, ""),
            _ => self.remove(property),
        }
    }

    /// The property as a language alternative, when it is held as a list.
    fn find_alternative(&self, property: Property<'_>) -> Option<Alternative> {
        let first = *self.locate(property).first()?;
        let Site::Child(child) = first.site else {
            return None;
        };
        let mut found = None;
        self.each_description(|description, element, scope| {
            if description != first.description {
                return;
            }
            let Some(Node::Element(property)) = element.children.get(child) else {
                return;
            };
            let scope = scope.enter(property);
            let Some(inner) = Holder::of(property, &scope).content_path() else {
                return;
            };
            let list = at_path(property, &scope, &inner, |holder, scope| {
                let (position, list) = container_in(holder, scope)?;
                let items = list_items(list, scope);
                let is_default = |item: &Item| item.language.as_deref() == Some(DEFAULT_LANGUAGE);
                let others = items.iter().any(|(_, item)| !is_default(item));
                let list_scope = scope.enter(list);
                let default = items.iter().find(|(_, item)| is_default(item));
                let default = default.and_then(|(index, _)| match list.children.get(*index) {
                    Some(Node::Element(item)) => {
                        Some((*index, Holder::of(item, &list_scope.enter(item))))
                    }
                    _ => None,
                });
                let item_name = match list.name().split_once(':') {
                    Some((prefix, _)) => format!("{prefix}:li"),
                    None => "li".to_owned(),
                };
                Some((position, item_name, default, others))
            });
            found = list
                .flatten()
                .map(|(position, item_name, default, others)| {
                    let mut list = vec![description, child];
                    list.extend(&inner);
                    list.push(position);
                    let default = default.map(|(index, holder)| {
                        let mut path = list.clone();
                        path.push(index);
                        (path, holder)
                    });
                    Alternative {
                        list,
                        item_name,
                        default,
                        others,
                    }
                });
        });
        found
    }
}
