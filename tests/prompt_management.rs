//! 端到端集成测试：启动信任门、应用内信任弹窗、文件夹嵌套约束、条目持久化与工作区隔离。
//!
//! 启动信任门（进入应用前的系统控制台询问）由 `tests/trust_prompt.rs` 在真实 PTY 中覆盖；
//! 本用例覆盖已放行后的完整业务流与应用内信任弹窗（新建未信任工作区）。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use lx_tui::app::actions::{EditorCommand, OverlayKey};
use lx_tui::app::overlay::{MenuCommand, NewWorkspace, Overlay, OverlayTarget, TextInput};
use lx_tui::app::prompt::{PromptEditing, PromptMode};
use lx_tui::app::state::AppState;
use lx_tui::app::update;
use lx_tui::config::Config;
use lx_tui::db::Database;
use lx_tui::db::service::Persistence;
use lx_tui::ui;
use ratatui::Terminal;
use ratatui::backend::TestBackend;

/// 独立临时根目录；每轮测试互不共享。
fn temp_root(label: &str) -> PathBuf {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or(0);
    let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("lx-tui-e2e-{label}-{nanos:x}-{sequence:x}"))
}

/// 构造指向给定 cwd 的初始状态。
fn state_with_cwd(cwd: &Path) -> AppState {
    let mut state = AppState::demo();
    state.active_workspace_mut().cwd = Some(cwd.to_path_buf());
    state
}

/// 执行一次菜单命令（打开对应目标菜单后激活指定项）。
fn run_menu_command(state: &mut AppState, target: OverlayTarget, command: MenuCommand) {
    let anchor = (0, 0);
    match target {
        OverlayTarget::PromptTree { .. } => update::open_prompt_tree_menu(state, anchor),
        OverlayTarget::PromptFolder { folder_id, .. } => {
            update::open_prompt_folder_menu(state, folder_id, anchor)
        }
        OverlayTarget::PromptItem { item_id, .. } => {
            update::open_prompt_item_menu(state, item_id, anchor)
        }
        _ => panic!("unsupported target"),
    }
    let Some(Overlay::Menu(menu)) = state.overlay.as_mut() else {
        panic!("menu open");
    };
    let index = menu
        .commands
        .iter()
        .position(|candidate| *candidate == command)
        .expect("command listed");
    menu.selected = index;
    update::activate_menu(state);
}

/// 渲染一帧，确保树视图可正常绘制（无 panic）。
fn draw_tree(state: &AppState) {
    let config = Config::default();
    let mut terminal =
        Terminal::new(TestBackend::new(100, 24)).expect("test backend is infallible");
    terminal
        .draw(|frame| ui::render(frame, state, &config))
        .expect("draw succeeds");
    let buffer = terminal.backend().buffer();
    assert!(
        buffer.area.width > 0 && buffer.area.height > 0,
        "frame rendered"
    );
}

#[test]
fn in_app_trust_dialog_folder_nesting_persistence_and_workspace_isolation() {
    let root = temp_root("main");
    let workspace_a = root.join("workspace-a");
    let workspace_b = root.join("workspace-b");
    std::fs::create_dir_all(&workspace_a).expect("create ws a");
    std::fs::create_dir_all(&workspace_b).expect("create ws b");
    let workspace_a = std::fs::canonicalize(&workspace_a).expect("canonical a");
    let workspace_b = std::fs::canonicalize(&workspace_b).expect("canonical b");
    let db_path = root.join("db").join("lx.db");

    // 1. 启动信任门放行工作区 a（控制台询问由 tests/trust_prompt.rs 覆盖）。
    let database = Database::open(&db_path).expect("db opens");
    database
        .trust_workspace(&workspace_a, "workspace-a")
        .expect("a trusted");
    let mut state = state_with_cwd(&workspace_a);
    let mut service = Persistence::with_database(database);
    assert_eq!(
        service.untrusted_startup_path(&state),
        None,
        "已信任工作区无启动信任页"
    );
    assert!(state.overlay.is_none());

    // 2. 应用内信任弹窗：显式新建未信任工作区 b → 弹信任浮层 → 确认后创建并入库。
    state.overlay = Some(Overlay::NewWorkspace(NewWorkspace {
        input: TextInput::new(workspace_b.to_string_lossy()),
    }));
    update::apply_overlay_key(&mut state, OverlayKey::Enter);
    assert!(state.overlay.is_none(), "新建弹窗先关闭");
    service.pump(&mut state);
    assert!(
        matches!(state.overlay, Some(Overlay::TrustWorkspace(_))),
        "未信任路径弹信任浮层"
    );
    assert_eq!(state.workspaces.len(), 1, "确认前不创建");
    update::apply_overlay_key(&mut state, OverlayKey::Enter);
    service.pump(&mut state);
    assert_eq!(state.workspaces.len(), 2, "信任后创建工作区");
    assert_eq!(
        state.workspaces[1].cwd.as_deref(),
        Some(workspace_b.as_path())
    );
    let reader = Database::open(&db_path).expect("reader opens");
    assert!(
        reader.is_trusted(&workspace_b).expect("trust query"),
        "信任已持久化"
    );

    // 3. 回到工作区 a，切到树视图，空白菜单新建文件夹。
    update::switch_workspace(&mut state, 0);
    update::show_prompt_tree(&mut state);
    assert_eq!(state.active_workspace().prompt_panel.mode, PromptMode::Tree);
    run_menu_command(
        &mut state,
        OverlayTarget::PromptTree { workspace: 0 },
        MenuCommand::NewFolder,
    );
    service.pump(&mut state);
    let root_folder = state.active_workspace().prompt_panel.tree.folders()[0].id;

    // 4. 根文件夹下新建子文件夹（第 2 层允许）。
    run_menu_command(
        &mut state,
        OverlayTarget::PromptFolder {
            workspace: 0,
            folder_id: root_folder,
        },
        MenuCommand::NewSubFolder,
    );
    service.pump(&mut state);
    assert_eq!(
        state.active_workspace().prompt_panel.tree.folders().len(),
        2
    );
    let sub_folder = state.active_workspace().prompt_panel.tree.folders()[1].id;

    // 5. 第 3 层被阻止：子文件夹菜单不展示新建子文件夹，服务层同样拒绝。
    update::open_prompt_folder_menu(&mut state, sub_folder, (0, 0));
    let Some(Overlay::Menu(menu)) = &state.overlay else {
        panic!("menu open");
    };
    assert!(
        !menu.commands.contains(&MenuCommand::NewSubFolder),
        "第 2 层不展示新建子文件夹"
    );
    update::close_overlay(&mut state);
    update::create_folder(&mut state, Some(sub_folder));
    service.pump(&mut state);
    assert_eq!(
        state.active_workspace().prompt_panel.tree.folders().len(),
        2,
        "第 3 层文件夹被拒绝"
    );

    // 6. 在子文件夹内新建提示词：进入编辑视图并绑定条目。
    run_menu_command(
        &mut state,
        OverlayTarget::PromptFolder {
            workspace: 0,
            folder_id: sub_folder,
        },
        MenuCommand::NewPrompt,
    );
    service.pump(&mut state);
    assert_eq!(
        state.active_workspace().prompt_panel.mode,
        PromptMode::Editor
    );
    let prompt_id = match state.active_workspace().prompt_panel.editing {
        Some(PromptEditing::Item(id)) => id,
        other => panic!("bound item expected, got {other:?}"),
    };
    assert_eq!(
        state
            .active_workspace()
            .prompt_panel
            .tree
            .item(prompt_id)
            .map(|item| item.folder_id),
        Some(Some(sub_folder))
    );

    // 7. 编辑内容并保存（返回树视图提交）；DB 收到内容。
    update::apply_editor(&mut state, EditorCommand::InsertText("hello world".into()));
    update::show_prompt_tree(&mut state);
    service.pump(&mut state);
    let records = reader.list_prompts(&workspace_a).expect("list prompts");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].content, "hello world");

    // 8. 工作区隔离：切到 b，树独立加载且为空；在 b 新建条目互不影响。
    update::switch_workspace(&mut state, 1);
    service.pump(&mut state);
    assert!(
        state
            .active_workspace()
            .prompt_panel
            .tree
            .items()
            .is_empty(),
        "b 工作区树为空"
    );
    update::create_prompt(&mut state, None);
    service.pump(&mut state);
    update::switch_workspace(&mut state, 0);
    service.pump(&mut state);
    assert_eq!(state.active_workspace().prompt_panel.tree.items().len(), 1);
    assert_eq!(
        state
            .active_workspace()
            .prompt_panel
            .tree
            .item(prompt_id)
            .map(|item| item.content.as_str()),
        Some("hello world")
    );

    // 9. 渲染树视图一帧，确认面板在树模式下可绘制。
    update::show_prompt_tree(&mut state);
    draw_tree(&state);

    // 10. 重开数据库（模拟重启）后树与内容仍在。
    drop(service);
    drop(state);
    let database = Database::open(&db_path).expect("reopen db");
    let mut reloaded = state_with_cwd(&workspace_a);
    let mut service = Persistence::with_database(database);
    assert_eq!(service.untrusted_startup_path(&reloaded), None);
    service.pump(&mut reloaded);
    let items = reloaded.active_workspace().prompt_panel.tree.items();
    assert_eq!(items.len(), 1, "重启后提示词持久化");
    assert_eq!(items[0].content, "hello world");
    // 工作区 b 的树也独立持久化（一条新建的默认名提示词）。
    assert_eq!(
        reader.list_prompts(&workspace_b).expect("list b").len(),
        1,
        "b 工作区独立保存"
    );
}

#[test]
fn draft_prompt_saved_to_new_folder_end_to_end() {
    let root = temp_root("draft");
    let workspace = root.join("workspace");
    std::fs::create_dir_all(&workspace).expect("create workspace");
    let workspace = std::fs::canonicalize(&workspace).expect("canonical");
    let db_path = root.join("db").join("lx.db");
    let database = Database::open(&db_path).expect("db opens");
    database
        .trust_workspace(&workspace, "workspace")
        .expect("trusted");
    let mut state = state_with_cwd(&workspace);
    let mut service = Persistence::with_database(database);
    service.pump(&mut state);

    // 空树 [edit] 回退打开草稿（不再无响应）；工具栏 [temp prompt] 同样进入草稿编辑。
    update::show_prompt_tree(&mut state);
    update::toggle_prompt_panel_mode(&mut state);
    assert_eq!(
        state.active_workspace().prompt_panel.mode,
        PromptMode::Editor
    );
    assert_eq!(
        state.active_workspace().prompt_panel.editing,
        Some(PromptEditing::Draft)
    );

    // 编辑草稿 → 回树（内容保留在内存槽位，不入库）。
    update::apply_editor(&mut state, EditorCommand::InsertText("draft body".into()));
    update::show_prompt_tree(&mut state);
    assert_eq!(state.active_workspace().prompt_panel.draft, "draft body");
    assert!(
        state
            .active_workspace()
            .prompt_panel
            .tree
            .items()
            .is_empty(),
        "草稿不入库"
    );

    // 保存选择器：名称预填默认名；根层 + 新建文件夹动作行。
    update::open_save_draft(&mut state);
    let Some(Overlay::SaveDraft(dialog)) = &state.overlay else {
        panic!("save picker open");
    };
    assert_eq!(dialog.entries.len(), 2, "根层 + 新建文件夹");
    // 选中动作行回车 → 内联新建行追加并选中；名称直接在该行输入。
    update::set_save_draft_selection(&mut state, 1);
    update::apply_overlay_key(&mut state, OverlayKey::Enter);
    update::apply_overlay_key(&mut state, OverlayKey::Clear);
    for ch in "docs".chars() {
        update::apply_overlay_key(&mut state, OverlayKey::Char(ch));
    }
    // 再回车（确定）执行保存：先建文件夹再存入草稿。
    update::apply_overlay_key(&mut state, OverlayKey::Enter);
    service.pump(&mut state);

    // 保存成功：文件夹与条目写入 DB，草稿清空，编辑器绑定新条目。
    let panel = &state.active_workspace().prompt_panel;
    assert!(panel.draft.is_empty());
    assert_eq!(panel.mode, PromptMode::Editor);
    assert_eq!(panel.tree.folders().len(), 1);
    let folder = panel.tree.folders()[0].clone();
    assert_eq!(folder.name, "docs");
    let item = &panel.tree.items()[0];
    assert_eq!(item.folder_id, Some(folder.id));
    assert_eq!(item.content, "draft body");
    assert_eq!(panel.editing, Some(PromptEditing::Item(item.id)));

    // 重启后草稿保存的文件夹与条目仍在。
    drop(service);
    drop(state);
    let database = Database::open(&db_path).expect("reopen db");
    let mut reloaded = state_with_cwd(&workspace);
    let mut service = Persistence::with_database(database);
    service.pump(&mut reloaded);
    let panel = &reloaded.active_workspace().prompt_panel;
    assert_eq!(panel.tree.folders().len(), 1);
    assert_eq!(panel.tree.items()[0].content, "draft body");
}
