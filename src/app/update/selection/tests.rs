use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::app::actions::{EditorCommand, LxInput, OverlayKey};
use crate::app::overlay::{
    NewNode, NewNodeKind, NewWorkspace, Overlay, ReferencedFolderInput, ReferencedFolderSearch,
    Rename, RenameTarget, ResumeSessionEdit, SaveDraft, SaveDraftEntry, TextInput, WorktreeOpen,
};
use crate::app::paste::encode_pasted_text;
use crate::app::selection::{Selection, SelectionUnit};
use crate::app::state::lx::chat::RequestKey;
use crate::app::state::lx::{AskState, TaskPage, TaskTab};
use crate::app::state::{AppState, AutoscrollDirection, ConnectPanel, PaneView, RightTab};
use crate::app::tooltip::TodoButton;
use crate::app::update::{
    apply_editor, apply_lx_input, apply_overlay_key, apply_page_search, apply_prompt_search,
    apply_todo_button, apply_todo_key, apply_workspace_history_key, begin_lx_input_selection,
    begin_lx_selection_range, begin_terminal_selection_with_clicks, drag_lx_input_selection,
    drag_terminal_selection, escape_clear_selection, has_visible_selection, prompt_pages_mode,
    prompt_selection_bounds, prompt_tree_mode, scroll_lx_input, set_lx_scroll, show_prompt_pages,
    show_prompt_tree, show_workspace_history, take_lx_input_selection, terminal_selection_text,
    workspace_history_mode,
};
use crate::chat::tools::AskQuestion;
use crate::config::ai::connect::ConnectSettings;
use crate::input::TodoKey;

#[test]
fn terminal_selection_esc_clears_first_and_empty_selection_does_not_consume() {
    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.view = PaneView::Terminal;
        target.terminal.feed(b"hello terminal world\r\n");
    }

    // 单击未拖出非空文本：不消费 Esc，但清理拖拽标记。
    begin_terminal_selection_with_clicks(&mut state, pane, 0, 0, 1);
    assert!(!has_visible_selection(&state));
    assert!(!escape_clear_selection(&mut state));
    assert!(state.terminal_selection.is_none());

    // 拖出非空选区：第一次 Esc 清除选区并消费按键，第二次 Esc 返回 false。
    begin_terminal_selection_with_clicks(&mut state, pane, 0, 0, 1);
    drag_terminal_selection(&mut state, pane, 0, 4);
    assert_eq!(
        terminal_selection_text(&state, pane).as_deref(),
        Some("hello")
    );
    assert!(has_visible_selection(&state));
    assert!(escape_clear_selection(&mut state));
    assert!(terminal_selection_text(&state, pane).is_none());
    assert!(state.terminal_selection.is_none());
    assert!(!escape_clear_selection(&mut state));
}

#[test]
fn lx_selections_take_priority_over_shell_exit_panels_and_expanded_pastes() {
    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();

    // 1. 空 Shell 模式 + 主内容区选区：第一下 Esc 仅清主内容选区，第二下 Esc 才退出 Shell 模式。
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.lx.insert_char('!');
        assert!(target.lx.shell_mode());
    }
    begin_lx_selection_range(&mut state, pane, (0, 0), (0, 5), SelectionUnit::Word);
    assert!(has_visible_selection(&state));
    apply_lx_input(&mut state, LxInput::Escape);
    assert!(state.lx_selection.is_none(), "第一下 Esc 先清除主内容选区");
    assert!(
        state.pane_anywhere(pane).expect("pane").lx.shell_mode(),
        "清除主内容选区时不得连带退出空 Shell 模式"
    );
    apply_lx_input(&mut state, LxInput::Escape);
    assert!(
        !state.pane_anywhere(pane).expect("pane").lx.shell_mode(),
        "无选区后第二下 Esc 退出空 Shell 模式"
    );

    // 2. 展开粘贴芯片 + 预览选区/输入框选区：严格分两步（先清选区，再收起展开芯片）。
    let token = encode_pasted_text("line1\nline2\nline3\nline4");
    let parsed_id = {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.lx.resize(60, 6);
        target.lx.insert_str(&token);
        target.lx.insert_str(" tail");
        let parsed = crate::app::paste::folded_tokens(&target.lx.input)[0].clone();
        assert!(target.lx.toggle_paste(parsed.id, 0));
        target.lx.set_preview_selection(Some((0, 4)));
        parsed.id
    };
    assert!(
        state
            .pane_anywhere(pane)
            .expect("pane")
            .lx
            .has_expanded_pastes()
    );
    apply_lx_input(&mut state, LxInput::Escape);
    {
        let target = state.pane_anywhere(pane).expect("pane");
        assert!(target.lx.preview_selection().is_none());
        assert!(
            target.lx.has_expanded_pastes(),
            "第一下 Esc 仅清预览选区，展开芯片保持展开"
        );
    }

    // 在展开芯片仍展开时，于输入框普通文本建立键盘选区：第一下 Esc 清输入框选区，第二下才收起芯片。
    apply_lx_input(&mut state, LxInput::SelectLeft);
    assert!(
        state
            .pane_anywhere(pane)
            .expect("pane")
            .lx
            .text_selection_range()
            .is_some()
    );
    apply_lx_input(&mut state, LxInput::Escape);
    {
        let target = state.pane_anywhere(pane).expect("pane");
        assert!(target.lx.text_selection_range().is_none());
        assert!(
            target.lx.has_expanded_pastes(),
            "第一下 Esc 仅清输入框选区，展开芯片保持展开"
        );
    }
    let _ = parsed_id;
    apply_lx_input(&mut state, LxInput::Escape);
    assert!(
        !state
            .pane_anywhere(pane)
            .expect("pane")
            .lx
            .has_expanded_pastes(),
        "无选区后第二下 Esc 收起展开芯片"
    );
}

#[test]
fn ask_editor_clears_selection_before_exiting_edit_mode() {
    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();

    // 1. Ask 手写输入框：选中文字后第一下 Esc 仅清选区，第二下 Esc 才退出编辑态。
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        let mut ask = AskState::new(
            RequestKey {
                pane,
                session: "sess-1".into(),
                generation: 1,
            },
            "call-1".into(),
            vec![AskQuestion {
                id: "q1".into(),
                question: "Pick one".into(),
                header: "Q1".into(),
                options: vec![],
                multi_select: false,
            }],
        );
        assert!(ask.begin_edit());
        if let Some(editor) = ask.editor_mut() {
            editor.insert_str("custom answer");
            editor.select_all();
        }
        target.lx.ask = Some(ask);
    }
    assert!(has_visible_selection(&state));
    apply_lx_input(&mut state, LxInput::Escape);
    {
        let ask = state
            .pane_anywhere(pane)
            .and_then(|p| p.lx.ask.as_ref())
            .expect("ask");
        assert!(ask.editing, "第一下 Esc 不退出 Ask 编辑态");
        assert!(
            ask.editor()
                .expect("editor")
                .text_selection_range()
                .is_none(),
            "第一下 Esc 清除 Ask 手写框选区"
        );
        assert_eq!(ask.editor().expect("editor").text(), "custom answer");
    }
    apply_lx_input(&mut state, LxInput::Escape);
    {
        let ask = state
            .pane_anywhere(pane)
            .and_then(|p| p.lx.ask.as_ref())
            .expect("ask");
        assert!(!ask.editing, "第二下 Esc 退出 Ask 编辑态");
    }
    state.pane_mut_anywhere(pane).expect("pane").lx.ask = None;
}

#[test]
fn prompt_editor_tree_pages_and_expanded_chip_clear_selection_before_closing_views() {
    let mut state = AppState::demo();
    state.prompt.resize(60, 10);

    // 1. Prompt 正文存在展开粘贴芯片 + 正文选区：第一下 Esc 仅清正文选区，第二下才收起展开芯片。
    let token = encode_pasted_text("alpha\nbeta\ngamma\ndelta");
    state.prompt.insert_str(&token);
    state.prompt.insert_str(" suffix");
    let parsed = crate::app::paste::folded_tokens(state.prompt.text())[0].clone();
    assert!(state.prompt.toggle_paste(parsed.id, 0));
    assert!(state.prompt.has_expanded_pastes());
    apply_editor(&mut state, EditorCommand::SelectAll);
    assert!(prompt_selection_bounds(&state).is_some());

    apply_editor(&mut state, EditorCommand::Escape);
    assert!(
        prompt_selection_bounds(&state).is_none(),
        "第一下 Esc 清除 Prompt 正文选区"
    );
    assert!(
        state.prompt.has_expanded_pastes(),
        "第一下 Esc 不得连带收起已展开粘贴芯片"
    );
    apply_editor(&mut state, EditorCommand::Escape);
    assert!(
        !state.prompt.has_expanded_pastes(),
        "第二下 Esc 收起已展开粘贴芯片"
    );

    // 2. Prompt 树视图搜索框：有选区时第一下 Esc 清选区，第二下 Esc 退出树视图。
    show_prompt_tree(&mut state);
    assert!(prompt_tree_mode(&state));
    apply_prompt_search(&mut state, &EditorCommand::InsertText("query".into()));
    apply_prompt_search(&mut state, &EditorCommand::SelectAll);
    assert!(state.prompt_owner().prompt_panel.search.has_selection());
    apply_editor(&mut state, EditorCommand::Escape);
    assert!(prompt_tree_mode(&state), "第一下 Esc 保持在树视图");
    assert!(
        !state.prompt_owner().prompt_panel.search.has_selection(),
        "第一下 Esc 清除树搜索框选区"
    );
    apply_editor(&mut state, EditorCommand::Escape);
    assert!(
        !prompt_tree_mode(&state),
        "第二下 Esc 退出树视图回到编辑视图"
    );

    // 3. Prompt 页列表搜索框：有选区时第一下 Esc 清选区，第二下 Esc 退出页列表视图。
    assert!(show_prompt_pages(&mut state));
    assert!(prompt_pages_mode(&state));
    apply_page_search(&mut state, &EditorCommand::InsertText("page".into()));
    apply_page_search(&mut state, &EditorCommand::SelectAll);
    assert!(
        state
            .prompt_owner()
            .prompt_panel
            .pages_view
            .search
            .has_selection()
    );
    apply_editor(&mut state, EditorCommand::Escape);
    assert!(prompt_pages_mode(&state), "第一下 Esc 保持在页列表视图");
    assert!(
        !state
            .prompt_owner()
            .prompt_panel
            .pages_view
            .search
            .has_selection(),
        "第一下 Esc 清除页搜索框选区"
    );
    apply_editor(&mut state, EditorCommand::Escape);
    assert!(!prompt_pages_mode(&state), "第二下 Esc 退出页列表视图");
}

#[test]
fn todo_edit_and_workspace_history_search_clear_selection_before_exiting() {
    let mut state = AppState::demo();

    // 1. Todo 内联编辑框：选中文字后第一下 Esc 仅清选区，第二下 Esc 才取消编辑。
    state.right_tab = RightTab::Todo;
    assert!(apply_todo_button(&mut state, TodoButton::New));
    assert!(apply_todo_key(&mut state, TodoKey::Char('t')));
    assert!(apply_todo_key(&mut state, TodoKey::Char('a')));
    assert!(apply_todo_key(&mut state, TodoKey::SelectAll));
    assert!(
        state
            .todo
            .edit_input()
            .is_some_and(TextInput::has_selection)
    );
    assert!(apply_todo_key(&mut state, TodoKey::Esc));
    assert!(state.todo.is_editing(), "第一下 Esc 保持 Todo 编辑态");
    assert!(
        !state
            .todo
            .edit_input()
            .is_some_and(TextInput::has_selection),
        "第一下 Esc 清除 Todo 输入框选区"
    );
    assert!(apply_todo_key(&mut state, TodoKey::Esc));
    assert!(!state.todo.is_editing(), "第二下 Esc 取消 Todo 编辑");

    // 2. 侧栏工作区历史搜索框：选中文字后第一下 Esc 仅清选区，第二下 Esc 退出历史模式。
    assert!(show_workspace_history(&mut state));
    assert!(workspace_history_mode(&state));
    assert!(apply_workspace_history_key(
        &mut state,
        OverlayKey::Char('l'),
        10
    ));
    assert!(apply_workspace_history_key(
        &mut state,
        OverlayKey::Char('x'),
        10
    ));
    assert!(apply_workspace_history_key(
        &mut state,
        OverlayKey::SelectAll,
        10
    ));
    assert!(state.workspace_history.search.has_selection());
    assert!(apply_workspace_history_key(&mut state, OverlayKey::Esc, 10));
    assert!(
        workspace_history_mode(&state),
        "第一下 Esc 保持在工作区历史列表模式"
    );
    assert!(
        !state.workspace_history.search.has_selection(),
        "第一下 Esc 清除历史搜索框选区"
    );
    assert!(apply_workspace_history_key(&mut state, OverlayKey::Esc, 10));
    assert!(
        !workspace_history_mode(&state),
        "第二下 Esc 退出工作区历史列表模式"
    );
}

#[test]
fn overlays_clear_own_selection_first_and_ignore_occluded_background_selections() {
    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    let root = PathBuf::from("/repo");
    let overlays = vec![
        Overlay::Rename(Rename {
            target: RenameTarget::Workspace(0),
            input: TextInput::new("ws"),
        }),
        Overlay::NewWorkspace(NewWorkspace {
            input: TextInput::new("new-ws"),
        }),
        Overlay::NewNode(NewNode {
            kind: NewNodeKind::Folder { parent_id: None },
            name: TextInput::new("folder"),
            resume: None,
        }),
        Overlay::SaveDraft(SaveDraft {
            workspace: 0,
            name: TextInput::new("draft"),
            entries: vec![SaveDraftEntry::Root],
            selected: 0,
        }),
        Overlay::WorktreeOpen(WorktreeOpen {
            source: 0,
            repo_root: root.clone(),
            entries: Vec::new(),
            selected: 0,
            query: TextInput::new("feat"),
            loading: false,
            failed: false,
        }),
        Overlay::ReferencedFolderInput(ReferencedFolderInput {
            anchor: (0, 0),
            input: TextInput::new("/tmp"),
            key: None,
            validating: false,
            error: None,
            request: None,
        }),
        Overlay::ReferencedFolderSearch(ReferencedFolderSearch {
            anchor: (0, 0),
            folder: 1,
            key: None,
            root: root.clone(),
            initial_root: root,
            stack: Vec::new(),
            query: TextInput::new("main"),
            entries: Vec::new(),
            items: Vec::new(),
            scan_truncated: false,
            truncated: false,
            selected: 0,
            viewport: None,
            loading: false,
            failed: false,
            request: None,
        }),
        Overlay::ResumeSessionEdit(ResumeSessionEdit {
            pane,
            workspace: PathBuf::from("/repo"),
            session_id: "sess".into(),
            input: TextInput::new("title"),
            confirming_delete: false,
            hint: None,
        }),
        {
            let mut panel = ConnectPanel::new_with_env(pane, &ConnectSettings::default(), |_| None);
            panel.input = TextInput::new("openai");
            Overlay::Connect(Box::new(panel))
        },
    ];

    for overlay in overlays {
        let kind = overlay.kind();
        state.overlay = Some(overlay);
        apply_overlay_key(&mut state, OverlayKey::SelectAll);
        assert!(
            state.overlay.as_ref().is_some_and(Overlay::has_selection),
            "{kind:?} 全选后应有选区"
        );
        apply_overlay_key(&mut state, OverlayKey::Esc);
        assert_eq!(
            state.overlay.as_ref().map(Overlay::kind),
            Some(kind),
            "{kind:?} 有选区时第一下 Esc 不得关闭或退回弹窗"
        );
        assert!(
            !state.overlay.as_ref().is_some_and(Overlay::has_selection),
            "{kind:?} 第一下 Esc 应清除输入框选区"
        );
        apply_overlay_key(&mut state, OverlayKey::Esc);
        assert_ne!(
            state.overlay.as_ref().map(Overlay::kind),
            Some(kind),
            "{kind:?} 无选区后第二下 Esc 应关闭或退回上一级弹窗"
        );
    }

    // 底层窗格有选区、但当前打开的模态弹窗自身无选区时：第一下 Esc 直接关闭弹窗，不被底层选区误吞。
    state.selection = Some(Selection::programmatic_range(
        state.prompt.id(),
        (0, 0),
        (0, 3),
    ));
    state.overlay = Some(Overlay::Rename(Rename {
        target: RenameTarget::Workspace(0),
        input: TextInput::new("clean"),
    }));
    assert!(!has_visible_selection(&state));
    apply_overlay_key(&mut state, OverlayKey::Esc);
    assert!(
        state.overlay.is_none(),
        "浮层无选区时按 Esc 应直接关闭浮层，不受底层被遮挡选区影响"
    );
}

#[test]
fn single_selection_invariant_clears_previous_container_on_keyboard_select() {
    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    state.prompt.resize(40, 6);
    state.prompt.insert_str("prompt text");
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.lx.resize(40, 4);
        target.lx.insert_str("lx draft");
    }

    // 先在 Prompt 全选，再在 Lx 输入框按 Shift+Left：Prompt 选区应自动清除。
    apply_editor(&mut state, EditorCommand::SelectAll);
    assert!(prompt_selection_bounds(&state).is_some());
    apply_lx_input(&mut state, LxInput::SelectLeft);
    assert!(
        prompt_selection_bounds(&state).is_none(),
        "Lx 开始键盘选区时应清理 Prompt 选区"
    );
    assert!(
        state
            .pane_anywhere(pane)
            .expect("pane")
            .lx
            .text_selection_range()
            .is_some()
    );

    // 再在 Prompt 全选：Lx 输入框选区应自动清除。
    apply_editor(&mut state, EditorCommand::SelectAll);
    assert!(
        state
            .pane_anywhere(pane)
            .expect("pane")
            .lx
            .text_selection_range()
            .is_none(),
        "Prompt 全选时应清理 Lx 输入框选区"
    );
    assert!(prompt_selection_bounds(&state).is_some());
}

#[test]
fn review_fixes_whitespace_selection_menu_exclusivity_and_background_tab_preservation() {
    use crate::app::overlay::{Menu, MenuCommand, OverlayTarget};
    use crate::app::prompt::PromptPageId;
    use crate::app::update::{
        activate_menu, begin_selection_with_clicks, create_tab, drag_selection,
        prompt_selection_range, terminal_has_selection,
    };

    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();

    // 1. 终端纯空白拖选（尚未松开鼠标）：屏幕存在反显选区，has_visible_selection 必须为 true，按 Esc 消费按键。
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.view = PaneView::Terminal;
        target.terminal.feed(b"     \r\n");
    }
    begin_terminal_selection_with_clicks(&mut state, pane, 0, 0, 1);
    drag_terminal_selection(&mut state, pane, 0, 3);
    assert!(terminal_has_selection(&state, pane));
    assert!(
        terminal_selection_text(&state, pane).is_none(),
        "纯空白选区无非空复制文本"
    );
    assert!(
        has_visible_selection(&state),
        "纯空白终端选区有反显高亮，应判定为可见选区"
    );
    assert!(
        escape_clear_selection(&mut state),
        "纯空白终端选区按 Esc 必须消费按键以防穿透到 PTY"
    );
    assert!(!terminal_has_selection(&state, pane));

    // 2. Prompt 展开粘贴芯片 + 行尾空白拖选（prompt_selection_bounds 为 None，但 prompt_selection_range 为 Some）：
    //    第一下 Esc 仅清除行尾空白反显，不收起已展开粘贴芯片；第二下 Esc 才收起芯片。
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.view = PaneView::Lx;
    }
    state.prompt.resize(60, 10);
    state.prompt.set_text("");
    let token = encode_pasted_text("l1\nl2\nl3\nl4");
    state.prompt.insert_str(&token);
    state.prompt.insert_str("\n");
    let parsed = crate::app::paste::folded_tokens(state.prompt.text())[0].clone();
    assert!(state.prompt.toggle_paste(parsed.id, 0));
    assert!(state.prompt.has_expanded_pastes());

    let prompt_id = state.prompt.id();
    let empty_row = (state.prompt.visual_rows().len() - 1) as u16;
    begin_selection_with_clicks(&mut state, prompt_id, empty_row, 5, 1);
    drag_selection(&mut state, prompt_id, empty_row, 15);
    assert!(prompt_selection_range(&state).is_some());
    assert!(
        prompt_selection_bounds(&state).is_none(),
        "行尾越界空白拖选无字节范围，但有视觉高亮范围"
    );
    assert!(has_visible_selection(&state));
    apply_editor(&mut state, EditorCommand::Escape);
    assert!(
        prompt_selection_range(&state).is_none(),
        "第一下 Esc 清除行尾空白选区"
    );
    assert!(
        state.prompt.has_expanded_pastes(),
        "第一下 Esc 不得连带收起已展开粘贴芯片"
    );
    apply_editor(&mut state, EditorCommand::Escape);
    assert!(
        !state.prompt.has_expanded_pastes(),
        "第二下 Esc 收起已展开粘贴芯片"
    );

    // 3. 右键菜单 Overlay::Menu 打开且底层有选区：第一下 Esc 仅关闭菜单并保留底层选区，第二下 Esc 清除底层选区。
    apply_editor(&mut state, EditorCommand::SelectAll);
    assert!(prompt_selection_range(&state).is_some());
    state.overlay = Some(Overlay::Menu(Menu {
        anchor: (10, 5),
        target: OverlayTarget::PromptPage {
            workspace: 0,
            owner: crate::app::prompt::PromptEditing::Temp(1),
            page: PromptPageId::Temp(1),
        },
        commands: vec![MenuCommand::SelectAll],
        selected: 0,
        confirming: false,
    }));
    assert!(
        !has_visible_selection(&state),
        "右键菜单自身无输入框选区，不阻塞关闭菜单"
    );
    apply_overlay_key(&mut state, OverlayKey::Esc);
    assert!(state.overlay.is_none(), "第一下 Esc 关闭右键菜单");
    assert!(
        prompt_selection_range(&state).is_some(),
        "关闭右键菜单时保留底层 Prompt 选区"
    );
    assert!(
        escape_clear_selection(&mut state),
        "第二下 Esc 清除底层 Prompt 选区"
    );
    assert!(prompt_selection_range(&state).is_none());

    // 4. 右键菜单 SelectAll 互斥清理：先在 Lx 产生选区，再通过右键菜单触发 Prompt SelectAll，Lx 选区应被清除。
    begin_lx_selection_range(&mut state, pane, (0, 0), (0, 4), SelectionUnit::Word);
    assert!(state.lx_selection.is_some());
    state.overlay = Some(Overlay::Menu(Menu {
        anchor: (10, 5),
        target: OverlayTarget::PromptPage {
            workspace: 0,
            owner: crate::app::prompt::PromptEditing::Temp(1),
            page: PromptPageId::Temp(1),
        },
        commands: vec![MenuCommand::SelectAll],
        selected: 0,
        confirming: false,
    }));
    activate_menu(&mut state);
    assert!(
        state.lx_selection.is_none(),
        "右键菜单 SelectAll 应清除其他容器选区"
    );
    assert!(prompt_selection_range(&state).is_some());

    // 5. 后台 Tab / 折叠容器选区保留：当选区位于后台 Tab 或已折叠容器中（当前不可见）时，按 Esc 不清空它。
    crate::app::update::clear_selection(&mut state);
    create_tab(&mut state);
    {
        let bg_pane = state.pane_mut_anywhere(pane).expect("bg pane");
        bg_pane.lx.insert_str("bg draft");
        bg_pane.lx.select_all();
    }
    assert!(
        state
            .pane_anywhere(pane)
            .expect("bg pane")
            .lx
            .text_selection_range()
            .is_some()
    );
    assert!(!has_visible_selection(&state), "当前活动 Tab 无可见选区");
    assert!(!escape_clear_selection(&mut state));
    assert!(
        state
            .pane_anywhere(pane)
            .expect("bg pane")
            .lx
            .text_selection_range()
            .is_some(),
        "当前 Tab 无可见选区时按 Esc 不得清空后台 Tab 保留的静态选区"
    );

    // 6. `/mcp` 二级面板打开时，在过滤词输入框中按 SelectAll 全选，第一下 Esc 仅清除选区并保持 `/mcp` 打开，第二下 Esc 关闭 `/mcp`。
    let active_pane = state.active_tab().layout.focus();
    {
        let target = state.pane_mut_anywhere(active_pane).expect("active pane");
        target.view = PaneView::Lx;
        target.lx.slash = Some(crate::app::state::SlashPanel::Mcp(Box::default()));
        target.lx.insert_str("context7");
    }
    apply_lx_input(&mut state, LxInput::SelectAll);
    assert!(
        state
            .pane_anywhere(active_pane)
            .expect("active pane")
            .lx
            .text_selection_range()
            .is_some(),
        "/mcp 面板内支持全选过滤词"
    );
    apply_lx_input(&mut state, LxInput::Escape);
    {
        let target = state.pane_anywhere(active_pane).expect("active pane");
        assert!(
            target.lx.text_selection_range().is_none(),
            "第一下 Esc 清除过滤词选区"
        );
        assert!(
            matches!(target.lx.slash, Some(crate::app::state::SlashPanel::Mcp(_))),
            "第一下 Esc 保持 /mcp 面板打开"
        );
        assert_eq!(target.lx.input, "context7");
    }
    apply_lx_input(&mut state, LxInput::Escape);
    {
        let target = state.pane_anywhere(active_pane).expect("active pane");
        assert!(target.lx.slash.is_none(), "第二下 Esc 关闭 /mcp 面板");
        assert!(target.lx.input.is_empty());
    }
}

#[test]
fn btw_composer_selection_and_scroll_stay_in_the_active_editor() {
    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.view = PaneView::Lx;
        target.lx.chat.session_id = Some("sess-test".into());
        target.lx.task = Some(TaskPage {
            tab: TaskTab::Btw,
            ..Default::default()
        });
        target.lx.btw.ensure_thread("qa-1");
        target.lx.btw.editing = true;
        let composer = target.lx.btw.active_composer_mut().expect("composer");
        composer.resize(10, 5);
        composer.insert_str("hello");
        target.lx.insert_str("main draft");
    }

    begin_lx_input_selection(&mut state, pane, 0, 1);
    drag_lx_input_selection(&mut state, pane, 0, 4);
    {
        let target = state.pane_anywhere(pane).expect("pane");
        assert_eq!(
            target
                .lx
                .btw
                .active_composer()
                .expect("composer")
                .selected_text()
                .as_deref(),
            Some("ell")
        );
        assert!(target.lx.selected_text().is_none(), "主输入框不能被选中");
    }
    assert_eq!(
        take_lx_input_selection(&mut state).map(|(_, text)| text),
        Some("ell".to_string())
    );

    {
        let composer = state
            .pane_mut_anywhere(pane)
            .expect("pane")
            .lx
            .btw
            .active_composer_mut()
            .expect("composer");
        composer.resize(2, 1);
        composer.replace_input("abcdef");
    }
    assert!(set_lx_scroll(&mut state, pane, 1));
    assert!(scroll_lx_input(&mut state, pane, -1));
    let target = state.pane_anywhere(pane).expect("pane");
    assert_eq!(
        target.lx.btw.active_composer().expect("composer").scroll(),
        0
    );
    assert_eq!(target.lx.scroll(), 0, "主输入框滚动不能被 BTW 滚轮改变");
}

#[test]
fn autoscroll_edge_has_two_row_bands_and_outside_distance() {
    let rect = ratatui::layout::Rect::new(0, 10, 40, 20);
    assert!(matches!(
        crate::app::update::autoscroll_edge(rect, 10),
        Some((AutoscrollDirection::Up, 0))
    ));
    assert!(matches!(
        crate::app::update::autoscroll_edge(rect, 11),
        Some((AutoscrollDirection::Up, 0))
    ));
    assert!(crate::app::update::autoscroll_edge(rect, 12).is_none());
    assert!(crate::app::update::autoscroll_edge(rect, 27).is_none());
    assert!(matches!(
        crate::app::update::autoscroll_edge(rect, 28),
        Some((AutoscrollDirection::Down, 0))
    ));
    assert!(matches!(
        crate::app::update::autoscroll_edge(rect, 29),
        Some((AutoscrollDirection::Down, 0))
    ));
    assert!(matches!(
        crate::app::update::autoscroll_edge(rect, 7),
        Some((AutoscrollDirection::Up, 3))
    ));
    assert!(matches!(
        crate::app::update::autoscroll_edge(rect, 33),
        Some((AutoscrollDirection::Down, 4))
    ));
}

#[test]
fn prompt_autoscroll_up_works_from_bottom_and_stops_at_top() {
    let mut state = AppState::demo();
    let prompt_id = state.prompt.id();
    let content = (0..30)
        .map(|index| format!("line {index}"))
        .collect::<Vec<_>>()
        .join("\n");
    state.prompt.insert_str(&content);
    state.prompt.resize(40, 6);
    let cap = state.prompt.text_max_scroll();
    assert!(cap > 0);
    state.prompt.scroll_to(cap);

    crate::app::update::begin_selection(&mut state, prompt_id, 4, 0);
    crate::app::update::drag_selection(&mut state, prompt_id, 4, 2);

    let inner = ratatui::layout::Rect::new(0, 0, 40, 6);
    let now = Instant::now();
    crate::app::update::arm_selection_autoscroll(&mut state, prompt_id, inner, (2, 0), now);
    assert!(state.selection_autoscroll.is_some());

    // 处于底部 (scroll == cap) 时，向上边缘自动滚动必须能成功上滚。
    assert!(crate::app::update::tick_selection_autoscroll(
        &mut state,
        now + Duration::from_millis(31)
    ));
    assert_eq!(state.prompt.scroll(), cap - 1, "底部向上自动滚动不被卡死");

    // 一直滚到顶部 scroll == 0 后，下一次 tick 必须返回 false 并停止，不得在顶部空转。
    let mut t = now + Duration::from_millis(62);
    let mut guard = 100;
    while state.selection_autoscroll.is_some() && guard > 0 {
        crate::app::update::tick_selection_autoscroll(&mut state, t);
        t += Duration::from_millis(31);
        guard -= 1;
    }
    assert_eq!(state.prompt.scroll(), 0);
    assert!(
        state.selection_autoscroll.is_none(),
        "到达顶部后停止自动滚动"
    );
}

#[test]
fn arm_selection_autoscroll_preserves_next_at_on_lateral_move() {
    let mut state = AppState::demo();
    let prompt_id = state.prompt.id();
    let content = (0..20)
        .map(|index| format!("line {index}"))
        .collect::<Vec<_>>()
        .join("\n");
    state.prompt.insert_str(&content);
    state.prompt.resize(40, 6);
    crate::app::update::begin_selection(&mut state, prompt_id, 1, 0);
    let inner = ratatui::layout::Rect::new(0, 0, 40, 6);
    let t0 = Instant::now();

    crate::app::update::arm_selection_autoscroll(&mut state, prompt_id, inner, (2, 0), t0);
    let expected_next = t0 + Duration::from_millis(30);
    assert_eq!(
        state.selection_autoscroll.expect("armed").next_at,
        expected_next
    );

    // 10ms 后指针沿顶边横向移动，不得推迟已定下的 next_at。
    crate::app::update::arm_selection_autoscroll(
        &mut state,
        prompt_id,
        inner,
        (10, 0),
        t0 + Duration::from_millis(10),
    );
    assert_eq!(
        state.selection_autoscroll.expect("still armed").next_at,
        expected_next,
        "同方向边缘内横向移动不得重置节拍"
    );
}
