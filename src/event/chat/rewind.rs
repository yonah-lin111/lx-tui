//! 最新轮次撤销的事件层入口：同步预检、后台执行与成功后的视图重载。
//!
//! 撤销文件 IO 与最终 DB 事务在 `spawn_blocking` 的独立连接上执行；事件循环不做
//! 文件读写，也不乐观删除消息——只有 `RewindFinished` 回投后才重载视图并回填。

mod restore;

use crate::app::effects::Effect;
use crate::app::state::{AppState, RewindAction};
use crate::db::chat::FileAvailability;
use crate::db::service::Persistence;
use crate::event::AppEvent;
use crate::layout::PaneId;

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
                Effect::PrepareRewind { .. } | Effect::ExecuteRewind(_)
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
            Effect::PrepareRewind { .. } | Effect::ExecuteRewind(_)
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
            _ => {}
        }
    }
    true
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
    let result = persistence.restore_plan(&workspace, session_id, user_message_id);
    let (availability, revision, user_text) = match result {
        Ok(plan) => (plan.availability, plan.revision, plan.user_text),
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
    let Some(path) = persistence.database_path() else {
        let _ = sender.send(AppEvent::RewindFinished {
            pane: request.pane,
            session_id: request.session_id.clone(),
            workspace,
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
    outcome: RewindOutcome,
) -> bool {
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
            deleted: _,
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
                        target.lx.rewind.clear();
                        if matches!(
                            target.lx.slash,
                            Some(crate::app::state::SlashPanel::Rewind(_))
                        ) {
                            target.lx.slash = None;
                        }
                    }
                }
                return true;
            }
            // 先重载数据库真相，再回填；其他绑定 pane 保留各自草稿。
            // workspace 必须用请求里的冻结值：执行期间切换工作区不能让已成功的撤销误报失败。
            let loaded = persistence.load_rewind_view(workspace, session_id);
            match loaded {
                Ok((messages, notices, revision)) => {
                    let panes = state.all_pane_ids();
                    for other in panes {
                        if state.pane_anywhere(other).is_some_and(|target| {
                            target.lx.chat.session_id.as_deref() == Some(session_id)
                        }) {
                            crate::app::update::on_rewind_session_reloaded(
                                state, other, session_id, &messages, &notices, revision,
                            );
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
