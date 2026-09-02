//! `docs/learning/lessons.md` is a queue, not a record: every entry is read by every session in
//! the repo until the commit that lands its gate deletes it. That only works if each rule can
//! reach its evidence, no evidence is stranded, and no entry can sit there without naming the gate
//! it is waiting for — a rule whose entry has gone missing is indistinguishable from one that was
//! never proved, and an entry no rule points at is never read at all.
//!
//! The set-difference checks below pass trivially when both files are empty, which is the correct
//! state for an emptied queue and the wrong thing to trust a parser about: a parser that silently
//! matched nothing would report a healthy file forever, including a half-emptied one. So the
//! parsers are proved against inline fixtures with known answers, and only then pointed at the
//! live files. Emptying one side alone still fails.

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

fn learning_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("docs/learning")
}

/// GitHub's heading-anchor algorithm: lowercase, drop punctuation, and each space becomes its
/// own hyphen so `a - b` yields `a---b`. Matching it exactly is the point — an anchor this
/// accepts but GitHub renders differently is a dead link the test would call healthy.
fn slug(heading: &str) -> String {
    heading
        .trim()
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-' || c.is_whitespace())
        .map(|c| if c.is_whitespace() { '-' } else { c })
        .collect()
}

/// Anchors the index actually links to, in document order.
fn index_anchors(index: &str) -> Vec<String> {
    const OPEN: &str = "[evidence](lessons-evidence.md#";
    let mut found = Vec::new();
    let mut rest = index;
    while let Some(at) = rest.find(OPEN) {
        rest = &rest[at + OPEN.len()..];
        match rest.find(')') {
            Some(end) => {
                found.push(rest[..end].to_string());
                rest = &rest[end..];
            }
            None => break,
        }
    }
    found
}

/// Slugs of the evidence file's entry headings, skipping anything inside a fenced block.
fn evidence_slugs(evidence: &str) -> Vec<String> {
    let mut fenced = false;
    evidence
        .lines()
        .filter_map(|line| {
            if line.trim_start().starts_with("```") {
                fenced = !fenced;
                return None;
            }
            if fenced {
                return None;
            }
            line.strip_prefix("## ").map(slug)
        })
        .collect()
}

/// The `- ` bullets under `## Active Lessons`, which are the queue entries proper. Prose
/// elsewhere in the file is not a rule and is not held to the rules' contract.
fn active_lesson_lines(index: &str) -> Vec<String> {
    index
        .split_once("## Active Lessons")
        .map(|(_, rest)| rest)
        .unwrap_or("")
        .lines()
        .take_while(|line| !line.starts_with("## "))
        .map(str::trim)
        .filter(|line| line.starts_with("- "))
        .map(str::to_string)
        .collect()
}

fn read(name: &str) -> String {
    let path = learning_dir().join(name);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

const INDEX_FIXTURE: &str = "\
# Lessons

Prose that is not a rule, mentioning lessons-evidence.md in passing.

## Active Lessons
- 2026-01-02 - A rule with a link ([evidence](lessons-evidence.md#a-rule-with-a-link))
- 2026-01-03 - Another one ([evidence](lessons-evidence.md#another-one))

## Some later section
- not a lesson
";

const EVIDENCE_FIXTURE: &str = "\
# Lessons — evidence

## A rule with a link

| Field | Value |
|---|---|
| Gate | some::test |

```
## This heading is inside a fence and is not an entry
```

## Another one

| Field | Value |
|---|---|
| Test added | other::test |
";

/// Proves the parsers against text whose answer is known, so the live-file checks below cannot be
/// satisfied by a parser that has quietly stopped matching anything.
#[test]
fn the_parsers_find_what_is_there_and_only_what_is_there() {
    assert_eq!(
        index_anchors(INDEX_FIXTURE),
        vec!["a-rule-with-a-link", "another-one"]
    );
    assert_eq!(
        evidence_slugs(EVIDENCE_FIXTURE),
        vec!["a-rule-with-a-link", "another-one"],
        "a fenced heading is not an entry"
    );
    assert_eq!(active_lesson_lines(INDEX_FIXTURE).len(), 2);
    assert!(active_lesson_lines(INDEX_FIXTURE)[0].contains("A rule with a link"));
    assert_eq!(slug("A - B: c!"), "a---b-c");
    assert!(index_anchors("no links here").is_empty());
    assert!(evidence_slugs("# not an entry\ntext").is_empty());
}

#[test]
fn every_rule_points_at_an_evidence_entry_that_exists() {
    let known: BTreeSet<String> = evidence_slugs(&read("lessons-evidence.md"))
        .into_iter()
        .collect();
    let dangling: BTreeSet<String> = index_anchors(&read("lessons.md"))
        .into_iter()
        .filter(|a| !known.contains(a))
        .collect();
    assert!(
        dangling.is_empty(),
        "lessons.md links to headings that do not exist in the evidence file: {dangling:?}"
    );
}

#[test]
fn every_evidence_entry_has_at_least_one_rule() {
    let linked: BTreeSet<String> = index_anchors(&read("lessons.md")).into_iter().collect();
    let stranded: BTreeSet<String> = evidence_slugs(&read("lessons-evidence.md"))
        .into_iter()
        .filter(|s| !linked.contains(s))
        .collect();
    assert!(
        stranded.is_empty(),
        "evidence entries no rule points at will never be read: {stranded:?}"
    );
}

/// Every queue entry carries its evidence link. The set-difference checks above only see rules
/// that link somewhere; a bullet with no link at all is invisible to them, and that is exactly the
/// shape an unanchored lesson takes. This file once carried 36 of them.
#[test]
fn every_active_lesson_carries_an_evidence_link() {
    let unanchored: Vec<String> = active_lesson_lines(&read("lessons.md"))
        .into_iter()
        .filter(|line| !line.contains("[evidence](lessons-evidence.md#"))
        .collect();
    assert!(
        unanchored.is_empty(),
        "lessons with no evidence anchor are folklore, not lessons: {unanchored:#?}"
    );
}

/// An entry that can name no gate does not belong in the queue — it is either constitution or
/// folklore. The evidence table is where the gate is named, so it has to carry that row.
#[test]
fn every_evidence_entry_names_the_gate_it_is_waiting_for() {
    let evidence = read("lessons-evidence.md");
    let mut current: Option<String> = None;
    let mut named = false;
    let mut missing: Vec<String> = Vec::new();
    for line in evidence.lines().chain(std::iter::once("## ")) {
        if let Some(heading) = line.strip_prefix("## ") {
            if let Some(previous) = current.take() {
                if !named {
                    missing.push(previous);
                }
            }
            current = Some(heading.trim().to_string());
            named = false;
        } else {
            let cell = line.trim_start_matches('|').trim().to_lowercase();
            if cell.starts_with("gate") || cell.starts_with("test added") {
                named = true;
            }
        }
    }
    assert!(
        missing.is_empty(),
        "these evidence entries name no gate, so their lessons cannot ever graduate: {missing:#?}"
    );
}

#[test]
fn the_index_stays_short_enough_to_read_at_session_start() {
    let lines = read("lessons.md").lines().count();
    // Length is what decides whether a session-start file gets read at all. Retire lessons
    // that have become gates rather than raising this ceiling.
    assert!(
        lines <= 120,
        "lessons.md is {lines} lines; the ceiling is 120"
    );
}
