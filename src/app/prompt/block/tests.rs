//! 补充块插入落点、撤销与视口滚动测试。

use super::*;
use crate::tiling::PaneId;

fn prompt_with(width: u16, height: u16, text: &str) -> Prompt {
    let mut p = Prompt::new(PaneId::from_raw_for_test(1));
    p.resize(width, height);
    p.set_text(text);
    p
}

fn trigger_supplement(p: &mut Prompt) {
    for ch in "/supplement".chars() {
        p.insert_char(ch);
    }
    assert!(p.slash_panel().is_some(), "斜杠面板应打开");
    assert!(p.slash_confirm(), "确认 /supplement");
}

#[test]
fn supplement_in_empty_document_replaces_entire_text_and_positions_cursor_in_title() {
    let mut p = prompt_with(60, 20, "");
    trigger_supplement(&mut p);

    assert!(
        p.text().starts_with("+++ supplement --start\n{title: }\n"),
        "全文即补充块: {:?}",
        p.text()
    );
    assert!(
        p.text().ends_with("+++ supplement --end"),
        "以 --end 结尾: {:?}",
        p.text()
    );
    assert_eq!(&p.text()[p.cursor()..p.cursor() + 1], "}");
    assert!(p.text()[..p.cursor()].ends_with("{title: "));
}

#[test]
fn supplement_before_template_inserts_before_template_at_document_start() {
    let initial = "\n\n&&& addTemplate --start\n{title: T}\n\n- body\n&&& addTemplate --end";
    let mut p = prompt_with(60, 20, initial);
    p.set_cursor_offset(0);
    trigger_supplement(&mut p);

    assert!(
        p.text().starts_with("+++ supplement --start\n{title: }\n"),
        "新块插在 &&& 之前且文首无前导空行: {:?}",
        p.text()
    );
    assert!(
        p.text()
            .contains("+++ supplement --end\n\n&&& addTemplate --start"),
        "新块与 &&& 之间恰好空一行: {:?}",
        p.text()
    );
    assert_eq!(&p.text()[p.cursor()..p.cursor() + 1], "}");
}

#[test]
fn supplement_inside_template_inserts_before_first_supple() {
    let initial = concat!(
        "&&& addTemplate --start\n",
        "{title: T}\n",
        "\n",
        "&&& addTemplate --end\n",
        "\n",
        "+++ supplement --start\n",
        "{title: S1}\n",
        "\n",
        "+++ supplement --end\n",
        "\n",
        "+++ supplement --start\n",
        "{title: S2}\n",
        "\n",
        "+++ supplement --end",
    );
    let mut p = prompt_with(60, 30, initial);
    let inside = p.text().find("\n&&& addTemplate --end").unwrap();
    p.set_cursor_offset(inside);
    trigger_supplement(&mut p);

    let spans = markdown::block_spans(p.text());
    assert_eq!(spans.len(), 4, "1 个 &&& + 3 个 +++: {spans:?}");
    assert_eq!(spans[0].kind, BlockKind::Template);
    assert!(spans[1..].iter().all(|span| span.kind == BlockKind::Supple));
    assert!(
        !p.text()[..p.text().find("&&& addTemplate --end").unwrap()].contains("+++"),
        "&&& 内不应残留 +++"
    );
    assert!(
        p.text()
            .starts_with("&&& addTemplate --start\n{title: T}\n\n&&& addTemplate --end"),
        "&&& 内部触发行的空行保留: {:?}",
        p.text()
    );

    let template_end = p.text().find("&&& addTemplate --end").unwrap();
    let new_start = p.text().find("+++ supplement --start\n{title: }").unwrap();
    let s1_start = p
        .text()
        .find("+++ supplement --start\n{title: S1}")
        .unwrap();
    assert!(
        template_end < new_start && new_start < s1_start,
        "新块应插在 &&& 与第一个 +++ 之间: {:?}",
        p.text()
    );
    assert!(
        p.text()
            .contains("+++ supplement --end\n\n+++ supplement --start\n{title: S1}"),
        "新块与 S1 之间恰好空一行: {:?}",
        p.text()
    );
}

#[test]
fn supplement_inside_supple_inserts_before_next_supple() {
    let initial = concat!(
        "+++ supplement --start\n",
        "{title: S1}\n",
        "\n",
        "+++ supplement --end\n",
        "\n",
        "+++ supplement --start\n",
        "{title: S2}\n",
        "\n",
        "+++ supplement --end",
    );
    let mut p = prompt_with(60, 30, initial);
    let inside = p.text().find("{title: S1}\n").unwrap() + "{title: S1}\n".len();
    p.set_cursor_offset(inside);
    for ch in "/supplement".chars() {
        p.insert_char(ch);
    }
    let before_confirm = p.text().to_string();
    let cursor_before = p.cursor();
    assert!(p.slash_confirm());

    let spans = markdown::block_spans(p.text());
    assert_eq!(spans.len(), 3, "{spans:?}");
    assert!(spans.iter().all(|span| span.kind == BlockKind::Supple));
    assert!(
        spans[0].start < spans[0].end
            && spans[0].end < spans[1].start
            && spans[1].end < spans[2].start,
        "新块应位于两个既有补充块之间: {spans:?}"
    );

    let s1_end = p.text().find("+++ supplement --end").unwrap();
    let new_start = p.text().find("+++ supplement --start\n{title: }").unwrap();
    let s2_start = p
        .text()
        .find("+++ supplement --start\n{title: S2}")
        .unwrap();
    assert!(
        s1_end < new_start && new_start < s2_start,
        "新块应插在 S2 之前: {:?}",
        p.text()
    );
    assert!(
        p.text()
            .contains("{title: S1}\n\n+++ supplement --end\n\n+++ supplement --start\n{title: }"),
        "S1 内命令空行保留，新块与 S1 空一行: {:?}",
        p.text()
    );
    assert!(
        p.text()
            .contains("+++ supplement --end\n\n+++ supplement --start\n{title: S2}"),
        "新块与 S2 空一行: {:?}",
        p.text()
    );
    assert_eq!(&p.text()[p.cursor()..p.cursor() + 1], "}");
    assert!(p.text()[..p.cursor()].ends_with("{title: "));

    assert!(p.can_undo());
    p.undo();
    assert_eq!(p.text(), before_confirm);
    assert_eq!(p.cursor(), cursor_before);
}

#[test]
fn supplement_before_second_template_inserts_before_it() {
    let initial = concat!(
        "+++ supplement --start\n",
        "{title: S1}\n",
        "\n",
        "+++ supplement --end\n",
        "\n",
        "&&& addTemplate --start\n",
        "{title: T}\n",
        "\n",
        "&&& addTemplate --end",
    );
    let mut p = prompt_with(60, 30, initial);
    let inside = p.text().find("{title: S1}\n").unwrap() + "{title: S1}\n".len();
    p.set_cursor_offset(inside);
    trigger_supplement(&mut p);

    let spans = markdown::block_spans(p.text());
    assert_eq!(spans.len(), 3, "{spans:?}");
    assert_eq!(spans[0].kind, BlockKind::Supple);
    assert_eq!(spans[1].kind, BlockKind::Supple);
    assert_eq!(spans[2].kind, BlockKind::Template);

    let s1_end = p.text().find("+++ supplement --end").unwrap();
    let new_start = p.text().find("+++ supplement --start\n{title: }").unwrap();
    let template_start = p.text().find("&&& addTemplate --start").unwrap();
    assert!(
        s1_end < new_start && new_start < template_start,
        "下一个 &&& 也参与边界判断: {:?}",
        p.text()
    );
    assert!(
        p.text()
            .contains("+++ supplement --end\n\n&&& addTemplate --start"),
        "新块与 &&& 之间恰好空一行: {:?}",
        p.text()
    );
}

#[test]
fn supplement_between_blocks_keeps_plain_text_above_new_block() {
    let initial = concat!(
        "+++ supplement --start\n",
        "{title: S1}\n",
        "+++ supplement --end\n",
        "\n",
        "plain note\n",
        "\n",
        "+++ supplement --start\n",
        "{title: S2}\n",
        "+++ supplement --end",
    );
    let mut p = prompt_with(60, 30, initial);
    let inside = p.text().find("plain note\n").unwrap() + "plain note\n".len();
    p.set_cursor_offset(inside);
    trigger_supplement(&mut p);

    let plain = p.text().find("plain note").unwrap();
    let new_start = p.text().find("+++ supplement --start\n{title: }").unwrap();
    let s2_start = p
        .text()
        .find("+++ supplement --start\n{title: S2}")
        .unwrap();
    assert!(
        plain < new_start && new_start < s2_start,
        "普通文字留在新块上方: {:?}",
        p.text()
    );
    assert!(
        p.text()
            .contains("plain note\n\n+++ supplement --start\n{title: }"),
        "普通文字与新块之间空一行: {:?}",
        p.text()
    );
    assert!(
        p.text()
            .contains("+++ supplement --end\n\n+++ supplement --start\n{title: S2}"),
        "新块与 S2 之间空一行: {:?}",
        p.text()
    );
}

#[test]
fn supplement_after_last_block_appends_after_last_supple() {
    let initial = concat!(
        "+++ supplement --start\n",
        "{title: S1}\n",
        "\n",
        "+++ supplement --end\n",
        "\n",
        "+++ supplement --start\n",
        "{title: S2}\n",
        "\n",
        "+++ supplement --end",
    );
    let mut p = prompt_with(60, 30, initial);
    let inside = p.text().find("{title: S2}\n").unwrap() + "{title: S2}\n".len();
    p.set_cursor_offset(inside);
    trigger_supplement(&mut p);

    let spans = markdown::block_spans(p.text());
    assert_eq!(spans.len(), 3, "{spans:?}");
    assert!(spans.iter().all(|span| span.kind == BlockKind::Supple));

    let s2_end = p
        .text()
        .find("+++ supplement --end\n\n+++ supplement --start\n{title: }")
        .unwrap();
    let new_start = p.text().find("+++ supplement --start\n{title: }").unwrap();
    assert!(
        s2_end < new_start,
        "无下一个块时应追加到最后一个 +++ 之后: {:?}",
        p.text()
    );
    assert!(
        p.text().ends_with("+++ supplement --end"),
        "新块应位于文末: {:?}",
        p.text()
    );
}

#[test]
fn supplement_spacing_collapses_extra_blank_lines() {
    let initial = concat!(
        "+++ supplement --start\n",
        "{title: S1}\n",
        "\n",
        "+++ supplement --end\n",
        "\n",
        "\n",
        "\n",
        "+++ supplement --start\n",
        "{title: S2}\n",
        "\n",
        "+++ supplement --end",
    );
    let mut p = prompt_with(60, 30, initial);
    let inside = p.text().find("{title: S1}\n").unwrap() + "{title: S1}\n".len();
    p.set_cursor_offset(inside);
    trigger_supplement(&mut p);

    assert!(
        !p.text().contains("\n\n\n"),
        "插入点附近不得保留连续多空行: {:?}",
        p.text()
    );
    assert!(
        p.text()
            .contains("+++ supplement --end\n\n+++ supplement --start\n{title: }"),
        "新块前恰好一个空行: {:?}",
        p.text()
    );
    assert!(
        p.text()
            .contains("+++ supplement --end\n\n+++ supplement --start\n{title: S2}"),
        "新块后恰好一个空行: {:?}",
        p.text()
    );
}

#[test]
fn custom_supple_block_uses_same_insert_position() {
    let initial = concat!(
        "+++ supplement --start\n",
        "{title: S1}\n",
        "\n",
        "+++ supplement --end\n",
        "\n",
        "+++ supplement --start\n",
        "{title: S2}\n",
        "\n",
        "+++ supplement --end",
    );
    let mut p = prompt_with(60, 30, initial);
    p.set_custom_commands(vec![markdown::SlashItem::custom_supple(
        "extra",
        markdown::CommandSource::User,
        Some("Extra"),
        "supple body",
        "extra supplement",
    )]);
    let inside = p.text().find("{title: S1}\n").unwrap() + "{title: S1}\n".len();
    p.set_cursor_offset(inside);
    for ch in "/extra".chars() {
        p.insert_char(ch);
    }
    assert!(p.slash_confirm());

    let new_start = p
        .text()
        .find("+++ extra --start\n{title: Extra}")
        .expect("custom supple block");
    let s2_start = p
        .text()
        .find("+++ supplement --start\n{title: S2}")
        .unwrap();
    assert!(
        new_start < s2_start,
        "自定义补充块应使用同一落点: {:?}",
        p.text()
    );
    assert!(
        p.text()
            .contains("+++ extra --end\n\n+++ supplement --start\n{title: S2}"),
        "自定义块与 S2 之间空一行: {:?}",
        p.text()
    );
    assert_eq!(&p.text()[p.cursor()..p.cursor() + 1], "}");
}

#[test]
fn supplement_with_trailing_plain_text_inserts_between_block_and_tail_with_blank_lines() {
    let initial = "+++ supplement --start\n{title: S1}\n\n+++ supplement --end\nplain tail";
    let mut p = prompt_with(60, 20, initial);
    p.set_cursor_offset(p.text().len());
    p.insert_char('\n');
    trigger_supplement(&mut p);

    assert!(
        p.text()
            .contains("{title: S1}\n\n+++ supplement --end\n\n+++ supplement --start\n{title: }"),
        "前文与新块隔一空行: {:?}",
        p.text()
    );
    assert!(
        p.text().ends_with("+++ supplement --end\n\nplain tail\n"),
        "新块与后续正文隔一空行且触发处空行保留: {:?}",
        p.text()
    );
}

#[test]
fn supplement_when_anchor_already_followed_by_blank_line_does_not_double_blanks() {
    let initial = "&&& addTemplate --start\n{title: T}\n\n&&& addTemplate --end\n\nplain tail\n";
    let mut p = prompt_with(60, 20, initial);
    p.set_cursor_offset(p.text().len());
    trigger_supplement(&mut p);

    assert!(
        !p.text().contains("\n\n\n"),
        "不得产生两个连续空行: {:?}",
        p.text()
    );
    assert!(
        p.text()
            .contains("&&& addTemplate --end\n\n+++ supplement --start"),
        "{:?}",
        p.text()
    );
    assert!(
        p.text().ends_with("+++ supplement --end\n\nplain tail\n"),
        "{:?}",
        p.text()
    );
}

#[test]
fn single_undo_restores_command_line_and_removes_appended_block() {
    let initial = "&&& addTemplate --start\n{title: T}\n\n&&& addTemplate --end";
    let mut p = prompt_with(60, 20, initial);
    p.set_cursor_offset(0);
    p.insert_char('\n');
    p.set_cursor_offset(0);
    for ch in "/supplement".chars() {
        p.insert_char(ch);
    }
    let before_confirm = p.text().to_string();
    let cursor_before = p.cursor();

    assert!(p.slash_confirm());
    assert!(p.text().contains("+++ supplement --start"));

    assert!(p.can_undo());
    p.undo();
    assert_eq!(p.text(), before_confirm);
    assert_eq!(p.cursor(), cursor_before);
}

#[test]
fn unclosed_block_hides_supplement_command_and_skips_append() {
    let mut p = prompt_with(60, 20, "&&& addTemplate --start\n{title: T}\nbody");
    p.set_cursor_offset(p.text().len());
    p.insert_char('\n');
    for ch in "/supplement".chars() {
        p.insert_char(ch);
    }
    assert!(p.slash_panel().is_none(), "未闭合块下不提供 /supplement");
    assert!(!p.slash_confirm(), "无候选可确认");

    // 即使绕过面板直接追加也不插入：落点会落在未闭合块内部（命令行仅清空命令文字、保留空行）。
    let cursor = p.cursor();
    let trigger = markdown::SlashTrigger {
        line_start: cursor - "/supplement".len(),
        line_end: cursor,
        cursor,
        query: "supplement".to_string(),
    };
    let (content, cursor_in_block) = markdown::supple_block_content("supplement", None, "");
    p.insert_supple_block(&trigger, &content, cursor_in_block);
    assert_eq!(p.text(), "&&& addTemplate --start\n{title: T}\nbody\n");
    assert!(!p.text().contains("+++"), "未闭合块下不得追加补充块");
}

#[test]
fn supplement_scrolls_viewport_so_new_block_start_line_is_visible() {
    let mut lines = vec![
        "&&& addTemplate --start".to_string(),
        "{title: T}".to_string(),
    ];
    for i in 0..20 {
        lines.push(format!("- line {i}"));
    }
    lines.push("&&& addTemplate --end".to_string());
    let initial = format!("\n{}", lines.join("\n"));

    // 视口仅 6 行，小于文档总行数与补充块高度（9 行）。
    let mut p = prompt_with(60, 6, &initial);
    p.set_cursor_offset(0);
    p.scroll_to(0);
    trigger_supplement(&mut p);

    let spans = markdown::block_spans(p.text());
    let supple = spans
        .iter()
        .find(|span| span.kind == BlockKind::Supple)
        .expect("new supple span");
    let rows = p.visual_rows();
    let start_visual = rows
        .iter()
        .position(|row| row.line == supple.start)
        .expect("start visual row");
    assert!(
        start_visual >= p.scroll() && start_visual < p.scroll() + 6,
        "新块起始视觉行 {start_visual} 应在视口 [{}..{}) 内",
        p.scroll(),
        p.scroll() + 6
    );
}

#[test]
fn block_top_border_row_is_skipped_by_navigation_and_kept_in_view_when_scrolling_up() {
    let mut p = prompt_with(
        40,
        4,
        "header line\n&&& addTemplate --start\n{title: T}\nbody\n&&& addTemplate --end",
    );
    p.scroll_to(0);
    // 起始状态：row 0 为 "header line"，row 1 为 &&& 顶边，row 2 为 "&&& addTemplate --start"。
    p.set_cursor_from_cell(0, 3);
    assert_eq!(p.cursor_cell(), Some((0, 3)));

    // 从 header line 下移：跳过 row 1 顶边，直接落到 row 2 的起始文本行并保持列偏移。
    p.move_down();
    assert_eq!(
        p.cursor_cell(),
        Some((2, 4)),
        "下移跳过顶边并加上左色条 1 列"
    );

    // 从起始文本行上移：跳过 row 1 顶边，直接回到 row 0 的 header line。
    p.move_up();
    assert_eq!(p.cursor_cell(), Some((0, 3)), "上移跳过顶边回到上一文本行");

    // 点击顶边行（row 1）：光标归位到块起始文本行行首（row 2, col 1）。
    p.set_cursor_from_cell(1, 5);
    assert_eq!(
        p.cursor_cell(),
        Some((2, 1)),
        "点击顶边落到块起始文本行行首"
    );

    // 当块起始行在文档首行、视口向下滚动后上移回块起始行时，顶边行（row 0）一并滚入视口。
    let mut top_block = prompt_with(
        40,
        3,
        "&&& addTemplate --start\n{title: T}\nline1\nline2\n&&& addTemplate --end",
    );
    top_block.set_cursor_offset(top_block.text().len());
    assert!(top_block.scroll() > 0);
    top_block.set_cursor_offset(0);
    assert_eq!(top_block.scroll(), 0, "上移到块起始行时连同顶边行滚入视口");
    assert_eq!(top_block.cursor_cell(), Some((1, 1)));
}

#[test]
fn slash_title_only_visible_inside_template_and_supple_blocks() {
    // 块外：不含 /title
    let mut outside = prompt_with(80, 12, "/title");
    outside.set_cursor_offset(outside.text().len());
    let outside_names: Vec<&str> = outside
        .slash_panel()
        .map(|panel| {
            panel
                .items()
                .iter()
                .map(|item| item.name.as_str())
                .collect()
        })
        .unwrap_or_default();
    assert!(!outside_names.contains(&"title"), "块外不显示 /title");

    // &&& 块内：包含 /title
    let tpl =
        "&&& addTemplate --start\n{title: }\n\n- Description: fix auth\n/ti\n&&& addTemplate --end";
    let mut in_tpl = prompt_with(80, 12, tpl);
    let cursor = tpl.find("/ti").expect("slash") + 3;
    in_tpl.set_cursor_offset(cursor);
    let tpl_names: Vec<&str> = in_tpl
        .slash_panel()
        .expect("panel")
        .items()
        .iter()
        .map(|item| item.name.as_str())
        .collect();
    assert!(tpl_names.contains(&"title"), "&&& 块内应显示 /title");

    // +++ 块内：包含 /title
    let sup = "+++ supplement --start\n{title: }\n\n- Notes: add tests\n/ti\n+++ supplement --end";
    let mut in_sup = prompt_with(80, 12, sup);
    let sup_cursor = sup.find("/ti").expect("slash") + 3;
    in_sup.set_cursor_offset(sup_cursor);
    let sup_names: Vec<&str> = in_sup
        .slash_panel()
        .expect("panel")
        .items()
        .iter()
        .map(|item| item.name.as_str())
        .collect();
    assert!(sup_names.contains(&"title"), "+++ 块内应显示 /title");
}

#[test]
fn slash_title_preserves_empty_line_and_sets_generating_placeholder() {
    let text = "&&& addTemplate --start\n{title: Old}\n\n- Description: Add OAuth login\n/title\n&&& addTemplate --end";
    let line_count_before = text.lines().count();
    let mut p = prompt_with(80, 14, text);
    let cursor = text.find("/title").expect("slash") + "/title".len();
    p.set_cursor_offset(cursor);

    assert!(p.slash_confirm(), "确认 /title");
    assert_eq!(
        p.text().lines().count(),
        line_count_before,
        "确认 /title 后不得删除命令所在的空白行（总行数保持不变）"
    );
    assert_eq!(
        p.text(),
        "&&& addTemplate --start\n{title: Generating title...}\n\n- Description: Add OAuth login\n\n&&& addTemplate --end"
    );
    // 光标应停在被清空的原 /title 空行行首
    let expected_cursor = p
        .text()
        .find("- Description: Add OAuth login\n")
        .expect("desc")
        + "- Description: Add OAuth login\n".len();
    assert_eq!(p.cursor(), expected_cursor);

    let action = p.take_title_action().expect("title action");
    assert_eq!(
        action,
        crate::app::prompt::PromptTitleAction::Generate {
            block_start_line: 0,
            original_title: Some("Old".to_string()),
            content: "- Description: Add OAuth login".to_string(),
        }
    );
}

#[test]
fn slash_title_in_supple_block_targets_supple_and_preserves_empty_line() {
    let text = "&&& addTemplate --start\n{title: Main}\n\n- Description: Main task\n\n&&& addTemplate --end\n\n+++ supplement --start\n{title: }\n\n- Notes: Extra detail\n/title\n+++ supplement --end";
    let line_count_before = text.lines().count();
    let mut p = prompt_with(80, 18, text);
    let cursor = text.find("/title").expect("slash") + "/title".len();
    p.set_cursor_offset(cursor);

    assert!(p.slash_confirm(), "在 +++ 块内确认 /title");
    assert_eq!(p.text().lines().count(), line_count_before);
    assert!(
        p.text().contains("&&& addTemplate --start\n{title: Main}"),
        "外层 &&& 标题不受影响"
    );
    assert!(
        p.text()
            .contains("+++ supplement --start\n{title: Generating title...}"),
        "内层/补充 +++ 标题更新为 Generating title..."
    );
    let action = p.take_title_action().expect("title action");
    assert_eq!(
        action,
        crate::app::prompt::PromptTitleAction::Generate {
            block_start_line: 7,
            original_title: Some(String::new()),
            content: "- Description: Main task\n\n- Notes: Extra detail".to_string(),
        }
    );
}

#[test]
fn enclosing_template_body_finds_preceding_template_for_sibling_supple() {
    let text = concat!(
        "&&& first --start\n",
        "{title: First}\n",
        "\n",
        "- Description: First task\n",
        "&&& first --end\n",
        "\n",
        "&&& second --start\n",
        "{title: Second}\n",
        "\n",
        "- Description: Second task\n",
        "&&& second --end\n",
        "\n",
        "+++ supplement --start\n",
        "{title: }\n",
        "\n",
        "- Notes: Extra detail\n",
        "+++ supplement --end",
    );
    let p = prompt_with(60, 30, text);
    let supple_line = text[..text.find("+++ supplement --start").expect("supple")]
        .matches('\n')
        .count();
    assert_eq!(
        p.enclosing_template_body(supple_line).as_deref(),
        Some("{title: Second}\n\n- Description: Second task"),
        "兄弟 +++ 应关联最近的前置闭合 &&&"
    );
}

#[test]
fn slash_title_in_supplement_after_template_includes_outer_task() {
    let initial = "&&& addTemplate --start\n{title: Main}\n\n- Description: Outer task\n&&& addTemplate --end";
    let mut p = prompt_with(60, 24, initial);
    p.set_cursor_offset(p.text().len());
    p.insert_char('\n');
    trigger_supplement(&mut p);

    // 正常 /supplement 落点：+++ 是 &&& 的兄弟块；在块内补一行说明再触发 /title。
    let end = p.text().find("+++ supplement --end").expect("supple end");
    p.set_cursor_offset(end);
    p.insert_char('\n');
    p.set_cursor_offset(end);
    p.insert_str("- Notes: Extra detail\n");
    for ch in "/title".chars() {
        p.insert_char(ch);
    }

    assert!(p.slash_confirm(), "在追加的 +++ 内确认 /title");
    match p.take_title_action().expect("title action") {
        crate::app::prompt::PromptTitleAction::Generate {
            block_start_line,
            original_title,
            content,
        } => {
            assert_eq!(block_start_line, 6);
            assert_eq!(original_title, Some(String::new()));
            assert!(
                content.contains("- Description: Outer task"),
                "正常 /supplement 生成的兄弟 +++ 必须带回外层任务: {content}"
            );
            assert!(content.contains("- Notes: Extra detail"), "{content}");
            assert!(!content.contains("+++"), "{content}");
        }
        other => panic!("unexpected title action: {other:?}"),
    }
}

#[test]
fn slash_title_in_supple_of_second_template_includes_second_task() {
    let text = concat!(
        "&&& first --start\n",
        "{title: First}\n",
        "\n",
        "- Description: First task\n",
        "&&& first --end\n",
        "\n",
        "&&& second --start\n",
        "{title: Second}\n",
        "\n",
        "- Description: Second task\n",
        "\n",
        "+++ supplement --start\n",
        "{title: }\n",
        "\n",
        "- Notes: Detail\n",
        "/title\n",
        "+++ supplement --end\n",
        "\n",
        "&&& second --end",
    );
    let mut p = prompt_with(60, 30, text);
    let cursor = text.find("/title").expect("slash") + "/title".len();
    p.set_cursor_offset(cursor);

    assert!(p.slash_confirm(), "在第二个 &&& 的 +++ 内确认 /title");
    assert_eq!(
        p.take_title_action(),
        Some(crate::app::prompt::PromptTitleAction::Generate {
            block_start_line: 11,
            original_title: Some(String::new()),
            content: "- Description: Second task\n\n- Notes: Detail".to_string(),
        })
    );
}

#[test]
fn nested_supple_block_title_keeps_content_after_inner_end() {
    let text = concat!(
        "&&& addTemplate --start\n",
        "{title: Main}\n",
        "\n",
        "- Description: Outer task\n",
        "\n",
        "+++ supplement --start\n",
        "{title: }\n",
        "\n",
        "- Notes: Inner detail\n",
        "+++ inner --start\n",
        "- Notes: Nested hidden\n",
        "+++ inner --end\n",
        "- Notes: After nested\n",
        "/title\n",
        "+++ supplement --end\n",
        "\n",
        "&&& addTemplate --end",
    );
    let inner_cursor = text.find("- Notes: Nested hidden").expect("inner body");
    assert_eq!(
        markdown::find_enclosing_supple_block(text, inner_cursor),
        Some((9, 11, true)),
        "光标在内层 +++ 时应返回最内层块"
    );
    let outer_cursor = text.find("- Notes: After nested").expect("outer body");
    assert_eq!(
        markdown::find_enclosing_supple_block(text, outer_cursor),
        Some((5, 14, true))
    );

    let mut p = prompt_with(60, 30, text);
    let cursor = text.find("/title").expect("slash") + "/title".len();
    p.set_cursor_offset(cursor);

    assert!(p.slash_confirm(), "在嵌套 +++ 的外层内确认 /title");
    assert_eq!(
        p.take_title_action(),
        Some(crate::app::prompt::PromptTitleAction::Generate {
            block_start_line: 5,
            original_title: Some(String::new()),
            content: "- Description: Outer task\n\n- Notes: Inner detail\n- Notes: After nested"
                .to_string(),
        })
    );
}

#[test]
fn slash_title_in_nested_template_excludes_supple_and_log_bodies() {
    let text = "&&& addTemplate --start\n{title: Old}\n\n- Description: Build auth\n\n+++ supplement --start\n- Notes: hidden supple\n+++ supplement --end\n\n+++ log --start\n- Time: 10:00\n+++ log --end\n\n/title\n&&& addTemplate --end";
    let mut p = prompt_with(80, 20, text);
    let cursor = text.find("/title").expect("slash") + "/title".len();
    p.set_cursor_offset(cursor);

    assert!(p.slash_confirm(), "确认 /title");
    assert_eq!(
        p.take_title_action(),
        Some(crate::app::prompt::PromptTitleAction::Generate {
            block_start_line: 0,
            original_title: Some("Old".to_string()),
            content: "- Description: Build auth".to_string(),
        })
    );
}

#[test]
fn slash_title_in_nested_supple_uses_outer_template_and_own_body() {
    let text = "&&& addTemplate --start\n{title: Main}\n\n- Description: Outer task\n\n+++ supplement --start\n{title: }\n\n- Notes: Inner detail\n/title\n+++ supplement --end\n\n+++ sibling --start\n- Notes: Sibling detail\n+++ sibling --end\n\n&&& addTemplate --end";
    let mut p = prompt_with(80, 24, text);
    let cursor = text.find("/title").expect("slash") + "/title".len();
    p.set_cursor_offset(cursor);

    assert!(p.slash_confirm(), "在嵌套 +++ 内确认 /title");
    assert_eq!(
        p.take_title_action(),
        Some(crate::app::prompt::PromptTitleAction::Generate {
            block_start_line: 5,
            original_title: Some(String::new()),
            content: "- Description: Outer task\n\n- Notes: Inner detail".to_string(),
        })
    );
}

#[test]
fn slash_title_on_empty_boilerplate_block_keeps_empty_line_and_reports_empty() {
    let text = "&&& addTemplate --start\n{title: }\n\n# Add Requirement\n\n- Reference: \n- Description: \n/title\n&&& addTemplate --end";
    let line_count_before = text.lines().count();
    let mut p = prompt_with(80, 16, text);
    let cursor = text.find("/title").expect("slash") + "/title".len();
    p.set_cursor_offset(cursor);

    assert!(p.slash_confirm());
    assert_eq!(p.text().lines().count(), line_count_before);
    assert!(!p.text().contains("/title"));
    assert_eq!(
        p.take_title_action(),
        Some(crate::app::prompt::PromptTitleAction::EmptyBlock)
    );
}
