//! Where a property keeps its value.
//!
//! Usually that is the property element's own content. A value that carries
//! qualifiers sits instead in the `rdf:value` of a resource, beside the
//! qualifiers, in one of the forms the XMP specification (part 1, under
//! "Qualifiers") allows:
//!
//! - `<xmp:Rating><rdf:Description><rdf:value>4</rdf:value><q:source>…</q:source></rdf:Description></xmp:Rating>`
//! - `<xmp:Rating rdf:parseType="Resource"><rdf:value>4</rdf:value><q:source>…</q:source></xmp:Rating>`
//! - `<xmp:Rating rdf:value="4" q:source="…"/>`, or those attributes on an
//!   `rdf:Description` inside the property.
//!
//! An item of a list may take the same forms. Changing a plain value, or the
//! text of one language of a title, changes only its `rdf:value`. A list
//! that changes is written afresh, so its items lose what qualifiers they
//! had; the qualifiers of the list as a whole stay.

use super::scope::Scope;
use super::{LANGUAGE, RDF};
use crate::dom::{Element, Node};

/// Where an element, a property or an item of a list, keeps its value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Holder {
    /// In its own content: text, or a list.
    Content,
    /// In the `rdf:value` of a resource, beside qualifiers. The resource is
    /// the element itself when `resource` is `None`, else the
    /// `rdf:Description` at that position among its children.
    Qualified {
        resource: Option<usize>,
        value: ValueAt,
    },
    /// It holds a structure without `rdf:value`: no value at all.
    Structure,
}

/// Where in a resource its `rdf:value` is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ValueAt {
    /// The attribute at this position.
    Attribute(usize),
    /// The child at this position.
    Element(usize),
}

impl Holder {
    /// How `element` keeps its value. `scope` is the element's own.
    pub(super) fn of(element: &Element, scope: &Scope<'_>) -> Self {
        if let Some(index) = value_attribute(element, scope) {
            let value = ValueAt::Attribute(index);
            return Self::Qualified {
                resource: None,
                value,
            };
        }
        if is_resource(element, scope) {
            return value_element(element, scope).map_or(Self::Structure, |position| {
                let value = ValueAt::Element(position);
                Self::Qualified {
                    resource: None,
                    value,
                }
            });
        }
        let Some((position, description)) = lone_description(element, scope) else {
            return Self::Content;
        };
        let scope = scope.enter(description);
        let attribute = value_attribute(description, &scope).map(ValueAt::Attribute);
        let value = attribute.or_else(|| value_element(description, &scope).map(ValueAt::Element));
        value.map_or(Self::Structure, |value| Self::Qualified {
            resource: Some(position),
            value,
        })
    }

    /// The way from the element to the one whose content is its value, as
    /// positions among children: empty when that is the element itself,
    /// `None` when the value is an attribute or there is no value.
    pub(super) fn content_path(self) -> Option<Vec<usize>> {
        match self {
            Self::Content => Some(Vec::new()),
            Self::Structure
            | Self::Qualified {
                value: ValueAt::Attribute(_),
                ..
            } => None,
            Self::Qualified {
                resource,
                value: ValueAt::Element(position),
            } => Some(resource.into_iter().chain([position]).collect()),
        }
    }

    /// Gives `element`, the one this was found in, the plain value `text`:
    /// in `rdf:value` when the value is qualified, which keeps the
    /// qualifiers, and otherwise as its whole content.
    pub(super) fn set_text(self, element: &mut Element, text: &str) {
        let Self::Qualified { resource, value } = self else {
            give_text(element, text);
            return;
        };
        let resource = match resource {
            None => Some(element),
            Some(position) => element.element_mut(position),
        };
        let Some(resource) = resource else {
            return;
        };
        match value {
            ValueAt::Attribute(index) => resource.set_attribute_at(index, text),
            ValueAt::Element(position) => {
                if let Some(value) = resource.element_mut(position) {
                    give_text(value, text);
                }
            }
        }
    }
}

/// Strips an element, before it is given a new value, of the attributes that
/// could contradict that value: all but its namespace declarations and its
/// language.
pub(super) fn clear_for_value(element: &mut Element) {
    element.retain_attributes(|attribute| {
        attribute.declared_prefix().is_some() || attribute.name == LANGUAGE
    });
}

fn give_text(element: &mut Element, text: &str) {
    clear_for_value(element);
    element.set_text(text);
}

fn value_attribute(element: &Element, scope: &Scope<'_>) -> Option<usize> {
    let mut attributes = element.attributes().iter();
    attributes.position(|attribute| scope.attribute_is(&attribute.name, RDF, "value"))
}

/// Whether `element` is itself a resource: `rdf:parseType="Resource"`.
fn is_resource(element: &Element, scope: &Scope<'_>) -> bool {
    element.attributes().iter().any(|attribute| {
        attribute.value == "Resource" && scope.attribute_is(&attribute.name, RDF, "parseType")
    })
}

fn value_element(element: &Element, scope: &Scope<'_>) -> Option<usize> {
    element
        .elements()
        .find(|(_, child)| scope.enter(child).element_is(child.name(), RDF, "value"))
        .map(|(position, _)| position)
}

/// The `rdf:Description` that is all `element` holds, with its position.
fn lone_description<'e>(element: &'e Element, scope: &Scope<'_>) -> Option<(usize, &'e Element)> {
    let mut found = None;
    for (position, node) in element.children.iter().enumerate() {
        match node {
            Node::Element(child)
                if found.is_none()
                    && scope
                        .enter(child)
                        .element_is(child.name(), RDF, "Description") =>
            {
                found = Some((position, child));
            }
            Node::Element(_) | Node::CData(_) => return None,
            Node::Text(_) if !node.is_blank() => return None,
            Node::Text(_) | Node::Comment(_) | Node::Instruction(_) | Node::DocType(_) => {}
        }
    }
    found
}
