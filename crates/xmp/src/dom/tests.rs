use super::parse::MAX_DEPTH;
use super::{Document, Element, Node};
use crate::XmpError;

fn document(root: Element) -> Document {
    Document {
        prolog: Vec::new(),
        root,
        epilog: Vec::new(),
    }
}

#[test]
fn an_untouched_document_is_written_back_byte_for_byte() {
    let documents = [
        "<a/>",
        "<a></a>",
        "<a b='1' c = \"2\"   d=\"&lt;&#x41;&amp;\" />",
        "<?xml version=\"1.0\"?>\n<!DOCTYPE a>\n<!-- c -->\n<a>\n  <b>t &amp; &#169; u</b>\n  <![CDATA[ <raw> & ]]>\n  <?pi data?>\n</a>\n<!-- after -->\n",
        "<a>text<b/>mixed <c>content</c> here</a>",
        "<a\n   b=\"1\"\n   c='say \"hi\"'>\r\n\t</a>",
        "<n:a xmlns:n=\"urn:x\"><n:b n:c=\"d\"/><e xmlns=\"urn:y\"></e></n:a>",
        "<?xpacket begin=\"\u{feff}\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?><a>ü 雪 🎞</a><?xpacket end=\"w\"?>",
    ];
    for text in documents {
        assert_eq!(Document::parse(text).unwrap().write(), text);
    }
}

#[test]
fn only_a_byte_order_mark_and_padding_in_end_tags_are_normalised() {
    assert_eq!(
        Document::parse("\u{feff}<a></a >").unwrap().write(),
        "<a></a>"
    );
}

#[test]
fn changing_an_attribute_rewrites_that_start_tag_and_no_other() {
    let mut document =
        Document::parse("<a x='1'>\n  <b y='2' z='&quot;3&quot;'/>\n  <c w='4'/>\n</a>").unwrap();
    let b = document.root.element_mut(1).unwrap();
    assert_eq!(b.attribute("z"), Some("\"3\""));
    b.set_attribute("y", "5 < 6 & \"7\"");
    assert_eq!(
        document.write(),
        "<a x='1'>\n  <b y=\"5 &lt; 6 &amp; &quot;7&quot;\" z=\"&quot;3&quot;\"/>\n  <c w='4'/>\n</a>"
    );
}

#[test]
fn an_attribute_edit_that_changes_nothing_keeps_the_tag_as_written() {
    let text = "<a x='1'  y='2'/>";
    let mut document = Document::parse(text).unwrap();
    document.root.retain_attributes(|_| true);
    document.root.remove_attribute(9);
    document.root.set_attribute_at(9, "ignored");
    assert_eq!(document.write(), text);
}

#[test]
fn many_attributes_are_written_one_to_a_line() {
    let mut document = Document::parse("<r>\n  <a/>\n</r>").unwrap();
    let a = document.root.element_mut(1).unwrap();
    for (name, value) in [("one", "1"), ("two", "2"), ("three", "3")] {
        a.set_attribute(name, value);
    }
    a.insert_attribute(1, "early", "0");
    assert_eq!(
        document.write(),
        "<r>\n  <a one=\"1\"\n    early=\"0\"\n    two=\"2\"\n    three=\"3\"/>\n</r>"
    );
    document
        .root
        .element_mut(1)
        .unwrap()
        .retain_attributes(|attribute| attribute.name != "two");
    document.root.element_mut(1).unwrap().remove_attribute(0);
    assert_eq!(
        document.write(),
        "<r>\n  <a early=\"0\" three=\"3\"/>\n</r>"
    );
}

#[test]
fn elements_made_here_are_laid_out_like_their_siblings() {
    let mut list = Element::new("list");
    list.children
        .push(Node::Element(Element::with_text("item", "one")));
    list.children
        .push(Node::Element(Element::with_text("item", "")));

    let mut indented = Document::parse("<r>\n    <a/>\n</r>").unwrap();
    assert_eq!(indented.root.push_element(list.clone()), 2);
    assert_eq!(
        indented.write(),
        "<r>\n    <a/>\n    <list>\n     <item>one</item>\n     <item/>\n    </list>\n</r>"
    );

    let mut empty = Document::parse("<r>\n  <a/>\n</r>").unwrap();
    empty
        .root
        .element_mut(1)
        .unwrap()
        .push_element(Element::with_text("b", "t"));
    assert_eq!(empty.write(), "<r>\n  <a>\n   <b>t</b>\n  </a>\n</r>");

    let mut compact = Document::parse("<r><a/></r>").unwrap();
    compact.root.push_element(Element::new("b"));
    assert_eq!(compact.write(), "<r><a/>\n <b/>\n</r>");

    let mut replaced = Document::parse("<r>\n\t<a><old/></a>\n</r>").unwrap();
    replaced.root.element_mut(1).unwrap().set_content(list);
    assert_eq!(
        replaced.write(),
        "<r>\n\t<a>\n\t <list>\n\t  <item>one</item>\n\t  <item/>\n\t </list>\n\t</a>\n</r>"
    );
}

#[test]
fn what_is_written_reads_back_as_the_same_bytes() {
    let mut root = Element::new("r");
    root.set_attribute("a", "1");
    let mut list = Element::new("list");
    list.children
        .push(Node::Element(Element::with_text("item", "one")));
    root.push_element(list);
    root.push_element(Element::new("empty"));
    let written = document(root).write();
    assert_eq!(
        written,
        "<r a=\"1\">\n <list>\n  <item>one</item>\n </list>\n <empty/>\n</r>"
    );
    assert_eq!(Document::parse(&written).unwrap().write(), written);
}

#[test]
fn removing_an_element_takes_its_indentation_with_it() {
    let mut document = Document::parse("<r>\n  <a/>\n  <b/>\n  <c/>\n</r>").unwrap();
    document.root.remove_child(3);
    assert_eq!(document.write(), "<r>\n  <a/>\n  <c/>\n</r>");
    document.root.remove_child(1);
    assert_eq!(document.write(), "<r>\n  <c/>\n</r>");
    document.root.remove_child(7);
    document.root.remove_child(1);
    assert_eq!(document.write(), "<r>\n</r>");

    let mut inline = Document::parse("<r>keep <a/> this</r>").unwrap();
    inline.root.remove_child(1);
    assert_eq!(inline.write(), "<r>keep  this</r>");
}

#[test]
fn text_is_decoded_on_request_and_escaped_when_set() {
    let read = Document::parse("<a>x &lt; y<![CDATA[ & ]]>&#65;\r\nz<!-- c --></a>").unwrap();
    assert_eq!(read.root.text().as_deref(), Some("x < y & A\nz"));
    assert_eq!(
        Document::parse("<a/>").unwrap().root.text().as_deref(),
        Some("")
    );
    assert_eq!(Document::parse("<a>x<b/></a>").unwrap().root.text(), None);

    let text = "1 < 2 & 3 > 2 ]]> \"q\"\ttab\r\n";
    let written = document(Element::with_text("a", text)).write();
    assert_eq!(
        written,
        "<a>1 &lt; 2 &amp; 3 &gt; 2 ]]&gt; \"q\"\ttab&#13;\n</a>"
    );
    assert_eq!(
        Document::parse(&written).unwrap().root.text().as_deref(),
        Some(text)
    );
}

#[test]
fn attribute_values_survive_whatever_they_contain() {
    let value = "tab\there\nnew line\r \"quoted\" 'single' <&> ü 雪";
    let mut element = Element::new("a");
    element.set_attribute("v", value);
    let written = document(element).write();
    assert!(
        !written.contains('\n') && !written.contains('\t'),
        "{written}"
    );
    assert_eq!(
        Document::parse(&written).unwrap().root.attribute("v"),
        Some(value)
    );
}

#[test]
fn characters_xml_cannot_represent_are_left_out() {
    let mut element =
        Element::with_text("a", "a\u{0}b\u{8}c\u{b}d\u{1f}e\u{fffe}f\u{ffff}g\u{7f}h");
    element.set_attribute("v", "x\u{1}y");
    let written = document(element).write();
    assert_eq!(written, "<a v=\"xy\">abcdefg\u{7f}h</a>");
}

#[test]
fn nesting_is_accepted_up_to_the_limit_and_refused_beyond_it() {
    let nested = |depth: usize| format!("{}{}", "<a>".repeat(depth), "</a>".repeat(depth));
    assert!(Document::parse(&nested(MAX_DEPTH)).is_ok());
    let error = Document::parse(&nested(MAX_DEPTH + 1)).unwrap_err();
    assert!(matches!(error, XmpError::Malformed { .. }), "{error:?}");
}

#[test]
fn a_document_needs_exactly_one_root_and_nothing_loose_around_it() {
    for text in [
        "",
        "  ",
        "<!-- only a comment -->",
        "<a/><b/>",
        "x<a/>",
        "<a/>x",
        "<a/><![CDATA[x]]>",
    ] {
        let error = Document::parse(text).unwrap_err();
        assert!(
            matches!(error, XmpError::Malformed { .. }),
            "{text:?}: {error:?}"
        );
    }
    let padded = Document::parse(" <?p?> <a/> <!-- c --> ").unwrap();
    assert_eq!((padded.prolog.len(), padded.epilog.len()), (3, 3));
}

#[test]
fn a_declared_encoding_other_than_utf8_is_refused() {
    for accepted in [
        "<?xml version=\"1.0\"?><a/>",
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?><a/>",
        "<?xml version='1.0' encoding='utf-8' standalone='yes'?><a/>",
    ] {
        assert_eq!(Document::parse(accepted).unwrap().write(), accepted);
    }
    for refused in [
        "<?xml version=\"1.0\" encoding=\"ISO-8859-1\"?><a/>",
        "<?xml version=\"1.0\" encoding=\"UTF-16\"?><a/>",
    ] {
        let error = Document::parse(refused).unwrap_err();
        assert!(error.to_string().contains("is not UTF-8"), "{error}");
    }
}

#[test]
fn an_unclosed_element_is_named_in_the_error() {
    let error = Document::parse("<a><b>").unwrap_err();
    let XmpError::Malformed { offset, message } = &error else {
        panic!("unexpected error {error:?}");
    };
    assert_eq!(*offset, 6);
    assert!(message.contains("<b>"), "{message}");
}
