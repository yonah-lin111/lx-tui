//! 信任页渲染单元测试：左上对齐、分隔线、logo、选项高亮与命中、窄屏降级。

use std::path::PathBuf;

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::style::{Color, Modifier};

use super::*;

const PATH: &str = "/Users/example/workspace";

fn draw(width: u16, height: u16, selected: usize) -> (Buffer, TrustLayout) {
    let screen = Rect::new(0, 0, width, height);
    let layout = layout(screen, &PathBuf::from(PATH));
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("test backend");
    terminal
        .draw(|frame| render(frame, &layout, selected))
        .expect("draw succeeds");
    (terminal.backend().buffer().clone(), layout)
}

fn row_text(buf: &Buffer, rect: Rect) -> String {
    (rect.x..rect.right())
        .map(|x| buf[(x, rect.y)].symbol().to_string())
        .collect()
}

fn content_text(buf: &Buffer, layout: &TrustLayout, width: u16) -> String {
    let mut text = String::new();
    for y in layout.content_y..layout.content_y + layout.lines.len() as u16 {
        if y >= buf.area.height {
            break;
        }
        text.push_str(&row_text(buf, Rect::new(0, y, width, 1)));
        text.push(' ');
    }
    text
}

#[test]
fn renders_top_left_aligned_page_with_rule_logo_and_options() {
    let (buf, layout) = draw(100, 30, 0);

    // 全宽分隔线在最上方。
    assert_eq!(layout.rule.y, 0, "分隔线在 logo 与内容上方");
    assert_eq!(buf[(0, layout.rule.y)].symbol(), text::DIVIDER_MID);
    assert_eq!(buf[(99, layout.rule.y)].symbol(), text::DIVIDER_MID);

    // logo 位于分隔线下方、左对齐（与内容同缩进）。
    let logo = layout.logo.expect("logo fits on large screen");
    assert_eq!(logo.x, PADDING_X);
    assert_eq!(logo.y, 2, "分隔线与空行之后");
    let logo_row = row_text(&buf, Rect::new(logo.x, logo.y, logo.width, 1));
    assert!(
        logo_row.chars().any(|ch| matches!(ch, '▀' | '▄' | '█')),
        "logo 像素已绘制: {logo_row}"
    );

    // 内容左上对齐：标题从第 1 列开始（前导 1 列缩进）。
    assert_eq!(buf[(0, layout.content_y)].symbol(), " ");
    let heading = row_text(&buf, Rect::new(1, layout.content_y, 98, 1));
    assert!(
        heading.starts_with(text::TRUST_CONSOLE_HEADING),
        "{heading}"
    );

    // 路径整行 accent 高亮；安全提示前缀加粗。
    let mut path_highlighted = false;
    let mut safety_seen = false;
    for y in layout.content_y..layout.content_y + layout.lines.len() as u16 {
        let row = row_text(&buf, Rect::new(0, y, 100, 1));
        if let Some(position) = row.find(PATH) {
            assert_eq!(
                buf[(position as u16, y)].fg,
                Color::Cyan,
                "路径强调色: {row}"
            );
            path_highlighted = true;
        }
        if let Some(position) = row.find(SAFETY_PREFIX) {
            assert!(
                buf[(position as u16, y)].modifier.contains(Modifier::BOLD),
                "安全提示前缀加粗: {row}"
            );
            safety_seen = true;
        }
    }
    assert!(path_highlighted, "路径行存在");
    assert!(safety_seen, "安全提示行存在");

    assert!(row_text(&buf, layout.options[0]).contains(text::TRUST_OPTION_NO));
    assert!(row_text(&buf, layout.options[1]).contains(text::TRUST_OPTION_YES));
}

#[test]
fn default_selection_is_safe_no_with_full_width_highlight() {
    // 默认选中 `No, exit`（安全默认，对齐 Claude Code），高亮铺满整行。
    let (buf, layout) = draw(100, 30, 0);
    let no = layout.options[0];
    assert_eq!(no.x, 0);
    assert_eq!(no.width, 100);
    assert_eq!(buf[(no.x, no.y)].symbol(), text::TRUST_SELECTION_MARKER);
    assert_eq!(buf[(no.x, no.y)].bg, Color::Indexed(240));
    assert_eq!(buf[(99, no.y)].bg, Color::Indexed(240), "高亮铺满整行");
    assert_eq!(
        buf[(layout.options[1].x, layout.options[1].y)].symbol(),
        " "
    );

    let (buf, layout) = draw(100, 30, 1);
    assert_eq!(
        buf[(layout.options[1].x, layout.options[1].y)].bg,
        Color::Indexed(240),
        "切换选中 Yes"
    );
    assert_eq!(
        buf[(layout.options[1].x, layout.options[1].y)].symbol(),
        text::TRUST_SELECTION_MARKER
    );
}

#[test]
fn option_at_hits_option_rows_only() {
    let (_, layout) = draw(100, 30, 0);
    let no = layout.options[0];
    let yes = layout.options[1];
    assert_eq!(option_at(&layout, no.x + 1, no.y), Some(0));
    assert_eq!(option_at(&layout, yes.x + 1, yes.y), Some(1));
    assert_eq!(option_at(&layout, 0, layout.rule.y), None);
    assert_eq!(option_at(&layout, no.x, yes.y + 1), None);
}

#[test]
fn narrow_screen_degrades_and_stays_inside_screen() {
    let (buf, layout) = draw(40, 10, 0);
    assert!(layout.logo.is_none(), "屏幕高度不足时省略 logo");
    assert!(layout.rule.right() <= 40);
    assert!(layout.options[1].bottom() <= 10, "选项在屏幕内可点击");
    let text_joined = content_text(&buf, &layout, 40);
    // 极小高度下优先保留标题、路径与选项。
    assert!(
        text_joined.contains(text::TRUST_CONSOLE_HEADING),
        "{text_joined}"
    );
    assert!(text_joined.contains(PATH), "{text_joined}");
    assert!(text_joined.contains(text::TRUST_OPTION_NO), "{text_joined}");
    assert!(
        text_joined.contains(text::TRUST_OPTION_YES),
        "{text_joined}"
    );
}

#[test]
fn mid_size_screen_keeps_safety_and_hint() {
    let (buf, layout) = draw(60, 30, 0);
    assert!(layout.options[1].bottom() <= 30);
    let text_joined = content_text(&buf, &layout, 60);
    assert!(text_joined.contains(SAFETY_PREFIX), "{text_joined}");
    assert!(
        text_joined.contains("lx-tui will add this folder as a workspace"),
        "{text_joined}"
    );
    assert!(
        text_joined.contains("Prompt management is included"),
        "{text_joined}"
    );
}
