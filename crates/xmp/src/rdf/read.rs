//! Reading properties out of a packet, whichever form they were written in.

use std::collections::HashMap;

use super::scope::Scope;
use super::{Container, DEFAULT_LANGUAGE, Item, LANGUAGE, Namespace, Packet, Property, RDF};
use crate::dom::Element;

/// The plain-valued properties of one namespace, by local name.
pub(crate) type Values = HashMap<String, String>;

/// What a property holds.
enum Value {
    Text(String),
    List(Vec<Item>),
    /// A structure this crate does not interpret.
    Other,
}

impl Packet {
    /// A property's plain value, from an attribute or a text-only element.
    pub(crate) fn text(&self, property: Property<'_>) -> Option<String> {
        match self.value(property)? {
            Value::Text(text) => Some(text),
            Value::List(_) | Value::Other => None,
        }
    }

    /// A property's items. A plain value counts as a list of one, which is
    /// how some applications write a single keyword.
    pub(crate) fn items(&self, property: Property<'_>) -> Option<Vec<Item>> {
        Some(match self.value(property)? {
            Value::List(items) => items,
            Value::Text(text) => vec![Item::plain(text)],
            Value::Other => Vec::new(),
        })
    }

    /// The texts of a property's items.
    pub(crate) fn list(&self, property: Property<'_>) -> Option<Vec<String>> {
        let items = self.items(property)?;
        Some(items.into_iter().map(|item| item.text).collect())
    }

    /// The text of a language alternative: the default language if there is
    /// one, else the first.
    pub(crate) fn alternative(&self, property: Property<'_>) -> Option<String> {
        let mut items = self.items(property)?;
        let position = default_position(&items)?;
        Some(items.swap_remove(position).text)
    }

    /// Every plain-valued property in `namespace`: those written as an
    /// attribute or as a text-only element.
    pub(crate) fn plain_values(&self, namespace: Namespace) -> Values {
        let mut values = Values::new();
        self.each_description(|_, description, scope| {
            let prefixes = scope.prefixes_of(namespace.uri);
            for attribute in description.attributes() {
                if let Some((prefix, name)) = attribute.name.split_once(':')
                    && prefixes.contains(&prefix)
                {
                    let value = || attribute.value.clone();
                    values.entry(name.to_owned()).or_insert_with(value);
                }
            }
            for (_, child) in description.elements() {
                let (prefix, name) = child.name().split_once(':').unwrap_or(("", child.name()));
                if scope.enter(child).uri(prefix) == Some(namespace.uri)
                    && let Some(text) = child.text()
                {
                    values.entry(name.to_owned()).or_insert(text);
                }
            }
        });
        values
    }

    /// What the property holds in the first place it appears: within a
    /// description, an attribute comes before a child element.
    fn value(&self, property: Property<'_>) -> Option<Value> {
        let Property { namespace, name } = property;
        let mut value = None;
        self.each_description(|_, description, scope| {
            if value.is_some() {
                return;
            }
            let is_attribute = |qualified: &str| scope.attribute_is(qualified, namespace, name);
            let is_element =
                |child: &Element| scope.enter(child).element_is(child.name(), namespace, name);
            let mut attributes = description.attributes().iter();
            value = match attributes.find(|attribute| is_attribute(&attribute.name)) {
                Some(attribute) => Some(Value::Text(attribute.value.clone())),
                None => description
                    .elements()
                    .find(|(_, child)| is_element(child))
                    .map(|(_, child)| element_value(child, &scope.enter(child))),
            };
        });
        value
    }
}

/// What a property element holds. `scope` is the element's own.
fn element_value(element: &Element, scope: &Scope<'_>) -> Value {
    match container_in(element, scope) {
        Some((_, list)) => {
            let items = list_items(list, scope);
            Value::List(items.into_iter().map(|(_, item)| item).collect())
        }
        None => element.text().map_or(Value::Other, Value::Text),
    }
}

/// The list directly inside a property element, with its position. `scope`
/// is the property element's.
pub(super) fn container_in<'e>(
    element: &'e Element,
    scope: &Scope<'_>,
) -> Option<(usize, &'e Element)> {
    element.elements().find(|(_, child)| {
        let scope = scope.enter(child);
        let is_container =
            |container: &Container| scope.element_is(child.name(), RDF, container.name());
        Container::ALL.iter().any(is_container)
    })
}

/// The items of a list that hold plain text, each with its position among
/// the list's children. `scope` is that of the property element around the
/// list.
pub(super) fn list_items(list: &Element, scope: &Scope<'_>) -> Vec<(usize, Item)> {
    let scope = scope.enter(list);
    let items = list.elements().filter_map(|(position, element)| {
        if !scope.enter(element).element_is(element.name(), RDF, "li") {
            return None;
        }
        let item = Item {
            language: element.attribute(LANGUAGE).map(str::to_owned),
            text: element.text()?,
        };
        Some((position, item))
    });
    items.collect()
}

/// Which item stands for a language alternative: the default language if
/// present, else the first.
pub(super) fn default_position(items: &[Item]) -> Option<usize> {
    let is_default = |item: &Item| item.language.as_deref() == Some(DEFAULT_LANGUAGE);
    let first = if items.is_empty() { None } else { Some(0) };
    items.iter().position(is_default).or(first)
}
