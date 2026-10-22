//! 恢复计划派生：同轮路径线性检查、镜像读取与孤儿镜像回收。
//!
//! 从 `rewind.rs` 拆出（单文件行数上限 1000）；只访问 SQLite，不做文件 IO。

use std::path::Path;

use rusqlite::{OptionalExtension, params};

use crate::db::{Database, DbError, Result};

use super::{FileAvailability, ImageRecord, MUTATION_COMMITTED, RestorePath, RestorePlan};

impl Database {
    /// 面板预检：只读 scope / mutation 元数据，**不加载任何文件字节**。
    ///
    /// 双击 Esc 的预检在事件主循环里同步执行；`restore_plan` 为了线性校验会把每份
    /// 镜像的完整 bytes 读进内存（单份上限 64 MiB、单轮 256 MiB），不能走这条路径。
    /// 线性与内容校验留给后台执行期的 `restore_plan`。
    pub fn rewind_availability(
        &self,
        workspace: &Path,
        session_id: &str,
        user_message_id: &str,
    ) -> Result<(FileAvailability, i64, String)> {
        let latest = self
            .latest_user_turn(workspace, session_id)?
            .ok_or(DbError::SessionMissing)?;
        if latest.user_message_id != user_message_id {
            return Err(DbError::RewindTargetChanged);
        }
        let availability = match latest.scope.as_ref() {
            None => FileAvailability::Unavailable,
            Some(scope) if !scope.is_complete() => FileAvailability::Incomplete,
            Some(scope) if scope.files_restored => FileAvailability::AlreadyRestored,
            Some(_) => {
                let (total, pending, distinct): (i64, i64, i64) = self.conn.query_row(
                    r#"SELECT COUNT(*),
                              COALESCE(SUM(CASE WHEN state != ?3 THEN 1 ELSE 0 END), 0),
                              COUNT(DISTINCT target_path)
                       FROM agent_file_mutation
                       WHERE session_id = ?1 AND user_message_id = ?2"#,
                    params![session_id, user_message_id, MUTATION_COMMITTED],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )?;
                if pending > 0 {
                    FileAvailability::Incomplete
                } else if total == 0 {
                    FileAvailability::KnownEmpty
                } else {
                    FileAvailability::Ready {
                        paths: usize::try_from(distinct).unwrap_or(0),
                    }
                }
            }
        };
        Ok((availability, latest.revision, latest.text))
    }

    /// 派生某轮的恢复计划：线性检查、当前期望内容与可用性。
    pub fn restore_plan(
        &self,
        workspace: &Path,
        session_id: &str,
        user_message_id: &str,
    ) -> Result<RestorePlan> {
        let latest = self
            .latest_user_turn(workspace, session_id)?
            .ok_or(DbError::SessionMissing)?;
        let mut plan = RestorePlan {
            session_id: session_id.to_string(),
            user_message_id: user_message_id.to_string(),
            revision: latest.revision,
            availability: FileAvailability::Unavailable,
            paths: Vec::new(),
            user_text: latest.text.clone(),
        };
        if latest.user_message_id != user_message_id {
            return Err(DbError::RewindTargetChanged);
        }
        let Some(scope) = latest.scope else {
            return Ok(plan);
        };
        if !scope.is_complete() {
            plan.availability = FileAvailability::Incomplete;
            return Ok(plan);
        }
        if scope.files_restored {
            plan.availability = FileAvailability::AlreadyRestored;
            return Ok(plan);
        }
        let mut statement = self.conn.prepare(
            r#"SELECT target_path, path, before_image_id,
                      actual_after_image_id, state, worktree_root, git_dir, head_ref, head_oid
               FROM agent_file_mutation
               WHERE session_id = ?1 AND user_message_id = ?2
               ORDER BY id"#,
        )?;
        let rows = statement.query_map(params![session_id, user_message_id], |row| {
            Ok(MutationRow {
                target_path: row.get(0)?,
                path: row.get(1)?,
                before: row.get(2)?,
                actual: row.get(3)?,
                state: row.get(4)?,
                worktree_root: row.get(5)?,
                git_dir: row.get(6)?,
                head_ref: row.get(7)?,
                head_oid: row.get(8)?,
            })
        })?;
        let mut mutations: Vec<MutationRow> = rows.collect::<std::result::Result<_, _>>()?;
        if mutations.iter().any(|row| row.state != MUTATION_COMMITTED) {
            plan.availability = FileAvailability::Incomplete;
            return Ok(plan);
        }
        if mutations.is_empty() {
            plan.availability = FileAvailability::KnownEmpty;
            return Ok(plan);
        }
        // 同一路径：先按 id 排序，再检查连续性与首 before / 末 after。
        mutations.sort_by_key(|row| row.target_path.clone());
        let mut index = 0;
        while index < mutations.len() {
            let start = index;
            while index < mutations.len()
                && mutations[index].target_path == mutations[start].target_path
            {
                index += 1;
            }
            let group = &mutations[start..index];
            let Some(head) = group.last() else {
                return Err(DbError::RewindUnavailable);
            };
            let mut previous_after: Option<ImageRecord> = None;
            for row in group {
                let before = self.image(row.before)?.ok_or(DbError::RewindUnavailable)?;
                if let Some(previous) = previous_after.as_ref()
                    && !previous.same_content(&before)
                {
                    // 中间有其他写入：整轮文件恢复保守拒绝。
                    plan.availability = FileAvailability::Incomplete;
                    return Ok(plan);
                }
                let actual_id = row.actual.ok_or(DbError::RewindUnavailable)?;
                previous_after = Some(self.image(actual_id)?.ok_or(DbError::RewindUnavailable)?);
            }
            let Some(first_row) = group.first() else {
                return Err(DbError::RewindUnavailable);
            };
            let first = self
                .image(first_row.before)?
                .ok_or(DbError::RewindUnavailable)?;
            let last = previous_after.ok_or(DbError::RewindUnavailable)?;
            plan.paths.push(RestorePath {
                path: head.path.clone(),
                target_path: head.target_path.clone(),
                before: first,
                expected_after: last,
                worktree_root: head.worktree_root.clone(),
                git_dir: head.git_dir.clone(),
                head_ref: head.head_ref.clone(),
                head_oid: head.head_oid.clone(),
            });
        }
        plan.availability = FileAvailability::Ready {
            paths: plan.paths.len(),
        };
        Ok(plan)
    }

    /// 读取单份镜像。
    pub fn image(&self, id: i64) -> Result<Option<ImageRecord>> {
        Ok(self
            .conn
            .query_row(
                "SELECT kind, bytes, mode, link_target, git_oid FROM agent_file_image WHERE id = ?1",
                params![id],
                |row| {
                    Ok(ImageRecord {
                        kind: row.get(0)?,
                        bytes: row.get(1)?,
                        mode: row.get(2)?,
                        link_target: row.get(3)?,
                        git_oid: row.get(4)?,
                    })
                },
            )
            .optional()?)
    }

    /// 清理指定会话中没有 mutation 引用的镜像（撤销成功/删除轮次后调用）。
    pub fn prune_orphan_images(&self, session_id: &str) -> Result<usize> {
        let removed = self.conn.execute(
            r#"DELETE FROM agent_file_image
               WHERE session_id = ?1
                 AND id NOT IN (
                     SELECT before_image_id FROM agent_file_mutation
                     UNION SELECT planned_after_image_id FROM agent_file_mutation
                     UNION SELECT actual_after_image_id FROM agent_file_mutation
                        WHERE actual_after_image_id IS NOT NULL
                 )"#,
            params![session_id],
        )?;
        Ok(removed)
    }
}

/// restore plan 的原始 mutation 行。
struct MutationRow {
    target_path: String,
    path: String,
    before: i64,
    actual: Option<i64>,
    state: String,
    worktree_root: Option<String>,
    git_dir: Option<String>,
    head_ref: Option<String>,
    head_oid: Option<String>,
}
