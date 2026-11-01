//! 补充块追加落点、撤销与视口滚动测试。

use super::*;
use crate::layout::PaneId;

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
fn supplement_typed_before_template_keeps_command_blank_line_and_appends_after_template() {
    let initial = "\n&&& addTemplate --start\n{title: T}\n\n- body\n&&& addTemplate --end";
    let mut p = prompt_with(60, 20, initial);
    p.set_cursor_offset(0);
    trigger_supplement(&mut p);

    assert!(
        p.text().starts_with("\n&&& addTemplate --start\n"),
        "原命令行只清空命令文字、保留空行: {:?}",
        p.text()
    );
    assert!(
        p.text()
            .contains("&&& addTemplate --end\n\n+++ supplement --start\n{title: }"),
        "块追加在 &&& 之后且恰好空一行: {:?}",
        p.text()
    );
    assert_eq!(&p.text()[p.cursor()..p.cursor() + 1], "}");
}

#[test]
fn supplement_typed_inside_template_appends_after_last_existing_supplement() {
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
    assert_eq!(spans[1].kind, BlockKind::Supple);
    assert_eq!(spans[2].kind, BlockKind::Supple);
    assert_eq!(spans[3].kind, BlockKind::Supple);
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
    assert!(
        p.text()
            .contains("{title: S2}\n\n+++ supplement --end\n\n+++ supplement --start\n{title: }"),
        "追加在第 2 个 +++ 之后且空一行: {:?}",
        p.text()
    );
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
    p.append_supple_block(&trigger, &content, cursor_in_block);
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
