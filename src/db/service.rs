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

/// 一次会话加载的结果：消息、模型快照、该会话的进程记录。
///
/// 进程记录单独带出来是为了给**历史卡片校正状态**：结果 JSON 写的是结算那一刻
/// （通常是 `running`），进程之后可能早已退出（见 `reconcile_process_records`）。
type LoadedSession = (
    Option<Vec<crate::db::chat::ChatMessage>>,
    Option<crate::db::chat::ModelSnapshot>,
    Vec<crate::db::chat::ProcessRecord>,
    Vec<crate::db::chat::NoticeRecord>,
    Vec<crate::db::chat::CompactCheckpoint>,
    i64,
);

/// 撤销成功后的视图重载结果：消息、独立提示、compact 检查点与会话 revision。
pub struct RewindView {
    pub messages: Vec<crate::db::chat::ChatMessage>,
    pub notices: Vec<crate::db::chat::NoticeRecord>,
    pub checkpoints: Vec<crate::db::chat::CompactCheckpoint>,
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

    /// 最新用户轮次（撤销目标与 revision）。
    pub fn latest_user_turn(
        &self,
        workspace: &Path,
        session_id: &str,
    ) -> super::Result<Option<super::chat::LatestTurn>> {
        let db = self.db.as_ref().ok_or(super::DbError::Unavailable)?;
        db.latest_user_turn(workspace, session_id)
    }

    /// 面板预检（轻量：只读元数据，不加载快照字节）。
    pub fn rewind_availability(
        &self,
        workspace: &Path,
        session_id: &str,
        user_message_id: &str,
    ) -> super::Result<(super::chat::FileAvailability, i64, String)> {
        let db = self.db.as_ref().ok_or(super::DbError::Unavailable)?;
        db.rewind_availability(workspace, session_id, user_message_id)
    }

    /// 派生恢复计划（只读：可用性、线性检查、目标路径）。
    pub fn restore_plan(
        &self,
        workspace: &Path,
        session_id: &str,
        user_message_id: &str,
    ) -> super::Result<super::chat::RestorePlan> {
        let db = self.db.as_ref().ok_or(super::DbError::Unavailable)?;
        db.restore_plan(workspace, session_id, user_message_id)
    }

    /// 记录 mutation 的 before / planned-after（超限在写入前拒绝）。
    pub fn prepare_mutations(
        &self,
        workspace: &Path,
        session_id: &str,
        user_message_id: &str,
        tool_call_id: &str,
        changes: &[super::chat::PathMutationWrite],
    ) -> super::Result<super::chat::PreparedTicket> {
        let db = self.db.as_ref().ok_or(super::DbError::Unavailable)?;
        db.prepare_mutations(
            workspace,
            session_id,
            user_message_id,
            tool_call_id,
            changes,
        )
    }

    /// 记录 mutation 实际观察到的 after。
    pub fn commit_mutations(
        &self,
        workspace: &Path,
        session_id: &str,
        ticket: &super::chat::PreparedTicket,
        observed: &[super::chat::ObservedWrite],
    ) -> super::Result<()> {
        let db = self.db.as_ref().ok_or(super::DbError::Unavailable)?;
        db.commit_mutations(workspace, session_id, ticket, observed)
    }

    /// 开始/读取撤销操作（幂等）。
    #[allow(clippy::too_many_arguments)]
    pub fn begin_rewind_operation(
        &self,
        workspace: &Path,
        operation_id: &str,
        session_id: &str,
        target_user_message_id: &str,
        expected_revision: i64,
        kind: &str,
        user_text: &str,
    ) -> super::Result<super::chat::OperationRecord> {
        let db = self.db.as_ref().ok_or(super::DbError::Unavailable)?;
        db.begin_operation(
            workspace,
            operation_id,
            session_id,
            target_user_message_id,
            expected_revision,
            kind,
            user_text,
        )
    }

    /// 推进撤销操作阶段。
    pub fn set_rewind_phase(
        &self,
        operation_id: &str,
        phase: &str,
        error_code: Option<&str>,
    ) -> super::Result<()> {
        let db = self.db.as_ref().ok_or(super::DbError::Unavailable)?;
        db.set_operation_phase(operation_id, phase, error_code)
    }

    /// 会话中尚未完成的撤销操作。
    pub fn unfinished_rewind(
        &self,
        session_id: &str,
    ) -> super::Result<Option<super::chat::OperationRecord>> {
        let db = self.db.as_ref().ok_or(super::DbError::Unavailable)?;
        db.unfinished_operation(session_id)
    }

    /// 推进撤销操作阶段（启动恢复/测试使用）。
    pub fn set_rewind_operation_phase(
        &self,
        operation_id: &str,
        phase: &str,
        error_code: Option<&str>,
    ) -> super::Result<()> {
        let db = self.db.as_ref().ok_or(super::DbError::Unavailable)?;
        db.set_operation_phase(operation_id, phase, error_code)
    }

    /// 撤销操作记录（幂等重放）。
    pub fn rewind_operation(
        &self,
        operation_id: &str,
    ) -> super::Result<Option<super::chat::OperationRecord>> {
        let db = self.db.as_ref().ok_or(super::DbError::Unavailable)?;
        db.operation(operation_id)
    }

    /// 最终事务：删对话 + 提示 + revision + operation committed。
    pub fn commit_rewind(
        &self,
        workspace: &Path,
        commit: &super::chat::RewindCommit,
    ) -> super::Result<super::chat::RewindCommitted> {
        let db = self.db.as_ref().ok_or(super::DbError::Unavailable)?;
        db.commit_rewind(workspace, commit)
    }

    /// 清理无引用的镜像。
    pub fn prune_rewind_images(&self, session_id: &str) -> super::Result<usize> {
        let db = self.db.as_ref().ok_or(super::DbError::Unavailable)?;
        db.prune_orphan_images(session_id)
    }

    /// 撤销成功后的视图重载：消息、独立提示、compact 检查点与会话 revision。
    pub fn load_rewind_view(
        &self,
        workspace: &Path,
        session_id: &str,
    ) -> super::Result<RewindView> {
        let db = self.db.as_ref().ok_or(super::DbError::Unavailable)?;
        let messages = db
            .load_chat_session(workspace, session_id)?
            .ok_or(super::DbError::SessionMissing)?;
        let notices = db.load_notices(workspace, session_id)?;
        let checkpoints = db.compact_checkpoints(workspace, session_id)?;
        let revision = db.session_revision(session_id)?;
        Ok(RewindView {
            messages,
            notices,
            checkpoints,
            revision,
        })
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

    /// 预登记一条本地命令；成功后才允许 spawn。
    pub fn begin_user_shell(
        &self,
        workspace: &Path,
        session_id: Option<&str>,
        record: &crate::db::chat::UserShellRecord,
    ) -> super::Result<crate::db::chat::UserShellCommitted> {
        let db = self.db.as_ref().ok_or(super::DbError::Unavailable)?;
        db.begin_user_shell(workspace, session_id, record)
    }

    /// 写入命令 checkpoint / 终态（幂等，revision 单调）。
    pub fn update_user_shell(
        &self,
        workspace: &Path,
        message_id: &str,
        record: &crate::db::chat::UserShellRecord,
    ) -> super::Result<bool> {
        let db = self.db.as_ref().ok_or(super::DbError::Unavailable)?;
        db.update_user_shell(workspace, message_id, record)
    }

    /// 读取会话在某个 sequence 之后的本地命令（模型下一次采样前增量吸收）。
    pub fn load_user_shells_since(
        &self,
        workspace: &Path,
        session_id: &str,
        after_sequence: i64,
    ) -> super::Result<Vec<crate::db::chat::ChatMessage>> {
        let db = self.db.as_ref().ok_or(super::DbError::Unavailable)?;
        db.load_user_shells_since(workspace, session_id, after_sequence)
    }

    /// 登记新建进程（`starting`）；失败时调用方必须停止刚 spawn 的进程。
    pub fn begin_process(
        &self,
        workspace: &Path,
        new: &crate::db::chat::NewProcess<'_>,
    ) -> super::Result<String> {
        let db = self.db.as_ref().ok_or(super::DbError::Unavailable)?;
        db.begin_process(workspace, new)
    }

    /// 合并一次输出快照（revision 必须更新，较旧写入零行）。
    pub fn update_process_output(
        &self,
        workspace: &Path,
        runtime_epoch: &str,
        process_session_id: u64,
        snapshot: &crate::db::chat::OutputSnapshot<'_>,
        revision: i64,
    ) -> super::Result<bool> {
        let db = self.db.as_ref().ok_or(super::DbError::Unavailable)?;
        db.update_process_output(
            workspace,
            runtime_epoch,
            process_session_id,
            snapshot,
            revision,
        )
    }

    /// 提交进程终态（revision 必须更新，重复终态幂等）。
    pub fn finish_process(
        &self,
        workspace: &Path,
        finish: &crate::db::chat::ProcessFinish<'_>,
    ) -> super::Result<bool> {
        let db = self.db.as_ref().ok_or(super::DbError::Unavailable)?;
        db.finish_process(workspace, finish)
    }

    /// 批量载入会话进程记录（重载时一次查询，不为每张卡片单独读库）。
    pub fn load_processes_for_session(
        &self,
        workspace: &Path,
        chat_session_id: &str,
    ) -> super::Result<Vec<crate::db::chat::ProcessRecord>> {
        let db = self.db.as_ref().ok_or(super::DbError::Unavailable)?;
        db.load_processes_for_session(workspace, chat_session_id)
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
                    // 失败：保留脏状态与待保存内容，下一轮静置重试并给出可见反馈。
                    update::reset_prompt_autosave(state, prompt);
                    update::show_toast(
                        state,
                        Toast::new(
                            ToastKind::Danger,
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
                            ToastKind::Danger,
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
                            ToastKind::Danger,
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
            Effect::CopyToClipboard { text, anchor } => {
                let now = Instant::now();
                let (kind, message) = if crate::platform::write_clipboard(&text) {
                    (ToastKind::Default, text::TOAST_COPIED)
                } else {
                    tracing::warn!("clipboard write failed");
                    (ToastKind::Danger, text::TOAST_COPY_FAILED)
                };
                // 锚在 lx 窗格时落该窗格自己的槽（不占用全局覆盖层）。
                update::show_toast_at(
                    state,
                    anchor,
                    Toast::new(kind, message, anchor, now).with_title(text::TOAST_CLIPBOARD_TITLE),
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
                // 未完成的撤销（尤其 recovery_required）必须先处理，否则新轮次会叠在
                // 一半恢复的文件状态上。检查放在 begin_turn 之前，避免先加一条 user 消息。
                if let Some(session) = session_id.as_deref()
                    && let Some(Some(operation)) =
                        self.with_db(|db| db.unfinished_operation(session))
                {
                    update::on_turn_failed(state, pane, generation);
                    // 反馈落在发起本轮的那个 lx 窗格自己的槽里。
                    update::show_lx_toast(
                        state,
                        pane,
                        Toast::new(
                            ToastKind::Danger,
                            format!("{} ({})", text::LX_REWIND_RECOVERY_BLOCKED, operation.phase),
                            Some(pane),
                            Instant::now(),
                        ),
                    );
                    return true;
                }
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
                    .chat_session_key_for_pane(key.pane)
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
                let Some(workspace) = state.chat_session_key_for_pane(pane) else {
                    return false;
                };
                // 主 checkout 的 workspace 行可能尚未登记（只开过 worktree）：
                // 先登记再查，否则共享列表会静默变空。
                let _ = self.with_db(|db| db.ensure_workspace(&workspace));
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
                let Some(workspace) = state.chat_session_key_for_pane(pane) else {
                    return false;
                };
                let _ = self.with_db(|db| db.ensure_workspace(&workspace));
                let loaded: Option<LoadedSession> = self.with_db(|db| {
                    let messages = db.load_chat_session(&workspace, &session_id)?;
                    let model = db.chat_session_model(&workspace, &session_id)?;
                    // 进程记录用来校正卡片状态：结果 JSON 写的是「结算那一刻」，
                    // 进程之后可能早已退出/被停止（见 `reconcile_process_records`）。
                    let processes = db
                        .load_processes_for_session(&workspace, &session_id)
                        .unwrap_or_default();
                    // 独立撤销提示、compact 检查点与 revision：三者都不进入 messages，
                    // 也不作为普通消息进模型上下文（检查点只投影为摘要前缀）。
                    let notices = db.load_notices(&workspace, &session_id).unwrap_or_default();
                    let checkpoints = db
                        .compact_checkpoints(&workspace, &session_id)
                        .unwrap_or_default();
                    let revision = db.session_revision(&session_id).unwrap_or(0);
                    Ok((messages, model, processes, notices, checkpoints, revision))
                });
                let Some((Some(messages), model, processes, notices, checkpoints, revision)) =
                    loaded
                else {
                    // 加载失败发生在 lx 页：反馈落该窗格自己的槽（不占用全局覆盖层）。
                    update::show_lx_toast(
                        state,
                        pane,
                        Toast::new(
                            ToastKind::Danger,
                            text::LX_CHAT_SESSION_BUSY,
                            Some(pane),
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
                let changed = update::on_session_loaded_full(
                    state,
                    pane,
                    &session_id,
                    &messages,
                    &notices,
                    revision,
                    selection,
                );
                // 检查点列表随会话一起回填（渲染与撤销都依赖它）。
                let checkpoints_changed =
                    update::set_compact_checkpoints(state, pane, &checkpoints);
                // 会话视图建好之后再按进程记录校正（需要卡片已存在）。
                let drains = update::request_report_drain_if_idle(state, pane);
                update::reconcile_process_records(state, pane, &processes)
                    | changed
                    | checkpoints_changed
                    | drains
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
                // 与轮次/撤销共用 session guard：另一实例或本进程在途轮次占用时拒绝删除，
                // 否则会级联删掉别人正在写入的轮次与工具记录。
                let guard = match crate::platform::try_lock_session(&session_id) {
                    Ok(Some(guard)) => guard,
                    Ok(None) => {
                        session_busy_feedback(state, &session_id);
                        return true;
                    }
                    Err(error) => {
                        tracing::warn!(%error, "session guard unavailable");
                        session_busy_feedback(state, &session_id);
                        return true;
                    }
                };
                // 进程记录由外键 `ON DELETE CASCADE` 随会话一起删。**不要**在这里
                // 单独预删：预删成功而会话删除失败时，会话还在、进程元数据却已清空，
                // 历史里 `running` 的卡片会永远停在 Running 且没有进程可停。
                let deleted = self.with_db(|db| db.delete_chat_session(&workspace, &session_id));
                // guard 必须活到删除落库之后。
                drop(guard);
                // 删除成功后解绑引用该会话的窗格（所有工作区内的窗格都可能持有），
                // 并从已打开的 `/resume` 列表里就地移除该行。
                if deleted.is_some() {
                    let mut changed = false;
                    // 子树 turn 仍在跑：取消它们，否则会把结果写进已删除的子会话。
                    state.effects.push(Effect::CancelSubagents {
                        session_id: session_id.clone(),
                        agent_id: None,
                    });
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
            Effect::DeleteTodosWithStatus { status } => {
                self.todo_write(state, |db| db.delete_todos_with_status(status).map(|_| ()));
                false
            }
            Effect::RestoreTodos { records } => {
                self.todo_write(state, |db| db.restore_todos(&records));
                false
            }
            // 网络与进程效应由事件层执行（pump 已过滤；此处兜底忽略并记录）。
            // 进程句柄只存在于事件层的运行实例里，持久化层既读不到也停不了。
            Effect::StartChatStream { .. }
            | Effect::CancelChatStream { .. }
            | Effect::AnswerAsk { .. } => {
                tracing::warn!("chat network effect reached persistence pump");
                false
            }
            Effect::ListSessionProcesses { .. } | Effect::StopSessionProcess { .. } => {
                tracing::warn!("process effect reached persistence pump");
                false
            }
            // 撤销/压缩效应由事件层取走；此处只兜底忽略。
            Effect::PrepareRewind { .. }
            | Effect::ExecuteRewind(_)
            | Effect::ExecuteCompact(_)
            | Effect::RevertCompact(_) => {
                tracing::warn!("rewind/compact effect reached persistence pump");
                false
            }
            Effect::BeginUserShell { .. } | Effect::CancelUserShells { .. } => {
                tracing::warn!("user shell effect reached persistence pump");
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
}

/// 是否为聊天网络/撤销效应（由事件层执行，非同步 DB 写）。
fn is_network_effect(effect: &Effect) -> bool {
    matches!(
        effect,
        Effect::StartChatStream { .. }
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
            | Effect::BeginUserShell { .. }
            | Effect::CancelUserShells { .. }
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
mod resume_tests;
#[cfg(test)]
mod tests;
