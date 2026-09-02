//! `docs/learning/lessons.md` is read at session start and is short by construction; the war
//! story and the evidence-anchor table live beside it in `lessons-evidence.md`. The split only
//! works if every rule can reach its evidence and no evidence is stranded — a rule whose entry
//! has gone missing is indistinguishable from one that was never proved, and an entry no rule
//! points at is never read at all.
//!
//! The grandfathered one-line lessons (dated before 2026-05-01) carry no evidence link and are
//! deliberately not checked here: they predate the anchor table, and inventing entries for them
//! would fabricate the evidence the rule exists to demand.

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

fn read(name: &str) -> String {
    let path = learning_dir().join(name);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

#[test]
fn parses_entries_from_both_sides() {
    // The two assertions below are set differences, which pass trivially when one side is
    // empty. A parser that silently matched nothing would report a healthy file forever.
    assert!(
        !index_anchors(&read("lessons.md")).is_empty(),
        "parsed no linked rules out of lessons.md"
    );
    assert!(
        !evidence_slugs(&read("lessons-evidence.md")).is_empty(),
        "parsed no entries out of lessons-evidence.md"
    );
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
