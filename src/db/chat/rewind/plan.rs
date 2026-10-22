//! 恢复计划派生：同轮路径线性检查、镜像读取与孤儿镜像回收。
//!
//! 从 `rewind.rs` 拆出（单文件行数上限 1000）；只访问 SQLite，不做文件 IO。

use std::path::Path;

use rusqlite::{OptionalExtension, params};

use crate::db::{Database, DbError, Result};
use crate::platform::decode_path;

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
        if latest.user_message_id != user_message_id {
            return Err(DbError::RewindTargetChanged);
        }
        let (availability, paths) =
            self.plan_for_turn(session_id, user_message_id, latest.scope.as_ref())?;
        Ok(RestorePlan {
            session_id: session_id.to_string(),
            user_message_id: user_message_id.to_string(),
            revision: latest.revision,
            availability,
            paths,
            user_text: latest.text,
        })
    }

    /// 启动恢复用：不要求目标仍是最新轮次，只按稳定 user id 派生路径计划。
    pub fn recovery_plan(&self, session_id: &str, user_message_id: &str) -> Result<RestorePlan> {
        let scope = self.turn_scope_for_session(session_id, user_message_id)?;
        let (availability, paths) =
            self.plan_for_turn(session_id, user_message_id, scope.as_ref())?;
        Ok(RestorePlan {
            session_id: session_id.to_string(),
            user_message_id: user_message_id.to_string(),
            revision: 0,
            availability,
            paths,
            user_text: String::new(),
        })
    }

    /// 恢复计划的核心：scope 状态 + 同路径连续性与首 before / 末 after。
    fn plan_for_turn(
        &self,
        session_id: &str,
        user_message_id: &str,
        scope: Option<&super::TurnScopeRecord>,
    ) -> Result<(FileAvailability, Vec<RestorePath>)> {
        let Some(scope) = scope else {
            return Ok((FileAvailability::Unavailable, Vec::new()));
        };
        if !scope.is_complete() {
            return Ok((FileAvailability::Incomplete, Vec::new()));
        }
        if scope.files_restored {
            return Ok((FileAvailability::AlreadyRestored, Vec::new()));
        }
        let mut statement = self.conn.prepare(
            r#"SELECT target_path, path, anchor_path, anchor_canonical, before_image_id,
                      actual_after_image_id, state, worktree_root, git_dir, head_ref, head_oid
               FROM agent_file_mutation
               WHERE session_id = ?1 AND user_message_id = ?2
               ORDER BY id"#,
        )?;
        let rows = statement.query_map(params![session_id, user_message_id], |row| {
            Ok(MutationRow {
                target_path: row.get(0)?,
                path: row.get(1)?,
                anchor_path: row.get(2)?,
                anchor_canonical: row.get(3)?,
                before: row.get(4)?,
                actual: row.get(5)?,
                state: row.get(6)?,
                worktree_root: row.get(7)?,
                git_dir: row.get(8)?,
                head_ref: row.get(9)?,
                head_oid: row.get(10)?,
            })
        })?;
        let mut mutations: Vec<MutationRow> = rows.collect::<std::result::Result<_, _>>()?;
        if mutations.iter().any(|row| row.state != MUTATION_COMMITTED) {
            return Ok((FileAvailability::Incomplete, Vec::new()));
        }
        if mutations.is_empty() {
            return Ok((FileAvailability::KnownEmpty, Vec::new()));
        }
        let mut paths = Vec::new();
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
                    return Ok((FileAvailability::Incomplete, Vec::new()));
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
            paths.push(RestorePath {
                path: decode_path(&head.path),
                target_path: decode_path(&head.target_path),
                anchor_path: head.anchor_path.as_deref().map(decode_path),
                anchor_canonical: head.anchor_canonical.as_deref().map(decode_path),
                before: first,
                expected_after: last,
                worktree_root: head.worktree_root.clone(),
                git_dir: head.git_dir.clone(),
                head_ref: head.head_ref.clone(),
                head_oid: head.head_oid.clone(),
            });
        }
        let availability = FileAvailability::Ready { paths: paths.len() };
        Ok((availability, paths))
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
    anchor_path: Option<String>,
    anchor_canonical: Option<String>,
    before: i64,
    actual: Option<i64>,
    state: String,
    worktree_root: Option<String>,
    git_dir: Option<String>,
    head_ref: Option<String>,
    head_oid: Option<String>,
}
