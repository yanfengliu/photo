//! Changing the properties of a packet in place.
//!
//! Every change is planned against the packet as it is (which places hold
//! the property, which prefix names its namespace) and then carried out by
//! position, so nothing but the property concerned is touched.

use super::read::{container_in, default_position, list_items};
use super::{Container, DEFAULT_LANGUAGE, Form, Item, LANGUAGE, Namespace, Packet, Property, RDF};
use crate::dom::{Element, Node};

/// A place a property was found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Location {
    /// Position of the description among the children of `rdf:RDF`.
    description: usize,
    site: Site,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Site {
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
        let Some(description) = self.rdf.element_mut(first.description) else {
            return;
        };
        match first.site {
            Site::Attribute(index) => description.set_attribute_at(index, text),
            Site::Child(index) => {
                if let Some(element) = description.element_mut(index) {
                    keep_only_declarations(element);
                    element.set_text(text);
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
        let description = match target {
            Some(location) => location.description,
            None => self.home(property.namespace),
        };
        let list = build_list(&self.prefix_in(description, RDF), container, items);
        if let Some(Location {
            site: Site::Child(child),
            ..
        }) = target
        {
            let parent = self.rdf.element_mut(description);
            if let Some(element) = parent.and_then(|parent| parent.element_mut(child)) {
                keep_only_declarations(element);
                element.set_content(list);
            }
        } else {
            let mut element = Element::new(self.qualified(description, property));
            element.set_content(list);
            if let Some(parent) = self.rdf.element_mut(description) {
                parent.push_element(element);
            }
        }
    }

    /// Sets the text [`Self::alternative`] reads, keeping the other languages.
    pub(crate) fn set_alternative(&mut self, property: Property<'_>, text: &str) {
        let path = self.default_item(property);
        let item = path.and_then(|[description, child, list, item]| {
            let holder = self.rdf.element_mut(description)?.element_mut(child)?;
            holder.element_mut(list)?.element_mut(item)
        });
        match item {
            Some(item) => item.set_text(text),
            None => {
                let item = Item {
                    language: Some(DEFAULT_LANGUAGE.to_owned()),
                    text: text.to_owned(),
                };
                self.set_list(property, Container::Alt, &[item]);
            }
        }
    }

    /// Removes a property wherever it appears.
    pub(crate) fn remove(&mut self, property: Property<'_>) {
        let locations = self.locate(property);
        self.remove_at(&locations);
    }

    /// Every place a property appears, in document order (within one
    /// description, attributes before child elements).
    fn locate(&self, property: Property<'_>) -> Vec<Location> {
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

    /// Removes the given places. They must be in document order, as
    /// [`Self::locate`] returns them: going backwards keeps the positions
    /// still to be visited valid.
    fn remove_at(&mut self, locations: &[Location]) {
        for location in locations.iter().rev() {
            let Some(description) = self.rdf.element_mut(location.description) else {
                continue;
            };
            match location.site {
                Site::Attribute(index) => description.remove_attribute(index),
                Site::Child(index) => description.remove_child(index),
            }
        }
    }

    /// Positions leading to the item [`Self::alternative`] reads, when the
    /// property is written as a list: description, property element, list,
    /// item.
    fn default_item(&self, property: Property<'_>) -> Option<[usize; 4]> {
        let first = *self.locate(property).first()?;
        let Site::Child(child) = first.site else {
            return None;
        };
        let mut path = None;
        self.each_description(|description, element, scope| {
            if description != first.description {
                return;
            }
            let Some(Node::Element(holder)) = element.children.get(child) else {
                return;
            };
            let scope = scope.enter(holder);
            let Some((list_index, list)) = container_in(holder, &scope) else {
                return;
            };
            let (positions, items): (Vec<usize>, Vec<Item>) =
                list_items(list, &scope).into_iter().unzip();
            let chosen = default_position(&items).and_then(|item| positions.get(item));
            path = chosen.map(|item_index| [description, child, list_index, *item_index]);
        });
        path
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
            if familiar.is_none() && !scope.prefixes_of(namespace.uri).is_empty() {
                familiar = Some(index);
            }
        });
        if let Some(index) = familiar.or(first) {
            return index;
        }
        let bound = self.in_rdf_scope(|scope| first_prefix(scope.prefixes_of(RDF.uri)));
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
            bound = first_prefix(scope.prefixes_of(namespace.uri));
            // The usual prefix may already mean something else here.
            let mut suffix = 0_u32;
            while scope.uri(&unused).is_some() {
                suffix += 1;
                unused = format!("{}{suffix}", namespace.prefix);
            }
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

    /// The qualified name for `property` inside the description at `index`.
    fn qualified(&mut self, index: usize, property: Property<'_>) -> String {
        let prefix = self.prefix_in(index, property.namespace);
        format!("{prefix}:{}", property.name)
    }
}

fn first_prefix(prefixes: Vec<&str>) -> Option<String> {
    prefixes.first().map(|prefix| (*prefix).to_owned())
}

fn build_list(rdf: &str, container: Container, items: &[Item]) -> Element {
    let mut list = Element::new(format!("{rdf}:{}", container.name()));
    for item in items {
        let mut element = Element::with_text(format!("{rdf}:li"), &item.text);
        if let Some(language) = &item.language {
            element.set_attribute(LANGUAGE, language);
        }
        list.children.push(Node::Element(element));
    }
    list
}

/// Strips a property element of everything but its namespace declarations
/// before it is given a new value.
fn keep_only_declarations(element: &mut Element) {
    element.retain_attributes(|attribute| attribute.declared_prefix().is_some());
}
