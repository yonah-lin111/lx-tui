//! `@` 提及面板与 `/` 命令、`/model`、`/rewind` 面板几何及 ASCII 渲染测试。

use super::*;
use crate::app::state::lx::slash::{ModelPicker, PanelWindow};
use crate::app::state::{ModelCandidate, ModelPickerStage, SlashCommand, SlashPanel};

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
    // 输入框贴近内容区顶部：上方放不下顶部分割线 + 一行，不绘制（不移动输入框）。
    let area = Rect::new(0, 0, 40, 5);
    let mut lx = mention_lx(vec![MentionEntry {
        path: "file.rs".into(),
        is_directory: false,
    }]);
    // 底部状态行占 1 行、输入框至少 3 行，此时上方仅 1 行：放不下面板（需至少 2 行）。
    let input = layout(area, &lx).input.expect("5 行高仍保留输入框");
    assert!(mention::layout(area, &lx).is_none(), "空间不足不绘制面板");
    assert_eq!(layout(area, &lx).input, Some(input), "输入框位置不变");
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
fn mention_panel_renders_tabs_and_directory_rows() {
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
    let _ = render(area, &mut buf, &lx, true, None, &mut PaneCache::default());
    let text = buffer_text(&buf);
    assert!(text.contains("Files"), "顶部 Files Tab 可见: {text}");
    assert!(text.contains("refDir"), "顶部 refDir Tab 可见: {text}");
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
    let _ = render(area, &mut buf, &lx, true, None, &mut PaneCache::default());
    let text = buffer_text(&buf);
    assert!(text.contains("scanning"), "扫描中状态行: {text}");

    // 空结果：状态行而非隐藏。
    let empty = mention_lx(Vec::new());
    let mut buf = Buffer::empty(area);
    let _ = render(
        area,
        &mut buf,
        &empty,
        true,
        None,
        &mut PaneCache::default(),
    );
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
    let _ = render(area, &mut buf, &lx, true, None, &mut PaneCache::default());
    let panel = mention::layout(area, &lx).expect("panel");
    // 面板右边界不超过输入框右边界。
    let input = layout(area, &lx).input.expect("input");
    assert_eq!(panel.rect.right(), input.right());
}

#[test]
fn mention_panel_height_capped_at_max_popup_rows() {
    // 内容区很高：候选很多，面板内容行仍夹到 MAX_POPUP_ROWS（8）+ 顶部横线 1 行 + 底边提示 1 行。
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
        mention::MAX_POPUP_ROWS + mention::PANEL_BORDER_ROWS + mention::PANEL_FOOTER_ROWS,
        "高度对齐 codex 的 MAX_POPUP_ROWS + 顶部横线 + 底边空行与提示"
    );
    assert!(panel.scrollbar.is_some(), "超出上限时出现滚动条");
}

#[test]
fn mention_panel_height_shrinks_to_content_when_few_rows() {
    // 候选较少时：面板高度按候选行数自适应收缩（顶部 3 行 + 根层单行候选 1 行 + 底部 3 行 = 7 行）。
    let area = Rect::new(0, 0, 60, 60);
    let entries = vec![MentionEntry {
        path: "only.rs".into(),
        is_directory: false,
    }];
    let lx = mention_lx(entries);
    let panel = mention::layout(area, &lx).expect("panel");
    assert_eq!(
        panel.rect.height,
        1 + mention::PANEL_BORDER_ROWS + mention::PANEL_FOOTER_ROWS,
        "少量候选按实际高度收缩: {}",
        panel.rect.height
    );
    assert!(panel.scrollbar.is_none(), "未超出最大内容行时不显示滚动条");
}

#[test]
fn mention_panel_renders_top_rule_with_title_and_tabs_and_borderless_footer() {
    let area = Rect::new(0, 0, 80, 24);
    let lx = mention_lx(vec![MentionEntry {
        path: "src/main.rs".into(),
        is_directory: false,
    }]);
    let panel = mention::layout(area, &lx).expect("panel");
    let mut buf = Buffer::empty(area);
    let _ = render(area, &mut buf, &lx, true, None, &mut PaneCache::default());
    // 顶部第 0 行为独立蓝色横线，第 1 行为干净的标题行（嵌 @Command 标题与 Tab，无横线穿透）
    assert_eq!(
        buf[(panel.rect.x, panel.rect.y)].symbol(),
        "─",
        "顶部为独立蓝色横线"
    );
    assert_eq!(
        buf[(panel.rect.x, panel.rect.y)].fg,
        ratatui::style::Color::Cyan,
        "顶线为强调色"
    );
    let header_row: String = (panel.rect.x..panel.rect.right())
        .map(|x| buf[(x, panel.rect.y + 1)].symbol())
        .collect();
    assert!(
        header_row.contains("@Command"),
        "标题行包含 @Command 标题: {header_row}"
    );
    assert!(
        header_row.contains("Files"),
        "标题行包含 Files 标签: {header_row}"
    );
    assert!(!header_row.contains('─'), "标题行无横线穿透: {header_row}");
    let footer_row = panel.rect.bottom() - 1;
    let footer: String = (panel.rect.x..panel.rect.right())
        .map(|x| buf[(x, footer_row)].symbol())
        .collect();
    assert!(
        footer.contains("←→ tabs") && footer.contains("esc close"),
        "底栏显示快捷键提示: {footer:?}"
    );
    assert!(
        !footer.contains('─') && !footer.contains('│'),
        "底栏无边框与分割线: {footer:?}"
    );
}

#[test]
fn mention_panel_height_limited_by_space_above_input() {
    // 输入框贴顶：可用行数小于上限时，面板高度受可用空间约束。
    let area = Rect::new(0, 0, 60, 9);
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
        current: None,
        current_variant: None,
        window: PanelWindow::new(0),
        filter: String::new(),
        filtered: vec![0, 1],
    })));
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
        current: None,
        current_variant: None,
        window: PanelWindow::new(0),
        filter: String::new(),
        filtered: Vec::new(),
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
    let built = build(area, &lx, true, &mut PaneCache::default()).grid;
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
fn model_panel_shows_provider_and_model_names_not_ids() {
    let area = Rect::new(0, 0, 80, 24);
    let lx = slash_model_lx();
    let mut buf = Buffer::empty(area);
    let _ = render(area, &mut buf, &lx, false, None, &mut PaneCache::default());
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

/// 撤销动作面板复用 `/` 面板：固定四项、默认第一项、快照不可用时仍可见。
#[test]
fn rewind_panel_lists_four_actions() {
    use crate::app::state::{RewindPanel, SlashPanel, SnapshotState};
    let panel = SlashPanel::Rewind(Box::new(RewindPanel::new(SnapshotState::KnownEmpty)));
    let items = super::slash::items(&panel, &[], &[]);
    assert_eq!(items.len(), 4, "四项动作固定顺序");
    assert_eq!(panel.window().active, 0, "默认第一项");
    let disabled = SlashPanel::Rewind(Box::new(RewindPanel::new(SnapshotState::Unavailable)));
    assert_eq!(
        super::slash::items(&disabled, &[], &[]).len(),
        4,
        "禁用仍可见"
    );
}
