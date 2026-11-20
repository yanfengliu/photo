//! Reading a document: `quick-xml` events into the tree, with the checks a
//! pull parser leaves to its caller.

use quick_xml::escape::resolve_xml_entity;
use quick_xml::events::{BytesDecl, BytesRef, BytesStart, Event};
use quick_xml::{Reader, XmlVersion};

use super::{Attribute, Document, Element, Node};
use crate::XmpError;

/// Deepest element nesting accepted. XMP nests a handful of levels; the limit
/// keeps a hostile file from exhausting the stack when the tree is walked.
pub(super) const MAX_DEPTH: usize = 128;

impl Document {
    /// Reads `text`, refusing anything that is not well-formed XML with
    /// namespaces: unclosed or mismatched tags, malformed attributes,
    /// references to entities XML does not define, names with a prefix that
    /// is not declared, text outside the root, more or less than one root.
    /// A declaration that names an encoding other than UTF-8 is refused too,
    /// because the document would be written back contradicting it.
    pub(crate) fn parse(text: &str) -> Result<Self, XmpError> {
        let mut reader = Reader::from_str(text);
        let mut open: Vec<Element> = Vec::new();
        let mut prefixes = Prefixes::default();
        let mut top: Vec<Node> = Vec::new();
        loop {
            let event = reader
                .read_event()
                .map_err(|error| malformed(reader.error_position(), error))?;
            let offset = reader.buffer_position();
            let fail = |message: String| malformed(offset, message);
            let node = match event {
                Event::Start(tag) => {
                    if open.len() >= MAX_DEPTH {
                        return Err(fail(format!(
                            "elements are nested more than {MAX_DEPTH} deep"
                        )));
                    }
                    let element = Element::from_tag(&tag, false).map_err(fail)?;
                    prefixes.open(&element).map_err(fail)?;
                    open.push(element);
                    continue;
                }
                Event::Empty(tag) => {
                    let element = Element::from_tag(&tag, true).map_err(fail)?;
                    prefixes.open(&element).map_err(fail)?;
                    prefixes.close();
                    Node::Element(element)
                }
                Event::End(_) => match open.pop() {
                    Some(element) => {
                        prefixes.close();
                        Node::Element(element)
                    }
                    None => return Err(fail("an end tag has no start tag".to_owned())),
                },
                Event::Text(text) => Node::Text(owned(&text).map_err(fail)?),
                Event::GeneralRef(reference) => {
                    Node::Text(checked_reference(&reference).map_err(fail)?)
                }
                Event::CData(data) => Node::CData(owned(&data).map_err(fail)?),
                Event::Comment(comment) => Node::Comment(owned(&comment).map_err(fail)?),
                Event::PI(instruction) => Node::Instruction(owned(&instruction).map_err(fail)?),
                Event::Decl(declaration) => {
                    Node::Instruction(checked_declaration(&declaration).map_err(fail)?)
                }
                Event::DocType(doctype) => Node::DocType(owned(&doctype).map_err(fail)?),
                Event::Eof => break,
            };
            let siblings = match open.last_mut() {
                Some(parent) => &mut parent.children,
                None => &mut top,
            };
            // Text arrives in pieces around each reference; keep it whole.
            match (siblings.last_mut(), node) {
                (Some(Node::Text(text)), Node::Text(more)) => text.push_str(&more),
                (_, node) => siblings.push(node),
            }
        }
        let end = reader.buffer_position();
        match open.last() {
            Some(element) => Err(malformed(
                end,
                format!("<{}> is never closed", element.name),
            )),
            None => Self::from_top_level(top, end),
        }
    }

    /// Splits the nodes outside any element at the one root element.
    fn from_top_level(nodes: Vec<Node>, end: u64) -> Result<Self, XmpError> {
        let mut prolog = Vec::new();
        let mut root = None;
        let mut epilog = Vec::new();
        for node in nodes {
            match node {
                Node::Element(element) if root.is_none() => root = Some(element),
                Node::Element(_) => {
                    return Err(malformed(end, "there is more than one root element"));
                }
                Node::Text(_) | Node::CData(_) if !node.is_blank() => {
                    return Err(malformed(end, "there is text outside the root element"));
                }
                node if root.is_none() => prolog.push(node),
                node => epilog.push(node),
            }
        }
        match root {
            Some(root) => Ok(Self {
                prolog,
                root,
                epilog,
            }),
            None => Err(malformed(end, "there is no root element")),
        }
    }
}

impl Element {
    /// The element a start tag describes, still without children.
    fn from_tag(tag: &BytesStart<'_>, self_closing: bool) -> Result<Self, String> {
        let name = utf8(tag.name().into_inner())?;
        if name.is_empty() {
            return Err("an element has no name".to_owned());
        }
        let mut attributes = Vec::new();
        for attribute in tag.attributes() {
            let attribute = attribute.map_err(|error| error.to_string())?;
            let value = attribute
                .normalized_value(XmlVersion::Implicit1_0)
                .map_err(|error| error.to_string())?;
            attributes.push(Attribute {
                name: utf8(attribute.key.into_inner())?.to_owned(),
                value: value.into_owned(),
            });
        }
        Ok(Self {
            name: name.to_owned(),
            attributes,
            children: Vec::new(),
            source_tag: Some(utf8(tag)?.to_owned()),
            self_closing,
            built: false,
        })
    }
}

/// The namespace prefixes declared by the elements open at one point of the
/// document.
///
/// A prefix nothing declares makes a name meaningless. It also must not be
/// let through: a declaration added to the document later would silently
/// give every such name a meaning.
#[derive(Default)]
struct Prefixes {
    /// Every declaration in force, innermost last: the prefix, and whether
    /// it is bound (an empty URI takes a binding away).
    declared: Vec<(String, bool)>,
    /// Where the declarations of each open element start.
    frames: Vec<usize>,
}

impl Prefixes {
    /// Takes in the declarations of an element being opened, then checks
    /// the prefixes of its own name and of its attributes.
    fn open(&mut self, element: &Element) -> Result<(), String> {
        self.frames.push(self.declared.len());
        for attribute in element.attributes() {
            if let Some(prefix) = attribute.declared_prefix() {
                let bound = !attribute.value.is_empty();
                self.declared.push((prefix.to_owned(), bound));
            }
        }
        let attributes = element.attributes().iter();
        let names = std::iter::once(element.name())
            .chain(attributes.map(|attribute| attribute.name.as_str()));
        for name in names {
            if let Some((prefix, _)) = name.split_once(':')
                && !self.is_bound(prefix)
            {
                return Err(format!("the namespace prefix of {name} is not declared"));
            }
        }
        Ok(())
    }

    /// Forgets the declarations of the element opened last.
    fn close(&mut self) {
        if let Some(frame) = self.frames.pop() {
            self.declared.truncate(frame);
        }
    }

    fn is_bound(&self, prefix: &str) -> bool {
        // `xml` is bound by definition, and `xmlns` is how declarations are spelled.
        let innermost = self
            .declared
            .iter()
            .rev()
            .find(|(declared, _)| declared == prefix);
        matches!(prefix, "xml" | "xmlns") || innermost.is_some_and(|(_, bound)| *bound)
    }
}

fn malformed(offset: u64, message: impl ToString) -> XmpError {
    XmpError::Malformed {
        offset,
        message: message.to_string(),
    }
}

fn utf8(bytes: &[u8]) -> Result<&str, String> {
    std::str::from_utf8(bytes).map_err(|error| error.to_string())
}

fn owned(bytes: &[u8]) -> Result<String, String> {
    utf8(bytes).map(str::to_owned)
}

/// The XML declaration as it was written, once it is known not to claim an
/// encoding other than the UTF-8 the text is in and will be written back in.
fn checked_declaration(declaration: &BytesDecl<'_>) -> Result<String, String> {
    if let Some(encoding) = declaration.encoding() {
        let encoding = encoding.map_err(|error| error.to_string())?;
        if !encoding.eq_ignore_ascii_case(b"utf-8") && !encoding.eq_ignore_ascii_case(b"utf8") {
            let encoding = String::from_utf8_lossy(&encoding);
            return Err(format!("the declared encoding {encoding} is not UTF-8"));
        }
    }
    owned(declaration)
}

/// A reference as it was written (`&name;`), once it is known to resolve:
/// a character reference, or one of the five entities XML defines.
fn checked_reference(reference: &BytesRef<'_>) -> Result<String, String> {
    let name = utf8(reference)?;
    let character = reference
        .resolve_char_ref()
        .map_err(|error| error.to_string())?;
    if character.is_some() || resolve_xml_entity(name).is_some() {
        Ok(format!("&{name};"))
    } else {
        Err(format!("&{name}; is not a defined entity"))
    }
}
