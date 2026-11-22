//! `deck guide` and the agent skill's front matter.

mod support;

use support::{Home, stderr, stdout};

const SKILL: &str = include_str!("../../../skills/deck/SKILL.md");

/// The `key: value` lines of the front matter.
fn front_matter() -> Vec<(&'static str, &'static str)> {
    let rest = SKILL.strip_prefix("---\n").unwrap_or_default();
    let (head, _) = rest.split_once("\n---\n").unwrap_or_default();
    head.lines().filter_map(|l| l.split_once(": ")).collect()
}

fn field(key: &str) -> &'static str {
    front_matter()
        .into_iter()
        .find(|(k, _)| *k == key)
        .map(|(_, v)| v)
        .unwrap_or_default()
}

#[test]
fn the_skill_front_matter_is_valid() {
    let name = field("name");
    assert_eq!(name, "deck");
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../skills/deck");
    assert!(dir.join("SKILL.md").is_file());
    let dir = dir.canonicalize().unwrap_or_default();
    assert_eq!(
        dir.file_name().and_then(|n| n.to_str()),
        Some(name),
        "the name matches the directory"
    );
    assert!(
        !name.is_empty()
            && name
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
        "{name}"
    );
    let description = field("description");
    assert!(!description.is_empty());
    assert!(description.chars().count() <= 1024, "{}", description.len());
    assert!(description.contains("DECK"));
    assert!(SKILL.lines().count() <= 160, "the body stays short");
}

#[test]
fn guide_prints_the_body_without_front_matter_and_needs_no_daemon() {
    let home = Home::new();
    let out = home.run_deck(&["guide"]);
    assert!(out.status.success(), "{}", stderr(&out));
    let text = stdout(&out);
    let body = SKILL
        .split_once("\n---\n")
        .map(|(_, b)| b.trim_start_matches('\n'))
        .unwrap_or_default();
    assert_eq!(text, body);
    assert!(!text.contains("description:"));
    assert!(!home.daemon_running());
}
