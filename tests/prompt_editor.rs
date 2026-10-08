//! 集成测试：prompt markdown 输入、点击定位、滚轮滚动贯穿几何同步、状态更新与渲染。

use lx_tui::app::actions::EditorCommand;
use lx_tui::app::markdown::MentionEntry;
use lx_tui::app::state::{AppState, PaneView};
use lx_tui::app::update;
use lx_tui::config::Config;
use lx_tui::layout;
use lx_tui::ui;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Color, Modifier};

/// 构造已同步几何的 prompt 场景（100x24，右栏展开）。
fn ready_state() -> (AppState, Config, ui::layout::ViewLayout) {
    let config = Config::default();
    let mut state = AppState::demo();
    let screen = Rect::new(0, 0, 100, 24);
    let view = ui::layout::compute(
        screen,
        &config,
        state.sidebar_collapsed,
        state.sidebar_width,
        state.prompt_collapsed,
        state.prompt_width,
    );
    let mut rects = layout::pane_rects(
        &state.active_tab().layout,
        view.panes,
        config.min_pane_width,
        config.min_pane_height,
    );
    rects.push((state.prompt.id(), view.prompt));
    update::resize_panes(&mut state, &rects);
    (state, config, view)
}

fn draw(state: &AppState, config: &Config) -> Terminal<TestBackend> {
    let mut terminal =
        Terminal::new(TestBackend::new(100, 24)).expect("test backend is infallible");
    terminal
        .draw(|frame| ui::render(frame, state, config))
        .expect("draw succeeds");
    terminal
}

#[test]
fn typed_markdown_renders_highlight_and_cursor() {
    let (mut state, config, view) = ready_state();
    update::focus_prompt(&mut state);
    for ch in "# 标题".chars() {
        update::apply_editor(&mut state, EditorCommand::InsertChar(ch));
    }

    let terminal = draw(&state, &config);
    let buffer = terminal.backend().buffer();
    let inner = layout::pane_inner_rect(view.prompt);
    let marker = &buffer[(inner.x, inner.y)];
    assert_eq!(marker.symbol(), "#");
    assert!(marker.modifier.contains(Modifier::DIM));

    let heading = &buffer[(inner.x + 2, inner.y)];
    assert_eq!(heading.symbol(), "标");
    assert_eq!(heading.fg, Color::Yellow);
    assert!(heading.modifier.contains(Modifier::BOLD));

    assert!(terminal.backend().cursor_visible());
    assert_eq!(
        terminal.backend().cursor_position(),
        Position::new(inner.x + 6, inner.y)
    );
}

#[test]
fn click_places_cursor_and_wheel_scrolls_viewport() {
    let (mut state, config, view) = ready_state();
    update::focus_prompt(&mut state);
    let text = (0..30)
        .map(|i| i.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    update::apply_editor(&mut state, EditorCommand::InsertText(text));

    state.prompt.scroll_by(-100);
    update::place_prompt_cursor(&mut state, 0, 0);
    assert_eq!(state.prompt.scroll(), 0);
    let terminal = draw(&state, &config);
    let buffer = terminal.backend().buffer();
    let inner = layout::pane_inner_rect(view.prompt);
    assert_eq!(buffer[(inner.x, inner.y)].symbol(), "0");
    assert!(terminal.backend().cursor_visible());
    assert_eq!(
        terminal.backend().cursor_position(),
        Position::new(inner.x, inner.y)
    );

    update::scroll_prompt(&mut state, 1);
    assert_eq!(state.prompt.scroll(), 3);
    let terminal = draw(&state, &config);
    let buffer = terminal.backend().buffer();
    assert_eq!(buffer[(inner.x, inner.y)].symbol(), "3");
    assert!(!terminal.backend().cursor_visible());
}

#[test]
fn hardware_cursor_tracks_focused_prompt_for_ime_preedit() {
    let (mut state, config, view) = ready_state();
    update::focus_prompt(&mut state);
    update::apply_editor(&mut state, EditorCommand::InsertText("你好".into()));

    let terminal = draw(&state, &config);
    let inner = layout::pane_inner_rect(view.prompt);
    assert!(terminal.backend().cursor_visible());
    assert_eq!(
        terminal.backend().cursor_position(),
        Position::new(inner.x + 4, inner.y)
    );

    let pane = state.active_tab().layout.focus();
    update::focus_pane(&mut state, pane);
    if let Some(target) = state.active_tab_mut().pane_mut(pane) {
        target.view = PaneView::Terminal;
    }
    let terminal = draw(&state, &config);
    let rects = layout::pane_rects(
        &state.active_tab().layout,
        view.panes,
        config.min_pane_width,
        config.min_pane_height,
    );
    let rect = rects
        .iter()
        .find(|(id, _)| *id == pane)
        .map(|(_, rect)| *rect)
        .expect("focused pane is tiled");
    let inner = layout::pane_inner_rect(rect);
    let (row, col) = state
        .active_tab()
        .pane(pane)
        .expect("focused pane exists")
        .terminal
        .cursor_viewport()
        .expect("terminal cursor is visible");
    assert!(terminal.backend().cursor_visible());
    assert_eq!(
        terminal.backend().cursor_position(),
        Position::new(inner.x + col as u16, inner.y + row as u16)
    );
}

#[test]
fn backspace_deletes_sole_task_marker_without_blank_placeholder() {
    let (mut state, config, view) = ready_state();
    update::focus_prompt(&mut state);
    for ch in "- [ ] ".chars() {
        update::apply_editor(&mut state, EditorCommand::InsertChar(ch));
    }
    update::apply_editor(&mut state, EditorCommand::Backspace);
    assert_eq!(state.prompt.text(), "");

    let terminal = draw(&state, &config);
    let inner = layout::pane_inner_rect(view.prompt);
    assert!(terminal.backend().cursor_visible());
    assert_eq!(
        terminal.backend().cursor_position(),
        Position::new(inner.x, inner.y)
    );
}

#[test]
fn clicking_terminal_pane_reads_back_focus_and_keeps_editing_target() {
    let mut state = AppState::demo();
    update::focus_prompt(&mut state);
    assert!(state.prompt_focused);

    let pane = state.active_tab().layout.focus();
    update::focus_pane(&mut state, pane);
    assert!(!state.prompt_focused);
    assert_eq!(state.active_tab().layout.focus(), pane);
}

#[test]
fn panel_task_command_then_backspace_clears_marker() {
    let (mut state, _config, _view) = ready_state();
    update::focus_prompt(&mut state);
    update::apply_editor(&mut state, EditorCommand::InsertChar('-'));
    update::apply_editor(&mut state, EditorCommand::Down);
    update::apply_editor(&mut state, EditorCommand::Newline);
    assert_eq!(state.prompt.text(), "- [ ] ");
    update::apply_editor(&mut state, EditorCommand::Backspace);
    assert_eq!(state.prompt.text(), "");
    assert_eq!(state.prompt.cursor_cell(), Some((0, 0)));
}

#[test]
fn ctrl_u_joins_line_at_line_start() {
    let (mut state, _config, _view) = ready_state();
    update::focus_prompt(&mut state);
    update::apply_editor(&mut state, EditorCommand::InsertText("ab\ncd".into()));
    update::apply_editor(&mut state, EditorCommand::LineStart);
    update::apply_editor(&mut state, EditorCommand::DeleteToLineStart);
    assert_eq!(state.prompt.text(), "abcd");
}

#[test]
fn mention_panel_renders_and_confirms_insertion() {
    let (mut state, config, view) = ready_state();
    update::focus_prompt(&mut state);
    update::apply_editor(&mut state, EditorCommand::InsertChar('@'));
    let (generation, _) = state
        .prompt
        .take_mention_scan_request()
        .expect("scan requested");
    update::apply_mention_entries(
        &mut state,
        generation,
        vec![MentionEntry {
            path: "src/app.rs".into(),
            is_directory: false,
        }],
    );

    let terminal = draw(&state, &config);
    let buffer = terminal.backend().buffer();
    let inner = layout::pane_inner_rect(view.prompt);
    let name_row: String = (inner.x..inner.x + inner.width)
        .map(|x| buffer[(x, inner.y + 2)].symbol())
        .collect();
    let detail_row: String = (inner.x..inner.x + inner.width)
        .map(|x| buffer[(x, inner.y + 3)].symbol())
        .collect();
    assert!(name_row.contains("app.rs"));
    assert!(name_row.contains(ui::text::MENTION_FILE_ICON), "{name_row}");
    assert!(detail_row.contains("src"));

    update::apply_editor(&mut state, EditorCommand::Newline);
    assert_eq!(state.prompt.text(), "@src/app.rs ");
}

#[test]
fn mention_panel_wheel_scrolls_viewport_and_keeps_active() {
    let (mut state, config, view) = ready_state();
    update::focus_prompt(&mut state);
    update::apply_editor(&mut state, EditorCommand::InsertChar('@'));
    let (generation, _) = state
        .prompt
        .take_mention_scan_request()
        .expect("scan requested");
    let entries: Vec<MentionEntry> = (0..8)
        .map(|index| MentionEntry {
            path: format!("src/f{index}.rs"),
            is_directory: false,
        })
        .collect();
    update::apply_mention_entries(&mut state, generation, entries);

    let inner = layout::pane_inner_rect(view.prompt);
    let before = ui::prompt::mention_layout(&state.prompt, inner).expect("panel visible");
    assert!(update::scroll_mention(&mut state, 1, before.start));
    let after = ui::prompt::mention_layout(&state.prompt, inner).expect("panel visible");
    assert_eq!(after.start, before.start + 1, "滚轮下移视口一项");
    assert_eq!(
        state.prompt.mention().map(|panel| panel.active()),
        Some(0),
        "滚轮不动高亮"
    );

    let terminal = draw(&state, &config);
    let buffer = terminal.backend().buffer();
    let first_row: String = (inner.x..inner.x + inner.width)
        .map(|x| buffer[(x, inner.y + 2)].symbol())
        .collect();
    assert!(first_row.contains("f1.rs"), "视口应下移一项：{first_row}");
    assert!(!first_row.contains("f0.rs"), "{first_row}");
}

#[test]
fn backspace_removes_whole_mention_after_insertion() {
    let (mut state, _config, _view) = ready_state();
    update::focus_prompt(&mut state);
    update::apply_editor(&mut state, EditorCommand::InsertText("@src/app.rs ".into()));
    update::apply_editor(&mut state, EditorCommand::Backspace);
    assert_eq!(state.prompt.text(), "");
}

#[test]
fn mention_panel_folder_navigation_flow() {
    let (mut state, config, view) = ready_state();
    update::focus_prompt(&mut state);
    update::apply_editor(&mut state, EditorCommand::InsertChar('@'));
    let (generation, _) = state
        .prompt
        .take_mention_scan_request()
        .expect("scan requested");
    update::apply_mention_entries(
        &mut state,
        generation,
        vec![
            MentionEntry {
                path: "docs/readme.md".into(),
                is_directory: false,
            },
            MentionEntry {
                path: "src".into(),
                is_directory: true,
            },
            MentionEntry {
                path: "src/app.rs".into(),
                is_directory: false,
            },
        ],
    );

    // 高亮 src 目录进入：文本写回 `@src/`，面板按范围过滤。
    assert!(state.prompt.mention_set_active(1));
    update::apply_editor(&mut state, EditorCommand::EnterFolder);
    assert_eq!(state.prompt.text(), "@src/");
    assert_eq!(
        state.prompt.mention().map(|panel| panel.scope_name()),
        Some(Some("src"))
    );

    // 顶边左 title、右上目录名、底边快捷键完整渲染。
    let inner = layout::pane_inner_rect(view.prompt);
    let panel = ui::prompt::mention_layout(&state.prompt, inner).expect("panel visible");
    let terminal = draw(&state, &config);
    let buffer = terminal.backend().buffer();
    let top: String = (panel.rect.x..panel.rect.right())
        .map(|x| buffer[(x, panel.rect.y)].symbol())
        .collect();
    let bottom: String = (panel.rect.x..panel.rect.right())
        .map(|x| buffer[(x, panel.rect.bottom() - 1)].symbol())
        .collect();
    assert!(top.contains(ui::text::MENTION_PANEL_TITLE), "{top}");
    assert!(top.contains(" src "), "{top}");
    assert!(bottom.contains(ui::text::MENTION_PANEL_FOOTER), "{bottom}");

    // Ctrl/Cmd+Z 撤销进入，回退到根，候选恢复全量。
    update::apply_editor(&mut state, EditorCommand::Undo);
    assert_eq!(state.prompt.text(), "@");
    assert_eq!(
        state.prompt.mention().map(|panel| panel.items().len()),
        Some(3)
    );
}
