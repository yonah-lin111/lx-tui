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
            Ok(db) => {
                // 异常退出（进程被杀/崩溃）会遗留 `streaming` assistant 行与 pending/running
                // 工具行；启动时统一收口，否则恢复会话时这些行永远停在进行中。
                match db.recover_interrupted_streams() {
                    Ok(0) => {}
                    Ok(recovered) => {
                        tracing::info!(recovered, "recovered interrupted chat streams");
                    }
                    Err(error) => {
                        tracing::warn!(%error, "recovering interrupted chat streams failed");
                    }
                }
                // 不自动重跑任何工具：只把遗留调用标为 interrupted，结果字段不伪造成功。
                match db.recover_interrupted_tool_calls() {
                    Ok(0) => {}
                    Ok(recovered) => {
                        tracing::info!(recovered, "recovered interrupted tool calls");
                    }
                    Err(error) => {
                        tracing::warn!(%error, "recovering interrupted tool calls failed");
                    }
                }
                Self { db: Some(db) }
            }
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

    /// 数据库是否可用（无持久化模式下为 false）。
    pub fn has_database(&self) -> bool {
        self.db.is_some()
    }

    /// 记录一次工具调用的 pending 行并返回执行票据（事务提交后才返回）。
    ///
    /// 「结果落库」「pending 落库」这类**必须区分成功/失败**的写操作不经
    /// [`Self::with_db`]：失败原因要回给 agent loop（决定是否执行文件 IO），
    /// 不能被折叠成 `None`。
    pub fn begin_tool_call(
        &self,
        workspace: &Path,
        session_id: &str,
        message_id: &str,
        call_id: &str,
        name: &str,
        arguments_json: &str,
    ) -> super::Result<crate::db::chat::ToolCallTicket> {
        let db = self.db.as_ref().ok_or(super::DbError::Unavailable)?;
        db.begin_tool_call(
            workspace,
            session_id,
            message_id,
            call_id,
            name,
            arguments_json,
        )
    }

    /// pending -> running；失败时调用方不得执行文件 IO。
    pub fn mark_tool_running(
        &self,
        workspace: &Path,
        ticket: &crate::db::chat::ToolCallTicket,
    ) -> super::Result<()> {
        let db = self.db.as_ref().ok_or(super::DbError::Unavailable)?;
        db.mark_tool_running(workspace, ticket)
    }

    /// 结算工具调用（running -> 终态；pending -> cancelled 允许）。
    pub fn finish_tool_call(
        &self,
        workspace: &Path,
        ticket: &crate::db::chat::ToolCallTicket,
        result: &crate::db::chat::ToolCallResult,
    ) -> super::Result<()> {
        let db = self.db.as_ref().ok_or(super::DbError::Unavailable)?;
        db.finish_tool_call(workspace, ticket, result)
    }

    /// 工具 pending 前的 parts 快照同步：让 DB 的时间线位置与 app 步骤时间线一致。
    ///
    /// `begin_tool_call` 按该消息内 parts/tools 的 `MAX(position) + 1` 分配工具位置，
    /// 若 app 已产生的 parts 尚未落库，分配的编号会与 app 时间线错位（后续 checkpoint /
    /// finish 会撞 `PositionConflict`）。因此 pending 之前先提交当前 parts 快照，
    /// 保证同一消息内两表位置严格对应。失败时调用方必须中止该工具（不执行文件 IO）。
    pub fn checkpoint_parts(
        &self,
        workspace: &Path,
        assistant_id: &str,
        parts: &[crate::db::chat::ChatPart],
    ) -> super::Result<()> {
        let db = self.db.as_ref().ok_or(super::DbError::Unavailable)?;
        db.checkpoint(workspace, assistant_id, parts)
    }

    /// 记录工作区信任（写库）；库不可用或写失败返回 false（仅日志，不阻断进入）。
    pub fn trust_path(&self, path: &Path, name: &str) -> bool {
        self.with_db(|db| db.trust_workspace(path, name)).is_some()
    }

    /// 载入全局 prompt 历史并写入状态（启动时调用一次）。
    ///
    /// 库不可用或读取失败时保持空历史：历史是便利功能，不应阻断启动。
    pub fn load_prompt_history(&self, state: &mut AppState) {
        let Some(db) = self.db.as_ref() else {
            return;
        };
        match db.load_prompt_history(crate::app::state::PROMPT_HISTORY_LIMIT) {
            Ok(entries) => state.prompt_history.replace_all(entries),
            Err(error) => tracing::warn!(%error, "loading prompt history failed"),
        }
    }

    /// 执行全部待办效应并惰性载入显示中工作区的提示词树；返回状态是否有变化。
    ///
    /// 聊天的网络效应（StartChatStream/CancelChatStream）不在此执行——
    /// 事件层 `pump_chat_streams` 取走后到 Tokio 任务运行；本函数把它们
    /// 原样放回队列尾部。
    pub fn pump(&mut self, state: &mut AppState) -> bool {
        let mut changed = false;
        for _ in 0..MAX_EFFECT_ROUNDS {
            if state.effects.is_empty() {
                break;
            }
            let effects = std::mem::take(&mut state.effects);
            let mut deferred: Vec<Effect> = Vec::new();
            for effect in effects {
                if is_network_effect(&effect) {
                    deferred.push(effect);
                    continue;
                }
                changed |= self.execute(state, effect);
            }
            if deferred.is_empty() {
                if state.effects.is_empty() {
                    break;
                }
                continue;
            }
            // 网络效应放回队首：事件层 pump_chat_streams 尽快取走。
            let rest = std::mem::take(&mut state.effects);
            deferred.extend(rest);
            state.effects = deferred;
            break;
        }
        changed |= self.ensure_tree_loaded(state);
        changed |= self.ensure_referenced_loaded(state);
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
                let Some(id) = self.with_db(|db| db.create_prompt(&key, folder_id, &name, &[]))
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
                pages,
                active,
            } => {
                let key = prompt_key_for(state, &workspace);
                let page_values: Vec<(String, String)> = pages
                    .iter()
                    .map(|page| (page.name.clone(), page.content.clone()))
                    .collect();
                let Some(id) =
                    self.with_db(|db| db.create_prompt(&key, folder_id, &name, &page_values))
                else {
                    // 失败：TempPrompt 仍在，保存选择器可重试。
                    return false;
                };
                let changed = self.reload_tree(state, &key);
                let Some(index) = workspace_index(state, &workspace) else {
                    return changed;
                };
                update::finish_draft_save(state, index);
                let bound = update::open_saved_draft(state, index, id, active);
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
                page_id,
                content,
                prompt,
                revision,
                ..
            } => {
                let saved = self.with_db(|db| db.update_page_content(page_id, &content));
                if saved.is_some() {
                    update::mark_prompt_page_persisted(
                        state, id, page_id, &content, prompt, revision,
                    );
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
            Effect::CreatePage {
                workspace,
                id,
                name,
            } => {
                let key = prompt_key_for(state, &workspace);
                let Some(page_id) = self.with_db(|db| db.create_prompt_page(id, &name)) else {
                    return false;
                };
                let changed = self.reload_tree(state, &key);
                let activated = workspace_index(state, &workspace).is_some_and(|index| {
                    update::open_created_page(state, index, id, page_id, &name)
                });
                changed || activated
            }
            Effect::RenamePage {
                workspace,
                page_id,
                name,
            } => {
                self.with_db(|db| db.rename_prompt_page(page_id, &name));
                self.reload_tree(state, &prompt_key_for(state, &workspace))
            }
            Effect::DeletePage {
                workspace,
                id,
                page_id,
            } => {
                let deleted = self.with_db(|db| db.delete_prompt_page(page_id));
                if deleted.is_none() {
                    return false;
                }
                let changed = self.reload_tree(state, &prompt_key_for(state, &workspace));
                let rebound = workspace_index(state, &workspace)
                    .is_some_and(|index| update::rebind_after_page_delete(state, index, id));
                changed || rebound
            }
            Effect::SetActivePage {
                workspace,
                id,
                page_id,
            } => {
                self.with_db(|db| db.set_prompt_active_page(id, page_id));
                self.reload_tree(state, &prompt_key_for(state, &workspace))
            }
            Effect::AddReferencedFolder {
                workspace,
                path,
                anchor,
            } => {
                let key = prompt_key_for(state, &workspace);
                let added = self.with_db(|db| db.add_referenced_folder(&key, &path));
                if added.is_none() {
                    // 写失败：不制造“已保存”假象，在输入弹窗上显示错误并保留输入重试。
                    update::show_toast(
                        state,
                        Toast::new(
                            ToastKind::Error,
                            text::TOAST_REFERENCED_SAVE_FAILED,
                            None,
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
                    update::show_toast(
                        state,
                        Toast::new(
                            ToastKind::Error,
                            text::TOAST_REFERENCED_DELETE_FAILED,
                            None,
                            Instant::now(),
                        ),
                    );
                    return false;
                }
                let mut changed = update::invalidate_referenced_search(state, id);
                changed |= self.reload_referenced(state, &key);
                changed
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
            // ===== lx 聊天（DB 效应；网络效应由事件层执行） =====
            Effect::BeginChatTurn {
                pane,
                generation,
                workspace,
                session_id,
                draft,
                model,
            } => {
                let committed = self
                    .with_db(|db| db.begin_turn(&workspace, session_id.as_deref(), &draft, &model));
                match committed {
                    Some(committed) => {
                        update::on_turn_committed(state, pane, generation, &committed)
                    }
                    None => update::on_turn_failed(state, pane, generation),
                }
            }
            Effect::CancelChatCommit { pane, generation } => {
                // Committing 阶段取消：DB ack 已在队列中由 BeginChatTurn 处理，
                // 此处仅记录（ack 到达后直接落 interrupted、不发 HTTP）。
                let _ = (pane, generation);
                false
            }
            Effect::FinishChatTurn {
                key,
                assistant_id,
                parts,
                outcome,
                usage,
            } => {
                // workspace 按窗格当前归属解析（关闭窗格后仍按原 workspace 完成写入）。
                let workspace = state
                    .workspace_cwd_for_pane(key.pane)
                    .unwrap_or_else(|| PathBuf::from(""));
                let success = self
                    .with_db(|db| db.finish(&workspace, &assistant_id, &parts, &outcome, usage));
                update::on_finish_ack(state, &key, success.is_some())
            }
            Effect::CheckpointChatTurn {
                workspace,
                assistant_id,
                parts,
            } => {
                // 幂等覆盖；失败静默（终态 finish 仍会写最终内容）。
                self.with_db(|db| db.checkpoint(&workspace, &assistant_id, &parts));
                false
            }
            Effect::ListChatSessions { pane } => {
                let workspace = state
                    .workspace_cwd_for_pane(pane)
                    .map(|path| path.to_path_buf());
                let Some(workspace) = workspace else {
                    return false;
                };
                let entries = self.with_db(|db| {
                    db.list_chat_sessions(&workspace, 0, crate::db::chat::SESSION_PAGE_SIZE)
                });
                let Some(entries) = entries else {
                    return false;
                };
                let views: Vec<crate::app::state::HistoryEntry> = entries
                    .into_iter()
                    .map(|summary| crate::app::state::HistoryEntry {
                        session_id: summary.external_id,
                        title: summary.title,
                        status: summary.status,
                        updated_at_ms: summary.updated_at_ms,
                    })
                    .collect();
                update::on_history_listed(state, pane, views)
            }
            Effect::LoadChatSession { pane, session_id } => {
                let workspace = state
                    .workspace_cwd_for_pane(pane)
                    .map(|path| path.to_path_buf());
                let Some(workspace) = workspace else {
                    return false;
                };
                let loaded: Option<(
                    Option<Vec<crate::db::chat::ChatMessage>>,
                    Option<crate::db::chat::ModelSnapshot>,
                )> = self.with_db(|db| {
                    let messages = db.load_chat_session(&workspace, &session_id)?;
                    let model = db.chat_session_model(&workspace, &session_id)?;
                    Ok((messages, model))
                });
                let Some((Some(messages), model)) = loaded else {
                    update::show_toast(
                        state,
                        Toast::new(
                            ToastKind::Error,
                            text::LX_CHAT_SESSION_BUSY,
                            None,
                            Instant::now(),
                        ),
                    );
                    return true;
                };
                // DB 的模型快照三列 → 窗格选择（恢复后 `/resume` 保留最后保存的选择）。
                let selection = model.map(|snapshot| crate::config::ai::ModelSelection {
                    provider: snapshot.provider,
                    model: snapshot.model,
                    variant: snapshot.variant,
                });
                update::on_session_loaded(state, pane, &session_id, &messages, selection)
            }
            Effect::RenameChatSession {
                workspace,
                session_id,
                title,
            } => {
                // 重命名成功后刷新该窗格的历史面板（若打开）。
                let renamed =
                    self.with_db(|db| db.rename_chat_session(&workspace, &session_id, &title));
                if renamed.is_some() {
                    // 就地更新 `/resume` 列表该行标题；面板保持打开、高亮不动。
                    let panes: Vec<crate::layout::PaneId> = state.all_pane_ids();
                    let mut changed = false;
                    for pane in panes {
                        changed |= update::on_session_renamed(state, pane, &session_id, &title);
                    }
                    return changed;
                }
                false
            }
            Effect::DeleteChatSession {
                workspace,
                session_id,
            } => {
                let deleted = self.with_db(|db| db.delete_chat_session(&workspace, &session_id));
                // 删除成功后解绑引用该会话的窗格（所有工作区内的窗格都可能持有），
                // 并从已打开的 `/resume` 列表里就地移除该行。
                if deleted.is_some() {
                    let mut changed = false;
                    let panes: Vec<crate::layout::PaneId> = state.all_pane_ids();
                    for pane in panes {
                        changed |= update::on_session_deleted(state, pane, &session_id);
                        changed |= update::on_session_removed(state, pane, &session_id);
                    }
                    return changed;
                }
                false
            }
            Effect::SetChatSessionModel {
                workspace,
                session_id,
                model,
            } => {
                self.with_db(|db| db.set_chat_session_model(&workspace, &session_id, &model));
                false
            }
            // 把当前选择的模型写回 `ai.json` 的 `defaultModel`（下次启动沿用）。
            Effect::SaveDefaultModel { selection } => {
                let path = match crate::config::ai::ai_config_path() {
                    Ok(path) => path,
                    Err(error) => {
                        tracing::warn!(%error, "ai config path unavailable");
                        return false;
                    }
                };
                match crate::config::ai::load_ai(&path) {
                    Ok(current) => {
                        let mut settings =
                            current.unwrap_or_else(crate::config::ai::AiSettings::empty);
                        settings.default_model = Some(selection);
                        if let Err(error) = crate::config::ai::save_ai(&path, &settings) {
                            tracing::warn!(%error, "saving default model failed");
                        }
                    }
                    Err(error) => tracing::warn!(%error, "ai config load failed"),
                }
                false
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
            // 网络效应由事件层执行（pump 已过滤；此处兜底忽略并记录）。
            Effect::StartChatStream { .. } | Effect::CancelChatStream { .. } => {
                tracing::warn!("chat network effect reached persistence pump");
                false
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

    /// 重载引用文件夹快照：按显示中 prompt 键读取；键变化或尚未载入时整体刷新。
    ///
    /// 数据库不可用时标记 `failed`（菜单展示不可用状态），不伪装成空表已持久化。
    fn ensure_referenced_loaded(&self, state: &mut AppState) -> bool {
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
    fn reload_referenced(&self, state: &mut AppState, key: &Path) -> bool {
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
}

/// 是否为聊天网络效应（由事件层 Tokio 任务执行，非同步 DB 写）。
fn is_network_effect(effect: &Effect) -> bool {
    matches!(
        effect,
        Effect::StartChatStream { .. } | Effect::CancelChatStream { .. }
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
mod resume_tests;
#[cfg(test)]
mod tests;
