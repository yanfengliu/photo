//! A small XML document model that keeps what it does not understand.
//!
//! [`Document::parse`] accepts exactly the well-formed documents: one root
//! element, every tag closed, every reference resolvable, every character
//! one XML allows. What it reads is kept as written: character data,
//! comments, processing instructions, and the start tag of every element
//! whose attributes are left alone.
//! [`Document::write`] therefore reproduces an untouched document byte for
//! byte (bar a leading byte-order mark and white space inside end tags), and
//! an edit disturbs nothing but the nodes it touches.

mod parse;
mod text;
mod write;

#[cfg(test)]
mod tests;

/// Anything that can stand between two tags.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Node {
    Element(Element),
    /// Character data exactly as written, references still unresolved.
    Text(String),
    /// The inside of a `<![CDATA[...]]>` section.
    CData(String),
    /// The inside of a `<!--...-->` comment.
    Comment(String),
    /// The inside of `<?...?>`: a processing instruction or the XML declaration.
    Instruction(String),
    /// What follows `<!DOCTYPE`.
    DocType(String),
}

impl Node {
    /// White space between tags: the indentation of whatever follows.
    pub(crate) fn is_blank(&self) -> bool {
        matches!(self, Self::Text(text) if text::is_blank(text))
    }
}

/// One attribute, with the references in its value resolved.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Attribute {
    pub(crate) name: String,
    pub(crate) value: String,
}

impl Attribute {
    /// The prefix this attribute binds to its value, if it is a namespace
    /// declaration; the default namespace counts as the empty prefix.
    pub(crate) fn declared_prefix(&self) -> Option<&str> {
        let rest = self.name.strip_prefix("xmlns")?;
        if rest.is_empty() {
            Some(rest)
        } else {
            rest.strip_prefix(':')
        }
    }
}

/// An element: its qualified name as written, its attributes in order
/// (namespace declarations included) and its children.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Element {
    name: String,
    attributes: Vec<Attribute>,
    pub(crate) children: Vec<Node>,
    /// The start tag between `<` and `>` exactly as read, until an attribute
    /// changes.
    source_tag: Option<String>,
    /// Whether the element is written `<a/>` rather than `<a></a>` while it
    /// has no children.
    self_closing: bool,
    /// Created by this crate rather than read: the writer gives it a line of
    /// its own.
    built: bool,
}

impl Element {
    /// A new, empty element made by this crate.
    pub(crate) fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            attributes: Vec::new(),
            children: Vec::new(),
            source_tag: None,
            self_closing: true,
            built: true,
        }
    }

    /// A new element holding `text`.
    pub(crate) fn with_text(name: impl Into<String>, text: &str) -> Self {
        let mut element = Self::new(name);
        element.set_text(text);
        element
    }

    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    /// Whether this crate made the element, rather than read it.
    pub(crate) fn is_built(&self) -> bool {
        self.built
    }

    pub(crate) fn attributes(&self) -> &[Attribute] {
        &self.attributes
    }

    pub(crate) fn attribute(&self, name: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find(|attribute| attribute.name == name)
            .map(|attribute| attribute.value.as_str())
    }

    /// Sets the attribute called `name`, adding it at the end when new.
    pub(crate) fn set_attribute(&mut self, name: &str, value: &str) {
        let existing = self
            .attributes
            .iter()
            .position(|attribute| attribute.name == name);
        match existing {
            Some(index) => self.set_attribute_at(index, value),
            None => self.insert_attribute(self.attributes.len(), name, value),
        }
    }

    /// Replaces the value of the attribute at `index`.
    pub(crate) fn set_attribute_at(&mut self, index: usize, value: &str) {
        if let Some(attribute) = self.attributes.get_mut(index) {
            attribute.value = value.to_owned();
            self.source_tag = None;
        }
    }

    /// Adds an attribute at `index` (or at the end, if that is past it). An
    /// attribute of that name already there is given the value where it is
    /// instead: an element never holds one name twice.
    pub(crate) fn insert_attribute(&mut self, index: usize, name: &str, value: &str) {
        if let Some(existing) = self
            .attributes
            .iter()
            .position(|attribute| attribute.name == name)
        {
            self.set_attribute_at(existing, value);
            return;
        }
        let attribute = Attribute {
            name: name.to_owned(),
            value: value.to_owned(),
        };
        let index = index.min(self.attributes.len());
        self.attributes.insert(index, attribute);
        self.source_tag = None;
    }

    /// Removes the attributes at `indices`, in one pass however many there
    /// are. Indices past the end are ignored.
    pub(crate) fn remove_attributes(&mut self, indices: &[usize]) {
        let doomed = marked(self.attributes.len(), indices.iter().copied());
        if doomed.contains(&true) {
            let mut position = 0;
            self.attributes.retain(|_| {
                position += 1;
                !doomed[position - 1]
            });
            self.source_tag = None;
        }
    }

    /// Drops the attributes `keep` rejects.
    pub(crate) fn retain_attributes(&mut self, keep: impl FnMut(&Attribute) -> bool) {
        let before = self.attributes.len();
        self.attributes.retain(keep);
        if self.attributes.len() != before {
            self.source_tag = None;
        }
    }

    /// The child elements, each with its position among all children.
    pub(crate) fn elements(&self) -> impl Iterator<Item = (usize, &Element)> {
        let children = self.children.iter().enumerate();
        children.filter_map(|(index, node)| match node {
            Node::Element(element) => Some((index, element)),
            _ => None,
        })
    }

    /// The child at `index`, if it is an element.
    pub(crate) fn element_mut(&mut self, index: usize) -> Option<&mut Element> {
        match self.children.get_mut(index) {
            Some(Node::Element(element)) => Some(element),
            _ => None,
        }
    }

    /// The character content with references resolved, or `None` when the
    /// element holds other elements instead of a plain value.
    pub(crate) fn text(&self) -> Option<String> {
        let mut content = String::new();
        for child in &self.children {
            match child {
                Node::Text(raw) => content.push_str(&text::decode(raw)),
                Node::CData(data) => content.push_str(&text::unix_line_ends(data)),
                Node::Element(_) => return None,
                Node::Comment(_) | Node::Instruction(_) | Node::DocType(_) => {}
            }
        }
        Some(content)
    }

    /// Replaces the content with `text`.
    pub(crate) fn set_text(&mut self, text: &str) {
        self.children.clear();
        if !text.is_empty() {
            self.children.push(Node::Text(text::escape(text, false)));
        }
    }

    /// Replaces the content with one element made by this crate.
    pub(crate) fn set_content(&mut self, child: Element) {
        self.children.clear();
        self.children.push(Node::Element(child));
    }

    /// Puts a child element at `index` among the children (at the end, if
    /// that is past them).
    pub(crate) fn insert_element(&mut self, index: usize, child: Element) {
        let index = index.min(self.children.len());
        self.children.insert(index, Node::Element(child));
    }

    /// Adds a child element after the existing ones (before the white space
    /// that indents the end tag) and returns its position.
    pub(crate) fn push_element(&mut self, child: Element) -> usize {
        let index = match self.children.last() {
            Some(last) if last.is_blank() => self.children.len() - 1,
            _ => self.children.len(),
        };
        self.children.insert(index, Node::Element(child));
        index
    }

    /// Removes the children at `indices`, each together with the white
    /// space that indented it, in one pass however many there are. Indices
    /// past the end are ignored.
    pub(crate) fn remove_children(&mut self, indices: &[usize]) {
        let children = &self.children;
        let indentation = indices.iter().filter_map(|&index| {
            let before = index.checked_sub(1)?;
            let indented = index < children.len() && children[before].is_blank();
            indented.then_some(before)
        });
        let all = indices.iter().copied().chain(indentation);
        let doomed = marked(children.len(), all);
        let mut position = 0;
        self.children.retain(|_| {
            position += 1;
            !doomed[position - 1]
        });
    }
}

/// For each of `count` positions, whether it is among `chosen`.
fn marked(count: usize, chosen: impl Iterator<Item = usize>) -> Vec<bool> {
    let mut marks = vec![false; count];
    for position in chosen {
        if let Some(mark) = marks.get_mut(position) {
            *mark = true;
        }
    }
    marks
}

/// A whole XML document, split at its root element.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Document {
    /// What precedes the root: declaration, packet header, comments, white space.
    pub(crate) prolog: Vec<Node>,
    pub(crate) root: Element,
    /// What follows the root.
    pub(crate) epilog: Vec<Node>,
}
