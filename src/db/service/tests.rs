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
fn is_trusted_reports_workspace_rows() {
    let db_path = temp_db_path("is-trusted");
    let cwd = workspace_path("is-trusted");
    let database = Database::open(&db_path).expect("db opens");
    let service = Persistence::with_database(database);
    assert!(!service.is_trusted(&cwd), "未登记路径不信任");
    assert!(service.trust_path(&cwd, "ws"), "登记成功");
    assert!(service.is_trusted(&cwd), "登记后信任");
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
        root: None,
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
        // 非 git 路径的仓库根即自身。
        root: Some(target.clone()),
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
fn check_trust_effect_requests_root_resolution_for_unknown_worktree() {
    let (mut service, mut state, _, _) = trusted_service("check-resolve");
    let target = workspace_path("check-resolve-worktree");
    state.effects.push(Effect::CheckTrust {
        path: target.clone(),
        root: None,
        trigger: crate::app::trust::TrustTrigger::CreateWorkspace {
            cwd: target.clone(),
        },
    });
    assert!(service.pump(&mut state));
    assert!(state.overlay.is_none(), "尚未解析出仓库根不弹浮层");
    assert!(matches!(
        state.trust_resolutions.as_slice(),
        [resolution] if resolution.path == target
    ));
    assert_eq!(state.workspaces.len(), 1, "未信任不创建");
}

#[test]
fn check_trust_effect_root_trusted_resumes_without_recording_worktree() {
    let (mut service, mut state, _, _) = trusted_service("check-root-trusted");
    let repo_root = workspace_path("check-root-trusted-repo");
    service
        .db
        .as_ref()
        .expect("db")
        .trust_workspace(&repo_root, "repo")
        .expect("trust root");
    let worktree = workspace_path("check-root-trusted-wt");
    state.effects.push(Effect::CheckTrust {
        path: worktree.clone(),
        root: Some(repo_root.clone()),
        trigger: crate::app::trust::TrustTrigger::CreateWorkspace {
            cwd: worktree.clone(),
        },
    });
    assert!(service.pump(&mut state));
    assert_eq!(state.workspaces.len(), 2, "仓库根已信任直接续作");
    assert!(state.overlay.is_none(), "不弹信任浮层");
    let db = service.db.as_ref().expect("db");
    assert!(
        !db.is_trusted(&worktree).expect("query"),
        "worktree 路径不入库"
    );
    assert!(db.is_trusted(&repo_root).expect("query"), "只信任仓库根");
}

#[test]
fn check_trust_effect_untrusted_root_opens_dialog_with_root_target() {
    let (mut service, mut state, _, _) = trusted_service("check-root-untrusted");
    let repo_root = workspace_path("check-root-untrusted-repo");
    let worktree = workspace_path("check-root-untrusted-wt");
    state.effects.push(Effect::CheckTrust {
        path: worktree.clone(),
        root: Some(repo_root.clone()),
        trigger: crate::app::trust::TrustTrigger::CreateWorkspace { cwd: worktree },
    });
    assert!(service.pump(&mut state));
    let Some(crate::app::overlay::Overlay::TrustWorkspace(trust)) = state.overlay.as_ref() else {
        panic!("trust dialog open");
    };
    assert_eq!(trust.path, repo_root, "浮层目标为仓库主 checkout");
    assert_eq!(state.workspaces.len(), 1, "未信任不创建");
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

/// 仓库主 checkout 工作区的 git 元数据。
fn main_git(repo_root: &std::path::Path) -> crate::app::state::WorkspaceGit {
    crate::app::state::WorkspaceGit {
        repo_root: repo_root.to_path_buf(),
        checkout_path: repo_root.to_path_buf(),
        is_linked: false,
        branch: Some("main".into()),
        main_branch: Some("main".into()),
    }
}

/// linked worktree 工作区的 git 元数据。
fn linked_git(
    repo_root: &std::path::Path,
    checkout: &std::path::Path,
) -> crate::app::state::WorkspaceGit {
    crate::app::state::WorkspaceGit {
        repo_root: repo_root.to_path_buf(),
        checkout_path: checkout.to_path_buf(),
        is_linked: true,
        branch: Some("feature/x".into()),
        main_branch: Some("main".into()),
    }
}

/// 追加一个 linked worktree 工作区条目并返回索引。
fn push_linked(
    state: &mut AppState,
    repo_root: &std::path::Path,
    worktree: &std::path::Path,
) -> usize {
    let mut workspace = crate::app::state::Workspace::single_terminal(
        "feat".to_string(),
        Some(worktree.to_path_buf()),
    );
    workspace.git = Some(linked_git(repo_root, worktree));
    state.workspaces.push(workspace);
    state.workspaces.len() - 1
}

#[test]
fn prompt_effects_keyed_by_repo_root_share_tree_across_worktree() {
    let (mut service, mut state, repo_root, _) = trusted_service("share-tree");
    let worktree = workspace_path("share-tree-wt");
    state.active_workspace_mut().git = Some(main_git(&repo_root));
    push_linked(&mut state, &repo_root, &worktree);
    // 在 worktree 条目中新建提示词：持久化键必须是仓库根。
    state.effects.push(Effect::CreatePrompt {
        workspace: worktree.clone(),
        folder_id: None,
        name: "shared".into(),
    });
    assert!(service.pump(&mut state));
    let db = service.db.as_ref().expect("db");
    assert_eq!(
        db.list_prompts(&repo_root).expect("list root").len(),
        1,
        "提示词挂在仓库根名下"
    );
    assert!(
        db.list_prompts(&worktree).expect("list wt").is_empty(),
        "worktree 路径无独立提示词"
    );
    assert!(
        !db.is_trusted(&worktree).expect("query"),
        "worktree 路径不入库"
    );
    // 两个条目的面板共享同一棵树（内容与 id 一致）。
    let main_items = state.workspaces[0].prompt_panel.tree.items();
    let linked_items = state.workspaces[1].prompt_panel.tree.items();
    assert_eq!(main_items.len(), 1);
    assert_eq!(linked_items.len(), 1);
    assert_eq!(main_items[0].id, linked_items[0].id);
    assert_eq!(main_items[0].name, "shared");
}

#[test]
fn save_prompt_content_syncs_all_sharing_panels() {
    let (mut service, mut state, repo_root, _) = trusted_service("share-save");
    let worktree = workspace_path("share-save-wt");
    state.active_workspace_mut().git = Some(main_git(&repo_root));
    push_linked(&mut state, &repo_root, &worktree);
    state.effects.push(Effect::CreatePrompt {
        workspace: worktree.clone(),
        folder_id: None,
        name: "shared".into(),
    });
    service.pump(&mut state);
    let id = state.workspaces[1].prompt_panel.tree.items()[0].id;
    state.effects.push(Effect::SavePromptContent {
        workspace: worktree.clone(),
        id,
        content: "typed".into(),
        prompt: crate::layout::PaneId::alloc(),
        revision: 1,
    });
    service.pump(&mut state);
    for workspace in &state.workspaces {
        assert_eq!(
            workspace
                .prompt_panel
                .tree
                .item(id)
                .expect("item in every sharing panel")
                .content,
            "typed"
        );
    }
}

#[test]
fn ensure_tree_loaded_reloads_when_git_metadata_switches_key() {
    let (mut service, mut state, repo_root, _) = trusted_service("share-rekey");
    let worktree = workspace_path("share-rekey-wt");
    // git 元数据未到：以 cwd 为键载入空树。
    state.active_workspace_mut().cwd = Some(worktree.clone());
    service.pump(&mut state);
    assert!(state.workspaces[0].prompt_panel.loaded);
    assert!(state.workspaces[0].prompt_panel.tree.items().is_empty());
    // 仓库根下已有提示词；元数据到达后键切到仓库根，面板自动重载。
    service
        .db
        .as_ref()
        .expect("db")
        .create_prompt(&repo_root, None, "existing", "")
        .expect("seed");
    state.active_workspace_mut().git = Some(linked_git(&repo_root, &worktree));
    service.pump(&mut state);
    assert_eq!(state.workspaces[0].prompt_panel.tree.items().len(), 1);
    assert_eq!(
        state.workspaces[0].prompt_panel.loaded_key.as_deref(),
        Some(repo_root.as_path())
    );
}
