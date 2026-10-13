//! 持久化服务单元测试：启动信任、效应执行与提示词树载入。

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use super::*;
use crate::app::effects::Effect;
use crate::app::prompt::PromptEditing;

/// 独立临时库路径。
fn temp_db_path(label: &str) -> PathBuf {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or(0);
    let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir()
        .join(format!("lx-tui-service-{label}-{nanos:x}-{sequence:x}"))
        .join("lx.db")
}

/// 独立工作区路径（不要求磁盘存在）。
fn workspace_path(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!("lx-tui-service-ws-{label}"))
}

/// demo 状态并把活动工作区 cwd 指向给定路径。
fn state_with_cwd(cwd: &std::path::Path) -> AppState {
    let mut state = AppState::demo();
    state.active_workspace_mut().cwd = Some(cwd.to_path_buf());
    state
}

/// 构造服务：数据库已信任该工作区。
fn trusted_service(label: &str) -> (Persistence, AppState, PathBuf, PathBuf) {
    let db_path = temp_db_path(label);
    let cwd = workspace_path(label);
    let database = Database::open(&db_path).expect("db opens");
    database
        .trust_workspace(&cwd, "workspace")
        .expect("trust recorded");
    let state = state_with_cwd(&cwd);
    let service = Persistence::with_database(database);
    (service, state, cwd, db_path)
}

#[test]
fn untrusted_startup_path_targets_untrusted_workspace() {
    let db_path = temp_db_path("untrusted");
    let cwd = workspace_path("untrusted");
    let database = Database::open(&db_path).expect("db opens");
    let state = state_with_cwd(&cwd);
    let service = Persistence::with_database(database);
    assert_eq!(service.untrusted_startup_path(&state), Some(cwd));
}

#[test]
fn untrusted_startup_path_none_when_trusted() {
    let (service, state, _, _) = trusted_service("startup-trusted");
    assert_eq!(service.untrusted_startup_path(&state), None);
}

#[test]
fn trust_path_records_workspace() {
    let (service, _state, _, _) = trusted_service("trust-path");
    let target = workspace_path("trust-path-new");
    assert!(service.trust_path(&target, "new"));
    assert!(
        service
            .db
            .as_ref()
            .expect("db")
            .is_trusted(&target)
            .expect("query"),
        "信任已写入"
    );
}

#[test]
fn check_trust_effect_creates_workspace_when_trusted() {
    let (mut service, mut state, _cwd, _) = trusted_service("check-trusted");
    let target = workspace_path("check-trusted-new");
    service
        .db
        .as_ref()
        .expect("db")
        .trust_workspace(&target, "new")
        .expect("trust");
    state.effects.push(Effect::CheckTrust {
        path: target.clone(),
        trigger: crate::app::trust::TrustTrigger::CreateWorkspace {
            cwd: target.clone(),
        },
    });
    assert!(service.pump(&mut state));
    assert_eq!(state.workspaces.len(), 2);
    assert_eq!(state.workspaces[1].cwd.as_deref(), Some(target.as_path()));
}

#[test]
fn check_trust_effect_opens_dialog_when_untrusted() {
    let (mut service, mut state, _, _) = trusted_service("check-untrusted");
    let target = workspace_path("check-untrusted-new");
    state.effects.push(Effect::CheckTrust {
        path: target.clone(),
        trigger: crate::app::trust::TrustTrigger::CreateWorkspace { cwd: target },
    });
    assert!(service.pump(&mut state));
    assert!(matches!(
        state.overlay,
        Some(crate::app::overlay::Overlay::TrustWorkspace(_))
    ));
    assert_eq!(state.workspaces.len(), 1);
}

#[test]
fn grant_trust_effect_persists_workspace() {
    let (mut service, mut state, _, _) = trusted_service("grant");
    let target = workspace_path("grant-new");
    state.effects.push(Effect::GrantTrust {
        path: target.clone(),
        name: "grant-new".into(),
    });
    service.pump(&mut state);
    assert!(
        service
            .db
            .as_ref()
            .expect("db")
            .is_trusted(&target)
            .expect("query")
    );
}

#[test]
fn create_folder_and_prompt_effects_reload_tree() {
    let (mut service, mut state, cwd, _) = trusted_service("tree-crud");
    state.effects.push(Effect::CreateFolder {
        workspace: cwd.clone(),
        parent_id: None,
        name: "root".into(),
    });
    assert!(service.pump(&mut state));
    let folder_id = state.workspaces[0].prompt_panel.tree.folders()[0].id;
    // 子文件夹允许；第三层被拒绝且不落库。
    state.effects.push(Effect::CreateFolder {
        workspace: cwd.clone(),
        parent_id: Some(folder_id),
        name: "sub".into(),
    });
    service.pump(&mut state);
    let sub_id = state.workspaces[0].prompt_panel.tree.folders()[1].id;
    state.effects.push(Effect::CreateFolder {
        workspace: cwd.clone(),
        parent_id: Some(sub_id),
        name: "deep".into(),
    });
    service.pump(&mut state);
    assert_eq!(state.workspaces[0].prompt_panel.tree.folders().len(), 2);

    // 新建提示词只从树视图发起：创建后停留在树视图并选中条目。
    crate::app::update::show_prompt_tree(&mut state);
    state.effects.push(Effect::CreatePrompt {
        workspace: cwd.clone(),
        folder_id: Some(folder_id),
        name: "draft".into(),
    });
    assert!(service.pump(&mut state));
    let item = &state.workspaces[0].prompt_panel.tree.items()[0];
    assert_eq!(item.name, "draft");
    assert_eq!(item.folder_id, Some(folder_id));
    assert_eq!(
        state.workspaces[0].prompt_panel.mode,
        crate::app::prompt::PromptMode::Tree,
        "新建提示词停留在树视图"
    );
    assert_eq!(
        state.workspaces[0].prompt_panel.editing, None,
        "新建提示词不进入编辑视图"
    );
    assert_eq!(
        state.workspaces[0].prompt_panel.selected_prompt_id(),
        Some(item.id),
        "新建提示词在列表中被选中"
    );
}

#[test]
fn save_and_status_effects_persist() {
    let (mut service, mut state, cwd, _) = trusted_service("save");
    state.effects.push(Effect::CreatePrompt {
        workspace: cwd.clone(),
        folder_id: None,
        name: "draft".into(),
    });
    service.pump(&mut state);
    let id = state.workspaces[0].prompt_panel.tree.items()[0].id;
    state.effects.push(Effect::SavePromptContent {
        workspace: cwd.clone(),
        id,
        content: "hello".into(),
        prompt: state.prompt.id(),
        revision: state.prompt.revision(),
    });
    state.effects.push(Effect::SetPromptStatus {
        workspace: cwd.clone(),
        id,
        status: crate::app::prompt::PromptItemStatus::Completed,
    });
    service.pump(&mut state);
    let records = service
        .db
        .as_ref()
        .expect("db")
        .list_prompts(&cwd)
        .expect("list");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].content, "hello");
    assert_eq!(records[0].status, "completed");
}

#[test]
fn delete_folder_effect_cascades_in_database() {
    let (mut service, mut state, cwd, _) = trusted_service("cascade");
    state.effects.push(Effect::CreateFolder {
        workspace: cwd.clone(),
        parent_id: None,
        name: "root".into(),
    });
    service.pump(&mut state);
    let root = state.workspaces[0].prompt_panel.tree.folders()[0].id;
    state.effects.push(Effect::CreateFolder {
        workspace: cwd.clone(),
        parent_id: Some(root),
        name: "sub".into(),
    });
    state.effects.push(Effect::CreatePrompt {
        workspace: cwd.clone(),
        folder_id: Some(root),
        name: "draft".into(),
    });
    service.pump(&mut state);
    state.effects.push(Effect::DeleteFolder {
        workspace: cwd.clone(),
        id: root,
    });
    service.pump(&mut state);
    let db = service.db.as_ref().expect("db");
    assert!(db.list_folders(&cwd).expect("list").is_empty());
    assert!(db.list_prompts(&cwd).expect("list").is_empty());
}

#[test]
fn quit_shutdown_commits_editing_prompt() {
    let (mut service, mut state, cwd, _) = trusted_service("shutdown");
    state.effects.push(Effect::CreatePrompt {
        workspace: cwd.clone(),
        folder_id: None,
        name: "draft".into(),
    });
    service.pump(&mut state);
    let id = state.workspaces[0].prompt_panel.tree.items()[0].id;
    assert!(
        crate::app::update::edit_prompt_item(&mut state, 0, id),
        "显式进入编辑视图"
    );
    crate::app::update::apply_editor(
        &mut state,
        crate::app::actions::EditorCommand::InsertText("typed".into()),
    );
    assert_eq!(state.prompt.text(), "typed", "编辑写入显示中缓冲");
    service.shutdown(&mut state);
    let records = service
        .db
        .as_ref()
        .expect("db")
        .list_prompts(&cwd)
        .expect("list");
    assert_eq!(records[0].content, "typed");
}

#[test]
fn tree_loads_on_demand_for_displayed_workspace() {
    let (mut service, mut state, cwd, _) = trusted_service("lazy-load");
    // 预置一条数据但不给面板加载标记。
    service
        .db
        .as_ref()
        .expect("db")
        .create_prompt(&cwd, None, "existing", "")
        .expect("seed");
    assert!(!state.workspaces[0].prompt_panel.loaded);
    service.pump(&mut state);
    assert!(state.workspaces[0].prompt_panel.loaded);
    assert_eq!(state.workspaces[0].prompt_panel.tree.items().len(), 1);
}

#[test]
fn save_draft_effect_persists_content_binds_editor_and_removes_temp() {
    let (mut service, mut state, cwd, db_path) = trusted_service("save-draft");
    let Some(PromptEditing::Temp(temp_id)) = state.workspaces[0].prompt_panel.editing else {
        panic!("temp bound by default");
    };
    state.workspaces[0]
        .prompt_panel
        .temp_mut(temp_id)
        .expect("temp exists")
        .content = "draft body".into();
    state.effects.push(Effect::SaveDraft {
        workspace: cwd.clone(),
        folder_id: None,
        name: "New prompt".into(),
        content: "draft body".into(),
    });
    assert!(service.pump(&mut state));
    let panel = &state.workspaces[0].prompt_panel;
    assert!(panel.temp(temp_id).is_none(), "保存成功后移除已升格的 temp");
    let item = &panel.tree.items()[0];
    assert_eq!(item.content, "draft body");
    assert_eq!(item.folder_id, None);
    assert_eq!(panel.editing, Some(PromptEditing::Item(item.id)));
    drop(service);
    let reader = Database::open(&db_path).expect("reader opens");
    let records = reader.list_prompts(&cwd).expect("list");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].content, "draft body");
}

#[test]
fn create_folder_from_picker_refreshes_entries_and_save_draft_uses_it() {
    let (mut service, mut state, cwd, _) = trusted_service("save-draft-folder");
    service.pump(&mut state);
    let Some(PromptEditing::Temp(temp_id)) = state.workspaces[0].prompt_panel.editing else {
        panic!("temp bound by default");
    };
    state.workspaces[0]
        .prompt_panel
        .temp_mut(temp_id)
        .expect("temp exists")
        .content = "body".into();
    // 选择器打开时建文件夹：服务刷新条目并选中新文件夹。
    crate::app::update::open_save_draft(&mut state);
    state.effects.push(Effect::CreateFolder {
        workspace: cwd.clone(),
        parent_id: None,
        name: "saved folder".into(),
    });
    assert!(service.pump(&mut state));
    let Some(Overlay::SaveDraft(dialog)) = &state.overlay else {
        panic!("picker restored");
    };
    assert!(matches!(
        &dialog.entries[dialog.selected],
        crate::app::overlay::SaveDraftEntry::Folder { name, .. } if name == "saved folder"
    ));
    let folder_id = state.workspaces[0].prompt_panel.tree.folders()[0].id;
    // 保存草稿到该文件夹。
    state.overlay = None;
    state.effects.push(Effect::SaveDraft {
        workspace: cwd.clone(),
        folder_id: Some(folder_id),
        name: "New prompt".into(),
        content: "body".into(),
    });
    assert!(service.pump(&mut state));
    let panel = &state.workspaces[0].prompt_panel;
    assert_eq!(panel.tree.folders().len(), 1);
    let item = &panel.tree.items()[0];
    assert_eq!(item.folder_id, Some(folder_id));
    assert_eq!(item.content, "body");
    assert!(panel.temp(temp_id).is_none());
}

// ===== v2：附件持久化与 managed 清理 =====

/// 独立测试目录（managed 截图与外部文件）。
fn temp_dir(label: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "lx-tui-service-files-{label}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).expect("temp dir");
    path
}

#[test]
fn save_draft_effect_moves_temp_into_tree() {
    let (mut service, mut state, cwd, _) = trusted_service("draft-move");
    let Some(PromptEditing::Temp(temp_id)) = state.workspaces[0].prompt_panel.editing else {
        panic!("temp bound by default");
    };
    {
        let temp = state.workspaces[0]
            .prompt_panel
            .temp_mut(temp_id)
            .expect("temp exists");
        temp.content = "draft body".into();
    }
    state.effects.push(Effect::SaveDraft {
        workspace: cwd.clone(),
        folder_id: None,
        name: "New prompt".into(),
        content: "draft body".into(),
    });
    assert!(service.pump(&mut state));
    let item = &state.workspaces[0].prompt_panel.tree.items()[0];
    assert_eq!(item.content, "draft body");
    let prompts = service
        .db
        .as_ref()
        .expect("db")
        .list_prompts(&cwd)
        .expect("list");
    assert_eq!(prompts[0].content, "draft body");
}

#[test]
fn save_prompt_content_confirms_revision_and_clears_pending() {
    let (mut service, mut state, cwd, _) = trusted_service("save-confirm");
    service
        .db
        .as_ref()
        .expect("db")
        .create_prompt(&cwd, None, "p", "old")
        .expect("seed");
    service.pump(&mut state);
    let id = state.workspaces[0].prompt_panel.tree.items()[0].id;
    assert!(crate::app::update::edit_prompt_item(&mut state, 0, id));
    state.prompt.insert_str(" edited");
    let revision = state.prompt.revision();
    let prompt = state.prompt.id();
    state.effects.push(Effect::SavePromptContent {
        workspace: cwd.clone(),
        id,
        content: "old edited".into(),
        prompt,
        revision,
    });
    assert!(service.pump(&mut state));
    assert!(state.prompt.is_saved(), "成功回执确认对应修订");
    assert!(!state.workspaces[0].prompt_panel.item_pending(id));
    let prompts = service
        .db
        .as_ref()
        .expect("db")
        .list_prompts(&cwd)
        .expect("list");
    assert_eq!(prompts[0].content, "old edited");
}

#[test]
fn save_prompt_content_failure_keeps_dirty_and_pending() {
    let (mut service, mut state, cwd, _) = trusted_service("save-failure");
    state.prompt.insert_str("body");
    assert!(!state.prompt.is_saved());
    let prompt = state.prompt.id();
    let revision = state.prompt.revision();
    // 指向不存在的条目：数据库写入失败。
    state.effects.push(Effect::SavePromptContent {
        workspace: cwd.clone(),
        id: 999,
        content: "body".into(),
        prompt,
        revision,
    });
    service.pump(&mut state);
    assert!(!state.prompt.is_saved(), "失败保留脏状态等待重试");
    assert!(state.toast.is_some(), "失败给出可见反馈");
}

#[test]
fn managed_cleanup_keeps_shared_references_and_leaves_unrelated_files() {
    let (mut service, state, cwd, _) = trusted_service("gc-shared");
    let dir = temp_dir("gc-shared");
    let asset_id = "0123456789abcdef0123456789abcdef";
    let asset_path = dir.join(format!("{asset_id}.png"));
    std::fs::write(&asset_path, b"png").expect("asset file");
    let unrelated_path = dir.join("unrelated.txt");
    std::fs::write(&unrelated_path, b"note").expect("unrelated file");
    service
        .db
        .as_ref()
        .expect("db")
        .register_managed_asset(asset_id, &asset_path)
        .expect("register");
    let reference = format!("see @[refer-image]({})", asset_path.display());
    let first = service
        .db
        .as_ref()
        .expect("db")
        .create_prompt(&cwd, None, "first", &reference)
        .expect("first");
    let second = service
        .db
        .as_ref()
        .expect("db")
        .create_prompt(&cwd, None, "second", &reference)
        .expect("second");
    service.cleanup_managed(&state);
    assert!(asset_path.exists(), "共享正文引用保留");
    assert!(unrelated_path.exists(), "目录内无关文件不删");

    service
        .db
        .as_ref()
        .expect("db")
        .delete_prompt(first)
        .expect("delete first");
    service.cleanup_managed(&state);
    assert!(asset_path.exists(), "仍有第二个 prompt 引用");

    service
        .db
        .as_ref()
        .expect("db")
        .delete_prompt(second)
        .expect("delete second");
    service.cleanup_managed(&state);
    assert!(!asset_path.exists(), "最后引用删除后清理文件");
    assert!(unrelated_path.exists(), "目录内无关文件始终保留");
    assert!(
        service
            .db
            .as_ref()
            .expect("db")
            .list_managed_assets()
            .expect("assets")
            .is_empty(),
        "登记行随文件一并回收"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn managed_cleanup_respects_in_memory_temp_references() {
    let (mut service, mut state, _cwd, _) = trusted_service("gc-temp");
    let dir = temp_dir("gc-temp");
    let asset_id = "fedcba9876543210fedcba9876543210";
    let asset_path = dir.join(format!("{asset_id}.png"));
    std::fs::write(&asset_path, b"png").expect("asset file");
    service
        .db
        .as_ref()
        .expect("db")
        .register_managed_asset(asset_id, &asset_path)
        .expect("register");
    // 只有内存临时草稿引用：正文引用与附件都算。
    {
        let Some(PromptEditing::Temp(temp_id)) = state.workspaces[0].prompt_panel.editing else {
            panic!("temp bound");
        };
        let temp = state.workspaces[0]
            .prompt_panel
            .temp_mut(temp_id)
            .expect("temp");
        temp.content = format!("@[refer-image]({})", asset_path.display());
    }
    service.cleanup_managed(&state);
    assert!(asset_path.exists(), "内存引用保护资产");

    {
        let Some(PromptEditing::Temp(temp_id)) = state.workspaces[0].prompt_panel.editing else {
            panic!("temp bound");
        };
        let temp = state.workspaces[0]
            .prompt_panel
            .temp_mut(temp_id)
            .expect("temp");
        temp.content = String::new();
    }
    service.cleanup_managed(&state);
    assert!(!asset_path.exists(), "无人引用时回收");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn orphan_scan_removes_temp_and_unregistered_files_but_keeps_registered() {
    let (service, _state, _cwd, _) = trusted_service("orphan-scan");
    let dir = temp_dir("orphan-scan");
    let token = "00112233445566778899aabbccddeeff";
    let registered_path = dir.join(format!("{token}.png"));
    std::fs::write(&registered_path, b"png").expect("registered");
    service
        .db
        .as_ref()
        .expect("db")
        .register_managed_asset(token, &registered_path)
        .expect("register");
    let unregistered = dir.join("ffffffffffffffffffffffffffffffff.png");
    std::fs::write(&unregistered, b"png").expect("unregistered");
    let temp = dir.join(".tmp-00112233445566778899aabbccddeeff.png");
    std::fs::write(&temp, b"png").expect("temp");
    let unknown = dir.join("notes.txt");
    std::fs::write(&unknown, b"note").expect("unknown");
    let nested = dir.join("subdir");
    std::fs::create_dir_all(&nested).expect("nested dir");

    service.cleanup_orphan_files_in(&dir);
    assert!(registered_path.exists(), "已登记文件保留");
    assert!(!unregistered.exists(), "未登记同规则文件清理");
    assert!(!temp.exists(), "临时落盘文件清理");
    assert!(unknown.exists(), "不认识的文件名跳过");
    assert!(nested.exists(), "目录不处理");
    let _ = std::fs::remove_dir_all(&dir);
}
