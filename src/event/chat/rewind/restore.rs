//! 撤销的后台执行：预检 → 逐路径恢复 → 补偿 → 最终 DB 事务。
//!
//! 运行在 `spawn_blocking` 的独立 SQLite 连接上；事件循环只接收结果并刷新视图。
//! 文件系统与 SQLite 不能共享一个事务，因此失败时明确区分「已补偿」与
//! `recovery_required`，绝不谎报原子成功。

use std::path::{Path, PathBuf};

use crate::app::effects::RewindRequest;
use crate::app::state::RewindAction;
use crate::db::chat::rewind::{
    FileAvailability, OP_FAILED, OP_FILES_RESTORED, OP_RECOVERY_REQUIRED, OP_RESTORING,
    RestorePath, RestorePlan, RewindCommit,
};
use crate::db::{Database, DbError};
use crate::git::{self, WorktreeIdentity};
use crate::platform::{RawImage, RawKind, read_raw};

use super::RewindOutcome;

/// 在独立连接上执行一次撤销。
pub(super) fn execute(db_path: &Path, workspace: &Path, request: &RewindRequest) -> RewindOutcome {
    let database = match Database::open(db_path) {
        Ok(database) => database,
        Err(error) => return failed(format!("rewind database unavailable: {}", error.code())),
    };
    run(&database, workspace, request)
}

fn run(database: &Database, workspace: &Path, request: &RewindRequest) -> RewindOutcome {
    // 异常退出遗留的未完成操作：不自动重跑，避免重复副作用。读取失败不能当成“没有”。
    match database.unfinished_operation(&request.session_id) {
        Ok(Some(unfinished)) => {
            return failed(format!(
                "an unfinished rewind ({}) blocks this session; recover the files manually before retrying",
                unfinished.phase
            ));
        }
        Ok(None) => {}
        Err(error) => {
            return failed(format!("rewind preflight failed: {}", error.code()));
        }
    }
    // 幂等重放：同 operation id 已成功时只返回原结果，不再动文件/对话。
    match database.operation(&request.operation_id) {
        Ok(Some(operation)) if operation.phase == crate::db::chat::rewind::OP_COMMITTED => {
            return RewindOutcome::Succeeded {
                action: request.action,
                restored_files: 0,
                already_restored: true,
                user_text: operation.user_text,
                deleted: request.action.deletes_conversation(),
                session_deleted: false,
            };
        }
        Ok(Some(operation)) => {
            return failed(format!(
                "a previous rewind is unfinished ({}); recover the files manually before retrying",
                operation.phase
            ));
        }
        Ok(None) => {}
        Err(error) => {
            return failed(format!("rewind preflight failed: {}", error.code()));
        }
    }
    // 跨进程 session guard 覆盖“预检 → 文件恢复 → 最终提交 → 补偿”全程；仅删对话同样持有。
    let _session_guard = match crate::platform::try_lock_session(&request.session_id) {
        Ok(Some(guard)) => guard,
        Ok(None) => {
            return failed(
                "another lx instance is running this session; nothing was rewritten".to_string(),
            );
        }
        Err(error) => {
            return failed(format!("session lock failed: {error}"));
        }
    };
    let plan = match database.restore_plan(
        workspace,
        &request.session_id,
        &request.target_user_message_id,
    ) {
        Ok(plan) => plan,
        Err(DbError::RewindTargetChanged) | Err(DbError::SessionMissing) => {
            return failed("the latest turn changed; nothing was rewritten".to_string());
        }
        Err(DbError::RewindUnavailable) => return failed_unavailable(),
        Err(error) => return failed(format!("rewind preflight failed: {}", error.code())),
    };
    if plan.revision != i64::try_from(request.expected_revision).unwrap_or(-1) {
        return failed("the session changed; nothing was rewritten".to_string());
    }
    let already_restored = matches!(plan.availability, FileAvailability::AlreadyRestored);
    if request.action.reverts_files() {
        match plan.availability {
            // 仅恢复文件是幂等无操作：不建 operation、不动文件、不再插新 tip。
            FileAvailability::AlreadyRestored if request.action == RewindAction::RevertOnly => {
                return RewindOutcome::Succeeded {
                    action: request.action,
                    restored_files: 0,
                    already_restored: true,
                    user_text: plan.user_text,
                    deleted: false,
                    session_deleted: false,
                };
            }
            // 组合操作此时只删对话；不覆盖别人后来的修改。
            FileAvailability::AlreadyRestored | FileAvailability::KnownEmpty => {}
            FileAvailability::Incomplete => {
                return failed(
                    "file snapshot is incomplete (interleaved writes); nothing was rewritten"
                        .to_string(),
                );
            }
            FileAvailability::Unavailable => return failed_unavailable(),
            FileAvailability::Ready { .. } => {}
        }
    }
    // 操作记录先于任何文件写入：崩溃后据此判定，且最终事务必须能找到它。
    if let Err(error) = database.begin_operation(
        workspace,
        &request.operation_id,
        &request.session_id,
        &request.target_user_message_id,
        i64::try_from(request.expected_revision).unwrap_or(-1),
        request.action.as_str(),
        &request.user_text,
    ) {
        return failed(format!("rewind operation record failed: {}", error.code()));
    }
    let mut restored: Option<Restored> = None;
    if request.action.reverts_files() && matches!(plan.availability, FileAvailability::Ready { .. })
    {
        match restore_files(database, request, &plan) {
            Ok(finished) => restored = Some(finished),
            Err(failure) => {
                // 预检/加锁失败不改文件，但 operation 行已经存在；不结算会让该会话
                // 永久被 unfinished_operation 挡住。写失败已由 restore_files 标成
                // failed/recovery_required，这里只处理仍停在 preparing/restoring 的情况。
                if let Ok(Some(operation)) = database.operation(&request.operation_id)
                    && matches!(
                        operation.phase.as_str(),
                        crate::db::chat::rewind::OP_PREPARING | OP_RESTORING
                    )
                {
                    let _ = database.set_operation_phase(
                        &request.operation_id,
                        OP_FAILED,
                        Some("restore preflight failed"),
                    );
                }
                return failure;
            }
        }
    }
    let restored_files = restored.as_ref().map_or(0, |finished| finished.count);
    let commit = RewindCommit {
        operation_id: request.operation_id.clone(),
        session_id: request.session_id.clone(),
        target_user_message_id: request.target_user_message_id.clone(),
        expected_revision: i64::try_from(request.expected_revision).unwrap_or(-1),
        kind: request.action.as_str().to_string(),
        delete_conversation: request.action.deletes_conversation(),
        revert_files: request.action.reverts_files() && !already_restored,
        restored_file_count: i64::try_from(restored_files).unwrap_or(0),
        files_already_restored: already_restored,
    };
    match database.commit_rewind(workspace, &commit) {
        Ok(committed) => {
            let _ = database.set_operation_phase(
                &request.operation_id,
                crate::db::chat::rewind::OP_COMMITTED,
                None,
            );
            let _ = database.prune_orphan_images(&request.session_id);
            RewindOutcome::Succeeded {
                action: request.action,
                restored_files,
                already_restored,
                user_text: committed.user_text,
                deleted: request.action.deletes_conversation(),
                session_deleted: committed.session_deleted,
            }
        }
        Err(error) => {
            // 文件已回退而最终事务失败：尝试补偿回执行前状态。
            let compensated = restored.as_ref().map(|finished| {
                compensate(&plan.paths, &finished.restored_to, &finished.preimages)
            });
            let _ = database.set_operation_phase(
                &request.operation_id,
                if compensated == Some(false) {
                    OP_RECOVERY_REQUIRED
                } else {
                    OP_FAILED
                },
                Some(error.code()),
            );
            failed(match compensated {
                Some(true) => "conversation commit failed; file changes were rolled back".to_string(),
                Some(false) => {
                    "conversation commit failed and automatic rollback failed; manual recovery required"
                        .to_string()
                }
                None => format!("conversation commit failed: {}", error.code()),
            })
        }
    }
}

/// 已恢复文件的执行前镜像与“本操作写入的镜像”（补偿判据）。
struct Restored {
    count: usize,
    /// 本次实际写到磁盘上的内容；补偿前必须与当前内容一致。
    restored_to: Vec<RawImage>,
    /// 写入前的磁盘内容；补偿写回它。
    preimages: Vec<RawImage>,
}

/// 恢复文件：路径锁 → 全量预检 → 写入 → 记录进度（session guard 由 `run` 持有）。
fn restore_files(
    database: &Database,
    request: &RewindRequest,
    plan: &RestorePlan,
) -> Result<Restored, RewindOutcome> {
    let targets: Vec<PathBuf> = plan
        .paths
        .iter()
        .map(|path| PathBuf::from(&path.target_path))
        .collect();
    let _paths = match crate::platform::lock_paths(&targets) {
        Ok(Some(guard)) => guard,
        Ok(None) => {
            return Err(failed(
                "timed out waiting for file locks; nothing was rewritten".to_string(),
            ));
        }
        Err(error) => return Err(failed(format!("file lock failed: {error}"))),
    };
    // 全量预检：任一冲突零写入。
    let mut preimages = Vec::with_capacity(plan.paths.len());
    let mut befores = Vec::with_capacity(plan.paths.len());
    for path in &plan.paths {
        let identity = verify_identity(path)?;
        let current = read_raw(Path::new(&path.target_path), false)
            .map_err(|error| failed(format!("cannot read {}: {error}", path.target_path)))?;
        let expected = raw_from_record(path, &path.expected_after, identity.as_ref())?;
        if !current.same_content(&expected) {
            return Err(failed(format!(
                "{} changed after this turn; nothing was rewritten",
                path.target_path
            )));
        }
        let before = raw_from_record(path, &path.before, identity.as_ref())?;
        preimages.push(current);
        befores.push(before);
    }
    let _ = database.set_operation_phase(&request.operation_id, OP_RESTORING, None);
    let mut written = 0usize;
    for (index, path) in plan.paths.iter().enumerate() {
        // 写前复检：预检与写入之间，不遵守 advisory lock 的外部程序仍可能改文件。
        let unchanged = read_raw(Path::new(&path.target_path), false)
            .map(|current| current.same_content(&preimages[index]))
            .unwrap_or(false);
        let failure = if unchanged {
            apply_image(Path::new(&path.target_path), &befores[index])
                .err()
                .map(|error| error.to_string())
        } else {
            Some("file changed while restoring".to_string())
        };
        if let Some(reason) = failure {
            let compensated = compensate(
                &plan.paths[..written],
                &befores[..written],
                &preimages[..written],
            );
            let _ = database.set_operation_phase(
                &request.operation_id,
                if compensated {
                    OP_FAILED
                } else {
                    OP_RECOVERY_REQUIRED
                },
                Some("restore failed"),
            );
            return Err(failed(if compensated {
                format!(
                    "failed to restore {}: {reason}; changes were rolled back",
                    path.target_path
                )
            } else {
                format!(
                    "failed to restore {}: {reason}; automatic rollback also failed, manual recovery required",
                    path.target_path
                )
            }));
        }
        written += 1;
    }
    let _ = database.set_operation_phase(&request.operation_id, OP_FILES_RESTORED, None);
    Ok(Restored {
        count: written,
        restored_to: befores,
        preimages,
    })
}

/// 逆序补偿已经写入的路径：先确认当前仍是本操作刚写的结果，再恢复执行前镜像。
///
/// `restored_to` 是本操作写入的内容，`preimages` 是写入前的磁盘内容。拿当前内容跟
/// `preimages` 比较是错的：恢复成功后磁盘上恰是 `restored_to`，那样永远判定为冲突，
/// 一个字节也不会回滚。
fn compensate(paths: &[RestorePath], restored_to: &[RawImage], preimages: &[RawImage]) -> bool {
    if paths.len() != restored_to.len() || paths.len() != preimages.len() {
        return false;
    }
    for ((path, written), preimage) in paths.iter().zip(restored_to).zip(preimages).rev() {
        let target = Path::new(&path.target_path);
        match read_raw(target, false) {
            Ok(current) if current.same_content(written) => {
                if apply_image(target, preimage).is_err() {
                    return false;
                }
            }
            // 补偿前发现被外部修改：停止，避免覆盖别人的写入。
            _ => return false,
        }
    }
    true
}

/// 写入单个镜像。
fn apply_image(target: &Path, image: &RawImage) -> std::io::Result<()> {
    match image.kind {
        RawKind::Missing => crate::platform::remove_entry(target),
        RawKind::Regular => crate::platform::write_raw_regular(target, image),
        RawKind::Symlink => {
            let link = image.link_target.as_deref().ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::InvalidData, "symlink target missing")
            })?;
            crate::platform::write_raw_symlink(target, link)
        }
    }
}

/// 校验实际目标仍属于同一 worktree / HEAD 身份；返回当前身份供 blob 读取。
fn verify_identity(path: &RestorePath) -> Result<Option<WorktreeIdentity>, RewindOutcome> {
    let target = Path::new(&path.target_path);
    let current = git::identify(target);
    match (&path.worktree_root, &current) {
        (None, None) => Ok(None),
        (Some(root), Some(identity)) => {
            let same = identity.root.to_string_lossy() == root.as_str()
                && path
                    .git_dir
                    .as_deref()
                    .is_some_and(|git_dir| identity.git_dir.to_string_lossy() == git_dir)
                && path.head_ref == identity.head_ref
                && path.head_oid == identity.head_oid;
            if same {
                Ok(Some(identity.clone()))
            } else {
                Err(failed(format!(
                    "{} is no longer in the recorded worktree/HEAD; nothing was rewritten",
                    path.target_path
                )))
            }
        }
        _ => Err(failed(format!(
            "{} changed its Git identity; nothing was rewritten",
            path.target_path
        ))),
    }
}

/// DB 镜像 → 原始镜像；字节缺失时回退项目对象库。
fn raw_from_record(
    _path: &RestorePath,
    record: &crate::db::chat::ImageRecord,
    identity: Option<&WorktreeIdentity>,
) -> Result<RawImage, RewindOutcome> {
    match record.kind.as_str() {
        "missing" => Ok(RawImage::missing()),
        "symlink" => Ok(RawImage::symlink(PathBuf::from(
            record
                .link_target
                .as_deref()
                .ok_or_else(|| failed("snapshot symlink target missing".to_string()))?,
        ))),
        "regular" => {
            let bytes = if let Some(bytes) = &record.bytes {
                bytes.clone()
            } else if let (Some(oid), Some(identity)) = (record.git_oid.as_deref(), identity) {
                git::read_blob(identity, oid).ok_or_else(|| {
                    failed("Git blob is gone and no byte backup exists".to_string())
                })?
            } else {
                return Err(failed("file snapshot bytes unavailable".to_string()));
            };
            Ok(RawImage::regular(
                bytes,
                record.mode.and_then(|mode| u32::try_from(mode).ok()),
            ))
        }
        _ => Err(failed("unknown snapshot kind".to_string())),
    }
}

fn failed_unavailable() -> RewindOutcome {
    failed("file snapshot unavailable for this turn".to_string())
}

fn failed(message: String) -> RewindOutcome {
    RewindOutcome::Failed { message }
}

#[cfg(test)]
mod tests;
