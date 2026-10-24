//! 文件 mutation 的事件层桥：把工具的同步 recorder 请求变成持久化 ACK。
//!
//! worker 在路径锁内阻塞等待回执；事件层只做 DB 写入（Git blob 已在 worker 线程完成），
//! **绝不重新获取该 worker 持有的路径锁**，也不等待它的 FinishTool。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::sync::{mpsc, oneshot};

use crate::chat::tools::{
    MutationError, MutationRecorder, MutationRecorderFactory, MutationTicket, ObservedMutation,
    PathMutation, PreparedMutation,
};
use crate::db::chat::{ImageWrite, ObservedWrite, PathMutationWrite};
use crate::db::service::Persistence;
use crate::event::AppEvent;
use crate::git::{WorktreeIdentity, identify, write_blob};
use crate::platform::{RawImage, RawKind};

/// 事件层收到的 mutation 持久化请求。
#[derive(Debug)]
pub enum MutationHostEvent {
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

/// 每轮构造一次的 recorder 工厂（绑定 session + 稳定 user message id）。
#[derive(Debug, Clone)]
pub struct HostRecorderFactory {
    sender: mpsc::UnboundedSender<AppEvent>,
    workspace: PathBuf,
    session_id: String,
    user_message_id: String,
}

impl HostRecorderFactory {
    pub fn new(
        sender: mpsc::UnboundedSender<AppEvent>,
        workspace: PathBuf,
        session_id: String,
        user_message_id: String,
    ) -> Self {
        Self {
            sender,
            workspace,
            session_id,
            user_message_id,
        }
    }
}

impl MutationRecorderFactory for HostRecorderFactory {
    fn for_call(&self, call_id: &str) -> Arc<dyn MutationRecorder> {
        Arc::new(HostRecorder {
            sender: self.sender.clone(),
            workspace: self.workspace.clone(),
            session_id: self.session_id.clone(),
            user_message_id: self.user_message_id.clone(),
            tool_call_id: call_id.to_string(),
        })
    }
}

/// 生产 recorder：在阻塞 worker 线程里准备 Git 对象并等待事件层 DB 回执。
#[derive(Debug)]
struct HostRecorder {
    sender: mpsc::UnboundedSender<AppEvent>,
    workspace: PathBuf,
    session_id: String,
    user_message_id: String,
    tool_call_id: String,
}

impl MutationRecorder for HostRecorder {
    fn prepare(&self, mutation: &PreparedMutation) -> Result<MutationTicket, MutationError> {
        let changes = mutation
            .paths
            .iter()
            .map(change_write)
            .collect::<Vec<PathMutationWrite>>();
        let (reply, receiver) = oneshot::channel();
        let event = MutationHostEvent::Prepare {
            workspace: self.workspace.clone(),
            session_id: self.session_id.clone(),
            user_message_id: self.user_message_id.clone(),
            tool_call_id: self.tool_call_id.clone(),
            changes,
            reply,
        };
        if self
            .sender
            .send(AppEvent::AgentHost(super::AgentHostEvent::Mutation(event)))
            .is_err()
        {
            return Err(MutationError::new(
                "cancelled",
                "the event loop is gone; file mutation was not started",
            ));
        }
        receiver.blocking_recv().map_err(|_| {
            MutationError::new("cancelled", "file snapshot acknowledgement was lost")
        })?
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
        let event = MutationHostEvent::Commit {
            workspace: self.workspace.clone(),
            session_id: self.session_id.clone(),
            ticket,
            observed,
            reply,
        };
        if self
            .sender
            .send(AppEvent::AgentHost(super::AgentHostEvent::Mutation(event)))
            .is_err()
        {
            return Err(MutationError::new(
                "cancelled",
                "the event loop is gone; file mutation result was not committed",
            ));
        }
        receiver.blocking_recv().map_err(|_| {
            MutationError::new("cancelled", "file mutation acknowledgement was lost")
        })?
    }
}

/// 事件层处理：只做 DB 写入并回执。
pub(crate) fn handle_mutation(persistence: &Persistence, event: MutationHostEvent) -> bool {
    match event {
        MutationHostEvent::Prepare {
            workspace,
            session_id,
            user_message_id,
            tool_call_id,
            changes,
            reply,
        } => {
            let result = persistence
                .prepare_mutations(
                    &workspace,
                    &session_id,
                    &user_message_id,
                    &tool_call_id,
                    &changes,
                )
                .map(|ticket| MutationTicket { ids: ticket.ids })
                .map_err(|error| {
                    // 版本核对失败必须回给模型一个可重试的语义（`stale_read`），
                    // 而不是笼统的持久化错误：模型据此重新读取后重试。
                    let code = match error {
                        crate::db::DbError::RewindLimit => "limit",
                        crate::db::DbError::StaleRead => "stale_read",
                        _ => "persist",
                    };
                    MutationError::new(
                        code,
                        format!("failed to persist file snapshot: {}", error.code()),
                    )
                });
            let _ = reply.send(result);
        }
        MutationHostEvent::Commit {
            workspace,
            session_id,
            ticket,
            observed,
            reply,
        } => {
            let result = persistence
                .commit_mutations(
                    &workspace,
                    &session_id,
                    &crate::db::chat::PreparedTicket { ids: ticket.ids },
                    &observed,
                )
                .map_err(|error| {
                    MutationError::new(
                        "persist",
                        format!("failed to commit file mutation: {}", error.code()),
                    )
                });
            let _ = reply.send(result);
        }
    }
    false
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
        worktree_root: identity.map(|identity| path_string(&identity.root)),
        git_dir: identity.map(|identity| path_string(&identity.git_dir)),
        head_ref: identity.and_then(|identity| identity.head_ref.clone()),
        head_oid: identity.and_then(|identity| identity.head_oid.clone()),
    }
}

fn path_string(path: &Path) -> String {
    crate::platform::encode_path(path)
}
