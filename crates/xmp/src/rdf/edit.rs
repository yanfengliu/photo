//! Changing the properties of a packet in place.
//!
//! Every change is planned against the packet as it is (which places hold
//! the property, which prefix names its namespace) and then carried out by
//! position, so nothing but the property concerned is touched. A value with
//! qualifiers is changed in its `rdf:value`, and the qualifiers stay.

use std::collections::BTreeMap;

use super::scope::Scope;
use super::value::{Holder, clear_for_value};
use super::{Container, Form, Item, LANGUAGE, Namespace, Packet, Property, RDF};
use crate::dom::{Element, Node};

/// A place a property was found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Location {
    /// Position of the description among the children of `rdf:RDF`.
    pub(super) description: usize,
    pub(super) site: Site,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Site {
    /// Position among the description's attributes.
    Attribute(usize),
    /// Position among the description's children.
    Child(usize),
}

impl Packet {
    /// Gives a property a plain value. Where the property exists it keeps
    /// its place and form; where it does not, `form` decides how it is added.
    pub(crate) fn set_text(&mut self, property: Property<'_>, text: &str, form: Form) {
        let locations = self.locate(property);
        let Some((first, duplicates)) = locations.split_first() else {
            let home = self.home(property.namespace);
            let name = self.qualified(home, property);
            if let Some(description) = self.rdf.element_mut(home) {
                match form {
                    Form::Attribute => description.set_attribute(&name, text),
                    Form::Element => {
                        description.push_element(Element::with_text(name, text));
                    }
                }
            }
            return;
        };
        self.remove_at(duplicates);
        let holder = match first.site {
            Site::Attribute(_) => Holder::Content,
            Site::Child(child) => self.holder_at(first.description, child),
        };
        let Some(description) = self.rdf.element_mut(first.description) else {
            return;
        };
        match first.site {
            Site::Attribute(index) => description.set_attribute_at(index, text),
            Site::Child(index) => {
                if let Some(element) = description.element_mut(index) {
                    holder.set_text(element, text);
                }
            }
        }
    }

    /// Gives a property a list of items, in the element that holds it now
    /// or in a new one.
    pub(crate) fn set_list(
        &mut self,
        property: Property<'_>,
        container: Container,
        items: &[Item],
    ) {
        // A list needs an element: the first one the property has is kept,
        // every other copy (attributes included) goes. Whatever comes before
        // the first element in document order is an attribute, so removing
        // the rest does not move it.
        let mut others = self.locate(property);
        let target = others
            .iter()
            .position(|location| matches!(location.site, Site::Child(_)))
            .map(|position| others.remove(position));
        self.remove_at(&others);
        match target {
            Some(Location {
                description,
                site: Site::Child(child),
            }) => self.fill_list(description, child, container, items),
            _ => self.add_list(property, container, items),
        }
    }

    /// Puts a new list in the property element at `child` of the description
    /// at `description`: in its `rdf:value` when the value has qualifiers
    /// beside it, so that they stay. A qualified value held in an attribute
    /// has no room for a list, and a structure no value to replace, so there
    /// the property's content is replaced whole.
    fn fill_list(
        &mut self,
        description: usize,
        child: usize,
        container: Container,
        items: &[Item],
    ) {
        let holder = self.holder_at(description, child);
        let path = holder.content_path().unwrap_or_default();
        // The property, or something inside it, may bind the usual prefix to
        // another namespace: the list is named in the scope it goes into.
        let (rdf, declare) = self.prefix_at(description, child, &path, RDF);
        let list = build_list(&rdf, declare, container, items);
        let property = self.rdf.element_mut(description);
        let property = property.and_then(|parent| parent.element_mut(child));
        if let Some(element) = property.and_then(|property| descend_mut(property, &path)) {
            clear_for_value(element);
            element.set_content(list);
        }
    }

    /// Adds a property holding a new list.
    fn add_list(&mut self, property: Property<'_>, container: Container, items: &[Item]) {
        let description = self.home(property.namespace);
        let list = build_list(&self.prefix_in(description, RDF), false, container, items);
        let mut element = Element::new(self.qualified(description, property));
        element.set_content(list);
        if let Some(parent) = self.rdf.element_mut(description) {
            parent.push_element(element);
        }
    }

    /// Removes a property wherever it appears.
    pub(crate) fn remove(&mut self, property: Property<'_>) {
        let locations = self.locate(property);
        self.remove_at(&locations);
    }

    /// Every place a property appears, in document order (within one
    /// description, attributes before child elements).
    pub(super) fn locate(&self, property: Property<'_>) -> Vec<Location> {
        let Property { namespace, name } = property;
        let mut locations = Vec::new();
        self.each_description(|description, element, scope| {
            let is_attribute = |qualified: &str| scope.attribute_is(qualified, namespace, name);
            let is_element =
                |child: &Element| scope.enter(child).element_is(child.name(), namespace, name);
            let attributes = element.attributes().iter().enumerate();
            let attributes = attributes
                .filter(|(_, attribute)| is_attribute(&attribute.name))
                .map(|(index, _)| Site::Attribute(index));
            let children = element
                .elements()
                .filter(|(_, child)| is_element(child))
                .map(|(index, _)| Site::Child(index));
            let sites = attributes.chain(children);
            locations.extend(sites.map(|site| Location { description, site }));
        });
        locations
    }

    /// Removes the given places, description by description, each in one
    /// pass: a property present many times costs no more to remove than its
    /// description costs to read.
    fn remove_at(&mut self, locations: &[Location]) {
        let mut places: BTreeMap<usize, (Vec<usize>, Vec<usize>)> = BTreeMap::new();
        for location in locations {
            let (attributes, children) = places.entry(location.description).or_default();
            match location.site {
                Site::Attribute(index) => attributes.push(index),
                Site::Child(index) => children.push(index),
            }
        }
        for (description, (attributes, children)) in places {
            if let Some(description) = self.rdf.element_mut(description) {
                description.remove_attributes(&attributes);
                description.remove_children(&children);
            }
        }
    }

    /// How the property element at `child` of the description at
    /// `description` keeps its value.
    fn holder_at(&self, description: usize, child: usize) -> Holder {
        let mut holder = Holder::Content;
        self.each_description(|position, element, scope| {
            if position == description
                && let Some(Node::Element(property)) = element.children.get(child)
            {
                holder = Holder::of(property, &scope.enter(property));
            }
        });
        holder
    }

    /// The element `path` leads to, as positions among children from those
    /// of `rdf:RDF`.
    pub(super) fn element_at_mut(&mut self, path: &[usize]) -> Option<&mut Element> {
        let (first, rest) = path.split_first()?;
        descend_mut(self.rdf.element_mut(*first)?, rest)
    }

    /// Position of the description a new property in `namespace` is added
    /// to: the first that already has a prefix for the namespace (some
    /// applications keep a description per namespace), else the first of
    /// all, else one created for the purpose.
    fn home(&mut self, namespace: Namespace) -> usize {
        let mut first = None;
        let mut familiar = None;
        self.each_description(|index, _, scope| {
            first.get_or_insert(index);
            if familiar.is_none() && scope.prefix_of(namespace.uri).is_some() {
                familiar = Some(index);
            }
        });
        if let Some(index) = familiar.or(first) {
            return index;
        }
        let bound = self.in_rdf_scope(|scope| scope.prefix_of(RDF.uri).map(str::to_owned));
        let prefix = bound.as_deref().unwrap_or(RDF.prefix);
        let mut description = Element::new(format!("{prefix}:Description"));
        if bound.is_none() {
            // `rdf:RDF` itself is named through the default namespace.
            description.set_attribute(&format!("xmlns:{prefix}"), RDF.uri);
        }
        description.set_attribute(&format!("{prefix}:about"), "");
        self.rdf.push_element(description)
    }

    /// A prefix that stands for `namespace` inside the description at
    /// `index`, declared there if the document has none.
    fn prefix_in(&mut self, index: usize, namespace: Namespace) -> String {
        let mut bound = None;
        let mut unused = namespace.prefix.to_owned();
        self.each_description(|position, _, scope| {
            if position != index {
                return;
            }
            bound = scope.prefix_of(namespace.uri).map(str::to_owned);
            unused = unused_prefix(scope, namespace);
        });
        if let Some(prefix) = bound {
            return prefix;
        }
        if let Some(description) = self.rdf.element_mut(index) {
            // Beside the other declarations, or else after the leading `rdf:about`.
            let attributes = description.attributes();
            let position = attributes
                .iter()
                .rposition(|attribute| attribute.declared_prefix().is_some())
                .map_or(attributes.len().min(1), |last| last + 1);
            description.insert_attribute(position, &format!("xmlns:{unused}"), namespace.uri);
        }
        unused
    }

    /// A prefix for `namespace` in force at the element `path` leads to from
    /// the property element at `child` of the description at `description`,
    /// and whether it still has to be declared: when none stands for the
    /// namespace there, one that means nothing there yet.
    fn prefix_at(
        &self,
        description: usize,
        child: usize,
        path: &[usize],
        namespace: Namespace,
    ) -> (String, bool) {
        let mut found = None;
        self.each_description(|position, element, scope| {
            if position != description {
                return;
            }
            let Some(Node::Element(property)) = element.children.get(child) else {
                return;
            };
            let scope = scope.enter(property);
            found = at_path(property, &scope, path, |_, scope| {
                match scope.prefix_of(namespace.uri) {
                    Some(prefix) => (prefix.to_owned(), false),
                    None => (unused_prefix(scope, namespace), true),
                }
            });
        });
        found.unwrap_or_else(|| (namespace.prefix.to_owned(), true))
    }

    /// The qualified name for `property` inside the description at `index`.
    fn qualified(&mut self, index: usize, property: Property<'_>) -> String {
        let prefix = self.prefix_in(index, property.namespace);
        format!("{prefix}:{}", property.name)
    }
}

/// The namespace's usual prefix, or that prefix numbered, whichever means
/// nothing in `scope` yet.
fn unused_prefix(scope: &Scope<'_>, namespace: Namespace) -> String {
    let mut unused = namespace.prefix.to_owned();
    let mut suffix = 0_u32;
    while scope.uri(&unused).is_some() {
        suffix += 1;
        unused = format!("{}{suffix}", namespace.prefix);
    }
    unused
}

/// What `visit` makes of the element `path` leads to from `element`, and of
/// the scope in force there; `None` when the path leads to no element.
/// `scope` is `element`'s own.
pub(super) fn at_path<T>(
    element: &Element,
    scope: &Scope<'_>,
    path: &[usize],
    visit: impl FnOnce(&Element, &Scope<'_>) -> T,
) -> Option<T> {
    match path.split_first() {
        None => Some(visit(element, scope)),
        Some((position, rest)) => match element.children.get(*position) {
            Some(Node::Element(child)) => at_path(child, &scope.enter(child), rest, visit),
            _ => None,
        },
    }
}

/// The element `path` leads to from `element`.
fn descend_mut<'e>(element: &'e mut Element, path: &[usize]) -> Option<&'e mut Element> {
    path.iter()
        .try_fold(element, |element, position| element.element_mut(*position))
}

/// A list of `items` named with the RDF prefix `rdf`, which the list
/// declares itself when `declare` says so.
fn build_list(rdf: &str, declare: bool, container: Container, items: &[Item]) -> Element {
    let mut list = Element::new(format!("{rdf}:{}", container.name()));
    if declare {
        list.set_attribute(&format!("xmlns:{rdf}"), RDF.uri);
    }
    for item in items {
        let mut element = Element::with_text(format!("{rdf}:li"), &item.text);
        if let Some(language) = &item.language {
            element.set_attribute(LANGUAGE, language);
        }
        list.children.push(Node::Element(element));
    }
    list
}
