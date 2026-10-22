//! 启动恢复：按 session guard 结算遗留的撤销 journal 与旧流/工具/进程。
//!
//! 只处理**拿得到跨进程 guard** 的会话；拿不到说明另一个实例仍在运行该会话，跳过。
//! 文件比较使用原始字节，绝不重跑工具；无法证明安全的中间状态标 `recovery_required`。

use std::path::PathBuf;

use crate::db::chat::rewind::{
    FileAvailability, ImageRecord, ImageWrite, OP_FAILED, OP_FILES_RESTORED, OP_PREPARING,
    OP_RECOVERY_REQUIRED, OP_RESTORING, OperationRecord, PendingMutation,
};
use crate::db::{Database, DbError};
use crate::platform::{RawImage, RawKind, decode_path, lock_paths, read_raw};

use super::restore::{apply_image, raw_from_record, verify_identity};

/// 启动入口：扫描并恢复所有可取得 guard 的会话。
pub fn recover_startup(database: &Database) {
    let sessions = match database.sessions_needing_recovery() {
        Ok(sessions) => sessions,
        Err(error) => {
            tracing::warn!(%error, "scanning sessions for recovery failed");
            return;
        }
    };
    if sessions.is_empty() {
        return;
    }
    let mut guards = Vec::new();
    let mut guarded = Vec::new();
    for session in sessions {
        match crate::platform::try_lock_session(&session) {
            Ok(Some(guard)) => {
                guards.push(guard);
                guarded.push(session);
            }
            Ok(None) => {
                tracing::info!(%session, "skipping recovery: session is owned by another instance");
            }
            Err(error) => {
                tracing::warn!(%error, %session, "session guard unavailable during recovery");
            }
        }
    }
    for session in &guarded {
        match recover_journal(database, session) {
            Ok(()) => {}
            Err(error) => tracing::warn!(%error, %session, "rewind journal recovery failed"),
        }
    }
    match database.recover_interrupted_streams(&guarded) {
        Ok(recovered) if recovered > 0 => {
            tracing::info!(recovered, "recovered interrupted chat streams");
        }
        Ok(_) => {}
        Err(error) => tracing::warn!(%error, "recovering interrupted chat streams failed"),
    }
    match database.recover_interrupted_tool_calls(&guarded) {
        Ok(recovered) if recovered > 0 => {
            tracing::info!(recovered, "recovered interrupted tool calls");
        }
        Ok(_) => {}
        Err(error) => tracing::warn!(%error, "recovering interrupted tool calls failed"),
    }
    match database.recover_interrupted_processes(&guarded) {
        Ok(recovered) if recovered > 0 => {
            tracing::info!(recovered, "recovered interrupted processes");
        }
        Ok(_) => {}
        Err(error) => tracing::warn!(%error, "recovering interrupted processes failed"),
    }
    match database.recover_interrupted_user_shells(&guarded) {
        Ok(recovered) if recovered > 0 => {
            tracing::info!(recovered, "recovered interrupted user shells");
        }
        Ok(_) => {}
        Err(error) => tracing::warn!(%error, "recovering interrupted user shells failed"),
    }
    drop(guards);
}

/// 单会话 journal：prepared mutation → 未完成操作 → scope 封口。
fn recover_journal(database: &Database, session: &str) -> Result<(), DbError> {
    settle_prepared_mutations(database, session)?;
    database.seal_recovered_scopes(session)?;
    for operation in database.unfinished_operations(session)? {
        match operation.phase.as_str() {
            // 进入 restoring 之前没有任何文件写入；直接结算为失败。
            OP_PREPARING => {
                database.set_operation_phase(
                    &operation.operation_id,
                    OP_FAILED,
                    Some("startup: no file changes"),
                )?;
            }
            OP_RESTORING | OP_FILES_RESTORED => {
                recover_operation(database, session, &operation)?;
            }
            _ => {}
        }
    }
    Ok(())
}

/// 结算 prepared mutation：当前等于 before/planned 才能推断实际结果；第三种状态保持
/// prepared，让该轮继续是 Incomplete（证据不足，不猜）。
fn settle_prepared_mutations(database: &Database, session: &str) -> Result<(), DbError> {
    let pending = database.pending_mutations(session)?;
    if pending.is_empty() {
        return Ok(());
    }
    let targets: Vec<PathBuf> = pending
        .iter()
        .map(|mutation| decode_path(&mutation.target_path))
        .collect();
    let _locks = match lock_paths(&targets) {
        Ok(Some(locks)) => locks,
        Ok(None) => {
            tracing::warn!(%session, "skipping mutation settlement: path lock timeout");
            return Ok(());
        }
        Err(error) => {
            tracing::warn!(%error, %session, "skipping mutation settlement: lock failed");
            return Ok(());
        }
    };
    for mutation in &pending {
        if let Some(actual) = classify_mutation(mutation) {
            database.settle_mutation(mutation.id, &actual)?;
        }
    }
    Ok(())
}

/// 当前磁盘状态 → 可结算的实际镜像；无法证明时返回 None。
fn classify_mutation(mutation: &PendingMutation) -> Option<ImageWrite> {
    let target = decode_path(&mutation.target_path);
    if let (Some(anchor), Some(canonical)) = (
        mutation.anchor_path.as_deref(),
        mutation.anchor_canonical.as_deref(),
    ) && crate::platform::verify_path_chain(
        &decode_path(anchor),
        &decode_path(canonical),
        &target,
    )
    .is_err()
    {
        return None;
    }
    let before = raw_of(&mutation.before)?;
    let planned = raw_of(&mutation.planned_after)?;
    let current = read_raw(&target, false).ok()?;
    if current.same_content(&before) {
        Some(raw_to_write(&before))
    } else if current.same_content(&planned) {
        Some(raw_to_write(&planned))
    } else {
        None
    }
}

/// 未完成操作：把文件恢复到操作前的状态；任何第三方变化都转 recovery_required。
fn recover_operation(
    database: &Database,
    session: &str,
    operation: &OperationRecord,
) -> Result<(), DbError> {
    let plan = database.recovery_plan(session, &operation.target_user_message_id)?;
    match plan.availability {
        FileAvailability::Unavailable | FileAvailability::Incomplete => {
            database.set_operation_phase(
                &operation.operation_id,
                OP_RECOVERY_REQUIRED,
                Some("startup: snapshot incomplete"),
            )?;
            return Ok(());
        }
        FileAvailability::KnownEmpty | FileAvailability::AlreadyRestored => {
            database.set_operation_phase(
                &operation.operation_id,
                OP_FAILED,
                Some("startup: no file changes to roll back"),
            )?;
            return Ok(());
        }
        FileAvailability::Ready { .. } => {}
    }
    let targets: Vec<PathBuf> = plan
        .paths
        .iter()
        .map(|path| path.target_path.clone())
        .collect();
    let _locks = match lock_paths(&targets) {
        Ok(Some(locks)) => locks,
        Ok(None) | Err(_) => {
            database.set_operation_phase(
                &operation.operation_id,
                OP_RECOVERY_REQUIRED,
                Some("startup: file lock timeout"),
            )?;
            return Ok(());
        }
    };
    // 先全量分类，任一第三种状态就整体放弃（零写入）。
    let mut rollbacks: Vec<(PathBuf, RawImage)> = Vec::new();
    for path in &plan.paths {
        let identity = match verify_identity(path) {
            Ok(identity) => identity,
            Err(_) => {
                return mark_recovery(database, operation, "startup: path identity changed");
            }
        };
        let before = match raw_from_record(path, &path.before, identity.as_ref()) {
            Ok(image) => image,
            Err(_) => return mark_recovery(database, operation, "startup: snapshot unavailable"),
        };
        let after = match raw_from_record(path, &path.expected_after, identity.as_ref()) {
            Ok(image) => image,
            Err(_) => return mark_recovery(database, operation, "startup: snapshot unavailable"),
        };
        let current = match read_raw(&path.target_path, false) {
            Ok(image) => image,
            Err(_) => return mark_recovery(database, operation, "startup: target unreadable"),
        };
        if current.same_content(&after) {
            continue;
        }
        if current.same_content(&before) {
            rollbacks.push((path.target_path.clone(), after));
            continue;
        }
        return mark_recovery(
            database,
            operation,
            "startup: file changed outside the rewind",
        );
    }
    for (target, after) in rollbacks {
        if apply_image(&target, &after).is_err() {
            return mark_recovery(database, operation, "startup: rollback failed");
        }
    }
    database.set_operation_phase(
        &operation.operation_id,
        OP_FAILED,
        Some("startup: rolled back"),
    )?;
    Ok(())
}

fn mark_recovery(
    database: &Database,
    operation: &OperationRecord,
    reason: &str,
) -> Result<(), DbError> {
    database.set_operation_phase(&operation.operation_id, OP_RECOVERY_REQUIRED, Some(reason))
}

/// DB 镜像 → 原始镜像；常规文件必须有字节兜底（生产捕获始终写入）。
fn raw_of(record: &ImageRecord) -> Option<RawImage> {
    match record.kind.as_str() {
        "missing" => Some(RawImage::missing()),
        "symlink" => record
            .link_target
            .as_deref()
            .map(|target| RawImage::symlink(decode_path(target))),
        "regular" => record.bytes.clone().map(|bytes| {
            RawImage::regular(bytes, record.mode.and_then(|mode| u32::try_from(mode).ok()))
        }),
        _ => None,
    }
}

fn raw_to_write(raw: &RawImage) -> ImageWrite {
    ImageWrite {
        kind: match raw.kind {
            RawKind::Missing => "missing",
            RawKind::Regular => "regular",
            RawKind::Symlink => "symlink",
        }
        .to_string(),
        bytes: (raw.kind == RawKind::Regular).then(|| raw.bytes.clone()),
        mode: raw.mode.map(i64::from),
        link_target: raw
            .link_target
            .as_ref()
            .map(|target| crate::platform::encode_path(target)),
        git_oid: None,
        worktree_root: None,
        git_dir: None,
        head_ref: None,
        head_oid: None,
    }
}

/// 便于测试：直接执行单会话恢复。
#[cfg(test)]
pub(super) fn recover_session_for_tests(database: &Database, session: &str) -> Result<(), DbError> {
    recover_journal(database, session)
}

#[cfg(test)]
mod tests;
