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
    let lx = Lx::new();
    let lay = layout(area, &lx);
    let input = lay.input.expect("input block");
    assert_eq!(input.height, INPUT_MIN_BOX);
    assert_eq!(input.x, area.x + MARGIN);
    // 输入框下沿贴窗格底部（状态绘制在底边框上，不再单独占行）。
    assert_eq!(input.bottom(), area.bottom());
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
    assert_eq!(input.bottom(), area.bottom());
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
    // 输入框高度上限=内容区高度取 40%。
    let expected = (area.height * INPUT_MAX_PERCENT / 100).max(INPUT_MIN_BOX);
    assert_eq!(input.height, expected, "封顶 40%");
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
    // 行相对内容区：输入框下沿贴底，内容行即倒数第 2 行（底边框为最后一行）。
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
    // 行相对内容区：输入框高 4，末行内容在倒数第 2 行（最后一行是底边框）。
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
fn input_geom_hit_and_scrollbar() {
    let area = Rect::new(0, 0, 40, 20);
    let mut lx = Lx::new();
    let width = area.width - 2 * MARGIN - INPUT_PREFIX - INPUT_SCROLLBAR;
    lx.resize(width, 6);
    lx.insert_str(&"x".repeat(usize::from(width) * 7));
    let geom = input_geom(area, &lx).expect("input geom");
    let (x, y) = geom.text_origin();
    assert!(geom.contains(x, y), "文本区左上角在内");
    assert!(!geom.contains(x.saturating_sub(1), y), "前缀列不在文本区");
    assert!(!geom.contains(x, y.saturating_sub(1)), "上分割线不在文本区");
    assert_eq!(geom.cell(x + 2, y).1, 2, "列映射");
    assert!(input_scrollbar(area, &lx).is_some(), "溢出时有滚动条几何");
}
#[test]
fn long_line_wraps_into_visible_rows() {
    let area = Rect::new(0, 0, 40, 20);
    let mut lx = Lx::new();
    let width = area.width - 2 * MARGIN - INPUT_PREFIX - INPUT_SCROLLBAR;
    lx.resize(width, 100);
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
    lx.resize(width, 100);
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

// ===== `@` 提及面板几何 =====

use crate::app::markdown::MentionEntry;
use std::path::Path;

/// 构造带候选缓存的 lx：根已同步、`@` 已键入、扫描结果已写入。
fn mention_lx(entries: Vec<MentionEntry>) -> Lx {
    let mut lx = Lx::new();
    lx.set_mention_root(Some(std::path::PathBuf::from("/work/demo")));
    lx.insert_char('@');
    let (generation, _) = lx.take_mention_scan_request().expect("scan request");
    lx.apply_mention_entries(generation, Path::new("/work/demo"), entries);
    lx
}

#[test]
fn mention_panel_is_same_width_as_input_and_adjacent() {
    let area = Rect::new(0, 0, 80, 24);
    let entries: Vec<MentionEntry> = (0..8)
        .map(|index| MentionEntry {
            path: format!("file{index:02}.rs"),
            is_directory: false,
        })
        .collect();
    let lx = mention_lx(entries);
    let input = layout(area, &lx).input.expect("input");
    let panel = mention::layout(area, &lx).expect("mention panel");
    assert_eq!(panel.rect.x, input.x, "面板左边缘与输入框一致");
    assert_eq!(panel.rect.width, input.width, "面板与输入框等宽");
    assert_eq!(
        panel.rect.bottom(),
        input.y,
        "面板底边紧贴输入框上沿（不覆盖输入框）"
    );
}

#[test]
fn mention_panel_widens_beyond_narrow_content_to_input_width() {
    // 单个短候选：内容远窄于输入框，仍需强制等宽。
    let area = Rect::new(0, 0, 80, 24);
    let lx = mention_lx(vec![MentionEntry {
        path: "a".into(),
        is_directory: false,
    }]);
    let input = layout(area, &lx).input.expect("input");
    let panel = mention::layout(area, &lx).expect("mention panel");
    assert_eq!(panel.rect.width, input.width);
}

#[test]
fn mention_panel_absent_without_trigger() {
    let area = Rect::new(0, 0, 80, 24);
    let lx = Lx::new();
    assert!(mention::layout(area, &lx).is_none());
}

#[test]
fn mention_layout_hidden_when_not_enough_room_above_input() {
    // 输入框贴近内容区顶部：上方放不下边框 + 一行，不绘制（不移动输入框）。
    let area = Rect::new(0, 0, 40, 5);
    let mut lx = mention_lx(vec![MentionEntry {
        path: "file.rs".into(),
        is_directory: false,
    }]);
    // 输入框内容框至少 3 行，此时上方仅 2 行：放不下面板（需 3 行）。
    let input = layout(area, &lx).input;
    if let Some(input) = input {
        assert!(mention::layout(area, &lx).is_none(), "空间不足不绘制面板");
        assert_eq!(layout(area, &lx).input, Some(input), "输入框位置不变");
    }
    // 强制触发刷新，避免 lint 未用。
    lx.refresh_panel();
}

#[test]
fn mention_item_hit_matches_visible_candidate() {
    let area = Rect::new(0, 0, 80, 24);
    let entries = vec![
        MentionEntry {
            path: "alpha.rs".into(),
            is_directory: false,
        },
        MentionEntry {
            path: "beta.rs".into(),
            is_directory: false,
        },
    ];
    let lx = mention_lx(entries);
    let panel = mention::layout(area, &lx).expect("panel");
    // 首个可见条目行在内容区首行。
    let row = panel.content.y;
    assert_eq!(
        mention::item_at(area, &lx, panel.content.x, row),
        Some(0),
        "命中首个候选"
    );
    // 面板外的坐标不命中。
    assert_eq!(mention::item_at(area, &lx, 0, 0), None);
}

#[test]
fn mention_panel_renders_title_and_directory_rows() {
    let area = Rect::new(0, 0, 80, 24);
    let entries = vec![
        MentionEntry {
            path: "src".into(),
            is_directory: true,
        },
        MentionEntry {
            path: "src/main.rs".into(),
            is_directory: false,
        },
    ];
    let lx = mention_lx(entries);
    let mut buf = Buffer::empty(area);
    let _ = render(area, &mut buf, &lx, true, None);
    let text = buffer_text(&buf);
    assert!(text.contains("@Command"), "面板标题可见: {text}");
    assert!(text.contains("src/"), "目录以 / 结尾展示: {text}");
    assert!(text.contains("main.rs"), "文件候选可见: {text}");
}

#[test]
fn mention_status_row_renders_when_scanning_and_empty() {
    // 扫描中：根已同步、@ 已输入但缓存未到。
    let area = Rect::new(0, 0, 80, 24);
    let mut lx = Lx::new();
    lx.set_mention_root(Some(std::path::PathBuf::from("/work/demo")));
    lx.insert_char('@');
    assert!(mention::layout(area, &lx).is_some(), "扫描中也显示状态面板");
    let mut buf = Buffer::empty(area);
    let _ = render(area, &mut buf, &lx, true, None);
    let text = buffer_text(&buf);
    assert!(text.contains("scanning"), "扫描中状态行: {text}");

    // 空结果：状态行而非隐藏。
    let empty = mention_lx(Vec::new());
    let mut buf = Buffer::empty(area);
    let _ = render(area, &mut buf, &empty, true, None);
    let text = buffer_text(&buf);
    assert!(text.contains("no matches"), "空结果状态行: {text}");
}

#[test]
fn mention_panel_viewport_limits_visible_rows() {
    // 大量候选但内容区高度有限：面板高度受限、只显示部分候选。
    let area = Rect::new(0, 0, 60, 12);
    let entries: Vec<MentionEntry> = (0..60)
        .map(|index| MentionEntry {
            path: format!("file{index:02}.rs"),
            is_directory: false,
        })
        .collect();
    let lx = mention_lx(entries);
    let panel = mention::layout(area, &lx).expect("panel");
    let above = layout(area, &lx).input.expect("input").y;
    assert!(panel.rect.height <= above, "面板不超过输入框上方空间");
    assert!(panel.visible < 60, "可见条目受限: {}", panel.visible);
}

#[test]
fn mention_panel_renders_cjk_candidate_without_overflow() {
    let area = Rect::new(0, 0, 40, 20);
    let lx = mention_lx(vec![MentionEntry {
        path: "文档/说明.md".into(),
        is_directory: false,
    }]);
    let mut buf = Buffer::empty(area);
    let _ = render(area, &mut buf, &lx, true, None);
    let panel = mention::layout(area, &lx).expect("panel");
    // 面板右边界不超过输入框右边界。
    let input = layout(area, &lx).input.expect("input");
    assert_eq!(panel.rect.right(), input.right());
}

#[test]
fn mention_panel_height_capped_at_max_popup_rows() {
    // 内容区很高：候选很多，面板内容行仍夹到 MAX_POPUP_ROWS（8）+ 边框 2 行。
    let area = Rect::new(0, 0, 60, 60);
    let entries: Vec<MentionEntry> = (0..100)
        .map(|index| MentionEntry {
            path: format!("file{index:03}.rs"),
            is_directory: false,
        })
        .collect();
    let lx = mention_lx(entries);
    let panel = mention::layout(area, &lx).expect("panel");
    assert_eq!(
        panel.rect.height,
        mention::MAX_POPUP_ROWS + 2,
        "高度对齐 codex 的 MAX_POPUP_ROWS + 边框"
    );
    assert!(panel.scrollbar.is_some(), "超出上限时出现滚动条");
}

#[test]
fn mention_panel_height_follows_content_when_few_rows() {
    // 候选少：面板按内容收缩，不撑到上限。
    let area = Rect::new(0, 0, 60, 60);
    let entries = vec![MentionEntry {
        path: "only.rs".into(),
        is_directory: false,
    }];
    let lx = mention_lx(entries);
    let panel = mention::layout(area, &lx).expect("panel");
    assert!(
        panel.rect.height < mention::MAX_POPUP_ROWS + 2,
        "少量候选按内容高度收缩: {}",
        panel.rect.height
    );
}

#[test]
fn mention_panel_height_limited_by_space_above_input() {
    // 输入框贴顶：可用行数小于上限时，面板高度受可用空间约束。
    let area = Rect::new(0, 0, 60, 8);
    let entries: Vec<MentionEntry> = (0..100)
        .map(|index| MentionEntry {
            path: format!("file{index:03}.rs"),
            is_directory: false,
        })
        .collect();
    let lx = mention_lx(entries);
    let input = layout(area, &lx).input.expect("input");
    let panel = mention::layout(area, &lx).expect("panel");
    assert!(panel.rect.height <= input.y, "面板不超过输入框上方空间");
    assert_eq!(panel.rect.bottom(), input.y, "面板紧贴输入框上沿");
}

// ===== `/` 命令面板（与 `@` 面板同款几何/交互） =====

use crate::app::state::lx::slash::{ModelPicker, PanelWindow};
use crate::app::state::{ModelCandidate, ModelPickerStage, SlashCommand, SlashPanel};

/// 构造带 `/` 命令候选面板的 lx 状态。
fn slash_commands_lx(active: usize) -> Lx {
    let mut lx = Lx::new();
    lx.slash = Some(SlashPanel::Commands {
        items: SlashCommand::ALL.to_vec(),
        window: PanelWindow::new(active),
    });
    lx
}

/// 构造带 `/model` 第一级面板的 lx 状态（两个候选，展示名与 id 不同）。
fn slash_model_lx() -> Lx {
    let mut lx = Lx::new();
    lx.slash = Some(SlashPanel::Model(Box::new(ModelPicker {
        stage: ModelPickerStage::Models,
        candidates: vec![
            ModelCandidate {
                selection: crate::config::ai::ModelSelection {
                    provider: "provider-alq374".into(),
                    model: "cbcn/deepseek-v4.1-flash".into(),
                    variant: None,
                },
                provider_name: "9router-lx-t".into(),
                model_name: "deepseek-v4.1-flash".into(),
            },
            ModelCandidate {
                selection: crate::config::ai::ModelSelection {
                    provider: "provider-alq374".into(),
                    model: "gpt-5.6-luna".into(),
                    variant: None,
                },
                provider_name: "9router-lx-t".into(),
                model_name: "gpt-5.6-luna".into(),
            },
        ],
        window: PanelWindow::new(0),
    })));
    lx
}

/// 构造带 `/resume` 空历史面板的 lx 状态。
fn slash_resume_empty_lx() -> Lx {
    let mut lx = Lx::new();
    lx.slash = Some(SlashPanel::History(Box::default()));
    lx
}

#[test]
fn slash_panel_is_same_width_as_input_and_adjacent() {
    let area = Rect::new(0, 0, 80, 24);
    let lx = slash_commands_lx(0);
    let input = layout(area, &lx).input.expect("input");
    let panel = slash_layout(area, &lx).expect("slash panel");
    assert_eq!(panel.rect.x, input.x, "面板左边缘与输入框一致（充满）");
    assert_eq!(panel.rect.width, input.width, "面板与输入框等宽");
    assert_eq!(
        panel.rect.bottom(),
        input.y,
        "面板底边紧贴输入框上沿（不覆盖输入框）"
    );
}

#[test]
fn slash_panel_width_stays_full_for_narrow_content() {
    // 命令文本很短，面板仍须撑满输入框宽度。
    let area = Rect::new(0, 0, 100, 30);
    let lx = slash_commands_lx(0);
    let input = layout(area, &lx).input.expect("input");
    let panel = slash_layout(area, &lx).expect("slash panel");
    assert_eq!(panel.rect.width, input.width);
    assert!(panel.rect.width > 40, "远宽于内容自然宽度");
}

#[test]
fn slash_panel_absent_without_panel() {
    let area = Rect::new(0, 0, 80, 24);
    assert!(slash_layout(area, &Lx::new()).is_none());
}

#[test]
fn slash_model_and_resume_show_empty_placeholder_row() {
    let area = Rect::new(0, 0, 80, 24);
    let resume = slash_resume_empty_lx();
    let layout = slash_layout(area, &resume).expect("empty panel still has a row");
    assert!(layout.visible >= 1, "空态占位行可见");
    // `/model` 无候选同样显示占位（而非整块消失）。
    let mut lx = Lx::new();
    lx.slash = Some(SlashPanel::Model(Box::new(ModelPicker {
        stage: ModelPickerStage::Models,
        candidates: Vec::new(),
        window: PanelWindow::new(0),
    })));
    assert!(slash_layout(area, &lx).is_some());
}

#[test]
fn slash_item_hit_matches_visible_row() {
    let area = Rect::new(0, 0, 80, 24);
    let lx = slash_commands_lx(0);
    let panel = slash_layout(area, &lx).expect("panel");
    // 首个可见条目行 → 索引等于窗口起点。
    assert_eq!(
        slash_item_at(area, &lx, panel.content.x, panel.content.y),
        Some(panel.start)
    );
    // 面板外不命中。
    assert_eq!(slash_item_at(area, &lx, 0, 0), None);
}

#[test]
fn slash_empty_placeholder_is_not_clickable() {
    let area = Rect::new(0, 0, 80, 24);
    let lx = slash_resume_empty_lx();
    let panel = slash_layout(area, &lx).expect("panel");
    assert_eq!(
        slash_item_at(area, &lx, panel.content.x, panel.content.y),
        None,
        "空态占位不可选"
    );
}

#[test]
fn slash_model_panel_is_full_width_and_lists_models() {
    // `/model` 第一级：候选为模型名，面板仍充满输入框宽度。
    let area = Rect::new(0, 0, 90, 24);
    let lx = slash_model_lx();
    let input = layout(area, &lx).input.expect("input");
    let panel = slash_layout(area, &lx).expect("model panel");
    assert_eq!(panel.rect.width, input.width);
    assert_eq!(panel.rect.bottom(), input.y);
    assert_eq!(panel.visible, 2, "两个模型候选可见");
}

#[test]
fn slash_panel_renders_ascii_only() {
    // 面板（含标题/预览/来源标签/底边提示）不得出现双宽字符，否则中文会撑破布局。
    let area = Rect::new(0, 0, 80, 24);
    let lx = slash_commands_lx(0);
    let mut buf = Buffer::empty(area);
    let (built, _cursor) = build(area, &lx, true);
    blit_grid(area, &mut buf, &built);
    for y in 0..area.height {
        for x in 0..area.width {
            let Some(cell) = buf.cell((x, y)) else {
                continue;
            };
            for ch in cell.symbol().chars() {
                let width = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
                assert!(width <= 1, "面板出现双宽字符 {ch:?} at ({x},{y})");
            }
        }
    }
    // 文案常量不得含双宽字符（含右对齐来源标签与底边提示）；命令描述须为纯 ASCII。
    for text in [
        crate::ui::text::LX_SLASH_PANEL_TITLE,
        crate::ui::text::LX_SLASH_PANEL_FOOTER,
        crate::ui::text::LX_MODEL_PANEL_TITLE,
        crate::ui::text::LX_HISTORY_PANEL_TITLE,
        crate::ui::text::LX_CHAT_NO_MODEL,
        crate::ui::text::LX_HISTORY_EMPTY,
    ] {
        for ch in text.chars() {
            let width = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
            assert!(width <= 1, "面板文案含双宽字符 {ch:?} in {text:?}");
        }
    }
    for command in SlashCommand::ALL {
        assert!(command.text().is_ascii(), "命令文本 ASCII");
        assert!(command.description().is_ascii(), "命令描述不得含中文");
        assert!(command.tag().is_ascii(), "来源标签 ASCII");
    }
}

#[test]
fn initial_page_shows_status_on_pane_bottom_border() {
    // 模拟窗格：内容区 `inner` + 底边框行（`inner.bottom()`，位于 inner 之外）。
    let rect = Rect::new(0, 0, 80, 24);
    let inner = crate::layout::pane_inner_rect(rect);
    let mut lx = Lx::new();
    lx.chat.status = crate::app::state::lx::chat::ChatStatus {
        model_name: Some("deepseek-v4.1-flash".into()),
        provider_name: Some("9router-lx-t".into()),
        context_limit: Some(1_000_000),
        input_tokens: Some(720_000),
        context_percent: Some(72),
        streaming: false,
        omitted_turns: None,
    };
    let lay = layout(inner, &lx);
    let input = lay.input.expect("input");
    // 输入框保留自己的上下分割线，下沿仍贴内容区底部（状态改画在窗格底边框上）。
    assert_eq!(input.bottom(), inner.bottom());
    // 底边框行预铺 `─`（保留两角）后渲染状态，检查覆盖效果。
    let mut buf = Buffer::empty(rect);
    let row = inner.bottom();
    for x in rect.x..rect.right() {
        if let Some(cell) = buf.cell_mut((x, row)) {
            cell.set_char('─');
        }
    }
    render(inner, &mut buf, &lx, false, None);
    let border: String = (rect.x..rect.right())
        .map(|x| buf.cell((x, row)).map(|c| c.symbol()).unwrap_or(" "))
        .collect();
    assert!(
        border.contains("deepseek-v4.1-flash") && border.contains("9router-lx-t"),
        "底边框显示模型 · 供应商：{border}"
    );
    assert!(
        border.contains("context:72%"),
        "底边框显示上下文占用 `context:72%`：{border}"
    );
}

#[test]
fn chat_history_scrollbar_geometry_appears_when_overflowing() {
    use crate::app::state::{ChatMessageView, ChatPartView, LxChat};
    let area = Rect::new(0, 0, 40, 6);
    let mut lx = Lx::new();
    let mut chat = LxChat::new();
    for index in 0..12 {
        chat.messages.push(ChatMessageView {
            id: format!("m{index}"),
            role: "user".into(),
            status: "complete".into(),
            turn_index: index,
            parts: vec![ChatPartView {
                kind: crate::chat::events::PartKind::Text,
                content: format!("消息 {index}"),
                provider_metadata_json: None,
                started_at_ms: None,
                ended_at_ms: None,
            }],
            model_label: None,
            error_code: None,
            input_tokens: None,
            output_tokens: None,
            cache_read_tokens: None,
            duration_ms: None,
        });
    }
    lx.chat = chat;
    let bar = history_scrollbar(area, &lx).expect("溢出时有滚动条几何");
    // 滚动条贴历史区（与输入框同宽）右缘一列。
    let input = layout(area, &lx).input.expect("input");
    assert_eq!(bar.track.x, input.right() - 1, "滚动条在输入框右缘内一列");
    assert!(bar.total > bar.visible);
}

#[test]
fn overflowing_history_draws_scrollbar_glyphs() {
    use crate::app::state::{ChatMessageView, ChatPartView, LxChat};
    let area = Rect::new(0, 0, 40, 10);
    let mut lx = Lx::new();
    let mut chat = LxChat::new();
    for index in 0..12 {
        chat.messages.push(ChatMessageView {
            id: format!("m{index}"),
            role: "user".into(),
            status: "complete".into(),
            turn_index: index,
            parts: vec![ChatPartView {
                kind: crate::chat::events::PartKind::Text,
                content: format!("消息 {index}"),
                provider_metadata_json: None,
                started_at_ms: None,
                ended_at_ms: None,
            }],
            model_label: None,
            error_code: None,
            input_tokens: None,
            output_tokens: None,
            cache_read_tokens: None,
            duration_ms: None,
        });
    }
    lx.chat = chat;
    let mut buf = Buffer::empty(area);
    let _ = render(area, &mut buf, &lx, false, None);
    let text = buffer_text(&buf);
    assert!(
        text.contains('▐') || text.contains('▕'),
        "溢出时历史滚动条可见：{text}"
    );
}

#[test]
fn model_panel_shows_provider_and_model_names_not_ids() {
    let area = Rect::new(0, 0, 80, 24);
    let lx = slash_model_lx();
    let mut buf = Buffer::empty(area);
    let _ = render(area, &mut buf, &lx, false, None);
    let text = buffer_text(&buf);
    assert!(text.contains("deepseek-v4.1-flash"), "显示模型名：{text}");
    assert!(text.contains("9router-lx-t"), "显示供应商名：{text}");
    // 不能出现原始 id。
    assert!(
        !text.contains("provider-alq374"),
        "不应显示供应商 id：{text}"
    );
    assert!(!text.contains("cbcn/"), "不应显示模型 id：{text}");
}

// ===== 输入框上方聊天栏（loading 转轮 + 回到底部按钮）=====

use crate::app::state::{ChatMessageView, ChatPartView, LxChat};

/// 构造带一条 assistant 流中消息的 lx 状态。
fn streaming_lx(streaming: bool, follow_bottom: bool) -> Lx {
    let mut lx = Lx::new();
    let mut chat = LxChat::new();
    chat.messages.push(ChatMessageView {
        id: "m0".into(),
        role: "assistant".into(),
        status: if streaming { "streaming" } else { "complete" }.into(),
        turn_index: 0,
        parts: vec![ChatPartView {
            kind: crate::chat::events::PartKind::Text,
            content: "答".into(),
            provider_metadata_json: None,
            started_at_ms: None,
            ended_at_ms: None,
        }],
        model_label: None,
        error_code: None,
        input_tokens: None,
        output_tokens: None,
        cache_read_tokens: None,
        duration_ms: None,
    });
    chat.follow_bottom = follow_bottom;
    chat.status.streaming = streaming;
    chat.loading_frame = 0;
    lx.chat = chat;
    lx
}

#[test]
fn streaming_shows_loading_bar_above_input() {
    let area = Rect::new(0, 0, 60, 16);
    let lx = streaming_lx(true, true);
    let lay = layout(area, &lx);
    let input = lay.input.expect("input");
    let bar = lay.chat_bar.expect("流中显示聊天栏");
    assert_eq!(bar.bottom(), input.y, "聊天栏紧贴输入框上沿");
    assert_eq!(bar.height, 1);
    let mut buf = Buffer::empty(area);
    let _ = render(area, &mut buf, &lx, false, None);
    let text = buffer_text(&buf);
    assert!(
        text.contains(text::LX_LOADING_LABEL),
        "显示 loading 文案：{text}"
    );
    assert!(
        text::LX_LOADING_FRAMES
            .iter()
            .any(|frame| text.contains(*frame)),
        "显示转轮帧：{text}"
    );
}

#[test]
fn idle_following_hides_chat_bar() {
    let area = Rect::new(0, 0, 60, 16);
    let lx = streaming_lx(false, true);
    assert!(
        layout(area, &lx).chat_bar.is_none(),
        "空闲且跟随底部：无聊天栏"
    );
}

#[test]
fn scrolled_up_shows_jump_bottom_button() {
    let area = Rect::new(0, 0, 60, 16);
    let lx = streaming_lx(false, false);
    let rect = chat_bar::jump_bottom_rect(area, &lx).expect("上滚时显示回到底部控件");
    let bar = layout(area, &lx).chat_bar.expect("chat bar");
    assert_eq!(rect.right(), bar.right() - 1, "控件贴聊天栏右端（留 1 列）");
    let mut buf = Buffer::empty(area);
    let _ = render(area, &mut buf, &lx, false, None);
    let text = buffer_text(&buf);
    assert!(text.contains(text::LX_JUMP_BOTTOM), "显示 ↓ 箭头：{text}");
    assert!(
        text.contains(text::LX_JUMP_BOTTOM_HINT),
        "箭头右侧显示说明：{text}"
    );
}

#[test]
fn jump_bottom_hit_matches_button_rect() {
    use crate::app::state::AppState;
    let area = Rect::new(0, 0, 60, 16);
    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    if let Some(target) = state.pane_mut_anywhere(pane) {
        target.lx = streaming_lx(false, false);
    }
    // 直接构造窗格矩形（内缩 1 边框），命中按钮内部一格。
    let rects = [(pane, Rect::new(0, 0, 62, 18))];
    let inner = crate::layout::pane_inner_rect(rects[0].1);
    let button =
        chat_bar::jump_bottom_rect(inner, &state.pane_anywhere(pane).unwrap().lx).expect("button");
    assert_eq!(
        chat_bar::jump_bottom_hit(&rects, &state, button.x, button.y),
        Some(pane),
        "命中回到底部按钮"
    );
    assert_eq!(
        chat_bar::jump_bottom_hit(&rects, &state, 0, 0),
        None,
        "未命中"
    );
    let _ = area;
}

#[test]
fn esc_armed_replaces_loading_label_with_stop_hint() {
    let area = Rect::new(0, 0, 60, 16);
    let mut lx = streaming_lx(true, true);
    lx.chat.stop_armed_at = Some(std::time::Instant::now());
    let mut buf = Buffer::empty(area);
    let _ = render(area, &mut buf, &lx, false, None);
    let text = buffer_text(&buf);
    assert!(text.contains("Press esc again to stop"), "待命提示：{text}");
    assert!(
        !text.contains(text::LX_LOADING_LABEL),
        "提示替换 Working：{text}"
    );
    // 未待命（超时/未按 Esc）时仍是普通 loading 文案。
    lx.chat.stop_armed_at = None;
    let mut buf = Buffer::empty(area);
    let _ = render(area, &mut buf, &lx, false, None);
    assert!(buffer_text(&buf).contains(text::LX_LOADING_LABEL));
}

#[test]
fn esc_hint_shortens_on_narrow_bar() {
    let area = Rect::new(0, 0, 28, 16);
    let mut lx = streaming_lx(true, true);
    lx.chat.stop_armed_at = Some(std::time::Instant::now());
    let mut buf = Buffer::empty(area);
    let _ = render(area, &mut buf, &lx, false, None);
    let text = buffer_text(&buf);
    assert!(text.contains("esc again"), "窄栏取短短语：{text}");
    assert!(
        !text.contains("Press esc again to stop"),
        "放不下时不画长短语：{text}"
    );
}
