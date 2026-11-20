//! Stars and the pick or reject flag: what a rating outside the usual range
//! means, how the private flag is read, and that both come back as written
//! from a new sidecar and from an updated one.

use photo_core::develop::DevelopSettings;
use photo_core::meta::{FileKind, Flag, Rating};
use photo_xmp::{ImageInfo, XmpData, parse, read_sidecar, to_string, write_sidecar};

const ELEMENT_FORM: &str = include_str!("fixtures/element_form.xmp");

const RAW: ImageInfo = ImageInfo::new(FileKind::Raw);

const FLAGS: [Flag; 3] = [Flag::Rejected, Flag::Unflagged, Flag::Picked];

/// A minimal packet with `attributes` on its one description.
fn packet(attributes: &str) -> String {
    format!(
        r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description rdf:about="" xmlns:xmp="http://ns.adobe.com/xap/1.0/" xmlns:photo="http://ns.photo.invalid/1.0/" {attributes}/></rdf:RDF></x:xmpmeta>"#
    )
}

/// The stars and the flag `xml` gives.
fn marks(xml: &str) -> (u8, Flag) {
    let data = parse(xml, RAW).unwrap();
    (data.rating.stars(), data.flag)
}

/// Data that says nothing but a rating and a flag.
fn marked(stars: u8, flag: Flag) -> XmpData {
    XmpData {
        rating: Rating::new(stars),
        flag,
        ..XmpData::default()
    }
}

/// How `flag` is written; an unflagged photo has no such property.
fn flag_attribute(flag: Flag) -> Option<&'static str> {
    match flag {
        Flag::Picked => Some(r#"photo:Flag="1""#),
        Flag::Rejected => Some(r#"photo:Flag="-1""#),
        Flag::Unflagged => None,
    }
}

/// `xml` without `attribute`, which must stand on a line of its own.
fn without_attribute(xml: &str, attribute: &str) -> String {
    let at = xml.find(attribute).expect("the attribute is there");
    let line = xml[..at].rfind('\n').expect("the attribute starts a line");
    assert!(xml[line..at].trim().is_empty(), "{xml}");
    format!("{}{}", &xml[..line], &xml[at + attribute.len()..])
}

#[test]
fn a_rating_is_rounded_to_whole_stars_and_capped_at_five() {
    for (text, stars) in [
        ("0", 0),
        ("-0", 0),
        ("0.4", 0),
        ("0.5", 1),
        ("1", 1),
        ("+2", 2),
        ("2.4", 2),
        ("3.0", 3),
        (" 4 ", 4),
        ("4.5", 5),
        ("4.6", 5),
        ("5", 5),
        ("5.4", 5),
        ("6", 5),
        ("9", 5),
        ("255", 5),
        ("256", 5),
        ("1e9", 5),
        // Finite as written, though too large for single precision.
        ("1e39", 5),
        ("1e300", 5),
    ] {
        let xml = packet(&format!(r#"xmp:Rating="{text}""#));
        assert_eq!(marks(&xml), (stars, Flag::Unflagged), "{text:?}");
    }
}

#[test]
fn a_negative_rating_means_rejected_and_has_no_stars() {
    // The specification writes -1 for a rejected file and allows no other
    // negative value; all of them are read alike.
    for text in [
        "-1", "-1.0", " -1 ", "-0.01", "-0.5", "-2", "-5", "-1000", "-1e9", "-1e39", "-1e300",
    ] {
        let xml = packet(&format!(r#"xmp:Rating="{text}""#));
        assert_eq!(marks(&xml), (0, Flag::Rejected), "{text:?}");
    }
    assert_eq!(marks(ELEMENT_FORM), (0, Flag::Rejected));
}

#[test]
fn a_rating_that_is_not_a_finite_number_counts_as_absent() {
    assert_eq!(marks(&packet("")), (0, Flag::Unflagged));
    for text in [
        "", " ", "lots", "3 stars", "1,5", "--1", "0x3", "NaN", "inf", "-inf", "1e999", "-1e999",
    ] {
        let xml = packet(&format!(r#"xmp:Rating="{text}""#));
        assert_eq!(marks(&xml), (0, Flag::Unflagged), "{text:?}");
    }
}

#[test]
fn the_private_flag_wins_over_what_the_rating_says() {
    for (text, flag) in [
        ("1", Flag::Picked),
        ("0", Flag::Unflagged),
        ("-1", Flag::Rejected),
        (" +1 ", Flag::Picked),
    ] {
        let beside = |rating: &str| {
            let xml = packet(&format!(r#"xmp:Rating="{rating}" photo:Flag="{text}""#));
            marks(&xml)
        };
        assert_eq!(beside("4"), (4, flag), "{text:?}");
        // A rating of -1 has no stars, whichever property decides the flag.
        assert_eq!(beside("-1"), (0, flag), "{text:?}");
        let alone = packet(&format!(r#"photo:Flag="{text}""#));
        assert_eq!(marks(&alone), (0, flag), "{text:?}");
    }
}

#[test]
fn a_private_flag_with_any_other_value_counts_as_absent() {
    for text in [
        "", "2", "-2", "1.0", "0.5", "1 1", "picked", "Rejected", "true", "NaN",
    ] {
        let beside = |rating: &str| {
            let xml = packet(&format!(r#"xmp:Rating="{rating}" photo:Flag="{text}""#));
            marks(&xml)
        };
        assert_eq!(beside("3"), (3, Flag::Unflagged), "{text:?}");
        assert_eq!(beside("-1"), (0, Flag::Rejected), "{text:?}");
    }
}

#[test]
fn the_private_flag_is_found_by_its_namespace_in_either_form() {
    let xml = r#"<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
<rdf:Description rdf:about="" xmlns:app="http://ns.photo.invalid/1.0/" xmlns:photo="http://ns.example.com/not-ours/" photo:Flag="-1"><app:Flag>1</app:Flag></rdf:Description>
</rdf:RDF>"#;
    assert_eq!(marks(xml), (0, Flag::Picked));
}

#[test]
fn every_rating_and_flag_reads_back_as_written() {
    for stars in 0..=5 {
        for flag in FLAGS {
            let data = marked(stars, flag);
            let xml = to_string(&data, RAW, None).unwrap();
            assert_eq!(parse(&xml, RAW).unwrap(), data, "{xml}");

            // The stars are the rating whatever the flag is, ...
            let rating = format!(r#"xmp:Rating="{stars}""#);
            assert_eq!(xml.contains("xmp:Rating"), stars > 0, "{xml}");
            assert!(stars == 0 || xml.contains(&rating), "{xml}");
            // ... and the flag is the private property, which an unflagged
            // photo does not have.
            match flag_attribute(flag) {
                Some(attribute) => assert!(xml.contains(attribute), "{xml}"),
                None => assert!(!xml.contains("photo:"), "{xml}"),
            }
        }
    }
}

#[test]
fn a_sidecar_moves_between_every_rating_and_flag() {
    let states: Vec<(u8, Flag)> = [0, 3, 5]
        .into_iter()
        .flat_map(|stars| FLAGS.map(|flag| (stars, flag)))
        .collect();
    for &(from_stars, from_flag) in &states {
        let before = to_string(&marked(from_stars, from_flag), RAW, None).unwrap();
        for &(stars, flag) in &states {
            let wanted = marked(stars, flag);
            let after = to_string(&wanted, RAW, Some(&before)).unwrap();
            let step = format!("{from_stars} {from_flag:?} to {stars} {flag:?}:\n{after}");
            assert_eq!(parse(&after, RAW).unwrap(), wanted, "{step}");
            let flags = after.matches("photo:Flag").count();
            assert_eq!(flags, usize::from(flag != Flag::Unflagged), "{step}");
            assert!(after.matches("xmp:Rating").count() <= 1, "{step}");
            assert!(!after.contains(r#"Rating="-1""#), "{step}");
            assert_eq!(
                to_string(&wanted, RAW, Some(&after)).unwrap(),
                after,
                "{step}"
            );
        }
    }
}

#[test]
fn unflagging_a_picked_photo_removes_the_flag_and_nothing_else() {
    let picked = XmpData {
        title: "Quay".to_owned(),
        keywords: vec!["harbour".to_owned(), "dusk".to_owned()],
        develop: Some(DevelopSettings::default_for(FileKind::Raw)),
        ..marked(3, Flag::Picked)
    };
    let unflagged = XmpData {
        flag: Flag::Unflagged,
        ..picked.clone()
    };
    let written = to_string(&picked, RAW, None).unwrap();
    let updated = to_string(&unflagged, RAW, Some(&written)).unwrap();
    assert_eq!(updated, without_attribute(&written, r#"photo:Flag="1""#));
    assert_eq!(parse(&updated, RAW).unwrap(), unflagged);

    // Picking it again puts back what was removed.
    let again = to_string(&picked, RAW, Some(&updated)).unwrap();
    assert_eq!(parse(&again, RAW).unwrap(), picked);
    assert_eq!(without_attribute(&again, r#"photo:Flag="1""#), updated);
}

#[test]
fn a_flag_held_in_an_element_is_changed_and_removed_where_it_is() {
    let picked = r#"<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
 <rdf:Description rdf:about="" xmlns:xmp="http://ns.adobe.com/xap/1.0/" xmlns:app="http://ns.photo.invalid/1.0/">
  <xmp:Rating>4</xmp:Rating>
  <app:Flag>1</app:Flag>
  <app:Later>kept</app:Later>
 </rdf:Description>
</rdf:RDF>"#;
    assert_eq!(marks(picked), (4, Flag::Picked));

    let rejected = to_string(&marked(4, Flag::Rejected), RAW, Some(picked)).unwrap();
    assert_eq!(
        rejected,
        picked.replace("<app:Flag>1</app:Flag>", "<app:Flag>-1</app:Flag>")
    );
    for existing in [picked, rejected.as_str()] {
        let unflagged = to_string(&marked(4, Flag::Unflagged), RAW, Some(existing)).unwrap();
        assert_eq!(unflagged, picked.replace("\n  <app:Flag>1</app:Flag>", ""));
    }
}

#[test]
fn marks_that_already_read_right_are_left_as_written() {
    for (attributes, stars, flag) in [
        (r#"xmp:Rating="-1""#, 0, Flag::Rejected),
        (r#"xmp:Rating="-1" photo:Flag="-1""#, 0, Flag::Rejected),
        (r#"xmp:Rating="3.2" photo:Flag=" +1 ""#, 3, Flag::Picked),
        (r#"xmp:Rating="2" photo:Flag="0""#, 2, Flag::Unflagged),
        (
            r#"xmp:Rating="lots" photo:Flag="maybe""#,
            0,
            Flag::Unflagged,
        ),
    ] {
        let xml = packet(attributes);
        assert_eq!(marks(&xml), (stars, flag), "{attributes}");
        let written = to_string(&marked(stars, flag), RAW, Some(&xml)).unwrap();
        assert_eq!(written, xml, "{attributes}");
    }
}

#[test]
fn a_rejection_made_elsewhere_over_a_flag_made_here_does_not_outlive_the_flag() {
    for (flag, attribute) in [(Flag::Picked, "photo:Flag=\"1\""), (Flag::Unflagged, "")] {
        // Flagged here, then rejected by an application that knows only
        // `xmp:Rating`; the private flag, which wins, still says otherwise.
        let here = to_string(&marked(3, flag), RAW, None).unwrap();
        let elsewhere = here.replace(r#"xmp:Rating="3""#, r#"xmp:Rating="-1""#);
        let elsewhere = match flag {
            Flag::Unflagged => elsewhere.replace(
                r#"xmp:Rating="-1""#,
                r#"xmp:Rating="-1" xmlns:photo="http://ns.photo.invalid/1.0/" photo:Flag="0""#,
            ),
            _ => elsewhere,
        };
        assert!(elsewhere.contains(r#"xmp:Rating="-1""#), "{elsewhere}");
        let read = parse(&elsewhere, RAW).unwrap();
        assert_eq!((read.rating.stars(), read.flag), (0, flag));

        // Writing back what was read leaves no rating that says otherwise.
        let written = to_string(&read, RAW, Some(&elsewhere)).unwrap();
        assert!(written.contains(r#"xmp:Rating="0""#), "{written}");
        assert!(written.contains(attribute), "{written}");
        assert_eq!(parse(&written, RAW).unwrap(), read);
        assert_eq!(to_string(&read, RAW, Some(&written)).unwrap(), written);
    }
}

#[test]
fn marks_in_any_state_are_put_right_once_they_change() {
    for attributes in [
        "",
        r#"xmp:Rating="-1""#,
        r#"xmp:Rating="-1" photo:Flag="0""#,
        r#"xmp:Rating="-1" photo:Flag="1""#,
        r#"xmp:Rating="3.2" photo:Flag=" +1 ""#,
        r#"xmp:Rating="lots" photo:Flag="maybe""#,
        r#"photo:Flag="-1""#,
    ] {
        let xml = packet(attributes);
        for stars in [0, 3] {
            for flag in FLAGS {
                let wanted = marked(stars, flag);
                let updated = to_string(&wanted, RAW, Some(&xml)).unwrap();
                let step = format!("{attributes} to {stars} {flag:?}:\n{updated}");
                assert_eq!(parse(&updated, RAW).unwrap(), wanted, "{step}");
                assert_eq!(
                    to_string(&wanted, RAW, Some(&updated)).unwrap(),
                    updated,
                    "{step}"
                );
                // A rating of -1 is left only where it still tells the truth.
                if updated.contains(r#"Rating="-1""#) {
                    assert_eq!((stars, flag), (0, Flag::Rejected), "{step}");
                }
                if marks(&xml) == (stars, flag) {
                    continue;
                }
                // From then on the rating holds the stars and only the stars,
                // and the flag is where this crate writes it.
                assert!(!updated.contains(r#"Rating="-1""#), "{step}");
                assert_eq!(
                    updated.contains("photo:Flag"),
                    flag != Flag::Unflagged,
                    "{step}"
                );
            }
        }
    }
}

#[test]
fn a_rejection_written_as_minus_one_stays_until_the_stars_or_the_flag_change() {
    let rejected = XmpData {
        develop: None,
        ..parse(ELEMENT_FORM, RAW).unwrap()
    };
    assert_eq!(
        (rejected.rating.stars(), rejected.flag),
        (0, Flag::Rejected)
    );
    let update = |data: &XmpData| {
        let updated = to_string(data, RAW, Some(ELEMENT_FORM)).unwrap();
        let read = XmpData {
            develop: None,
            ..parse(&updated, RAW).unwrap()
        };
        assert_eq!(&read, data, "{updated}");
        updated
    };
    assert_eq!(update(&rejected), ELEMENT_FORM);

    // Given stars, the rating holds them and the rejection moves to the flag.
    let starred = update(&XmpData {
        rating: Rating::new(2),
        ..rejected.clone()
    });
    assert!(starred.contains("<xap:Rating>2</xap:Rating>"), "{starred}");
    assert!(starred.contains(r#"photo:Flag="-1""#), "{starred}");

    // No longer rejected, the rating stops saying so.
    let unflagged = update(&XmpData {
        flag: Flag::Unflagged,
        ..rejected.clone()
    });
    assert!(
        unflagged.contains("<xap:Rating>0</xap:Rating>"),
        "{unflagged}"
    );
    assert!(!unflagged.contains("photo:"), "{unflagged}");
    let picked = update(&XmpData {
        flag: Flag::Picked,
        ..rejected
    });
    assert!(picked.contains("<xap:Rating>0</xap:Rating>"), "{picked}");
    assert!(picked.contains(r#"photo:Flag="1""#), "{picked}");
}

#[test]
fn a_sidecar_file_carries_the_flag_from_one_state_to_the_next() {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("IMG_0001.xmp");
    for flag in [
        Flag::Picked,
        Flag::Rejected,
        Flag::Unflagged,
        Flag::Rejected,
        Flag::Picked,
        Flag::Unflagged,
    ] {
        let data = XmpData {
            keywords: vec!["quay".to_owned()],
            ..marked(4, flag)
        };
        write_sidecar(&path, &data, RAW).unwrap();
        assert_eq!(read_sidecar(&path, RAW).unwrap(), Some(data));
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            text.contains("photo:Flag"),
            flag != Flag::Unflagged,
            "{text}"
        );
        assert!(text.contains(r#"xmp:Rating="4""#), "{text}");
    }
}
