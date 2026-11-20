//! Reading properties out of a packet, whichever form they were written in.

use std::collections::HashMap;

use super::scope::Scope;
use super::value::{Holder, ValueAt};
use super::{Container, DEFAULT_LANGUAGE, Item, LANGUAGE, Namespace, Packet, Property, RDF};
use crate::dom::{Element, Node};

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
            for attribute in description.attributes() {
                if let Some((prefix, name)) = attribute.name.split_once(':')
                    && scope.uri(prefix) == Some(namespace.uri)
                {
                    let value = || attribute.value.clone();
                    values.entry(name.to_owned()).or_insert_with(value);
                }
            }
            for (_, child) in description.elements() {
                let (prefix, name) = child.name().split_once(':').unwrap_or(("", child.name()));
                let scope = scope.enter(child);
                if scope.uri(prefix) == Some(namespace.uri)
                    && let Value::Text(text) = element_value(child, &scope)
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

/// What a property element, or an item of a list, holds: its own content,
/// or the `rdf:value` beside its qualifiers. `scope` is the element's own.
fn element_value(element: &Element, scope: &Scope<'_>) -> Value {
    match Holder::of(element, scope) {
        Holder::Content => content_value(element, scope),
        Holder::Structure => Value::Other,
        Holder::Qualified {
            resource: None,
            value,
        } => qualified_value(element, scope, value),
        Holder::Qualified {
            resource: Some(position),
            value,
        } => match element.children.get(position) {
            Some(Node::Element(resource)) => {
                qualified_value(resource, &scope.enter(resource), value)
            }
            _ => Value::Other,
        },
    }
}

/// What a resource's `rdf:value` holds. `scope` is the resource's own.
fn qualified_value(resource: &Element, scope: &Scope<'_>, value: ValueAt) -> Value {
    match value {
        ValueAt::Attribute(index) => resource
            .attributes()
            .get(index)
            .map_or(Value::Other, |attribute| {
                Value::Text(attribute.value.clone())
            }),
        ValueAt::Element(position) => match resource.children.get(position) {
            Some(Node::Element(element)) => content_value(element, &scope.enter(element)),
            _ => Value::Other,
        },
    }
}

/// What an element holds as its own content: a list, or plain text.
/// `scope` is the element's own.
fn content_value(element: &Element, scope: &Scope<'_>) -> Value {
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

/// The items of a list that hold plain text, qualified or not, each with its
/// position among the list's children. `scope` is that of the element
/// around the list.
pub(super) fn list_items(list: &Element, scope: &Scope<'_>) -> Vec<(usize, Item)> {
    let scope = scope.enter(list);
    let items = list.elements().filter_map(|(position, element)| {
        let scope = scope.enter(element);
        if !scope.element_is(element.name(), RDF, "li") {
            return None;
        }
        let Value::Text(text) = element_value(element, &scope) else {
            return None;
        };
        let language = element.attribute(LANGUAGE).map(str::to_owned);
        Some((position, Item { language, text }))
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
