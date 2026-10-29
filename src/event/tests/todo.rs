use super::test_support::*;
use super::*;

// ===== 右侧统一容器 tab 事件层 =====

#[test]
fn worktree_candidates_do_not_resurrect_mention_panel_on_todo() {
    // `apply_worktree_entries` 结尾无条件 `mention.refresh`，而 `clear()` 只 bump
    // generation、不清 cache。若 Todo 期间的回调仍被处理，刚收起的提及面板会被
    // 重新置 Some —— 一个看不见、却能吞滚轮的面板。
    use crate::app::markdown::{MentionEntry, WorktreeCandidate};
    use crate::db::Database;
    use crate::db::service::Persistence;
    use crate::event::chat::ChatStreams;
    use std::collections::HashSet;

    fn candidates() -> Vec<WorktreeCandidate> {
        vec![WorktreeCandidate {
            branch: "main".into(),
            path: PathBuf::from("/tmp/lx-tui-wt"),
            is_main: true,
        }]
    }

    let mut state = demo_terminal();
    state.prompt.resize(40, 8);
    update::focus_prompt(&mut state);
    // 制造提及面板：`@` 触发 + 扫描结果入缓存。
    update::apply_editor(
        &mut state,
        crate::app::actions::EditorCommand::InsertChar('@'),
    );
    let (generation, _) = state
        .prompt
        .take_mention_scan_request()
        .expect("scan requested");
    let prompt = state.prompt.id();
    update::apply_mention_entries(
        &mut state,
        prompt,
        generation,
        vec![MentionEntry {
            path: "app.rs".into(),
            is_directory: false,
        }],
    );
    // 回调只在工作区 git 元数据的仓库根匹配时才被处理（`targeted`）：
    // 不造出来，下面的断言就只是"回调被丢弃"，测不到闸门。
    let prompt_workspace = state.prompt_workspace();
    state.workspaces[prompt_workspace].git = Some(crate::app::state::WorkspaceGit {
        repo_root: PathBuf::from("/tmp/lx-tui-wt"),
        checkout_path: PathBuf::from("/tmp/lx-tui-wt"),
        is_linked: false,
        branch: Some("main".into()),
        main_branch: Some("main".into()),
    });
    // 先建立 worktree 候选，再补一次扫描缓存（候选取值变化会清 cache）。
    state.prompt.apply_worktree_entries(candidates());
    let (generation, _) = state
        .prompt
        .take_mention_scan_request()
        .expect("scan requested again");
    update::apply_mention_entries(
        &mut state,
        prompt,
        generation,
        vec![MentionEntry {
            path: "app.rs".into(),
            is_directory: false,
        }],
    );
    assert!(state.prompt.mention().is_some(), "前提：提及面板可见");

    assert!(update::select_right_tab(&mut state, RightTab::Todo));
    assert!(state.prompt.mention().is_none(), "切走即收起");

    // 相同候选到达（cache 不失效）——这正是复活的那条路径。
    // 库路径必须**每次唯一**：固定路径会被并发测试与其他 worktree 反复打开，
    // 一旦某个分支把 `SCHEMA_VERSION` 推进到更高值，这个文件就被写成高版本库，
    // 低版本二进制再打开会直接 `UnsupportedSchema`（实测：v15 分支跑过之后，
    // 另一侧再跑这条用例必挂）。用完即删。
    let db_path = std::env::temp_dir().join(format!(
        "lx-tui-wt-gate-{}-{:?}.db",
        std::process::id(),
        std::thread::current().id()
    ));
    let untrusted = Database::open(&db_path).expect("open side db");
    let persistence = Persistence::with_database(untrusted);
    let mut streams = ChatStreams::default();
    let mut inflight = HashSet::new();
    handle_app_event(
        AppEvent::PromptWorktreeListed {
            repo_root: PathBuf::from("/tmp/lx-tui-wt"),
            candidates: candidates(),
            failed: false,
        },
        &mut state,
        &mut HashMap::new(),
        &mut inflight,
        &mut streams,
        &persistence,
    );

    assert!(
        state.prompt.mention().is_none(),
        "Todo 期间到达的 worktree 回调不得复活隐藏的提及面板"
    );
    drop(persistence);
    let _ = std::fs::remove_file(&db_path);
}

#[test]
fn mouse_click_todo_blank_still_focuses_container() {
    let mut state = demo_terminal();
    let (config, geo) = toolbar_geometry(&mut state);
    state.prompt.insert_str("ab\ncd");
    assert!(crate::app::update::select_right_tab(
        &mut state,
        RightTab::Todo
    ));
    // 点分割线（既不是按钮也不是行）：容器进入选中态，但不动数据、不穿透。
    let divider = layout::prompt_divider_rect(geo.view.prompt).expect("divider visible");
    let cursor = state.prompt.cursor();
    let focused_pane = state.active_tab().layout.focus();
    let mut sessions = HashMap::new();
    let mut dirty = false;

    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            divider.x + 1,
            divider.y,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );

    assert!(!state.prompt_focused(), "Todo 空白处点击不聚焦编辑器");
    assert!(state.todo_focused(), "点容器任意位置即让右侧容器进入选中态");
    assert_eq!(state.prompt.cursor(), cursor, "Todo 空白处点击不动光标");
    assert_eq!(
        state.active_tab().layout.focus(),
        focused_pane,
        "不穿透到窗格、不改焦点"
    );
    assert!(!dirty, "已聚焦时点空白不产生变化");
    assert!(sessions.is_empty(), "点击不写终端");
}

#[test]
fn mouse_click_prompt_toolbar_ignored_on_todo_tab() {
    let mut state = demo_terminal();
    let (config, geo) = toolbar_geometry(&mut state);
    assert!(crate::app::update::select_right_tab(
        &mut state,
        RightTab::Todo
    ));
    let mode_before: PromptMode = state.prompt_owner().prompt_panel.mode;
    assert_eq!(mode_before, PromptMode::Editor, "前提：当前是编辑视图");
    let mut sessions = HashMap::new();
    let mut dirty = false;

    // 编辑视图 `[≡]` 按钮：Todo 下必须整行吞掉，绝不执行动作。
    let mode = layout::prompt_toolbar_button_rect(
        geo.view.prompt,
        layout::PromptToolbarButton::Mode,
        false,
    )
    .expect("mode button visible");
    assert_eq!(
        layout::prompt_toolbar_button_at(geo.view.prompt, mode.x, mode.y, false),
        Some(layout::PromptToolbarButton::Mode),
        "前提：该坐标确实命中工具栏按钮"
    );
    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Left), mode.x, mode.y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );

    assert_eq!(
        state.prompt_owner().prompt_panel.mode,
        mode_before,
        "Todo 下提示词工具栏按钮不切换视图"
    );
    assert!(state.overlay.is_none(), "Todo 下提示词工具栏按钮不打开浮层");
    // 同一坐标现在属于 Todo 自己的 `[new]` 按钮（两个面板共用工具栏行），
    // 因此这里**会**产生副作用：命中 Todo 按钮并进入编辑。旧断言「不置脏」
    // 随 Todo 成为真实面板而作废。
    assert_eq!(
        state.todo.editing().map(|edit| edit.target.is_none()),
        Some(true),
        "点击落到 Todo 的 [new] 按钮上，进入新建编辑"
    );
    assert!(dirty, "Todo 按钮命中必须置脏");
}

#[test]
fn mouse_wheel_over_todo_scrolls_list_without_moving_selection() {
    let mut state = demo_terminal();
    let (config, geo) = toolbar_geometry(&mut state);
    let text = layout::prompt_text_rect(geo.view.prompt);
    state.prompt.resize(text.width, text.height);
    for _ in 0..=usize::from(text.height) {
        state.prompt.insert_str("line\n");
    }
    crate::app::update::set_prompt_scroll(&mut state, 2);
    assert!(state.prompt.scroll() > 0, "前提：正文可滚动且已有偏移");
    let prompt_scroll = state.prompt.scroll();
    let tree_scroll = state.prompt_owner().prompt_panel.scroll;

    // 塞满超过一屏的 todo，保证视口可以滚动。
    let visible = crate::ui::right_panel::todo::visible_rows(geo.view.prompt);
    assert!(visible > 0, "前提：列表区可见");
    state.todo.replace(
        (0..visible + 5)
            .map(|index| {
                crate::db::todo::TodoRecord::new(
                    format!("item {index}"),
                    crate::db::todo::TodoStatus::Todo,
                    i64::try_from(index).unwrap_or(i64::MAX),
                )
            })
            .collect(),
    );
    state.todo.set_viewport_rows(visible);
    assert!(crate::app::update::select_right_tab(
        &mut state,
        RightTab::Todo
    ));
    let selected = state.todo.selected();
    let mut sessions = HashMap::new();
    let mut dirty = false;

    handle_terminal_event(
        mouse(MouseEventKind::ScrollDown, text.x + 1, text.y + 1),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );

    assert!(state.todo.scroll() > 0, "Todo 下滚轮滚动列表视口");
    assert_eq!(state.todo.selected(), selected, "滚轮不动高亮");
    assert!(dirty, "视口移动必须置脏");
    assert_eq!(
        state.prompt.scroll(),
        prompt_scroll,
        "Todo 下滚轮不动 prompt 正文视口"
    );
    assert_eq!(
        state.prompt_owner().prompt_panel.scroll,
        tree_scroll,
        "Todo 下滚轮不动树视图视口"
    );
}

#[test]
fn mouse_click_todo_row_selects_and_focuses() {
    let mut state = demo_terminal();
    let (config, geo) = toolbar_geometry(&mut state);
    state.todo.replace(
        (0..3)
            .map(|index| {
                crate::db::todo::TodoRecord::new(
                    format!("item {index}"),
                    crate::db::todo::TodoStatus::Todo,
                    index,
                )
            })
            .collect(),
    );
    state
        .todo
        .set_viewport_rows(crate::ui::right_panel::todo::visible_rows(geo.view.prompt));
    assert!(crate::app::update::select_right_tab(
        &mut state,
        RightTab::Todo
    ));
    let list = crate::ui::right_panel::todo::list_rect(geo.view.prompt);
    let mut sessions = HashMap::new();
    let mut dirty = false;

    // 点第二行。
    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            list.x + 1,
            list.y + 1,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );

    assert_eq!(state.todo.selected(), Some(1), "点击选中该行");
    assert!(state.todo_focused(), "点击行聚焦 Todo 面板");
    assert!(!state.prompt_focused(), "与 prompt 焦点互斥");
    assert!(dirty, "选中变化置脏");
}

#[test]
fn mouse_click_todo_toolbar_button_runs_action() {
    let mut state = demo_terminal();
    let (config, geo) = toolbar_geometry(&mut state);
    assert!(crate::app::update::select_right_tab(
        &mut state,
        RightTab::Todo
    ));
    let new = crate::ui::right_panel::todo::button_rect(
        geo.view.prompt,
        crate::app::tooltip::TodoButton::New,
    )
    .expect("[new] visible");
    let mut sessions = HashMap::new();
    let mut dirty = false;

    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Left), new.x, new.y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );

    assert!(
        state
            .todo
            .editing()
            .is_some_and(|edit| edit.target.is_none()),
        "[new] 进入新建编辑"
    );
    assert!(state.todo_focused(), "按钮隐式聚焦");
    assert!(dirty, "按钮动作置脏");
}

#[test]
fn mouse_double_click_todo_row_enters_edit() {
    let mut state = demo_terminal();
    let (config, geo) = toolbar_geometry(&mut state);
    state.todo.replace(
        (0..3)
            .map(|index| {
                crate::db::todo::TodoRecord::new(
                    format!("item {index}"),
                    crate::db::todo::TodoStatus::Todo,
                    index,
                )
            })
            .collect(),
    );
    assert!(crate::app::update::select_right_tab(
        &mut state,
        RightTab::Todo
    ));
    let list = crate::ui::right_panel::todo::list_rect(geo.view.prompt);
    let mut sessions = HashMap::new();
    let mut cache = ui::RenderCache::default();
    let mut dirty = false;

    // 第一击：选中 + 聚焦，不进编辑。
    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            list.x + 1,
            list.y + 2,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut cache,
        &mut dirty,
    );
    assert_eq!(state.todo.selected(), Some(2));
    assert!(!state.todo.is_editing(), "单击不进入编辑");

    // 第二击（仍在判定窗口内）：进入该行的行内编辑，输入框带原文。
    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            list.x + 1,
            list.y + 2,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut cache,
        &mut dirty,
    );
    let edit = state.todo.editing().expect("双击进入编辑");
    assert_eq!(
        edit.target.as_deref(),
        Some(state.todo.items()[2].id.as_str())
    );
    assert_eq!(edit.input.text(), "item 2", "编辑现场带原文");
    assert!(state.todo_focused());
}

/// 点容器顶边框的 tab 旁空白：同样让右侧容器进入选中态（不应"只有点进输入框"才算）。
#[test]
fn mouse_click_todo_border_blank_focuses_container() {
    let mut state = demo_terminal();
    let (config, geo) = toolbar_geometry(&mut state);
    assert!(crate::app::update::select_right_tab(
        &mut state,
        RightTab::Todo
    ));
    assert!(state.todo_focused(), "前提：切 tab 后右栏持焦点");
    // 顶边框最右端（tab 之外、折叠按钮之外的空白）。
    let border = crate::layout::prompt_border_rect(geo.view.prompt).expect("border visible");
    let mut sessions = HashMap::new();
    let mut dirty = false;

    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            border.right() - 3,
            border.y,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );

    assert!(
        state.todo_focused(),
        "切 tab 后右栏即选中；点边框空白不改焦点"
    );
    assert!(!state.prompt_focused());
    assert!(!dirty, "已聚焦时点边框空白不产生变化");
}

#[test]
fn mouse_click_todo_stats_row_focuses_without_data_change() {
    let mut state = demo_terminal();
    let (config, geo) = toolbar_geometry(&mut state);
    // 条目必须多到「统计行所在行」在列表下标范围内：若列表区误吞了统计块，
    // 这一击就会选中一行——用单条列表测不出这个问题。
    let rows = usize::from(crate::layout::pane_inner_rect(geo.view.prompt).height);
    state.todo.replace(
        (0..rows.saturating_add(8))
            .map(|index| {
                crate::db::todo::TodoRecord::new(
                    format!("item {index}"),
                    crate::db::todo::TodoStatus::Todo,
                    i64::try_from(index).unwrap_or(i64::MAX),
                )
            })
            .collect(),
    );
    assert!(crate::app::update::select_right_tab(
        &mut state,
        RightTab::Todo
    ));
    let stats = crate::ui::right_panel::todo::stats_rect(geo.view.prompt).expect("统计行可见");
    let cursor = state.prompt.cursor();
    let focused_pane = state.active_tab().layout.focus();
    let mut sessions = HashMap::new();
    let mut dirty = false;

    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            stats.x + 2,
            stats.y,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );

    assert!(state.todo_focused(), "点统计块时右栏保持选中");
    assert!(!state.todo.is_editing(), "统计块不是行，不进入编辑");
    assert_eq!(state.todo.selected(), Some(0), "选中行不变");
    assert_eq!(state.prompt.cursor(), cursor, "光标不动");
    assert_eq!(
        state.active_tab().layout.focus(),
        focused_pane,
        "窗格焦点不动"
    );
    assert!(!dirty, "已聚焦时点统计块不产生变化");
}

#[test]
fn todo_keys_never_reach_pty() {
    let mut state = demo_terminal();
    let (config, geo) = toolbar_geometry(&mut state);
    assert!(crate::app::update::select_right_tab(
        &mut state,
        RightTab::Todo
    ));
    assert!(state.todo_focused(), "切到 Todo tab 后右栏持焦点");
    let mut sessions = HashMap::new();
    let keys = [
        KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE),
        KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE),
        KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE),
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE),
        // 下面三种都**不在** `todo_key` 映射表里：它们只能靠 `route` 的
        // `todo_focused` 分支吞掉。若该分支写成「未映射键回落去编码写窗格」，
        // 这三条就会变成 `Routed::Pane`，测试随即失败。
        KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE),
        KeyEvent::new(KeyCode::F(5), KeyModifiers::NONE),
        KeyEvent::new(KeyCode::Insert, KeyModifiers::NONE),
    ];

    // 路由层是唯一可能产出终端字节的地方。这里断言的是**强不变量**：
    // Todo 聚焦时任何键只能路由成 `None`（吞掉）或 `Routed::Todo`（进面板）；
    // `Routed::Pane`/`Lx`/`Editor` 全都意味着字节会流向别处。
    // （不断言「`sessions` 表为空」——那张表只会被 `write_to_pane` 查不会插，
    // 空表上的断言恒真，是空转测试。）
    for key in keys {
        let routed = crate::input::route(
            key,
            alacritty_terminal::term::TermMode::empty(),
            crate::app::state::FocusedContainer::Todo,
            None,
            false,
        );
        assert!(
            matches!(routed, None | Some(crate::input::Routed::Todo(_))),
            "{key:?} 只允许进 Todo 或被吞掉，实际 {routed:?}"
        );
    }

    // 端到端：`a` 开新建、`j`/`x` 成为文本、`Enter` 提交、`Tab` 被吞——
    // 全都留在 Todo 面板里。
    for key in keys {
        let mut dirty = false;
        handle_terminal_event(
            TerminalEvent::Key(key),
            &mut state,
            &mut sessions,
            &geo,
            &config,
            &mut ui::RenderCache::default(),
            &mut dirty,
        );
    }
    assert_eq!(state.todo.items()[0].text, "jx", "按键落在 Todo 编辑框");
    assert!(!state.todo.is_editing(), "Enter 已提交");
}

/// 点终端窗格要提交编辑、并释放 Todo 焦点（`focus_pane` 的两件事）。
#[test]
fn focus_pane_releases_todo_focus() {
    let mut state = demo_terminal();
    let (config, geo) = toolbar_geometry(&mut state);
    state.todo.replace(vec![crate::db::todo::TodoRecord::new(
        "keep",
        crate::db::todo::TodoStatus::Todo,
        0,
    )]);
    assert!(crate::app::update::select_right_tab(
        &mut state,
        RightTab::Todo
    ));
    assert!(crate::app::update::begin_todo_edit(
        &mut state,
        crate::app::state::TodoEditTarget::New
    ));
    assert!(crate::app::update::apply_todo_paste(&mut state, "draft"));
    let pane = state.active_tab().layout.focus();
    let mut sessions = HashMap::new();
    let mut dirty = false;

    let inner = layout::pane_inner_rect(geo.view.panes);
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

    assert_eq!(
        state.active_tab().layout.focus(),
        pane,
        "前提：点的是终端窗格"
    );
    assert!(!state.todo_focused(), "点终端窗格释放 Todo 焦点");
    assert!(!state.todo.is_editing(), "编辑中的内容已提交");
    assert_eq!(state.todo.items().len(), 2, "提交后新增一条");
    assert_eq!(state.todo.items()[1].text, "draft");
}

/// 没有可撤销内容时 `Ctrl+Z` 必须被吞掉，不能漏进终端。
#[test]
fn ctrl_z_without_undo_is_consumed_not_forwarded() {
    let mut state = demo_terminal();
    let (config, geo) = toolbar_geometry(&mut state);
    assert!(crate::app::update::select_right_tab(
        &mut state,
        RightTab::Todo
    ));
    assert!(state.todo_focused(), "切到 Todo tab 后右栏持焦点");
    assert!(!state.todo.can_undo(), "前提：撤销槽为空");
    let mut sessions = HashMap::new();
    let mut dirty = false;

    let key = KeyEvent::new(KeyCode::Char('z'), KeyModifiers::CONTROL);
    assert!(
        !matches!(
            crate::input::route(
                key,
                alacritty_terminal::term::TermMode::empty(),
                crate::app::state::FocusedContainer::Todo,
                None,
                false
            ),
            Some(crate::input::Routed::Pane(_))
        ),
        "Ctrl+Z 在 Todo 聚焦时绝不路由到终端"
    );
    handle_terminal_event(
        TerminalEvent::Key(key),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );

    assert!(!state.todo.can_undo(), "无内容可撤销");
    assert!(!dirty, "撤销槽为空时不产生状态变化");
}

#[test]
fn todo_key_escape_blurs() {
    let mut state = demo_terminal();
    let (config, geo) = toolbar_geometry(&mut state);
    assert!(crate::app::update::select_right_tab(
        &mut state,
        RightTab::Todo
    ));
    assert!(state.todo_focused(), "切到 Todo tab 后右栏持焦点");
    let mut sessions = HashMap::new();
    let mut dirty = false;

    handle_terminal_event(
        TerminalEvent::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );

    assert!(!state.todo_focused(), "Esc 释放 Todo 焦点");
    assert!(dirty);
}

#[test]
fn ctrl_q_wins_over_todo_focus() {
    let mut state = demo_terminal();
    let (config, geo) = toolbar_geometry(&mut state);
    assert!(crate::app::update::select_right_tab(
        &mut state,
        RightTab::Todo
    ));
    assert!(state.todo_focused(), "切到 Todo tab 后右栏持焦点");
    let mut sessions = HashMap::new();
    let mut dirty = false;

    handle_terminal_event(
        TerminalEvent::Key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL)),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );

    assert!(state.should_quit, "Ctrl+Q 始终优先");
    assert!(dirty, "退出动作置脏");
    assert!(state.todo_focused(), "Ctrl+Q 不碰 todo 焦点");
    assert!(state.todo.items().is_empty(), "Ctrl+Q 不改 todo 列表");
    assert!(!state.todo.is_editing(), "Ctrl+Q 不开编辑");
}

#[test]
fn pane_content_inner_is_zero_on_todo_tab() {
    // 兜底口径：Todo 下 prompt 没有可交互内容区，按内容区坐标换算必须拿到零矩形，
    // 否则残留路径会把鼠标位置写进一个看不见的编辑器光标。
    let config = Config::default();
    let mut state = demo_terminal();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let prompt = state.prompt.id();
    let inner = layout::pane_inner_rect(geo.view.prompt);
    let active = super::mouse::hit::pane_content_inner(&state, &geo.view, prompt, inner);
    assert!(
        active.width > 0 && active.height > 0,
        "前提：Prompt 态内容区非空: {active:?}"
    );

    state.right_tab = RightTab::Todo;
    assert_eq!(
        super::mouse::hit::pane_content_inner(&state, &geo.view, prompt, inner),
        Rect::ZERO,
        "Todo 下 prompt 内容区为零矩形"
    );
}

#[test]
fn mouse_right_click_prompt_tree_on_todo_opens_no_menu() {
    // 提示词树右键菜单的闸门：Todo 下内容区不接收右键。
    // 正向对照在前提里：同一坐标在 Prompt 态确实能打开菜单。
    use crate::app::prompt::PromptItem;
    let config = Config::default();
    let mut state = demo_terminal();
    update::show_prompt_tree(&mut state);
    state.workspaces[0]
        .prompt_panel
        .tree
        .replace(vec![], vec![PromptItem::single(7, None, "item", "body")]);
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let area = paste::prompt_text_area(&geo.view);
    let target = ui::prompt::tree::row_at(
        area,
        &state.prompt_owner().prompt_panel,
        area.x + 2,
        area.y + 2,
    )
    .map(|_| (area.x + 2, area.y + 2));
    let Some((col, row)) = target else {
        panic!("前提：该坐标落在树行上");
    };
    let mut sessions = HashMap::new();
    let mut dirty = false;

    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Right), col, row),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(
        matches!(state.overlay, Some(Overlay::Menu(_))),
        "前提：Prompt 态同坐标能开菜单"
    );

    update::close_overlay(&mut state);
    assert!(crate::app::update::select_right_tab(
        &mut state,
        RightTab::Todo
    ));
    dirty = false;
    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Right), col, row),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(state.overlay.is_none(), "Todo 下右键不打开提示词树菜单");
    assert!(!dirty);
}

#[test]
fn mouse_drag_on_todo_does_not_move_hidden_cursor() {
    // 拖拽光标跟随的闸门：Todo 下拖动不得改写隐藏编辑器的光标。
    let config = Config::default();
    let mut state = demo_terminal();
    state.prompt.set_text("alpha beta gamma\nsecond line");
    state.prompt.resize(40, 8);
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let text = layout::prompt_text_rect(geo.view.prompt);
    let prompt = state.prompt.id();
    let mut sessions = HashMap::new();
    let mut dirty = false;

    // 正向对照：Prompt 态拖拽会移动光标。
    update::focus_prompt(&mut state);
    update::begin_selection(&mut state, prompt, 0, 0);
    handle_terminal_event(
        mouse(
            MouseEventKind::Drag(MouseButton::Left),
            text.x + 3,
            text.y + 1,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(state.prompt.cursor() > 0, "前提：Prompt 态拖拽移动光标");

    assert!(crate::app::update::select_right_tab(
        &mut state,
        RightTab::Todo
    ));
    let cursor = state.prompt.cursor();
    // `select_right_tab` 已清选区；这里直接构造拖拽事件，验证闸门而非选区状态。
    state.selection = Some(crate::app::selection::Selection::begin(prompt, 0, 0));
    dirty = false;
    handle_terminal_event(
        mouse(
            MouseEventKind::Drag(MouseButton::Left),
            text.x + 1,
            text.y + 2,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert_eq!(
        state.prompt.cursor(),
        cursor,
        "Todo 下拖拽不改隐藏编辑器的光标"
    );
    // 这一条由两道防线共同保证：`drag.rs` 的 `prompt_panel_active` 闸门与
    // `pane_content_inner` 的零矩形兜底。删掉闸门后零矩形提前返回，行为不变；
    // 删掉兜底后闸门兜住，行为也不变。两道都删才会漏，因此单独移除任一处的
    // 变异都不会让本用例变红——这是**冗余防线**，不是测试失效。
    state.selection = None;
}

#[test]
fn mouse_click_title_row_focuses_without_moving_cursor() {
    let config = Config::default();
    let mut state = paged_prompt_state();
    state.set_focus(crate::app::state::FocusedContainer::Panes);
    state.prompt.set_cursor_offset_for_test(0);
    let geo = geometry(&state, &config, Rect::new(0, 0, 140, 30));
    let title = ui::prompt_frame::title_row(geo.view.prompt).expect("title row");
    assert_eq!(
        title.y,
        crate::layout::pane_inner_rect(geo.view.prompt).y + 1,
        "标题行 = 工具栏下一行的分割线"
    );
    // 标题行内找一个既非页控件、也非页名的空白列（避开两侧边框列的分割线拖拽区）。
    let blank_x = (title.x + 2..title.right().saturating_sub(2))
        .find(|x| {
            ui::prompt_frame::prompt_page_control_at(geo.view.prompt, &state, *x, title.y).is_none()
        })
        .expect("title row has blank column");
    let mut sessions = HashMap::new();
    let mut dirty = false;

    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Left), blank_x, title.y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );

    assert!(state.prompt_focused(), "点标题行空白只把焦点交给编辑器");
    assert_eq!(state.prompt.cursor(), 0, "标题行点击不移动正文光标");
    assert!(dirty);
}

/// 释放 Todo 滚动条拖拽后必须清空抓取状态：否则之后所有左键拖拽都被 Todo 分支吞掉。
#[test]
fn todo_scrollbar_drag_releases_on_mouse_up() {
    let mut state = demo_terminal();
    let (config, geo) = toolbar_geometry(&mut state);
    assert!(crate::app::update::select_right_tab(
        &mut state,
        RightTab::Todo
    ));
    let visible = crate::ui::right_panel::todo::visible_rows(geo.view.prompt);
    state.todo.replace(
        (0..visible + 8)
            .map(|index| {
                crate::db::todo::TodoRecord::new(
                    format!("t{index}"),
                    crate::db::todo::TodoStatus::Todo,
                    i64::try_from(index).unwrap_or(i64::MAX),
                )
            })
            .collect(),
    );
    state.todo.set_viewport_rows(visible);
    let bar =
        crate::ui::right_panel::todo::scrollbar(geo.view.prompt, &state).expect("前提：列表溢出");
    let mut sessions = HashMap::new();
    let mut dirty = false;

    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            bar.thumb.x,
            bar.thumb.y,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(state.todo_scroll_drag.is_some(), "前提：抓住了 thumb");

    handle_terminal_event(
        mouse(
            MouseEventKind::Up(MouseButton::Left),
            bar.thumb.x,
            bar.thumb.y,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );

    assert!(
        state.todo_scroll_drag.is_none(),
        "松开必须释放抓取状态，否则后续所有拖拽被劫持"
    );
}

/// Prompt tab 下点正文右缘不得滚动隐藏的 Todo 列表（两者共用最右一列）。
#[test]
fn prompt_gutter_click_does_not_scroll_hidden_todo() {
    let mut state = demo_terminal();
    let (config, geo) = toolbar_geometry(&mut state);
    assert_eq!(state.right_tab, RightTab::Prompt, "前提：Prompt tab");
    let visible = crate::ui::right_panel::todo::visible_rows(geo.view.prompt);
    state.todo.replace(
        (0..visible + 8)
            .map(|index| {
                crate::db::todo::TodoRecord::new(
                    format!("t{index}"),
                    crate::db::todo::TodoStatus::Todo,
                    i64::try_from(index).unwrap_or(i64::MAX),
                )
            })
            .collect(),
    );
    state.todo.set_viewport_rows(visible);
    // 点 Todo 列表区最右列的最后一行：两面板的最右列重合（都是 inner.right()-1），
    // 这一行只在 Todo 列表轨道内，能真正触发「隐藏滚动条被点中」的路径。
    let list = crate::ui::right_panel::todo::list_rect(geo.view.prompt);
    let bar = Rect {
        x: list.right(),
        y: list.y,
        width: 1,
        height: list.height,
    };
    let mut sessions = HashMap::new();
    let mut dirty = false;

    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            bar.x,
            bar.bottom().saturating_sub(1),
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );

    assert_eq!(state.todo.scroll(), 0, "隐藏的 Todo 列表不得被滚动");
    assert!(state.todo_scroll_drag.is_none(), "也不得开始 Todo 拖拽");
}
