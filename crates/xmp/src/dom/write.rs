//! Writing a document: what was read goes back exactly as it was, and what
//! this crate added is laid out to match its surroundings.

use super::text::escape;
use super::{Attribute, Document, Element, Node};

/// One level of indentation for elements this crate creates, as the XMP
/// toolkits other applications use write it.
const INDENT: &str = " ";

/// A start tag with more attributes than this puts them one to a line.
const ATTRIBUTES_ON_ONE_LINE: usize = 2;

impl Document {
    /// The document as text.
    pub(crate) fn write(&self) -> String {
        let mut out = String::new();
        for node in &self.prolog {
            write_node(&mut out, node);
        }
        write_element(&mut out, &self.root);
        for node in &self.epilog {
            write_node(&mut out, node);
        }
        out
    }
}

impl Element {
    /// The indentation of this element's children, as the file already
    /// formats them.
    fn child_indent(&self) -> Option<&str> {
        self.children.windows(2).find_map(|pair| match pair {
            [Node::Text(text), Node::Element(_)] if pair[0].is_blank() => {
                text.rsplit_once('\n').map(|(_, indent)| indent)
            }
            _ => None,
        })
    }
}

/// The white space that starts the last line of `text`.
fn line_indent(text: &str) -> &str {
    let line = text.rsplit_once('\n').map_or(text, |(_, line)| line);
    let end = line.find(|character| character != ' ' && character != '\t');
    &line[..end.unwrap_or(line.len())]
}

fn write_node(out: &mut String, node: &Node) {
    let (open, content, close) = match node {
        Node::Element(element) => return write_element(out, element),
        Node::Text(text) => ("", text, ""),
        Node::CData(data) => ("<![CDATA[", data, "]]>"),
        Node::Comment(comment) => ("<!--", comment, "-->"),
        Node::Instruction(instruction) => ("<?", instruction, "?>"),
        Node::DocType(doctype) => ("<!DOCTYPE ", doctype, ">"),
    };
    out.push_str(open);
    out.push_str(content);
    out.push_str(close);
}

fn write_element(out: &mut String, element: &Element) {
    let start = out.len();
    out.push('<');
    match &element.source_tag {
        Some(tag) => out.push_str(tag),
        None => {
            out.push_str(&element.name);
            write_attributes(out, &element.attributes, start);
        }
    }
    if element.children.is_empty() && element.self_closing {
        out.push_str("/>");
        return;
    }
    out.push('>');
    write_children(out, element, start);
    out.push_str("</");
    out.push_str(&element.name);
    out.push('>');
}

/// Writes the attributes of a start tag that began at `start`: on one line
/// when there are few, otherwise one to a line after the first.
fn write_attributes(out: &mut String, attributes: &[Attribute], start: usize) {
    let wrap = attributes.len() > ATTRIBUTES_ON_ONE_LINE;
    let line_break = format!("\n{}{INDENT}{INDENT}", line_indent(&out[..start]));
    for (index, attribute) in attributes.iter().enumerate() {
        out.push_str(if wrap && index > 0 { &line_break } else { " " });
        out.push_str(&attribute.name);
        out.push_str("=\"");
        out.push_str(&escape(&attribute.value, true));
        out.push('"');
    }
}

/// Writes the children of the element whose start tag began at `start`.
/// Children that were read are written as they were. Children this crate
/// made are each put on a line of their own, indented like their siblings,
/// unless the file already indents the place they took over.
fn write_children(out: &mut String, element: &Element, start: usize) {
    let is_built = |node: &Node| matches!(node, Node::Element(element) if element.built);
    if !element.children.iter().any(is_built) {
        for child in &element.children {
            write_node(out, child);
        }
        return;
    }
    let own_indent = line_indent(&out[..start]).to_owned();
    let child_indent = match element.child_indent() {
        Some(indent) => indent.to_owned(),
        None => format!("{own_indent}{INDENT}"),
    };
    let mut indented = false;
    for child in &element.children {
        if is_built(child) && !indented {
            out.push('\n');
            out.push_str(&child_indent);
        }
        write_node(out, child);
        indented = child.is_blank();
    }
    if element.children.last().is_some_and(is_built) {
        out.push('\n');
        out.push_str(&own_indent);
    }
}
