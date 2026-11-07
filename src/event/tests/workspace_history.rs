use super::test_support::*;
use super::*;
use crate::app::effects::Effect;
use crate::app::overlay::{ConfirmClose, MenuCommand, OverlayTarget, TextInput};
use crate::app::state::{WorkspaceHistoryEntry, WorkspaceMode};
use crate::db::{Database, service::Persistence};

/// 左键单击：浮层按钮与列表行的点击都在按下时处理。
fn click(
    state: &mut AppState,
    geo: &crate::event::session::Geometry,
    config: &Config,
    point: (u16, u16),
) {
    press_mouse(state, geo, config, MouseButton::Left, point);
}

/// 右键单击：打开列表/窗口的上下文菜单。
fn right_click(
    state: &mut AppState,
    geo: &crate::event::session::Geometry,
    config: &Config,
    point: (u16, u16),
) {
    press_mouse(state, geo, config, MouseButton::Right, point);
}

/// 当前上下文菜单第 `index` 项的矩形（确认态文案变化后需重新计算）。
fn menu_item_rect(state: &AppState, screen: Rect, index: usize) -> Rect {
    let Some(Overlay::Menu(menu)) = state.overlay.as_ref() else {
        panic!("context menu expected");
    };
    ui::overlay::menu_layout(state, screen, menu)
        .item_rects
        .get(index)
        .copied()
        .expect("menu item visible")
}

fn press_mouse(
    state: &mut AppState,
    geo: &crate::event::session::Geometry,
    config: &Config,
    button: MouseButton,
    point: (u16, u16),
) {
    let mut sessions = HashMap::new();
    let mut dirty = false;
    handle_terminal_event(
        mouse(MouseEventKind::Down(button), point.0, point.1),
        state,
        &mut sessions,
        geo,
        config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
}

/// 纵向滚轮一格：走事件主循环的鼠标滚轮分派。
fn wheel(
    state: &mut AppState,
    geo: &crate::event::session::Geometry,
    config: &Config,
    kind: MouseEventKind,
    point: (u16, u16),
) {
    let mut sessions = HashMap::new();
    let mut dirty = false;
    handle_terminal_event(
        mouse(kind, point.0, point.1),
        state,
        &mut sessions,
        geo,
        config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
}

/// 按下单个无修饰键。
fn press(
    state: &mut AppState,
    geo: &crate::event::session::Geometry,
    config: &Config,
    code: KeyCode,
) {
    let mut sessions = HashMap::new();
    let mut dirty = false;
    handle_terminal_event(
        TerminalEvent::Key(KeyEvent::new(code, KeyModifiers::NONE)),
        state,
        &mut sessions,
        geo,
        config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
}

/// 逐字符键入（走浮层单行输入路由）。
fn type_text(
    state: &mut AppState,
    geo: &crate::event::session::Geometry,
    config: &Config,
    text: &str,
) {
    for ch in text.chars() {
        press(state, geo, config, KeyCode::Char(ch));
    }
}

/// 确认弹窗 `[Confirm]` 按钮中心坐标（按几何反查，避免硬编码按钮宽度）。
fn confirm_button_point(shell: &ui::widgets::modal::ModalShell) -> (u16, u16) {
    for row in shell.inner.y..shell.inner.bottom() {
        for column in shell.inner.x..shell.inner.right() {
            if ui::overlay::confirm_button_at(shell, column, row)
                == Some(ui::overlay::ConfirmButton::Confirm)
            {
                return (column, row);
            }
        }
    }
    panic!("confirm button not found");
}

#[test]
fn mouse_click_mode_button_toggles_history_and_typing_filters_rows() {
    let config = Config::default();
    let mut state = demo_terminal();
    let existing = std::env::temp_dir();
    let missing = PathBuf::from("/tmp/lx-event-history-missing-dir-12345");
    update::apply_workspace_history(
        &mut state,
        vec![
            crate::app::state::WorkspaceHistoryEntry {
                name: "AlphaWorkspace".into(),
                path: existing,
                exists: true,
                name_is_manual: true,
                position: 0,
                trusted: true,
                git: None,
            },
            crate::app::state::WorkspaceHistoryEntry {
                name: "BrokenWorkspace".into(),
                path: missing,
                exists: false,
                name_is_manual: true,
                position: 1,
                trusted: true,
                git: None,
            },
        ],
    );
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let mode_btn = ui::workspace_mode_button(&geo.view).expect("mode button");
    let mut sessions = HashMap::new();
    let mut dirty = false;

    // 点击 [≡] 打开历史列表。
    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            mode_btn.x,
            mode_btn.y,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert_eq!(state.workspace_history.mode, WorkspaceMode::History);
    assert!(state.sidebar_focused());
    assert!(dirty);

    // 键入 "broken" 过滤历史列表。
    for ch in "broken".chars() {
        handle_terminal_event(
            crossterm::event::Event::Key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE)),
            &mut state,
            &mut sessions,
            &geo,
            &config,
            &mut ui::RenderCache::default(),
            &mut dirty,
        );
    }
    assert_eq!(state.workspace_history.search.text(), "broken");
    assert_eq!(state.workspace_history.filtered_indices(), vec![1]);

    // 按 Esc 退出历史列表回到默认活跃列表。
    handle_terminal_event(
        crossterm::event::Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert_eq!(state.workspace_history.mode, WorkspaceMode::Active);
}

#[test]
fn double_click_history_row_opens_workspace_and_right_click_missing_cascades_delete() {
    let config = Config::default();
    let mut state = demo_terminal();
    let db = Database::open_in_memory().expect("in-memory db");
    let existing = std::env::temp_dir();
    let missing = PathBuf::from("/tmp/lx-event-history-cascade-del-99999");

    db.record_workspace_open(&existing, "ExistingWS")
        .expect("record existing");
    db.trust_workspace(&existing, "ExistingWS")
        .expect("trust existing");
    db.record_workspace_open(&missing, "MissingWS")
        .expect("record missing");
    db.create_folder(&missing, None, "orphan-folder")
        .expect("create folder");

    let mut persistence = Persistence::with_database(db);
    state.effects.push(Effect::LoadWorkspaceHistory);
    persistence.pump(&mut state);
    assert_eq!(state.workspace_history.entries.len(), 2);

    update::show_workspace_history(&mut state);
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let sections =
        ui::layout::sidebar_sections(geo.view.sidebar, state.agents_collapsed).expect("sections");
    let row0_y = sections.workspaces.y + layout::PROMPT_TREE_HEADER_HEIGHT;
    let row_x = sections.workspaces.x + 5;

    // 右键点击失效路径行（MissingWS 定位其可见行）。
    let missing_vis_row = state
        .workspace_history
        .filtered_indices()
        .iter()
        .position(|&idx| state.workspace_history.entries[idx].path == missing)
        .expect("missing row");
    let missing_y =
        sections.workspaces.y + layout::PROMPT_TREE_HEADER_HEIGHT + missing_vis_row as u16;
    let mut sessions = HashMap::new();
    let mut dirty = false;
    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Right), row_x, missing_y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    let Some(Overlay::Menu(menu)) = state.overlay.as_ref() else {
        panic!("context menu expected");
    };
    assert!(matches!(menu.target, OverlayTarget::WorkspaceHistory(_)));
    assert!(menu.commands.contains(&MenuCommand::DeleteWorkspaceHistory));
    assert!(
        menu.commands
            .contains(&MenuCommand::DeleteMissingWorkspaces)
    );

    // 两次 Down 选中 Delete，第 1 次 Enter 进入行内确认态，第 2 次 Enter 打开口令弹窗。
    press(&mut state, &geo, &config, KeyCode::Down);
    press(&mut state, &geo, &config, KeyCode::Down);
    press(&mut state, &geo, &config, KeyCode::Enter);
    assert!(
        matches!(state.overlay, Some(Overlay::Menu(ref m)) if m.confirming),
        "first enter arms inline confirm"
    );
    press(&mut state, &geo, &config, KeyCode::Enter);
    let Some(Overlay::ConfirmClose(confirm)) = state.overlay.as_ref() else {
        panic!("second enter opens confirmation modal");
    };
    assert_eq!(
        confirm.target,
        OverlayTarget::WorkspaceHistory(missing_entry_index(&state, &missing))
    );

    // 未输入口令就点击 [Confirm]：弹窗保持打开且不执行删除。
    let confirm_point = {
        let Some(Overlay::ConfirmClose(confirm)) = state.overlay.as_ref() else {
            panic!("confirmation modal expected");
        };
        let shell = ui::overlay::confirm_close_layout(&state, geo.screen, confirm)
            .expect("confirm layout")
            .shell;
        confirm_button_point(&shell)
    };
    click(&mut state, &geo, &config, confirm_point);
    assert!(
        matches!(state.overlay, Some(Overlay::ConfirmClose(_))),
        "口令未匹配时 Confirm 点击被拦截"
    );
    assert_eq!(state.workspace_history.entries.len(), 2);
    assert!(
        !persistence
            .database()
            .expect("db")
            .list_folders(&missing)
            .expect("folders")
            .is_empty(),
        "未确认前不删除关联数据"
    );

    // 键入口令后点击 [Confirm]：执行级联删除。
    type_text(&mut state, &geo, &config, "confirm delete");
    assert!(
        matches!(state.overlay.as_ref(), Some(Overlay::ConfirmClose(c)) if c.is_confirmed()),
        "输入口令后确认按钮可用"
    );
    click(&mut state, &geo, &config, confirm_point);
    assert!(state.overlay.is_none());
    persistence.pump(&mut state);
    assert_eq!(state.workspace_history.entries.len(), 1);
    assert!(
        persistence
            .database()
            .expect("db")
            .list_folders(&missing)
            .expect("folders")
            .is_empty(),
        "associated folders cascaded on workspace delete"
    );

    // 双击剩余的 ExistingWS 行（第 0 行）：打开该工作区并自动返回 Active 模式。
    for _ in 0..2 {
        click(&mut state, &geo, &config, (row_x, row0_y));
    }
    assert_eq!(state.workspace_history.mode, WorkspaceMode::Active);
    assert_eq!(
        state.active_workspace().cwd.as_deref(),
        Some(existing.as_path())
    );
}

/// 失效条目在 `entries` 中的原始下标（确认浮层 target 断言用）。
fn missing_entry_index(state: &AppState, path: &std::path::Path) -> usize {
    state
        .workspace_history
        .entries
        .iter()
        .position(|entry| entry.path == path)
        .expect("missing entry")
}

#[test]
fn right_click_blank_delete_missing_removes_only_missing_entries() {
    let config = Config::default();
    let mut state = demo_terminal();
    let db = Database::open_in_memory().expect("in-memory db");
    let existing = std::env::temp_dir();
    let missing_a = PathBuf::from("/tmp/lx-event-history-missing-a-99999");
    let missing_b = PathBuf::from("/tmp/lx-event-history-missing-b-99999");
    for (path, name) in [
        (&existing, "ExistingWS"),
        (&missing_a, "MissingA"),
        (&missing_b, "MissingB"),
    ] {
        db.record_workspace_open(path, name)
            .expect("record workspace");
    }

    let mut persistence = Persistence::with_database(db);
    state.effects.push(Effect::LoadWorkspaceHistory);
    persistence.pump(&mut state);
    assert_eq!(state.workspace_history.entries.len(), 3);

    update::show_workspace_history(&mut state);
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let sections =
        ui::layout::sidebar_sections(geo.view.sidebar, state.agents_collapsed).expect("sections");

    // 右键点击列表空白处（条目行之下）。
    let blank_y = sections.workspaces.bottom().saturating_sub(1);
    right_click(
        &mut state,
        &geo,
        &config,
        (sections.workspaces.x + 5, blank_y),
    );
    let Some(Overlay::Menu(menu)) = state.overlay.as_ref() else {
        panic!("context menu expected");
    };
    assert!(matches!(menu.target, OverlayTarget::WorkspaceList));
    assert_eq!(
        menu.commands,
        vec![
            MenuCommand::NewWorkspace,
            MenuCommand::DeleteMissingWorkspaces
        ]
    );

    // 第 1 击：行内确认；第 2 击：打开批量清理确认浮层。
    let first = menu_item_rect(&state, geo.screen, 1);
    click(&mut state, &geo, &config, (first.x + 1, first.y));
    assert!(
        matches!(state.overlay, Some(Overlay::Menu(ref m)) if m.confirming),
        "first click arms inline confirm"
    );
    let second = menu_item_rect(&state, geo.screen, 1);
    click(&mut state, &geo, &config, (second.x + 1, second.y));
    assert!(
        matches!(
            state.overlay.as_ref(),
            Some(Overlay::ConfirmClose(c)) if c.target == OverlayTarget::MissingWorkspaces
        ),
        "second click opens missing-workspaces confirmation"
    );

    // 未输入口令：Enter 被拦截，不删除任何条目。
    press(&mut state, &geo, &config, KeyCode::Enter);
    assert!(matches!(state.overlay, Some(Overlay::ConfirmClose(_))));
    assert_eq!(state.workspace_history.entries.len(), 3);

    // 输入口令后回车：只清理失效条目。
    type_text(&mut state, &geo, &config, "confirm delete");
    press(&mut state, &geo, &config, KeyCode::Enter);
    assert!(state.overlay.is_none());
    persistence.pump(&mut state);
    let remaining: Vec<_> = state
        .workspace_history
        .entries
        .iter()
        .map(|entry| entry.path.clone())
        .collect();
    assert_eq!(remaining, vec![existing.clone()]);
    let db_paths: Vec<_> = persistence
        .database()
        .expect("db")
        .list_workspace_history_entries()
        .expect("history entries")
        .into_iter()
        .map(|entry| entry.path)
        .collect();
    assert_eq!(db_paths, vec![existing]);
}

#[test]
fn confirm_close_list_scrolls_with_wheel_and_deletes_all_missing() {
    let config = Config::default();
    let mut state = demo_terminal();
    let db = Database::open_in_memory().expect("in-memory db");
    let existing = std::env::temp_dir();
    db.record_workspace_open(&existing, "ExistingWS")
        .expect("record existing");
    let missing: Vec<PathBuf> = (0..8)
        .map(|index| PathBuf::from(format!("/tmp/lx-event-history-scroll-{index}-99999")))
        .collect();
    for (index, path) in missing.iter().enumerate() {
        db.record_workspace_open(path, &format!("Missing{index}"))
            .expect("record missing");
    }

    let mut persistence = Persistence::with_database(db);
    state.effects.push(Effect::LoadWorkspaceHistory);
    persistence.pump(&mut state);
    assert_eq!(state.workspace_history.entries.len(), 9);

    update::show_workspace_history(&mut state);
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 40));
    let sections =
        ui::layout::sidebar_sections(geo.view.sidebar, state.agents_collapsed).expect("sections");

    // 右键任一条目（第 0 行）→ 2 次点击 `Delete missing` 打开确认模态。
    let row_y = sections.workspaces.y + layout::PROMPT_TREE_HEADER_HEIGHT;
    right_click(
        &mut state,
        &geo,
        &config,
        (sections.workspaces.x + 5, row_y),
    );
    let delete_missing = {
        let Some(Overlay::Menu(menu)) = state.overlay.as_ref() else {
            panic!("context menu expected");
        };
        menu.commands
            .iter()
            .position(|command| *command == MenuCommand::DeleteMissingWorkspaces)
            .expect("delete missing item")
    };
    let first = menu_item_rect(&state, geo.screen, delete_missing);
    click(&mut state, &geo, &config, (first.x + 1, first.y));
    let second = menu_item_rect(&state, geo.screen, delete_missing);
    click(&mut state, &geo, &config, (second.x + 1, second.y));

    let (list_point, max_scroll) = {
        let Some(Overlay::ConfirmClose(confirm)) = state.overlay.as_ref() else {
            panic!("confirmation modal expected");
        };
        assert_eq!(confirm.target, OverlayTarget::MissingWorkspaces);
        assert_eq!(confirm.scroll, 0);
        let layout =
            ui::overlay::confirm_close_layout(&state, geo.screen, confirm).expect("confirm layout");
        assert_eq!(layout.visible_rows, 5);
        let list = layout.list.expect("workspace list");
        ((list.x + 1, list.y), missing.len() - layout.visible_rows)
    };
    assert_eq!(max_scroll, 3);

    // 模态内滚轮滚动列表视口，弹窗保持打开。
    wheel(
        &mut state,
        &geo,
        &config,
        MouseEventKind::ScrollDown,
        list_point,
    );
    let Some(Overlay::ConfirmClose(confirm)) = state.overlay.as_ref() else {
        panic!("wheel must not dismiss the modal");
    };
    assert_eq!(confirm.scroll, max_scroll);
    wheel(
        &mut state,
        &geo,
        &config,
        MouseEventKind::ScrollUp,
        list_point,
    );
    let Some(Overlay::ConfirmClose(confirm)) = state.overlay.as_ref() else {
        panic!("wheel must not dismiss the modal");
    };
    assert_eq!(confirm.scroll, 0);

    // 输入口令后回车：全部失效条目被清理，磁盘仍存在的条目保留。
    type_text(&mut state, &geo, &config, "confirm delete");
    press(&mut state, &geo, &config, KeyCode::Enter);
    assert!(state.overlay.is_none());
    persistence.pump(&mut state);
    let remaining: Vec<_> = state
        .workspace_history
        .entries
        .iter()
        .map(|entry| entry.path.clone())
        .collect();
    assert_eq!(remaining, vec![existing]);
}

/// 4 条失效历史条目：确认浮层列表测试的固定夹具。
fn missing_history_entries() -> Vec<WorkspaceHistoryEntry> {
    (0..4)
        .map(|index| WorkspaceHistoryEntry {
            name: format!("Missing{index}"),
            path: PathBuf::from(format!("/tmp/lx-event-missing-{index}-99999")),
            exists: false,
            name_is_manual: true,
            position: index,
            trusted: true,
            git: None,
        })
        .collect()
}

/// 确认浮层当前滚动偏移。
fn confirm_scroll(state: &AppState) -> usize {
    match state.overlay.as_ref() {
        Some(Overlay::ConfirmClose(confirm)) => confirm.scroll,
        other => panic!("confirmation modal expected, got {other:?}"),
    }
}

#[test]
fn confirm_close_wheel_reaches_tail_on_small_screen() {
    let config = Config::default();
    let mut state = demo_terminal();
    state.workspace_history.entries = missing_history_entries();
    state.overlay = Some(Overlay::ConfirmClose(ConfirmClose::new(
        OverlayTarget::MissingWorkspaces,
    )));
    let geo = geometry(&state, &config, Rect::new(0, 0, 80, 11));

    let (list_point, max_scroll) = {
        let Some(Overlay::ConfirmClose(confirm)) = state.overlay.as_ref() else {
            panic!("confirmation modal expected");
        };
        let layout =
            ui::overlay::confirm_close_layout(&state, geo.screen, confirm).expect("confirm layout");
        assert!(
            layout.visible_rows < crate::app::overlay::CONFIRM_DELETE_LIST_MAX_ROWS,
            "small screen must shrink the list viewport"
        );
        let list = layout.list.expect("workspace list");
        (
            (list.x + 1, list.y),
            state
                .workspace_history
                .target_delete_entries(&confirm.target)
                .len()
                - layout.visible_rows,
        )
    };
    assert_eq!(max_scroll, 2);

    // 旧实现按固定 5 行夹取，max_scroll = 0：这里会停在顶部。
    wheel(
        &mut state,
        &geo,
        &config,
        MouseEventKind::ScrollDown,
        list_point,
    );
    assert_eq!(confirm_scroll(&state), max_scroll);
    wheel(
        &mut state,
        &geo,
        &config,
        MouseEventKind::ScrollUp,
        list_point,
    );
    assert_eq!(confirm_scroll(&state), 0);
}

#[test]
fn mouse_down_clears_stale_confirm_scroll_drag_and_selects_input() {
    let config = Config::default();
    let mut state = demo_terminal();
    state.workspace_history.entries = missing_history_entries();
    let mut confirm = ConfirmClose::new(OverlayTarget::MissingWorkspaces);
    confirm.input = TextInput::new("confirm delete");
    confirm.scroll_drag = Some(0);
    state.overlay = Some(Overlay::ConfirmClose(confirm));
    // 模拟滚动条拖拽中指针移出窗口丢失 MouseUp：两个残留标志都不该劫持下一次按下。
    state.overlay_input_drag = true;
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let input = {
        let Some(Overlay::ConfirmClose(confirm)) = state.overlay.as_ref() else {
            panic!("confirmation modal expected");
        };
        ui::overlay::confirm_close_layout(&state, geo.screen, confirm)
            .expect("confirm layout")
            .input
    };

    press_mouse(
        &mut state,
        &geo,
        &config,
        MouseButton::Left,
        (input.x + 2, input.y),
    );
    let Some(Overlay::ConfirmClose(confirm)) = state.overlay.as_ref() else {
        panic!("confirmation modal expected");
    };
    assert!(confirm.scroll_drag.is_none(), "stale thumb drag must clear");
    assert!(state.overlay_input_drag, "input drag starts on press");

    let mut sessions = HashMap::new();
    let mut dirty = false;
    handle_terminal_event(
        mouse(
            MouseEventKind::Drag(MouseButton::Left),
            input.x + 8,
            input.y,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    let Some(Overlay::ConfirmClose(confirm)) = state.overlay.as_ref() else {
        panic!("confirmation modal expected");
    };
    assert!(
        confirm.input.selection().is_some(),
        "text selection must extend after the stale drag flag is cleared"
    );
}

/// `git` 可用时才执行 worktree 相关用例。
fn git_available() -> bool {
    std::process::Command::new("git")
        .arg("--version")
        .output()
        .is_ok_and(|out| out.status.success())
}

/// 在指定仓库中执行 git（失败即断言）。
fn run_git(repo: &std::path::Path, args: &[&str]) {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .expect("run git");
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn deleted_missing_linked_worktree_stays_hidden_after_reload() {
    if !git_available() {
        return;
    }
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let base = std::env::temp_dir().join(format!("lx-event-missing-wt-{nanos}"));
    std::fs::create_dir_all(&base).expect("base dir");
    let base = std::fs::canonicalize(&base).expect("canon base");
    let repo = base.join("repo");
    let wt = repo.join(".worktrees").join("gone");
    std::fs::create_dir_all(&repo).expect("repo dir");
    run_git(&repo, &["init", "--quiet", "-b", "dev"]);
    run_git(
        &repo,
        &[
            "-c",
            "user.email=test@example.invalid",
            "-c",
            "user.name=Test",
            "commit",
            "--quiet",
            "--allow-empty",
            "-m",
            "init",
        ],
    );
    run_git(
        &repo,
        &[
            "worktree",
            "add",
            "--quiet",
            "-b",
            "feature/gone",
            wt.to_str().expect("utf8"),
        ],
    );
    let canon_wt = std::fs::canonicalize(&wt).expect("canon worktree");

    let db = Database::open_in_memory().expect("in-memory db");
    db.record_workspace_open(&repo, "Repo")
        .expect("record repo");
    db.trust_workspace(&repo, "Repo").expect("trust repo");
    db.record_workspace_open(&canon_wt, "gone")
        .expect("record worktree");
    let mut persistence = Persistence::with_database(db);
    let mut state = demo_terminal();
    state.effects.push(Effect::LoadWorkspaceHistory);
    persistence.pump(&mut state);
    assert!(
        state.workspace_history.entries.iter().any(|entry| {
            entry.path == canon_wt && entry.git.as_ref().is_some_and(|git| git.is_linked)
        }),
        "linked worktree must load as a grouped child"
    );

    // 模拟 worktree 目录被外部删除但主仓库尚未执行 `git worktree prune`：
    // `git worktree list` 仍会列出该 prunable 条目。
    std::fs::remove_dir_all(&canon_wt).expect("remove worktree dir");

    let config = Config::default();
    update::show_workspace_history(&mut state);
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let sections =
        ui::layout::sidebar_sections(geo.view.sidebar, state.agents_collapsed).expect("sections");
    let row_y = sections.workspaces.y + layout::PROMPT_TREE_HEADER_HEIGHT;
    right_click(
        &mut state,
        &geo,
        &config,
        (sections.workspaces.x + 5, row_y),
    );
    let delete_missing = {
        let Some(Overlay::Menu(menu)) = state.overlay.as_ref() else {
            panic!("context menu expected");
        };
        menu.commands
            .iter()
            .position(|command| *command == MenuCommand::DeleteMissingWorkspaces)
            .expect("delete missing item")
    };
    let first = menu_item_rect(&state, geo.screen, delete_missing);
    click(&mut state, &geo, &config, (first.x + 1, first.y));
    let second = menu_item_rect(&state, geo.screen, delete_missing);
    click(&mut state, &geo, &config, (second.x + 1, second.y));
    type_text(&mut state, &geo, &config, "confirm delete");
    press(&mut state, &geo, &config, KeyCode::Enter);
    assert!(state.overlay.is_none());
    persistence.pump(&mut state);
    assert!(
        !state
            .workspace_history
            .entries
            .iter()
            .any(|entry| entry.path == canon_wt)
    );

    let hidden = persistence
        .database()
        .expect("db")
        .hidden_workspace_paths()
        .expect("hidden workspaces");
    assert!(
        hidden.contains(&canon_wt),
        "missing linked worktree must be persisted in workspace_hidden"
    );

    // 清空内存隐藏集后重载：SQLite 的 workspace_hidden 必须阻止 prunable 条目复活。
    state.workspace_history.hidden_worktrees.clear();
    state.effects.push(Effect::LoadWorkspaceHistory);
    persistence.pump(&mut state);
    assert!(
        !state
            .workspace_history
            .entries
            .iter()
            .any(|entry| entry.path == canon_wt),
        "deleted missing linked worktree must not resurrect after reload"
    );
    let _ = std::fs::remove_dir_all(&base);
}
