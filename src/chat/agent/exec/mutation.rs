//! `exec` 文件 mutation 持久化桥：工具 worker 线程 → 无头事件循环 → SQLite。
//!
//! recorder 在路径锁内**同步等待**事件循环回执；事件循环消失时按取消收尾，
//! 避免 worker 永久阻塞。与 [`super::runner`] 同边界：不导入 `app` / `ui` / `tui` / `event`。

use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::{mpsc, oneshot};

use crate::chat::tools::{
    MutationError, MutationRecorder, MutationRecorderFactory, MutationTicket, ObservedMutation,
    PathMutation, PreparedMutation,
};
use crate::db::Database;
use crate::db::chat::{ImageWrite, ObservedWrite, PathMutationWrite};
use crate::git::{WorktreeIdentity, identify, write_blob};
use crate::platform::{RawImage, RawKind};

use super::runner::ExecLoopEvent;

/// 文件 mutation 的持久化请求（worker 在路径锁内同步等待回执）。
pub(crate) enum ExecMutationEvent {
    Prepare {
        workspace: PathBuf,
        session_id: String,
        user_message_id: String,
        tool_call_id: String,
        changes: Vec<PathMutationWrite>,
        reply: oneshot::Sender<Result<MutationTicket, MutationError>>,
    },
    Commit {
        workspace: PathBuf,
        session_id: String,
        ticket: MutationTicket,
        observed: Vec<ObservedWrite>,
        reply: oneshot::Sender<Result<(), MutationError>>,
    },
}

/// 事件层 mutation 请求的直连 DB 处理；回执丢失由 recorder 按取消收尾。
pub(crate) fn handle_mutation_event(db: &Database, event: ExecMutationEvent) {
    match event {
        ExecMutationEvent::Prepare {
            workspace,
            session_id,
            user_message_id,
            tool_call_id,
            changes,
            reply,
        } => {
            let result = db
                .prepare_mutations(
                    &workspace,
                    &session_id,
                    &user_message_id,
                    &tool_call_id,
                    &changes,
                )
                .map(|ticket| MutationTicket { ids: ticket.ids })
                .map_err(mutation_error);
            let _ = reply.send(result);
        }
        ExecMutationEvent::Commit {
            workspace,
            session_id,
            ticket,
            observed,
            reply,
        } => {
            let result = db
                .commit_mutations(
                    &workspace,
                    &session_id,
                    &crate::db::chat::PreparedTicket { ids: ticket.ids },
                    &observed,
                )
                .map_err(mutation_error);
            let _ = reply.send(result);
        }
    }
}

fn mutation_error(error: crate::db::DbError) -> MutationError {
    let code = match error {
        crate::db::DbError::RewindLimit => "limit",
        crate::db::DbError::StaleRead => "stale_read",
        _ => "persist",
    };
    MutationError::new(
        code,
        format!("failed to persist file snapshot: {}", error.code()),
    )
}

/// 每轮构造一次的 recorder；每次 `for_call` 克隆出按 `call_id` 绑定的实例。
///
/// worker 线程在路径锁内同步等待事件循环的 DB 回执。
#[derive(Clone)]
struct ExecMutationRecorder {
    sender: mpsc::UnboundedSender<ExecLoopEvent>,
    workspace: PathBuf,
    session_id: String,
    user_message_id: String,
    tool_call_id: String,
}

impl MutationRecorderFactory for ExecMutationRecorder {
    fn for_call(&self, call_id: &str) -> Arc<dyn MutationRecorder> {
        Arc::new(Self {
            tool_call_id: call_id.to_string(),
            ..self.clone()
        })
    }
}

impl MutationRecorder for ExecMutationRecorder {
    fn prepare(&self, mutation: &PreparedMutation) -> Result<MutationTicket, MutationError> {
        let changes = mutation.paths.iter().map(change_write).collect();
        let (reply, receiver) = oneshot::channel();
        let event = ExecMutationEvent::Prepare {
            workspace: self.workspace.clone(),
            session_id: self.session_id.clone(),
            user_message_id: self.user_message_id.clone(),
            tool_call_id: self.tool_call_id.clone(),
            changes,
            reply,
        };
        if self.sender.send(ExecLoopEvent::Mutation(event)).is_err() {
            return Err(MutationError::new(
                "cancelled",
                "the exec event loop is gone; file mutation was not started",
            ));
        }
        receiver
            .blocking_recv()
            .map_err(|_| MutationError::new("cancelled", "snapshot acknowledgement lost"))?
    }

    fn commit(
        &self,
        ticket: MutationTicket,
        observed: &ObservedMutation,
    ) -> Result<(), MutationError> {
        let observed = observed
            .paths
            .iter()
            .map(|item| ObservedWrite {
                actual_after: image_write(&item.actual_after, None, false),
                success: item.success,
            })
            .collect();
        let (reply, receiver) = oneshot::channel();
        let event = ExecMutationEvent::Commit {
            workspace: self.workspace.clone(),
            session_id: self.session_id.clone(),
            ticket,
            observed,
            reply,
        };
        if self.sender.send(ExecLoopEvent::Mutation(event)).is_err() {
            return Err(MutationError::new(
                "cancelled",
                "the exec event loop is gone; file mutation was not committed",
            ));
        }
        receiver
            .blocking_recv()
            .map_err(|_| MutationError::new("cancelled", "mutation acknowledgement lost"))?
    }
}

/// 注入生产 recorder 工厂（每轮一次，按 call_id 绑定所有权）。
pub(crate) fn new_mutation_recorder_factory(
    sender: mpsc::UnboundedSender<ExecLoopEvent>,
    workspace: PathBuf,
    session_id: String,
    user_message_id: String,
) -> Arc<dyn MutationRecorderFactory> {
    Arc::new(ExecMutationRecorder {
        sender,
        workspace,
        session_id,
        user_message_id,
        tool_call_id: String::new(),
    })
}

/// 工具侧 mutation → DB 写入参数；Git blob 在调用线程（worker）完成。
fn change_write(change: &PathMutation) -> PathMutationWrite {
    let identity = identify(&change.target);
    let anchor = crate::platform::path_anchor(&change.target);
    PathMutationWrite {
        operation: change.operation.as_str().to_string(),
        path: crate::platform::encode_path(&change.path),
        target_path: crate::platform::encode_path(&change.target),
        anchor_path: anchor
            .as_ref()
            .map(|(lexical, _)| crate::platform::encode_path(lexical)),
        anchor_canonical: anchor
            .as_ref()
            .map(|(_, canonical)| crate::platform::encode_path(canonical)),
        before: image_write(&change.before, identity.as_ref(), true),
        planned_after: image_write(&change.planned_after, identity.as_ref(), true),
        expected_version: change.expected_version.clone(),
    }
}

/// 原始镜像 → DB 镜像；`with_git` 为真时写入项目对象库（失败降级为纯字节）。
fn image_write(
    image: &RawImage,
    identity: Option<&WorktreeIdentity>,
    with_git: bool,
) -> ImageWrite {
    let git_oid = if with_git && image.kind == RawKind::Regular {
        identity.and_then(|identity| write_blob(identity, &image.bytes))
    } else {
        None
    };
    ImageWrite {
        kind: match image.kind {
            RawKind::Missing => "missing".to_string(),
            RawKind::Regular => "regular".to_string(),
            RawKind::Symlink => "symlink".to_string(),
        },
        bytes: (image.kind == RawKind::Regular).then(|| image.bytes.clone()),
        mode: image.mode.map(i64::from),
        link_target: image
            .link_target
            .as_ref()
            .map(|target| crate::platform::encode_path(target)),
        git_oid,
        worktree_root: identity.map(|identity| crate::platform::encode_path(&identity.root)),
        git_dir: identity.map(|identity| crate::platform::encode_path(&identity.git_dir)),
        head_ref: identity.and_then(|identity| identity.head_ref.clone()),
        head_oid: identity.and_then(|identity| identity.head_oid.clone()),
    }
}
