//! lx 聊天会话、历史、撤销、压缩与模型切换的持久化辅助方法及效应执行。
//!
//! 从 `service.rs` 按职责拆出：这里只做 DB 读写与状态层回投，不做网络或文件 IO。

use std::path::Path;
use std::time::Instant;

use crate::app::effects::Effect;
use crate::app::state::AppState;
use crate::app::toast::{Toast, ToastKind};
use crate::app::update;
use crate::config::ai::ModelSelection;
use crate::config::sandbox::SandboxMode;
use crate::ui::text;

use super::Persistence;

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
    Vec<crate::db::chat::ModelSwitchRecord>,
    Vec<crate::db::chat::MessageRecap>,
    i64,
);

impl Persistence {
    /// 条件更新会话标题（仅当数据库当前标题仍等于 `expected_title` 时改名）。
    pub fn rename_chat_session_if_title_matches(
        &self,
        workspace: &Path,
        session_id: &str,
        expected_title: &str,
        new_title: &str,
    ) -> bool {
        self.with_db(|db| {
            db.rename_chat_session_if_title_matches(
                workspace,
                session_id,
                expected_title,
                new_title,
            )
        })
        .unwrap_or(false)
    }

    /// 最新用户轮次（撤销目标与 revision）。
    pub fn latest_user_turn(
        &self,
        workspace: &Path,
        session_id: &str,
    ) -> crate::db::Result<Option<crate::db::chat::LatestTurn>> {
        let db = self.db.as_ref().ok_or(crate::db::DbError::Unavailable)?;
        db.latest_user_turn(workspace, session_id)
    }

    /// 面板预检（轻量：只读元数据，不加载快照字节）。
    pub fn rewind_availability(
        &self,
        workspace: &Path,
        session_id: &str,
        user_message_id: &str,
    ) -> crate::db::Result<(crate::db::chat::FileAvailability, i64, String)> {
        let db = self.db.as_ref().ok_or(crate::db::DbError::Unavailable)?;
        db.rewind_availability(workspace, session_id, user_message_id)
    }

    /// 派生恢复计划（只读：可用性、线性检查、目标路径）。
    pub fn restore_plan(
        &self,
        workspace: &Path,
        session_id: &str,
        user_message_id: &str,
    ) -> crate::db::Result<crate::db::chat::RestorePlan> {
        let db = self.db.as_ref().ok_or(crate::db::DbError::Unavailable)?;
        db.restore_plan(workspace, session_id, user_message_id)
    }

    /// 记录 mutation 的 before / planned-after（超限在写入前拒绝）。
    pub fn prepare_mutations(
        &self,
        workspace: &Path,
        session_id: &str,
        user_message_id: &str,
        tool_call_id: &str,
        changes: &[crate::db::chat::PathMutationWrite],
    ) -> crate::db::Result<crate::db::chat::PreparedTicket> {
        let db = self.db.as_ref().ok_or(crate::db::DbError::Unavailable)?;
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
        ticket: &crate::db::chat::PreparedTicket,
        observed: &[crate::db::chat::ObservedWrite],
    ) -> crate::db::Result<()> {
        let db = self.db.as_ref().ok_or(crate::db::DbError::Unavailable)?;
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
    ) -> crate::db::Result<crate::db::chat::OperationRecord> {
        let db = self.db.as_ref().ok_or(crate::db::DbError::Unavailable)?;
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
    ) -> crate::db::Result<()> {
        let db = self.db.as_ref().ok_or(crate::db::DbError::Unavailable)?;
        db.set_operation_phase(operation_id, phase, error_code)
    }

    /// 会话中尚未完成的撤销操作。
    pub fn unfinished_rewind(
        &self,
        session_id: &str,
    ) -> crate::db::Result<Option<crate::db::chat::OperationRecord>> {
        let db = self.db.as_ref().ok_or(crate::db::DbError::Unavailable)?;
        db.unfinished_operation(session_id)
    }

    /// 推进撤销操作阶段（启动恢复/测试使用）。
    pub fn set_rewind_operation_phase(
        &self,
        operation_id: &str,
        phase: &str,
        error_code: Option<&str>,
    ) -> crate::db::Result<()> {
        let db = self.db.as_ref().ok_or(crate::db::DbError::Unavailable)?;
        db.set_operation_phase(operation_id, phase, error_code)
    }

    /// 撤销操作记录（幂等重放）。
    pub fn rewind_operation(
        &self,
        operation_id: &str,
    ) -> crate::db::Result<Option<crate::db::chat::OperationRecord>> {
        let db = self.db.as_ref().ok_or(crate::db::DbError::Unavailable)?;
        db.operation(operation_id)
    }

    /// 最终事务：删对话 + 提示 + revision + operation committed。
    pub fn commit_rewind(
        &self,
        workspace: &Path,
        commit: &crate::db::chat::RewindCommit,
    ) -> crate::db::Result<crate::db::chat::RewindCommitted> {
        let db = self.db.as_ref().ok_or(crate::db::DbError::Unavailable)?;
        db.commit_rewind(workspace, commit)
    }

    /// 清理无引用的镜像。
    pub fn prune_rewind_images(&self, session_id: &str) -> crate::db::Result<usize> {
        let db = self.db.as_ref().ok_or(crate::db::DbError::Unavailable)?;
        db.prune_orphan_images(session_id)
    }

    /// 撤销成功后的视图重载：消息、独立提示、compact 检查点与会话 revision。
    pub fn load_rewind_view(
        &self,
        workspace: &Path,
        session_id: &str,
    ) -> crate::db::Result<super::RewindView> {
        let db = self.db.as_ref().ok_or(crate::db::DbError::Unavailable)?;
        let messages = db
            .load_chat_session(workspace, session_id)?
            .ok_or(crate::db::DbError::SessionMissing)?;
        let notices = db.load_notices(workspace, session_id)?;
        let checkpoints = db.compact_checkpoints(workspace, session_id)?;
        let switches = db.model_switches(workspace, session_id)?;
        let permission_switches = db.permission_switches(workspace, session_id)?;
        let recaps = db.load_recaps(workspace, session_id)?;
        let revision = db.session_revision(session_id)?;
        Ok(super::RewindView {
            messages,
            notices,
            checkpoints,
            switches,
            permission_switches,
            recaps,
            revision,
        })
    }

    /// 读取某会话中指定附件的图片 BLOB。
    pub fn load_image_payloads(
        &self,
        workspace: &Path,
        session_id: &str,
        attachment_ids: &[&str],
    ) -> crate::db::Result<std::collections::HashMap<String, Vec<u8>>> {
        let db = self.db.as_ref().ok_or(crate::db::DbError::Unavailable)?;
        db.load_image_payloads(workspace, session_id, attachment_ids)
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
    ) -> crate::db::Result<crate::db::chat::ToolCallTicket> {
        let db = self.db.as_ref().ok_or(crate::db::DbError::Unavailable)?;
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
    ) -> crate::db::Result<()> {
        let db = self.db.as_ref().ok_or(crate::db::DbError::Unavailable)?;
        db.mark_tool_running(workspace, ticket)
    }

    /// 结算工具调用（running -> 终态；pending -> cancelled 允许）。
    pub fn finish_tool_call(
        &self,
        workspace: &Path,
        ticket: &crate::db::chat::ToolCallTicket,
        result: &crate::db::chat::ToolCallResult,
    ) -> crate::db::Result<()> {
        let db = self.db.as_ref().ok_or(crate::db::DbError::Unavailable)?;
        db.finish_tool_call(workspace, ticket, result)
    }

    /// steer 提交：同一事务关闭旧 streaming assistant 并开启新轮；失败不产生半轮。
    #[allow(clippy::too_many_arguments)]
    pub fn begin_steer_turn(
        &self,
        workspace: &Path,
        session_id: &str,
        previous_assistant_id: &str,
        previous_parts: &[crate::db::chat::ChatPart],
        user_text: &str,
        attachments: &[crate::db::chat::MessageAttachmentInput],
        model: &crate::db::chat::ModelSnapshot,
    ) -> crate::db::Result<crate::db::chat::TurnCommitted> {
        let db = self.db.as_ref().ok_or(crate::db::DbError::Unavailable)?;
        db.begin_steer_turn(
            workspace,
            session_id,
            previous_assistant_id,
            previous_parts,
            user_text,
            attachments,
            model,
        )
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
    ) -> crate::db::Result<()> {
        let db = self.db.as_ref().ok_or(crate::db::DbError::Unavailable)?;
        db.checkpoint(workspace, assistant_id, parts)
    }

    /// 预登记一条本地命令；成功后才允许 spawn。
    pub fn begin_user_shell(
        &self,
        workspace: &Path,
        session_id: Option<&str>,
        record: &crate::db::chat::UserShellRecord,
    ) -> crate::db::Result<crate::db::chat::UserShellCommitted> {
        let db = self.db.as_ref().ok_or(crate::db::DbError::Unavailable)?;
        db.begin_user_shell(workspace, session_id, record)
    }

    /// 写入命令 checkpoint / 终态（幂等，revision 单调）。
    pub fn update_user_shell(
        &self,
        workspace: &Path,
        message_id: &str,
        record: &crate::db::chat::UserShellRecord,
    ) -> crate::db::Result<bool> {
        let db = self.db.as_ref().ok_or(crate::db::DbError::Unavailable)?;
        db.update_user_shell(workspace, message_id, record)
    }

    /// 读取会话在某个 sequence 之后的本地命令（模型下一次采样前增量吸收）。
    pub fn load_user_shells_since(
        &self,
        workspace: &Path,
        session_id: &str,
        after_sequence: i64,
    ) -> crate::db::Result<Vec<crate::db::chat::ChatMessage>> {
        let db = self.db.as_ref().ok_or(crate::db::DbError::Unavailable)?;
        db.load_user_shells_since(workspace, session_id, after_sequence)
    }

    /// 登记新建进程（`starting`）；失败时调用方必须停止刚 spawn 的进程。
    pub fn begin_process(
        &self,
        workspace: &Path,
        new: &crate::db::chat::NewProcess<'_>,
    ) -> crate::db::Result<String> {
        let db = self.db.as_ref().ok_or(crate::db::DbError::Unavailable)?;
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
    ) -> crate::db::Result<bool> {
        let db = self.db.as_ref().ok_or(crate::db::DbError::Unavailable)?;
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
    ) -> crate::db::Result<bool> {
        let db = self.db.as_ref().ok_or(crate::db::DbError::Unavailable)?;
        db.finish_process(workspace, finish)
    }

    /// 写入或覆盖一条消息回顾。
    pub fn commit_recap(
        &self,
        workspace: &Path,
        commit: &crate::db::chat::RecapCommit,
    ) -> crate::db::Result<crate::db::chat::MessageRecap> {
        let db = self.db.as_ref().ok_or(crate::db::DbError::Unavailable)?;
        db.commit_recap(workspace, commit)
    }

    /// 撤销最新一次模型切换。
    pub fn revert_model_switch(
        &self,
        workspace: &Path,
        session_id: &str,
        switch_id: &str,
        expected_revision: i64,
        operation_id: &str,
    ) -> crate::db::Result<Option<crate::db::chat::ModelSnapshot>> {
        let db = self.db.as_ref().ok_or(crate::db::DbError::Unavailable)?;
        db.revert_model_switch(
            workspace,
            session_id,
            switch_id,
            expected_revision,
            operation_id,
        )
    }

    /// 删除一条消息回顾。
    pub fn delete_recap(
        &self,
        workspace: &Path,
        session_id: &str,
        message_id: &str,
        expected_revision: i64,
        operation_id: &str,
    ) -> crate::db::Result<()> {
        let db = self.db.as_ref().ok_or(crate::db::DbError::Unavailable)?;
        db.delete_recap(
            workspace,
            session_id,
            message_id,
            expected_revision,
            operation_id,
        )
    }

    /// 批量载入会话进程记录（重载时一次查询，不为每张卡片单独读库）。
    pub fn load_processes_for_session(
        &self,
        workspace: &Path,
        chat_session_id: &str,
    ) -> crate::db::Result<Vec<crate::db::chat::ProcessRecord>> {
        let db = self.db.as_ref().ok_or(crate::db::DbError::Unavailable)?;
        db.load_processes_for_session(workspace, chat_session_id)
    }

    /// 装载会话到窗格：消息、模型、进程、撤销提示、compact 检查点、模型切换与 revision。
    ///
    /// `/resume` 与 `/fork` 共用；返回状态是否有变化。失败时在窗格内提示会话忙。
    fn load_chat_session_into_pane(
        &self,
        state: &mut AppState,
        pane: crate::tiling::PaneId,
        workspace: &Path,
        session_id: &str,
    ) -> bool {
        let _ = self.with_db(|db| db.ensure_workspace(workspace));
        // 旁路会话（`source='btw'`）与子会话只能由各自的面板打开，不能当主会话恢复。
        if !self
            .with_db(|db| db.is_native_session(workspace, session_id))
            .unwrap_or(false)
        {
            tracing::warn!(session = %session_id, "refusing to resume a non-native session");
            return false;
        }
        let loaded: Option<LoadedSession> = self.with_db(|db| {
            let messages = db.load_chat_session(workspace, session_id)?;
            let model = db.chat_session_model(workspace, session_id)?;
            // 进程记录用来校正卡片状态：结果 JSON 写的是「结算那一刻」，
            // 进程之后可能早已退出/被停止（见 `reconcile_process_records`）。
            let processes = db
                .load_processes_for_session(workspace, session_id)
                .unwrap_or_default();
            // 独立撤销提示、compact 检查点与 revision：三者都不进入 messages，
            // 也不作为普通消息进模型上下文（检查点只投影为摘要前缀）。
            let notices = db.load_notices(workspace, session_id).unwrap_or_default();
            let checkpoints = db
                .compact_checkpoints(workspace, session_id)
                .unwrap_or_default();
            let switches = db.model_switches(workspace, session_id).unwrap_or_default();
            let recaps = db.load_recaps(workspace, session_id).unwrap_or_default();
            let revision = db.session_revision(session_id).unwrap_or(0);
            Ok((
                messages,
                model,
                processes,
                notices,
                checkpoints,
                switches,
                recaps,
                revision,
            ))
        });
        let Some((
            Some(messages),
            model,
            processes,
            notices,
            checkpoints,
            switches,
            recaps,
            revision,
        )) = loaded
        else {
            if let Some(target) = state.pane_mut_anywhere(pane) {
                target.lx.chat.loading_session = false;
            }
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
        let selection = model.map(|snapshot| ModelSelection {
            provider: snapshot.provider,
            model: snapshot.model,
            variant: snapshot.variant,
        });
        let session_title = self
            .with_db(|db| db.chat_session_title(workspace, session_id))
            .flatten();
        let changed = update::on_session_loaded_full(
            state, pane, session_id, &messages, &notices, revision, selection,
        );
        // 恢复会话的沙箱覆盖（NULL = 跟随全局默认）。
        let sandbox_mode = self
            .with_db(|db| db.session_sandbox_mode(workspace, session_id))
            .flatten()
            .and_then(|text| SandboxMode::parse(&text));
        let sandbox_rules = self
            .with_db(|db| db.sandbox_rules(workspace, session_id))
            .unwrap_or_default();
        if let Some(target) = state.pane_mut_anywhere(pane) {
            target.lx.sandbox_mode = sandbox_mode;
        }
        let sandbox_rules_changed = update::set_sandbox_rules(state, pane, &sandbox_rules);
        // 恢复会话后立即列出子 agent：回投后填充 `pane.lx.agents.runs`，
        // 并把其中 `running` / `idle` / `blocked` 的实例同步进 `@` 面板。
        state.effects.push(Effect::ListSubagents {
            pane,
            parent_session: session_id.to_string(),
        });
        let title_changed = state
            .pane_mut_anywhere(pane)
            .is_some_and(|target| target.lx.chat.set_session_title(session_title));
        // 检查点、模型/权限切换列表随会话一起回填（渲染与撤销都依赖它）。
        let checkpoints_changed = update::set_compact_checkpoints(state, pane, &checkpoints);
        let switches_changed = update::set_model_switches(state, pane, &switches);
        let permission_switches = self
            .with_db(|db| db.permission_switches(workspace, session_id))
            .unwrap_or_default();
        let perm_switches_changed =
            update::set_permission_switches(state, pane, &permission_switches);
        let recaps_changed = update::set_message_recaps(state, pane, &recaps);
        // 会话视图建好之后再按进程记录校正（需要卡片已存在）。
        let drains = update::request_report_drain_if_idle(state, pane);
        update::reconcile_process_records(state, pane, &processes)
            | changed
            | sandbox_rules_changed
            | title_changed
            | checkpoints_changed
            | switches_changed
            | perm_switches_changed
            | recaps_changed
            | drains
    }

    /// 执行 lx 聊天类效应；返回状态是否有变化。
    pub(super) fn execute_chat_effect(&mut self, state: &mut AppState, effect: Effect) -> bool {
        match effect {
            Effect::BeginChatTurn {
                pane,
                generation,
                workspace,
                session_id,
                draft,
                attachments,
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
                let committed = self.with_db(|db| {
                    db.begin_turn_with_attachments(
                        &workspace,
                        session_id.as_deref(),
                        &draft,
                        &attachments,
                        &model,
                    )
                });
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
                billing,
            } => {
                // workspace 按窗格当前归属解析（关闭窗格后仍按原 workspace 完成写入）。
                let workspace = state
                    .chat_session_key_for_pane(key.pane)
                    .unwrap_or_else(|| std::path::PathBuf::from(""));
                let success = self.with_db(|db| {
                    db.finish_with_billing(
                        &workspace,
                        &assistant_id,
                        &parts,
                        &outcome,
                        usage,
                        billing,
                    )
                });
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
                    if let Some(target) = state.pane_mut_anywhere(pane)
                        && let Some(crate::app::state::SlashPanel::History(panel)) =
                            target.lx.slash.as_mut()
                    {
                        panel.loading = false;
                    }
                    return false;
                };
                // 主 checkout 的 workspace 行可能尚未登记（只开过 worktree）：
                // 先登记再查，否则共享列表会静默变空。
                let _ = self.with_db(|db| db.ensure_workspace(&workspace));
                let entries = self.with_db(|db| {
                    db.list_chat_sessions(&workspace, 0, crate::db::chat::SESSION_PAGE_SIZE)
                });
                let Some(entries) = entries else {
                    if let Some(target) = state.pane_mut_anywhere(pane)
                        && let Some(crate::app::state::SlashPanel::History(panel)) =
                            target.lx.slash.as_mut()
                    {
                        panel.loading = false;
                    }
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
                    if let Some(target) = state.pane_mut_anywhere(pane) {
                        target.lx.chat.loading_session = false;
                    }
                    return false;
                };
                let loaded = self.load_chat_session_into_pane(state, pane, &workspace, &session_id);
                if !loaded && let Some(target) = state.pane_mut_anywhere(pane) {
                    target.lx.chat.loading_session = false;
                }
                loaded
            }
            Effect::ForkChatSession {
                pane,
                workspace,
                session_id,
                before,
            } => {
                let Some(forked) = self
                    .with_db(|db| db.fork_chat_session(&workspace, &session_id, before.as_deref()))
                else {
                    update::on_fork_failed(state, pane);
                    return true;
                };
                self.load_chat_session_into_pane(state, pane, &workspace, &forked.session_id);
                let is_bound = state.pane_anywhere(pane).is_some_and(|target| {
                    target.lx.chat.session_id.as_deref() == Some(&forked.session_id)
                });
                if !is_bound {
                    update::on_fork_failed(state, pane);
                    return true;
                }
                update::on_fork_completed(state, pane, forked.restore_text.as_deref())
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
                    let panes: Vec<crate::tiling::PaneId> = state.all_pane_ids();
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
                        super::session_busy_feedback(state, &session_id);
                        return true;
                    }
                    Err(error) => {
                        tracing::warn!(%error, "session guard unavailable");
                        super::session_busy_feedback(state, &session_id);
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
                    let panes: Vec<crate::tiling::PaneId> = state.all_pane_ids();
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
                commit,
            } => {
                if let Some(mut commit) = commit {
                    // 提示词渲染发生在纯状态层，拿不到磁盘；这里用窗格真实 cwd 与项目根
                    // 重新加载指令，补齐 `core/agents_md` 后重新生成权威 `injected_prompt`。
                    let user_root = crate::config::user_data_root().ok();
                    let project_root = super::prompt_key_for(state, &workspace);
                    let effective_cwd = commit.cwd.as_deref().unwrap_or(&workspace);
                    let trusted = self.is_trusted(effective_cwd)
                        || self.is_trusted(&workspace)
                        || self.is_trusted(&project_root);
                    let loaded_inst = crate::chat::agent::load_instructions(
                        user_root.as_deref(),
                        Some(&project_root),
                        Some(effective_cwd),
                        trusted,
                    );
                    if !loaded_inst.is_empty()
                        && !commit
                            .updated_sources
                            .iter()
                            .any(|s| s == crate::chat::agent::SOURCE_CORE_AGENTS_MD)
                    {
                        commit
                            .updated_sources
                            .push(crate::chat::agent::SOURCE_CORE_AGENTS_MD.to_string());
                    }
                    // 无论本次是否产生切换记录都重算：落库的提示词必须与来源列表一致。
                    commit.injected_prompt = crate::chat::agent::render_model_switch_update(
                        commit.from_label.as_deref(),
                        &commit.to_identity,
                        &commit.updated_sources,
                        &loaded_inst.source_labels(),
                    );
                    // 即使 record 失败或没有产生记录也重载：不能让窗格保留乐观切换。
                    let recorded = self.with_db(|db| {
                        let _ = db.record_model_switch(&workspace, &commit);
                        db.model_switches(&workspace, &session_id)
                    });
                    if let Some(switches) = recorded {
                        let selection = ModelSelection {
                            provider: model.provider.clone(),
                            model: model.model.clone(),
                            variant: model.variant.clone(),
                        };
                        let mut changed = false;
                        for pane in state.all_pane_ids() {
                            if state.pane_anywhere(pane).is_some_and(|target| {
                                target.lx.chat.session_id.as_deref() == Some(session_id.as_str())
                            }) {
                                changed |= update::set_model_switches(state, pane, &switches);
                                if let Some(target) = state.pane_mut_anywhere(pane) {
                                    let next = Some(selection.clone());
                                    if target.lx.chat.selection != next {
                                        target.lx.chat.selection = next;
                                        changed = true;
                                    }
                                }
                            }
                        }
                        return changed;
                    }
                } else {
                    self.with_db(|db| db.set_chat_session_model(&workspace, &session_id, &model));
                }
                false
            }
            Effect::PersistSandboxMode {
                workspace,
                session_id,
                mode,
                switch_commit,
            } => {
                let default_mode = state.sandbox.mode;
                let commit_backup = switch_commit.clone();
                let result = self.db_result(|db| {
                    if let Some(commit) = switch_commit {
                        let cleared = commit.from_mode != commit.to_mode;
                        db.record_permission_switch(&workspace, &commit)?;
                        let switches = db.permission_switches(&workspace, &session_id)?;
                        Ok((Some(switches), cleared))
                    } else {
                        let prev = db
                            .session_sandbox_mode(&workspace, &session_id)?
                            .as_deref()
                            .and_then(SandboxMode::parse);
                        let cleared = mode.is_some_and(|next| next != prev.unwrap_or(default_mode))
                            || (mode.is_none() && prev.is_some());
                        db.set_session_sandbox_mode(
                            &workspace,
                            &session_id,
                            mode.map(SandboxMode::as_str),
                        )?;
                        if cleared {
                            db.clear_sandbox_rules(&session_id)?;
                        }
                        Ok((None, cleared))
                    }
                });
                match result {
                    Ok((switches, cleared)) => {
                        let mut changed =
                            update::sync_session_sandbox_mode(state, &session_id, mode, cleared);
                        if let Some(switches) = switches {
                            for target_pane in state.all_pane_ids() {
                                if state.pane_anywhere(target_pane).is_some_and(|target| {
                                    target.lx.chat.session_id.as_deref()
                                        == Some(session_id.as_str())
                                }) {
                                    changed |= update::set_permission_switches(
                                        state,
                                        target_pane,
                                        &switches,
                                    );
                                }
                            }
                        }
                        changed
                    }
                    Err(error) => {
                        tracing::warn!(%error, session = %session_id, "failed to persist sandbox mode");
                        let reverted = self.with_db(|db| {
                            let auth_mode = db
                                .session_sandbox_mode(&workspace, &session_id)?
                                .as_deref()
                                .and_then(SandboxMode::parse);
                            let switches = db.permission_switches(&workspace, &session_id)?;
                            let rules = db.sandbox_rules(&workspace, &session_id)?;
                            Ok((auth_mode, switches, rules))
                        });
                        if let Some((auth_mode, switches, rules)) = reverted {
                            update::sync_session_sandbox_mode(state, &session_id, auth_mode, true);
                            for target_pane in state.all_pane_ids() {
                                if state.pane_anywhere(target_pane).is_some_and(|target| {
                                    target.lx.chat.session_id.as_deref()
                                        == Some(session_id.as_str())
                                }) {
                                    update::set_permission_switches(state, target_pane, &switches);
                                    update::set_sandbox_rules(state, target_pane, &rules);
                                }
                            }
                        } else if let Some(commit) = commit_backup.as_ref() {
                            let fallback = SandboxMode::parse(&commit.from_mode);
                            update::sync_session_sandbox_mode(state, &session_id, fallback, false);
                            for target_pane in state.all_pane_ids() {
                                if let Some(target) = state.pane_mut_anywhere(target_pane)
                                    && target.lx.chat.session_id.as_deref()
                                        == Some(session_id.as_str())
                                    && target.lx.permission_switch.switches.last().is_some_and(
                                        |last| {
                                            last.from_mode == commit.from_mode
                                                && last.to_mode == commit.to_mode
                                                && last.injected_prompt == commit.injected_prompt
                                        },
                                    )
                                {
                                    target.lx.permission_switch.switches.pop();
                                }
                            }
                        }
                        let toast = Toast::new(
                            ToastKind::Warning,
                            "Failed to save permission mode; reverted to previous state",
                            None,
                            Instant::now(),
                        );
                        if !update::show_lx_toast_for_session(state, &session_id, toast.clone()) {
                            update::show_toast(state, toast);
                        }
                        true
                    }
                }
            }
            other => {
                tracing::warn!(?other, "non-chat effect reached chat handler");
                false
            }
        }
    }
}

/// 是否为 lx 聊天类持久化效应（与网络效应互斥）。
pub(super) fn handles(effect: &Effect) -> bool {
    matches!(
        effect,
        Effect::BeginChatTurn { .. }
            | Effect::CancelChatCommit { .. }
            | Effect::FinishChatTurn { .. }
            | Effect::CheckpointChatTurn { .. }
            | Effect::ListChatSessions { .. }
            | Effect::LoadChatSession { .. }
            | Effect::ForkChatSession { .. }
            | Effect::RenameChatSession { .. }
            | Effect::DeleteChatSession { .. }
            | Effect::SetChatSessionModel { .. }
            | Effect::PersistSandboxMode { .. }
    )
}
