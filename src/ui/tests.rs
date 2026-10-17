//! 单元测试；仅测试构建编译。

use super::*;
use crate::app::actions::Action;
use crate::app::overlay::{
    ConfirmClose, Overlay, OverlayTarget, TextInput, WorktreeOpen, WorktreeOpenEntry,
};
use crate::app::prompt::PromptMode;
use crate::app::state::{LxEntry, PaneKind, PaneView, Workspace, WorkspaceGit};
use crate::app::toast::{TOAST_DURATION, Toast, ToastKind};

use crate::app::tooltip::{TOOLTIP_DELAY, Tooltip, TooltipTarget};
use crate::app::update;
use ratatui::Terminal as RatatuiTerminal;
use ratatui::backend::TestBackend;
use std::path::{Path, PathBuf};
use std::time::Instant;

fn render_lines(state: &AppState) -> Vec<String> {
    let config = Config::default();
    let mut terminal =
        RatatuiTerminal::new(TestBackend::new(100, 24)).expect("test backend is infallible");
    if let Err(error) = terminal.draw(|frame| render(frame, state, &config)) {
        panic!("draw failed: {error}");
    }
    let buffer = terminal.backend().buffer().clone();
    (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect()
        })
        .collect()
}

/// 渲染一帧并返回缓冲；单元格样式断言使用。
fn render_buffer(state: &AppState) -> ratatui::buffer::Buffer {
    let config = Config::default();
    let mut terminal =
        RatatuiTerminal::new(TestBackend::new(100, 24)).expect("test backend is infallible");
    if let Err(error) = terminal.draw(|frame| render(frame, state, &config)) {
        panic!("draw failed: {error}");
    }
    terminal.backend().buffer().clone()
}

/// prompt 面板内的行文本（按面板列范围裁剪）。
fn prompt_line(lines: &[String], view: &layout::ViewLayout, y: u16) -> String {
    lines[y as usize]
        .chars()
        .skip(view.prompt.x as usize)
        .take(view.prompt.width as usize)
        .collect()
}

fn view_for(state: &AppState) -> layout::ViewLayout {
    layout::compute(
        Rect::new(0, 0, 100, 24),
        &Config::default(),
        state.sidebar_collapsed,
        state.sidebar_width,
        state.prompt_collapsed,
        state.prompt_width,
        state.prompt_left,
        state.panes_collapsed,
    )
}

/// 把活动窗格置为终端视图：终端标题与网格断言的前置。
fn terminal_view(state: &mut AppState) {
    let focus = state.active_tab().layout.focus();
    if let Some(pane) = state.active_tab_mut().pane_mut(focus) {
        pane.view = PaneView::Terminal;
    }
}

fn button_for(view: &layout::ViewLayout, target: CollapseTarget) -> CollapseButton {
    button_for_state(view, false, target)
}

fn button_for_state(
    view: &layout::ViewLayout,
    agents_collapsed: bool,
    target: CollapseTarget,
) -> CollapseButton {
    collapse_buttons(view, agents_collapsed)
        .into_iter()
        .find(|button| button.target == target)
        .expect("panel button is visible")
}

fn rendered_label(lines: &[String], button: &CollapseButton) -> String {
    let row: Vec<char> = lines[button.area.y as usize].chars().collect();
    (button.area.x..button.area.right())
        .map(|x| row[x as usize])
        .collect()
}

fn rendered_collapsed_label(
    lines: &[String],
    view: &layout::ViewLayout,
    target: CollapseTarget,
) -> String {
    let button = button_for(view, target);
    assert!(button.collapsed);
    let separator_right = target == CollapseTarget::Sidebar;
    let start = collapsed_content_x(button.area, separator_right);
    let end = if separator_right {
        button.area.right() - 1
    } else {
        button.area.right()
    };
    let row: Vec<char> = lines[button.area.y as usize].chars().collect();
    (start..end).map(|x| row[x as usize]).collect()
}

#[test]
fn collapse_buttons_render_directional_arrow_labels() {
    let mut state = AppState::demo();
    let view = view_for(&state);
    let lines = render_lines(&state);
    assert_eq!(
        rendered_label(&lines, &button_for(&view, CollapseTarget::Sidebar)),
        text::SIDEBAR_COLLAPSE_LABEL
    );
    assert_eq!(
        rendered_label(&lines, &button_for(&view, CollapseTarget::Prompt)),
        text::RIGHT_COLLAPSE_LABEL
    );

    update::apply(Action::ToggleSidebar, &mut state);
    let view = view_for(&state);
    let lines = render_lines(&state);
    assert_eq!(
        rendered_collapsed_label(&lines, &view, CollapseTarget::Sidebar),
        text::SIDEBAR_EXPAND_LABEL
    );
    assert_eq!(
        rendered_label(&lines, &button_for(&view, CollapseTarget::Prompt)),
        text::RIGHT_COLLAPSE_LABEL
    );

    update::apply(Action::TogglePrompt, &mut state);
    let view = view_for(&state);
    let lines = render_lines(&state);
    assert_eq!(
        rendered_collapsed_label(&lines, &view, CollapseTarget::Sidebar),
        text::SIDEBAR_EXPAND_LABEL
    );
    assert_eq!(
        rendered_collapsed_label(&lines, &view, CollapseTarget::Prompt),
        text::RIGHT_EXPAND_LABEL
    );
}

#[test]
fn prompt_sidebar_renders_content_and_collapse_button() {
    let mut state = AppState::demo();
    state.prompt.resize(28, 20);
    state.prompt.insert_str("## markdown");
    let lines = render_lines(&state);
    assert!(lines.iter().any(|line| line.contains("markdown")));
    assert!(lines.iter().any(|line| line.contains(text::PROMPT_TITLE)));
    assert!(
        lines
            .iter()
            .any(|line| line.contains(text::SIDEBAR_COLLAPSE_LABEL))
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains(text::RIGHT_COLLAPSE_LABEL))
    );
}

#[test]
fn collapsed_prompt_renders_strip_and_expand_icon() {
    let mut state = AppState::demo();
    state.prompt.insert_str("hidden content");
    update::apply(Action::TogglePrompt, &mut state);
    let view = view_for(&state);
    let lines = render_lines(&state);
    assert_eq!(
        rendered_collapsed_label(&lines, &view, CollapseTarget::Prompt),
        text::RIGHT_EXPAND_LABEL
    );
    assert!(
        !lines
            .iter()
            .any(|line| line.contains(text::RIGHT_COLLAPSE_LABEL))
    );
    assert!(!lines.iter().any(|line| line.contains("hidden content")));
}

#[test]
fn sidebar_collapse_button_renders_icons() {
    let mut state = AppState::demo();
    let lines = render_lines(&state);
    assert!(
        lines
            .iter()
            .any(|line| line.contains(text::SIDEBAR_COLLAPSE_LABEL))
    );
    update::apply(Action::ToggleSidebar, &mut state);
    let view = view_for(&state);
    let lines = render_lines(&state);
    assert_eq!(
        rendered_collapsed_label(&lines, &view, CollapseTarget::Sidebar),
        text::SIDEBAR_EXPAND_LABEL
    );
    assert!(
        !lines
            .iter()
            .any(|line| line.contains(text::SIDEBAR_COLLAPSE_LABEL))
    );
}

#[test]
fn sidebar_lists_single_workspace_named_after_current_directory() {
    let state = AppState::demo();
    let lines = render_lines(&state);
    // 侧栏宽度有限，长目录名会被截断；断言至少展示名字前缀。
    let prefix: String = state.active_workspace().name.chars().take(8).collect();
    assert!(lines.iter().any(|line| line.contains(&prefix)));
    assert!(!lines.iter().any(|line| line.contains("notes")));
}

#[test]
fn exit_button_sits_at_tab_bar_right_end() {
    let state = AppState::demo();
    let view = view_for(&state);
    let area = exit_button(&view).expect("exit button is visible");
    assert_eq!(area.y, view.tab_bar.y);
    assert_eq!(area.right(), view.tab_bar.right());
    assert!(exit_button_at(&view, area.x, area.y));
    assert!(!exit_button_at(&view, area.x.saturating_sub(1), area.y));

    let lines = render_lines(&state);
    let row: Vec<char> = lines[area.y as usize].chars().collect();
    let rendered: String = row[area.x as usize..area.right() as usize].iter().collect();
    assert_eq!(rendered, text::EXIT_LABEL);
}

#[test]
fn exit_button_aligns_with_collapsed_prompt_button() {
    let mut state = AppState::demo();
    update::apply(Action::TogglePrompt, &mut state);
    let view = view_for(&state);
    let exit = exit_button(&view).expect("exit button is visible");
    let prompt_button = button_for_state(&view, false, CollapseTarget::Prompt);
    assert!(prompt_button.collapsed);
    assert_eq!(exit.right(), prompt_button.area.right());
    assert_eq!(exit.width, text::EXIT_LABEL.chars().count() as u16);
}

#[test]
fn agents_header_is_left_aligned_without_junctions() {
    let state = AppState::demo();
    let view = view_for(&state);
    let lines = render_lines(&state);
    let sections = layout::sidebar_sections(view.sidebar, false).expect("sections are visible");
    let row: Vec<char> = lines[sections.divider.y as usize].chars().collect();
    let divider: String = (sections.divider.x..sections.divider.right())
        .map(|x| row[x as usize])
        .collect();
    assert_eq!(text::SIDEBAR_AGENTS_TITLE, "Agents");
    assert!(!divider.contains('├') && !divider.contains('┤'));

    let label = format!(" {} ", text::SIDEBAR_AGENTS_TITLE);
    let start = sections.divider.x as usize;
    let rendered: String = row[start..start + label.chars().count()].iter().collect();
    assert_eq!(rendered, label);

    // 标题与其他边框标题同色（淡蓝色）。
    let buffer = render_buffer(&state);
    let cell = &buffer[(sections.divider.x + 1, sections.divider.y)];
    assert_eq!(cell.fg, ratatui::style::Color::Cyan);
    assert!(cell.modifier.contains(ratatui::style::Modifier::DIM));

    let top: Vec<char> = lines[view.sidebar.y as usize].chars().collect();
    let workspaces_start = top
        .iter()
        .position(|symbol| *symbol == 'W')
        .expect("workspaces title is rendered");
    let agents_start = row
        .iter()
        .position(|symbol| *symbol == 'A')
        .expect("agents title is rendered");
    assert_eq!(agents_start, workspaces_start);
}

#[test]
fn agents_section_below_header_shows_empty_hint_only() {
    let state = AppState::demo();
    let view = view_for(&state);
    let sections = layout::sidebar_sections(view.sidebar, false).expect("sections are visible");
    let lines = render_lines(&state);
    for (index, line) in lines[sections.agents.y as usize..sections.agents.bottom() as usize]
        .iter()
        .enumerate()
    {
        let row: String = line
            .chars()
            .skip(view.sidebar.x as usize)
            .take(view.sidebar.width as usize)
            .collect();
        let content = row.trim_matches(|symbol| symbol == '│' || symbol == ' ');
        if index == 0 {
            assert_eq!(content, text::AGENTS_EMPTY, "empty state renders hint");
        } else {
            assert!(
                content.is_empty(),
                "agents section keeps remaining rows empty: {row:?}"
            );
        }
    }
}

#[test]
fn agents_button_aligns_with_panel_button_and_toggles_header() {
    let mut state = AppState::demo();
    let view = view_for(&state);
    let sidebar_button = button_for(&view, CollapseTarget::Sidebar);
    let agents_button = button_for(&view, CollapseTarget::Agents);
    assert_eq!(agents_button.area.x, sidebar_button.area.x);
    assert_eq!(agents_button.area.width, sidebar_button.area.width);
    assert!(!agents_button.collapsed);
    assert_eq!(
        collapse_button_at(&view, false, agents_button.area.x, agents_button.area.y),
        Some(CollapseTarget::Agents)
    );

    update::apply(Action::ToggleAgents, &mut state);
    let view = view_for(&state);
    let agents_button = button_for_state(&view, true, CollapseTarget::Agents);
    assert!(agents_button.collapsed);
    assert_eq!(agents_button.area.x, sidebar_button.area.x);
    assert_eq!(agents_button.area.y, view.sidebar.bottom() - 2);
    let lines = render_lines(&state);
    assert!(lines[agents_button.area.y as usize].contains(text::AGENTS_EXPAND_LABEL));
    assert!(
        !lines
            .iter()
            .any(|line| line.contains(text::AGENTS_COLLAPSE_LABEL))
    );

    update::apply(Action::ToggleAgents, &mut state);
    let view = view_for(&state);
    let agents_button = button_for_state(&view, false, CollapseTarget::Agents);
    let lines = render_lines(&state);
    assert!(lines[agents_button.area.y as usize].contains(text::AGENTS_COLLAPSE_LABEL));
}

#[test]
fn collapsed_agents_header_docks_above_bottom_border() {
    let mut state = AppState::demo();
    update::apply(Action::ToggleAgents, &mut state);
    let view = view_for(&state);
    let sections = layout::sidebar_sections(view.sidebar, true).expect("sections are visible");
    assert_eq!(sections.agents.height, 0);
    assert_eq!(sections.divider.y, view.sidebar.bottom() - 2);
    let lines = render_lines(&state);
    assert!(lines[sections.divider.y as usize].contains(text::SIDEBAR_AGENTS_TITLE));
}

#[test]
fn collapsed_labels_fill_content_area_with_continuous_separator() {
    let mut state = AppState::demo();
    update::apply(Action::ToggleSidebar, &mut state);
    update::apply(Action::TogglePrompt, &mut state);
    let view = view_for(&state);
    let lines = render_lines(&state);

    let sidebar_row: Vec<char> = lines[view.sidebar.bottom() as usize - 1].chars().collect();
    let sidebar_start = collapsed_content_x(view.sidebar, true);
    assert_eq!(sidebar_start, view.sidebar.x);
    let sidebar_label: String = (sidebar_start..view.sidebar.right() - 1)
        .map(|x| sidebar_row[x as usize])
        .collect();
    assert_eq!(sidebar_label, text::SIDEBAR_EXPAND_LABEL);
    assert_eq!(
        sidebar_row[(view.sidebar.right() - 1) as usize].to_string(),
        text::STRIP_LINE
    );

    let prompt_row: Vec<char> = lines[view.prompt.bottom() as usize - 1].chars().collect();
    let prompt_start = collapsed_content_x(view.prompt, false);
    assert_eq!(prompt_start, view.prompt.x + 1);
    let prompt_label: String = (prompt_start..view.prompt.right())
        .map(|x| prompt_row[x as usize])
        .collect();
    assert_eq!(prompt_label, text::RIGHT_EXPAND_LABEL);
    assert_eq!(
        prompt_row[view.prompt.x as usize].to_string(),
        text::STRIP_LINE
    );
}

#[test]
fn collapsed_strips_hide_panel_content() {
    let mut state = AppState::demo();
    update::apply(Action::ToggleSidebar, &mut state);
    update::apply(Action::TogglePrompt, &mut state);
    let view = view_for(&state);
    let lines = render_lines(&state);

    let strip = |y: usize| -> String {
        lines[y]
            .chars()
            .skip(view.sidebar.x as usize)
            .take(view.sidebar.width as usize)
            .collect()
    };
    assert_eq!(strip(1), "   │");
    assert_eq!(strip(10), "   │");

    let prompt = |y: usize| -> String {
        lines[y]
            .chars()
            .skip(view.prompt.x as usize)
            .take(view.prompt.width as usize)
            .collect()
    };
    // 折叠按钮贴窄条底行，其余行是纯窄条。
    assert_eq!(prompt(view.prompt.y as usize), "│   ");
    assert_eq!(prompt(10), "│   ");
}

#[test]
fn collapse_buttons_hit_their_targets() {
    let mut state = AppState::demo();
    let view = view_for(&state);
    let sidebar = collapse_buttons(&view, false)
        .into_iter()
        .find(|button| button.target == CollapseTarget::Sidebar);
    assert!(sidebar.is_some());
    let Some(sidebar) = sidebar else { return };
    assert!(!sidebar.collapsed);
    assert_eq!(
        collapse_button_at(&view, false, sidebar.area.x, sidebar.area.y),
        Some(CollapseTarget::Sidebar)
    );
    let prompt = collapse_buttons(&view, false)
        .into_iter()
        .find(|button| button.target == CollapseTarget::Prompt);
    assert!(prompt.is_some());
    let Some(prompt) = prompt else { return };
    assert_eq!(
        collapse_button_at(&view, false, prompt.area.x, prompt.area.y),
        Some(CollapseTarget::Prompt)
    );

    update::apply(Action::TogglePrompt, &mut state);
    let view = view_for(&state);
    let collapsed = collapse_buttons(&view, false)
        .into_iter()
        .find(|button| button.target == CollapseTarget::Prompt);
    assert!(collapsed.is_some_and(|button| button.collapsed));
}

#[test]
fn collapse_button_cells_use_pure_accent_style() {
    let state = AppState::demo();
    let view = view_for(&state);
    let mut terminal =
        RatatuiTerminal::new(TestBackend::new(100, 24)).expect("test backend is infallible");
    if let Err(error) = terminal.draw(|frame| render(frame, &state, &Config::default())) {
        panic!("draw failed: {error}");
    }
    let buffer = terminal.backend().buffer().clone();
    for button in collapse_buttons(&view, false) {
        for x in button.area.x..button.area.right() {
            let cell = &buffer[(x, button.area.y)];
            assert_eq!(cell.fg, ratatui::style::Color::Cyan);
            assert!(cell.modifier.contains(ratatui::style::Modifier::BOLD));
            assert!(!cell.modifier.contains(ratatui::style::Modifier::DIM));
        }
    }
}

#[test]
fn resize_hint_highlights_divider_on_hover() {
    let mut state = AppState::demo();
    let config = Config::default();
    let view = view_for(&state);
    let mut terminal =
        RatatuiTerminal::new(TestBackend::new(100, 24)).expect("test backend is infallible");

    if let Err(error) = terminal.draw(|frame| render(frame, &state, &config)) {
        panic!("draw failed: {error}");
    }
    let plain = terminal.backend().buffer().clone();
    let left = &plain[(view.prompt.x, 10)];
    assert_ne!(left.fg, ratatui::style::Color::Cyan);

    state.prompt_hover = true;
    if let Err(error) = terminal.draw(|frame| render(frame, &state, &config)) {
        panic!("draw failed: {error}");
    }
    let hovered = terminal.backend().buffer().clone();
    for column in [view.prompt.x - 1, view.prompt.x] {
        let cell = &hovered[(column, 10)];
        assert_eq!(cell.symbol(), text::STRIP_LINE);
        assert_eq!(cell.fg, ratatui::style::Color::Cyan);
    }
}

#[test]
fn placeholder_pane_renders_title_without_content() {
    let mut state = AppState::demo();
    terminal_view(&mut state);
    let focus = state.active_tab().layout.focus();
    if let Some(pane) = state.active_tab_mut().pane_mut(focus) {
        pane.kind = PaneKind::Placeholder;
    }
    let expected = format!("pane {}", focus.raw());
    let lines = render_lines(&state);
    assert!(lines.iter().any(|line| line.contains(&expected)));
    assert!(!lines.iter().any(|line| line.contains("exited")));
}

#[test]
fn status_bar_is_removed() {
    let state = AppState::demo();
    let lines = render_lines(&state);
    let last = lines.last().map(String::as_str).unwrap_or_default();
    // 末行为面板底边框（含 `╰`），不再有独立状态栏；`ws:` 路径名合法落在 prompt 底边框。
    assert!(last.contains('╰'), "last row should be pane border");
}

#[test]
fn toast_renders_title_and_message() {
    let mut state = AppState::demo();
    let anchor = state.prompt.id();
    update::show_toast(
        &mut state,
        Toast::new(
            ToastKind::Info,
            text::TOAST_COPIED,
            Some(anchor),
            Instant::now(),
        )
        .with_title("Clipboard"),
    );
    let lines = render_lines(&state);
    let title_line = lines
        .iter()
        .find(|line| line.contains("Clipboard"))
        .expect("toast title is visible");
    assert!(title_line.contains('╭'), "title must sit on the top border");
    assert!(lines.iter().any(|line| line.contains(text::TOAST_COPIED)));
}

#[test]
fn expired_toast_is_not_rendered() {
    let mut state = AppState::demo();
    update::show_toast(
        &mut state,
        Toast::new(ToastKind::Info, text::TOAST_COPIED, None, Instant::now()),
    );
    update::tick(&mut state, Instant::now() + TOAST_DURATION);
    let lines = render_lines(&state);
    assert!(!lines.iter().any(|line| line.contains(text::TOAST_COPIED)));
}

/// 渲染一帧并返回硬件光标状态：可见性与位置（TestBackend 记录 Hide/Show 与 MoveTo）。
fn render_cursor(state: &AppState) -> (bool, (u16, u16)) {
    let config = Config::default();
    let mut terminal =
        RatatuiTerminal::new(TestBackend::new(100, 24)).expect("test backend is infallible");
    if let Err(error) = terminal.draw(|frame| render(frame, state, &config)) {
        panic!("draw failed: {error}");
    }
    let backend = terminal.backend();
    let position = backend.cursor_position();
    (backend.cursor_visible(), (position.x, position.y))
}

/// prompt 聚焦且处于树视图：搜索框持有硬件光标。
fn focused_tree_state() -> AppState {
    let mut state = AppState::demo();
    state.prompt_focused = true;
    state.workspaces[0].prompt_panel.mode = PromptMode::Tree;
    state
}

#[test]
fn tooltip_over_prompt_search_hides_hardware_cursor() {
    let mut state = focused_tree_state();
    let (visible, _) = render_cursor(&state);
    assert!(visible, "无浮层时搜索框硬件光标可见");

    let now = Instant::now();
    state.tooltip = Some(Tooltip::new(TooltipTarget::PromptMode, now));
    assert!(update::tick(&mut state, now + TOOLTIP_DELAY));
    let (visible, _) = render_cursor(&state);
    assert!(!visible, "tooltip 覆盖搜索框时硬件光标隐藏");
}

#[test]
fn toast_over_prompt_search_hides_hardware_cursor() {
    let mut state = focused_tree_state();
    let view = view_for(&state);
    let message = "x".repeat(usize::from(view.prompt.width.saturating_sub(6)));
    let anchor = state.prompt.id();
    update::show_toast(
        &mut state,
        Toast::new(ToastKind::Info, message, Some(anchor), Instant::now()),
    );
    let (visible, _) = render_cursor(&state);
    assert!(!visible, "toast 覆盖搜索框时硬件光标隐藏");
}

#[test]
fn modal_without_input_hides_prompt_hardware_cursor() {
    let mut state = focused_tree_state();
    state.overlay = Some(Overlay::ConfirmClose(ConfirmClose {
        target: OverlayTarget::Workspace(0),
    }));
    let (visible, _) = render_cursor(&state);
    assert!(!visible, "模态接管后 prompt 搜索框硬件光标隐藏");
}

#[test]
fn error_toast_uses_red_border() {
    let mut state = AppState::demo();
    let anchor = state.prompt.id();
    update::show_toast(
        &mut state,
        Toast::new(
            ToastKind::Error,
            text::TOAST_COPY_FAILED,
            Some(anchor),
            Instant::now(),
        ),
    );
    let config = Config::default();
    let view = view_for(&state);
    let pane_rects = crate::layout::pane_rects(
        &state.active_tab().layout,
        view.panes,
        config.min_pane_width,
        config.min_pane_height,
    );
    let area = toast::rect(
        &state,
        &view,
        &pane_rects,
        Rect::new(0, 0, 100, 24),
        &config,
    )
    .expect("toast area is resolvable");
    let mut terminal =
        RatatuiTerminal::new(TestBackend::new(100, 24)).expect("test backend is infallible");
    if let Err(error) = terminal.draw(|frame| render(frame, &state, &config)) {
        panic!("draw failed: {error}");
    }
    let buffer = terminal.backend().buffer().clone();
    assert_eq!(buffer[(area.x, area.y)].fg, ratatui::style::Color::Red);
}

#[test]
fn add_workspace_button_sits_at_sidebar_top_border_right() {
    let state = AppState::demo();
    let view = view_for(&state);
    let button = add_workspace_button(&view).expect("button is visible");
    // 原折叠按钮位置：顶边框右端，距右边框 1 列。
    assert_eq!(button.y, view.sidebar.y);
    assert_eq!(button.right(), view.sidebar.right() - 1);
    assert_eq!(
        button.width,
        text::ADD_WORKSPACE_LABEL.chars().count() as u16
    );
    assert!(button.x > view.sidebar.x);

    let lines = render_lines(&state);
    let row: Vec<char> = lines[button.y as usize].chars().collect();
    let rendered: String = (button.x..button.right())
        .map(|x| row[x as usize])
        .collect();
    assert_eq!(rendered, text::ADD_WORKSPACE_LABEL);
}

#[test]
fn workspace_items_hit_full_list_rows() {
    let mut state = AppState::demo();
    update::create_workspace(&mut state);
    let view = view_for(&state);
    let sections = layout::sidebar_sections(view.sidebar, false).expect("sections are visible");
    let first_row = sections.workspaces.y;
    assert_eq!(
        workspace_item_at(&view, &state, sections.workspaces.x, first_row),
        Some(0)
    );
    assert_eq!(
        workspace_item_at(&view, &state, sections.workspaces.x, first_row + 1),
        Some(1)
    );
    assert_eq!(
        workspace_item_at(&view, &state, sections.workspaces.x, first_row + 2),
        None
    );
    assert_eq!(
        workspace_item_at(&view, &state, sections.workspaces.x, sections.divider.y),
        None
    );
    assert_eq!(
        workspace_item_at(&view, &state, sections.workspaces.x, sections.agents.y),
        None
    );
}

#[test]
fn workspace_list_blank_hits_rows_below_items_only() {
    let mut state = AppState::demo();
    update::create_workspace(&mut state);
    let view = view_for(&state);
    let sections = layout::sidebar_sections(view.sidebar, false).expect("sections are visible");
    let list = sections.workspaces;
    assert!(
        !workspace_list_blank_at(&view, &state, list.x, list.y),
        "项行不命中空白"
    );
    assert!(
        workspace_list_blank_at(&view, &state, list.x, list.y + 2),
        "末项下方空行命中空白"
    );
    assert!(!workspace_list_blank_at(
        &view,
        &state,
        list.x,
        sections.divider.y
    ));
    assert!(!workspace_list_blank_at(
        &view,
        &state,
        list.x,
        sections.agents.y
    ));
}

#[test]
fn workspace_list_blank_hidden_when_sidebar_collapsed() {
    let mut state = AppState::demo();
    state.sidebar_collapsed = true;
    let view = view_for(&state);
    assert!(!workspace_list_blank_at(&view, &state, 0, 1));
}

#[test]
fn workspace_list_last_row_is_hittable_without_footer() {
    let mut state = AppState::demo();
    for _ in 0..11 {
        update::create_workspace(&mut state);
    }
    let view = view_for(&state);
    let sections = layout::sidebar_sections(view.sidebar, false).expect("sections are visible");
    let last = sections.workspaces.bottom() - 1;
    let index = usize::from(last - sections.workspaces.y);
    assert_eq!(
        workspace_item_at(&view, &state, sections.workspaces.x, last),
        Some(index)
    );
}

#[test]
fn add_workspace_button_hidden_when_sidebar_collapsed() {
    let mut state = AppState::demo();
    state.sidebar_collapsed = true;
    let view = view_for(&state);
    assert_eq!(add_workspace_button(&view), None);
    assert_eq!(workspace_item_at(&view, &state, 0, 1), None);
}

#[test]
fn workspace_scrollbar_appears_only_on_overflow() {
    let mut state = AppState::demo();
    let view = view_for(&state);
    assert_eq!(workspace_scrollbar(&view, &state), None);

    for _ in 0..20 {
        update::create_workspace(&mut state);
    }
    let view = view_for(&state);
    let bar = workspace_scrollbar(&view, &state).expect("scrollbar is visible");
    let sections = layout::sidebar_sections(view.sidebar, false).expect("sections are visible");
    let list = sections.workspaces;
    assert_eq!(bar.track.x, list.right() - 1);
    assert_eq!(bar.track.y, list.y);
    assert_eq!(bar.track.height, list.height);
    assert_eq!(bar.total, 21);
    assert_eq!(bar.visible, usize::from(list.height));
}

#[test]
fn workspace_list_renders_scrolled_window_and_scrollbar() {
    let mut state = AppState::demo();
    for _ in 0..20 {
        update::create_workspace(&mut state);
    }
    // 短名断言：顶层缩进占 2 列，长名会被省略号截断。
    for (index, workspace) in state.workspaces.iter_mut().enumerate() {
        workspace.name = format!("w{index}");
    }
    let view = view_for(&state);
    let sections = layout::sidebar_sections(view.sidebar, false).expect("sections are visible");
    let list = sections.workspaces;
    let rows = usize::from(list.height);
    state.workspace_scroll = update::workspace_scroll_max(&state, rows);
    let expected = state.workspaces[state.workspace_scroll].name.clone();
    let lines = render_lines(&state);
    let top: Vec<char> = lines[list.y as usize].chars().collect();
    let rendered: String = (list.x..list.right().saturating_sub(1))
        .map(|x| top[x as usize])
        .collect();
    assert!(rendered.contains(&expected), "{rendered}");

    let scrollbar_row: Vec<char> = lines[list.y as usize].chars().collect();
    assert_eq!(
        scrollbar_row[usize::from(list.right() - 1)].to_string(),
        "▕"
    );
    let bottom_row: Vec<char> = lines[list.bottom() as usize - 1].chars().collect();
    assert_eq!(bottom_row[usize::from(list.right() - 1)].to_string(), "▐");
}

#[test]
fn workspace_item_at_follows_scroll_offset_and_excludes_scrollbar() {
    let mut state = AppState::demo();
    for _ in 0..20 {
        update::create_workspace(&mut state);
    }
    let view = view_for(&state);
    let sections = layout::sidebar_sections(view.sidebar, false).expect("sections are visible");
    let list = sections.workspaces;
    state.workspace_scroll = 5;
    assert_eq!(workspace_item_at(&view, &state, list.x, list.y), Some(5));
    assert_eq!(
        workspace_item_at(&view, &state, list.x, list.y + 1),
        Some(6)
    );
    assert_eq!(
        workspace_item_at(&view, &state, list.right() - 1, list.y),
        None
    );
}

#[test]
fn initial_workspace_marker_uses_accent_style() {
    let mut state = AppState::demo();
    update::create_workspace(&mut state);
    // 显式短名保证列表不截断，标记断言与工作目录名无关。
    state.workspaces[0].name = "alpha".to_string();
    state.workspaces[1].name = "beta".to_string();
    let initial = "alpha".to_string();
    let created = "beta".to_string();
    let lines = render_lines(&state);
    let marker_line = lines
        .iter()
        .position(|line| line.contains(&format!("{initial} *")))
        .expect("initial workspace shows marker");
    assert!(
        lines
            .iter()
            .any(|line| line.contains(&created) && !line.contains(&format!("{created} *")))
    );

    let config = Config::default();
    let mut terminal =
        RatatuiTerminal::new(TestBackend::new(100, 24)).expect("test backend is infallible");
    if let Err(error) = terminal.draw(|frame| render(frame, &state, &config)) {
        panic!("draw failed: {error}");
    }
    let buffer = terminal.backend().buffer().clone();
    let row: Vec<char> = lines[marker_line].chars().collect();
    let marker_x = row
        .iter()
        .position(|symbol| *symbol == '*')
        .expect("marker is rendered");
    // `*` 用强调色标识（与去重序号同色，不再跟随正文样式）。
    let cell = &buffer[(marker_x as u16, marker_line as u16)];
    assert_eq!(cell.fg, ratatui::style::Color::Cyan);
    assert!(cell.modifier.contains(ratatui::style::Modifier::BOLD));
    assert_eq!(cell.bg, ratatui::style::Color::Reset);
}

#[test]
fn long_initial_workspace_name_keeps_marker_visible() {
    let mut state = AppState::demo();
    state.workspaces[0].name = "a-very-long-workspace-name-that-would-be-clipped".to_string();
    let lines = render_lines(&state);
    assert!(
        lines
            .iter()
            .any(|line| line.contains('…') && line.contains(" *")),
        "marker should survive ellipsis"
    );
}

#[test]
fn sidebar_boundary_hits_adjacent_columns_in_pane_rows() {
    let state = AppState::demo();
    let view = view_for(&state);
    let left = view.sidebar.right() - 1;
    let right = view.sidebar.right();
    assert!(sidebar_boundary_at(&view, left, view.panes.y));
    assert!(sidebar_boundary_at(&view, right, view.panes.bottom() - 1));
    assert!(!sidebar_boundary_at(&view, left, view.panes.y - 1));
    assert!(!sidebar_boundary_at(&view, left - 1, view.panes.y));

    let mut collapsed = AppState::demo();
    collapsed.sidebar_collapsed = true;
    let view = view_for(&collapsed);
    assert!(!sidebar_boundary_at(
        &view,
        view.sidebar.right() - 1,
        view.panes.y
    ));
}

#[test]
fn sidebar_boundary_hit_uses_runtime_width() {
    let mut state = AppState::demo();
    state.sidebar_width = 32;
    let view = view_for(&state);
    assert_eq!(view.sidebar.width, 32);
    assert!(sidebar_boundary_at(&view, 31, view.panes.y));
    assert!(sidebar_boundary_at(&view, 32, view.panes.y));
    assert!(!sidebar_boundary_at(&view, 23, view.panes.y));
}

#[test]
fn workspace_drop_index_maps_rows_with_scroll_and_clamps() {
    let mut state = AppState::demo();
    update::create_workspace(&mut state);
    update::create_workspace(&mut state);
    let view = view_for(&state);
    let sections = layout::sidebar_sections(view.sidebar, false).expect("sections are visible");
    let list_y = sections.workspaces.y;
    assert_eq!(workspace_drop_index(&view, &state, list_y), Some(0));
    assert_eq!(workspace_drop_index(&view, &state, list_y + 2), Some(2));
    assert_eq!(workspace_drop_index(&view, &state, 0), Some(0));
    assert_eq!(
        workspace_drop_index(&view, &state, sections.workspaces.bottom()),
        Some(2)
    );

    state.workspace_scroll = 1;
    assert_eq!(workspace_drop_index(&view, &state, list_y), Some(1));

    state.workspaces.clear();
    assert_eq!(workspace_drop_index(&view, &state, list_y), None);
}

#[test]
fn workspace_rows_mark_selected_and_fill_hover_backgrounds() {
    let mut state = AppState::demo();
    update::create_workspace(&mut state);
    update::create_workspace(&mut state);
    state.workspace_hover = Some(1);

    let config = Config::default();
    let mut terminal =
        RatatuiTerminal::new(TestBackend::new(100, 24)).expect("test backend is infallible");
    if let Err(error) = terminal.draw(|frame| render(frame, &state, &config)) {
        panic!("draw failed: {error}");
    }
    let buffer = terminal.backend().buffer().clone();
    let view = view_for(&state);
    let sections = layout::sidebar_sections(view.sidebar, false).expect("sections are visible");
    let x = sections.workspaces.x;
    let y = sections.workspaces.y;
    let right = sections.workspaces.right() - 1;

    // 激活项（create_workspace 激活末项）行首 `❯` 标记，不铺背景。
    assert_eq!(buffer[(x, y + 2)].symbol(), text::AGENT_SELECTED_MARKER);
    assert_eq!(buffer[(x, y + 2)].bg, ratatui::style::Color::Reset);
    assert_eq!(
        buffer[(right, y + 2)].bg,
        ratatui::style::Color::Reset,
        "行尾空白无底色"
    );

    // 悬停项使用显式深色背景（selection）。
    assert!(
        !buffer[(x, y + 1)]
            .modifier
            .contains(ratatui::style::Modifier::REVERSED)
    );
    assert_eq!(buffer[(x, y + 1)].bg, ratatui::style::Color::Indexed(240));
    assert_eq!(buffer[(x, y + 1)].fg, ratatui::style::Color::Indexed(255));

    // 未交互项无背景。
    assert_eq!(buffer[(x, y)].bg, ratatui::style::Color::Reset);
}

#[test]
fn hovered_parent_workspace_item_uses_selection_background() {
    let mut state = AppState::demo();
    state.workspaces[0].name = "repo".to_string();
    state.workspaces[0].git = Some(git_info("/repo", "/repo", false, Some("main")));
    push_git_workspace(
        &mut state,
        "feat",
        "/repo/.worktrees/feat",
        git_info("/repo", "/repo/.worktrees/feat", true, Some("feat")),
    );
    // 激活子项，使父项处于非激活状态。
    state.active_workspace = 1;

    let config = Config::default();
    let mut terminal =
        RatatuiTerminal::new(TestBackend::new(100, 24)).expect("test backend is infallible");

    // 1. 未交互态：父项行首标记列为空、折叠箭头为 Cyan 强调色，无反显与背景。
    if let Err(error) = terminal.draw(|frame| render(frame, &state, &config)) {
        panic!("draw failed: {error}");
    }
    let buffer = terminal.backend().buffer().clone();
    let view = view_for(&state);
    let sections = layout::sidebar_sections(view.sidebar, false).expect("sections are visible");
    let x = sections.workspaces.x;
    let y = sections.workspaces.y;
    let arrow_x = x + 2;

    assert_eq!(buffer[(x, y)].symbol(), " ");
    assert_eq!(
        buffer[(arrow_x, y)].symbol(),
        text::WORKSPACE_GROUP_EXPANDED
    );
    assert_eq!(buffer[(arrow_x, y)].fg, ratatui::style::Color::Cyan);
    assert_eq!(buffer[(arrow_x, y)].bg, ratatui::style::Color::Reset);
    assert!(
        !buffer[(arrow_x, y)]
            .modifier
            .contains(ratatui::style::Modifier::REVERSED)
    );

    // 2. 悬停态：父项整行统一深色高亮背景（Indexed 240）；折叠箭头是特殊文字，
    //    保留 accent 强调色，不被 hover 的 Indexed 255 前景覆盖。
    state.workspace_hover = Some(0);
    if let Err(error) = terminal.draw(|frame| render(frame, &state, &config)) {
        panic!("draw failed: {error}");
    }
    let buffer = terminal.backend().buffer().clone();

    assert_eq!(
        buffer[(arrow_x, y)].symbol(),
        text::WORKSPACE_GROUP_EXPANDED
    );
    assert_eq!(
        buffer[(arrow_x, y)].fg,
        ratatui::style::Color::Cyan,
        "悬停不覆盖分组箭头强调色"
    );
    assert!(
        buffer[(arrow_x, y)]
            .modifier
            .contains(ratatui::style::Modifier::BOLD)
    );
    assert_eq!(buffer[(arrow_x, y)].bg, ratatui::style::Color::Indexed(240));
    assert!(
        !buffer[(arrow_x, y)]
            .modifier
            .contains(ratatui::style::Modifier::REVERSED)
    );

    // 3. 激活态：父项行首 `❯` 标记，箭头保留强调色、无填充背景。
    state.workspace_hover = None;
    state.active_workspace = 0;
    if let Err(error) = terminal.draw(|frame| render(frame, &state, &config)) {
        panic!("draw failed: {error}");
    }
    let buffer = terminal.backend().buffer().clone();

    assert_eq!(buffer[(x, y)].symbol(), text::AGENT_SELECTED_MARKER);
    assert_eq!(
        buffer[(arrow_x, y)].symbol(),
        text::WORKSPACE_GROUP_EXPANDED
    );
    assert_eq!(buffer[(arrow_x, y)].fg, ratatui::style::Color::Cyan);
    assert_eq!(buffer[(arrow_x, y)].bg, ratatui::style::Color::Reset);
}

#[test]
fn dragged_workspace_item_renders_selection() {
    let mut state = AppState::demo();
    update::create_workspace(&mut state);
    state.workspace_drag = Some(0);
    state.workspace_dragging = true;
    let config = Config::default();
    let mut terminal =
        RatatuiTerminal::new(TestBackend::new(100, 24)).expect("test backend is infallible");
    if let Err(error) = terminal.draw(|frame| render(frame, &state, &config)) {
        panic!("draw failed: {error}");
    }
    let buffer = terminal.backend().buffer().clone();
    let view = view_for(&state);
    let sections = layout::sidebar_sections(view.sidebar, false).expect("sections are visible");
    let y = sections.workspaces.y;
    assert!(
        !buffer[(sections.workspaces.x, y)]
            .modifier
            .contains(ratatui::style::Modifier::REVERSED)
    );
    assert_eq!(
        buffer[(sections.workspaces.x, y)].bg,
        ratatui::style::Color::Indexed(240)
    );
    assert!(
        !buffer[(sections.workspaces.x, y + 1)]
            .modifier
            .contains(ratatui::style::Modifier::REVERSED)
    );
    // 拖动高亮整行铺满，空格区同样覆盖。
    assert_eq!(
        buffer[(sections.workspaces.right() - 1, y)].bg,
        ratatui::style::Color::Indexed(240),
        "拖动高亮覆盖整行空白区"
    );
}

#[test]
fn drag_highlight_marks_group_only_after_pointer_move() {
    let mut state = AppState::demo();
    state.workspaces[0].git = Some(git_info("/repo", "/repo", false, Some("main")));
    push_git_workspace(
        &mut state,
        "feat",
        "/repo/.worktrees/feat",
        git_info("/repo", "/repo/.worktrees/feat", true, Some("feature/x")),
    );
    state.workspace_drag = Some(0);

    let config = Config::default();
    let mut terminal =
        RatatuiTerminal::new(TestBackend::new(100, 24)).expect("test backend is infallible");
    let view = view_for(&state);
    let sections = layout::sidebar_sections(view.sidebar, false).expect("sections are visible");
    let mut observe = |state: &AppState| {
        if let Err(error) = terminal.draw(|frame| render(frame, state, &config)) {
            panic!("draw failed: {error}");
        }
        let buffer = terminal.backend().buffer().clone();
        (
            [
                buffer[(sections.workspaces.x, sections.workspaces.y)].bg
                    == ratatui::style::Color::Indexed(240),
                buffer[(sections.workspaces.x, sections.workspaces.y + 1)].bg
                    == ratatui::style::Color::Indexed(240),
            ],
            buffer[(sections.workspaces.x, sections.workspaces.y)].bg,
        )
    };

    // 按下未移动：激活项以行首标记表示，不做拖动反显。
    assert_eq!(
        observe(&state),
        ([false, false], ratatui::style::Color::Reset),
        "按下未移动无拖动反显"
    );
    state.workspace_dragging = true;
    assert_eq!(
        observe(&state),
        ([true, true], ratatui::style::Color::Indexed(240)),
        "指针移动后整块使用深色高亮背景"
    );
}

#[test]
fn panel_collapse_buttons_sit_on_bottom_border() {
    let state = AppState::demo();
    let view = view_for(&state);
    let sidebar = button_for(&view, CollapseTarget::Sidebar);
    let prompt = button_for(&view, CollapseTarget::Prompt);
    assert!(!sidebar.collapsed && !prompt.collapsed);
    assert_eq!(sidebar.area.y, view.sidebar.bottom() - 1);
    assert_eq!(prompt.area.y, view.prompt.bottom() - 1);
    assert_eq!(
        collapse_button_at(&view, false, sidebar.area.x, sidebar.area.y),
        Some(CollapseTarget::Sidebar)
    );

    let lines = render_lines(&state);
    assert_eq!(
        rendered_label(&lines, &sidebar),
        text::SIDEBAR_COLLAPSE_LABEL
    );
    assert_eq!(rendered_label(&lines, &prompt), text::RIGHT_COLLAPSE_LABEL);
}

#[test]
fn collapsed_panel_buttons_sit_on_bottom_strip_row() {
    let mut state = AppState::demo();
    update::apply(Action::ToggleSidebar, &mut state);
    update::apply(Action::TogglePrompt, &mut state);
    let view = view_for(&state);
    let sidebar = button_for(&view, CollapseTarget::Sidebar);
    let prompt = button_for(&view, CollapseTarget::Prompt);
    assert!(sidebar.collapsed && prompt.collapsed);
    assert_eq!(sidebar.area.y, view.sidebar.bottom() - 1);
    assert_eq!(prompt.area.y, view.prompt.bottom() - 1);
    assert_eq!(
        rendered_collapsed_label(&render_lines(&state), &view, CollapseTarget::Sidebar),
        text::SIDEBAR_EXPAND_LABEL
    );
    assert_eq!(
        rendered_collapsed_label(&render_lines(&state), &view, CollapseTarget::Prompt),
        text::RIGHT_EXPAND_LABEL
    );
}

#[test]
fn swap_button_renders_between_panels_and_moves_prompt() {
    let mut state = AppState::demo();
    let view = view_for(&state);
    let button = swap_button(&view).expect("swap button visible");
    assert!(view.panes.x < view.prompt.x);
    assert_eq!(
        rendered_area(&render_lines(&state), button),
        text::SWAP_PROMPT_LABEL
    );
    let buffer = render_buffer(&state);
    for x in button.x..button.right() {
        assert_eq!(buffer[(x, button.y)].fg, ratatui::style::Color::Cyan);
    }

    update::swap_prompt_side(&mut state, 76, 30);
    let view = view_for(&state);
    assert!(view.prompt_left);
    assert!(view.prompt.right() <= view.panes.x);
    let button = swap_button(&view).expect("swap button visible after swap");
    assert_eq!(button.x, view.prompt.right() - 1);
    assert_eq!(
        rendered_area(&render_lines(&state), button),
        text::SWAP_PROMPT_LABEL
    );
}

#[test]
fn swap_button_needs_vertical_room() {
    let config = Config::default();
    let short = layout::compute(
        Rect::new(0, 0, 100, 2),
        &config,
        false,
        config.sidebar_width,
        false,
        30,
        false,
        false,
    );
    assert_eq!(swap_button(&short), None);
}

#[test]
fn swap_button_stays_visible_when_prompt_collapsed() {
    let mut state = AppState::demo();
    update::apply(Action::TogglePrompt, &mut state);
    let view = view_for(&state);
    assert!(state.prompt_collapsed);
    let button = swap_button(&view).expect("swap button visible on collapsed strip");
    assert_eq!(button.right(), view.prompt.x + 1);
}

#[test]
fn swapped_layout_moves_collapse_button_to_panes() {
    let mut state = AppState::demo();
    update::swap_prompt_side(&mut state, 76, 30);
    let view = view_for(&state);
    // prompt 在左不可折叠：折叠按钮与能力归右列主区。
    let buttons = collapse_buttons(&view, false);
    assert!(
        buttons
            .iter()
            .all(|button| button.target != CollapseTarget::Prompt)
    );
    let panes = buttons
        .into_iter()
        .find(|button| button.target == CollapseTarget::Panes)
        .expect("panes collapse button");
    assert!(!panes.collapsed);
    assert_eq!(panes.area.y, view.panes.bottom() - 1);
    assert_eq!(
        rendered_label(&render_lines(&state), &panes),
        text::RIGHT_COLLAPSE_LABEL
    );

    // 折叠：主区收为贴右缘窄条（分隔线在左缘），prompt 占满其余宽度。
    update::apply(Action::TogglePanes, &mut state);
    let view = view_for(&state);
    assert_eq!(view.panes, Rect::new(96, 1, 4, 23));
    assert_eq!(view.prompt, Rect::new(24, 1, 72, 23));
    let lines = render_lines(&state);
    let row: Vec<char> = lines[view.panes.y as usize].chars().collect();
    assert_eq!(row[usize::from(view.panes.x)], '│');
    assert_eq!(
        rendered_collapsed_label(&lines, &view, CollapseTarget::Panes),
        text::RIGHT_EXPAND_LABEL
    );
}

/// 让 prompt 文本溢出视口：按视口尺寸 resize 后写入两倍视口高度的行。
fn overflow_prompt(state: &mut AppState, view: &layout::ViewLayout) {
    let text = crate::layout::prompt_text_rect(view.prompt);
    state.prompt.resize(text.width, text.height);
    for _ in 0..=usize::from(text.height) {
        state.prompt.insert_str("line\n");
    }
}

#[test]
fn prompt_tree_renders_divider_and_scrollbar_below_search() {
    use crate::app::prompt::PromptItem;
    let mut state = AppState::demo();
    update::show_prompt_tree(&mut state);
    let panel = &mut state.workspaces[0].prompt_panel;
    panel.tree.replace(
        vec![],
        (0..30)
            .map(|index| PromptItem::single(index, None, format!("item-{index:02}"), String::new()))
            .collect(),
    );
    let view = view_for(&state);
    let lines = render_lines(&state);
    let text = crate::layout::prompt_text_rect(view.prompt);
    let search = prompt_line(&lines, &view, text.y);
    assert!(
        search.contains(text::PROMPT_TREE_SEARCH_PLACEHOLDER),
        "{search}"
    );
    // 分割线整行 `─`，两端 `├` / `┤` 衔接面板边框。
    let divider = prompt_line(&lines, &view, text.y + 1);
    assert!(divider.contains(text::DIVIDER_LEFT_JOIN), "{divider}");
    assert!(divider.contains(text::DIVIDER_RIGHT_JOIN), "{divider}");
    // 滚动条槽从分割线之下开始：搜索行与分割线行无滚动条，列表首行起有。
    let gutter = crate::layout::prompt_tree_scrollbar_rect(view.prompt).expect("gutter");
    assert_eq!(gutter.y, text.y + 2);
    let search_row: Vec<char> = lines[text.y as usize].chars().collect();
    assert_ne!(search_row[usize::from(gutter.x)].to_string(), "▐");
    let list_row: Vec<char> = lines[gutter.y as usize].chars().collect();
    assert_eq!(list_row[usize::from(gutter.x)].to_string(), "▐");
}

#[test]
fn prompt_editor_divider_shows_editing_title() {
    use crate::app::prompt::PromptItem;
    let mut state = AppState::demo();
    state.prompt_width = 40;
    let panel = &mut state.workspaces[0].prompt_panel;
    panel.tree.replace(
        vec![],
        vec![PromptItem::single(1, None, "named prompt", String::new())],
    );
    assert!(update::edit_prompt_item(&mut state, 0, 1));
    let view = view_for(&state);
    let lines = render_lines(&state);
    let inner = crate::layout::pane_inner_rect(view.prompt);
    let divider = prompt_line(
        &lines,
        &view,
        inner.y + crate::layout::PROMPT_DIVIDER_HEIGHT,
    );
    assert!(divider.contains(" named prompt "), "{divider}");
    assert!(!divider.contains("[tmp]"), "条目不带 tmp 标识: {divider}");

    // 临时提示词显示固定标题（无 `[tmp]` 标识），标题为未聚焦灰。
    state.prompt_width = 56;
    let view = view_for(&state);
    update::show_prompt_tree(&mut state);
    assert!(update::open_temp_prompt(&mut state));
    let lines = render_lines(&state);
    let inner = crate::layout::pane_inner_rect(view.prompt);
    let divider = prompt_line(
        &lines,
        &view,
        inner.y + crate::layout::PROMPT_DIVIDER_HEIGHT,
    );
    assert!(divider.contains(text::PROMPT_DRAFT_TITLE), "{divider}");
    assert!(!divider.contains("[tmp]"), "标题不带 tmp 标识: {divider}");
    assert!(divider.contains("Page 1"), "内嵌当前页名: {divider}");
    assert!(divider.contains('‹') && divider.contains('›'), "{divider}");
    assert!(divider.contains("1/1"), "{divider}");
    // 标题为未聚焦灰（muted）：前景默认、DIM，即使 prompt 持有焦点。
    let buffer = render_buffer(&state);
    let y = inner.y + crate::layout::PROMPT_DIVIDER_HEIGHT;
    let title_byte = divider.find(text::PROMPT_DRAFT_TITLE).expect("title col");
    let title_col = u16::try_from(divider[..title_byte].chars().count()).expect("col fits");
    let cell = &buffer[(view.prompt.x + title_col, y)];
    let debug_row: String = (view.prompt.x..view.prompt.right())
        .map(|x| buffer[(x, y)].symbol())
        .collect();
    assert_eq!(cell.symbol(), "T", "标题起点单元格: {debug_row:?}");
    assert_eq!(cell.fg, ratatui::style::Color::Reset, "标题前景为默认灰");
    assert!(cell.modifier.contains(ratatui::style::Modifier::DIM));
}

#[test]
fn prompt_scrollbar_appears_only_on_overflow() {
    let mut state = AppState::demo();
    let view = view_for(&state);
    assert_eq!(prompt_scrollbar(&view, &state), None);

    overflow_prompt(&mut state, &view);
    let view = view_for(&state);
    let bar = prompt_scrollbar(&view, &state).expect("scrollbar is visible");
    let gutter = crate::layout::prompt_scrollbar_rect(view.prompt).expect("gutter is reserved");
    assert_eq!(bar.track, gutter);
    assert_eq!(bar.visible, usize::from(gutter.height));
    assert_eq!(
        bar.total,
        state.prompt.visual_rows().len() + state.prompt.bottom_padding(),
        "滚动条总长含底部虚拟空占位"
    );
    assert!(bar.total > bar.visible);
}

#[test]
fn prompt_text_wraps_before_scrollbar_gutter() {
    let mut state = AppState::demo();
    let view = view_for(&state);
    let text = crate::layout::prompt_text_rect(view.prompt);
    state.prompt.resize(text.width, text.height);
    assert!(text.width > 1);
    state
        .prompt
        .insert_str(&"x".repeat(usize::from(text.width) + 1));
    assert_eq!(state.prompt.visual_rows().len(), 2);
}

#[test]
fn prompt_scrollbar_renders_track_and_thumb_in_gutter() {
    let mut state = AppState::demo();
    let view = view_for(&state);
    overflow_prompt(&mut state, &view);
    let view = view_for(&state);
    let gutter = crate::layout::prompt_scrollbar_rect(view.prompt).expect("gutter is reserved");
    update::set_prompt_scroll(&mut state, 0);
    let lines = render_lines(&state);
    let row: Vec<char> = lines[gutter.y as usize].chars().collect();
    assert_eq!(row[usize::from(gutter.x)].to_string(), "▐");

    update::set_prompt_scroll(&mut state, usize::MAX);
    let lines = render_lines(&state);
    let last: Vec<char> = lines[gutter.bottom() as usize - 1].chars().collect();
    assert_eq!(last[usize::from(gutter.x)].to_string(), "▐");
    let middle: Vec<char> = lines[gutter.y as usize + 1].chars().collect();
    assert_eq!(middle[usize::from(gutter.x)].to_string(), "▕");
}

#[test]
fn prompt_renders_toolbar_and_divider_rows() {
    let mut state = AppState::demo();
    // 足够宽以容纳模式/undo/redo、右对齐的 select all 与标题行页控件。
    state.prompt_width = 56;
    state.prompt.insert_str("hello");
    let view = view_for(&state);
    let lines = render_lines(&state);
    let inner = crate::layout::pane_inner_rect(view.prompt);

    // 按钮行：功能按钮齐全，分割线不穿过按钮行。
    let toolbar = prompt_line(&lines, &view, inner.y);
    assert!(toolbar.contains(text::PROMPT_MODE_LIST_LABEL), "{toolbar}");
    assert!(toolbar.contains(text::PROMPT_UNDO_LABEL), "{toolbar}");
    assert!(toolbar.contains(text::PROMPT_REDO_LABEL), "{toolbar}");
    assert!(toolbar.contains(text::PROMPT_SELECT_ALL_LABEL), "{toolbar}");
    assert!(toolbar.contains(text::PROMPT_DRAFT_DOT), "{toolbar}");
    assert!(
        !toolbar.contains(text::DIVIDER_MID),
        "分割线不穿过按钮行: {toolbar}"
    );

    // 下一行为贯穿分割线（两端 `├` / `┤`），编辑视图内嵌目标 title。
    let divider = prompt_line(
        &lines,
        &view,
        inner.y + crate::layout::PROMPT_TOOLBAR_HEIGHT,
    );
    assert!(divider.contains(text::DIVIDER_LEFT_JOIN), "{divider}");
    assert!(divider.contains(text::DIVIDER_RIGHT_JOIN), "{divider}");
    assert!(divider.contains(text::DIVIDER_MID), "{divider}");
    assert!(
        divider.contains(text::PROMPT_DRAFT_TITLE),
        "草稿 title 内嵌分割线: {divider}"
    );
}

#[test]
fn prompt_bottom_shows_branch_and_worktree_only() {
    let mut state = AppState::demo();
    state.workspaces[0].name = "work".to_string();
    state.workspaces[0].cwd = Some(PathBuf::from("/repo/work"));
    state.workspaces[0].git = Some(git_info("/repo", "/repo", false, Some("main")));
    let view = view_for(&state);
    let bottom = prompt_line(&render_lines(&state), &view, view.prompt.bottom() - 1);
    assert!(bottom.contains("b:main"), "{bottom}");
    assert!(!bottom.contains("wt:"), "{bottom}");
    assert!(!bottom.contains("/repo"), "不显示项目路径: {bottom}");

    // linked worktree：显示仓库主 checkout 分支 + ` wt:工作区名`，不显示自身分支。
    state.workspaces[0].git = Some(WorkspaceGit {
        repo_root: PathBuf::from("/repo"),
        checkout_path: PathBuf::from("/repo/.worktrees/work"),
        is_linked: true,
        branch: Some("feat/prompt-controls".to_string()),
        main_branch: Some("dev".to_string()),
    });
    let bottom = prompt_line(&render_lines(&state), &view, view.prompt.bottom() - 1);
    assert!(bottom.contains("b:dev"), "{bottom}");
    assert!(bottom.contains("wt:work"), "{bottom}");
    assert!(!bottom.contains("-wt:"), "不再使用 -wt: 标记: {bottom}");
    assert!(
        !bottom.contains("feat/"),
        "不显示 worktree 自身分支: {bottom}"
    );

    // 非 git 不显示任何状态。
    state.workspaces[0].git = None;
    let bottom = prompt_line(&render_lines(&state), &view, view.prompt.bottom() - 1);
    assert!(!bottom.contains("b:"), "{bottom}");
    assert!(!bottom.contains("wt:"), "{bottom}");
}

#[test]
fn prompt_title_moves_path_name_to_bottom_right() {
    let mut state = AppState::demo();
    state.workspaces[0].git = Some(git_info("/repo", "/repo/lx-tui", false, Some("main")));
    let view = view_for(&state);
    let lines = render_lines(&state);
    let top = prompt_line(&lines, &view, view.prompt.y);
    assert!(top.contains(text::PROMPT_TITLE), "{top}");
    assert!(!top.contains("ws:"), "顶边框不再显示 ws 路径名: {top}");

    // `ws:` 落在底边框右侧、折叠按钮左侧隔 1 列。
    let bottom = prompt_line(&lines, &view, view.prompt.bottom() - 1);
    let collapse = button_for(&view, CollapseTarget::Prompt);
    let chars: Vec<char> = bottom.chars().collect();
    let label: Vec<char> = "ws:lx-tui".chars().collect();
    let label_start = chars
        .windows(label.len())
        .position(|window| window == label.as_slice())
        .expect("ws 标签已渲染");
    let label_end = view.prompt.x + label_start as u16 + label.len() as u16;
    assert_eq!(label_end, collapse.area.x - 1, "ws 右端与折叠按钮隔 1 列");
    assert_eq!(
        collapse.area.x - label_end,
        1,
        "ws 与折叠按钮之间有且仅有 1 列空档: {bottom}"
    );

    // 高亮与 `b:` 一致：前缀强调色、值 muted。
    let buffer = render_buffer(&state);
    let row = view.prompt.bottom() - 1;
    let prefix = &buffer[(view.prompt.x + label_start as u16, row)];
    assert_eq!(prefix.symbol(), "w");
    assert_eq!(prefix.fg, ratatui::style::Color::Cyan);
    assert!(prefix.modifier.contains(ratatui::style::Modifier::BOLD));
    let value = &buffer[(
        view.prompt.x + label_start as u16 + text::PROMPT_WORKSPACE_PREFIX.len() as u16,
        row,
    )];
    assert_eq!(value.symbol(), "l");
    assert!(value.modifier.contains(ratatui::style::Modifier::DIM));
    assert_ne!(value.fg, ratatui::style::Color::Cyan);

    // linked worktree：取 checkout 路径末段，而非工作区 cwd 子目录。
    state.workspaces[0].git = Some(WorkspaceGit {
        repo_root: PathBuf::from("/repo"),
        checkout_path: PathBuf::from("/repo/.worktrees/work"),
        is_linked: true,
        branch: Some("feat/x".to_string()),
        main_branch: Some("main".to_string()),
    });
    state.workspaces[0].cwd = Some(PathBuf::from("/repo/.worktrees/work/src"));
    // 放宽面板，避免 `b:`/`wt:` 挤压导致 `ws:` 截断，从而验证取 checkout 末段而非 cwd 子目录。
    state.prompt_width = 60;
    let wide_view = view_for(&state);
    let bottom = prompt_line(
        &render_lines(&state),
        &wide_view,
        wide_view.prompt.bottom() - 1,
    );
    assert!(bottom.contains("ws:work"), "{bottom}");
    assert!(
        !bottom.contains("ws:src"),
        "不取工作区 cwd 子目录: {bottom}"
    );

    // 非 git 回退工作区 cwd；无路径时无 `ws:` 片段。
    state.workspaces[0].git = None;
    state.workspaces[0].cwd = Some(PathBuf::from("/repo/plain"));
    let view = view_for(&state);
    let bottom = prompt_line(&render_lines(&state), &view, view.prompt.bottom() - 1);
    assert!(bottom.contains("ws:plain"), "{bottom}");

    state.workspaces[0].cwd = None;
    let bottom = prompt_line(&render_lines(&state), &view, view.prompt.bottom() - 1);
    assert!(bottom.contains('╰'), "{bottom}");
    assert!(!bottom.contains("ws:"), "{bottom}");
}

#[test]
fn prompt_bottom_bar_prefers_bound_prompt_root() {
    let mut state = AppState::demo();
    // 标题文案变长后，默认宽度容不下长路径名与标题共存；本用例只验证路径优先级。
    state.prompt_width = 40;
    state.workspaces[0].git = Some(git_info("/repo", "/repo/lx-tui", false, Some("main")));
    let view = view_for(&state);

    // 显式绑定根路径优先于工作区路径。
    state.prompt_root = Some(PathBuf::from("/elsewhere/bound-root"));
    let bottom = prompt_line(&render_lines(&state), &view, view.prompt.bottom() - 1);
    assert!(bottom.contains("ws:bound-root"), "{bottom}");
    assert!(!bottom.contains("ws:lx-tui"), "{bottom}");

    // 绑定路径无末段时回退工作区路径。
    state.prompt_root = Some(PathBuf::from("/"));
    let bottom = prompt_line(&render_lines(&state), &view, view.prompt.bottom() - 1);
    assert!(bottom.contains("ws:lx-tui"), "{bottom}");

    // 未绑定时仍显示工作区路径。
    state.prompt_root = None;
    let bottom = prompt_line(&render_lines(&state), &view, view.prompt.bottom() - 1);
    assert!(bottom.contains("ws:lx-tui"), "{bottom}");
}

#[test]
fn prompt_bottom_ws_label_truncates_tail_keeping_prefix() {
    let mut state = AppState::demo();
    let long = "ws-very-long-project-directory-name";
    let checkout = format!("/repo/{long}");
    state.workspaces[0].git = Some(git_info("/repo", &checkout, false, Some("main")));
    state.prompt_width = 40;
    let view = view_for(&state);
    let bottom = prompt_line(&render_lines(&state), &view, view.prompt.bottom() - 1);
    assert!(bottom.contains("ws:"), "{bottom}");
    assert!(bottom.contains('…'), "超长名截断以省略号收尾: {bottom}");
    // 保留开头：截断后紧跟的前缀是路径名开头，而非结尾。
    let after = &bottom[bottom.find("ws:").unwrap() + 3..];
    assert!(after.starts_with("ws-very"), "保留名字开头、砍尾: {bottom}");
    // `ws:` 左界严格晚于 `b:` 状态右端（两者至少空 1 列，不重叠）。
    let b_end = bottom.find("b:main").expect("branch status") + "b:main".len();
    let ws_pos = bottom.find("ws:").expect("ws label");
    assert!(ws_pos > b_end, "ws 需与 b: 内容保持间隔: {bottom}");
}

#[test]
fn prompt_bottom_ws_label_hides_when_too_narrow() {
    let mut state = AppState::demo();
    state.workspaces[0].git = Some(git_info("/repo", "/repo/lx-tui", false, Some("main")));
    state.prompt_width = 14;
    let view = view_for(&state);
    let bottom = prompt_line(&render_lines(&state), &view, view.prompt.bottom() - 1);
    assert!(!bottom.contains("ws:"), "过窄时整体隐藏 ws 片段: {bottom}");
    assert!(
        bottom.contains("b:"),
        "b: 分支状态优先级最高、保留: {bottom}"
    );
    // 折叠按钮仍在。
    assert!(bottom.contains(text::RIGHT_COLLAPSE_LABEL), "{bottom}");
}

#[test]
fn prompt_pin_button_sits_left_of_select_all_and_tracks_state() {
    let mut state = AppState::demo();
    let view = view_for(&state);
    let bar = crate::layout::prompt_toolbar_rect(view.prompt).expect("toolbar visible");
    let pin = crate::layout::prompt_toolbar_button_rect(
        view.prompt,
        crate::layout::PromptToolbarButton::Pin,
        false,
    )
    .expect("pin visible");
    let select_all = crate::layout::prompt_toolbar_button_rect(
        view.prompt,
        crate::layout::PromptToolbarButton::SelectAll,
        false,
    )
    .expect("select all visible");
    assert_eq!(
        pin.width,
        text::PROMPT_UNPINNED_LABEL.chars().count() as u16
    );
    assert_eq!(pin.y, bar.y, "位于功能按钮行");
    assert_eq!(select_all.x - pin.right(), 1, "紧邻 [*] 左侧，间隔 1 列");
    assert_eq!(
        crate::layout::prompt_toolbar_button_at(view.prompt, pin.x, pin.y, false),
        Some(crate::layout::PromptToolbarButton::Pin)
    );
    assert_eq!(
        crate::layout::prompt_toolbar_button_at(view.prompt, pin.x, pin.y - 1, false),
        None,
        "分割线行不命中"
    );

    state.prompt_pinned = Some(0);
    let pinned = crate::layout::prompt_toolbar_button_rect(
        view.prompt,
        crate::layout::PromptToolbarButton::Pin,
        false,
    )
    .expect("pin visible");
    assert_eq!(pinned, pin, "两态同宽同位（仅符号变化）");

    update::apply(Action::TogglePrompt, &mut state);
    let collapsed = view_for(&state);
    assert_eq!(
        crate::layout::prompt_toolbar_button_rect(
            collapsed.prompt,
            crate::layout::PromptToolbarButton::Pin,
            false
        ),
        None,
        "折叠态无表头不显示"
    );
}

#[test]
fn prompt_pin_button_renders_state_symbol_in_accent() {
    let mut state = AppState::demo();
    let view = view_for(&state);
    let pin = crate::layout::prompt_toolbar_button_rect(
        view.prompt,
        crate::layout::PromptToolbarButton::Pin,
        false,
    )
    .expect("pin visible");
    let buffer = render_buffer(&state);
    let rendered: String = (pin.x..pin.right())
        .map(|x| buffer[(x, pin.y)].symbol())
        .collect();
    assert_eq!(rendered, text::PROMPT_UNPINNED_LABEL);
    for x in pin.x..pin.right() {
        assert_eq!(buffer[(x, pin.y)].fg, ratatui::style::Color::Cyan);
    }

    state.prompt_pinned = Some(0);
    let buffer = render_buffer(&state);
    let rendered: String = (pin.x..pin.right())
        .map(|x| buffer[(x, pin.y)].symbol())
        .collect();
    assert_eq!(rendered, text::PROMPT_PINNED_LABEL);
}

#[test]
fn prompt_pin_button_follows_tree_toolbar_left_of_temp_prompt() {
    let mut state = AppState::demo();
    state.workspaces[0].prompt_panel.mode = PromptMode::Tree;
    let view = view_for(&state);
    let pin = crate::layout::prompt_toolbar_button_rect(
        view.prompt,
        crate::layout::PromptToolbarButton::Pin,
        true,
    )
    .expect("pin visible in tree view");
    let temp = crate::layout::prompt_toolbar_button_rect(
        view.prompt,
        crate::layout::PromptToolbarButton::TempPrompt,
        true,
    )
    .expect("temp prompt visible");
    assert_eq!(temp.x - pin.right(), 1, "树视图紧邻 [+] 左侧");
}

#[test]
fn prompt_status_follows_pinned_workspace() {
    let mut state = AppState::demo();
    state.workspaces[0].name = "one".to_string();
    state.workspaces[0].cwd = Some(PathBuf::from("/repo/one"));
    state.workspaces[0].git = Some(git_info("/repo", "/repo/one", false, Some("main")));
    push_git_workspace(
        &mut state,
        "two",
        "/repo/two",
        git_info("/repo", "/repo/two", false, Some("dev")),
    );

    state.prompt_pinned = Some(0);
    update::switch_workspace(&mut state, 1);
    assert_eq!(state.active_workspace, 1);
    let view = view_for(&state);
    let lines = render_lines(&state);
    let bottom = prompt_line(&lines, &view, view.prompt.bottom() - 1);
    assert!(
        bottom.contains("ws:one"),
        "钉住时底栏取显示中工作区: {bottom}"
    );
    assert!(
        bottom.contains("b:main"),
        "钉住时底边取显示中工作区: {bottom}"
    );

    update::release_prompt_pin(&mut state);
    let lines = render_lines(&state);
    let bottom = prompt_line(&lines, &view, view.prompt.bottom() - 1);
    assert!(
        bottom.contains("ws:two"),
        "释放后底栏回到激活工作区: {bottom}"
    );
    assert!(
        bottom.contains("b:dev"),
        "释放后底边回到激活工作区: {bottom}"
    );
}

#[test]
fn prompt_toolbar_styles_reflect_undo_and_save_state() {
    let mut state = AppState::demo();
    let view = view_for(&state);
    let undo = crate::layout::prompt_toolbar_button_rect(
        view.prompt,
        crate::layout::PromptToolbarButton::Undo,
        false,
    )
    .expect("undo button visible");
    let undo_x = undo.x;
    let bar_y = undo.y;
    let dot_x = crate::layout::pane_inner_rect(view.prompt).right() - 1;

    let buffer = render_buffer(&state);
    assert_eq!(buffer[(undo_x, bar_y)].symbol(), "[");
    assert!(
        buffer[(undo_x, bar_y)]
            .modifier
            .contains(ratatui::style::Modifier::DIM),
        "无历史时 undo 置灰"
    );
    assert_eq!(
        buffer[(dot_x, bar_y)].fg,
        ratatui::style::Color::DarkGray,
        "临时草稿恒灰点"
    );

    state.prompt.insert_str("x");
    let buffer = render_buffer(&state);
    assert_eq!(
        buffer[(undo_x, bar_y)].fg,
        ratatui::style::Color::Cyan,
        "可撤销时强调色"
    );
    assert_eq!(
        buffer[(dot_x, bar_y)].fg,
        ratatui::style::Color::DarkGray,
        "草稿编辑后仍为灰点（不参与自动保存）"
    );

    // 绑定条目：已保存绿点、编辑后黄点。
    use crate::app::prompt::PromptItem;
    let panel = &mut state.workspaces[0].prompt_panel;
    panel.tree.replace(
        vec![],
        vec![PromptItem::single(1, None, "named", String::new())],
    );
    assert!(update::edit_prompt_item(&mut state, 0, 1));
    let buffer = render_buffer(&state);
    assert_eq!(
        buffer[(dot_x, bar_y)].fg,
        ratatui::style::Color::Green,
        "条目已保存为绿点"
    );
    state.prompt.insert_str("x");
    let buffer = render_buffer(&state);
    assert_eq!(
        buffer[(dot_x, bar_y)].fg,
        ratatui::style::Color::Yellow,
        "条目编辑后未保存为黄点"
    );
}

#[test]
fn tab_bar_renders_auto_titles_and_add_button() {
    let state = AppState::demo();
    let lines = render_lines(&state);
    assert!(lines[0].contains("tab 1"), "{}", lines[0]);
    assert!(lines[0].contains(text::TAB_ITEM_ICON), "{}", lines[0]);
    assert!(!lines[0].contains("logs"));
    assert!(lines[0].contains(text::ADD_TAB_LABEL), "{}", lines[0]);
}

#[test]
fn workspace_items_distinguish_git_and_non_git_icons() {
    let mut state = AppState::demo();
    state.workspaces[0].name = "plain-dir".to_string();
    state.workspaces[0].git = None;
    push_git_workspace(
        &mut state,
        "repo-dir",
        "/repo",
        git_info("/repo", "/repo", false, Some("main")),
    );
    let lines = render_lines(&state);
    assert!(
        lines
            .iter()
            .any(|line| line.contains(&format!("{} plain-dir", text::WORKSPACE_NON_GIT_ICON))),
        "non-git workspace must show non-git icon: {lines:?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains(&format!("{} repo-dir", text::WORKSPACE_GIT_ICON))),
        "git workspace must show git icon: {lines:?}"
    );
}

#[test]
fn tab_bar_overflow_renders_scroll_buttons_and_keeps_exit() {
    let mut state = AppState::demo();
    for _ in 1..12 {
        update::create_tab(&mut state);
    }
    let lines = render_lines(&state);
    assert!(
        lines[0].contains(text::TAB_SCROLL_LEFT_LABEL),
        "{}",
        lines[0]
    );
    assert!(
        lines[0].contains(text::TAB_SCROLL_RIGHT_LABEL),
        "{}",
        lines[0]
    );
    assert!(
        lines[0].trim_end().ends_with(text::EXIT_LABEL),
        "{}",
        lines[0]
    );
}

#[test]
fn pane_title_uses_cwd_label_before_placeholder() {
    let mut state = AppState::demo();
    terminal_view(&mut state);
    let id = state.active_tab().layout.focus();
    update::update_pane_cwd(&mut state, id, Path::new("/dev-dir"), "dev-dir".to_string());
    let lines = render_lines(&state);
    assert!(lines.iter().any(|line| line.contains("dev-dir")));
    assert!(
        !lines
            .iter()
            .any(|line| line.contains(&format!("pane {}", id.raw())))
    );
}

#[test]
fn pane_title_prefers_osc_over_cwd_label() {
    let mut state = AppState::demo();
    terminal_view(&mut state);
    let id = state.active_tab().layout.focus();
    update::update_pane_cwd(&mut state, id, Path::new("/dev-dir"), "dev-dir".to_string());
    if let Some(pane) = state.active_tab_mut().pane_mut(id) {
        let _ = pane.terminal.feed(b"\x1b]0;Claude Code\x07");
    }
    let lines = render_lines(&state);
    assert!(lines.iter().any(|line| line.contains("Claude Code")));
    assert!(!lines.iter().any(|line| line.contains("dev-dir")));
}

/// 矩形内的渲染文本。
fn rendered_area(lines: &[String], area: Rect) -> String {
    let row: Vec<char> = lines[area.y as usize].chars().collect();
    (area.x..area.right()).map(|x| row[x as usize]).collect()
}

#[test]
fn lx_pane_top_border_shows_workspace_path_tail() {
    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    // pane cwd 优先于工作区 cwd；取末两段。
    update::update_pane_cwd(&mut state, pane, Path::new("/work/demo/src"), "demo".into());
    let config = Config::default();
    let view = view_for(&state);
    let rects = crate::layout::pane_rects(
        &state.active_tab().layout,
        view.panes,
        config.min_pane_width,
        config.min_pane_height,
    );
    let (_, rect) = rects[0];
    let lines = render_lines(&state);
    let top = &lines[usize::from(rect.y)];
    assert!(top.contains("ws:demo/src"), "顶边框显示 ws:末两段: {top}");
    // 前缀强调色、值 muted（与 prompt 底边框 ws: 一致）。
    let buffer = render_buffer(&state);
    let chars: Vec<char> = top.chars().collect();
    let start = chars
        .windows(3)
        .position(|window| window == ['w', 's', ':'])
        .expect("ws 前缀");
    let prefix = &buffer[(start as u16, rect.y)];
    assert_eq!(prefix.symbol(), "w");
    assert_eq!(prefix.fg, ratatui::style::Color::Cyan, "前缀强调色");
    let value = &buffer[(start as u16 + 3, rect.y)];
    assert_eq!(value.symbol(), "d");
    assert!(
        value.modifier.contains(ratatui::style::Modifier::DIM),
        "值带 dim"
    );
    assert_eq!(
        (value.fg, value.modifier),
        (
            ratatui::style::Color::Cyan,
            ratatui::style::Modifier::BOLD | ratatui::style::Modifier::DIM
        ),
        "聚焦窗格值高亮（accent + dim，与 prompt ws: 值一致）"
    );

    // 失焦（prompt 持有键盘焦点）：值回退 muted。
    state.prompt_focused = true;
    let buffer = render_buffer(&state);
    let value = &buffer[(start as u16 + 3, rect.y)];
    assert_eq!(
        (value.fg, value.modifier),
        (ratatui::style::Color::Reset, ratatui::style::Modifier::DIM),
        "失焦窗格值回退 muted"
    );
    state.prompt_focused = false;

    // 切到终端视图不显示路径。
    update::toggle_pane_view(&mut state, pane);
    let lines = render_lines(&state);
    let top = &lines[usize::from(rect.y)];
    assert!(!top.contains("ws:"), "终端视图不显示 lx 路径: {top}");
}

#[test]
fn lx_pane_top_border_keeps_corners_with_long_path() {
    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    let config = Config::default();
    let view = view_for(&state);
    let rects = crate::layout::pane_rects(
        &state.active_tab().layout,
        view.panes,
        config.min_pane_width,
        config.min_pane_height,
    );
    let (_, rect) = rects[0];
    // 超长路径：顶边仍保留左右角字符。
    let long = format!("/very/long/{}", "x".repeat(200));
    update::update_pane_cwd(&mut state, pane, Path::new(&long), "x".into());
    let lines = render_lines(&state);
    let top: Vec<char> = lines[usize::from(rect.y)].chars().collect();
    assert_eq!(top[usize::from(rect.x)], '╭', "保留左上角");
    assert_eq!(top[usize::from(rect.right() - 1)], '╮', "保留右上角");
}

#[test]
fn pane_title_and_toggle_label_follow_view() {
    let mut state = AppState::demo();
    let config = Config::default();
    let view = view_for(&state);
    let rects = crate::layout::pane_rects(
        &state.active_tab().layout,
        view.panes,
        config.min_pane_width,
        config.min_pane_height,
    );
    let (id, rect) = rects[0];
    assert_eq!(id, state.active_tab().layout.focus());

    let button = main_content::toggle_button(rect).expect("toggle button visible");
    let lines = render_lines(&state);
    let title = format!(" lx-[{}] ", text::lx_entry_label(LxEntry::Native));
    let title_area = Rect::new(rect.x + 1, rect.y, title.chars().count() as u16, 1);
    assert_eq!(rendered_area(&lines, title_area), title);
    assert_eq!(rendered_area(&lines, button), text::LX_TOGGLE_TERMINAL);
    // 入口名保留渐变着色，`lx-[` 用标题淡蓝。
    let buffer = render_buffer(&state);
    let label = text::lx_entry_label(LxEntry::Native);
    let label_x = rect.x + 1 + " lx-[".chars().count() as u16;
    let expected =
        crate::ui::style::lx_entry_label_style(LxEntry::Native, 0, label.chars().count(), false)
            .fg
            .expect("gradient color");
    assert_eq!(buffer[(label_x, rect.y)].fg, expected, "入口名渐变着色");
    assert_ne!(
        buffer[(label_x, rect.y)].fg,
        ratatui::style::Color::Cyan,
        "入口名不改成标题色"
    );

    update::toggle_pane_view(&mut state, id);
    let lines = render_lines(&state);
    let terminal_title = format!(" pane {} ", id.raw());
    let title_area = Rect::new(rect.x + 1, rect.y, terminal_title.chars().count() as u16, 1);
    assert_eq!(rendered_area(&lines, title_area), terminal_title);
    assert_eq!(rendered_area(&lines, button), text::LX_TOGGLE_LX);
}

#[test]
fn pane_border_shows_agent_label_separated_from_toggle_button() {
    let mut state = AppState::demo();
    terminal_view(&mut state);
    let pane = state.active_tab().layout.focus();
    update::update_pane_agent(
        &mut state,
        pane,
        Some(crate::detect::PaneAgentSnapshot {
            kind: crate::detect::AgentKind::Claude,
            state: crate::detect::AgentState::Working,
            title: None,
        }),
    );
    let config = Config::default();
    let view = view_for(&state);
    let rects = crate::layout::pane_rects(
        &state.active_tab().layout,
        view.panes,
        config.min_pane_width,
        config.min_pane_height,
    );
    let (_, rect) = rects[0];
    let button = main_content::toggle_button(rect).expect("toggle button visible");
    let label = format!(" {}claude", text::PANE_AGENT_PREFIX);
    let label_width = label.chars().count() as u16;
    // 右端落在按钮左侧第 2 列，与按钮之间保留 1 列边框。
    let label_area = Rect::new(button.x - 2 - label_width + 1, rect.y, label_width, 1);
    let lines = render_lines(&state);
    assert_eq!(rendered_area(&lines, label_area), label);
    let gap = Rect::new(button.x - 1, rect.y, 1, 1);
    assert!(
        matches!(rendered_area(&lines, gap).as_str(), "─" | " "),
        "agent label must not touch the toggle button"
    );

    let name_col = label_area.x + 1 + text::PANE_AGENT_PREFIX.len() as u16;
    let buffer = render_buffer(&state);
    let prefix = &buffer[(label_area.x + 1, label_area.y)];
    assert_eq!(
        prefix.fg,
        ratatui::style::Color::Cyan,
        "agent prefix uses accent color like the prompt ws: label"
    );
    assert_eq!(prefix.modifier, ratatui::style::Modifier::BOLD);
    let name = &buffer[(name_col, label_area.y)];
    assert_eq!(
        (name.fg, name.modifier),
        (
            ratatui::style::Color::Cyan,
            ratatui::style::Modifier::BOLD | ratatui::style::Modifier::DIM
        ),
        "focused pane matches the prompt ws: value style (accent + dim)"
    );

    // 窗格失焦（prompt 持有键盘焦点）：值回退 muted。
    state.prompt_focused = true;
    let buffer = render_buffer(&state);
    let name = &buffer[(name_col, label_area.y)];
    assert_eq!(
        (name.fg, name.modifier),
        (ratatui::style::Color::Reset, ratatui::style::Modifier::DIM),
        "unfocused pane keeps the agent name muted"
    );
}

#[test]
fn pane_border_agent_label_absent_without_agent() {
    let mut state = AppState::demo();
    terminal_view(&mut state);
    let lines = render_lines(&state);
    assert!(
        !lines[0].contains(text::PANE_AGENT_PREFIX),
        "no agent means no border label: {:?}",
        lines[0]
    );
}

/// 浮层顶边框标题单元格：淡蓝色（Cyan + dim）、非粗体，面板底色保留。
fn assert_overlay_title(buffer: &ratatui::buffer::Buffer, area: Rect) {
    let cell = &buffer[(area.x + 1, area.y)];
    assert!(
        cell.modifier.contains(ratatui::style::Modifier::DIM),
        "边框标题必须 dim: {cell:?}"
    );
    assert_eq!(cell.fg, ratatui::style::Color::Cyan);
    assert_eq!(cell.bg, ratatui::style::Color::Indexed(236));
    assert!(!cell.modifier.contains(ratatui::style::Modifier::BOLD));
}

#[test]
fn focused_pane_title_uses_soft_blue_style() {
    let state = AppState::demo();
    let config = Config::default();
    let view = view_for(&state);
    let rects = crate::layout::pane_rects(
        &state.active_tab().layout,
        view.panes,
        config.min_pane_width,
        config.min_pane_height,
    );
    let (id, rect) = rects[0];
    assert_eq!(id, state.active_tab().layout.focus(), "唯一窗格持有焦点");
    let buffer = render_buffer(&state);
    for x in rect.x + 1..rect.x + 5 {
        let cell = &buffer[(x, rect.y)];
        assert!(
            cell.modifier.contains(ratatui::style::Modifier::DIM),
            "边框标题为淡蓝色（Cyan + dim）"
        );
        assert!(!cell.modifier.contains(ratatui::style::Modifier::BOLD));
        assert_eq!(cell.fg, ratatui::style::Color::Cyan);
    }
    // 焦点仍由边框颜色区分。
    assert_eq!(
        buffer[(rect.x, rect.y + 1)].fg,
        ratatui::style::Color::Cyan,
        "焦点窗格边框保持强调色"
    );
}

#[test]
fn overlay_border_titles_use_soft_blue_style() {
    let screen = Rect::new(0, 0, 100, 24);
    let config = Config::default();

    // toast 顶边框标题。
    let mut state = AppState::demo();
    update::show_toast(
        &mut state,
        Toast::new(ToastKind::Info, text::TOAST_COPIED, None, Instant::now())
            .with_title("Clipboard"),
    );
    let view = view_for(&state);
    let rects = crate::layout::pane_rects(
        &state.active_tab().layout,
        view.panes,
        config.min_pane_width,
        config.min_pane_height,
    );
    let area = toast::rect(&state, &view, &rects, screen, &config).expect("toast visible");
    let buffer = render_buffer(&state);
    assert_overlay_title(&buffer, area);

    // 右键菜单顶边框标题。
    let mut state = AppState::demo();
    update::open_workspace_menu(&mut state, 0, (5, 5));
    let Some(Overlay::Menu(menu)) = state.overlay.as_ref() else {
        panic!("menu overlay expected");
    };
    let menu_layout = overlay::menu_layout(&state, screen, menu);
    let buffer = render_buffer(&state);
    assert_overlay_title(&buffer, menu_layout.area);

    // 模态（重命名）顶边框标题。
    let Some(Overlay::Menu(menu)) = state.overlay.as_ref() else {
        panic!("menu overlay expected");
    };
    let rename_index = menu
        .commands
        .iter()
        .position(|command| *command == crate::app::overlay::MenuCommand::RenameWorkspace)
        .expect("rename offered by menu");
    update::set_menu_selection(&mut state, rename_index);
    update::activate_menu(&mut state);
    let shell = overlay::rename_shell(screen).expect("rename modal visible");
    let buffer = render_buffer(&state);
    assert_overlay_title(&buffer, shell.area);
}

/// 工作区 git 元数据（侧栏分组渲染测试用）。
fn git_info(repo_root: &str, checkout: &str, linked: bool, branch: Option<&str>) -> WorkspaceGit {
    WorkspaceGit {
        repo_root: PathBuf::from(repo_root),
        checkout_path: PathBuf::from(checkout),
        is_linked: linked,
        branch: branch.map(str::to_string),
        main_branch: branch.map(str::to_string),
    }
}

/// 追加带 git 元数据的工作区。
fn push_git_workspace(state: &mut AppState, name: &str, cwd: &str, git: WorkspaceGit) {
    let mut workspace = Workspace::single_terminal(name.to_string(), Some(PathBuf::from(cwd)));
    workspace.git = Some(git);
    state.workspaces.push(workspace);
}

#[test]
fn sidebar_renders_duplicate_main_checkout_top_level() {
    let mut state = AppState::demo();
    state.workspaces[0].name = "main".to_string();
    state.workspaces[0].git = Some(git_info("/repo", "/repo", false, Some("main")));
    push_git_workspace(
        &mut state,
        "main 2",
        "/repo",
        git_info("/repo", "/repo", false, Some("main")),
    );
    push_git_workspace(
        &mut state,
        "feat",
        "/repo/.worktrees/feat",
        git_info("/repo", "/repo/.worktrees/feat", true, Some("feature/x")),
    );

    let lines = render_lines(&state);
    assert!(lines.iter().any(|line| line.contains("▾ ⑂ main")));
    assert!(
        lines.iter().any(|line| line.contains("  ⑂ main 2")),
        "重复主 checkout 与根同列、独立顶层: {lines:?}"
    );
    assert!(
        !lines.iter().any(|line| line.contains("─ ⑂ main 2")),
        "重复主 checkout 不显示连接符: {lines:?}"
    );
    assert!(
        lines.iter().any(|line| line.contains("  └─ ⑂ feature/x")),
        "linked worktree 仍是子项: {lines:?}"
    );
}

#[test]
fn sidebar_renders_duplicate_child_index_before_initial_marker() {
    let mut state = AppState::demo();
    state.workspaces[0].name = "main".to_string();
    state.workspaces[0].git = Some(git_info("/repo", "/repo", false, Some("main")));
    push_git_workspace(
        &mut state,
        "feat",
        "/repo/.worktrees/feat",
        git_info("/repo", "/repo/.worktrees/feat", true, Some("feature/x")),
    );
    push_git_workspace(
        &mut state,
        "feat2",
        "/repo/.worktrees/feat2",
        git_info(
            "/repo",
            "/repo/.worktrees/feat2",
            true,
            Some("worktree/feature/x"),
        ),
    );
    state.workspaces[2].is_initial = true;

    let lines = render_lines(&state);
    assert!(
        lines.iter().any(|line| line.contains("  ├─ ⑂ feature/x")),
        "first duplicate keeps clean label: {lines:?}"
    );
    let row_index = lines
        .iter()
        .position(|line| line.contains("  └─ ⑂ feature/x 2 *"))
        .unwrap_or_else(|| panic!("second duplicate gets index before marker: {lines:?}"));

    // `*` 与去重序号用强调色标识，hover 时前景不被覆盖（只加 hover 背景）。
    let sidebar_width = usize::from(view_for(&state).sidebar.width);
    let sidebar_line: String = lines[row_index].chars().take(sidebar_width).collect();
    let anchor = sidebar_line.find(" 2 *").expect("index + initial marker");
    let index_x = sidebar_line[..anchor].chars().count() + 1;
    for hover in [None, Some(2)] {
        state.workspace_hover = hover;
        let buffer = render_buffer(&state);
        for (offset, symbol) in [(0usize, "2"), (2usize, "*")] {
            let cell = &buffer[((index_x + offset) as u16, row_index as u16)];
            assert_eq!(cell.symbol(), symbol, "hover={hover:?}");
            assert_eq!(
                cell.fg,
                ratatui::style::Color::Cyan,
                "{symbol} 使用强调色 (hover={hover:?})"
            );
            assert!(cell.modifier.contains(ratatui::style::Modifier::BOLD));
        }
    }
    state.workspace_hover = None;
}

#[test]
fn sidebar_renders_tree_connectors_for_grouped_linked_worktrees() {
    let mut state = AppState::demo();
    state.workspaces[0].name = "main".to_string();
    state.workspaces[0].git = Some(git_info("/repo", "/repo", false, Some("main")));
    push_git_workspace(
        &mut state,
        "feat",
        "/repo/.worktrees/feat",
        git_info("/repo", "/repo/.worktrees/feat", true, Some("feature/x")),
    );
    push_git_workspace(
        &mut state,
        "notes",
        "/repo/.worktrees/notes",
        git_info(
            "/repo",
            "/repo/.worktrees/notes",
            true,
            Some("worktree/notes"),
        ),
    );
    state.workspaces[2].name_is_manual = true;
    state
        .workspaces
        .push(Workspace::single_terminal("tmp".to_string(), None));

    let lines = render_lines(&state);
    assert!(lines.iter().any(|line| line.contains("▾ ⑂ main")));
    assert!(
        lines.iter().any(|line| line.contains("  ├─ ⑂ feature/x")),
        "non-last child indents and uses middle connector: {lines:?}"
    );
    assert!(
        lines.iter().any(|line| line.contains("  └─ ⑂ notes")),
        "last child uses last connector and manual name wins: {lines:?}"
    );
    let parent_line = lines
        .iter()
        .find(|line| line.contains(&format!("{} main", text::WORKSPACE_GIT_ICON)))
        .expect("parent row");
    let child_line = lines
        .iter()
        .find(|line| line.contains("feature/x"))
        .expect("child row");
    let plain_line = lines
        .iter()
        .find(|line| line.contains(&format!("{} tmp", text::WORKSPACE_NON_GIT_ICON)))
        .expect("plain row");
    let char_col =
        |line: &str, needle: &str| line.find(needle).map(|byte| line[..byte].chars().count());
    let parent_col = char_col(parent_line, "main");
    let child_col = char_col(child_line, "feature/x");
    let connector_col = char_col(child_line, "├");
    let parent_icon_col = char_col(parent_line, text::WORKSPACE_GIT_ICON);
    let plain_icon_col = char_col(plain_line, text::WORKSPACE_NON_GIT_ICON);
    assert_eq!(parent_icon_col, connector_col, "父项图标与子项直角符号对齐");
    assert_eq!(
        plain_icon_col, connector_col,
        "普通项图标与子项直角符号对齐"
    );
    assert_eq!(
        child_col,
        parent_col.map(|column| column + sidebar::WORKSPACE_ITEM_INDENT + 1),
        "子项名字在连接符与图标之后"
    );
    assert_eq!(
        char_col(plain_line, "tmp"),
        parent_col,
        "普通项与父项名字同列"
    );
}

#[test]
fn sidebar_keeps_linked_only_worktrees_flat() {
    let mut state = AppState::demo();
    state.workspaces[0].name = "one".to_string();
    state.workspaces[0].git = Some(git_info(
        "/repo",
        "/repo/.worktrees/one",
        true,
        Some("feature/one"),
    ));
    push_git_workspace(
        &mut state,
        "two",
        "/repo/.worktrees/two",
        git_info("/repo", "/repo/.worktrees/two", true, Some("feature/two")),
    );

    let view = view_for(&state);
    let lines = render_lines(&state);
    // 底边框状态栏会显示分支名，断言只作用于侧栏区域。
    let sidebar: String = lines
        .iter()
        .map(|line| {
            line.chars()
                .skip(view.sidebar.x as usize)
                .take(view.sidebar.width as usize)
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(sidebar.contains("one"));
    assert!(sidebar.contains("two"));
    assert!(!sidebar.contains("feature/one"));
    assert!(!sidebar.contains("feature/two"));
}

#[test]
fn sidebar_keeps_single_git_workspace_flat() {
    let mut state = AppState::demo();
    state.workspaces[0].name = "solo".to_string();
    state.workspaces[0].git = Some(git_info("/repo", "/repo", false, Some("main")));

    let view = view_for(&state);
    let lines = render_lines(&state);
    // 底边框状态栏会显示分支名，断言只作用于侧栏区域。
    let sidebar: String = lines
        .iter()
        .map(|line| {
            line.chars()
                .skip(view.sidebar.x as usize)
                .take(view.sidebar.width as usize)
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(sidebar.contains("solo"));
    assert!(!sidebar.contains("main"));
}

/// worktree 对话框条目。
fn dialog_entry(
    path: &str,
    branch: Option<&str>,
    linked: bool,
    open: Option<usize>,
) -> WorktreeOpenEntry {
    WorktreeOpenEntry {
        path: PathBuf::from(path),
        branch: branch.map(str::to_string),
        is_bare: false,
        is_linked: linked,
        already_open: open,
    }
}

/// 构造对话框浮层（loading 已结束）。
fn worktree_open_dialog(entries: Vec<WorktreeOpenEntry>, selected: usize) -> WorktreeOpen {
    WorktreeOpen {
        source: 0,
        repo_root: PathBuf::from("/repo"),
        entries,
        selected,
        query: TextInput::new(""),
        loading: false,
        failed: false,
    }
}

#[test]
fn worktree_dialog_renders_search_entries_status_and_buttons() {
    let mut state = AppState::demo();
    state.workspaces[0].git = Some(git_info("/repo", "/repo", false, Some("main")));
    state.overlay = Some(Overlay::WorktreeOpen(worktree_open_dialog(
        vec![
            dialog_entry("/repo", Some("main"), false, Some(0)),
            dialog_entry("/repo/.worktrees/feat", Some("feature/x"), true, None),
        ],
        0,
    )));

    let lines = render_lines(&state);
    let joined = lines.join("\n");
    assert!(joined.contains(text::WORKTREE_OPEN_TITLE));
    assert!(joined.contains(text::WORKTREE_OPEN_FILTER));
    assert!(joined.contains("feature/x"));
    assert!(joined.contains(text::WORKTREE_STATUS_OPEN));
    assert!(joined.contains("/repo/.worktrees/feat"));
    assert!(joined.contains(text::BUTTON_OPEN));
    assert!(joined.contains(text::BUTTON_CANCEL));
}

#[test]
fn worktree_dialog_renders_loading_and_empty_states() {
    let mut state = AppState::demo();
    let mut dialog = worktree_open_dialog(Vec::new(), 0);
    dialog.loading = true;
    state.overlay = Some(Overlay::WorktreeOpen(dialog));
    let lines = render_lines(&state);
    assert!(lines.join("\n").contains(text::WORKTREE_OPEN_LOADING));

    let mut dialog = worktree_open_dialog(Vec::new(), 0);
    dialog.failed = true;
    state.overlay = Some(Overlay::WorktreeOpen(dialog));
    let lines = render_lines(&state);
    assert!(lines.join("\n").contains(text::WORKTREE_OPEN_FAILED));

    state.overlay = Some(Overlay::WorktreeOpen(worktree_open_dialog(Vec::new(), 0)));
    let lines = render_lines(&state);
    assert!(lines.join("\n").contains(text::WORKTREE_OPEN_EMPTY));
}

#[test]
fn worktree_dialog_hit_testing_maps_rows_and_buttons() {
    let screen = Rect::new(0, 0, 100, 24);
    let dialog = worktree_open_dialog(
        vec![
            dialog_entry("/repo", Some("main"), false, Some(0)),
            dialog_entry("/repo/.worktrees/feat", Some("feature/x"), true, None),
            dialog_entry("/repo/.worktrees/notes", None, true, None),
        ],
        0,
    );
    let shell = overlay::worktree_dialog_shell(screen, dialog.entries.len()).expect("shell fits");
    let first_row = shell.inner.y + 2;
    assert_eq!(
        overlay::worktree_dialog_entry_at(&shell, &dialog, shell.inner.x + 1, first_row),
        Some(0)
    );
    assert_eq!(
        overlay::worktree_dialog_entry_at(&shell, &dialog, shell.inner.x + 1, first_row + 2),
        Some(1)
    );
    assert_eq!(
        overlay::worktree_dialog_entry_at(&shell, &dialog, shell.inner.x + 1, first_row + 2 * 3),
        None,
        "rows below the list do not hit entries"
    );
    assert_eq!(
        overlay::worktree_dialog_entry_at(
            &shell,
            &dialog,
            shell.inner.x.saturating_sub(1),
            first_row
        ),
        None
    );

    let open_button = widgets::modal::button_row(
        shell.inner,
        &[text::BUTTON_OPEN, text::BUTTON_CANCEL],
        2,
        shell.inner.height.saturating_sub(1),
    );
    assert_eq!(
        overlay::worktree_dialog_button_at(&shell, open_button[0].x, open_button[0].y),
        Some(overlay::WorktreeDialogButton::Open)
    );
    assert_eq!(
        overlay::worktree_dialog_button_at(&shell, open_button[1].x, open_button[1].y),
        Some(overlay::WorktreeDialogButton::Cancel)
    );
    assert_eq!(
        overlay::worktree_dialog_button_at(&shell, open_button[1].right(), open_button[1].y),
        None
    );
}

#[test]
fn worktree_dialog_window_follows_selection() {
    let entries: Vec<WorktreeOpenEntry> = (0..6)
        .map(|index| {
            dialog_entry(
                &format!("/repo/.worktrees/w{index}"),
                Some(&format!("worktree/w{index}")),
                true,
                None,
            )
        })
        .collect();
    let mut dialog = worktree_open_dialog(entries, 0);
    assert_eq!(overlay::worktree_dialog_visible_start(&dialog, 2), 0);
    dialog.selected = 4;
    assert_eq!(overlay::worktree_dialog_visible_start(&dialog, 2), 3);
    dialog.selected = 5;
    assert_eq!(overlay::worktree_dialog_visible_start(&dialog, 2), 4);
}

#[test]
fn sidebar_renders_group_chevron_and_hides_collapsed_children() {
    let mut state = AppState::demo();
    state.workspaces[0].name = "main".to_string();
    state.workspaces[0].git = Some(git_info("/repo", "/repo", false, Some("main")));
    push_git_workspace(
        &mut state,
        "feat",
        "/repo/.worktrees/feat",
        git_info("/repo", "/repo/.worktrees/feat", true, Some("feature/x")),
    );
    push_git_workspace(
        &mut state,
        "notes",
        "/repo/.worktrees/notes",
        git_info("/repo", "/repo/.worktrees/notes", true, Some("notes")),
    );

    let joined = render_lines(&state).join("\n");
    assert!(joined.contains(text::WORKSPACE_GROUP_EXPANDED));
    assert!(joined.contains("  ├─ ⑂ feature/x"), "非末位子项用中连接符");
    assert!(joined.contains("  └─ ⑂ notes"));

    state.collapsed_groups.push(PathBuf::from("/repo"));
    let joined = render_lines(&state).join("\n");
    assert!(joined.contains(text::WORKSPACE_GROUP_COLLAPSED));
    assert!(!joined.contains("feature/x"));
    assert!(!joined.contains("notes"));

    state.active_workspace = 1;
    let joined = render_lines(&state).join("\n");
    assert!(joined.contains(text::WORKSPACE_GROUP_COLLAPSED));
    assert!(
        joined.contains("  └─ ⑂ feature/x"),
        "唯一可见子项用末连接符"
    );
    assert!(!joined.contains("notes"));
}

#[test]
fn workspace_group_toggle_hit_targets_parent_chevron_cell_only() {
    let mut state = AppState::demo();
    state.workspaces[0].name = "main".to_string();
    state.workspaces[0].git = Some(git_info("/repo", "/repo", false, Some("main")));
    push_git_workspace(
        &mut state,
        "feat",
        "/repo/.worktrees/feat",
        git_info("/repo", "/repo/.worktrees/feat", true, Some("feature/x")),
    );

    let view = view_for(&state);
    let list = layout::sidebar_sections(view.sidebar, state.agents_collapsed)
        .expect("sidebar sections")
        .workspaces;
    assert_eq!(
        workspace_group_toggle_at(&view, &state, list.x, list.y),
        Some(0),
        "父项箭头格命中"
    );
    assert_eq!(
        workspace_group_toggle_at(&view, &state, list.x + 1, list.y),
        None,
        "箭头之外不命中"
    );
    assert_eq!(
        workspace_group_toggle_at(&view, &state, list.x, list.y + 1),
        None,
        "子项行不命中"
    );
}

#[test]
fn workspace_item_at_maps_visible_rows_when_collapsed() {
    let mut state = AppState::demo();
    state.workspaces[0].name = "main".to_string();
    state.workspaces[0].git = Some(git_info("/repo", "/repo", false, Some("main")));
    push_git_workspace(
        &mut state,
        "feat",
        "/repo/.worktrees/feat",
        git_info("/repo", "/repo/.worktrees/feat", true, Some("feature/x")),
    );
    push_git_workspace(
        &mut state,
        "notes",
        "/repo/.worktrees/notes",
        git_info("/repo", "/repo/.worktrees/notes", true, Some("notes")),
    );
    state.collapsed_groups.push(PathBuf::from("/repo"));

    let view = view_for(&state);
    let list = layout::sidebar_sections(view.sidebar, state.agents_collapsed)
        .expect("sidebar sections")
        .workspaces;
    assert_eq!(
        workspace_item_at(&view, &state, list.x + 3, list.y),
        Some(0)
    );
    assert_eq!(
        workspace_item_at(&view, &state, list.x + 3, list.y + 1),
        None,
        "隐藏行不命中"
    );

    state.active_workspace = 2;
    assert_eq!(
        workspace_item_at(&view, &state, list.x + 3, list.y + 1),
        Some(2)
    );
}

#[test]
fn settings_button_renders_on_sidebar_bottom_and_hits() {
    let mut state = AppState::demo();
    state.sidebar_collapsed = false;
    state.settings_available = true;
    let view = view_for(&state);
    let button = settings_button(&view, &state).expect("button fits in the sidebar");
    assert_eq!(button.y, view.sidebar.bottom() - 1);
    assert_eq!(button.x, view.sidebar.x + 1);
    assert!(settings_button_at(&view, &state, button.x, button.y));
    assert!(!settings_button_at(
        &view,
        &state,
        button.x,
        button.y.saturating_sub(1)
    ));

    // 图标贴左角 1 列、宽 3 列，与右端折叠按钮同款（强调色逐格绘制）。
    let lines = render_lines(&state);
    let bottom: String = lines[button.y as usize]
        .chars()
        .skip(usize::from(view.sidebar.x))
        .take(usize::from(view.sidebar.width))
        .collect();
    assert!(bottom.contains(text::SETTINGS_LABEL), "{bottom}");
    assert_eq!(
        bottom
            .chars()
            .skip(1)
            .take(text::SETTINGS_LABEL.chars().count())
            .collect::<String>(),
        text::SETTINGS_LABEL,
        "左边距 1 列: {bottom}"
    );
    assert!(bottom.contains(text::SIDEBAR_COLLAPSE_LABEL), "{bottom}");

    // 折叠态或服务不可用时隐藏。
    state.sidebar_collapsed = true;
    let collapsed_view = view_for(&state);
    assert!(settings_button(&collapsed_view, &state).is_none());
    state.sidebar_collapsed = false;
    state.settings_available = false;
    let unavailable = view_for(&state);
    assert!(settings_button(&unavailable, &state).is_none());
}

#[test]
fn ui_gallery_button_sits_right_of_settings_and_renders() {
    let mut state = AppState::demo();
    state.sidebar_collapsed = false;
    state.settings_available = true;
    let view = view_for(&state);
    let settings = settings_button(&view, &state).expect("settings fits");
    let gallery = ui_gallery_button(&view, &state).expect("gallery button fits");
    assert_eq!(gallery.y, settings.y);
    assert_eq!(gallery.x, settings.right() + 1, "紧邻右侧隔 1 列");
    assert_eq!(gallery.width, text::UI_GALLERY_LABEL.len() as u16);
    assert!(ui_gallery_button_at(&view, &state, gallery.x, gallery.y));
    assert!(!ui_gallery_button_at(
        &view,
        &state,
        gallery.x,
        gallery.y.saturating_sub(1)
    ));

    let lines = render_lines(&state);
    let bottom: String = lines[gallery.y as usize]
        .chars()
        .skip(usize::from(view.sidebar.x))
        .take(usize::from(view.sidebar.width))
        .collect();
    assert!(bottom.contains(text::UI_GALLERY_LABEL), "{bottom}");
    assert!(bottom.contains(text::SETTINGS_LABEL), "{bottom}");
}

#[test]
fn ui_gallery_button_independent_of_settings_service() {
    let mut state = AppState::demo();
    state.sidebar_collapsed = false;
    state.settings_available = false;
    let view = view_for(&state);
    // [SET] 因服务不可用隐藏，但 [UI] 仍在其固定槽位显示。
    assert!(settings_button(&view, &state).is_none());
    let gallery = ui_gallery_button(&view, &state).expect("gallery independent of service");
    assert_eq!(
        gallery.x,
        view.sidebar.x + 1 + text::SETTINGS_LABEL.len() as u16 + 1
    );
}

#[test]
fn ui_gallery_button_hidden_when_sidebar_collapsed() {
    let mut state = AppState::demo();
    state.settings_available = true;
    state.sidebar_collapsed = true;
    let view = view_for(&state);
    assert!(ui_gallery_button(&view, &state).is_none());
}

#[test]
fn ui_gallery_button_hidden_when_sidebar_too_narrow() {
    let mut state = AppState::demo();
    state.settings_available = true;
    state.sidebar_collapsed = false;
    // 直接构造窄侧栏：展开宽度 13，恰好容纳 [SET]（含余量）但放不下 [UI]。
    let view = layout::ViewLayout {
        sidebar: Rect::new(0, 0, 13, 24),
        tab_bar: Rect::new(13, 0, 87, 1),
        panes: Rect::new(13, 1, 87, 22),
        prompt: Rect::new(60, 1, 40, 22),
        prompt_left: false,
    };
    // 侧栏过窄时优先保留 [SET]、隐藏 [UI]。
    assert!(settings_button(&view, &state).is_some());
    assert!(ui_gallery_button(&view, &state).is_none());
}

#[test]
fn slash_rows_show_source_tags_and_worktree_selector_title() {
    let mut state = AppState::demo();
    state.prompt_focused = true;
    let dir = std::env::temp_dir().join(format!("lx-tui-ui-wt-{}", std::process::id()));
    state.prompt.set_worktree_repo(Some(dir));
    state
        .prompt
        .set_text("&&& addTemplate --start\n/gitW\n&&& addTemplate --end");
    state.prompt.set_cursor_offset_for_test(
        state.prompt.text().find("/gitW").expect("slash line") + "/gitW".len(),
    );
    state.prompt.insert_char(' ');
    state.prompt.backspace();
    let panel = state.prompt.slash_panel().expect("chooser candidate");
    assert_eq!(panel.items()[0].name, "gitWorkTree");
    // 面板候选一行渲染出 [wt] 标签与 /gitWorkTree 文案，且标签只包一层方括号。
    let data = prompt_view_data(&state);
    assert!(
        data.iter()
            .any(|line| line.contains("[wt]") && line.contains("/gitWorkTree")),
        "{data:?}"
    );
    assert!(data.iter().all(|line| !line.contains("[[")), "{data:?}");
}

/// 把 prompt 面板内所有行拼接成字符串（用于面板文案断言）。
fn prompt_view_data(state: &AppState) -> Vec<String> {
    let lines = render_lines(state);
    let view = view_for(state);
    lines
        .iter()
        .map(|line| {
            line.chars()
                .skip(usize::from(view.prompt.x))
                .take(usize::from(view.prompt.width))
                .collect()
        })
        .collect()
}

#[test]
fn controls_and_prompt_frame_geometry_and_hit_tests() {
    let state = AppState::demo();
    let view = view_for(&state);

    // 1. exit button
    let exit = exit_button(&view).expect("exit button visible");
    assert!(exit_button_at(&view, exit.x, exit.y));
    assert!(!exit_button_at(&view, 0, 0));

    // 2. swap button
    let swap = swap_button(&view).expect("swap button visible");
    assert!(swap_button_at(&view, swap.x, swap.y));
    assert!(!swap_button_at(&view, 0, 0));

    // 3. collapse buttons
    let buttons = collapse_buttons(&view, false);
    assert!(!buttons.is_empty());
    let sidebar_btn = buttons
        .iter()
        .find(|b| b.target == CollapseTarget::Sidebar)
        .expect("sidebar collapse button");
    assert_eq!(
        collapse_button_at(&view, false, sidebar_btn.area.x, sidebar_btn.area.y),
        Some(CollapseTarget::Sidebar)
    );

    // 4. prompt scrollbar
    let mut scrolled_state = AppState::demo();
    for _ in 0..50 {
        scrolled_state.prompt.insert_char('a');
        scrolled_state.prompt.newline();
    }

    assert!(
        prompt_scrollbar(&view, &scrolled_state).is_some(),
        "文本超出视口时显示滚动条"
    );
    assert!(
        prompt_scrollbar(&view, &state).is_none(),
        "内容未超出时不显示滚动条"
    );
}
