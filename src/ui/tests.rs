//! 单元测试；仅测试构建编译。

use super::*;
use crate::app::actions::Action;
use crate::app::toast::{TOAST_DURATION, Toast, ToastKind};
use crate::app::update;
use ratatui::Terminal as RatatuiTerminal;
use ratatui::backend::TestBackend;
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
        state.prompt_collapsed,
        state.prompt_width,
    )
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
    let state = AppState::demo();
    let lines = render_lines(&state);
    assert!(lines.iter().any(|line| line.contains("Drag to select")));
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
    assert!(!lines.iter().any(|line| line.contains("Drag to select")));
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
fn agents_button_aligns_with_top_button_and_toggles_header() {
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

    let sidebar_row: Vec<char> = lines[view.sidebar.y as usize].chars().collect();
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

    let prompt_row: Vec<char> = lines[view.prompt.y as usize].chars().collect();
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
    // 折叠按钮占据右栏首行，第二行起才是纯窄条。
    assert_eq!(prompt(view.prompt.y as usize + 1), "│   ");
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
    assert!(!lines.iter().any(|line| line.contains("lx-tui")));
    let last = lines.last().map(String::as_str).unwrap_or_default();
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
