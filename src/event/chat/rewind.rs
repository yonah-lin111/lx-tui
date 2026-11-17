//! 最新轮次撤销的事件层入口：同步预检、后台执行与成功后的视图重载。
//!
//! 撤销文件 IO 与最终 DB 事务在 `spawn_blocking` 的独立连接上执行；事件循环不做
//! 文件读写，也不乐观删除消息——只有 `RewindFinished` 回投后才重载视图并回填。

mod recovery;
mod restore;

pub use recovery::recover_startup;

use crate::app::effects::Effect;
use crate::app::state::{AppState, RewindAction};
use crate::db::chat::FileAvailability;
use crate::db::service::Persistence;
use crate::event::AppEvent;
use crate::tiling::PaneId;

/// 一次撤销的最终结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RewindOutcome {
    Succeeded {
        action: RewindAction,
        restored_files: usize,
        already_restored: bool,
        user_text: String,
        deleted: bool,
        /// 删空对话后会话已被整体移除（UI 需解绑并回到空白草稿）。
        session_deleted: bool,
    },
    Failed {
        message: String,
    },
}

/// 供无头 CLI 与事件层共用的同步撤销执行入口（在给定数据库连接上执行预检、文件恢复与事务提交）。
pub fn execute_rewind(
    database: &crate::db::Database,
    workspace: &std::path::Path,
    request: &crate::app::effects::RewindRequest,
) -> RewindOutcome {
    restore::run(database, workspace, request)
}

/// 取走并执行撤销类效应；返回是否发生变化。
pub fn apply_rewind_effects(
    state: &mut AppState,
    sender: &tokio::sync::mpsc::UnboundedSender<AppEvent>,
    persistence: &Persistence,
) -> bool {
    let effects: Vec<Effect> = state
        .effects
        .iter()
        .filter(|effect| {
            matches!(
                effect,
                Effect::PrepareRewind { .. }
                    | Effect::ExecuteRewind(_)
                    | Effect::PrepareDetailRewind { .. }
                    | Effect::ExecuteDetailRewind(_)
                    | Effect::RevertModelSwitch(_)
                    | Effect::DeleteRecap(_)
            )
        })
        .cloned()
        .collect();
    if effects.is_empty() {
        return false;
    }
    state.effects.retain(|effect| {
        !matches!(
            effect,
            Effect::PrepareRewind { .. }
                | Effect::ExecuteRewind(_)
                | Effect::PrepareDetailRewind { .. }
                | Effect::ExecuteDetailRewind(_)
                | Effect::RevertModelSwitch(_)
                | Effect::DeleteRecap(_)
        )
    });
    for effect in effects {
        match effect {
            Effect::PrepareRewind {
                pane,
                session_id,
                user_message_id,
            } => prepare(state, pane, &session_id, &user_message_id, persistence),
            Effect::ExecuteRewind(request) => execute(sender, persistence, request),
            Effect::PrepareDetailRewind {
                pane,
                target,
                input_at_open,
            } => {
                super::detail_rewind::prepare(state, persistence, pane, target, input_at_open);
            }
            Effect::ExecuteDetailRewind(request) => {
                super::detail_rewind::execute(sender, persistence, request)
            }
            Effect::RevertModelSwitch(request) => revert_model_switch(state, persistence, request),
            Effect::DeleteRecap(request) => delete_recap(state, persistence, request),
            _ => {}
        }
    }
    true
}

/// 模型切换撤销：同步 DB 事务后重载切换列表并把会话选择回滚到切换前。
fn revert_model_switch(
    state: &mut AppState,
    persistence: &Persistence,
    request: crate::app::effects::ModelSwitchRevertRequest,
) {
    let expected_revision = i64::try_from(request.expected_revision).unwrap_or(-1);
    let result = persistence.revert_model_switch(
        &request.workspace,
        &request.session_id,
        &request.switch_id,
        expected_revision,
        &request.operation_id,
    );
    match result {
        Ok(previous) => {
            let models = state.models.clone();
            let loaded = persistence.load_rewind_view(&request.workspace, &request.session_id);
            let selection = previous.map(|snapshot| {
                models.normalize_selection(crate::config::ai::ModelSelection {
                    provider: snapshot.provider,
                    model: snapshot.model,
                    variant: snapshot.variant,
                })
            });
            // 同一会话可能被多个 pane 绑定：模型与切换列表必须一起回滚。
            for pane in state.all_pane_ids() {
                if !state.pane_anywhere(pane).is_some_and(|target| {
                    target.lx.chat.session_id.as_deref() == Some(request.session_id.as_str())
                }) {
                    continue;
                }
                match &loaded {
                    Ok(view) => {
                        crate::app::update::set_model_switches(state, pane, &view.switches);
                        crate::app::update::set_permission_switches(
                            state,
                            pane,
                            &view.permission_switches,
                        );
                        if let Some(target) = state.pane_mut_anywhere(pane) {
                            target.lx.chat.revision = u64::try_from(view.revision).unwrap_or(0);
                        }
                    }
                    Err(error) => {
                        tracing::warn!(%error, "model switch reverted but reload failed");
                        if let Some(target) = state.pane_mut_anywhere(pane) {
                            target
                                .lx
                                .model_switch
                                .switches
                                .retain(|switch| switch.id != request.switch_id);
                        }
                    }
                }
                if let Some(target) = state.pane_mut_anywhere(pane) {
                    target.lx.chat.selection = selection.clone();
                    target.lx.clear_rewind();
                    if matches!(
                        target.lx.slash,
                        Some(crate::app::state::SlashPanel::Rewind(_))
                    ) {
                        target.lx.slash = None;
                    }
                    target.lx.chat.follow_bottom = true;
                }
                crate::app::update::refresh_lx_status(state, pane);
            }
            crate::app::update::show_lx_hint(
                state,
                request.pane,
                crate::app::state::InputHintLevel::Warning,
                crate::ui::text::LX_MODEL_SWITCH_REWOUND,
            );
        }
        Err(error) => {
            crate::app::update::on_rewind_failed(state, request.pane, error.code());
        }
    }
}

/// recap 删除：同步 DB 事务后重载回顾列表。
fn delete_recap(
    state: &mut AppState,
    persistence: &Persistence,
    request: crate::app::effects::RecapDeleteRequest,
) {
    let expected_revision = i64::try_from(request.expected_revision).unwrap_or(-1);
    let result = persistence.delete_recap(
        &request.workspace,
        &request.session_id,
        &request.message_id,
        expected_revision,
        &request.operation_id,
    );
    match result {
        Ok(()) => {
            let loaded = persistence.load_rewind_view(&request.workspace, &request.session_id);
            // 同一会话可能被多个 pane 绑定：recap 列表必须一起刷新。
            for pane in state.all_pane_ids() {
                if !state.pane_anywhere(pane).is_some_and(|target| {
                    target.lx.chat.session_id.as_deref() == Some(request.session_id.as_str())
                }) {
                    continue;
                }
                match &loaded {
                    Ok(view) => {
                        crate::app::update::set_message_recaps(state, pane, &view.recaps);
                        if let Some(target) = state.pane_mut_anywhere(pane) {
                            target.lx.chat.revision = u64::try_from(view.revision).unwrap_or(0);
                        }
                    }
                    Err(error) => {
                        tracing::warn!(%error, "recap deleted but reload failed");
                        if let Some(target) = state.pane_mut_anywhere(pane) {
                            target.lx.recap.entries.remove(&request.message_id);
                        }
                    }
                }
                if let Some(target) = state.pane_mut_anywhere(pane) {
                    target.lx.clear_rewind();
                    if matches!(
                        target.lx.slash,
                        Some(crate::app::state::SlashPanel::Rewind(_))
                    ) {
                        target.lx.slash = None;
                    }
                    target.lx.chat.follow_bottom = true;
                }
                crate::app::update::refresh_lx_status(state, pane);
            }
            crate::app::update::show_lx_hint(
                state,
                request.pane,
                crate::app::state::InputHintLevel::Warning,
                crate::ui::text::LX_RECAP_REWOUND,
            );
        }
        Err(error) => {
            crate::app::update::on_rewind_failed(state, request.pane, error.code());
        }
    }
}

/// 同步预检：只读 DB，更新面板可用性；失败按 unavailable 处理。
fn prepare(
    state: &mut AppState,
    pane: PaneId,
    session_id: &str,
    user_message_id: &str,
    persistence: &Persistence,
) {
    let Some(workspace) = state
        .chat_session_key_for_pane(pane)
        .or_else(|| state.pane_anywhere(pane).and_then(|pane| pane.cwd.clone()))
    else {
        return;
    };
    // 只做元数据预检：这条路径在事件主循环里同步执行，不能把快照 BLOB 读进内存。
    let result = persistence.rewind_availability(&workspace, session_id, user_message_id);
    let (availability, revision, user_text) = match result {
        Ok(value) => value,
        Err(_) => {
            // 目标过期或库不可用：面板保留但禁用文件动作。
            // 预检失败时不要用 0 覆盖冻结 revision：一次瞬时读失败会让 delete_only
            // 在最终提交时恒报“最新轮次已变化”。
            let selected = state
                .pane_anywhere(pane)
                .and_then(|pane| pane.lx.rewind.selected.as_ref())
                .cloned();
            let revision = persistence
                .latest_user_turn(&workspace, session_id)
                .ok()
                .flatten()
                .map(|turn| turn.revision)
                .or_else(|| {
                    selected
                        .as_ref()
                        .map(|target| i64::try_from(target.expected_revision).unwrap_or(0))
                })
                .unwrap_or(0);
            let user_text = selected.map(|target| target.user_text).unwrap_or_default();
            (FileAvailability::Unavailable, revision, user_text)
        }
    };
    crate::app::update::on_rewind_prepared(
        state,
        pane,
        session_id,
        user_message_id,
        &availability,
        revision,
        &user_text,
    );
}

/// 后台执行：独立 SQLite 连接 + 文件 IO，结果回投事件循环。
fn execute(
    sender: &tokio::sync::mpsc::UnboundedSender<AppEvent>,
    persistence: &Persistence,
    request: crate::app::effects::RewindRequest,
) {
    let workspace = request.workspace.clone();
    let operation_id = request.operation_id.clone();
    let Some(path) = persistence.database_path() else {
        let _ = sender.send(AppEvent::RewindFinished {
            pane: request.pane,
            session_id: request.session_id.clone(),
            workspace,
            operation_id,
            outcome: RewindOutcome::Failed {
                message: "rewind requires persistent storage".to_string(),
            },
        });
        return;
    };
    let sender = sender.clone();
    let pane = request.pane;
    let session_id = request.session_id.clone();
    tokio::task::spawn_blocking(move || {
        let outcome = restore::execute(&path, &workspace, &request);
        let _ = sender.send(AppEvent::RewindFinished {
            pane,
            session_id,
            workspace,
            operation_id,
            outcome,
        });
    });
}

/// 执行回执：成功时重载全部绑定该会话的窗格，只有发起 pane 回填原文。
pub fn handle_rewind_finished(
    state: &mut AppState,
    persistence: &Persistence,
    pane: PaneId,
    session_id: &str,
    workspace: &std::path::Path,
    operation_id: &str,
    outcome: RewindOutcome,
) -> bool {
    // 详情 composer 的撤销：按 pending operation id 分流，不触碰主聊天。
    if state.pane_anywhere(pane).is_some_and(|target| {
        target
            .lx
            .detail_rewind
            .executing
            .as_ref()
            .is_some_and(|exec| exec.operation_id == operation_id)
    }) {
        return super::detail_rewind::handle_finished(
            state,
            persistence,
            pane,
            workspace,
            operation_id,
            outcome,
        );
    }
    // 详情撤销的 operation id 命名空间固定；执行态可能已随会话切换清掉，
    // 这时也只能忽略，绝不能落进主聊天处理器。
    if operation_id.starts_with("detail-rewind-") {
        return false;
    }
    // 迟到回执：当前正在执行的是更晚的一次撤销，旧结果不得清掉它的 executing。
    if let Some(executing) = state
        .pane_anywhere(pane)
        .and_then(|target| target.lx.rewind.executing.as_ref())
        && executing.operation_id != operation_id
    {
        return false;
    }
    match outcome {
        RewindOutcome::Failed { message } => {
            crate::app::update::on_rewind_failed(state, pane, &message);
            true
        }
        RewindOutcome::Succeeded {
            action,
            restored_files: _,
            already_restored: _,
            user_text,
            deleted,
            session_deleted,
        } => {
            // 删空会话：先回填发起 pane，再解绑所有绑定该会话的 pane（保留草稿）。
            if session_deleted {
                crate::app::update::on_rewind_succeeded(state, pane, action, &user_text);
                let panes = state.all_pane_ids();
                for target_pane in panes {
                    if state.pane_anywhere(target_pane).is_some_and(|target| {
                        target.lx.chat.session_id.as_deref() == Some(session_id)
                    }) && let Some(target) = state.pane_mut_anywhere(target_pane)
                    {
                        target.lx.chat.reset_session_view();
                        target.lx.clear_agents();
                        target.lx.compact.reset();
                        target.lx.model_switch.reset();
                        target.lx.permission_switch.reset();
                        target.lx.recap.reset();
                        target.lx.clear_rewind();
                        target.lx.reset_session_sandbox_state();
                        crate::app::update::refresh_lx_status(state, target_pane);
                    }
                }
                return true;
            }
            // 先重载数据库真相，再回填；其他绑定 pane 保留各自草稿。
            // workspace 必须用请求里的冻结值：执行期间切换工作区不能让已成功的撤销误报失败。
            let loaded = persistence.load_rewind_view(workspace, session_id);
            // 删除对话后 `agent_session` 的模型列与沙箱状态已回滚到剩余历史：把窗格选择同步回来，
            // 否则状态行仍显示被删轮次的模型与权限。
            let restored_selection = if deleted {
                persistence
                    .with_db(|db| db.chat_session_model(workspace, session_id))
                    .flatten()
                    .map(|snapshot| crate::config::ai::ModelSelection {
                        provider: snapshot.provider,
                        model: snapshot.model,
                        variant: snapshot.variant,
                    })
                    .map(|selection| state.models.normalize_selection(selection))
            } else {
                None
            };
            let restored_sandbox = if deleted {
                persistence.with_db(|db| {
                    let mode = db
                        .session_sandbox_mode(workspace, session_id)?
                        .and_then(|text| crate::config::sandbox::SandboxMode::parse(&text));
                    let rules = db.sandbox_rules(workspace, session_id)?;
                    Ok((mode, rules))
                })
            } else {
                None
            };
            match loaded {
                Ok(view) => {
                    if let Some((mode, _)) = &restored_sandbox {
                        crate::app::update::sync_session_sandbox_mode(
                            state, session_id, *mode, true,
                        );
                    }
                    let panes = state.all_pane_ids();
                    for other in panes {
                        if state.pane_anywhere(other).is_some_and(|target| {
                            target.lx.chat.session_id.as_deref() == Some(session_id)
                        }) {
                            crate::app::update::on_rewind_session_reloaded(
                                state,
                                other,
                                session_id,
                                &view.messages,
                                &view.notices,
                                view.revision,
                            );
                            // 撤销普通轮次不会改检查点，但重载时一并回填更省一次查询。
                            crate::app::update::set_compact_checkpoints(
                                state,
                                other,
                                &view.checkpoints,
                            );
                            crate::app::update::set_model_switches(state, other, &view.switches);
                            crate::app::update::set_permission_switches(
                                state,
                                other,
                                &view.permission_switches,
                            );
                            if let Some((_, rules)) = &restored_sandbox {
                                crate::app::update::set_sandbox_rules(state, other, rules);
                            }
                            crate::app::update::set_message_recaps(state, other, &view.recaps);
                            if let Some(selection) = &restored_selection
                                && let Some(target) = state.pane_mut_anywhere(other)
                            {
                                target.lx.chat.selection = Some(selection.clone());
                            }
                        }
                    }
                    crate::app::update::on_rewind_succeeded(state, pane, action, &user_text);
                }
                Err(error) => {
                    crate::app::update::on_rewind_failed(
                        state,
                        pane,
                        &format!("rewind committed but reload failed: {}", error.code()),
                    );
                }
            }
            true
        }
    }
}
