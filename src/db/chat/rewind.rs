//! 最新轮次撤销的持久化：轮次文件 scope、文件镜像、mutation 日志、操作日志与提示。
//!
//! 分层约束：本模块只接触 rusqlite，不读取磁盘、不执行 Git；文件字节由事件层交入，
//! 恢复时的 IO 由事件层完成。所有写操作以 `workspace + session + 稳定 user message id`
//! 三条件校验，`operation_id` 唯一以支持幂等重放。

use std::path::Path;

use rusqlite::{Connection, OptionalExtension, params};

use super::{Database, DbError, Result, new_external_id, now_ms};

/// 单份文件镜像上限（64 MiB）。
pub const MAX_IMAGE_BYTES: u64 = 64 * 1024 * 1024;
/// 单轮新增快照数据上限（256 MiB）。
pub const MAX_TURN_SNAPSHOT_BYTES: u64 = 256 * 1024 * 1024;

/// 轮次文件 scope 状态。
pub const SCOPE_CAPTURING: &str = "capturing";
pub const SCOPE_COMPLETE: &str = "complete";

/// 文件镜像类型。
pub const IMAGE_MISSING: &str = "missing";
pub const IMAGE_REGULAR: &str = "regular";
pub const IMAGE_SYMLINK: &str = "symlink";

/// mutation 状态。
pub const MUTATION_PREPARED: &str = "prepared";
pub const MUTATION_COMMITTED: &str = "committed";

/// 操作阶段。
pub const OP_PREPARING: &str = "preparing";
pub const OP_RESTORING: &str = "restoring";
pub const OP_FILES_RESTORED: &str = "files_restored";
pub const OP_COMMITTED: &str = "committed";
pub const OP_FAILED: &str = "failed";
pub const OP_RECOVERY_REQUIRED: &str = "recovery_required";

/// 一条持久化撤销提示（独立于 messages，不进入模型上下文）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoticeRecord {
    pub id: String,
    pub sequence: i64,
    pub kind: String,
    pub target_user_message_id: String,
    pub restored_file_count: i64,
    pub files_already_restored: bool,
    pub created_at_ms: i64,
}

/// 一轮的捕获范围；缺行表示旧轮次（快照不可用）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnScopeRecord {
    pub user_message_id: String,
    pub state: String,
    pub capture_version: i64,
    pub snapshot_bytes: i64,
    pub files_restored: bool,
}

impl TurnScopeRecord {
    /// 是否为完整捕获（只有它才允许文件动作）。
    pub fn is_complete(&self) -> bool {
        self.state == SCOPE_COMPLETE
    }
}

/// 会话最新用户轮次：目标选择与预检的只读快照。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LatestTurn {
    pub user_message_id: String,
    pub sequence: i64,
    pub turn_index: i64,
    pub text: String,
    pub revision: i64,
    pub scope: Option<TurnScopeRecord>,
}

/// 一个可恢复路径的镜像与身份。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageRecord {
    pub kind: String,
    pub bytes: Option<Vec<u8>>,
    pub mode: Option<i64>,
    pub link_target: Option<String>,
    pub git_oid: Option<String>,
}

impl ImageRecord {
    /// 内容同一性（存在性、类型、字节、链接目标、权限）。
    ///
    /// 只用于“当前文件是否仍是该轮最后一次写入的结果”和同轮线性检查；
    /// 不比较 Git OID（同一字节可能有不同 OID 形式，且字节才是持久事实）。
    pub fn same_content(&self, other: &Self) -> bool {
        self.kind == other.kind
            && self.link_target == other.link_target
            && self.mode == other.mode
            && self.bytes.as_deref() == other.bytes.as_deref()
    }
}

/// 写入一份镜像（before / planned-after / actual-after 共用）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageWrite {
    pub kind: String,
    pub bytes: Option<Vec<u8>>,
    pub mode: Option<i64>,
    pub link_target: Option<String>,
    pub git_oid: Option<String>,
    /// 实际目标所属 worktree 根（无 Git 时为 None）。
    pub worktree_root: Option<String>,
    pub git_dir: Option<String>,
    pub head_ref: Option<String>,
    pub head_oid: Option<String>,
}

impl ImageWrite {
    /// 该镜像占用的快照字节数。
    pub fn size(&self) -> u64 {
        self.bytes.as_ref().map_or(0, |bytes| bytes.len() as u64)
    }
}

/// 一次路径变更。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathMutationWrite {
    /// `create` / `update` / `delete` / `move_src` / `move_dst`。
    pub operation: String,
    /// 模型/工具使用的原始路径（审计）。
    pub path: String,
    /// 实际写入对象（链接写入时为物理目标；删除/移动时为目录项本身）。
    pub target_path: String,
    pub before: ImageWrite,
    pub planned_after: ImageWrite,
}

/// 一个路径的恢复计划。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestorePath {
    pub path: String,
    pub target_path: String,
    pub before: ImageRecord,
    /// 该轮最后一次写入后的内容；预检必须与当前文件一致。
    pub expected_after: ImageRecord,
    pub worktree_root: Option<String>,
    pub git_dir: Option<String>,
    pub head_ref: Option<String>,
    pub head_oid: Option<String>,
}

/// 文件动作的可用性。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileAvailability {
    /// 快照完整，有 N 个路径可恢复。
    Ready { paths: usize },
    /// 快照完整而该轮确认没有文件变更。
    KnownEmpty,
    /// 该轮此前已恢复。
    AlreadyRestored,
    /// 旧轮次 / 缺 scope。
    Unavailable,
    /// 捕获未封口或存在未结算 mutation（证据不足）。
    Incomplete,
}

/// 恢复计划。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestorePlan {
    pub session_id: String,
    pub user_message_id: String,
    pub revision: i64,
    pub availability: FileAvailability,
    pub paths: Vec<RestorePath>,
    /// 已删除对话后仍需回填的原文（仅在操作表里保存）。
    pub user_text: String,
}

/// mutation prepare 的结果：每个路径一条记录 id。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedTicket {
    pub ids: Vec<i64>,
}

/// commit 时观察到的实际结果（与 prepare 顺序一致）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservedWrite {
    pub actual_after: ImageWrite,
    pub success: bool,
}

/// protected 操作日志条目。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationRecord {
    pub operation_id: String,
    pub phase: String,
    pub user_text: String,
    pub error_code: Option<String>,
}

/// 撤销最终提交参数。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RewindCommit {
    pub operation_id: String,
    pub session_id: String,
    pub target_user_message_id: String,
    pub expected_revision: i64,
    pub kind: String,
    pub delete_conversation: bool,
    pub revert_files: bool,
    pub restored_file_count: i64,
    pub files_already_restored: bool,
}

/// 最终提交结果；重复提交返回原结果（幂等）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RewindCommitted {
    pub user_text: String,
    /// 是否本次真正执行（false = 幂等重放）。
    pub executed: bool,
    /// 删空对话后会话是否已被整体移除（提示与快照随会话级联清理）。
    pub session_deleted: bool,
}

impl Database {
    /// 读取会话最新用户轮次（含 scope 与 revision）。
    pub fn latest_user_turn(
        &self,
        workspace: &Path,
        session_id: &str,
    ) -> Result<Option<LatestTurn>> {
        let Some(workspace_external) = self.workspace_external_id(workspace)? else {
            return Ok(None);
        };
        let owned: Option<i64> = self
            .conn
            .query_row(
                "SELECT id FROM agent_session WHERE external_id = ?1 AND workspace_id = ?2",
                params![session_id, workspace_external],
                |row| row.get(0),
            )
            .optional()?;
        if owned.is_none() {
            return Ok(None);
        }
        let revision: i64 = self.conn.query_row(
            "SELECT revision FROM agent_session WHERE external_id = ?1",
            params![session_id],
            |row| row.get(0),
        )?;
        let row: Option<(String, i64, i64)> = self
            .conn
            .query_row(
                r#"SELECT external_id, sequence, turn_index FROM agent_message
                   WHERE session_id = ?1 AND role = 'user'
                   ORDER BY sequence DESC LIMIT 1"#,
                params![session_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        let Some((user_message_id, sequence, turn_index)) = row else {
            return Ok(None);
        };
        let text: String = self
            .conn
            .query_row(
                "SELECT content FROM agent_message_part WHERE message_id = ?1 AND kind = 'text' ORDER BY position LIMIT 1",
                params![user_message_id],
                |row| row.get(0),
            )
            .optional()?
            .unwrap_or_default();
        Ok(Some(LatestTurn {
            scope: self.turn_scope(&user_message_id)?,
            user_message_id,
            sequence,
            turn_index,
            text,
            revision,
        }))
    }

    /// 读取某个用户轮次的文件 scope。
    pub fn turn_scope(&self, user_message_id: &str) -> Result<Option<TurnScopeRecord>> {
        Ok(self
            .conn
            .query_row(
                r#"SELECT user_message_id, state, capture_version, snapshot_bytes, files_restored
                   FROM agent_turn_file_scope WHERE user_message_id = ?1"#,
                params![user_message_id],
                |row| {
                    Ok(TurnScopeRecord {
                        user_message_id: row.get(0)?,
                        state: row.get(1)?,
                        capture_version: row.get(2)?,
                        snapshot_bytes: row.get(3)?,
                        files_restored: row.get::<_, i64>(4)? != 0,
                    })
                },
            )
            .optional()?)
    }

    /// 会话 revision（恢复前复核）。
    pub fn session_revision(&self, session_id: &str) -> Result<i64> {
        Ok(self.conn.query_row(
            "SELECT revision FROM agent_session WHERE external_id = ?1",
            params![session_id],
            |row| row.get(0),
        )?)
    }

    /// 载入会话的持久化撤销提示（按 sequence 升序）。
    pub fn load_notices(&self, workspace: &Path, session_id: &str) -> Result<Vec<NoticeRecord>> {
        let Some(workspace_external) = self.workspace_external_id(workspace)? else {
            return Ok(Vec::new());
        };
        let mut statement = self.conn.prepare(
            r#"SELECT n.id, n.sequence, n.kind, n.target_user_message_id,
                      n.restored_file_count, n.files_already_restored, n.created_at_ms
               FROM agent_session_notice n
               JOIN agent_session s ON s.external_id = n.session_id
               WHERE n.session_id = ?1 AND s.workspace_id = ?2
               ORDER BY n.sequence"#,
        )?;
        let rows = statement.query_map(params![session_id, workspace_external], |row| {
            Ok(NoticeRecord {
                id: row.get(0)?,
                sequence: row.get(1)?,
                kind: row.get(2)?,
                target_user_message_id: row.get(3)?,
                restored_file_count: row.get(4)?,
                files_already_restored: row.get::<_, i64>(5)? != 0,
                created_at_ms: row.get(6)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(DbError::from)
    }

    /// 记录一次 mutation 的 before / planned-after，返回可提交的记录 id。
    ///
    /// 限额在该次写入之前检查：超限返回错误，调用方必须**零写入**。
    pub fn prepare_mutations(
        &self,
        workspace: &Path,
        session_id: &str,
        user_message_id: &str,
        tool_call_id: &str,
        changes: &[PathMutationWrite],
    ) -> Result<PreparedTicket> {
        let workspace_external = self
            .workspace_external_id(workspace)?
            .ok_or(DbError::WorkspaceMissing)?;
        let owned: Option<i64> = self
            .conn
            .query_row(
                "SELECT id FROM agent_session WHERE external_id = ?1 AND workspace_id = ?2",
                params![session_id, workspace_external],
                |row| row.get(0),
            )
            .optional()?;
        if owned.is_none() {
            return Err(DbError::SessionMissing);
        }
        let transaction = self.conn.unchecked_transaction()?;
        let scope: Option<(String, i64)> = transaction
            .query_row(
                "SELECT state, snapshot_bytes FROM agent_turn_file_scope WHERE user_message_id = ?1 AND session_id = ?2",
                params![user_message_id, session_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let Some((state, used)) = scope else {
            return Err(DbError::RewindUnavailable);
        };
        if state != SCOPE_CAPTURING {
            return Err(DbError::RewindUnavailable);
        }
        let mut added: u64 = 0;
        for change in changes {
            if change.before.size() > MAX_IMAGE_BYTES
                || change.planned_after.size() > MAX_IMAGE_BYTES
            {
                return Err(DbError::RewindLimit);
            }
            added = added.saturating_add(change.before.size());
            added = added.saturating_add(change.planned_after.size());
        }
        if u64::try_from(used)
            .unwrap_or(u64::MAX)
            .saturating_add(added)
            > MAX_TURN_SNAPSHOT_BYTES
        {
            return Err(DbError::RewindLimit);
        }
        let now = now_ms();
        let mut ids = Vec::with_capacity(changes.len());
        for (ordinal, change) in changes.iter().enumerate() {
            let before = insert_image(&transaction, session_id, &change.before, now)?;
            let planned = insert_image(&transaction, session_id, &change.planned_after, now)?;
            transaction.execute(
                r#"INSERT INTO agent_file_mutation
                       (external_id, session_id, user_message_id, tool_call_id, ordinal,
                        operation, path, target_path,
                        before_image_id, planned_after_image_id, actual_after_image_id,
                        worktree_root, git_dir, head_ref, head_oid, state, created_at_ms)
                   VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, NULL,
                           ?11, ?12, ?13, ?14, 'prepared', ?15)"#,
                params![
                    new_external_id("mut"),
                    session_id,
                    user_message_id,
                    tool_call_id,
                    ordinal as i64,
                    change.operation,
                    change.path,
                    change.target_path,
                    before,
                    planned,
                    change.before.worktree_root,
                    change.before.git_dir,
                    change.before.head_ref,
                    change.before.head_oid,
                    now,
                ],
            )?;
            ids.push(transaction.last_insert_rowid());
        }
        transaction.execute(
            "UPDATE agent_turn_file_scope SET snapshot_bytes = snapshot_bytes + ?1 WHERE user_message_id = ?2",
            params![i64::try_from(added).unwrap_or(i64::MAX), user_message_id],
        )?;
        transaction.commit()?;
        Ok(PreparedTicket { ids })
    }

    /// 记录实际观察到的结果（prepared → committed）。
    ///
    /// 重复提交同一记录幂等；记录不存在或已以另一实际结果提交时返回冲突。
    pub fn commit_mutations(
        &self,
        workspace: &Path,
        session_id: &str,
        ticket: &PreparedTicket,
        observed: &[ObservedWrite],
    ) -> Result<()> {
        let workspace_external = self
            .workspace_external_id(workspace)?
            .ok_or(DbError::WorkspaceMissing)?;
        let transaction = self.conn.unchecked_transaction()?;
        let owned: Option<i64> = transaction
            .query_row(
                "SELECT id FROM agent_session WHERE external_id = ?1 AND workspace_id = ?2",
                params![session_id, workspace_external],
                |row| row.get(0),
            )
            .optional()?;
        if owned.is_none() {
            return Err(DbError::SessionMissing);
        }
        if ticket.ids.len() != observed.len() {
            return Err(DbError::MutationConflict);
        }
        let now = now_ms();
        for (id, item) in ticket.ids.iter().zip(observed) {
            let existing: Option<String> = transaction
                .query_row(
                    "SELECT state FROM agent_file_mutation WHERE id = ?1 AND session_id = ?2",
                    params![id, session_id],
                    |row| row.get(0),
                )
                .optional()?;
            let Some(state) = existing else {
                return Err(DbError::MutationConflict);
            };
            if state == MUTATION_COMMITTED {
                continue;
            }
            if state != MUTATION_PREPARED {
                return Err(DbError::MutationConflict);
            }
            let actual = insert_image(&transaction, session_id, &item.actual_after, now)?;
            transaction.execute(
                "UPDATE agent_file_mutation SET actual_after_image_id = ?1, state = 'committed', success = ?2, committed_at_ms = ?3 WHERE id = ?4",
                params![actual, i64::from(item.success), now, id],
            )?;
        }
        transaction.commit()?;
        Ok(())
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
            let first = self
                .image(group.first().expect("non-empty group").before)?
                .ok_or(DbError::RewindUnavailable)?;
            let last = previous_after.ok_or(DbError::RewindUnavailable)?;
            let head = group.last().expect("non-empty group");
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

    /// 开始一次撤销操作（幂等：同 operation_id 返回既有记录）。
    pub fn begin_operation(
        &self,
        workspace: &Path,
        operation_id: &str,
        session_id: &str,
        target_user_message_id: &str,
        expected_revision: i64,
        kind: &str,
        user_text: &str,
    ) -> Result<OperationRecord> {
        let workspace_external = self
            .workspace_external_id(workspace)?
            .ok_or(DbError::WorkspaceMissing)?;
        let transaction = self.conn.unchecked_transaction()?;
        let owned: Option<i64> = transaction
            .query_row(
                "SELECT id FROM agent_session WHERE external_id = ?1 AND workspace_id = ?2",
                params![session_id, workspace_external],
                |row| row.get(0),
            )
            .optional()?;
        if owned.is_none() {
            return Err(DbError::SessionMissing);
        }
        if let Some(existing) = operation_by_id(&transaction, operation_id)? {
            transaction.commit()?;
            return Ok(existing);
        }
        let now = now_ms();
        transaction.execute(
            r#"INSERT INTO agent_rewind_operation
                   (external_id, operation_id, session_id, action, target_user_message_id,
                    expected_revision, phase, user_text, created_at_ms, updated_at_ms)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'preparing', ?7, ?8, ?8)"#,
            params![
                new_external_id("op"),
                operation_id,
                session_id,
                kind,
                target_user_message_id,
                expected_revision,
                user_text,
                now,
            ],
        )?;
        transaction.commit()?;
        Ok(OperationRecord {
            operation_id: operation_id.to_string(),
            phase: OP_PREPARING.to_string(),
            user_text: user_text.to_string(),
            error_code: None,
        })
    }

    /// 推进操作阶段。
    pub fn set_operation_phase(
        &self,
        operation_id: &str,
        phase: &str,
        error_code: Option<&str>,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE agent_rewind_operation SET phase = ?1, error_code = ?2, updated_at_ms = ?3 WHERE operation_id = ?4",
            params![phase, error_code, now_ms(), operation_id],
        )?;
        Ok(())
    }

    /// 按 operation_id 查询阶段（幂等重放）。
    pub fn operation(&self, operation_id: &str) -> Result<Option<OperationRecord>> {
        operation_by_id(&self.conn, operation_id)
    }

    /// 会话中尚未完成的撤销操作（异常退出遗留）；存在时禁止再次撤销。
    pub fn unfinished_operation(&self, session_id: &str) -> Result<Option<OperationRecord>> {
        Ok(self
            .conn
            .query_row(
                r#"SELECT operation_id, phase, user_text, error_code
                   FROM agent_rewind_operation
                   WHERE session_id = ?1 AND phase IN ('preparing', 'restoring', 'files_restored')
                   ORDER BY id DESC LIMIT 1"#,
                params![session_id],
                |row| {
                    Ok(OperationRecord {
                        operation_id: row.get(0)?,
                        phase: row.get(1)?,
                        user_text: row.get(2)?,
                        error_code: row.get(3)?,
                    })
                },
            )
            .optional()?)
    }

    /// 最终事务：删对话 + 提示 + revision + operation committed 原子提交。
    ///
    /// 同一 operation_id 重复提交返回既有结果（`executed = false`），不再删第二次。
    pub fn commit_rewind(
        &self,
        workspace: &Path,
        commit: &RewindCommit,
    ) -> Result<RewindCommitted> {
        let workspace_external = self
            .workspace_external_id(workspace)?
            .ok_or(DbError::WorkspaceMissing)?;
        let transaction = self.conn.unchecked_transaction()?;
        let owned: Option<i64> = transaction
            .query_row(
                "SELECT id FROM agent_session WHERE external_id = ?1 AND workspace_id = ?2",
                params![commit.session_id, workspace_external],
                |row| row.get(0),
            )
            .optional()?;
        if owned.is_none() {
            return Err(DbError::SessionMissing);
        }
        let existing = operation_by_id(&transaction, &commit.operation_id)?;
        match existing {
            Some(record) if record.phase == OP_COMMITTED => {
                transaction.commit()?;
                return Ok(RewindCommitted {
                    user_text: record.user_text,
                    executed: false,
                    session_deleted: false,
                });
            }
            None => return Err(DbError::OperationMissing),
            Some(_) => {}
        }
        let revision: i64 = transaction.query_row(
            "SELECT revision FROM agent_session WHERE external_id = ?1",
            params![commit.session_id],
            |row| row.get(0),
        )?;
        if revision != commit.expected_revision {
            return Err(DbError::RewindTargetChanged);
        }
        // 目标必须仍是最新用户消息。
        let latest: Option<(String, i64)> = transaction
            .query_row(
                r#"SELECT external_id, turn_index FROM agent_message
                   WHERE session_id = ?1 AND role = 'user'
                   ORDER BY sequence DESC LIMIT 1"#,
                params![commit.session_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let Some((latest_id, turn_index)) = latest else {
            return Err(DbError::RewindTargetChanged);
        };
        if latest_id != commit.target_user_message_id {
            return Err(DbError::RewindTargetChanged);
        }
        // 在删除之前取联合新序号：保留的 tip 必须保住高水位。
        let next_sequence: i64 = transaction.query_row(
            r#"SELECT COALESCE(MAX(sequence) + 1, 0) FROM (
                   SELECT sequence FROM agent_message WHERE session_id = ?1
                   UNION ALL
                   SELECT sequence FROM agent_session_notice WHERE session_id = ?1
               )"#,
            params![commit.session_id],
            |row| row.get(0),
        )?;
        let user_text: String = transaction
            .query_row(
                "SELECT content FROM agent_message_part WHERE message_id = ?1 AND kind = 'text' ORDER BY position LIMIT 1",
                params![commit.target_user_message_id],
                |row| row.get(0),
            )
            .optional()?
            .unwrap_or_default();
        if commit.delete_conversation {
            transaction.execute(
                "DELETE FROM agent_message WHERE session_id = ?1 AND turn_index >= ?2",
                params![commit.session_id, turn_index],
            )?;
        }
        // 删掉最后一轮后会话只剩提示（或什么都没有）：整体移除会话及其提示、快照、
        // mutation、操作日志与进程记录；不保留“只剩 Rewind 提示”的空会话。
        let remaining: i64 = transaction.query_row(
            "SELECT COUNT(*) FROM agent_message WHERE session_id = ?1",
            params![commit.session_id],
            |row| row.get(0),
        )?;
        if commit.delete_conversation && remaining == 0 {
            transaction.execute(
                "DELETE FROM agent_session WHERE external_id = ?1",
                params![commit.session_id],
            )?;
            transaction.commit()?;
            return Ok(RewindCommitted {
                user_text,
                executed: true,
                session_deleted: true,
            });
        }
        let now = now_ms();
        transaction.execute(
            r#"INSERT INTO agent_session_notice
                   (id, session_id, sequence, operation_id, kind, target_user_message_id,
                    restored_file_count, files_already_restored, created_at_ms)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
               ON CONFLICT(operation_id) DO NOTHING"#,
            params![
                new_external_id("notice"),
                commit.session_id,
                next_sequence,
                commit.operation_id,
                commit.kind,
                commit.target_user_message_id,
                commit.restored_file_count,
                i64::from(commit.files_already_restored),
                now,
            ],
        )?;
        if commit.revert_files {
            transaction.execute(
                "UPDATE agent_turn_file_scope SET files_restored = 1 WHERE session_id = ?1 AND user_message_id = ?2",
                params![commit.session_id, commit.target_user_message_id],
            )?;
        }
        // usage 聚合按剩余消息重算，不保留被删轮次的用量。
        transaction.execute(
            r#"UPDATE agent_session
               SET revision = revision + 1,
                   updated_at_ms = ?1,
                   last_message_at_ms = COALESCE(
                       (SELECT MAX(created_at_ms) FROM agent_message WHERE session_id = ?2), ?1),
                   total_input_tokens = COALESCE(
                       (SELECT SUM(input_tokens) FROM agent_message WHERE session_id = ?2), 0),
                   total_output_tokens = COALESCE(
                       (SELECT SUM(output_tokens) FROM agent_message WHERE session_id = ?2), 0)
               WHERE external_id = ?2"#,
            params![now, commit.session_id],
        )?;
        transaction.execute(
            "UPDATE agent_rewind_operation SET phase = 'committed', user_text = ?1, updated_at_ms = ?2 WHERE operation_id = ?3",
            params![user_text, now, commit.operation_id],
        )?;
        transaction.commit()?;
        Ok(RewindCommitted {
            user_text,
            executed: true,
            session_deleted: false,
        })
    }

    /// 测试钩子：模拟升级前的旧轮次（删除 scope）。
    #[cfg(test)]
    pub(crate) fn drop_turn_scope_for_tests(&self, user_message_id: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM agent_turn_file_scope WHERE user_message_id = ?1",
            params![user_message_id],
        )?;
        Ok(())
    }

    /// 清理指定会话中没有未完成操作引用的镜像（恢复成功后调用）。
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

/// 插入一份镜像并返回 id。
fn insert_image(
    connection: &Connection,
    session_id: &str,
    image: &ImageWrite,
    now: i64,
) -> Result<i64> {
    connection.execute(
        r#"INSERT INTO agent_file_image
               (external_id, session_id, kind, bytes, mode, link_target, git_oid,
                worktree_root, git_dir, head_ref, head_oid, size, created_at_ms)
           VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)"#,
        params![
            new_external_id("img"),
            session_id,
            image.kind,
            image.bytes,
            image.mode,
            image.link_target,
            image.git_oid,
            image.worktree_root,
            image.git_dir,
            image.head_ref,
            image.head_oid,
            i64::try_from(image.size()).unwrap_or(i64::MAX),
            now,
        ],
    )?;
    Ok(connection.last_insert_rowid())
}

/// 按 operation_id 读取操作记录。
fn operation_by_id(connection: &Connection, operation_id: &str) -> Result<Option<OperationRecord>> {
    Ok(connection
        .query_row(
            "SELECT operation_id, phase, user_text, error_code FROM agent_rewind_operation WHERE operation_id = ?1",
            params![operation_id],
            |row| {
                Ok(OperationRecord {
                    operation_id: row.get(0)?,
                    phase: row.get(1)?,
                    user_text: row.get(2)?,
                    error_code: row.get(3)?,
                })
            },
        )
        .optional()?)
}

/// 在 `begin_turn` 事务内创建本轮 scope。
pub(super) fn begin_turn_scope(
    transaction: &Connection,
    session_id: &str,
    user_message_id: &str,
    now: i64,
) -> Result<()> {
    transaction.execute(
        r#"INSERT INTO agent_turn_file_scope
               (session_id, user_message_id, capture_version, state, snapshot_bytes,
                files_restored, created_at_ms)
           VALUES (?1, ?2, 1, 'capturing', 0, 0, ?3)"#,
        params![session_id, user_message_id, now],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests;

/// 终态提交时封口 scope：仍有未结算 mutation 时保持 `capturing`（证据不足）。
pub(super) fn seal_turn_scope(
    transaction: &Connection,
    session_id: &str,
    assistant_message_id: &str,
) -> Result<()> {
    transaction.execute(
        r#"UPDATE agent_turn_file_scope
           SET state = 'complete'
           WHERE session_id = ?1
             AND user_message_id = (
                 SELECT u.external_id FROM agent_message m
                 JOIN agent_message u ON u.session_id = m.session_id
                     AND u.turn_index = m.turn_index AND u.role = 'user'
                 WHERE m.external_id = ?2
                 ORDER BY u.sequence LIMIT 1
             )
             AND NOT EXISTS (
                 SELECT 1 FROM agent_file_mutation f
                 WHERE f.session_id = ?1 AND f.state = 'prepared'
                   AND f.user_message_id = agent_turn_file_scope.user_message_id
             )"#,
        params![session_id, assistant_message_id],
    )?;
    Ok(())
}
