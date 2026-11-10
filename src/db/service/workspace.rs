//! 工作区历史、信任门、提示词树惰性重载、引用文件夹与全局 Todo 的持久化辅助方法及效应执行。
//!
//! 从 `service.rs` 按职责拆出：保持单文件水位在规范上限内，这里只做 DB 读写与状态层回投。

use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::app::effects::Effect;
use crate::app::state::AppState;
use crate::app::toast::{Toast, ToastKind};
use crate::app::update;
use crate::db::{DbError, Result};
use crate::ui::text;

use super::{Database, Persistence};

impl Persistence {
    /// 执行工作区、信任、引用文件夹、提示词历史与 Todo 持久化效应；返回状态是否有变化。
    pub(super) fn execute_workspace_effect(
        &mut self,
        state: &mut AppState,
        effect: Effect,
    ) -> bool {
        match effect {
            Effect::CheckTrust {
                path,
                root,
                trigger,
            } => {
                let path = canonical_path(&path);
                // 路径自身已信任（含历史 worktree 路径）直接续作。
                if self.is_trusted(&path) {
                    update::resume_trust(state, trigger);
                    return true;
                }
                match root {
                    // 仓库根已知：已信任则续作，否则信任浮层以仓库根为目标。
                    Some(root) => {
                        let root = canonical_path(&root);
                        if self.is_trusted(&root) {
                            update::resume_trust(state, trigger);
                        } else {
                            update::open_trust_dialog(state, root, trigger);
                        }
                    }
                    // 仓库归属未知：交给事件层后台解析后回投重新检查。
                    None => update::request_trust_root(state, path, trigger),
                }
                true
            }
            Effect::GrantTrust { path, name } => {
                self.with_db(|db| db.trust_workspace(&path, &name));
                false
            }
            Effect::AddReferencedFolder {
                workspace,
                path,
                anchor,
            } => {
                let key = prompt_key_for(state, &workspace);
                let added = self.with_db(|db| db.add_referenced_folder(&key, &path));
                if added.is_none() {
                    // 写失败：不制造“已保存”假象，在输入弹窗上显示错误并保留输入重试（弹窗未关，show_toast_at 走全局）。
                    let source = Some(state.prompt.id());
                    update::show_toast_at(
                        state,
                        source,
                        Toast::new(
                            ToastKind::Danger,
                            text::TOAST_REFERENCED_SAVE_FAILED,
                            source,
                            Instant::now(),
                        ),
                    );
                    update::apply_referenced_add_failure(state);
                    return false;
                }
                let mut changed = self.reload_referenced(state, &key);
                changed |= update::finish_referenced_add(state, anchor);
                changed
            }
            Effect::DeleteReferencedFolder { workspace, id } => {
                let key = prompt_key_for(state, &workspace);
                let deleted = self.with_db(|db| db.delete_referenced_folder(&key, id));
                if deleted.is_none() {
                    // 删除确认弹窗点击后已立即关闭，Toast 回到打开弹窗的右侧面板容器显示。
                    let source = Some(state.prompt.id());
                    update::show_toast_at(
                        state,
                        source,
                        Toast::new(
                            ToastKind::Danger,
                            text::TOAST_REFERENCED_DELETE_FAILED,
                            source,
                            Instant::now(),
                        ),
                    );
                    return false;
                }
                let mut changed = update::invalidate_referenced_search(state, id);
                changed |= self.reload_referenced(state, &key);
                changed
            }
            // 持久化一条已发送 prompt 到全局历史；失败只记日志，不影响聊天。
            Effect::RecordPromptHistory { text } => {
                if let Some(db) = self.db.as_ref()
                    && let Err(error) =
                        db.record_prompt_history(&text, crate::app::state::PROMPT_HISTORY_LIMIT)
                {
                    tracing::warn!(%error, "recording prompt history failed");
                }
                false
            }
            // todo 写路径：单事务完成，失败只 toast（内存列表已更新，是真源）。
            Effect::SaveTodo { record } => {
                self.todo_write(state, |db| db.save_todo(&record));
                false
            }
            Effect::DeleteTodo { id } => {
                self.todo_write(state, |db| db.delete_todo(&id));
                false
            }
            Effect::RestoreTodos { records } => {
                self.todo_write(state, |db| db.restore_todos(&records));
                false
            }
            Effect::ReorderTodos {
                entry_date,
                ordered_ids,
            } => {
                self.todo_write(state, |db| db.reorder_todos(&entry_date, &ordered_ids));
                false
            }
            Effect::RolloverTodos { ids, target_date } => {
                self.todo_write(state, |db| db.rollover_todos_to_date(&ids, &target_date));
                false
            }
            Effect::SyncDateTodos {
                entry_date,
                records,
            } => {
                self.todo_write(state, |db| db.sync_date_todos(&entry_date, &records));
                false
            }
            Effect::RecordWorkspace { path, name } => {
                self.with_db(|db| db.record_workspace_open(&path, &name));
                self.with_db(|db| db.unhide_workspace(&path));
                // 任何显式登记入口（历史列表、worktree 对话框、同步 cwd）都应解除临时隐藏，
                // 否则删除后重新打开同一 worktree 会被 apply_workspace_history 永久过滤。
                state
                    .workspace_history
                    .hidden_worktrees
                    .retain(|hidden| hidden != &path);
                self.reload_workspace_history(state)
            }
            Effect::RenameWorkspace { path, name } => {
                let written = self.with_db(|db| db.rename_workspace(&path, &name));
                if written.is_none() {
                    self.workspace_write_failed(state);
                }
                self.reload_workspace_history(state)
            }
            Effect::DeleteWorkspaceHistory { path } => {
                // 最后一个打开的工作区无法关闭：删除历史后保留一条已信任隐藏行，避免以未信任默认名复活。
                let open_name = state
                    .workspaces
                    .iter()
                    .find(|workspace| {
                        workspace.cwd.as_deref() == Some(path.as_path())
                            || workspace
                                .git
                                .as_ref()
                                .is_some_and(|git| git.checkout_path == path)
                    })
                    .map(|workspace| workspace.name.clone());
                // 分页接口有 200 条上限；循环取全量，确保第 201 条及更早的运行中会话也被中断。
                let mut offset = 0;
                while let Some(page) = self.with_db(|db| {
                    db.list_chat_sessions(&path, offset, crate::db::chat::SESSION_PAGE_SIZE)
                }) {
                    let page_len = page.len();
                    let panes = state.all_pane_ids();
                    for summary in page {
                        state.effects.push(Effect::CancelSubagents {
                            session_id: summary.external_id.clone(),
                            agent_id: None,
                        });
                        for &pane in &panes {
                            update::on_session_deleted(state, pane, &summary.external_id);
                            update::on_session_removed(state, pane, &summary.external_id);
                        }
                    }
                    if page_len < crate::db::chat::SESSION_PAGE_SIZE {
                        break;
                    }
                    offset += page_len;
                }
                // linked worktree（`.git` 为文件或已记入 hidden_worktrees）持久化隐藏，避免被主仓库重新扫描复活。
                if path.join(".git").is_file()
                    || state.workspace_history.hidden_worktrees.contains(&path)
                {
                    self.with_db(|db| db.hide_workspace(&path));
                }
                let deleted = self.with_db(|db| db.delete_workspace(&path));
                if deleted.is_none() {
                    self.workspace_write_failed(state);
                }
                if let Some(name) = open_name {
                    self.with_db(|db| db.trust_workspace(&path, &name));
                    self.with_db(|db| db.hide_workspace(&path));
                }
                self.reload_workspace_history(state)
            }
            Effect::SaveWorkspaceOrder { entries } => {
                let changed = self
                    .with_db(|db| db.save_workspace_order(&entries))
                    .is_some();
                let reloaded = self.reload_workspace_history(state);
                changed || reloaded
            }
            Effect::LoadWorkspaceHistory => self.reload_workspace_history(state),
            other => {
                tracing::warn!(?other, "non-workspace effect reached workspace handler");
                false
            }
        }
    }

    /// 执行一次 todo 写操作：失败时记录日志并提示用户（内存列表不回滚）。
    /// 与 [`Self::with_db`] 的区别只有失败反馈：todo 写入是用户可见动作，不能静默失败。
    pub(super) fn todo_write(
        &self,
        state: &mut AppState,
        operation: impl FnOnce(&Database) -> Result<()>,
    ) {
        let Some(db) = self.db.as_ref() else {
            return;
        };
        if let Err(error) = operation(db) {
            tracing::warn!(%error, "todo write failed");
            update::fail_todo_write(state);
        }
    }

    /// 工作区写失败提示；无数据库（只读降级）时不打扰用户。
    pub(super) fn workspace_write_failed(&self, state: &mut AppState) {
        if self.db.is_none() {
            return;
        }
        let toast = Toast::new(
            ToastKind::Danger,
            text::TOAST_WORKSPACE_WRITE_FAILED,
            None,
            Instant::now(),
        )
        .with_title(text::TOAST_WORKSPACE_TITLE);
        update::show_toast(state, toast);
    }

    /// 重载指定 prompt 键的全部工作区面板：同仓库的主 checkout 与 linked worktree
    /// 共享同一棵树，变更后一起刷新；返回状态是否有变化。
    pub(super) fn reload_tree(&self, state: &mut AppState, key: &Path) -> bool {
        let targets: Vec<usize> = state
            .workspaces
            .iter()
            .enumerate()
            .filter(|(_, workspace)| workspace.repo_key() == Some(key))
            .map(|(index, _)| index)
            .collect();
        if targets.is_empty() {
            return false;
        }
        let Some(db) = &self.db else {
            for index in targets {
                mark_panel_loaded(state, index, Some(key));
            }
            return true;
        };
        match (db.list_folders(key), db.list_prompts(key)) {
            (Ok(folders), Ok(prompts)) => {
                let mut changed = false;
                for index in targets {
                    changed |= update::apply_prompt_tree(
                        state,
                        index,
                        key,
                        folders.clone(),
                        prompts.clone(),
                    );
                }
                changed
            }
            (Err(error), _) | (_, Err(error)) => {
                tracing::warn!(%error, "prompt tree load failed");
                for index in targets {
                    mark_panel_loaded(state, index, Some(key));
                }
                false
            }
        }
    }

    /// 惰性载入显示中工作区的提示词树；git 元数据晚到使 prompt 键变化时重载。
    /// 返回状态是否有变化。
    pub(super) fn ensure_tree_loaded(&self, state: &mut AppState) -> bool {
        let index = state.prompt_workspace();
        let Some(workspace) = state.workspaces.get(index) else {
            return false;
        };
        let key = workspace.repo_key().map(Path::to_path_buf);
        if workspace.prompt_panel.loaded && workspace.prompt_panel.loaded_key == key {
            return false;
        }
        match key {
            Some(key) => self.reload_tree(state, &key),
            None => {
                mark_panel_loaded(state, index, None);
                true
            }
        }
    }

    /// 重载引用文件夹快照：按显示中 prompt 键读取；键变化或尚未载入时整体刷新。
    ///
    /// 数据库不可用时标记 `failed`（菜单展示不可用状态），不伪装成空表已持久化。
    pub(super) fn ensure_referenced_loaded(&self, state: &mut AppState) -> bool {
        let key = state.referenced_key();
        if state.referenced.loaded && state.referenced.key == key {
            return false;
        }
        // 归属键变化：作废旧菜单/弹窗与在途扫描结果，避免归给新项目。
        state.referenced_requests.clear();
        let mut changed = update::invalidate_referenced_overlays(state);
        match key {
            Some(key) => changed |= self.reload_referenced(state, &key),
            None => {
                // 无归属键：记录空快照并标记已载入，避免每个 pump 周期重复触发重绘。
                let had_data =
                    state.referenced.key.is_some() || !state.referenced.folders.is_empty();
                state.referenced.reset(None);
                state.referenced.loaded = true;
                changed |= had_data;
            }
        }
        changed
    }

    /// 按项目键重载引用文件夹快照；写失败标记不可用状态。
    pub(super) fn reload_referenced(&self, state: &mut AppState, key: &Path) -> bool {
        let Some(db) = &self.db else {
            state.referenced.apply(key.to_path_buf(), Vec::new(), true);
            return true;
        };
        match db.list_referenced_folders(key) {
            Ok(records) => {
                let folders = records
                    .into_iter()
                    .map(update::referenced_folder_from_record)
                    .collect();
                state.referenced.apply(key.to_path_buf(), folders, false);
                true
            }
            Err(error) => {
                tracing::warn!(%error, "referenced folder load failed");
                state.referenced.apply(key.to_path_buf(), Vec::new(), true);
                true
            }
        }
    }

    /// 从数据库重新加载工作区历史列表并同步到 `state.workspace_history`。
    pub(super) fn reload_workspace_history(&self, state: &mut AppState) -> bool {
        let Some(db) = &self.db else {
            return false;
        };
        match db.list_workspace_history_entries() {
            Ok(entries) => update::apply_workspace_history(state, entries),
            Err(error) => {
                tracing::warn!(%error, "workspace history load failed");
                false
            }
        }
    }
}

/// 是否为工作区、信任、引用文件夹、提示词历史或 Todo 持久化效应。
pub(super) fn handles(effect: &Effect) -> bool {
    matches!(
        effect,
        Effect::CheckTrust { .. }
            | Effect::GrantTrust { .. }
            | Effect::AddReferencedFolder { .. }
            | Effect::DeleteReferencedFolder { .. }
            | Effect::RecordPromptHistory { .. }
            | Effect::SaveTodo { .. }
            | Effect::DeleteTodo { .. }
            | Effect::RestoreTodos { .. }
            | Effect::ReorderTodos { .. }
            | Effect::RolloverTodos { .. }
            | Effect::SyncDateTodos { .. }
            | Effect::RecordWorkspace { .. }
            | Effect::RenameWorkspace { .. }
            | Effect::DeleteWorkspaceHistory { .. }
            | Effect::SaveWorkspaceOrder { .. }
            | Effect::LoadWorkspaceHistory
    )
}

/// 标记面板已载入（无持久化或查询失败时的空树；键为 None 表示工作区无路径）。
fn mark_panel_loaded(state: &mut AppState, index: usize, key: Option<&Path>) {
    if let Some(workspace) = state.workspaces.get_mut(index) {
        workspace.prompt_panel.loaded = true;
        workspace.prompt_panel.loaded_key = key.map(Path::to_path_buf);
    }
}

/// 按 cwd 定位工作区索引（效应携带的路径即来源工作区 cwd 克隆）。
pub(super) fn workspace_index(state: &AppState, path: &Path) -> Option<usize> {
    state
        .workspaces
        .iter()
        .position(|workspace| workspace.cwd.as_deref() == Some(path))
}

/// 效应目标工作区的仓库键：git 工作区为仓库主 checkout，其余为 cwd；
/// 来源工作区已关闭时回退效应携带的路径。
pub(super) fn prompt_key_for(state: &AppState, workspace: &Path) -> PathBuf {
    workspace_index(state, workspace)
        .and_then(|index| state.workspaces[index].repo_key())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| workspace.to_path_buf())
}

/// 移动被 DB 拒绝时的用户文案：状态层同样分流这三条，两层口径保持一致。
pub(super) fn move_reject_message(error: &DbError) -> &'static str {
    match error {
        DbError::DepthExceeded => text::TOAST_PROMPT_MOVE_DEPTH,
        DbError::FolderHasChildren => text::TOAST_PROMPT_MOVE_NESTED,
        _ => text::TOAST_PROMPT_MOVE_FAILED,
    }
}

/// 规范化路径；失败时保留原路径（路径可能刚被删除）。
fn canonical_path(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}
