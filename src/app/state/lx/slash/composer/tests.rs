//! 详情命令模型的纯状态测试：候选集合、整段解析、别名与参数提示。

use super::super::{DetailCommand, SlashEntry, SlashPanel};
use super::{DetailContext, ParsedDetailCommand, entries, parse};

fn names(context: DetailContext, query: &str) -> Vec<&'static str> {
    entries(context, query)
        .into_iter()
        .map(|entry| match entry {
            SlashEntry::Detail(command) => command.text(),
            SlashEntry::Command(command) => command.text(),
        })
        .collect()
}

#[test]
fn agent_context_lists_rewind_compact_report_only() {
    assert_eq!(
        names(DetailContext::Agent, ""),
        vec!["/rewind", "/compact", "/report"]
    );
    assert_eq!(names(DetailContext::Agent, "/rep"), vec!["/report"]);
    assert!(!names(DetailContext::Agent, "").contains(&"/new"));
}

#[test]
fn btw_context_lists_rewind_compact_new_only() {
    assert_eq!(
        names(DetailContext::Btw, ""),
        vec!["/rewind", "/compact", "/new"]
    );
    assert!(!names(DetailContext::Btw, "").contains(&"/report"));
}

#[test]
fn report_takes_multiline_note() {
    assert_eq!(
        parse(
            "/report check the log\nand report back",
            DetailContext::Agent
        ),
        Some(ParsedDetailCommand::Report(
            "check the log\nand report back"
        ))
    );
    assert_eq!(
        parse("/report", DetailContext::Agent),
        Some(ParsedDetailCommand::Report(""))
    );
    assert_eq!(
        parse("  /report  spaced note  ", DetailContext::Agent),
        Some(ParsedDetailCommand::Report("spaced note"))
    );
}

#[test]
fn non_report_commands_reject_arguments() {
    assert_eq!(parse("/rewind x", DetailContext::Agent), None);
    assert_eq!(parse("/compact now", DetailContext::Agent), None);
    assert_eq!(parse("/rewind\nsecond line", DetailContext::Agent), None);
    assert_eq!(
        parse("/rewind", DetailContext::Agent),
        Some(ParsedDetailCommand::Rewind)
    );
    // `/undo` 保留为与主输入一致的隐藏别名。
    assert_eq!(
        parse("/undo", DetailContext::Agent),
        Some(ParsedDetailCommand::Rewind)
    );
    assert_eq!(
        parse("/compact", DetailContext::Btw),
        Some(ParsedDetailCommand::Compact)
    );
    // 命令词必须完整：`/rewinds` 不是 `/rewind`。
    assert_eq!(parse("/rewinds", DetailContext::Agent), None);
}

#[test]
fn btw_new_accepts_clear_and_reset_aliases() {
    assert_eq!(
        parse("/new", DetailContext::Btw),
        Some(ParsedDetailCommand::New)
    );
    assert_eq!(
        parse("/clear", DetailContext::Btw),
        Some(ParsedDetailCommand::New)
    );
    assert_eq!(
        parse("/reset", DetailContext::Btw),
        Some(ParsedDetailCommand::New)
    );
    // 别名只属于 BTW。
    assert_eq!(parse("/new", DetailContext::Agent), None);
}

#[test]
fn argument_hint_only_for_report_prefix() {
    assert_eq!(
        super::argument_hint("/report "),
        Some(DetailCommand::Report.argument_hint().unwrap_or(""))
    );
    assert_eq!(super::argument_hint("/report  "), Some("[note]"));
    assert_eq!(super::argument_hint("/report"), None);
    assert_eq!(super::argument_hint("/report note"), None);
    assert_eq!(super::argument_hint("/rewind "), None);
}

#[test]
fn main_context_does_not_expose_detail_commands() {
    assert!(SlashPanel::command_entries("report").is_empty());
    assert!(SlashPanel::command_entries("newer").is_empty());
    // 主输入 `/undo` 仍是 `/rewind` 的别名。
    assert_eq!(
        SlashPanel::command_entries("undo"),
        vec![SlashEntry::Command(crate::app::state::SlashCommand::Rewind)]
    );
}
