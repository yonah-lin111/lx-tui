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

mod chat;

/// 单次泵送内效应执行的轮数上限（效应可级联产生新效应，如信任授予后的续作）。
const MAX_EFFECT_ROUNDS: usize = 16;

/// 撤销成功后的视图重载结果：消息、独立提示、compact 检查点、模型切换与会话 revision。
pub struct RewindView {
    pub messages: Vec<crate::db::chat::ChatMessage>,
    pub notices: Vec<crate::db::chat::NoticeRecord>,
    pub checkpoints: Vec<crate::db::chat::CompactCheckpoint>,
    pub switches: Vec<crate::db::chat::ModelSwitchRecord>,
    pub revision: i64,
}

/// 持久化服务实例：数据库句柄可为 None（库不可用时降级为无持久化）。
pub struct Persistence {
    db: Option<Database>,
    /// 数据库文件路径：撤销在后台线程用独立连接执行，需要重新打开。
    path: Option<PathBuf>,
}

impl Persistence {
    /// 打开默认数据库；库不可用时降级为无持久化模式。
    ///
    /// `linked_worktree` 由入口解析：开发构建在 linked worktree 内使用独立库文件，
    /// 避免不同分支的 Schema 互相判废。
    /// 不做启动信任门，由入口在进入 TUI 前处理（见 `untrusted_startup_path`）。
    pub fn start(linked_worktree: Option<&Path>) -> Self {
        let path = default_database_path(linked_worktree);
        match Database::open_default(linked_worktree) {
            Ok(db) => {
                crate::db::image_migration::migrate_legacy_session_images(&db);
                if let Ok(referenced) = db.referenced_attachment_paths() {
                    crate::platform::prune_default_session_images(&referenced);
                }
                // 遗留流的恢复由启动装配层按 session guard 执行（见 event::recover_startup）。
                Self { db: Some(db), path }
            }
            Err(error) => {
                tracing::warn!(%error, "database unavailable; persistence disabled");
                Self {
                    db: None,
                    path: None,
                }
            }
        }
    }

    /// 使用指定数据库（测试）。
    pub fn with_database(db: Database) -> Self {
        Self {
            db: Some(db),
            path: None,
        }
    }

    /// 使用指定文件数据库（测试撤销后台执行）。
    pub fn with_database_at(db: Database, path: PathBuf) -> Self {
        Self {
            db: Some(db),
            path: Some(path),
        }
    }

    /// 启动恢复与只读扫描使用的数据库句柄。
    pub(crate) fn database(&self) -> Option<&Database> {
        self.db.as_ref()
    }

    /// 数据库文件路径；内存测试库为 None（撤销将退化为同步执行）。
    pub fn database_path(&self) -> Option<PathBuf> {
        self.path.clone()
    }

    /// 无持久化模式（测试 DB 失败路径）。
    #[cfg(test)]
    pub fn without_database() -> Self {
        Self {
            db: None,
            path: None,
        }
    }

    /// 数据库是否可用（无持久化模式下为 false）。
    pub fn has_database(&self) -> bool {
        self.db.is_some()
    }

    /// 记录工作区信任（写库）；库不可用或写失败返回 false（仅日志，不阻断进入）。
    pub fn trust_path(&self, path: &Path, name: &str) -> bool {
        self.with_db(|db| db.trust_workspace(path, name)).is_some()
    }

    /// 载入全局 todo 列表并写入状态（启动时调用一次）。
    ///
    /// 库不可用或读取失败时保持空列表：todo 是便利功能，不应阻断启动。
    pub fn load_todos(&self, state: &mut AppState) {
        let Some(db) = self.db.as_ref() else {
            return;
        };
        match db.list_todos() {
            Ok(items) => update::apply_todo_list(state, items),
            Err(error) => tracing::warn!(%error, "loading todo list failed"),
        }
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

    /// 登记启动初始工作区并载入全局工作区历史列表（启动信任通过后调用一次）。
    pub fn load_workspace_history(&self, state: &mut AppState) {
        let Some(db) = self.db.as_ref() else {
            return;
        };
        if let Some(ws) = state.workspaces.first()
            && let Some(cwd) = ws.cwd.as_deref()
            && let Err(error) = db.record_workspace_open(cwd, &ws.name)
        {
            tracing::warn!(%error, "recording initial workspace failed");
        }
        self.reload_workspace_history(state);
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
                if is_network_effect(state, &effect) {
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
        update::commit_todo_edit(state);
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
        if chat::handles(&effect) {
            return self.execute_chat_effect(state, effect);
        }
        match effect {
            // 子 agent 效应由事件层直接执行（需要 `ChatStreams` 里的句柄），
            // 与进程面板同一处置：持久化泵只记录并忽略。
            Effect::ListSubagents { .. }
            | Effect::LoadSubagentSession { .. }
            | Effect::CancelSubagents { .. }
            | Effect::DrainSubagentReports { .. } => {
                tracing::warn!(target: "lx::event", "subagent effect reached the persistence pump");
                false
            }
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
            Effect::MovePrompt {
                workspace,
                id,
                folder_id,
            } => {
                let key = prompt_key_for(state, &workspace);
                let result = self.db_result(|db| db.move_prompt(id, folder_id));
                // 失败也要重载：以 DB 真值把界面拉回原位，不留下“看起来移动了”的假象。
                let changed = self.reload_tree(state, &key);
                if let Err(ref error) = result {
                    update::show_toast(
                        state,
                        Toast::new(
                            ToastKind::Danger,
                            move_reject_message(error),
                            Some(state.prompt.id()),
                            Instant::now(),
                        ),
                    );
                }
                changed || result.is_err()
            }
            Effect::MoveFolder {
                workspace,
                id,
                parent_id,
            } => {
                let key = prompt_key_for(state, &workspace);
                let result = self.db_result(|db| db.move_folder(id, parent_id));
                let changed = self.reload_tree(state, &key);
                if let Err(ref error) = result {
                    update::show_toast(
                        state,
                        Toast::new(
                            ToastKind::Danger,
                            move_reject_message(error),
                            Some(state.prompt.id()),
                            Instant::now(),
                        ),
                    );
                }
                changed || result.is_err()
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
                    // 失败：保留脏状态与待保存内容，下一轮静置重试并在右侧面板给出可见反馈。
                    update::reset_prompt_autosave(state, prompt);
                    let anchor = Some(state.prompt.id());
                    update::show_toast(
                        state,
                        Toast::new(
                            ToastKind::Danger,
                            text::TOAST_PROMPT_SAVE_FAILED,
                            anchor,
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
                content,
            } => {
                let key = prompt_key_for(state, &workspace);
                let Some(page_id) = self.with_db(|db| db.create_prompt_page(id, &name, &content))
                else {
                    return false;
                };
                let changed = self.reload_tree(state, &key);
                let activated = workspace_index(state, &workspace).is_some_and(|index| {
                    update::open_created_page(state, index, id, page_id, &name)
                });
                changed || activated
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
            Effect::CopyToClipboard { text, anchor } => {
                let now = Instant::now();
                let (kind, message) = if crate::platform::write_clipboard(&text) {
                    (ToastKind::Default, text::toast_copied(&text))
                } else {
                    tracing::warn!("clipboard write failed");
                    (ToastKind::Danger, text::TOAST_COPY_FAILED.to_string())
                };
                // 按触发容器分派：lx 窗格落窗格专属槽，终端窗格与右侧面板落各自容器锚点，
                // 左侧栏与居中模态弹窗（anchor = None）落全局屏幕顶部居中。
                update::show_toast_at(
                    state,
                    anchor,
                    Toast::new(kind, message, anchor, now).with_title(text::TOAST_CLIPBOARD_TITLE),
                );
                true
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
            // todo 写路径：单事务完成，失败只 toast（内存列表已更新，是真源）。
            // 无持久化（`db: None`）时静默跳过——内存模式可用，与 prompt_history 同口径。
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
                // 最后一个打开的工作区无法关闭；删除其历史数据后需要保留一条已信任的
                // 隐藏行，供仍运行的终端/后续 prompt 保存使用，否则会以未信任默认名复活。
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
                // 仅对仍存在的 linked worktree（`.git` 为文件）持久化隐藏；
                // 普通目录/仓库根删行后不会通过 `git worktree list` 自动复活，无需隐藏。
                if path.join(".git").is_file() {
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
            // 网络与进程效应由事件层执行（pump 已过滤；此处兜底忽略并记录）。
            // 进程句柄只存在于事件层的运行实例里，持久化层既读不到也停不了。
            Effect::StartChatStream { .. }
            | Effect::StartAiWebStream { .. }
            | Effect::ResetAiWebSession { .. }
            | Effect::CancelChatStream { .. }
            | Effect::AnswerAsk { .. } => {
                tracing::warn!("chat network effect reached persistence pump");
                false
            }
            Effect::ListSessionProcesses { .. } | Effect::StopSessionProcess { .. } => {
                tracing::warn!("process effect reached persistence pump");
                false
            }
            // 撤销/压缩/标题生成效应由事件层取走；此处只兜底忽略。
            Effect::PrepareRewind { .. }
            | Effect::ExecuteRewind(_)
            | Effect::ExecuteCompact(_)
            | Effect::RevertCompact(_)
            | Effect::GenerateSessionTitle { .. }
            | Effect::GeneratePromptBlockTitle { .. } => {
                tracing::warn!("rewind/compact/title effect reached persistence pump");
                false
            }
            Effect::BeginUserShell { .. }
            | Effect::CancelUserShells { .. }
            | Effect::RefreshMcpCatalog { .. }
            | Effect::RestartMcpServer { .. } => {
                tracing::warn!("user shell/mcp effect reached persistence pump");
                false
            }
            // 聊天类效应已在入口由 `chat::handles` 分流到 `execute_chat_effect`。
            _ => {
                tracing::warn!("chat effect reached the non-chat persistence path");
                false
            }
        }
    }

    /// 执行一次数据库操作并透出错误类型；库不可用时返回 [`super::DbError::Unavailable`]。
    ///
    /// 与 [`Self::with_db`] 同样记录 `tracing::warn!`，区别只在调用方能按错误分流文案。
    fn db_result<T>(
        &self,
        operation: impl FnOnce(&Database) -> super::Result<T>,
    ) -> super::Result<T> {
        let db = self.db.as_ref().ok_or(super::DbError::Unavailable)?;
        operation(db).inspect_err(|error| tracing::warn!(%error, "database operation failed"))
    }

    /// 执行一次 todo 写操作：失败时记录日志并提示用户（内存列表不回滚）。
    ///
    /// 与 [`Self::with_db`] 的区别只有失败反馈：todo 的写入是用户可见动作，
    /// 静默失败会让「明明加了一条、重启就没了」变得不可解释。
    fn todo_write(
        &self,
        state: &mut AppState,
        operation: impl FnOnce(&Database) -> super::Result<()>,
    ) {
        let Some(db) = self.db.as_ref() else {
            return;
        };
        if let Err(error) = operation(db) {
            tracing::warn!(%error, "todo write failed");
            update::fail_todo_write(state);
        }
    }

    /// 执行一次数据库操作并记录错误；库不可用时跳过。
    pub(crate) fn with_db<T>(
        &self,
        operation: impl FnOnce(&Database) -> super::Result<T>,
    ) -> Option<T> {
        let db = self.db.as_ref()?;
        match operation(db) {
            Ok(value) => Some(value),
            Err(error) => {
                tracing::warn!(%error, "database operation failed");
                None
            }
        }
    }

    /// 工作区写失败提示；无数据库（只读降级）时不打扰用户。
    fn workspace_write_failed(&self, state: &mut AppState) {
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
    fn reload_tree(&self, state: &mut AppState, key: &Path) -> bool {
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
    fn ensure_tree_loaded(&self, state: &mut AppState) -> bool {
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

    /// 从数据库重新加载工作区历史列表并同步到 `state.workspace_history`。
    fn reload_workspace_history(&self, state: &mut AppState) -> bool {
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

/// 是否为聊天网络/撤销效应（由事件层执行，非同步 DB 写）。
fn is_network_effect(state: &AppState, effect: &Effect) -> bool {
    match effect {
        Effect::ListChatSessions { pane }
            if state
                .pane_anywhere(*pane)
                .is_some_and(|p| p.lx.entry == crate::app::state::LxEntry::AiWeb) =>
        {
            return true;
        }
        Effect::LoadChatSession { session_id, .. }
        | Effect::RenameChatSession { session_id, .. }
        | Effect::DeleteChatSession { session_id, .. }
            if crate::app::update::is_ai_web_session(session_id) =>
        {
            return true;
        }
        _ => {}
    }
    matches!(
        effect,
        Effect::StartChatStream { .. }
            | Effect::StartAiWebStream { .. }
            | Effect::ResetAiWebSession { .. }
            | Effect::CancelChatStream { .. }
            | Effect::AnswerAsk { .. }
            // 进程效应与网络效应同一处置：持久化泵不碰，交给事件层。
            | Effect::ListSessionProcesses { .. }
            | Effect::StopSessionProcess { .. }
            | Effect::ListSubagents { .. }
            | Effect::LoadSubagentSession { .. }
            | Effect::CancelSubagents { .. }
            | Effect::DrainSubagentReports { .. }
            | Effect::PrepareRewind { .. }
            | Effect::ExecuteRewind(_)
            | Effect::ExecuteCompact(_)
            | Effect::RevertCompact(_)
            | Effect::GenerateSessionTitle { .. }
            | Effect::GeneratePromptBlockTitle { .. }
            | Effect::BeginUserShell { .. }
            | Effect::CancelUserShells { .. }
            | Effect::RefreshMcpCatalog { .. }
            | Effect::RestartMcpServer { .. }
    )
}

/// 默认数据库路径（与 `Database::open_default` 同一环境判定）。
fn default_database_path(linked_worktree: Option<&Path>) -> Option<PathBuf> {
    if let Some(overridden) = std::env::var_os("LX_TUI_DB_PATH").filter(|path| !path.is_empty()) {
        return Some(PathBuf::from(overridden));
    }
    super::database_path(super::is_dev_environment(), linked_worktree).ok()
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

/// 效应目标工作区的仓库键：git 工作区为仓库主 checkout，其余为 cwd；
/// 来源工作区已关闭时回退效应携带的路径。
fn prompt_key_for(state: &AppState, workspace: &Path) -> PathBuf {
    workspace_index(state, workspace)
        .and_then(|index| state.workspaces[index].repo_key())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| workspace.to_path_buf())
}

/// 移动被 DB 拒绝时的用户文案：状态层同样分流这三条，两层口径保持一致。
fn move_reject_message(error: &super::DbError) -> &'static str {
    match error {
        super::DbError::DepthExceeded => text::TOAST_PROMPT_MOVE_DEPTH,
        super::DbError::FolderHasChildren => text::TOAST_PROMPT_MOVE_NESTED,
        _ => text::TOAST_PROMPT_MOVE_FAILED,
    }
}

/// 规范化路径；失败时保留原路径（路径可能刚被删除）。
fn canonical_path(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// 会话被占用/不可删除的反馈：优先落到绑定该会话的 lx 窗格自己的 toast 槽。
///
/// 会话级效应没有单一窗格可比锚定——同一会话可能被多个窗格共享，也可能一个都
/// 没绑定（此时只能退化为全局 toast，否则用户看不到任何反馈）。
fn session_busy_feedback(state: &mut AppState, session_id: &str) {
    let toast = Toast::new(
        ToastKind::Danger,
        text::LX_CHAT_SESSION_BUSY,
        None,
        Instant::now(),
    );
    if !update::show_lx_toast_for_session(state, session_id, toast.clone()) {
        update::show_toast(state, toast);
    }
}

#[cfg(test)]
mod fork_tests;
#[cfg(test)]
mod resume_tests;
#[cfg(test)]
mod tests;
