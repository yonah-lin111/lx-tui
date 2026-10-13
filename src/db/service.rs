//! 同步持久化服务：打开数据库、执行 `app` 层效应出箱、按需载入提示词树。
//!
//! 事件循环持有唯一实例；app 层保持纯内存，所有 DB 读写经此收口。
//! 启动信任门由入口在进入应用前调用：`untrusted_startup_path` 给出待信任路径，
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

    /// 启动信任门目标：未信任的启动工作区规范化路径；已信任、无 cwd 或库不可用时 None。
    pub fn untrusted_startup_path(&self, state: &AppState) -> Option<PathBuf> {
        let cwd = state.active_workspace().cwd.clone()?;
        let path = canonical_path(&cwd);
        (!self.is_trusted(&path)).then_some(path)
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
    fn is_trusted(&self, path: &Path) -> bool {
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
            Effect::CheckTrust { path, trigger } => {
                let path = canonical_path(&path);
                let trusted = self.is_trusted(&path);
                update::resolve_trust_check(state, &path, trusted, trigger);
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
                let Some(id) = self.with_db(|db| db.create_folder(&workspace, parent_id, &name))
                else {
                    return false;
                };
                let changed = self.reload_tree(state, &workspace);
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
                let Some(id) =
                    self.with_db(|db| db.create_prompt(&workspace, folder_id, &name, ""))
                else {
                    return false;
                };
                let changed = self.reload_tree(state, &workspace);
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
                let Some(id) =
                    self.with_db(|db| db.create_prompt(&workspace, folder_id, &name, &content))
                else {
                    // 失败：TempPrompt 仍在，保存选择器可重试。
                    return false;
                };
                let changed = self.reload_tree(state, &workspace);
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
                self.reload_tree(state, &workspace)
            }
            Effect::DeleteFolder { workspace, id } => {
                self.with_db(|db| db.delete_folder(id));
                self.reload_tree(state, &workspace)
            }
            Effect::RenamePrompt {
                workspace,
                id,
                name,
            } => {
                self.with_db(|db| db.rename_prompt(id, &name));
                self.reload_tree(state, &workspace)
            }
            Effect::DeletePrompt { workspace, id } => {
                self.with_db(|db| db.delete_prompt(id));
                self.reload_tree(state, &workspace)
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
                    update::mark_prompt_item_persisted(state, id);
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

    /// 重载指定工作区（按 cwd 匹配）的提示词树；返回状态是否有变化。
    fn reload_tree(&self, state: &mut AppState, workspace: &Path) -> bool {
        let Some(index) = workspace_index(state, workspace) else {
            return false;
        };
        let Some(cwd) = state
            .workspaces
            .get(index)
            .and_then(|workspace| workspace.cwd.clone())
        else {
            return false;
        };
        let Some(db) = &self.db else {
            if let Some(workspace) = state.workspaces.get_mut(index) {
                workspace.prompt_panel.loaded = true;
            }
            return true;
        };
        match (db.list_folders(&cwd), db.list_prompts(&cwd)) {
            (Ok(folders), Ok(prompts)) => update::apply_prompt_tree(state, index, folders, prompts),
            (Err(error), _) | (_, Err(error)) => {
                tracing::warn!(%error, "prompt tree load failed");
                if let Some(workspace) = state.workspaces.get_mut(index) {
                    workspace.prompt_panel.loaded = true;
                }
                false
            }
        }
    }

    /// 惰性载入显示中工作区的提示词树；返回状态是否有变化。
    fn ensure_tree_loaded(&self, state: &mut AppState) -> bool {
        let index = state.prompt_workspace();
        let Some(workspace) = state.workspaces.get(index) else {
            return false;
        };
        if workspace.prompt_panel.loaded {
            return false;
        }
        let cwd = workspace.cwd.clone();
        match cwd {
            Some(cwd) => self.reload_tree(state, &cwd),
            None => {
                if let Some(workspace) = state.workspaces.get_mut(index) {
                    workspace.prompt_panel.loaded = true;
                }
                true
            }
        }
    }
}

/// 按 cwd 定位工作区索引（效应携带的路径即工作区 cwd 克隆）。
fn workspace_index(state: &AppState, path: &Path) -> Option<usize> {
    state
        .workspaces
        .iter()
        .position(|workspace| workspace.cwd.as_deref() == Some(path))
}

/// 规范化路径；失败时保留原路径（路径可能刚被删除）。
fn canonical_path(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
mod tests;
