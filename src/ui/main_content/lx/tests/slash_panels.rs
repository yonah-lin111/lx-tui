//! `/model`、`/resume` 面板渲染测试：当前项 `❯` 标记、同底色高亮、时间列与去轨道滚动条。

use super::*;
use crate::app::state::lx::slash::{HistoryEntry, HistoryPanel};
use crate::app::state::{
    ConnectManageAction, ConnectPanel, ConnectPromptStep, ConnectStage, ModelCandidate,
    ModelPickerStage, PanelWindow, SlashPanel,
};
use crate::config::ai::connect::ConnectSettings;

/// 把整页缓冲区拼成文本行。
pub(super) fn buffer_lines(buf: &Buffer, area: Rect) -> Vec<String> {
    (0..area.height)
        .map(|y| {
            (0..area.width)
                .map(|x| buf[(x, y)].symbol().chars().next().unwrap_or(' '))
                .collect()
        })
        .collect()
}

/// 三个候选的 `/model` 第一级面板，指定索引为"当前模型"。
fn model_panel_lx(current: Option<usize>) -> Lx {
    let mut lx = Lx::new();
    lx.slash = Some(SlashPanel::Model(Box::new(ModelPicker {
        stage: ModelPickerStage::Models,
        candidates: (0..3)
            .map(|index| ModelCandidate {
                selection: crate::config::ai::ModelSelection {
                    provider: "p".into(),
                    model: format!("model-{index}"),
                    variant: None,
                },
                provider_name: "Provider".into(),
                model_name: format!("model-{index}"),
            })
            .collect(),
        current,
        current_variant: None,
        window: PanelWindow::new(current.unwrap_or(0)),
        filter: String::new(),
        filtered: (0..3).collect(),
    })));
    lx
}

/// 两条历史的 `/resume` 面板，第 2 条为"当前会话"（5 分钟前更新）。
fn history_panel_lx() -> Lx {
    let mut lx = Lx::new();
    lx.slash = Some(SlashPanel::History(Box::new(HistoryPanel {
        entries: vec![
            HistoryEntry {
                session_id: "sess-a".into(),
                title: "Alpha".into(),
                status: "active".into(),
                updated_at_ms: 1_000,
            },
            HistoryEntry {
                session_id: "sess-b".into(),
                title: "Beta".into(),
                status: "active".into(),
                updated_at_ms: 2_000,
            },
        ],
        current: Some(1),
        now_ms: 2_000 + 5 * 60_000,
        filter: String::new(),
        filtered: vec![0, 1],
        window: PanelWindow::new(1),
    })));
    lx
}

/// `/connect` 面板，指定阶段。
fn connect_panel_lx(stage: ConnectStage) -> Lx {
    let mut panel = ConnectPanel::new_with_env(&ConnectSettings::default(), |_| None);
    panel.stage = stage;
    let mut lx = Lx::new();
    lx.slash = Some(SlashPanel::Connect(Box::new(panel)));
    lx
}

fn render_lines(lx: &Lx) -> Vec<String> {
    let area = Rect::new(0, 0, 100, 30);
    let mut buf = Buffer::empty(area);
    let _ = render(area, &mut buf, lx, false, None, &mut PaneCache::default());
    buffer_lines(&buf, area)
}

#[test]
fn connect_providers_panel_renders_names_notes_and_status_tags() {
    let lx = connect_panel_lx(ConnectStage::Providers);
    let lines = render_lines(&lx);
    let text = lines.join("\n");
    assert!(text.contains("302.AI"), "首个 A-Z 供应商可见: {lines:?}");
    assert!(text.contains("Connect"), "面板标题: {lines:?}");
    assert!(text.contains("type to filter"), "底栏提示: {lines:?}");
}

#[test]
fn connect_manage_panel_renders_actions() {
    let lx = connect_panel_lx(ConnectStage::Manage {
        provider_id: "anthropic".into(),
        provider_name: "Anthropic".into(),
        source: crate::config::ai::connect::ConnectionSource::ApiKey,
        actions: vec![
            ConnectManageAction::SelectModel,
            ConnectManageAction::Reconnect,
            ConnectManageAction::Disconnect,
        ],
        window: PanelWindow::default(),
    });
    let text = render_lines(&lx).join("\n");
    assert!(text.contains("Manage connection"));
    assert!(text.contains("Select model"));
    assert!(text.contains("Reconnect / Replace credentials"));
    assert!(text.contains("Disconnect"));
    assert!(text.contains("Anthropic"));
}

#[test]
fn connect_methods_panel_renders_oauth_before_api_key() {
    let methods = crate::config::ai::presets::find_preset("openai")
        .expect("openai")
        .methods;
    let lx = connect_panel_lx(ConnectStage::Methods {
        provider_id: "openai".into(),
        provider_name: "OpenAI".into(),
        methods,
        window: PanelWindow::default(),
    });
    let lines = render_lines(&lx);
    let text = lines.join("\n");
    assert!(text.contains("Sign-in method"));
    assert!(text.contains("ChatGPT Plus/Pro (browser)"));
    assert!(text.contains("OAuth"));
    assert!(text.contains("API key"));
}

#[test]
fn connect_form_options_panel_renders_choices() {
    let field = crate::config::ai::presets::find_preset("github-copilot")
        .expect("copilot")
        .methods[0]
        .form()[0]
        .clone();
    let lx = connect_panel_lx(ConnectStage::InputPrompt {
        provider_id: "github-copilot".into(),
        provider_name: "GitHub Copilot".into(),
        method_index: 0,
        step: ConnectPromptStep::FormField {
            field_index: 0,
            field,
            option_window: PanelWindow::default(),
        },
        answers: std::collections::BTreeMap::new(),
        error: None,
    });
    let text = render_lines(&lx).join("\n");
    assert!(text.contains("GitHub.com"));
    assert!(text.contains("GitHub Enterprise"));
}

#[test]
fn connect_api_key_panel_renders_provider_and_paste_hint() {
    let lx = connect_panel_lx(ConnectStage::InputPrompt {
        provider_id: "anthropic".into(),
        provider_name: "Anthropic".into(),
        method_index: 0,
        step: ConnectPromptStep::ApiKey,
        answers: std::collections::BTreeMap::new(),
        error: None,
    });
    let body = render_lines(&lx).join("\n");
    assert!(body.contains("Anthropic"));
    assert!(body.contains(text::LX_CONNECT_API_KEY_PROMPT));
}

#[test]
fn connect_oauth_panel_renders_open_copy_and_waiting_status() {
    let lx = connect_panel_lx(ConnectStage::OAuthWaiting {
        attempt_id: "a".into(),
        provider_id: "github-copilot".into(),
        provider_name: "GitHub Copilot".into(),
        method_id: "device".into(),
        form_answers: std::collections::BTreeMap::new(),
        url: "https://github.com/login/device".into(),
        user_code: Some("ABCD-1234".into()),
        instructions: "Enter code: ABCD-1234".into(),
        mode: crate::config::ai::oauth::OAuthPromptMode::Auto,
        error: None,
        window: PanelWindow::default(),
    });
    let text = render_lines(&lx).join("\n");
    assert!(text.contains("Open in browser"));
    assert!(text.contains("Copy code / URL"));
    assert!(text.contains("ABCD-1234"));
    assert!(text.contains("https://github.com/login/device"));
    assert!(text.contains("Enter code: ABCD-1234"));
}

#[test]
fn model_panel_marks_current_model_with_marker() {
    let area = Rect::new(0, 0, 80, 24);
    let lx = model_panel_lx(Some(1));
    let mut buf = Buffer::empty(area);
    let _ = render(area, &mut buf, &lx, false, None, &mut PaneCache::default());
    let lines = buffer_lines(&buf, area);
    let marked: Vec<&String> = lines
        .iter()
        .filter(|line| line.contains(text::CURRENT_ITEM_MARKER))
        .collect();
    assert_eq!(marked.len(), 1, "只有当前模型带标记: {lines:?}");
    assert!(marked[0].contains("model-1"), "{:?}", marked[0]);
}

#[test]
fn model_panel_without_current_model_marks_active_with_default_marker() {
    let area = Rect::new(0, 0, 80, 24);
    let lx = model_panel_lx(None);
    let panel = slash_layout(area, &lx).expect("slash layout");
    let mut buf = Buffer::empty(area);
    let _ = render(area, &mut buf, &lx, false, None, &mut PaneCache::default());
    let row0 = panel.content.y;
    assert_eq!(
        buf[(panel.content.x, row0)].symbol(),
        text::CURRENT_ITEM_MARKER,
        "正在选择的项带 ❯ 标记"
    );
    assert_eq!(
        buf[(panel.content.x, row0)].bg,
        ratatui::style::Color::Reset,
        "无边框面板保持透明底色"
    );
}

#[test]
fn model_panel_renders_top_blue_rule_and_markers() {
    // 顶部单条蓝色横线、面板透明底色；当前模型标为蓝色 ❯ + 蓝色名称，正在选择的行带默认色 ❯。
    let area = Rect::new(0, 0, 80, 24);
    let mut lx = model_panel_lx(Some(0));
    if let Some(SlashPanel::Model(picker)) = lx.slash.as_mut() {
        picker.window.active = 1;
        picker.window.anchor = 1;
    }
    let panel = slash_layout(area, &lx).expect("slash layout");
    let mut buf = Buffer::empty(area);
    let _ = render(area, &mut buf, &lx, false, None, &mut PaneCache::default());

    // 顶部第 0 行为独立蓝色横线，第 1 行为干净的标题行。
    assert_eq!(buf[(panel.rect.x, panel.rect.y)].symbol(), "─");
    assert_eq!(
        buf[(panel.rect.x, panel.rect.y)].fg,
        ratatui::style::Color::Cyan,
        "顶部横线为蓝色"
    );
    for y in panel.rect.y..panel.rect.bottom() {
        for x in panel.rect.x..panel.rect.right() {
            assert_ne!(
                buf[(x, y)].bg,
                ratatui::style::Color::Indexed(240),
                "无边框模式移除条目选中背景色"
            );
        }
    }
    // 第 0 行是当前生效模型（current=0, active=1）：蓝色 ❯ + 蓝色名称。
    let row0 = panel.content.y;
    assert_eq!(
        buf[(panel.content.x, row0)].symbol(),
        text::CURRENT_ITEM_MARKER
    );
    assert_eq!(
        buf[(panel.content.x, row0)].fg,
        ratatui::style::Color::Cyan,
        "已选中模型的 ❯ 为蓝色"
    );
    assert_eq!(
        buf[(panel.content.x + 2, row0)].fg,
        ratatui::style::Color::Cyan,
        "已选中模型的名称为蓝色"
    );
    // 第 1 行是正在选择的模型（active=1, current=0）：默认颜色 ❯ + 默认颜色名称。
    let row1 = panel.content.y + 1;
    assert_eq!(
        buf[(panel.content.x, row1)].symbol(),
        text::CURRENT_ITEM_MARKER
    );
    assert_eq!(
        buf[(panel.content.x, row1)].fg,
        ratatui::style::Color::Reset,
        "正在选择的项 ❯ 为默认颜色"
    );
}

#[test]
fn model_panel_names_stay_aligned_with_marker_column() {
    // 按单元格验证：标记行 `❯ 名字`，未标记行 `  名字`，名字左缘同列。
    let area = Rect::new(0, 0, 80, 24);
    let lx = model_panel_lx(Some(1));
    let mut buf = Buffer::empty(area);
    let _ = render(area, &mut buf, &lx, false, None, &mut PaneCache::default());
    let line_at = |y: u16| -> String { (0..area.width).map(|x| buf[(x, y)].symbol()).collect() };
    let row_of = |needle: &str| -> u16 {
        (0..area.height)
            .find(|y| line_at(*y).contains(needle))
            .expect("row")
    };
    let marked_row = row_of("model-1");
    let plain_row = row_of("model-0");
    assert!(
        line_at(marked_row).contains(&format!("{} model-1", text::CURRENT_ITEM_MARKER)),
        "标记行是 `❯ 名字`: {:?}",
        line_at(marked_row)
    );
    // 名字首字符的单元格列：标记列只占宽度，不挤动名字左缘。
    let first_name_cell = |line: &str, name: &str| -> u16 {
        let chars: Vec<char> = line.chars().collect();
        let needle: Vec<char> = name.chars().collect();
        let width = chars.len();
        let name_width = needle.len();
        (0..width.saturating_sub(name_width))
            .find(|start| chars[*start..*start + name_width] == needle[..])
            .map(|start| u16::try_from(start).unwrap_or(0))
            .unwrap_or(0)
    };
    assert_eq!(
        first_name_cell(&line_at(marked_row), "model-1"),
        first_name_cell(&line_at(plain_row), "model-0"),
        "标记项与未标记项名字同列"
    );
}

#[test]
fn history_panel_shows_relative_time_instead_of_session_id() {
    let area = Rect::new(0, 0, 80, 24);
    let lx = history_panel_lx();
    let mut buf = Buffer::empty(area);
    let _ = render(area, &mut buf, &lx, false, None, &mut PaneCache::default());
    let lines = buffer_lines(&buf, area);
    let all = lines.join("\n");
    assert!(all.contains("5m ago"), "显示相对时间: {all}");
    assert!(!all.contains("sess-a"), "不再显示会话 id: {all}");
    assert!(
        all.contains(text::CURRENT_ITEM_MARKER),
        "当前会话带标记: {all}"
    );
}

#[test]
fn history_panel_marks_only_bound_session() {
    let area = Rect::new(0, 0, 80, 24);
    let lx = history_panel_lx();
    let mut buf = Buffer::empty(area);
    let _ = render(area, &mut buf, &lx, false, None, &mut PaneCache::default());
    let lines = buffer_lines(&buf, area);
    let marked: Vec<&String> = lines
        .iter()
        .filter(|line| line.contains(text::CURRENT_ITEM_MARKER))
        .collect();
    assert_eq!(marked.len(), 1, "只有已绑定会话带标记: {lines:?}");
    assert!(marked[0].contains("Beta"), "{:?}", marked[0]);
}

#[test]
fn slash_panel_scrollbar_has_no_track_border() {
    // 候选多于可见行时出现滚动条：只有 thumb，没有轨道竖线。
    let area = Rect::new(0, 0, 80, 24);
    let mut lx = Lx::new();
    lx.slash = Some(SlashPanel::Model(Box::new(ModelPicker {
        stage: ModelPickerStage::Models,
        candidates: (0..30)
            .map(|index| ModelCandidate {
                selection: crate::config::ai::ModelSelection {
                    provider: "p".into(),
                    model: format!("model-{index}"),
                    variant: None,
                },
                provider_name: "Provider".into(),
                model_name: format!("model-{index}"),
            })
            .collect(),
        current: Some(0),
        current_variant: None,
        window: PanelWindow::new(0),
        filter: String::new(),
        filtered: (0..30).collect(),
    })));
    let mut buf = Buffer::empty(area);
    let _ = render(area, &mut buf, &lx, false, None, &mut PaneCache::default());
    let layout = slash_layout(area, &lx).expect("panel");
    let bar = layout.scrollbar.expect("overflow");
    let lines = buffer_lines(&buf, area);
    assert!(lines.iter().any(|line| line.contains('▐')), "thumb 可见");
    assert!(
        lines.iter().all(|line| !line.contains('▕')),
        "轨道不再画竖线: {lines:?}"
    );
    // 轨道列在 thumb 之外是空格（保留占位，不画符号）。
    for y in bar.track.y..bar.track.bottom() {
        if y >= bar.thumb.y && y < bar.thumb.bottom() {
            continue;
        }
        let row: Vec<char> = lines[usize::from(y)].chars().collect();
        assert_eq!(row[usize::from(bar.track.x)], ' ', "轨道列留空");
    }
}

#[test]
fn history_panel_renders_top_rule_and_borderless_footer() {
    let area = Rect::new(0, 0, 100, 24);
    let lx = history_panel_lx();
    let panel = slash_layout(area, &lx).expect("history layout");
    let mut buf = Buffer::empty(area);
    let _ = render(area, &mut buf, &lx, false, None, &mut PaneCache::default());
    let lines = buffer_lines(&buf, area);
    let all = lines.join("\n");
    assert!(
        all.contains(crate::ui::text::LX_HISTORY_PANEL_TITLE),
        "顶栏显示标题: {all}"
    );
    let footer_row = usize::from(panel.rect.bottom() - 1);
    assert!(
        lines[footer_row].contains(crate::ui::text::LX_HISTORY_PANEL_FOOTER),
        "底边显示快捷键提示: {:?}",
        lines[footer_row]
    );
    assert!(
        !lines[footer_row].contains('─') && !lines[footer_row].contains('│'),
        "底边提示行无边框: {:?}",
        lines[footer_row]
    );
    assert_eq!(
        panel.rect.height,
        2 + mention::PANEL_BORDER_ROWS + mention::PANEL_FOOTER_ROWS,
        "命令面板高度按实际候选数自适应收缩（2 行顶部 + 2 行内容 + 2 行底边 = 6 行）"
    );
}

#[test]
fn model_panel_renders_top_rule_and_borderless_footer() {
    let area = Rect::new(0, 0, 100, 24);
    let lx = model_panel_lx(Some(0));
    let panel = slash_layout(area, &lx).expect("model layout");
    let mut buf = Buffer::empty(area);
    let _ = render(area, &mut buf, &lx, false, None, &mut PaneCache::default());
    let lines = buffer_lines(&buf, area);
    let all = lines.join("\n");
    assert!(
        all.contains(crate::ui::text::LX_MODEL_PANEL_TITLE),
        "顶栏显示 Model 标题: {all}"
    );
    let footer_row = usize::from(panel.rect.bottom() - 1);
    assert!(
        lines[footer_row].contains(crate::ui::text::LX_SLASH_PANEL_FOOTER),
        "底边显示快捷键提示: {:?}",
        lines[footer_row]
    );
    assert_eq!(
        panel.rect.height,
        3 + mention::PANEL_BORDER_ROWS + mention::PANEL_FOOTER_ROWS,
        "命令面板高度按实际候选数自适应收缩（2 行顶部 + 3 行内容 + 2 行底边 = 7 行）"
    );
}

#[test]
fn mention_panel_tabs_render_and_hit_test_in_lx() {
    let area = Rect::new(0, 0, 80, 24);
    let mut lx = mention_lx(vec![MentionEntry {
        path: "src/main.rs".into(),
        is_directory: false,
    }]);
    lx.sync_referenced_folders(&[crate::app::referenced::ReferencedFolder {
        id: 1,
        path: std::path::PathBuf::from("/Users/yonah/projects/codex-main"),
    }]);
    lx.set_mention_skills(vec![crate::skills::SkillEntry {
        name: "release-checklist".into(),
        description: "Release steps".into(),
        path: std::path::PathBuf::from("/skills/release-checklist/SKILL.md"),
        scope: crate::skills::SkillScope::User,
        source: crate::skills::SkillSource::Lx,
        enabled: true,
        shadowed_by: None,
    }]);
    lx.set_mention_mcp_servers(vec![crate::app::prompt::McpMentionEntry::new(
        "codegraph",
        "stdio · codegraph serve --mcp",
    )]);
    let panel = mention::layout(area, &lx).expect("mention layout");
    assert_eq!(
        panel.tabs.len(),
        4,
        "顶栏包含 Files / refDir / Skills / MCP 四个 Tab"
    );
    for idx in 0..4 {
        assert_eq!(
            mention_tab_at(area, &lx, panel.tabs[idx].x, panel.tabs[idx].y),
            Some(idx)
        );
    }

    // 切到 refDir 后渲染出引用文件夹条目，且顶边为蓝色横线、无边框。
    assert!(lx.mention_switch_tab(1));
    let panel = mention::layout(area, &lx).expect("refDir layout");
    let mut buf = Buffer::empty(area);
    let _ = render(area, &mut buf, &lx, true, None, &mut PaneCache::default());
    let all = buffer_lines(&buf, area).join("\n");
    assert!(all.contains("codex-main/"), "refDir 条目可见: {all}");
    assert_eq!(
        buf[(panel.rect.x, panel.rect.y)].symbol(),
        "─",
        "提及面板第 0 行为蓝色横线"
    );
    assert_eq!(
        buf[(panel.rect.x, panel.rect.y)].fg,
        ratatui::style::Color::Cyan,
        "横线为强调色"
    );

    // 切到 Skills Tab 渲染 skill 条目。
    assert!(lx.mention_switch_tab(1));
    let mut buf = Buffer::empty(area);
    let _ = render(area, &mut buf, &lx, true, None, &mut PaneCache::default());
    let all = buffer_lines(&buf, area).join("\n");
    assert!(all.contains("release-checklist"), "Skills 条目可见: {all}");
    assert!(
        all.contains("Skill · Release steps"),
        "Skills 描述可见: {all}"
    );

    // 切到 MCP Tab 渲染 MCP server 条目。
    assert!(lx.mention_switch_tab(1));
    let mut buf = Buffer::empty(area);
    let _ = render(area, &mut buf, &lx, true, None, &mut PaneCache::default());
    let all = buffer_lines(&buf, area).join("\n");
    assert!(all.contains("codegraph"), "MCP 条目可见: {all}");
    assert!(
        all.contains("MCP · stdio · codegraph serve --mcp"),
        "MCP 描述可见: {all}"
    );
}

#[test]
fn rename_input_renders_muted_argument_hint_until_user_types_argument() {
    let area = Rect::new(0, 0, 80, 24);
    let mut lx = Lx::new();
    lx.input = "/rename ".into();
    lx.cursor = lx.input.len();

    let mut buf = Buffer::empty(area);
    let cursor = render(area, &mut buf, &lx, true, None, &mut PaneCache::default());
    assert!(cursor.is_some(), "聚焦时返回光标坐标");

    let lines = buffer_lines(&buf, area);
    let input_row = (0..area.height)
        .find(|y| lines[usize::from(*y)].contains("/rename"))
        .expect("input row");
    assert!(
        lines[usize::from(input_row)].contains("/rename  [name]"),
        "输入框在 `/rename ` 后显示 `[name]` 参数说明: {:?}",
        lines[usize::from(input_row)]
    );
    let bracket_x = (0..area.width)
        .find(|x| buf[(*x, input_row)].symbol() == "[")
        .expect("`[name]` 起始列");
    assert_eq!(
        buf[(bracket_x, input_row)].fg,
        crate::ui::style::muted().fg.unwrap_or_default(),
        "`[name]` 必须为灰色 muted 字体"
    );

    // 用户输入参数字符后，`[name]` 立即消失。
    lx.input = "/rename demo".into();
    lx.cursor = lx.input.len();
    let mut buf2 = Buffer::empty(area);
    let _ = render(area, &mut buf2, &lx, true, None, &mut PaneCache::default());
    let lines2 = buffer_lines(&buf2, area);
    assert!(
        !lines2[usize::from(input_row)].contains("[name]"),
        "输入参数后不再显示 `[name]`: {:?}",
        lines2[usize::from(input_row)]
    );
}

#[test]
fn model_panel_refilter_renders_first_matching_item_selected() {
    let area = Rect::new(0, 0, 80, 24);
    let mut lx = model_panel_lx(None);
    if let Some(SlashPanel::Model(picker)) = lx.slash.as_mut() {
        picker.window.active = 2;
        picker.window.anchor = 2;
        picker.refilter("model");
        assert_eq!(picker.window.active, 0, "过滤后 active 必须重置为 0");
    }
    let panel = slash_layout(area, &lx).expect("slash layout");
    let mut buf = Buffer::empty(area);
    let _ = render(area, &mut buf, &lx, false, None, &mut PaneCache::default());
    assert_eq!(
        buf[(panel.content.x, panel.content.y)].symbol(),
        text::CURRENT_ITEM_MARKER,
        "首项带 ❯ 标记"
    );
}

#[test]
fn history_panel_refilter_renders_first_matching_item_selected() {
    let area = Rect::new(0, 0, 100, 24);
    let mut lx = history_panel_lx();
    if let Some(SlashPanel::History(history)) = lx.slash.as_mut() {
        history.window.active = 1;
        history.refilter("Alpha");
        assert_eq!(history.window.active, 0, "过滤后 active 必须重置为 0");
    }
    let panel = slash_layout(area, &lx).expect("slash layout");
    let mut buf = Buffer::empty(area);
    let _ = render(area, &mut buf, &lx, false, None, &mut PaneCache::default());
    assert_eq!(
        buf[(panel.content.x, panel.content.y)].symbol(),
        text::CURRENT_ITEM_MARKER,
        "首项 Alpha 带 ❯ 标记"
    );
}

#[test]
fn lx_md_block_panel_renders_square_borders_and_selection_highlight() {
    let area = Rect::new(0, 0, 80, 24);
    let mut lx = Lx::new();
    lx.resize(80, 24);
    lx.insert_char('#');
    assert!(lx.panel.is_some(), "输入 # 触发 md 格式面板");
    let lay = layout(area, &lx);
    let rect = lay.panel.expect("md panel rect");
    let mut buf = Buffer::empty(area);
    let _ = render(area, &mut buf, &lx, true, None, &mut PaneCache::default());
    assert_eq!(buf[(rect.x, rect.y)].symbol(), "┌", "md 面板左上角为直角");
    assert_eq!(
        buf[(rect.right() - 1, rect.y)].symbol(),
        "┐",
        "md 面板右上角为直角"
    );
    assert_eq!(
        buf[(rect.x, rect.bottom() - 1)].symbol(),
        "└",
        "md 面板左下角为直角"
    );
    assert_eq!(
        buf[(rect.right() - 1, rect.bottom() - 1)].symbol(),
        "┘",
        "md 面板右下角为直角"
    );
    assert_eq!(buf[(rect.x, rect.y)].bg, ratatui::style::Color::Reset);
    assert_eq!(
        buf[(rect.x + 2, rect.y + 3)].bg,
        ratatui::style::Color::Indexed(240),
        "选中项铺高亮底色"
    );
    assert_eq!(
        buf[(rect.x + 2, rect.y + 4)].bg,
        ratatui::style::Color::Reset,
        "非选中项保持透明底色"
    );
    assert!(
        super::super::command_panel_contains(area, &lx, rect.x + 2, rect.y + 1),
        "md 格式面板矩形纳入 command_panel_contains 防穿透保护"
    );

    // 窄宽度下绘制 md 面板：长底栏与预览不得写穿右边框 `│`。
    let narrow = Rect::new(0, 0, 26, 20);
    let mut narrow_lx = Lx::new();
    narrow_lx.resize(26, 20);
    narrow_lx.insert_char('#');
    let narrow_rect = layout(narrow, &narrow_lx).panel.expect("narrow md panel");
    let mut narrow_buf = Buffer::empty(narrow);
    let _ = render(
        narrow,
        &mut narrow_buf,
        &narrow_lx,
        true,
        None,
        &mut PaneCache::default(),
    );
    for y in narrow_rect.y + 1..narrow_rect.bottom() - 1 {
        assert_eq!(
            narrow_buf[(narrow_rect.right() - 1, y)].symbol(),
            "│",
            "第 {y} 行右边框不得被内部文字覆盖"
        );
    }
}

#[test]
fn lx_slash_and_mention_panels_adapt_height_up_to_max() {
    let area = Rect::new(0, 0, 80, 24);
    let frame_rows = mention::PANEL_BORDER_ROWS + mention::PANEL_FOOTER_ROWS;

    // 1. `/` 命令列表（8 个内置命令）与空态 `/resume`（1 行占位）按条目数自适应高度。
    let slash_lx = slash_commands_lx(0);
    let slash_panel = slash_layout(area, &slash_lx).expect("slash layout");
    assert_eq!(slash_panel.rect.height, 8 + frame_rows);

    let empty_resume = slash_resume_empty_lx();
    let resume_panel = slash_layout(area, &empty_resume).expect("empty resume layout");
    assert_eq!(resume_panel.rect.height, 1 + frame_rows);

    // 2. `/model` 过滤前后（3 个候选 vs 1 个候选）高度按候选数自适应收缩，底边始终紧贴输入框。
    let mut model_lx = model_panel_lx(Some(0));
    let before = slash_layout(area, &model_lx).expect("before filter");
    assert_eq!(before.rect.height, 3 + frame_rows);
    if let Some(SlashPanel::Model(picker)) = model_lx.slash.as_mut() {
        picker.refilter("model-2");
    }
    let single = slash_layout(area, &model_lx).expect("single match");
    assert_eq!(single.rect.height, 1 + frame_rows);
    assert_eq!(before.rect.bottom(), single.rect.bottom());
}
