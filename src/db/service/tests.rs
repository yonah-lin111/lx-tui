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
        crate::app::prompt::PromptMode::Editor,
        "新建提示词进入编辑视图"
    );
    assert_eq!(
        state.workspaces[0].prompt_panel.editing,
        Some(PromptEditing::Item(item.id)),
        "新建提示词绑定编辑目标"
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
fn save_draft_effect_persists_content_binds_editor_and_clears_draft() {
    let (mut service, mut state, cwd, db_path) = trusted_service("save-draft");
    state.workspaces[0].prompt_panel.draft = "draft body".into();
    state.effects.push(Effect::SaveDraft {
        workspace: cwd.clone(),
        folder_id: None,
        name: "New prompt".into(),
        content: "draft body".into(),
    });
    assert!(service.pump(&mut state));
    let panel = &state.workspaces[0].prompt_panel;
    assert!(panel.draft.is_empty(), "保存成功后清空草稿槽位");
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
    state.workspaces[0].prompt_panel.draft = "body".into();
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
    assert!(panel.draft.is_empty());
}
