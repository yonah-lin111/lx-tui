//! 同步持久化服务：打开数据库、执行 `app` 层效应出箱、按需载入提示词树。
//!
//! 事件循环持有唯一实例；app 层保持纯内存，所有 DB 读写经此收口。
//! 启动信任门由入口在进入应用前调用：`is_trusted` 配合事件层解析仓库根给出待信任路径，
//! 信任页确认后以 `trust_path` 登记；应用内部触发的信任检查仍走模态浮层。

use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::app::effects::Effect;
use crate::app::overlay::Overlay;
use crate::app::state::AppState;
use crate::app::toast::{Toast, ToastKind};
use crate::app::update;
use crate::ui::text;

use super::Database;

/// 单次泵送内效应执行的轮数上限（效应可级联产生新效应，如信任授予后的续作）。
const MAX_EFFECT_ROUNDS: usize = 16;

/// 持久化服务实例：数据库句柄可为 None（库不可用时降级为无持久化）。
pub struct Persistence {
    db: Option<Database>,
}

impl Persistence {
    /// 打开默认数据库；库不可用时降级为无持久化模式。
    ///
    /// 不做启动信任门，由入口在进入 TUI 前处理（见 `untrusted_startup_path`）。
    pub fn start() -> Self {
        match Database::open_default() {
            Ok(db) => Self { db: Some(db) },
            Err(error) => {
                tracing::warn!(%error, "database unavailable; persistence disabled");
                Self { db: None }
            }
        }
    }

    /// 使用指定数据库（测试）。
    pub fn with_database(db: Database) -> Self {
        Self { db: Some(db) }
    }

    /// 记录工作区信任（写库）；库不可用或写失败返回 false（仅日志，不阻断进入）。
    pub fn trust_path(&self, path: &Path, name: &str) -> bool {
        self.with_db(|db| db.trust_workspace(path, name)).is_some()
    }

    /// 执行全部待办效应并惰性载入显示中工作区的提示词树；返回状态是否有变化。
    pub fn pump(&mut self, state: &mut AppState) -> bool {
        let mut changed = false;
        for _ in 0..MAX_EFFECT_ROUNDS {
            if state.effects.is_empty() {
                break;
            }
            let effects = std::mem::take(&mut state.effects);
            for effect in effects {
                changed |= self.execute(state, effect);
            }
        }
        changed |= self.ensure_tree_loaded(state);
        changed
    }

    /// 退出前收尾：提交编辑中的提示词并落库。
    pub fn shutdown(&mut self, state: &mut AppState) {
        update::commit_editing_prompt(state);
        self.pump(state);
    }

    /// 信任查询；库不可用或查询失败按放行处理（不因持久化故障阻断应用）。
    pub fn is_trusted(&self, path: &Path) -> bool {
        let Some(db) = &self.db else {
            return true;
        };
        match db.is_trusted(path) {
            Ok(trusted) => trusted,
            Err(error) => {
                tracing::warn!(%error, "trust query failed; allowing access");
                true
            }
        }
    }

    /// 执行单条效应；返回状态是否有变化。
    fn execute(&mut self, state: &mut AppState, effect: Effect) -> bool {
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
            Effect::CreateFolder {
                workspace,
                parent_id,
                name,
            } => {
                let key = prompt_key_for(state, &workspace);
                let Some(id) = self.with_db(|db| db.create_folder(&key, parent_id, &name)) else {
                    return false;
                };
                let changed = self.reload_tree(state, &key);
                // 来自草稿保存选择器：刷新条目并选中新文件夹（选择器已同步恢复）。
                if matches!(state.overlay, Some(Overlay::SaveDraft(_))) {
                    update::refresh_save_draft(state, id);
                    return true;
                }
                let selection = workspace_index(state, &workspace)
                    .is_some_and(|index| update::select_prompt_folder(state, index, id));
                changed || selection
            }
            Effect::CreatePrompt {
                workspace,
                folder_id,
                name,
            } => {
                let key = prompt_key_for(state, &workspace);
                let Some(id) = self.with_db(|db| db.create_prompt(&key, folder_id, &name, ""))
                else {
                    return false;
                };
                let changed = self.reload_tree(state, &key);
                let selection = workspace_index(state, &workspace)
                    .is_some_and(|index| update::select_prompt_item(state, index, id));
                changed || selection
            }
            Effect::SaveDraft {
                workspace,
                folder_id,
                name,
                content,
            } => {
                let key = prompt_key_for(state, &workspace);
                let Some(id) =
                    self.with_db(|db| db.create_prompt(&key, folder_id, &name, &content))
                else {
                    // 失败：TempPrompt 仍在，保存选择器可重试。
                    return false;
                };
                let changed = self.reload_tree(state, &key);
                let Some(index) = workspace_index(state, &workspace) else {
                    return changed;
                };
                update::finish_draft_save(state, index);
                let bound = update::edit_prompt_item(state, index, id);
                changed || bound
            }
            Effect::RenameFolder {
                workspace,
                id,
                name,
            } => {
                self.with_db(|db| db.rename_folder(id, &name));
                self.reload_tree(state, &prompt_key_for(state, &workspace))
            }
            Effect::DeleteFolder { workspace, id } => {
                self.with_db(|db| db.delete_folder(id));
                self.reload_tree(state, &prompt_key_for(state, &workspace))
            }
            Effect::RenamePrompt {
                workspace,
                id,
                name,
            } => {
                self.with_db(|db| db.rename_prompt(id, &name));
                self.reload_tree(state, &prompt_key_for(state, &workspace))
            }
            Effect::DeletePrompt { workspace, id } => {
                self.with_db(|db| db.delete_prompt(id));
                self.reload_tree(state, &prompt_key_for(state, &workspace))
            }
            Effect::SavePromptContent {
                id,
                content,
                prompt,
                revision,
                ..
            } => {
                let saved = self.with_db(|db| db.update_prompt_document(id, &content));
                if saved.is_some() {
                    update::mark_prompt_item_persisted(state, id, &content);
                    update::mark_prompt_saved(state, prompt, revision);
                    true
                } else {
                    // 失败：保留脏状态与待保存内容，下一轮静置重试并给出可见反馈。
                    update::reset_prompt_autosave(state, prompt);
                    update::show_toast(
                        state,
                        Toast::new(
                            ToastKind::Error,
                            text::TOAST_PROMPT_SAVE_FAILED,
                            None,
                            Instant::now(),
                        ),
                    );
                    true
                }
            }
            Effect::SetPromptStatus { id, status, .. } => {
                self.with_db(|db| db.update_prompt_status(id, status.as_str()));
                false
            }
            Effect::CopyToClipboard(text) => {
                let now = Instant::now();
                let (kind, message) = if crate::platform::write_clipboard(&text) {
                    (ToastKind::Info, text::TOAST_COPIED)
                } else {
                    tracing::warn!("clipboard write failed");
                    (ToastKind::Error, text::TOAST_COPY_FAILED)
                };
                update::show_toast(
                    state,
                    Toast::new(kind, message, None, now).with_title(text::TOAST_CLIPBOARD_TITLE),
                );
                true
            }
        }
    }

    /// 执行一次数据库操作并记录错误；库不可用时跳过。
    fn with_db<T>(&self, operation: impl FnOnce(&Database) -> super::Result<T>) -> Option<T> {
        let db = self.db.as_ref()?;
        match operation(db) {
            Ok(value) => Some(value),
            Err(error) => {
                tracing::warn!(%error, "database operation failed");
                None
            }
        }
    }

    /// 重载指定 prompt 键的全部工作区面板：同仓库的主 checkout 与 linked worktree
    /// 共享同一棵树，变更后一起刷新；返回状态是否有变化。
    fn reload_tree(&self, state: &mut AppState, key: &Path) -> bool {
        let targets: Vec<usize> = state
            .workspaces
            .iter()
            .enumerate()
            .filter(|(_, workspace)| workspace.prompt_key() == Some(key))
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
    fn ensure_tree_loaded(&self, state: &mut AppState) -> bool {
        let index = state.prompt_workspace();
        let Some(workspace) = state.workspaces.get(index) else {
            return false;
        };
        let key = workspace.prompt_key().map(Path::to_path_buf);
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
}

/// 标记面板已载入（无持久化或查询失败时的空树；键为 None 表示工作区无路径）。
fn mark_panel_loaded(state: &mut AppState, index: usize, key: Option<&Path>) {
    if let Some(workspace) = state.workspaces.get_mut(index) {
        workspace.prompt_panel.loaded = true;
        workspace.prompt_panel.loaded_key = key.map(Path::to_path_buf);
    }
}

/// 按 cwd 定位工作区索引（效应携带的路径即来源工作区 cwd 克隆）。
fn workspace_index(state: &AppState, path: &Path) -> Option<usize> {
    state
        .workspaces
        .iter()
        .position(|workspace| workspace.cwd.as_deref() == Some(path))
}

/// 效应目标工作区的 prompt 键：git 工作区为仓库主 checkout，其余为 cwd；
/// 来源工作区已关闭时回退效应携带的路径。
fn prompt_key_for(state: &AppState, workspace: &Path) -> PathBuf {
    workspace_index(state, workspace)
        .and_then(|index| state.workspaces[index].prompt_key())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| workspace.to_path_buf())
}

/// 规范化路径；失败时保留原路径（路径可能刚被删除）。
fn canonical_path(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
mod tests;
