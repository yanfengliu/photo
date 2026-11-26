//! The naming template: text with tokens in braces, the values the tokens
//! stand for, and what a token without a value takes with it.
//!
//! A token without a value (no capture date, no custom text) must not
//! leave a stray separator behind: `{date}-{text}-{seq:3}` without a date
//! is `coast-001`, not `-coast-001`. The separators are `-`, `_`, space and
//! `.`, and the characters no file name may hold, which become `_`;
//! several typed together count as one. An empty token with a separator on
//! both sides takes the one before it, so two never meet. With a separator
//! on one side only, it takes that one when the name would otherwise begin
//! or end with it, and leaves it when something is on its other side,
//! because the separator then still separates two things. Everything else
//! stays as typed: the separators between two values, doubled ones
//! included, and the separators a template itself begins or ends with.

use crate::naming::forbidden;

/// What the tokens of a template stand for, for one item.
pub(crate) struct Values<'a> {
    /// `{name}`: the original file name without its extension.
    pub(crate) name: &'a str,
    /// `{text}`: the custom text.
    pub(crate) text: &'a str,
    /// `{seq}` and `{seq:N}`: the item's number in the export.
    pub(crate) sequence: u32,
    /// `{date}`: the capture date, when it is known.
    pub(crate) date: Option<String>,
}

impl Values<'_> {
    /// What a token stands for, or `None` when it is not a token.
    fn of(&self, token: &str) -> Option<String> {
        let token = token.trim().to_ascii_lowercase();
        match token.as_str() {
            "name" => Some(self.name.to_string()),
            "text" => Some(self.text.to_string()),
            "seq" => Some(self.sequence.to_string()),
            "date" => Some(self.date.clone().unwrap_or_default()),
            _ => {
                let digits: usize = token.strip_prefix("seq:")?.trim().parse().ok()?;
                (1..=9)
                    .contains(&digits)
                    .then(|| format!("{:0digits$}", self.sequence))
            }
        }
    }
}

/// The template with its tokens replaced, as the module documentation
/// says. The result is empty when no token has a value and nothing but
/// separators is left.
pub(crate) fn expand(template: &str, values: &Values) -> String {
    let mut name = Name::default();
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        let after = &rest[open + 1..];
        let Some(close) = after.find('}') else {
            break;
        };
        match values.of(&after[..close]) {
            Some(value) => {
                name.typed(&rest[..open]);
                name.value(&value);
            }
            // Not a token: text like any other, braces included.
            None => name.typed(&rest[..open + close + 2]),
        }
        rest = &after[close + 1..];
    }
    name.typed(rest);
    name.finish()
}

/// True for the characters that separate the parts of a name.
fn separator(c: char) -> bool {
    matches!(c, '-' | '_' | ' ' | '.') || forbidden(c)
}

/// A name being put together, in order, from the text typed in a template
/// and the values of its tokens.
#[derive(Default)]
struct Name {
    text: String,
    /// How many bytes at the end of `text` are separators typed in the
    /// template with nothing after them yet.
    typed_tail: usize,
    /// A token without a value has been met and what follows it is not yet
    /// known.
    gap: bool,
    /// Some token had no value.
    any_empty: bool,
}

impl Name {
    /// Adds text typed in the template.
    fn typed(&mut self, mut typed: &str) {
        if typed.is_empty() {
            return;
        }
        if std::mem::take(&mut self.gap) {
            let after = typed.len() - typed.trim_start_matches(separator).len();
            if after > 0 && self.typed_tail > 0 {
                // Two separators would meet: the one before the token goes.
                self.text.truncate(self.text.len() - self.typed_tail);
            } else if after > 0 && self.text.is_empty() {
                // The name would begin with the separator.
                typed = &typed[after..];
            }
        }
        if !typed.is_empty() {
            self.text.push_str(typed);
            self.typed_tail = typed.len() - typed.trim_end_matches(separator).len();
        }
    }

    /// Adds the value of a token; one of nothing but spaces is no value.
    fn value(&mut self, value: &str) {
        if value.trim().is_empty() {
            self.gap = true;
            self.any_empty = true;
        } else {
            // A separator typed before an empty token now stands between
            // what was before it and this value, so it stays.
            self.gap = false;
            self.text.push_str(value);
            self.typed_tail = 0;
        }
    }

    /// The name. Empty when no token had a value and nothing but separators
    /// is left of the template.
    fn finish(mut self) -> String {
        if self.gap {
            // The name would end with the separator.
            self.text.truncate(self.text.len() - self.typed_tail);
        }
        if self.any_empty && self.text.chars().all(separator) {
            self.text.clear();
        }
        self.text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DATE: Option<&str> = Some("2024-03-15");

    /// The values of a photo called `beach`, number 7 in its export, with
    /// this custom text and capture date.
    fn values<'a>(text: &'a str, date: Option<&str>) -> Values<'a> {
        Values {
            name: "beach",
            text,
            sequence: 7,
            date: date.map(String::from),
        }
    }

    /// Checks what each `(template, custom text, capture date)` expands to.
    fn expands(cases: &[(&str, &str, Option<&str>, &str)]) {
        for &(template, text, date, expected) in cases {
            let name = expand(template, &values(text, date));
            assert_eq!(name, expected, "{template:?}, text {text:?}, date {date:?}");
        }
    }

    #[test]
    fn an_empty_first_token_leaves_no_separator_at_the_start() {
        expands(&[
            // The export dialog's example: a photo without a capture date.
            ("{date}-{text}-{seq:3}", "coast", None, "coast-007"),
            ("{date}_{name}", "", None, "beach"),
            ("{text} {name}", "", DATE, "beach"),
            ("{date}.{name}", "", None, "beach"),
            // Separators typed together are one separator.
            ("{date} - {name}", "", None, "beach"),
            ("{date}__{name}", "", None, "beach"),
        ]);
    }

    #[test]
    fn an_empty_last_token_leaves_no_separator_at_the_end() {
        expands(&[
            ("{name}_{text}", "", None, "beach"),
            ("{name}-{seq}-{date}", "", None, "beach-7"),
            ("{name} - {text}", "", None, "beach"),
            ("{date}.{text}", "", DATE, "2024-03-15"),
        ]);
    }

    #[test]
    fn an_empty_middle_token_leaves_one_separator_not_two() {
        expands(&[
            ("{name}-{text}-{seq}", "", None, "beach-7"),
            // The separator before the empty token is the one that goes.
            ("{name}_{text}-{seq}", "", None, "beach-7"),
            ("{name}-{text}_{seq}", "", None, "beach_7"),
            ("{date}_{text}_{name}", "", DATE, "2024-03-15_beach"),
            ("{name} - {text} - {seq}", "", None, "beach - 7"),
        ]);
    }

    #[test]
    fn two_empty_tokens_in_a_row_leave_nothing_behind() {
        expands(&[
            ("{date}-{text}-{seq:3}", "", None, "007"),
            ("{date}{text}-{name}", "", None, "beach"),
            ("{name}-{text}-{date}", "", None, "beach"),
            ("{name}_{text}{date}", "", None, "beach"),
            ("{name}-{text}-{date}-{seq}", "", None, "beach-7"),
            ("{seq}_{date}{text}_{name}", "", None, "7_beach"),
        ]);
    }

    /// Every combination of missing values, for tokens joined by different
    /// separators: what is left is the values that are there, each joined
    /// to the one before it by the separator typed before it.
    #[test]
    fn the_values_that_are_there_are_joined_by_the_separator_before_each() {
        for missing in 0..8_u8 {
            let values = Values {
                name: if missing & 1 == 0 { "beach" } else { "" },
                text: if missing & 2 == 0 { "coast" } else { "" },
                sequence: 7,
                date: (missing & 4 == 0).then(|| "2024-03-15".to_string()),
            };
            let date = values.date.clone().unwrap_or_default();
            let (text, name) = (values.text, values.name);
            let joined = |parts: [(&str, &str); 4]| {
                let mut joined = String::new();
                for (separator, value) in parts {
                    if !value.is_empty() {
                        if !joined.is_empty() {
                            joined.push_str(separator);
                        }
                        joined.push_str(value);
                    }
                }
                joined
            };
            let cases = [
                // The token that always has a value last, then first.
                (
                    "{date}-{text}_{name}.{seq}",
                    [("", date.as_str()), ("-", text), ("_", name), (".", "7")],
                ),
                (
                    "{seq}.{date}-{text}_{name}",
                    [("", "7"), (".", date.as_str()), ("-", text), ("_", name)],
                ),
                // No token that always has a value: nothing may be left.
                (
                    "{date} {text}__{name}",
                    [("", date.as_str()), (" ", text), ("__", name), ("", "")],
                ),
            ];
            for (template, parts) in cases {
                assert_eq!(
                    expand(template, &values),
                    joined(parts),
                    "{template:?}, missing {missing:03b}"
                );
            }
        }
    }

    #[test]
    fn a_template_of_only_empty_tokens_leaves_nothing() {
        for template in [
            "{text}",
            "{date}-{text}",
            "{date}{text}",
            "{date} _-. {text}",
            // Nothing but separators would be left.
            "-{text}-",
            "_{date}",
            "{text}.",
        ] {
            assert_eq!(expand(template, &values("", None)), "", "{template:?}");
        }
    }

    #[test]
    fn a_template_with_no_empty_token_is_unchanged() {
        expands(&[
            (
                "{date}_{text}-{name}_{seq:4}",
                "Trip",
                DATE,
                "2024-03-15_Trip-beach_0007",
            ),
            // Doubled separators, and the ones a template begins and ends
            // with, are as typed.
            ("{name}--{seq}", "", None, "beach--7"),
            ("-{name}-", "", None, "-beach-"),
            ("{name} - {text}__{seq}", "Trip", None, "beach - Trip__7"),
            ("__-", "", None, "__-"),
            // What is not a token is text.
            ("{nope}-{name}", "", None, "{nope}-beach"),
            ("{name}_{seq:0}", "", None, "beach_{seq:0}"),
            ("{name", "", None, "{name"),
        ]);
        // Whatever is typed around them, tokens that all have values are
        // simply replaced.
        for template in [
            "",
            "-",
            "--{name}--{text}--",
            "{name}{text}{seq}{date}",
            " {name} . {seq:2} _ {date}",
            "a/b:{name}|{text}",
            "{nope}{name}{",
            "..{name}..",
        ] {
            let replaced = template
                .replace("{name}", "beach")
                .replace("{text}", "Trip")
                .replace("{seq:2}", "07")
                .replace("{seq}", "7")
                .replace("{date}", "2024-03-15");
            assert_eq!(
                expand(template, &values("Trip", DATE)),
                replaced,
                "{template:?}"
            );
        }
    }

    #[test]
    fn a_separator_that_still_separates_two_things_stays() {
        expands(&[
            ("IMG{text}-{seq}", "", None, "IMG-7"),
            ("{name}-{text}{seq}", "", None, "beach-7"),
            ("{date}_at_{name}", "", None, "at_beach"),
            // Brackets are not separators.
            ("{name} ({text})", "", None, "beach ()"),
            // The template's own first and last separators.
            ("-{text}{name}", "", None, "-beach"),
            ("{name}{text}-", "", None, "beach-"),
            ("{name}-{text}-", "", None, "beach-"),
        ]);
    }

    #[test]
    fn only_typed_separators_are_taken_and_blank_values_are_empty() {
        expands(&[
            // A value's own separators are part of the value.
            ("{text}{date}", "a-", None, "a-"),
            ("{text}{date}-{seq}", "a-", None, "a--7"),
            // A value of nothing but spaces is no value.
            ("{name}_{text}", "   ", None, "beach"),
            // A character no file name may hold becomes `_`, a separator.
            ("{date}/{name}", "", None, "beach"),
            ("{name}:{text}", "", None, "beach"),
        ]);
    }
}
