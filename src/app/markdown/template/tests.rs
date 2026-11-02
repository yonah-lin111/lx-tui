//! 模板块协议单元测试；仅测试构建编译。

use super::*;
use crate::app::markdown::{
    SlashCommandId, slash_template_content, slash_trigger, template_counts,
};

#[test]
fn slash_trigger_suppressed_in_fence_but_allowed_inside_template_blocks() {
    assert_eq!(slash_trigger("```\n/add", 8), None);
    // 模板块内允许触发（`/gitWorkTree` 需要在块内绑定）；候选按作用域过滤。
    let block = "&&& addTemplate --start {title: }\n/add\n&&& addTemplate --end";
    let cursor = block.find("/add").expect("line") + "/add".len();
    assert_eq!(
        slash_trigger(block, cursor).map(|t| t.query),
        Some("add".into())
    );
    let after = format!("{block}\n/add");
    assert_eq!(
        slash_trigger(&after, after.len()).map(|t| t.query),
        Some("add".into())
    );
}

#[test]
fn slash_template_content_matches_protocol_and_cursor() {
    for id in SlashCommandId::ALL {
        let (content, cursor) = slash_template_content(id);
        let start = format!("&&& {} --start\n{{title: }}", id.long_name());
        let end = format!("&&& {} --end", id.long_name());
        assert!(content.starts_with(&start), "{content}");
        assert!(content.ends_with(&end), "{content}");
        assert_eq!(content.as_bytes().get(cursor - 1), Some(&b' '));
        assert!(content[cursor..].starts_with('}'), "光标落在 }} 之前");
        assert_eq!(
            content.lines().rev().nth(1),
            Some(""),
            "结束行上方固定一空行: {content}"
        );
        assert!(
            parse_template_start_line(content.lines().next().expect("start line")).is_some(),
            "起止行仍是合法开始行"
        );
    }
    let (add, _) = slash_template_content(SlashCommandId::Add);
    assert!(add.contains("# Add Requirement"));
    assert!(add.contains("- Requirements: \n  - "));
}

#[test]
fn parse_template_start_line_forms() {
    let parsed =
        parse_template_start_line("&&& addTemplate --start {title: 标题}").expect("start line");
    assert_eq!(parsed.command, "addTemplate");
    assert_eq!(parsed.title, Some(" 标题"));
    let parsed = parse_template_start_line("  &&& bugTemplate").expect("short form");
    assert_eq!(parsed.indent, "  ");
    assert_eq!(parsed.title, None);
    assert_eq!(
        parse_template_start_line("&&& commonTemplate --start").map(|p| p.title),
        Some(None)
    );

    assert_eq!(parse_template_start_line("&&& addTemplate --end"), None);
    assert_eq!(
        parse_template_start_line("&&& done"),
        None,
        "保留词不算起始行"
    );
    assert_eq!(
        parse_template_start_line("&&& suppleTemplate --start"),
        None
    );
    assert_eq!(parse_template_start_line("&&&addTemplate"), None);
    assert_eq!(parse_template_start_line("&&& 1bad --start"), None);
    assert_eq!(
        parse_template_start_line("&&& addTemplate --start junk"),
        None
    );
    assert_eq!(
        parse_template_start_line("&&& addTemplate --start {title: a} trailing"),
        None
    );
}

#[test]
fn parse_template_end_line_forms() {
    let parsed =
        parse_template_end_line("&&& addTemplate --end done {id:abc} {wt:main}").expect("end line");
    assert_eq!(parsed.command, Some("addTemplate"));
    assert!(parsed.end_flag);
    assert_eq!(parsed.status, TemplateStatus::Done);
    assert_eq!(parsed.id, Some("abc"));
    assert_eq!(parsed.wt, Some("main"));
    assert_eq!(
        parse_template_end_line("&&& --end in_progress")
            .expect("no command")
            .status,
        TemplateStatus::InProgress
    );
    assert_eq!(
        parse_template_end_line("&&&").expect("bare marker").status,
        TemplateStatus::Todo
    );
    assert_eq!(
        parse_template_end_line("&&& done").map(|p| p.status),
        Some(TemplateStatus::Done)
    );

    assert_eq!(
        parse_template_end_line("&&& addTemplate"),
        None,
        "缺 --end 不是结束行"
    );
    assert_eq!(parse_template_end_line("&&& addTemplate --start"), None);
    assert_eq!(parse_template_end_line("&&& --end junk"), None);
    assert_eq!(parse_template_end_line("&&& --end --end"), None);
}

#[test]
fn parse_title_line_forms() {
    assert_eq!(parse_title_line("{title: My Title}"), Some(" My Title"));
    assert_eq!(parse_title_line("  {title: }  "), Some(" "));
    assert_eq!(parse_title_line("{title:My}"), Some("My"));
    assert_eq!(parse_title_line("{title: a} trailing"), None);
    assert_eq!(parse_title_line("{title: a{b}}"), None);
    assert_eq!(parse_title_line("{wt:main}"), None);
    assert_eq!(parse_title_line("&&& addTemplate --end"), None);
}

#[test]
fn parse_worktree_line_forms() {
    assert_eq!(parse_worktree_line("{wt:feature/x}"), Some("feature/x"));
    assert_eq!(parse_worktree_line("  {wt:main}  "), Some("main"));
    assert_eq!(parse_worktree_line("{wt:}"), None);
    assert_eq!(parse_worktree_line("{wt:a b}"), None);
    assert_eq!(parse_worktree_line("body {wt:main}"), None);
    assert_eq!(parse_worktree_line("{wt:a{b}}"), None);
    assert_eq!(parse_worktree_line("&&& addTemplate --end"), None);
}

#[test]
fn edit_touches_title_line_protects_marker_and_allows_value() {
    let text = "&&& x --start\n{title: 标题}\n\nbody\n&&& x --end\n";
    let start = text.find("{title:").expect("title line");
    let prefix_end = start + "{title:".len();
    let brace = start + "{title: 标题}".len() - 1;
    let line_end = brace + 1;
    // 值内编辑放行（含 `}` 前与 `:` 后）。
    assert!(!edit_touches_title_line(text, prefix_end, prefix_end));
    assert!(!edit_touches_title_line(text, brace, brace));
    assert!(!edit_touches_title_line(
        text,
        prefix_end + 1,
        prefix_end + 2
    ));
    assert!(!edit_touches_title_line(text, line_end, line_end));
    // 前缀、收尾 `}` 与相邻换行被拒。
    assert!(edit_touches_title_line(text, start, start + 1));
    assert!(edit_touches_title_line(text, start, prefix_end));
    assert!(edit_touches_title_line(text, prefix_end - 1, prefix_end));
    assert!(edit_touches_title_line(text, brace, brace + 1));
    // 删除值末尾字符（`}` 之前）放行。
    assert!(!edit_touches_title_line(text, brace - 1, brace));
    assert!(edit_touches_title_line(text, start, start));
    assert!(edit_touches_title_line(text, start, line_end));
    assert!(edit_touches_title_line(text, line_end, line_end + 1));
    assert!(edit_touches_title_line(text, start - 1, start));
    // 严格覆盖整行以上范围（清空全文、删整块）放行。
    assert!(!edit_touches_title_line(text, 0, text.len()));
    // 非结构性标题行（不在模板块起始行下一行）不受保护。
    let loose = "正文\n{title: 不是结构行}\n";
    let loose_start = loose.find("{title:").expect("title line");
    assert!(!edit_touches_title_line(
        loose,
        loose_start,
        loose_start + 2
    ));
}

#[test]
fn parse_template_block_finds_range_and_unclosed_tail() {
    let text = "a\n&&& addTemplate --start {title: }\n# Add\n&&& addTemplate --end\nb";
    assert_eq!(
        parse_template_block_at_line(text, 1),
        Some(TemplateBlockRange { start: 1, end: 3 })
    );
    assert_eq!(parse_template_block_at_line(text, 0), None);
    assert_eq!(parse_template_block_at_line(text, 2), None);
    let unclosed = "&&& bugTemplate --start {title: }\n# Fix";
    assert_eq!(
        parse_template_block_at_line(unclosed, 0),
        Some(TemplateBlockRange { start: 0, end: 1 })
    );
}

#[test]
fn cycle_template_status_cycles_and_preserves_metadata() {
    assert_eq!(
        cycle_template_status("&&& addTemplate --end").as_deref(),
        Some("&&& addTemplate --end in_progress")
    );
    assert_eq!(
        cycle_template_status("&&& addTemplate --end in_progress").as_deref(),
        Some("&&& addTemplate --end done")
    );
    assert_eq!(
        cycle_template_status("&&& addTemplate --end done").as_deref(),
        Some("&&& addTemplate --end")
    );
    assert_eq!(
        cycle_template_status("  &&& bugTemplate --end done {id:ff} {wt:dev}").as_deref(),
        Some("  &&& bugTemplate --end {id:ff} {wt:dev}")
    );
    assert_eq!(
        cycle_template_status("&&& done").as_deref(),
        Some("&&&"),
        "done 循环回 todo 时移除状态后缀"
    );
    assert_eq!(cycle_template_status("&&& addTemplate"), None);
    assert_eq!(cycle_template_status("plain"), None);
}

#[test]
fn clean_template_content_drops_unfilled_items() {
    let content = "# Add Requirement\n\n- Reference: \n- Location: \n- Description: \n- Requirements: \n  - \n- Notes: \n  - ";
    let cleaned = clean_template_content(content);
    assert_eq!(cleaned, "# Add Requirement");
}

#[test]
fn clean_template_content_keeps_filled_children() {
    let content = "- Requirements: \n  - keep me\n- Notes: \n  - \n- Location: here";
    let cleaned = clean_template_content(content);
    assert_eq!(cleaned, "- Requirements: \n  - keep me\n- Location: here");
}

#[test]
fn clean_template_content_preserves_subblocks_and_collapses_blanks() {
    let content = "- Reference: \n+++ suppleTemplate --start {title: }\n- Requirements: \n  - \n+++ suppleTemplate --end\n\n\n\n- Location: x";
    let cleaned = clean_template_content(content);
    assert!(cleaned.contains("+++ suppleTemplate --start"));
    assert!(cleaned.contains("- Requirements: \n  - \n+++ suppleTemplate --end"));
    assert!(!cleaned.contains("\n\n\n"), "折叠多余空行");
    assert!(cleaned.ends_with("- Location: x"));
}

#[test]
fn ensure_bottom_gap_keeps_binding_and_is_idempotent() {
    assert_eq!(ensure_bottom_gap("body".to_string()), "body\n");
    assert_eq!(ensure_bottom_gap("body\n".to_string()), "body\n");
    assert_eq!(
        ensure_bottom_gap("body\n\n{wt:main}".to_string()),
        "body\n\n{wt:main}",
        "绑定行为末行时不补空行"
    );
    assert_eq!(ensure_bottom_gap(String::new()), "");
    assert_eq!(ensure_bottom_gap("\n".to_string()), "");
}

#[test]
fn copy_template_content_strips_title_worktree_and_log_markers() {
    let content = "{title: 标题}\n# Add Requirement\n\n%%% logTemplate --start {title: }\n- Time: 10:00\n%%% logTemplate --end\n{wt:main}\n- Location: here";
    let copied = copy_template_content(content);
    assert!(
        copied.starts_with("# Add Requirement"),
        "标题占位行不进复制内容: {copied}"
    );
    assert!(!copied.contains("%%%"), "记录子块标记剔除: {copied}");
    assert!(!copied.contains("{wt:main}"), "绑定行剔除: {copied}");
    assert!(
        copied.contains("- Time: 10:00"),
        "记录子块内容保留: {copied}"
    );
    assert!(copied.ends_with("- Location: here"));
}

#[test]
fn copy_template_content_drops_comments_and_unfilled_items() {
    let content =
        "# Add\n// 注释行\n- Reference: \n  // 子注释\n- Location: here\n- Requirements: \n  - \n";
    let copied = copy_template_content(content);
    assert_eq!(copied, "# Add\n- Location: here");
}

#[test]
fn copy_template_content_keeps_legacy_log_blocks() {
    let content = "- Location: x\n+++ log --start\n- Records: a\n+++ log --end";
    let copied = copy_template_content(content);
    assert_eq!(copied, "- Location: x\n- Records: a");
}

#[test]
fn template_block_start_at_finds_enclosing_block() {
    let text = "a\n&&& addTemplate --start\n{title: }\nbody\n&&& addTemplate --end\nb";
    assert_eq!(template_block_start_at(text, 0), None, "块外行");
    assert_eq!(template_block_start_at(text, 2), Some(1), "起始行");
    assert_eq!(
        template_block_start_at(text, text.find("body").expect("body")),
        Some(1),
        "正文行"
    );
    assert_eq!(
        template_block_start_at(text, text.find("--end").expect("end")),
        Some(1),
        "结束行"
    );
    assert_eq!(template_block_start_at(text, text.len()), None, "块后行");
}

#[test]
fn template_line_infos_mark_roles_and_status() {
    let text = "a\n&&& addTemplate --start {title: }\n# Add\n&&& addTemplate --end done\nb";
    let infos = template_line_infos(text);
    assert_eq!(infos.len(), 5);
    assert_eq!(infos[0], None);
    assert_eq!(
        infos[1],
        Some(TemplateLineInfo {
            role: TemplateLineRole::Start,
            status: TemplateStatus::Done,
            separator: false,
        })
    );
    assert_eq!(
        infos[2],
        Some(TemplateLineInfo {
            role: TemplateLineRole::Middle,
            status: TemplateStatus::Done,
            separator: false,
        })
    );
    assert_eq!(
        infos[3],
        Some(TemplateLineInfo {
            role: TemplateLineRole::End,
            status: TemplateStatus::Done,
            separator: false,
        })
    );
    assert_eq!(infos[4], None);
}

#[test]
fn template_line_infos_mark_separator_rows() {
    // 标题下空行与绑定行上空行标记为结构分割行；正文中的空行不标记。
    let text = "&&& x --start\n{title: T}\n\n# Head\n\n- a\n\n{wt:main}\n&&& x --end\n";
    let infos = template_line_infos(text);
    let separator = |index: usize| infos[index].is_some_and(|info| info.separator);
    assert!(separator(2), "标题下空行: {infos:?}");
    assert!(!separator(4), "正文空行: {infos:?}");
    assert!(separator(6), "绑定行上空行: {infos:?}");
    assert!(!separator(1) && !separator(3));
}

#[test]
fn template_line_infos_unclosed_block_extends_to_end() {
    let text = "&&& bugTemplate --start {title: }\n# Fix";
    let infos = template_line_infos(text);
    assert_eq!(
        infos[0],
        Some(TemplateLineInfo {
            role: TemplateLineRole::Start,
            status: TemplateStatus::Todo,
            separator: false,
        })
    );
    assert_eq!(
        infos[1],
        Some(TemplateLineInfo {
            role: TemplateLineRole::Middle,
            status: TemplateStatus::Todo,
            separator: false,
        })
    );
}

#[test]
fn is_template_block_line_checks_membership_and_bounds() {
    let text = "a\n&&& addTemplate --start {title: }\nbody\n&&& addTemplate --end\nb";
    assert!(!is_template_block_line(text, 0));
    assert!(is_template_block_line(text, 1));
    assert!(is_template_block_line(text, 2));
    assert!(is_template_block_line(text, 3));
    assert!(!is_template_block_line(text, 4));
    assert!(!is_template_block_line(text, 99), "越界行不算块内");
    let unclosed = "&&& bugTemplate --start {title: }\n# Fix";
    assert!(is_template_block_line(unclosed, 1));
}

// ===== 模板块状态计数（分页统计） =====

#[test]
fn template_counts_only_first_closed_top_level_template() {
    let text = "\
&&& bugTemplate --start
&&& bugTemplate --end in_progress
+++ noteSupple --start
+++ noteSupple --end
&&& refactorTemplate --start
plain tail
&&& refactorTemplate --end done
```
&&& commonTemplate --start
&&& commonTemplate --end
```";
    let counts = template_counts(text);
    assert_eq!(counts.todo, 0);
    assert_eq!(
        counts.in_progress, 1,
        "只统计首个闭合顶层 &&&，第二个 &&&、+++ 与围栏不计数"
    );
    assert_eq!(counts.done, 0);
}

#[test]
fn template_counts_exclude_unclosed_blocks() {
    let counts = template_counts("&&& addTemplate --start\nbody without end");
    assert!(counts.is_empty(), "未闭合块不纳入完成情况");
    let mixed =
        template_counts("&&& addTemplate --start\n&&& addTemplate --end\n&&& bugTemplate --start");
    assert_eq!(mixed.todo, 1, "闭合块计一次；未闭合块忽略");
}

#[test]
fn template_counts_merge_sums_pages() {
    let left = template_counts("&&& addTemplate --start\n&&& addTemplate --end");
    let right = template_counts("&&& bugTemplate --start\n&&& bugTemplate --end done");
    let merged = left.merge(right);
    assert_eq!(merged.todo, 1);
    assert_eq!(merged.done, 1);
    assert!(!merged.is_empty());
}

// ===== 模板块标题定位与 Fork 正文 =====

/// 断言 `title` 在 `text` 中的字节区间被识别为首个模板块标题值。
fn assert_title_span(text: &str, title: &str) {
    let value_start = text.find(title).expect("title value");
    assert_eq!(
        find_first_template_title_span(text),
        TemplateTitleLookup::Found {
            value_start,
            value_end: value_start + title.len(),
        }
    );
}

#[test]
fn find_first_template_title_span_prefers_standalone_title_line() {
    let text = "&&& addTemplate --start\n  {title: 登录接口重构}\n\nbody\n&&& addTemplate --end\n";
    assert_title_span(text, "登录接口重构");
}

#[test]
fn find_first_template_title_span_reads_inline_title() {
    let text = "&&& addTemplate --start {title: Inline Title}\nbody";
    assert_title_span(text, "Inline Title");
}

#[test]
fn find_first_template_title_span_trims_padding() {
    let text = "&&& addTemplate --start\n{title:   padded   }\n&&& addTemplate --end\n";
    assert_title_span(text, "padded");
    let (start, end) = title_value_byte_range("{title: a}", 0).expect("range");
    assert_eq!((start, end), (8, 9));
}

#[test]
fn find_first_template_title_span_empty_value_points_at_brace() {
    let text = "&&& addTemplate --start\n{title: }\n&&& addTemplate --end\n";
    let brace = text.find('}').expect("closing brace");
    assert_eq!(
        find_first_template_title_span(text),
        TemplateTitleLookup::Found {
            value_start: brace,
            value_end: brace,
        }
    );
}

#[test]
fn find_first_template_title_span_reports_missing_title_line() {
    let text = "&&& addTemplate --start\nbody\n&&& addTemplate --end\n";
    assert_eq!(
        find_first_template_title_span(text),
        TemplateTitleLookup::MissingTitleLine {
            start_line_end: "&&& addTemplate --start".len(),
        }
    );
}

#[test]
fn find_first_template_title_span_without_template_block() {
    assert_eq!(
        find_first_template_title_span("plain text\n{title: loose}\n"),
        TemplateTitleLookup::NoTemplateBlock
    );
    assert_eq!(
        find_first_template_title_span(""),
        TemplateTitleLookup::NoTemplateBlock
    );
}

#[test]
fn fork_page_content_appends_suffix_to_nonempty_title() {
    let standalone = "&&& addTemplate --start\n{title: Refactor}\n\nbody\n&&& addTemplate --end\n";
    assert_eq!(
        fork_page_content(standalone),
        "&&& addTemplate --start\n{title: Refactor (fork)}\n\nbody\n&&& addTemplate --end\n"
    );
    assert_eq!(
        fork_page_content("&&& addTemplate --start {title: Inline}\nbody"),
        "&&& addTemplate --start {title: Inline (fork)}\nbody"
    );
    // 追加紧贴标题值末尾，原有尾随空白保持不变。
    assert_eq!(
        fork_page_content("&&& x --start\n{title:  Padded  }\n&&& x --end"),
        "&&& x --start\n{title:  Padded (fork)  }\n&&& x --end"
    );
}

#[test]
fn fork_page_content_copies_when_title_empty_or_missing() {
    for text in [
        "",
        "plain text",
        "&&& addTemplate --start\n{title: }\n&&& addTemplate --end\n",
        "&&& addTemplate --start\nbody\n&&& addTemplate --end\n",
    ] {
        assert_eq!(fork_page_content(text), text);
    }
}
