//! lx 页单元测试：图标数据一致性、布局降级、命中测试、渲染、md 面板与输入高亮。

use super::*;
use crate::app::markdown::MentionEntry;
use crate::app::state::lx::slash::{ModelPicker, PanelWindow};
use crate::app::state::lx::{StartupCount, StartupCounts};
use crate::app::state::{
    AiWebSite, ChatMessageView, ChatPartView, LxChat, LxEntry, SlashCommand, SlashPanel,
};
use ratatui::buffer::{Buffer, CellDiffOption};
use ratatui::style::Color;
use std::path::Path;

mod bottom_bar;
mod chat_bar_and_history;
mod fork_panels;
mod mention_and_slash;
mod open_entry;
mod selection_copy;
mod slash_panels;

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
    let tinted: Vec<String> = LxEntry::ALL
        .iter()
        .map(|entry| {
            ICON_LX
                .iter()
                .map(|row| {
                    row.chars()
                        .map(|ch| icon_pixel(*entry, ch))
                        .collect::<String>()
                })
                .collect()
        })
        .collect();
    for left in 0..tinted.len() {
        for right in (left + 1)..tinted.len() {
            assert_ne!(
                tinted[left], tinted[right],
                "入口 {left} 与 {right} 的图标配色必须不同"
            );
        }
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
fn entry_label_gradients_are_distinct() {
    let endpoints: Vec<(Color, Color)> = LxEntry::ALL
        .iter()
        .map(|entry| style::lx_entry_gradient(*entry))
        .collect();
    for left in 0..endpoints.len() {
        for right in (left + 1)..endpoints.len() {
            assert_ne!(
                endpoints[left], endpoints[right],
                "入口 {left} 与 {right} 的渐变必须不同"
            );
        }
    }
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
    // 输入框下沿紧贴底部状态行（位于 `area.bottom() - 1`）。
    assert_eq!(input.bottom(), area.bottom() - BOTTOM_BAR_HEIGHT);
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
    assert_eq!(input.bottom(), area.bottom() - BOTTOM_BAR_HEIGHT);
}

#[test]
fn tiny_page_falls_back_to_wordmark() {
    let area = Rect::new(0, 0, 6, 5);
    assert!(layout(area, &Lx::new()).fallback);
    let mut buf = Buffer::empty(area);
    let _ = render(
        area,
        &mut buf,
        &Lx::new(),
        false,
        None,
        &mut PaneCache::default(),
    );
    assert!(buffer_text(&buf).contains("lx"));
}

#[test]
fn entries_are_horizontal_and_centered_when_roomy() {
    let area = Rect::new(0, 0, 100, 20);
    let lay = layout(area, &Lx::new());
    assert!(lay.horizontal);
    assert_eq!(lay.entries.len(), 4);
    let first_y = lay.entries[0].1.y;
    assert!(
        lay.entries.iter().all(|(_, rect)| rect.y == first_y),
        "同一行"
    );
    let xs: Vec<u16> = lay.entries.iter().map(|(_, rect)| rect.x).collect();
    assert!(xs[0] < xs[1] && xs[1] < xs[2] && xs[2] < xs[3], "从左到右");
}

#[test]
fn narrow_page_stacks_entries_vertically() {
    let area = Rect::new(0, 0, 34, 30);
    let lay = layout(area, &Lx::new());
    assert!(!lay.horizontal);
    let ys: Vec<u16> = lay.entries.iter().map(|(_, rect)| rect.y).collect();
    assert!(ys[1] > ys[0] && ys[2] > ys[1] && ys[3] > ys[2], "逐行堆叠");
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
    let _ = render(
        area,
        &mut buf,
        &Lx::new(),
        true,
        None,
        &mut PaneCache::default(),
    );
    let text = buffer_text(&buf);
    assert!(text.contains("Native"), "{text}");
    assert!(text.contains("Hybrid"), "{text}");
    assert!(text.contains("Automation"), "{text}");
    assert!(text.contains("Web"), "{text}");
    assert!(text.contains("Ask anything"), "{text}");
    assert!(text.contains("Tip:"), "Tip 前缀可见");
    assert!(text.contains('·'), "标签页分隔符可见");
    assert!(text.contains('─'), "输入框上下分割线可见");
}

#[test]
fn ai_web_entry_replaces_inventory_with_three_sites_and_brand_gradients() {
    let area = Rect::new(0, 0, 100, 60);
    let mut lx = Lx::new();
    lx.entry = LxEntry::AiWeb;
    let mut buf = Buffer::empty(area);
    let _ = render(area, &mut buf, &lx, true, None, &mut PaneCache::default());
    let text = buffer_text(&buf);
    assert!(text.contains("DeepSeek"), "{text}");
    assert!(text.contains("Gemini"), "{text}");
    assert!(text.contains("Grok"), "{text}");
    assert!(!text.contains("AI Web"), "{text}");
    assert!(text.contains("chat directly with web AI models"), "{text}");
    assert!(!text.contains("AGENTS.md"), "{text}");
    assert!(!text.contains("mcp"), "{text}");
    assert!(!text.contains("skills"), "{text}");

    let inventory = layout(area, &lx).inventory.expect("site row");
    assert_eq!(inventory.height, 2, "站点行包含标签行 + 选中下划线行");
    let placed = brand::place_ai_web_sites(inventory);
    assert_eq!(placed.len(), 3);
    let (_, ds_rect) = placed
        .iter()
        .find(|(site, _)| *site == AiWebSite::Deepseek)
        .copied()
        .expect("deepseek site");
    let (_, gm_rect) = placed
        .iter()
        .find(|(site, _)| *site == AiWebSite::Gemini)
        .copied()
        .expect("gemini site");
    let (_, gk_rect) = placed
        .iter()
        .find(|(site, _)| *site == AiWebSite::Grok)
        .copied()
        .expect("grok site");
    assert!(
        ds_rect.x < gm_rect.x && gm_rect.x < gk_rect.x,
        "顺序必须是 DeepSeek · Gemini · Grok"
    );

    assert_eq!(
        ai_web_site_hit(area, &lx, ds_rect.x, ds_rect.y),
        Some(AiWebSite::Deepseek)
    );
    assert_eq!(
        ai_web_site_hit(area, &lx, gm_rect.x, gm_rect.y),
        Some(AiWebSite::Gemini)
    );
    assert_eq!(
        ai_web_site_hit(area, &lx, gk_rect.x, gk_rect.y),
        Some(AiWebSite::Grok)
    );

    // 默认选中 DeepSeek：DeepSeek 加粗且 y+1 有 `─` 下划线，其它站点保持品牌渐变但无下划线。
    assert_eq!(buf[(ds_rect.x, ds_rect.y)].fg, Color::Indexed(63));
    assert_eq!(
        buf[(ds_rect.x + ds_rect.width - 1, ds_rect.y)].fg,
        Color::Indexed(39),
        "DeepSeek 标签末端必须落在渐变终点"
    );
    assert!(
        buf[(ds_rect.x, ds_rect.y)]
            .modifier
            .contains(ratatui::style::Modifier::BOLD)
    );
    assert_eq!(buf[(ds_rect.x, ds_rect.y + 1)].symbol(), "─");
    assert_eq!(
        buf[(ds_rect.x + ds_rect.width - 1, ds_rect.y + 1)].fg,
        Color::Indexed(39),
        "下划线沿用同一逐字渐变"
    );
    assert_eq!(buf[(gm_rect.x, gm_rect.y)].fg, Color::Indexed(69));
    assert_eq!(
        buf[(gm_rect.x + gm_rect.width - 1, gm_rect.y)].fg,
        Color::Indexed(204),
        "Gemini 标签末端必须落在渐变终点"
    );
    assert!(
        !buf[(gm_rect.x, gm_rect.y)]
            .modifier
            .contains(ratatui::style::Modifier::BOLD)
    );
    assert_eq!(buf[(gm_rect.x, gm_rect.y + 1)].symbol(), " ");
    assert_eq!(buf[(gk_rect.x, gk_rect.y)].fg, Color::Indexed(255));
    assert_eq!(
        buf[(gk_rect.x + 1, gk_rect.y)].fg,
        Color::Indexed(253),
        "Grok 渐变第二个字符必须是灰阶插值，而不是硬切到 250"
    );
    assert_eq!(
        buf[(gk_rect.x + 2, gk_rect.y)].fg,
        Color::Indexed(252),
        "Grok 渐变第三个字符必须是灰阶插值，而不是硬切到 250"
    );
    assert_eq!(
        buf[(gk_rect.x + gk_rect.width - 1, gk_rect.y)].fg,
        Color::Indexed(250),
        "Grok 标签末端必须落在白到浅灰的渐变终点"
    );
    assert!(
        !buf[(gk_rect.x, gk_rect.y)]
            .modifier
            .contains(ratatui::style::Modifier::BOLD)
    );
    assert_eq!(buf[(gk_rect.x, gk_rect.y + 1)].symbol(), " ");

    // 切换到 Grok：Grok 加粗且 y+1 出现同品牌渐变下划线。
    lx.ai_web_site = AiWebSite::Grok;
    let mut buf2 = Buffer::empty(area);
    let _ = render(area, &mut buf2, &lx, true, None, &mut PaneCache::default());
    assert!(
        buf2[(gk_rect.x, gk_rect.y)]
            .modifier
            .contains(ratatui::style::Modifier::BOLD)
    );
    assert_eq!(buf2[(gk_rect.x, gk_rect.y + 1)].symbol(), "─");
    assert_eq!(
        buf2[(gk_rect.x + gk_rect.width - 1, gk_rect.y + 1)].fg,
        Color::Indexed(250)
    );
    assert_eq!(buf2[(ds_rect.x, ds_rect.y + 1)].symbol(), " ");

    lx.entry = LxEntry::Native;
    assert_eq!(ai_web_site_hit(area, &lx, ds_rect.x, ds_rect.y), None);
}

#[test]
fn narrow_ai_web_site_row_keeps_three_sites_visible_and_hittable() {
    // 窄窗格把站点间隔降为 1：8 列即可放下 3 个 2 字符标签。
    for width in [8, 9, 10, 12, 16] {
        let rect = Rect::new(0, 0, width, 2);
        let placed = brand::place_ai_web_sites(rect);
        assert_eq!(placed.len(), 3, "width={width} 必须同时放下三个站点");
        for (_, site_rect) in &placed {
            assert!(site_rect.width > 0, "width={width} 站点宽度必须大于 0");
            assert!(
                site_rect.right() <= rect.right(),
                "width={width} 站点不得越过可用区域"
            );
        }
        assert_eq!(placed[0].0, AiWebSite::Deepseek);
        assert_eq!(placed[1].0, AiWebSite::Gemini);
        assert_eq!(placed[2].0, AiWebSite::Grok);
        for pair in placed.windows(2) {
            assert!(
                pair[0].1.right() <= pair[1].1.x,
                "width={width} 相邻站点不得重叠"
            );
        }
        let labels: Vec<String> = placed
            .iter()
            .map(|(site, site_rect)| brand::ai_web_site_label_for_width(*site, site_rect.width))
            .collect();
        if width == 8 {
            assert_eq!(labels, vec!["De", "Ge", "Gr"], "width=8 标签");
        }
        if width == 10 {
            assert_eq!(labels, vec!["Dee", "Gem", "Gr"], "width=10 标签");
        }
    }
}

#[test]
fn resource_counts_sit_one_blank_row_below_entries() {
    let area = Rect::new(0, 0, 100, 60);
    let lx = Lx::new();
    let lay = layout(area, &lx);
    let entries = lay.entries;
    let last_y = entries
        .iter()
        .map(|(_, rect)| rect.y)
        .max()
        .expect("entries");
    let inventory = lay.inventory.expect("inventory row");
    assert_eq!(inventory.y, last_y + 3, "label + underline + 1 blank row");
}

#[test]
fn home_shows_mcp_and_skill_counts() {
    let area = Rect::new(0, 0, 100, 60);
    let mut lx = Lx::new();
    lx.startup = StartupCounts {
        agents_md: 2,
        mcp: StartupCount {
            total: 4,
            ready: 4,
            errors: 0,
            loading: false,
        },
        skills: StartupCount {
            total: 3,
            ready: 3,
            errors: 0,
            loading: false,
        },
    };
    let mut buf = Buffer::empty(area);
    let _ = render(area, &mut buf, &lx, true, None, &mut PaneCache::default());
    let text = buffer_text(&buf);
    assert!(text.contains("2 AGENTS.md · 4 mcp · 3 skills"), "{text}");

    // 后台连接/扫描仍在途时，不再显示阻塞性的 `loading mcp`，而是实时展示当前已就绪数量。
    lx.startup.mcp.ready = 1;
    lx.startup.mcp.loading = true;
    lx.startup.skills.loading = true;
    let mut buf = Buffer::empty(area);
    let _ = render(area, &mut buf, &lx, true, None, &mut PaneCache::default());
    let text = buffer_text(&buf);
    assert!(text.contains("2 AGENTS.md · 1 mcp · 3 skills"), "{text}");
    assert!(!text.contains("loading"), "{text}");

    lx.startup = StartupCounts {
        agents_md: 1,
        mcp: StartupCount {
            total: 4,
            ready: 3,
            errors: 1,
            loading: false,
        },
        skills: StartupCount {
            total: 3,
            ready: 3,
            errors: 0,
            loading: false,
        },
    };
    let mut buf = Buffer::empty(area);
    let _ = render(area, &mut buf, &lx, true, None, &mut PaneCache::default());
    let text = buffer_text(&buf);
    assert!(
        text.contains("1 AGENTS.md · 3 mcp 1 error · 3 skills"),
        "{text}"
    );

    // 窄窗格容不下全段（34 列 < 38 列）时，优先省略左侧 AGENTS.md，完整保留 mcp 1 error · skills。
    let narrow = Rect::new(0, 0, 34, 60);
    let mut narrow_buf = Buffer::empty(narrow);
    let _ = render(
        narrow,
        &mut narrow_buf,
        &lx,
        true,
        None,
        &mut PaneCache::default(),
    );
    let narrow_text = buffer_text(&narrow_buf);
    assert!(
        narrow_text.contains("3 mcp 1 error · 3 skills"),
        "{narrow_text}"
    );
    assert!(!narrow_text.contains("AGENTS.md"), "{narrow_text}");
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
    let _ = render(area, &mut buf, &lx, true, None, &mut PaneCache::default());
    assert!(buffer_text(&buf).contains("mixes native and external agents"));
}

#[test]
fn selected_entry_has_underline_below() {
    let area = Rect::new(0, 0, 100, 60);
    let mut buf = Buffer::empty(area);
    let _ = render(
        area,
        &mut buf,
        &Lx::new(),
        true,
        None,
        &mut PaneCache::default(),
    );
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
    let _ = render(area, &mut buf, &lx, true, None, &mut PaneCache::default());
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
    let _ = render(area, &mut buf, &lx, false, None, &mut PaneCache::default());
    let row = area.bottom() - 3;
    let content_x = area.x + MARGIN + 2;
    // 输入框走**编辑器**高亮色板（`markdown_*`，独立于阅读渲染的 `md_read_*`）：
    // `#` 为标题专用标记（Yellow + dim），`Title` 为 Yellow 加粗标题。
    assert_eq!(
        buf[(content_x, row)].fg,
        crate::ui::style::markdown_heading_marker()
            .fg
            .unwrap_or(Color::Reset)
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
    let cursor =
        render(area, &mut buf, &lx, true, None, &mut PaneCache::default()).expect("cursor");
    assert!(buffer_text(&buf).contains("hi"));
    // 行相对内容区：最末行为状态行，输入框底分割线在倒数第 2 行，正文行在倒数第 3 行。
    assert_eq!(cursor.0, area.height - 3);
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
    let cursor =
        render(area, &mut buf, &lx, true, None, &mut PaneCache::default()).expect("cursor");
    // 行相对内容区：最末行为状态行，输入框高 4，末行正文在倒数第 3 行。
    assert_eq!(cursor.0, area.height - 3);
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
    let cursor =
        render(area, &mut buf, &lx, true, None, &mut PaneCache::default()).expect("cursor");
    let row = area.bottom() - 3;
    let content_x = area.x + MARGIN + 2;
    assert_eq!(buf[(content_x, row)].symbol(), "你");
    assert_eq!(
        buf[(content_x + 1, row)].diff_option,
        CellDiffOption::Skip,
        "宽字符后续列标记跳过"
    );
    assert_eq!(buf[(content_x + 2, row)].symbol(), "谁");
    assert_eq!(cursor, (area.height - 3, MARGIN + 2 + 4));
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

/// 构造溢出视口的历史：两轮问答，正文各若干行。
fn sticky_history_lx() -> Lx {
    let mut lx = Lx::new();
    let mut chat = LxChat::new();
    let mut push = |role: &str, text: &str| {
        chat.messages.push(ChatMessageView {
            sequence: 0,
            id: format!("{role}-{}", chat.messages.len()),
            role: role.into(),
            status: "complete".into(),
            turn_index: 0,
            kind: "chat".into(),
            shell: None,
            steps: vec![crate::app::state::AssistantStepView::Part {
                position: 0,
                part: ChatPartView {
                    kind: crate::chat::events::PartKind::Text,
                    content: text.into(),
                    provider_metadata_json: None,
                    started_at_ms: None,
                    ended_at_ms: None,
                    completed_at_ms: None,
                },
            }],
            model_label: None,
            error_code: None,
            input_tokens: None,
            output_tokens: None,
            cache_read_tokens: None,
            duration_ms: None,
            reports: Vec::new(),
            attachments: Vec::new(),
        });
    };
    push("user", "第一轮的问题");
    push("assistant", "A1\nA2\nA3\nA4\nA5");
    push("user", "第二轮的问题");
    push("assistant", "B1\nB2\nB3\nB4\nB5");
    lx.chat = chat;
    lx
}

/// 构造带候选缓存的 lx：根已同步、`@` 已键入、扫描结果已写入。
fn mention_lx(entries: Vec<MentionEntry>) -> Lx {
    let mut lx = Lx::new();
    lx.set_mention_root(Some(std::path::PathBuf::from("/work/demo")));
    lx.insert_char('@');
    let (generation, _) = lx.take_mention_scan_request().expect("scan request");
    lx.apply_mention_entries(generation, Path::new("/work/demo"), entries);
    lx
}

/// 构造带 `/` 命令候选面板的 lx 状态。
fn slash_commands_lx(active: usize) -> Lx {
    let mut lx = Lx::new();
    lx.slash = Some(SlashPanel::Commands {
        items: SlashCommand::ALL
            .into_iter()
            .map(crate::app::state::SlashEntry::Command)
            .collect(),
        window: PanelWindow::new(active),
        query: "/".into(),
    });
    lx
}

/// 构造带 `/resume` 空历史面板的 lx 状态。
fn slash_resume_empty_lx() -> Lx {
    let mut lx = Lx::new();
    lx.slash = Some(SlashPanel::History(Box::default()));
    lx
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
    let _ = render(area, &mut buf, &lx, true, None, &mut PaneCache::default());
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
    let _ = render(area, &mut buf, &lx, true, None, &mut PaneCache::default());
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
    let _ = render(area, &mut buf, &lx, true, None, &mut PaneCache::default());
    let text = buffer_text(&buf);
    // 轨道不画符号，只看 thumb：滚动条列只剩 `▐`。
    assert!(text.contains('▐'), "溢出时滚动条可见: {text}");
    assert!(!text.contains('▕'), "轨道不再绘制竖线: {text}");
}

#[test]
fn lx_input_highlights_inline_code_and_list_markers() {
    // lx 输入框与 prompt 容器共用 `token_style`：`xxx` 带底色、列表符号上强调色。
    let area = Rect::new(0, 0, 60, 20);
    let mut lx = Lx::new();
    lx.insert_str("use `cargo test`\n- bullet item\n1. ordered item");
    let mut buf = Buffer::empty(area);
    let _ = render(area, &mut buf, &lx, true, None, &mut PaneCache::default());

    // 行内代码内容格带前景色（不铺背景）。
    let mut code_colored = false;
    for y in 0..area.height {
        for x in 0..area.width {
            if buf[(x, y)].fg == ratatui::style::Color::LightRed {
                code_colored = true;
            }
        }
    }
    assert!(code_colored, "输入框里的行内代码有前景色");

    // 列表符号（`•`/序号）带前景色，正文无。
    let mut marker_colored = false;
    for y in 0..area.height {
        for x in 0..area.width {
            let symbol = buf[(x, y)].symbol();
            if (symbol == "•" || symbol == "1") && buf[(x, y)].fg == ratatui::style::Color::Yellow
            {
                marker_colored = true;
            }
        }
    }
    assert!(marker_colored, "列表符号有强调色");
}
