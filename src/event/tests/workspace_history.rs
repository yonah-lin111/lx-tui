use super::test_support::*;
use super::*;
use crate::app::effects::Effect;
use crate::app::overlay::{MenuCommand, OverlayTarget};
use crate::app::state::WorkspaceMode;
use crate::db::{Database, service::Persistence};

#[test]
fn mouse_click_mode_button_toggles_history_and_typing_filters_rows() {
    let config = Config::default();
    let mut state = demo_terminal();
    let existing = std::env::temp_dir();
    let missing = PathBuf::from("/tmp/lx-event-history-missing-dir-12345");
    update::apply_workspace_history(
        &mut state,
        vec![
            ("AlphaWorkspace".into(), existing),
            ("BrokenWorkspace".into(), missing),
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
    let row1_y = row0_y + 1;
    let row_x = sections.workspaces.x + 2;
    let mut sessions = HashMap::new();
    let mut dirty = false;

    // 右键点击失效路径行（MissingWS 在列表第 0 或第 1 行，定位其可见行）。
    let missing_vis_row = state
        .workspace_history
        .filtered_indices()
        .iter()
        .position(|&idx| state.workspace_history.entries[idx].path == missing)
        .expect("missing row");
    let missing_y =
        sections.workspaces.y + layout::PROMPT_TREE_HEADER_HEIGHT + missing_vis_row as u16;
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

    // 通过键盘选中 Delete 并回车两次（行内二次确认 -> 级联删除）。
    handle_terminal_event(
        crossterm::event::Event::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    handle_terminal_event(
        crossterm::event::Event::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    handle_terminal_event(
        crossterm::event::Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(
        matches!(state.overlay, Some(Overlay::Menu(ref m)) if m.confirming),
        "first enter arms inline confirm"
    );
    handle_terminal_event(
        crossterm::event::Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
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
    let _ = row1_y;
    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Left), row_x, row0_y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Left), row_x, row0_y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert_eq!(state.workspace_history.mode, WorkspaceMode::Active);
    assert_eq!(
        state.active_workspace().cwd.as_deref(),
        Some(existing.as_path())
    );
}
