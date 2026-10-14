//! lx 页单元测试：图标数据一致性、布局降级、命中测试、渲染与整页取词。

use super::*;
use crate::app::state::LxEntry;
use ratatui::buffer::Buffer;
use ratatui::style::Color;

/// 像素字标：行数 = 单元格行 × 2、每行宽度一致、只含调色板字符。
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
fn logo_art_is_consistent() {
    assert_icon(ICON_LX);
}

#[test]
fn entries_share_lx_shape_with_tweaked_colors() {
    // 三个入口形状一致（非透明像素位置相同），仅配色不同。
    for entry in LxEntry::ALL {
        let mapped: Vec<String> = ICON_LX
            .iter()
            .map(|row| row.chars().map(|ch| icon_pixel(entry, ch)).collect())
            .collect();
        for (row, mapped_row) in ICON_LX.iter().zip(&mapped) {
            for (original, tinted) in row.chars().zip(mapped_row.chars()) {
                assert_eq!(original == '.', tinted == '.', "形状必须一致（透明位相同）");
            }
        }
    }
    // 配色确实不同：至少一个非透明像素被改写。
    let tinted = |entry| -> String {
        ICON_LX
            .iter()
            .map(|row| {
                row.chars()
                    .map(|ch| icon_pixel(entry, ch))
                    .collect::<String>()
            })
            .collect()
    };
    assert_ne!(tinted(LxEntry::Native), tinted(LxEntry::Cognitive));
    assert_ne!(tinted(LxEntry::Cognitive), tinted(LxEntry::Assistant));
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
fn input_sits_on_bottom_row_with_margin() {
    let area = Rect::new(0, 0, 60, 16);
    let lay = layout(area, &Lx::new());
    let input = lay.input.expect("input row");
    assert_eq!(input.height, INPUT_HEIGHT);
    assert_eq!(input.x, area.x + MARGIN);
    assert_eq!(input.y, area.bottom() - 1);
}

#[test]
fn tiny_page_falls_back_to_wordmark() {
    let area = Rect::new(0, 0, 6, 5);
    let lay = layout(area, &Lx::new());
    assert!(lay.fallback);
    let mut buf = Buffer::empty(area);
    let _ = render(area, &mut buf, &Lx::new(), false, None);
    assert!(buffer_text(&buf).contains("lx"));
}

#[test]
fn entries_are_equal_width_and_horizontal_when_roomy() {
    let area = Rect::new(0, 0, 100, 20);
    let lay = layout(area, &Lx::new());
    assert_eq!(lay.entries.len(), 3);
    let first_y = lay.entries[0].1.y;
    assert!(
        lay.entries.iter().all(|(_, rect)| rect.y == first_y),
        "同一行"
    );
    let widths: Vec<u16> = lay.entries.iter().map(|(_, rect)| rect.width).collect();
    assert!(widths.iter().all(|w| *w == widths[0]), "三按钮等宽");
    assert!(lay.entries[0].1.x < lay.entries[1].1.x, "从左到右");
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
        assert_eq!(entry_hit(area, &lx, rect.x + 1, rect.y), Some(*entry));
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
    assert!(text.contains('[') && text.contains(']'), "极简方括号按钮");
    assert!(!text.contains("type a message"), "框下提示已移除");
    assert!(!text.contains('▛'), "像素方块边框已移除");
    assert!(!text.contains("placeholder"), "白色占位面板已移除");
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
fn cursor_is_relative_to_content_area() {
    let area = Rect::new(5, 3, 100, 20);
    let mut lx = Lx::new();
    lx.insert_char('h');
    lx.insert_char('i');
    let mut buf = Buffer::empty(area);
    let cursor = render(area, &mut buf, &lx, true, None).expect("cursor");
    let text = buffer_text(&buf);
    assert!(text.contains("hi"));
    assert!(!text.contains("Ask anything"), "输入后占位消失");
    // 行相对内容区（不是绝对坐标）：输入在最后一行。
    assert_eq!(cursor.0, area.height - 1);
    // 列相对内容区：左边距 2 + `> ` 2 + `hi` 2 = 6。
    assert_eq!(cursor.1, MARGIN + 2 + 2);
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
    let area = buf.area;
    (area.y..area.bottom())
        .map(|y| {
            (area.x..area.right())
                .map(|x| buf[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}
