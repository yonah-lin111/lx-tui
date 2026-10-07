//! 单元测试；仅测试构建编译。

use super::*;
use crate::app::actions::Action;
use crate::app::overlay::{Overlay, TextInput, WorktreeOpen, WorktreeOpenEntry};
use crate::app::state::WorkspaceGit;
use crate::app::toast::{TOAST_DURATION, Toast, ToastKind};
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

fn view_for(state: &AppState) -> layout::ViewLayout {
    layout::compute(
        Rect::new(0, 0, 100, 24),
        &Config::default(),
        state.sidebar_collapsed,
        state.sidebar_width,
        state.prompt_collapsed,
        state.prompt_width,
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
        text::PROMPT_COLLAPSE_LABEL
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
        text::PROMPT_COLLAPSE_LABEL
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
        text::PROMPT_EXPAND_LABEL
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
            .any(|line| line.contains(text::PROMPT_COLLAPSE_LABEL))
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
        text::PROMPT_EXPAND_LABEL
    );
    assert!(
        !lines
            .iter()
            .any(|line| line.contains(text::PROMPT_COLLAPSE_LABEL))
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
    assert!(
        lines
            .iter()
            .any(|line| line.contains(&state.active_workspace().name))
    );
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
fn agents_section_below_header_stays_empty() {
    let state = AppState::demo();
    let view = view_for(&state);
    let sections = layout::sidebar_sections(view.sidebar, false).expect("sections are visible");
    let lines = render_lines(&state);
    for line in &lines[sections.agents.y as usize..sections.agents.bottom() as usize] {
        let row: String = line
            .chars()
            .skip(view.sidebar.x as usize)
            .take(view.sidebar.width as usize)
            .collect();
        let content = row.trim_matches(|symbol| symbol == '│' || symbol == ' ');
        assert!(
            content.is_empty(),
            "agents section should stay empty: {row:?}"
        );
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
    assert_eq!(prompt_label, text::PROMPT_EXPAND_LABEL);
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
    // 工作区名可能合法等于应用名，这里只校验末行不再是状态栏。
    assert!(
        !last.contains("lx-tui"),
        "last row should not be a status bar"
    );
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
    assert!(lines.iter().any(|line| line.contains("Clipboard")));
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
fn initial_workspace_renders_green_marker() {
    let mut state = AppState::demo();
    update::create_workspace(&mut state);
    let initial = state.workspaces[0].name.clone();
    let created = state.workspaces[1].name.clone();
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
    assert_eq!(
        buffer[(marker_x as u16, marker_line as u16)].fg,
        ratatui::style::Color::Green
    );
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
fn dragged_workspace_item_renders_reversed() {
    let mut state = AppState::demo();
    update::create_workspace(&mut state);
    state.workspace_drag = Some(0);
    let config = Config::default();
    let mut terminal =
        RatatuiTerminal::new(TestBackend::new(100, 24)).expect("test backend is infallible");
    if let Err(error) = terminal.draw(|frame| render(frame, &state, &config)) {
        panic!("draw failed: {error}");
    }
    let buffer = terminal.backend().buffer().clone();
    let view = view_for(&state);
    let sections = layout::sidebar_sections(view.sidebar, false).expect("sections are visible");
    assert!(
        buffer[(sections.workspaces.x, sections.workspaces.y)]
            .modifier
            .contains(ratatui::style::Modifier::REVERSED)
    );
    assert!(
        !buffer[(sections.workspaces.x, sections.workspaces.y + 1)]
            .modifier
            .contains(ratatui::style::Modifier::REVERSED)
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
    let mut reversed = |state: &AppState| {
        if let Err(error) = terminal.draw(|frame| render(frame, state, &config)) {
            panic!("draw failed: {error}");
        }
        let buffer = terminal.backend().buffer().clone();
        [
            buffer[(sections.workspaces.x, sections.workspaces.y)]
                .modifier
                .contains(ratatui::style::Modifier::REVERSED),
            buffer[(sections.workspaces.x, sections.workspaces.y + 1)]
                .modifier
                .contains(ratatui::style::Modifier::REVERSED),
        ]
    };

    assert_eq!(reversed(&state), [true, false], "按下未移动只反显被按行");
    state.workspace_dragging = true;
    assert_eq!(reversed(&state), [true, true], "指针移动后整块反显");
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
    assert_eq!(rendered_label(&lines, &prompt), text::PROMPT_COLLAPSE_LABEL);
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
        text::PROMPT_EXPAND_LABEL
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
    assert_eq!(bar.total, state.prompt.visual_rows().len());
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
fn tab_bar_renders_auto_titles_and_add_button() {
    let state = AppState::demo();
    let lines = render_lines(&state);
    assert!(lines[0].contains("tab 1"), "{}", lines[0]);
    assert!(!lines[0].contains("logs"));
    assert!(lines[0].contains(text::ADD_TAB_LABEL), "{}", lines[0]);
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
    let title_area = Rect::new(rect.x + 1, rect.y, 4, 1);
    assert_eq!(rendered_area(&lines, title_area), " lx ");
    assert_eq!(rendered_area(&lines, button), text::LX_TOGGLE_TERMINAL);

    update::toggle_pane_view(&mut state, id);
    let lines = render_lines(&state);
    let terminal_title = format!(" pane {} ", id.raw());
    let title_area = Rect::new(rect.x + 1, rect.y, terminal_title.chars().count() as u16, 1);
    assert_eq!(rendered_area(&lines, title_area), terminal_title);
    assert_eq!(rendered_area(&lines, button), text::LX_TOGGLE_LX);
}

/// 工作区 git 元数据（侧栏分组渲染测试用）。
fn git_info(repo_root: &str, checkout: &str, linked: bool, branch: Option<&str>) -> WorkspaceGit {
    WorkspaceGit {
        repo_root: PathBuf::from(repo_root),
        checkout_path: PathBuf::from(checkout),
        is_linked: linked,
        branch: branch.map(str::to_string),
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
    assert!(lines.iter().any(|line| line.contains("▾ main")));
    assert!(
        lines.iter().any(|line| line.contains("  main 2")),
        "重复主 checkout 与根同列、独立顶层: {lines:?}"
    );
    assert!(
        !lines.iter().any(|line| line.contains("─ main 2")),
        "重复主 checkout 不显示连接符: {lines:?}"
    );
    assert!(
        lines.iter().any(|line| line.contains("  └─ feature/x")),
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
        lines.iter().any(|line| line.contains("  ├─ feature/x")),
        "first duplicate keeps clean label: {lines:?}"
    );
    assert!(
        lines.iter().any(|line| line.contains("  └─ feature/x 2 *")),
        "second duplicate gets index before marker: {lines:?}"
    );
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
    assert!(lines.iter().any(|line| line.contains("▾ main")));
    assert!(
        lines.iter().any(|line| line.contains("  ├─ feature/x")),
        "non-last child indents and uses middle connector: {lines:?}"
    );
    assert!(
        lines.iter().any(|line| line.contains("  └─ notes")),
        "last child uses last connector and manual name wins: {lines:?}"
    );
    let parent_line = lines
        .iter()
        .find(|line| line.contains("main"))
        .expect("parent row");
    let child_line = lines
        .iter()
        .find(|line| line.contains("feature/x"))
        .expect("child row");
    let plain_line = lines
        .iter()
        .find(|line| line.contains("tmp"))
        .expect("plain row");
    let char_col =
        |line: &str, needle: &str| line.find(needle).map(|byte| line[..byte].chars().count());
    let parent_col = char_col(parent_line, "main");
    let child_col = char_col(child_line, "feature/x");
    let connector_col = char_col(child_line, "├");
    assert_eq!(parent_col, connector_col, "父项首字母与子项直角符号对齐");
    assert_eq!(
        child_col,
        parent_col.map(|column| column + WORKSPACE_ITEM_INDENT + 1),
        "子项名字在连接符之后"
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

    let lines = render_lines(&state);
    assert!(lines.iter().any(|line| line.contains("one")));
    assert!(lines.iter().any(|line| line.contains("two")));
    assert!(!lines.iter().any(|line| line.contains("feature/one")));
    assert!(!lines.iter().any(|line| line.contains("feature/two")));
}

#[test]
fn sidebar_keeps_single_git_workspace_flat() {
    let mut state = AppState::demo();
    state.workspaces[0].name = "solo".to_string();
    state.workspaces[0].git = Some(git_info("/repo", "/repo", false, Some("main")));

    let lines = render_lines(&state);
    assert!(lines.iter().any(|line| line.contains("solo")));
    assert!(!lines.iter().any(|line| line.contains("main")));
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
    assert!(joined.contains("  ├─ feature/x"), "非末位子项用中连接符");
    assert!(joined.contains("  └─ notes"));

    state.collapsed_groups.push(PathBuf::from("/repo"));
    let joined = render_lines(&state).join("\n");
    assert!(joined.contains(text::WORKSPACE_GROUP_COLLAPSED));
    assert!(!joined.contains("feature/x"));
    assert!(!joined.contains("notes"));

    state.active_workspace = 1;
    let joined = render_lines(&state).join("\n");
    assert!(joined.contains(text::WORKSPACE_GROUP_COLLAPSED));
    assert!(joined.contains("  └─ feature/x"), "唯一可见子项用末连接符");
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
