//! prompt 块渲染测试：色条 `▎`、分割细线 `─` 与起始行按钮组。

use super::*;
use crate::layout::PaneId;
use ratatui::style::Color;

fn prompt_at(cols: u16, rows: u16, content: &str) -> Prompt {
    let mut prompt = Prompt::new(PaneId::from_raw_for_test(1));
    prompt.resize(cols, rows);
    prompt.set_text(content);
    prompt
}

#[test]
fn template_block_paints_left_bar_on_every_visual_row_with_status_color() {
    let area = Rect::new(0, 0, 12, 8);
    // "longbodyline1234" 在宽 12（扣除 1 列色条后可用 11 列）下软换行为 2 个视觉行。
    let todo = prompt_at(
        12,
        8,
        "&&& addTemplate --start\nlongbodyline1234\n&&& addTemplate --end",
    );
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &todo, None);
    for y in 0..4 {
        assert_eq!(
            buf[(0, y)].symbol(),
            text::PROMPT_BLOCK_BAR,
            "视觉行 {y} 首列应画色条"
        );
        assert_eq!(buf[(0, y)].fg, style::LX_PINK, "&&& 色条固定 lx 粉");
    }

    let in_progress = prompt_at(
        12,
        8,
        "&&& addTemplate --start\nbody\n&&& addTemplate --end in_progress",
    );
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &in_progress, None);
    for y in 0..3 {
        assert_eq!(buf[(0, y)].symbol(), text::PROMPT_BLOCK_BAR);
        assert_eq!(buf[(0, y)].fg, style::LX_PINK, "&&& 色条固定 lx 粉");
    }

    let done = prompt_at(
        12,
        8,
        "&&& addTemplate --start\nbody\n&&& addTemplate --end done",
    );
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &done, None);
    for y in 0..3 {
        assert_eq!(buf[(0, y)].symbol(), text::PROMPT_BLOCK_BAR);
        assert_eq!(buf[(0, y)].fg, style::LX_PINK, "&&& 色条固定 lx 粉");
    }
}

#[test]
fn supple_block_paints_magenta_left_bar() {
    let area = Rect::new(0, 0, 30, 5);
    let prompt = prompt_at(
        30,
        5,
        "+++ supplement --start\n{title: S}\n- item\n+++ supplement --end",
    );
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &prompt, None);
    for y in 0..4 {
        assert_eq!(buf[(0, y)].symbol(), text::PROMPT_BLOCK_BAR);
        assert_eq!(buf[(0, y)].fg, Color::LightMagenta, "+++ 色条固定品红");
    }
}

#[test]
fn separator_row_paints_bar_then_full_horizontal_rule_without_join_glyphs() {
    let area = Rect::new(0, 0, 24, 6);
    let prompt = prompt_at(
        24,
        6,
        "&&& x --start\n{title: T}\n\nbody\n&&& x --end in_progress",
    );
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &prompt, None);
    assert_eq!(buf[(0, 2)].symbol(), text::PROMPT_BLOCK_BAR);
    assert_eq!(buf[(0, 2)].fg, style::LX_PINK);
    for x in 1..area.width {
        assert_eq!(buf[(x, 2)].symbol(), "─", "列 {x} 应为 ─，不含 ├/┤");
        assert_eq!(buf[(x, 2)].fg, style::LX_PINK);
    }
}

#[test]
fn start_row_buttons_right_align_to_area_edge_and_ellipsize_long_text() {
    let area = Rect::new(0, 0, 36, 4);
    let prompt = prompt_at(
        36,
        4,
        "&&& addTemplate --start {title: very long title text}\n&&& addTemplate --end",
    );
    let hits = template_buttons(&prompt, area);
    assert_eq!(hits.len(), 4);
    assert_eq!(
        hits.last().unwrap().rect.right(),
        area.right(),
        "最后一个按钮右缘贴齐文本区右边界（不再预留右边框列）"
    );
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &prompt, None);
    let text_before_buttons: String = (1..hits[0].rect.x).map(|x| buf[(x, 0)].symbol()).collect();
    assert!(
        text_before_buttons.contains('…'),
        "超宽起始行文本以省略号截断: {text_before_buttons:?}"
    );
}

#[test]
fn second_template_block_has_no_bar_and_no_buttons() {
    let area = Rect::new(0, 0, 50, 8);
    let prompt = prompt_at(
        50,
        8,
        concat!(
            "&&& addTemplate --start\n",
            "&&& addTemplate --end\n",
            "&&& bugTemplate --start\n",
            "body\n",
            "&&& bugTemplate --end",
        ),
    );
    let hits = template_buttons(&prompt, area);
    assert_eq!(hits.len(), 4, "只有第一个 &&& 产生 4 个按钮");
    assert!(hits.iter().all(|hit| hit.line == 0));

    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &prompt, None);
    assert_eq!(buf[(0, 0)].symbol(), text::PROMPT_BLOCK_BAR);
    assert_eq!(buf[(0, 1)].symbol(), text::PROMPT_BLOCK_BAR);
    for y in 2..=4 {
        assert_ne!(
            buf[(0, y)].symbol(),
            text::PROMPT_BLOCK_BAR,
            "第二个 &&& 的第 {y} 行不画色条"
        );
    }
}

#[test]
fn plain_line_does_not_paint_block_bar() {
    let area = Rect::new(0, 0, 30, 4);
    let prompt = prompt_at(30, 4, "plain line\n# heading");
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &prompt, None);
    assert_eq!(buf[(0, 0)].symbol(), "p");
    assert_eq!(buf[(0, 1)].symbol(), "#");
}
