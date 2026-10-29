use super::*;
use crate::skills::{SkillScope, SkillSource};
use std::path::PathBuf;

fn entry(name: &str, description: &str) -> SkillEntry {
    SkillEntry {
        name: name.into(),
        description: description.into(),
        path: PathBuf::from(format!("/skills/{name}/SKILL.md")),
        scope: SkillScope::User,
        source: SkillSource::Lx,
        enabled: true,
        shadowed_by: None,
    }
}

#[test]
fn catalog_contains_usage_and_file_paths() {
    let text = render_catalog(&[entry("pdf", "Read PDF files.")], false).expect("catalog");
    for needle in [
        "## Skills",
        "### Available skills",
        "- pdf: Read PDF files. (file: /skills/pdf/SKILL.md)",
        "### How to use skills",
        "`read`",
        "`spawn_agent`",
        "Announce which skill you are using",
    ] {
        assert!(text.contains(needle), "缺少 `{needle}`：{text}");
    }
}

#[test]
fn subagent_catalog_replaces_delegation_policy() {
    let text = render_catalog(&[entry("pdf", "Read PDF files.")], true).expect("catalog");
    assert!(text.contains("If your task names a skill"));
    assert!(!text.contains("`spawn_agent`"));
}

#[test]
fn disabled_and_shadowed_entries_are_omitted() {
    let mut hidden = entry("hidden", "x");
    hidden.enabled = false;
    let mut shadowed = entry("shadow", "x");
    shadowed.shadowed_by = Some(PathBuf::from("/first/SKILL.md"));
    assert!(render_catalog(&[hidden, shadowed], false).is_none());
}

#[test]
fn budget_drops_whole_lines_and_reports_omitted_count() {
    let entries: Vec<SkillEntry> = (0..400)
        .map(|index| entry(&format!("skill{index}"), &"d".repeat(200)))
        .collect();
    let text = render_catalog(&entries, false).expect("catalog");
    assert!(text.chars().count() <= MAX_CATALOG_CHARS + 80);
    assert!(text.contains("additional skills omitted."));
}

#[test]
fn injection_truncates_on_a_char_boundary() {
    let (block, truncated) = render_injection("big", Path::new("/x/SKILL.md"), "abcdef", 4);
    assert!(truncated);
    assert!(block.contains("abcd"));
    assert!(block.contains("read the file for the rest"));
    assert!(block.starts_with("<skill>\n<name>big</name>"));
}
