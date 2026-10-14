//! lx 页单元测试：图标数据一致性、布局降级、命中测试、渲染、md 面板与整页取词。

use super::*;
use crate::app::state::LxEntry;
use ratatui::buffer::{Buffer, CellDiffOption};
use ratatui::style::Color;

/// 像素字标：行数符合预期、每行宽度一致、只含调色板字符。
fn assert_icon(art: &[&str], width: u16, rows: usize) {
    assert_eq!(art.len(), rows, "像素行数");
    for row in art {
        assert_eq!(row.chars().count(), usize::from(width), "像素行宽度一致");
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
    assert_icon(ICON_LX, LOGO_WIDTH, usize::from(LOGO_ROWS) * 2);
}

#[test]
fn entries_share_lx_shape_with_tweaked_colors() {
    for entry in LxEntry::ALL {
        for row in ICON_LX {
            for original in row.chars() {
                let tinted = icon_pixel(entry, original);
                assert_eq!(original == '.', tinted == '.', "形状必须一致");
            }
        }
    }
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
    assert_ne!(tinted(LxEntry::Native), tinted(LxEntry::Hybrid));
    assert_ne!(tinted(LxEntry::Hybrid), tinted(LxEntry::Automation));
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
fn entry_label_gradients_are_distinct() {
    let endpoints: Vec<(Color, Color)> = LxEntry::ALL
        .iter()
        .map(|entry| style::lx_entry_gradient(*entry))
        .collect();
    assert_ne!(endpoints[0], endpoints[1]);
    assert_ne!(endpoints[1], endpoints[2]);
    assert_ne!(endpoints[0], endpoints[2]);
}

#[test]
fn entry_label_spans_a_gradient() {
    let length = 12;
    let first = style::lx_entry_label_style(LxEntry::Native, 0, length, false);
    let last = style::lx_entry_label_style(LxEntry::Native, length - 1, length, false);
    assert_ne!(first.fg, last.fg, "渐变两端颜色不同");
    let selected = style::lx_entry_label_style(LxEntry::Native, 0, length, true);
    assert!(
        selected
            .add_modifier
            .contains(ratatui::style::Modifier::BOLD)
    );
}

#[test]
fn empty_input_box_is_minimal_height() {
    let area = Rect::new(0, 0, 60, 16);
    let input = layout(area, &Lx::new()).input.expect("input block");
    assert_eq!(input.height, INPUT_MIN_BOX);
    assert_eq!(input.x, area.x + MARGIN);
    assert_eq!(input.y, area.bottom() - INPUT_MIN_BOX);
}

#[test]
fn multiline_input_grows_the_box() {
    let area = Rect::new(0, 0, 60, 16);
    let mut lx = Lx::new();
    lx.insert_char('a');
    lx.newline();
    lx.insert_char('b');
    let input = layout(area, &lx).input.expect("input block");
    assert_eq!(input.height, INPUT_MIN_BOX + 1, "两行内容");
    assert_eq!(input.y, area.bottom() - INPUT_MIN_BOX - 1);
}

#[test]
fn tiny_page_falls_back_to_wordmark() {
    let area = Rect::new(0, 0, 6, 5);
    assert!(layout(area, &Lx::new()).fallback);
    let mut buf = Buffer::empty(area);
    let _ = render(area, &mut buf, &Lx::new(), false, None);
    assert!(buffer_text(&buf).contains("lx"));
}

#[test]
fn entries_are_horizontal_and_centered_when_roomy() {
    let area = Rect::new(0, 0, 100, 20);
    let lay = layout(area, &Lx::new());
    assert!(lay.horizontal);
    assert_eq!(lay.entries.len(), 3);
    let first_y = lay.entries[0].1.y;
    assert!(
        lay.entries.iter().all(|(_, rect)| rect.y == first_y),
        "同一行"
    );
    assert!(lay.entries[0].1.x < lay.entries[1].1.x, "从左到右");
}

#[test]
fn narrow_page_stacks_entries_vertically() {
    let area = Rect::new(0, 0, 34, 30);
    let lay = layout(area, &Lx::new());
    assert!(!lay.horizontal);
    let ys: Vec<u16> = lay.entries.iter().map(|(_, rect)| rect.y).collect();
    assert!(ys[1] > ys[0] && ys[2] > ys[1], "逐行堆叠");
}

#[test]
fn entry_hit_matches_labels_and_ignores_blank() {
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
    assert!(text.contains("Hybrid Agent"), "{text}");
    assert!(text.contains("Automation Agent"), "{text}");
    assert!(text.contains("Ask anything"), "{text}");
    assert!(text.contains("Tip:"), "Tip 前缀可见");
    assert!(text.contains('·'), "标签页分隔符可见");
    assert!(text.contains('─'), "输入框上下分割线可见");
}

#[test]
fn icon_is_horizontally_centered() {
    let area = Rect::new(0, 0, 100, 60);
    let (icon_x, _) = layout(area, &Lx::new()).icon.expect("icon");
    let content_x = area.x + MARGIN;
    let content_w = area.width - MARGIN * 2;
    assert_eq!(icon_x, content_x + (content_w - LOGO_WIDTH) / 2);
}

#[test]
fn logo_sits_two_rows_above_tip() {
    let area = Rect::new(0, 0, 100, 60);
    let lay = layout(area, &Lx::new());
    let (_, icon_y) = lay.icon.expect("icon");
    let tip = lay.description.expect("tip");
    assert_eq!(
        tip.y - (icon_y + LOGO_ROWS),
        LOGO_GAP,
        "logo 底边距 Tip 2 行"
    );
}

#[test]
fn logo_hidden_when_container_is_short_but_tip_and_entries_remain() {
    let tall = Rect::new(0, 0, 100, 60);
    let short = Rect::new(0, 0, 100, 20);
    assert!(layout(tall, &Lx::new()).icon.is_some(), "高屏显示 logo");
    let lay = layout(short, &Lx::new());
    assert!(lay.icon.is_none(), "矮屏隐藏 logo");
    assert!(lay.description.is_some(), "Tip 仍显示");
    assert!(!lay.entries.is_empty(), "按钮仍显示");
}

#[test]
fn input_box_is_capped_at_forty_percent() {
    let area = Rect::new(0, 0, 60, 40);
    let mut lx = Lx::new();
    for i in 0..30 {
        if i > 0 {
            lx.newline();
        }
        lx.insert_char('x');
    }
    let input = layout(area, &lx).input.expect("input");
    assert_eq!(input.height, 40 * INPUT_MAX_PERCENT / 100, "封顶 40%");
}

#[test]
fn tall_input_keeps_entries_above_it() {
    let area = Rect::new(0, 0, 100, 40);
    let mut lx = Lx::new();
    for i in 0..30 {
        if i > 0 {
            lx.newline();
        }
        lx.insert_char('x');
    }
    let lay = layout(area, &lx);
    let input = lay.input.expect("input");
    assert!(!lay.entries.is_empty(), "输入框变高后按钮仍在");
    let last = lay
        .entries
        .iter()
        .map(|(_, rect)| rect.y)
        .max()
        .expect("entries");
    assert!(last < input.y, "按钮在输入框上方");
}

#[test]
fn tip_sits_above_entries_below_icon() {
    let area = Rect::new(0, 0, 100, 60);
    let lay = layout(area, &Lx::new());
    let (_, icon_y) = lay.icon.expect("icon");
    let tip = lay.description.expect("tip");
    let entries_y = lay
        .entries
        .iter()
        .map(|(_, rect)| rect.y)
        .min()
        .expect("entries");
    assert!(icon_y < tip.y, "图标在 Tip 上方");
    assert!(tip.y < entries_y, "Tip 在按钮上方");
}

#[test]
fn selected_entry_switches_text() {
    let area = Rect::new(0, 0, 100, 20);
    let mut buf = Buffer::empty(area);
    let mut lx = Lx::new();
    lx.entry = LxEntry::Hybrid;
    let _ = render(area, &mut buf, &lx, true, None);
    assert!(buffer_text(&buf).contains("mixes native and external agents"));
}

#[test]
fn selected_entry_has_underline_below() {
    let area = Rect::new(0, 0, 100, 60);
    let mut buf = Buffer::empty(area);
    let _ = render(area, &mut buf, &Lx::new(), true, None);
    let lay = layout(area, &Lx::new());
    let find = |entry| {
        lay.entries
            .iter()
            .find(|(e, _)| *e == entry)
            .map(|(_, rect)| *rect)
            .expect("entry")
    };
    let native = find(LxEntry::Native);
    assert_eq!(
        buf[(native.x, native.y + 1)].symbol(),
        "─",
        "选中项下方有下划线"
    );
    let hybrid = find(LxEntry::Hybrid);
    assert_ne!(
        buf[(hybrid.x, hybrid.y + 1)].symbol(),
        "─",
        "未选中项无下划线"
    );
}

#[test]
fn typing_hash_opens_markdown_panel() {
    let area = Rect::new(0, 0, 100, 24);
    let mut lx = Lx::new();
    lx.insert_char('#');
    assert!(lx.panel.is_some(), "行尾 # 触发 md 面板");
    let mut buf = Buffer::empty(area);
    let _ = render(area, &mut buf, &lx, true, None);
    let text = buffer_text(&buf);
    assert!(text.contains("Markdown"), "面板标题: {text}");
    assert!(text.contains("Heading 1"), "候选可见: {text}");
}

#[test]
fn markdown_line_is_styled() {
    let area = Rect::new(0, 0, 60, 12);
    let mut lx = Lx::new();
    for ch in "## Title".chars() {
        lx.insert_char(ch);
    }
    let mut buf = Buffer::empty(area);
    let _ = render(area, &mut buf, &lx, false, None);
    let row = area.bottom() - 2;
    let content_x = area.x + MARGIN + 2;
    // `#` 为标记（muted/dim），`Title` 为标题（黄色）。
    assert!(
        buf[(content_x, row)]
            .modifier
            .contains(ratatui::style::Modifier::DIM)
    );
    assert_eq!(buf[(content_x + 3, row)].fg, Color::Yellow);
}

#[test]
fn cursor_is_relative_to_content_area() {
    let area = Rect::new(5, 3, 100, 20);
    let mut lx = Lx::new();
    lx.insert_char('h');
    lx.insert_char('i');
    let mut buf = Buffer::empty(area);
    let cursor = render(area, &mut buf, &lx, true, None).expect("cursor");
    assert!(buffer_text(&buf).contains("hi"));
    // 行相对内容区：输入行是底部 3 行块的中行。
    assert_eq!(cursor.0, area.height - 2);
    assert_eq!(cursor.1, MARGIN + 2 + 2);
}

#[test]
fn cursor_follows_the_active_line() {
    let area = Rect::new(0, 0, 60, 16);
    let mut lx = Lx::new();
    lx.insert_char('a');
    lx.newline();
    lx.insert_char('b');
    let mut buf = Buffer::empty(area);
    let cursor = render(area, &mut buf, &lx, true, None).expect("cursor");
    // 输入框高 4：内容行在倒数第 2、3 行；光标在第二行。
    assert_eq!(cursor.0, area.height - 2);
    assert_eq!(cursor.1, MARGIN + 2 + 1);
}

#[test]
fn cursor_aligns_after_wide_chars() {
    let area = Rect::new(0, 0, 60, 12);
    let mut lx = Lx::new();
    for ch in "你谁".chars() {
        lx.insert_char(ch);
    }
    let mut buf = Buffer::empty(area);
    let cursor = render(area, &mut buf, &lx, true, None).expect("cursor");
    let row = area.bottom() - 2;
    let content_x = area.x + MARGIN + 2;
    assert_eq!(buf[(content_x, row)].symbol(), "你");
    assert_eq!(
        buf[(content_x + 1, row)].diff_option,
        CellDiffOption::Skip,
        "宽字符后续列标记跳过"
    );
    assert_eq!(buf[(content_x + 2, row)].symbol(), "谁");
    assert_eq!(cursor, (area.height - 2, MARGIN + 2 + 4));
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
fn copy_text_keeps_wide_chars_once() {
    let area = Rect::new(0, 0, 60, 12);
    let mut lx = Lx::new();
    for ch in "你谁".chars() {
        lx.insert_char(ch);
    }
    let row = i32::from(area.height - 2);
    let text = copy_text(area, &lx, ((row, MARGIN), (row, area.width - 1)));
    assert_eq!(text, "> 你谁");
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

#[test]
fn path_paste_panel_renders_above_input() {
    use crate::app::paste::{PasteCandidate, PasteCandidateKind};
    use std::path::PathBuf;

    let area = Rect::new(0, 0, 100, 24);
    let mut lx = Lx::new();
    assert!(lx.paste_open(vec![PasteCandidate::external(
        PathBuf::from("/tmp/dir"),
        PasteCandidateKind::Folder,
    )]));
    let mut buf = Buffer::empty(area);
    let _ = render(area, &mut buf, &lx, true, None);
    let text = buffer_text(&buf);
    assert!(text.contains("Paste"), "参照面板标题: {text}");
    assert!(text.contains("Insert reference"), "引用项: {text}");
    assert!(text.contains("Insert path"), "路径项: {text}");
}

#[test]
fn long_line_wraps_into_visible_rows() {
    let area = Rect::new(0, 0, 40, 20);
    let mut lx = Lx::new();
    let width = area.width - 2 * MARGIN - INPUT_PREFIX - INPUT_SCROLLBAR;
    lx.resize(width);
    lx.insert_str(&"a".repeat(usize::from(width)));
    lx.insert_str("ZZ");
    let mut buf = Buffer::empty(area);
    let _ = render(area, &mut buf, &lx, true, None);
    let text = buffer_text(&buf);
    assert!(text.contains("ZZ"), "折行后的第二视觉行可见: {text}");
}

#[test]
fn wrapped_input_caps_height_and_shows_scrollbar() {
    let area = Rect::new(0, 0, 40, 20);
    let mut lx = Lx::new();
    let width = area.width - 2 * MARGIN - INPUT_PREFIX - INPUT_SCROLLBAR;
    lx.resize(width);
    let cap = usize::from((area.height * INPUT_MAX_PERCENT / 100).max(INPUT_MIN_BOX) - 2).max(1);
    lx.insert_str(&"x".repeat(usize::from(width) * (cap + 3)));
    assert!(lx.visual_rows().len() > cap, "视觉行超过可视上限");
    assert_eq!(layout(area, &lx).input_visible, cap, "输入框高度封顶");
    let mut buf = Buffer::empty(area);
    let _ = render(area, &mut buf, &lx, true, None);
    let text = buffer_text(&buf);
    assert!(
        text.contains('▐') || text.contains('▕'),
        "溢出时滚动条可见: {text}"
    );
}
