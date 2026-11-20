//! The XMP packet an export carries: Dublin Core text and the rating, in
//! the serialization of the XMP specification (Part 1, §7), wrapped as a
//! packet (§7.3).

/// The XMP properties of an export; `None` and empty are left out.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct XmpFields<'a> {
    /// `dc:title`.
    pub(crate) title: Option<&'a str>,
    /// `dc:description`.
    pub(crate) description: Option<&'a str>,
    /// `dc:creator`.
    pub(crate) creator: Option<&'a str>,
    /// `dc:rights`.
    pub(crate) rights: Option<&'a str>,
    /// `dc:subject`.
    pub(crate) keywords: &'a [&'a str],
    /// `xmp:Rating`, 1 to 5.
    pub(crate) rating: Option<u8>,
}

/// The packet for `fields`, or `None` when there is nothing to write.
pub(crate) fn packet(fields: &XmpFields) -> Option<String> {
    let mut body = String::new();
    if let Some(rating) = fields.rating {
        body.push_str(&format!("   <xmp:Rating>{rating}</xmp:Rating>\n"));
    }
    for (name, value) in [
        ("dc:title", fields.title),
        ("dc:description", fields.description),
    ] {
        if let Some(value) = value {
            body.push_str(&alternative(name, value));
        }
    }
    if let Some(creator) = fields.creator {
        body.push_str(&array("dc:creator", "rdf:Seq", &[creator]));
    }
    if let Some(rights) = fields.rights {
        body.push_str(&alternative("dc:rights", rights));
    }
    if !fields.keywords.is_empty() {
        body.push_str(&array("dc:subject", "rdf:Bag", fields.keywords));
    }
    if body.is_empty() {
        return None;
    }
    Some(format!(
        "<?xpacket begin=\"\u{feff}\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>\n\
         <x:xmpmeta xmlns:x=\"adobe:ns:meta/\">\n \
         <rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\n  \
         <rdf:Description rdf:about=\"\"\n    \
         xmlns:dc=\"http://purl.org/dc/elements/1.1/\"\n    \
         xmlns:xmp=\"http://ns.adobe.com/xap/1.0/\">\n\
         {body}  \
         </rdf:Description>\n \
         </rdf:RDF>\n\
         </x:xmpmeta>\n\
         <?xpacket end=\"w\"?>"
    ))
}

/// A language alternative with only the default language.
fn alternative(name: &str, value: &str) -> String {
    format!(
        "   <{name}>\n    <rdf:Alt>\n     <rdf:li xml:lang=\"x-default\">{}</rdf:li>\n    </rdf:Alt>\n   </{name}>\n",
        escape(value)
    )
}

/// An ordered (`rdf:Seq`) or unordered (`rdf:Bag`) array.
fn array(name: &str, kind: &str, values: &[&str]) -> String {
    let items: String = values
        .iter()
        .map(|value| format!("     <rdf:li>{}</rdf:li>\n", escape(value)))
        .collect();
    format!("   <{name}>\n    <{kind}>\n{items}    </{kind}>\n   </{name}>\n")
}

/// `text` as XML character data: markup characters escaped, characters
/// XML 1.0 cannot hold (control characters other than tab and line breaks)
/// dropped.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\t' | '\n' | '\r' => out.push(c),
            c if c.is_control() || c == '\u{fffe}' || c == '\u{ffff}' => {}
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_to_say_is_no_packet() {
        assert_eq!(packet(&XmpFields::default()), None);
    }

    #[test]
    fn text_is_escaped_and_control_characters_dropped() {
        assert_eq!(
            escape("a & <b> \"c\"\u{1}\u{7f}"),
            "a &amp; &lt;b&gt; &quot;c&quot;"
        );
        let xml = packet(&XmpFields {
            title: Some("Fish & <Chips>"),
            keywords: &["sea", "boat"],
            rating: Some(4),
            ..XmpFields::default()
        })
        .unwrap();
        assert!(xml.contains("<rdf:li xml:lang=\"x-default\">Fish &amp; &lt;Chips&gt;</rdf:li>"));
        assert!(xml.contains("<rdf:li>sea</rdf:li>\n     <rdf:li>boat</rdf:li>"));
        assert!(xml.contains("<xmp:Rating>4</xmp:Rating>"));
        assert!(xml.starts_with("<?xpacket begin=\"\u{feff}\""));
        assert!(xml.ends_with("<?xpacket end=\"w\"?>"));
    }
}
