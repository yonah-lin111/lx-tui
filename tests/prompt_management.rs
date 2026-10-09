//! 端到端集成测试：工作区信任、提示词文件夹嵌套约束、条目编辑持久化与工作区隔离。
//!
//! 全程驱动公开的 app 行为与持久化服务（不依赖终端）：
//! 未信任工作区 → 确认信任 → 新建文件夹 → 嵌套子文件夹 → 第三层被阻止 →
//! 新建提示词 → 编辑并保存 → 重开数据库验证持久化 → 工作区之间树相互隔离。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use lx_tui::app::actions::{EditorCommand, OverlayKey};
use lx_tui::app::overlay::{MenuCommand, Overlay, OverlayTarget};
use lx_tui::app::prompt::PromptMode;
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
fn trust_folder_nesting_prompt_persistence_and_workspace_isolation() {
    let root = temp_root("main");
    let workspace_a = root.join("workspace-a");
    let workspace_b = root.join("workspace-b");
    std::fs::create_dir_all(&workspace_a).expect("create ws a");
    std::fs::create_dir_all(&workspace_b).expect("create ws b");
    let workspace_a = std::fs::canonicalize(&workspace_a).expect("canonical a");
    let workspace_b = std::fs::canonicalize(&workspace_b).expect("canonical b");
    let db_path = root.join("db").join("lx.db");

    // 1. 未信任的新工作区：启动即弹出信任浮层，拒绝/信任前的树不可用。
    let database = Database::open(&db_path).expect("db opens");
    let mut state = state_with_cwd(&workspace_a);
    let mut service = Persistence::with_database(&mut state, database);
    assert!(
        matches!(state.overlay, Some(Overlay::TrustWorkspace(_))),
        "未信任启动弹信任浮层"
    );

    // 2. 确认信任：录入 DB 并继续。
    update::apply_overlay_key(&mut state, OverlayKey::Enter);
    service.pump(&mut state);
    assert!(state.overlay.is_none());
    let reader = Database::open(&db_path).expect("reader opens");
    assert!(
        reader.is_trusted(&workspace_a).expect("trust query"),
        "信任已持久化"
    );

    // 3. 切到树视图，空白菜单新建文件夹。
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
    let prompt_id = state
        .active_workspace()
        .prompt_panel
        .editing
        .expect("bound");
    assert_eq!(
        state
            .active_workspace()
            .prompt_panel
            .tree
            .item(prompt_id)
            .map(|item| item.folder_id),
        Some(Some(sub_folder))
    );

    // 7. 编辑内容并保存（Esc 返回树视图提交）；DB 收到内容。
    update::apply_editor(&mut state, EditorCommand::InsertText("hello world".into()));
    update::show_prompt_tree(&mut state);
    service.pump(&mut state);
    let records = reader.list_prompts(&workspace_a).expect("list prompts");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].content, "hello world");

    // 8. 工作区隔离：新建第二个工作区，树独立加载且为空；a 的条目不受影响。
    update::create_workspace_with_cwd(&mut state, Some(workspace_b.clone()));
    service.pump(&mut state);
    assert_eq!(
        state.active_workspace().cwd.as_deref(),
        Some(workspace_b.as_path())
    );
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
    // 切回 a：树与内容原样。
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
    let mut service = Persistence::with_database(&mut reloaded, database);
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
