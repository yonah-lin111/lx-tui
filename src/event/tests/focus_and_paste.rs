use super::test_support::*;
use super::*;

#[test]
fn terminal_paste_valid_paths_open_proposal_and_plain_text_inserts() {
    let dir = paste_temp_dir("terminal");
    let file = dir.join("note.txt");
    std::fs::write(&file, b"note").expect("file");
    let mut state = AppState::demo();
    state.set_focus(crate::app::state::FocusedContainer::Prompt);
    paste::handle_terminal_paste(&mut state, file.display().to_string());
    let panel = state.prompt.paste_panel().expect("proposal open");
    assert_eq!(panel.candidates.len(), 1);
    assert_eq!(panel.candidates[0].kind, PasteCandidateKind::File);
    // 新粘贴先取消旧提案；非路径文本弹出文本粘贴面板，默认选中首项 `Paste raw`。
    paste::handle_terminal_paste(&mut state, "/api/users".to_string());
    assert!(state.prompt.paste_panel().is_none());
    assert!(state.prompt.text_paste_panel().is_some());
    assert!(
        state
            .prompt
            .text_paste_confirm(crate::app::paste::TextPasteAction::Raw)
    );
    assert_eq!(state.prompt.text(), "/api/users");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn terminal_empty_paste_is_ignored() {
    let mut state = AppState::demo();
    state.set_focus(crate::app::state::FocusedContainer::Prompt);
    state.prompt.insert_str("keep ");
    state.prompt.paste_cancel();
    // 终端自身粘贴只承载位图时送来空内容：没有正文可插入，静默忽略。
    paste::handle_terminal_paste(&mut state, String::new());
    assert_eq!(state.prompt.text(), "keep ");
    assert!(state.prompt.paste_panel().is_none());
}

#[test]
fn terminal_paste_missing_path_falls_back_to_text() {
    let mut state = AppState::demo();
    state.set_focus(crate::app::state::FocusedContainer::Prompt);
    let missing = std::env::temp_dir().join("lx-tui-definitely-missing-paste.txt");
    let _ = std::fs::remove_file(&missing);
    paste::handle_terminal_paste(&mut state, missing.display().to_string());
    assert!(state.prompt.paste_panel().is_none());
    assert!(state.prompt.text_paste_panel().is_some());
    assert!(
        state
            .prompt
            .text_paste_confirm(crate::app::paste::TextPasteAction::Raw)
    );
    assert_eq!(state.prompt.text(), missing.display().to_string());
}

#[test]
fn terminal_paste_plain_text_does_not_arm_completion_panel() {
    let mut state = AppState::demo();
    state.set_focus(crate::app::state::FocusedContainer::Prompt);
    state
        .prompt
        .insert_str("see @[refer-file](/tmp/template.md) then ");
    // 不存在且带 shell 转义的路径：进入文本粘贴面板，确认原文插入后尾部 `a.md` 不触发补全面板。
    let pasted = "/missing/ai_agent_api\\(1\\).md a.md".to_string();
    paste::handle_terminal_paste(&mut state, pasted.clone());
    assert!(
        state
            .prompt
            .text_paste_confirm(crate::app::paste::TextPasteAction::Raw)
    );
    assert!(
        state.prompt.text().ends_with(&pasted),
        "原文插入：{:?}",
        state.prompt.text()
    );
    assert!(state.prompt.refer_panel().is_none(), "粘贴不重启补全面板");
    let before = state.prompt.text().to_string();
    // 紧跟的回车不得把粘贴文本尾部替换成候选原文。
    crate::app::update::apply_editor(&mut state, crate::app::actions::EditorCommand::Newline);
    let text = state.prompt.text();
    assert!(text.starts_with(&before), "只追加换行：{text:?}");
    assert_eq!(
        text.matches("@[refer-file]").count(),
        1,
        "未插入额外引用：{text:?}"
    );
}

#[test]
fn terminal_paste_shell_escaped_path_builds_proposal_from_real_file() {
    let dir = paste_temp_dir("escaped");
    let file = dir.join("ai_agent_api(1).md");
    std::fs::write(&file, b"x").expect("file");
    let escaped = format!("{}/ai_agent_api\\(1\\).md", dir.display());
    let mut state = AppState::demo();
    state.set_focus(crate::app::state::FocusedContainer::Prompt);
    paste::handle_terminal_paste(&mut state, escaped.clone());
    let panel = state.prompt.paste_panel().expect("转义路径打开提案");
    assert_eq!(panel.candidates[0].path, file, "候选用去转义后的真实路径");
    assert!(
        state
            .prompt
            .paste_confirm(crate::app::prompt::PasteAction::Reference)
    );
    assert_eq!(
        state.prompt.text(),
        format!("@[refer-file]({}/ai_agent_api\\(1\\).md) ", dir.display()),
        "引用按语法转义括号"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn event_input_paste_overlay_and_prompt_routing() {
    let mut state = demo_terminal();
    let config = Config::default();
    let geo = geometry(&state, &config, Rect::new(0, 0, 80, 24));
    let mut sessions = HashMap::new();
    let mut dirty = false;

    // 1. 浮层打开时：粘贴被吞掉，dirty 保持 false。
    state.overlay = Some(Overlay::ConfirmClose(crate::app::overlay::ConfirmClose {
        target: crate::app::overlay::OverlayTarget::PromptPage {
            workspace: 0,
            owner: crate::app::prompt::PromptEditing::Item(1),
            page: crate::app::prompt::PromptPageId::Saved(1),
        },
    }));

    handle_terminal_event(
        TerminalEvent::Paste("echo test".to_string()),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(!dirty, "浮层激活时粘贴被吞掉");

    // 1b. 带单行输入框的浮层：粘贴写入当前浮层，不穿透。
    state.overlay = Some(Overlay::Rename(crate::app::overlay::Rename {
        target: crate::app::overlay::RenameTarget::Workspace(0),
        input: crate::app::overlay::TextInput::new(""),
    }));
    handle_terminal_event(
        TerminalEvent::Paste("renamed".to_string()),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(dirty, "输入类浮层粘贴使画面变脏");
    assert_eq!(
        state
            .overlay
            .as_ref()
            .and_then(Overlay::input)
            .unwrap()
            .text(),
        "renamed"
    );

    // 2. prompt 聚焦时：粘贴打开文本粘贴面板，回车确认默认首项（Paste raw）写入 prompt 编辑器。
    state.overlay = None;
    state.set_focus(crate::app::state::FocusedContainer::Prompt);
    dirty = false;
    handle_terminal_event(
        TerminalEvent::Paste("hello prompt".to_string()),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(dirty, "prompt 聚焦时粘贴使画面变脏");
    assert!(state.prompt.text_paste_panel().is_some());
    assert!(
        state
            .prompt
            .text_paste_confirm(crate::app::paste::TextPasteAction::Raw)
    );
    assert!(state.prompt.text().contains("hello prompt"));
}

// ===== 单一焦点源：同一时刻只有一个容器高亮/持有光标/收键 =====

/// 点右侧容器后主区让出焦点高亮（真机缺陷：两者同时描边强调色）。
#[test]
fn focusing_right_panel_releases_pane_highlight() {
    let mut state = demo_terminal();
    let (config, geo) = toolbar_geometry(&mut state);
    assert!(crate::app::update::select_right_tab(
        &mut state,
        RightTab::Todo
    ));
    let list = crate::ui::right_panel::todo::list_rect(geo.view.prompt);
    let mut sessions = HashMap::new();
    let mut dirty = false;

    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Left), list.x + 1, list.y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );

    assert!(state.todo_focused());
    assert!(!state.panes_focused(), "主区必须让出焦点");
    assert!(!state.sidebar_focused());
    assert!(state.todo_focused() ^ state.prompt_focused() ^ state.panes_focused());
}

/// 点主区窗格后右侧容器让出焦点：不会出现"两个容器同时高亮"。
#[test]
fn focusing_pane_releases_right_panel_highlight() {
    let mut state = demo_terminal();
    let (config, geo) = toolbar_geometry(&mut state);
    assert!(crate::app::update::select_right_tab(
        &mut state,
        RightTab::Todo
    ));
    let list = crate::ui::right_panel::todo::list_rect(geo.view.prompt);
    let mut sessions = HashMap::new();
    let mut dirty = false;
    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Left), list.x + 1, list.y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(state.todo_focused());

    let inner = layout::pane_inner_rect(geo.rects[0].1);
    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            inner.x + 1,
            inner.y + 1,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );

    assert!(state.panes_focused(), "点主区后主区持焦点");
    assert!(!state.todo_focused(), "右侧容器必须让出焦点");
}

/// 点侧栏后：侧栏高亮，主区与右侧容器都不再高亮。
#[test]
fn focusing_sidebar_releases_other_containers() {
    let mut state = demo_terminal();
    let (config, geo) = toolbar_geometry(&mut state);
    assert!(crate::app::update::select_right_tab(
        &mut state,
        RightTab::Todo
    ));
    // 先让右侧容器持焦点。
    state.set_focus(crate::app::state::FocusedContainer::Todo);
    let mut sessions = HashMap::new();
    let mut dirty = false;

    // 点 Agents 分区空白（侧栏内、非条目）。
    let sections = ui::layout::sidebar_sections(geo.view.sidebar, state.agents_collapsed)
        .expect("sidebar sections visible");
    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            sections.agents.x,
            sections.agents.y + sections.agents.height.saturating_sub(1),
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );

    assert!(state.sidebar_focused(), "点侧栏空白让侧栏持焦点");
    assert!(!state.todo_focused(), "右侧容器让出焦点");
    assert!(!state.panes_focused(), "主区让出焦点");
}

/// 真实鼠标点击右侧容器与侧栏空白区时，当前激活窗格的 Agent 条目仍保留 `❯` 标记。
#[test]
fn clicking_other_containers_keeps_selected_agent_marker() {
    use crate::detect::{AgentKind, AgentState, PaneAgentSnapshot};
    use ratatui::Terminal as RatatuiTerminal;
    use ratatui::backend::TestBackend;

    let mut state = demo_terminal();
    let pane = state.active_tab().layout.focus();
    assert!(crate::app::update::update_pane_agent(
        &mut state,
        pane,
        Some(PaneAgentSnapshot {
            kind: AgentKind::Claude,
            state: AgentState::Working,
            title: None,
        }),
    ));
    let (config, geo) = toolbar_geometry(&mut state);
    let sections = ui::layout::sidebar_sections(geo.view.sidebar, state.agents_collapsed)
        .expect("sidebar sections visible");
    let mut sessions = HashMap::new();
    let mut dirty = false;

    // 1. 点击右侧 Prompt 文本框。
    let prompt_area = crate::layout::prompt_text_rect(geo.view.prompt);
    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            prompt_area.x + 1,
            prompt_area.y + 1,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(state.prompt_focused());

    let mut terminal = RatatuiTerminal::new(TestBackend::new(120, 30)).expect("backend");
    terminal
        .draw(|frame| ui::render(frame, &state, &config))
        .expect("draw");
    assert_eq!(
        terminal.backend().buffer()[(sections.agents.x, sections.agents.y)].symbol(),
        ui::text::AGENT_SELECTED_MARKER,
        "点击 Prompt 后 Agent 标记不消失"
    );

    // 2. 点击侧栏 Agents 分区下方空白处。
    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            sections.agents.x,
            sections.agents.y + sections.agents.height.saturating_sub(1),
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(state.sidebar_focused());

    terminal
        .draw(|frame| ui::render(frame, &state, &config))
        .expect("draw");
    assert_eq!(
        terminal.backend().buffer()[(sections.agents.x, sections.agents.y)].symbol(),
        ui::text::AGENT_SELECTED_MARKER,
        "点击侧栏空白后 Agent 标记不消失"
    );
}

/// 侧栏持焦点时按键全部吞掉，绝不写进 PTY 或其它输入框。
#[test]
fn sidebar_focus_swallows_all_keys() {
    use alacritty_terminal::term::TermMode;
    for key in [
        KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE),
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE),
        KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
    ] {
        let routed = crate::input::route(
            key,
            TermMode::empty(),
            crate::app::state::FocusedContainer::Sidebar,
            None,
            true,
        );
        assert!(routed.is_none(), "{key:?} 必须被吞掉，实际 {routed:?}");
    }
}

/// 焦点容器互斥由类型保证：设置任一容器后，其余三个的查询都返回 false。
#[test]
fn focused_container_is_mutually_exclusive() {
    use crate::app::state::FocusedContainer;
    let mut state = AppState::demo();
    for target in [
        FocusedContainer::Sidebar,
        FocusedContainer::Panes,
        FocusedContainer::Prompt,
        FocusedContainer::Todo,
    ] {
        state.set_focus(target);
        let flags = [
            state.sidebar_focused(),
            state.panes_focused(),
            state.prompt_focused(),
            state.todo_focused(),
        ];
        assert_eq!(flags.iter().filter(|flag| **flag).count(), 1, "{target:?}");
        assert_eq!(state.focus(), target);
    }
}

/// 侧栏持焦点时粘贴必须吞掉（与键盘路由同口径：绝不穿透到别的容器）。
#[test]
fn paste_while_sidebar_focused_is_swallowed() {
    let mut state = demo_terminal();
    assert!(crate::app::update::focus_sidebar(&mut state));
    assert!(state.sidebar_focused(), "前提：侧栏持焦点");
    let before = state.prompt.text().to_string();
    let mut sessions = HashMap::new();
    let mut dirty = false;

    super::input::handle_paste_event("secret".into(), &mut state, &mut sessions, &mut dirty);

    assert_eq!(state.prompt.text(), before, "不得写进 prompt");
    assert!(!dirty, "无变化不置脏");
}

#[test]
fn paste_panel_swallows_mouse_wheel_without_scrolling_underlying_views() {
    let dir = std::env::temp_dir().join(format!("lx-wheel-paste-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("dir");
    let file_path = dir.join("sample.rs");
    std::fs::write(&file_path, "fn main() {}\n").expect("write");
    let path_str = file_path.to_string_lossy().into_owned();

    let config = Config::default();
    let mut state = AppState::demo();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 20));
    let area = crate::layout::prompt_text_rect(geo.view.prompt);
    state.prompt.resize(area.width, area.height);
    state.set_focus(crate::app::state::FocusedContainer::Prompt);
    for i in 0..30 {
        state.prompt.insert_str(&format!("line {i}\n"));
    }
    paste::handle_terminal_paste(&mut state, path_str);
    let rect = crate::ui::prompt::paste_panel_rect(&state.prompt, area).expect("paste panel");
    let prompt_scroll_before = state.prompt.scroll();

    let mut sessions = HashMap::new();
    let mut dirty = false;
    handle_terminal_event(
        mouse(MouseEventKind::ScrollUp, rect.x + 1, rect.y + 1),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(!dirty, "粘贴面板吞掉滚轮且不置脏");
    assert_eq!(
        state.prompt.scroll(),
        prompt_scroll_before,
        "粘贴面板上滚轮不得穿透滚动底层 prompt"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn overlay_cut_event_deletes_selection_and_emits_toast() {
    let mut state = AppState::demo();
    let config = Config::default();
    let geo = geometry(&state, &config, Rect::new(0, 0, 80, 24));
    let mut sessions = HashMap::new();
    let mut dirty = false;
    state.overlay = Some(Overlay::Rename(crate::app::overlay::Rename {
        target: crate::app::overlay::RenameTarget::Workspace(0),
        input: crate::app::overlay::TextInput::new("hello world"),
    }));
    {
        let input = state.overlay.as_mut().and_then(Overlay::input_mut).unwrap();
        input.set_selection(6, 11);
    }
    handle_terminal_event(
        TerminalEvent::Key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL)),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert_eq!(
        state
            .overlay
            .as_ref()
            .and_then(Overlay::input)
            .unwrap()
            .text(),
        "hello "
    );
    assert!(state.toast.is_some(), "剪切写入剪贴板结果应有 toast");
}

#[test]
fn long_text_paste_opens_panel_in_prompt_and_lx_and_up_key_does_not_trigger_history() {
    let config = Config::default();
    let mut state = AppState::demo();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 24));
    let area = crate::layout::prompt_text_rect(geo.view.prompt);
    state.prompt.resize(area.width, area.height);

    // Seed prompt_history so we verify pressing Up in Lx paste panel never recalls history!
    state.prompt_history.record("previous history entry");

    // 1. Prompt focus: pasting 3 lines opens text paste panel without modifying buffer yet.
    state.set_focus(crate::app::state::FocusedContainer::Prompt);
    let long_text = "first line\nsecond line\nthird line".to_string();
    paste::handle_terminal_paste(&mut state, long_text.clone());
    assert!(state.prompt.text_paste_panel().is_some());
    assert_eq!(state.prompt.text(), "");

    // Select second item (`Paste text`, index 1) and press Enter to insert `<pasted-text lines="3">...</pasted-text>`.
    let mut sessions = HashMap::new();
    let mut dirty = false;
    handle_terminal_event(
        TerminalEvent::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    handle_terminal_event(
        TerminalEvent::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(state.prompt.text_paste_panel().is_none());
    assert!(state.prompt.text().starts_with("<pasted-text lines=\"3\">"));

    // 2. Lx focus: pasting 3 lines opens lx text paste panel (4 items: Raw, Text, Code, Log).
    // Pressing Up while panel is open MUST move panel highlight to index 3 (`Paste log`), NOT recall prompt_history!
    state.set_focus(crate::app::state::FocusedContainer::Panes);
    let pane_id = state.active_tab().layout.focus();
    assert!(paste::handle_lx_paste(&mut state, long_text.clone()));
    assert!(
        state
            .pane_anywhere(pane_id)
            .unwrap()
            .lx
            .text_paste_panel()
            .is_some()
    );
    update::apply_lx_input(&mut state, crate::app::actions::LxInput::Up);
    let lx_after_up = &state.pane_anywhere(pane_id).unwrap().lx;
    assert!(
        lx_after_up.text_paste_panel().is_some(),
        "Pressing Up while Paste panel is open must NOT close panel or trigger history recall"
    );
    assert_eq!(lx_after_up.input, "");
    assert_eq!(lx_after_up.text_paste_panel().unwrap().active(), 3);

    // Press Enter on index 3 (`Paste log`) to confirm `<pasted-log lines="3">`.
    update::apply_lx_input(&mut state, crate::app::actions::LxInput::Enter);
    let lx = &state.pane_anywhere(pane_id).unwrap().lx;
    assert!(lx.text_paste_panel().is_none());
    assert!(lx.input.starts_with("<pasted-log lines=\"3\">"));

    // 3. Selecting all in Prompt, pasting and confirming clears `AppState.selection`
    // so subsequent typing does not overwrite the newly inserted chip.
    state.set_focus(crate::app::state::FocusedContainer::Prompt);
    state.select_all_prompt();
    assert!(state.selection.is_some());
    paste::handle_terminal_paste(&mut state, "replaced\nby\nchip".to_string());
    update::select_paste(&mut state, 1); // Paste text
    assert!(
        state.selection.is_none(),
        "Confirming paste must clear AppState.selection"
    );
    update::apply_editor(
        &mut state,
        crate::app::actions::EditorCommand::InsertChar('Z'),
    );
    assert!(state.prompt.text().ends_with("Z"));
    assert!(state.prompt.text().starts_with("<pasted-text lines=\"3\">"));
}
