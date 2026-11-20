//! Namespace resolution: which URI a prefix stands for at a given element.

use super::Namespace;
use crate::dom::Element;

/// The namespace declarations in force at one element: its own, then those
/// of the elements around it.
///
/// An element's declarations are picked out once, when its scope is made,
/// so that looking a prefix up does not depend on how many other attributes
/// the element carries.
pub(super) struct Scope<'a> {
    /// What the element itself declares: each prefix (empty for the default
    /// namespace) with the URI it is bound to.
    declared: Vec<(&'a str, &'a str)>,
    outer: Option<&'a Scope<'a>>,
}

impl<'a> Scope<'a> {
    /// The scope of an element with nothing around it.
    pub(super) fn of(element: &'a Element) -> Self {
        Self {
            declared: declarations(element),
            outer: None,
        }
    }

    /// The scope of `element`, a descendant of this scope's element.
    pub(super) fn enter<'b>(&'b self, element: &'b Element) -> Scope<'b>
    where
        'a: 'b,
    {
        Scope {
            declared: declarations(element),
            outer: Some(self),
        }
    }

    /// The namespace `prefix` stands for here; an empty prefix asks for the
    /// default namespace.
    pub(super) fn uri(&self, prefix: &str) -> Option<&'a str> {
        match self
            .declared
            .iter()
            .find(|(declared, _)| *declared == prefix)
        {
            // An empty value takes the binding away again.
            Some((_, uri)) => Some(*uri).filter(|uri| !uri.is_empty()),
            None => self.outer.and_then(|outer| outer.uri(prefix)),
        }
    }

    /// The prefixes that stand for `uri` here, innermost first.
    pub(super) fn prefixes_of(&self, uri: &str) -> Vec<&'a str> {
        let mut prefixes = Vec::new();
        let mut scope = Some(self);
        while let Some(current) = scope {
            for &(prefix, bound) in &current.declared {
                // A prefix an inner element has bound to something else no
                // longer stands for this namespace.
                let stands = !prefix.is_empty() && bound == uri && self.uri(prefix) == Some(uri);
                if stands && !prefixes.contains(&prefix) {
                    prefixes.push(prefix);
                }
            }
            scope = current.outer;
        }
        prefixes
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

        assert_eq!(outside.prefixes_of("urn:one"), ["p", "q"]);
        assert_eq!(
            inside.prefixes_of("urn:one"),
            ["q"],
            "p now means something else"
        );
        assert_eq!(
            inside.prefixes_of("urn:two"),
            ["p"],
            "the default namespace has no prefix"
        );
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
