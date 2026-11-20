//! XMP properties on top of the DOM: where an RDF packet keeps them and the
//! forms they take.
//!
//! A property lives on one of the `rdf:Description` elements directly inside
//! `rdf:RDF`, as an attribute or as a child element, and a child element
//! holds either plain text or an `rdf:Bag`, `rdf:Seq` or `rdf:Alt` of
//! `rdf:li` items; a value with qualifiers holds the same in its
//! `rdf:value`. Names are matched by namespace URI, never by prefix, and a
//! property is read from the first place it appears.
//!
//! Setting a property changes it where it already is, in the form it already
//! has, and leaves a single copy. Only a property that does not exist yet is
//! added, to the first description, under whichever prefix the document
//! already binds to the namespace.

mod alternative;
mod edit;
mod read;
mod scope;
mod value;

pub(crate) use read::Values;
use scope::Scope;

use crate::XmpError;
use crate::dom::{Document, Element, Node};

/// A namespace and the prefix declared for it in a document that has none.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Namespace {
    uri: &'static str,
    prefix: &'static str,
}

impl Namespace {
    pub(crate) const fn property(self, name: &str) -> Property<'_> {
        Property {
            namespace: self,
            name,
        }
    }
}

const RDF: Namespace = Namespace {
    uri: "http://www.w3.org/1999/02/22-rdf-syntax-ns#",
    prefix: "rdf",
};
const META: Namespace = Namespace {
    uri: "adobe:ns:meta/",
    prefix: "x",
};
pub(crate) const XMP: Namespace = Namespace {
    uri: "http://ns.adobe.com/xap/1.0/",
    prefix: "xmp",
};
pub(crate) const DC: Namespace = Namespace {
    uri: "http://purl.org/dc/elements/1.1/",
    prefix: "dc",
};
pub(crate) const LR: Namespace = Namespace {
    uri: "http://ns.adobe.com/lightroom/1.0/",
    prefix: "lr",
};
pub(crate) const EXIF: Namespace = Namespace {
    uri: "http://ns.adobe.com/exif/1.0/",
    prefix: "exif",
};
pub(crate) const TIFF: Namespace = Namespace {
    uri: "http://ns.adobe.com/tiff/1.0/",
    prefix: "tiff",
};
pub(crate) const CRS: Namespace = Namespace {
    uri: "http://ns.adobe.com/camera-raw-settings/1.0/",
    prefix: "crs",
};
/// This application's own namespace.
pub(crate) const PHOTO: Namespace = Namespace {
    uri: crate::SETTINGS_NAMESPACE,
    prefix: "photo",
};

/// The identifier every XMP packet header carries.
const PACKET_ID: &str = "W5M0MpCehiHzreSzNTczkc9d";
/// How this crate names itself in the packets it creates.
const TOOLKIT: &str = concat!("Photo ", env!("CARGO_PKG_VERSION"));
/// The attribute that tells language alternatives apart.
const LANGUAGE: &str = "xml:lang";
/// The language of the alternative that stands for all the others.
const DEFAULT_LANGUAGE: &str = "x-default";

/// A property name: a namespace and a local name.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Property<'a> {
    namespace: Namespace,
    name: &'a str,
}

/// The three RDF list types.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Container {
    /// Unordered.
    Bag,
    /// Ordered.
    Seq,
    /// Alternatives, told apart by language.
    Alt,
}

impl Container {
    const ALL: [Self; 3] = [Self::Bag, Self::Seq, Self::Alt];

    const fn name(self) -> &'static str {
        match self {
            Self::Bag => "Bag",
            Self::Seq => "Seq",
            Self::Alt => "Alt",
        }
    }
}

/// How a property that does not exist yet is added.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Form {
    Attribute,
    Element,
}

/// One item of a list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Item {
    pub(crate) language: Option<String>,
    pub(crate) text: String,
}

impl Item {
    /// An item with no language, as the items of a bag or sequence are.
    pub(crate) fn plain(text: impl Into<String>) -> Self {
        Self {
            language: None,
            text: text.into(),
        }
    }
}

/// The element around `rdf:RDF` (normally `x:xmpmeta`), taken apart: itself
/// without children, and the children on either side of `rdf:RDF`.
struct Wrapper {
    element: Element,
    before: Vec<Node>,
    after: Vec<Node>,
}

/// An XMP packet: a document known to contain `rdf:RDF`.
pub(crate) struct Packet {
    prolog: Vec<Node>,
    wrapper: Option<Wrapper>,
    rdf: Element,
    epilog: Vec<Node>,
}

impl Packet {
    /// An empty packet in the standard wrapper.
    pub(crate) fn new() -> Self {
        let mut wrapper = Element::new(format!("{}:xmpmeta", META.prefix));
        wrapper.set_attribute(&format!("xmlns:{}", META.prefix), META.uri);
        wrapper.set_attribute(&format!("{}:xmptk", META.prefix), TOOLKIT);
        let line_end = || Node::Text("\n".to_owned());
        Self {
            prolog: vec![
                Node::Instruction(format!("xpacket begin=\"\u{feff}\" id=\"{PACKET_ID}\"")),
                line_end(),
            ],
            wrapper: Some(Wrapper {
                element: wrapper,
                before: Vec::new(),
                after: Vec::new(),
            }),
            rdf: new_rdf(),
            epilog: vec![
                line_end(),
                Node::Instruction("xpacket end=\"w\"".to_owned()),
                line_end(),
            ],
        }
    }

    /// Reads a packet. The root must be `rdf:RDF`, contain it as a child, or
    /// be an `x:xmpmeta` that does not hold one yet; anything else is not
    /// XMP.
    pub(crate) fn parse(text: &str) -> Result<Self, XmpError> {
        let Document {
            prolog,
            mut root,
            epilog,
        } = Document::parse(text)?;
        let scope = Scope::of(&root);
        if scope.element_is(root.name(), RDF, "RDF") {
            return Ok(Self {
                prolog,
                wrapper: None,
                rdf: root,
                epilog,
            });
        }
        let found = root
            .elements()
            .find(|(_, child)| scope.enter(child).element_is(child.name(), RDF, "RDF"))
            .map(|(position, _)| position);
        let is_wrapper = scope.element_is(root.name(), META, "xmpmeta");
        let position = match found {
            Some(position) => position,
            None if is_wrapper => root.push_element(new_rdf()),
            None => return Err(XmpError::NotXmp),
        };
        let mut rest = root.children.split_off(position).into_iter();
        let Some(Node::Element(rdf)) = rest.next() else {
            return Err(XmpError::NotXmp);
        };
        let wrapper = Wrapper {
            before: std::mem::take(&mut root.children),
            after: rest.collect(),
            element: root,
        };
        Ok(Self {
            prolog,
            wrapper: Some(wrapper),
            rdf,
            epilog,
        })
    }

    /// The packet as text. An `rdf:RDF` this crate created and nothing was
    /// put into is left out, so that a packet that had none and gained no
    /// property comes out as it went in.
    pub(crate) fn into_string(self) -> String {
        let root = match self.wrapper {
            None => self.rdf,
            Some(wrapper) => {
                let mut root = wrapper.element;
                root.children = wrapper.before;
                if !(self.rdf.is_built() && self.rdf.children.is_empty()) {
                    root.children.push(Node::Element(self.rdf));
                }
                root.children.extend(wrapper.after);
                root
            }
        };
        let document = Document {
            prolog: self.prolog,
            root,
            epilog: self.epilog,
        };
        document.write()
    }

    /// Runs `read` with the namespaces in force inside `rdf:RDF`.
    fn in_rdf_scope<T>(&self, read: impl FnOnce(&Scope<'_>) -> T) -> T {
        let wrapper = self
            .wrapper
            .as_ref()
            .map(|wrapper| Scope::of(&wrapper.element));
        match &wrapper {
            Some(outer) => read(&outer.enter(&self.rdf)),
            None => read(&Scope::of(&self.rdf)),
        }
    }

    /// Visits each description directly inside `rdf:RDF`, with its position
    /// among the children and the namespaces in force inside it.
    fn each_description<'p>(&'p self, mut visit: impl FnMut(usize, &'p Element, &Scope<'_>)) {
        self.in_rdf_scope(|rdf| {
            for (index, element) in self.rdf.elements() {
                let scope = rdf.enter(element);
                if scope.element_is(element.name(), RDF, "Description") {
                    visit(index, element, &scope);
                }
            }
        });
    }
}

fn new_rdf() -> Element {
    let mut rdf = Element::new(format!("{}:RDF", RDF.prefix));
    rdf.set_attribute(&format!("xmlns:{}", RDF.prefix), RDF.uri);
    rdf
}
