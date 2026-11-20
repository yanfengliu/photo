//! Namespace resolution: which URI a prefix stands for at a given element.

use std::cell::{Cell, OnceCell};
use std::collections::HashMap;

use super::Namespace;
use crate::dom::Element;

/// How many lookups an element's declarations are looked through one by one
/// before they are indexed by prefix. A scope asked only a few questions
/// never pays for an index; one asked many pays for it once, after which a
/// lookup costs the same however many declarations there are.
const SCANS_BEFORE_INDEX: usize = 8;

/// The namespace declarations in force at one element: its own, then those
/// of the elements around it.
///
/// An element's declarations are picked out once, when its scope is made,
/// so that looking a prefix up does not depend on how many other attributes
/// the element carries, and, past a few lookups, not on how many
/// declarations either.
pub(super) struct Scope<'a> {
    /// What the element itself declares, in order: each prefix (empty for
    /// the default namespace) with the URI it is bound to.
    declared: Vec<(&'a str, &'a str)>,
    /// How many lookups have looked through `declared` one by one.
    scans: Cell<usize>,
    /// Positions in `declared` by prefix, once lookups have been many.
    index: OnceCell<HashMap<String, usize>>,
    outer: Option<&'a Scope<'a>>,
}

impl<'a> Scope<'a> {
    /// The scope of an element with nothing around it.
    pub(super) fn of(element: &'a Element) -> Self {
        Self::around(element, None)
    }

    /// The scope of `element`, a descendant of this scope's element.
    pub(super) fn enter<'b>(&'b self, element: &'b Element) -> Scope<'b>
    where
        'a: 'b,
    {
        Scope::around(element, Some(self))
    }

    fn around(element: &'a Element, outer: Option<&'a Scope<'a>>) -> Self {
        Self {
            declared: declarations(element),
            scans: Cell::new(0),
            index: OnceCell::new(),
            outer,
        }
    }

    /// What the element itself binds `prefix` to, if it declares it.
    fn own(&self, prefix: &str) -> Option<&'a str> {
        let indexed = self.index.get().or_else(|| {
            let scans = self.scans.get();
            self.scans.set(scans + 1);
            let worth_it = scans >= SCANS_BEFORE_INDEX && self.declared.len() > SCANS_BEFORE_INDEX;
            worth_it.then(|| {
                self.index.get_or_init(|| {
                    let positions = self.declared.iter().enumerate();
                    positions
                        .map(|(position, (declared, _))| ((*declared).to_owned(), position))
                        .collect()
                })
            })
        });
        let position = match indexed {
            Some(index) => index.get(prefix).copied(),
            None => self
                .declared
                .iter()
                .position(|(declared, _)| *declared == prefix),
        };
        position.map(|position| self.declared[position].1)
    }

    /// The namespace `prefix` stands for here; an empty prefix asks for the
    /// default namespace.
    pub(super) fn uri(&self, prefix: &str) -> Option<&'a str> {
        match self.own(prefix) {
            // An empty value takes the binding away again.
            Some(uri) => Some(uri).filter(|uri| !uri.is_empty()),
            None => self.outer.and_then(|outer| outer.uri(prefix)),
        }
    }

    /// A prefix that stands for `uri` here: the innermost declared, and the
    /// first of an element's declarations.
    pub(super) fn prefix_of(&self, uri: &str) -> Option<&'a str> {
        let mut scope = Some(self);
        while let Some(current) = scope {
            for &(prefix, bound) in &current.declared {
                // A prefix an inner element has bound to something else no
                // longer stands for this namespace.
                if !prefix.is_empty() && bound == uri && self.uri(prefix) == Some(uri) {
                    return Some(prefix);
                }
            }
            scope = current.outer;
        }
        None
    }

    /// Whether an element called `qualified` here is `local` in `namespace`.
    pub(super) fn element_is(&self, qualified: &str, namespace: Namespace, local: &str) -> bool {
        let (prefix, name) = qualified.split_once(':').unwrap_or(("", qualified));
        name == local && self.uri(prefix) == Some(namespace.uri)
    }

    /// Whether an attribute called `qualified` here is `local` in
    /// `namespace`. An attribute without a prefix is in no namespace.
    pub(super) fn attribute_is(&self, qualified: &str, namespace: Namespace, local: &str) -> bool {
        qualified
            .split_once(':')
            .is_some_and(|(prefix, name)| name == local && self.uri(prefix) == Some(namespace.uri))
    }
}

fn declarations(element: &Element) -> Vec<(&str, &str)> {
    let attributes = element.attributes().iter();
    attributes
        .filter_map(|attribute| Some((attribute.declared_prefix()?, attribute.value.as_str())))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dom::Document;

    const ONE: Namespace = Namespace {
        uri: "urn:one",
        prefix: "one",
    };
    const TWO: Namespace = Namespace {
        uri: "urn:two",
        prefix: "two",
    };

    fn root(xml: &str) -> Element {
        Document::parse(xml).unwrap().root
    }

    #[test]
    fn an_inner_declaration_hides_an_outer_one() {
        let outer = root(
            r#"<a xmlns:p="urn:one" xmlns:q="urn:one" xmlns="urn:two"><b xmlns:p="urn:two" xmlns=""/></a>"#,
        );
        let inner = outer.elements().next().unwrap().1;
        let outside = Scope::of(&outer);
        let inside = outside.enter(inner);

        assert_eq!(outside.uri("p"), Some("urn:one"));
        assert_eq!(inside.uri("p"), Some("urn:two"));
        assert_eq!(inside.uri("q"), Some("urn:one"));
        assert_eq!(inside.uri("r"), None);
        assert_eq!(outside.uri(""), Some("urn:two"));
        assert_eq!(
            inside.uri(""),
            None,
            "an empty declaration removes the default namespace"
        );

        assert_eq!(outside.prefix_of("urn:one"), Some("p"));
        assert_eq!(
            inside.prefix_of("urn:one"),
            Some("q"),
            "p now means something else"
        );
        assert_eq!(
            inside.prefix_of("urn:two"),
            Some("p"),
            "the default namespace has no prefix"
        );
        assert_eq!(inside.prefix_of("urn:three"), None);
    }

    #[test]
    fn many_declarations_are_resolved_like_a_few() {
        let mut xml = String::from("<a");
        for index in 0..40 {
            xml.push_str(&format!(r#" xmlns:p{index}="urn:{index}""#));
        }
        xml.push_str("><b");
        for index in (0..40).step_by(2) {
            xml.push_str(&format!(r#" xmlns:p{index}="urn:inner{index}""#));
        }
        xml.push_str(r#" xmlns:p1="" xmlns:q="urn:3"/></a>"#);
        let outer = root(&xml);
        let inner = outer.elements().next().unwrap().1;
        let outside = Scope::of(&outer);
        let inside = outside.enter(inner);

        // Twice over, so that the answers come from one-by-one lookups and
        // from the index alike.
        for _ in 0..2 {
            for index in 0..40 {
                let prefix = format!("p{index}");
                let own = format!("urn:{index}");
                let inner = format!("urn:inner{index}");
                assert_eq!(outside.uri(&prefix), Some(own.as_str()));
                let expected = match index {
                    1 => None,
                    _ if index % 2 == 0 => Some(inner.as_str()),
                    _ => Some(own.as_str()),
                };
                assert_eq!(inside.uri(&prefix), expected, "{prefix}");
            }
            assert_eq!(inside.uri("p40"), None);
            assert_eq!(inside.prefix_of("urn:3"), Some("q"));
            assert_eq!(inside.prefix_of("urn:5"), Some("p5"));
            assert_eq!(inside.prefix_of("urn:0"), None, "p0 means something else");
            assert_eq!(outside.prefix_of("urn:0"), Some("p0"));
        }
    }

    #[test]
    fn names_are_matched_by_namespace_not_by_prefix() {
        let element = root(r#"<a xmlns:p="urn:one" xmlns="urn:two"/>"#);
        let scope = Scope::of(&element);
        assert!(scope.element_is("p:thing", ONE, "thing"));
        assert!(scope.element_is("thing", TWO, "thing"));
        assert!(!scope.element_is("p:thing", TWO, "thing"));
        assert!(!scope.element_is("p:other", ONE, "thing"));
        assert!(!scope.element_is("x:thing", ONE, "thing"));

        assert!(scope.attribute_is("p:thing", ONE, "thing"));
        assert!(
            !scope.attribute_is("thing", TWO, "thing"),
            "the default namespace is not for attributes"
        );
        assert!(!scope.attribute_is("xmlns:p", ONE, "p"));
    }
}
