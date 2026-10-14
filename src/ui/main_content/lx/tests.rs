//! lx 页单元测试：图标数据一致性、布局降级、命中测试、渲染与整页取词。

use super::*;
use crate::app::state::LxEntry;
use ratatui::buffer::Buffer;
use ratatui::style::Color;

/// 图标像素图：行数 = 单元格行 × 2、每行宽度一致、只含调色板字符。
fn assert_icon(art: &[&str]) {
    assert_eq!(art.len(), usize::from(LOGO_ROWS) * 2, "行数 = 单元格行 × 2");
    for row in art {
        assert_eq!(
            row.chars().count(),
            usize::from(LOGO_WIDTH),
            "像素行宽度一致"
        );
        for ch in row.chars() {
            assert!(
                matches!(ch, '.' | 'k' | 'p' | 'n' | 'b' | 'c' | 'w'),
                "未知像素字符 {ch:?}"
            );
        }
    }
}

#[test]
fn icons_are_consistent() {
    assert_icon(ICON_NATIVE);
    assert_icon(ICON_COGNITIVE);
    assert_icon(ICON_ASSISTANT);
}

#[test]
fn new_icons_are_distinct_and_show_lx() {
    assert_ne!(ICON_NATIVE, ICON_COGNITIVE, "Cognitive 区别于 Native");
    assert_ne!(ICON_COGNITIVE, ICON_ASSISTANT, "两个新图标互不相同");
    for art in [ICON_COGNITIVE, ICON_ASSISTANT] {
        assert!(art.iter().any(|row| row.contains('w')), "新图标含 lx 字标");
    }
}

#[test]
fn logo_palette_maps_logo_colors() {
    assert_eq!(style::logo_pixel('k'), Some(Color::Indexed(17)));
    assert_eq!(style::logo_pixel('p'), Some(Color::Indexed(218)));
    assert_eq!(style::logo_pixel('w'), Some(Color::Indexed(231)));
    assert_eq!(style::logo_pixel('.'), None);
    assert_eq!(style::logo_pixel('x'), None);
}

#[test]
fn input_box_sits_at_bottom_with_hint() {
    let area = Rect::new(0, 0, 60, 16);
    let lay = layout(area, &Lx::new());
    let input = lay.input.expect("input box");
    assert_eq!(input.height, INPUT_HEIGHT);
    assert_eq!(input.x, area.x + MARGIN);
    assert_eq!(input.bottom(), area.bottom() - 1, "底部留一行提示");
    assert_eq!(lay.hint.expect("hint").y, area.bottom() - 1);
}

#[test]
fn tiny_page_falls_back_to_wordmark() {
    let area = Rect::new(0, 0, 8, 2);
    let lay = layout(area, &Lx::new());
    assert!(lay.fallback);
    let mut buf = Buffer::empty(area);
    let _ = render(area, &mut buf, &Lx::new(), false, None);
    assert!(buffer_text(&buf).contains("lx"));
}

#[test]
fn wide_page_places_three_entries_horizontally() {
    let area = Rect::new(0, 0, 100, 20);
    let lay = layout(area, &Lx::new());
    assert_eq!(lay.entries.len(), 3);
    let first_y = lay.entries[0].1.y;
    assert!(
        lay.entries.iter().all(|(_, rect)| rect.y == first_y),
        "同一行"
    );
    assert!(lay.entries[0].1.x < lay.entries[1].1.x, "从左到右");
    assert!(lay.icon.is_some(), "图标可见");
    assert!(lay.description.is_some(), "描述可见");
}

#[test]
fn narrow_page_stacks_entries_vertically() {
    let area = Rect::new(0, 0, 34, 30);
    let lay = layout(area, &Lx::new());
    assert_eq!(lay.entries.len(), 3);
    let ys: Vec<u16> = lay.entries.iter().map(|(_, rect)| rect.y).collect();
    assert!(ys[1] > ys[0] && ys[2] > ys[1], "逐行堆叠");
}

#[test]
fn entry_hit_matches_buttons_and_ignores_blank() {
    let area = Rect::new(0, 0, 100, 20);
    let lx = Lx::new();
    let lay = layout(area, &lx);
    for (entry, rect) in &lay.entries {
        assert_eq!(entry_hit(area, &lx, rect.x + 1, rect.y + 1), Some(*entry));
    }
    assert_eq!(entry_hit(area, &lx, area.x, area.y), None);
}

#[test]
fn wide_page_renders_brand_entries_and_input() {
    let area = Rect::new(0, 0, 100, 20);
    let mut buf = Buffer::empty(area);
    let _ = render(area, &mut buf, &Lx::new(), true, None);
    let text = buffer_text(&buf);
    assert!(text.contains("Native Agent"), "{text}");
    assert!(text.contains("Cognitive Orchestrator"), "{text}");
    assert!(text.contains("Assistant Orchestrator"), "{text}");
    assert!(text.contains("Ask anything"), "{text}");
    assert!(text.contains("type a message"), "{text}");
    assert!(text.contains('❯'), "默认选中标记可见");
    assert!(text.contains('▛'), "像素边框可见");
    assert!(!text.contains("placeholder"), "白色占位面板已移除");
    assert!(!text.contains("click [>_]"), "切换提示已移除");
}

#[test]
fn selected_entry_switches_icon_and_text() {
    let area = Rect::new(0, 0, 100, 20);
    let mut buf = Buffer::empty(area);
    let mut lx = Lx::new();
    lx.entry = LxEntry::Cognitive;
    let _ = render(area, &mut buf, &lx, true, None);
    let text = buffer_text(&buf);
    assert!(
        text.contains("orchestrates all your coding agents"),
        "{text}"
    );
    assert!(text.contains("examples: opencode"), "{text}");
}

#[test]
fn typing_updates_input_and_cursor() {
    let area = Rect::new(0, 0, 100, 20);
    let mut lx = Lx::new();
    lx.insert_char('h');
    lx.insert_char('i');
    let mut buf = Buffer::empty(area);
    let cursor = render(area, &mut buf, &lx, true, None).expect("cursor");
    let text = buffer_text(&buf);
    assert!(text.contains("hi"));
    assert!(!text.contains("Ask anything"), "输入后占位消失");
    let input = layout(area, &lx).input.expect("input box");
    assert_eq!(cursor.1, input.x + 4 + 2, "光标在 `hi` 之后");
    assert_eq!(cursor.0, input.y + 1);
}

#[test]
fn copy_text_extracts_selected_rows() {
    let area = Rect::new(0, 0, 100, 20);
    let lx = Lx::new();
    let text = copy_text(area, &lx, ((0, 0), (19, 99)));
    assert!(text.contains("Native Agent"), "{text}");
    assert!(text.contains("Ask anything"), "{text}");
}

#[test]
fn copy_text_blank_selection_is_empty() {
    let area = Rect::new(0, 0, 100, 20);
    assert_eq!(copy_text(area, &Lx::new(), ((0, 0), (0, 0))), "");
}

/// 把缓冲区按行拼成文本，便于断言内容。
fn buffer_text(buf: &Buffer) -> String {
    (0..buf.area.height)
        .map(|y| {
            (0..buf.area.width)
                .map(|x| buf[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}
