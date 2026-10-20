//! 会话进程元数据持久化：登记、输出快照合并、终态提交与启动恢复。
//!
//! 边界与不变量（对应 design §9.2）：
//!
//! - 工具原始结果（`agent_tool_call.result_json`）**不可变**：进程后续输出与退出状态
//!   只写本表，绝不改写已经回给模型的结果；
//! - `revision` 单调整数：较旧 revision 的写入命中 0 行，不能覆盖终态；
//! - `(runtime_epoch, process_session_id)` 唯一：不同运行代次不会串号；
//! - 外键级联删除：会话删除即清理进程记录；
//! - 恢复时遗留 `starting`/`running` 统一标 `interrupted`：不重跑、不用旧 PID 操作进程。

use std::path::{Path, PathBuf};

use rusqlite::params;

use super::super::{Database, DbError, Result, new_external_id, now_ms};

/// 一次进程状态的持久化记录（加载与展示用）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessRecord {
    pub external_id: String,
    pub chat_session_id: String,
    pub origin_call_id: String,
    pub runtime_epoch: String,
    pub process_session_id: i64,
    /// starting / running / exited / stopped / timed_out / interrupted / failed
    pub state: String,
    pub exit_code: Option<i64>,
    pub stop_reason: Option<String>,
    pub command: String,
    pub cwd: String,
    pub tty: bool,
    pub output_tail: String,
    pub output_start: i64,
    pub output_end: i64,
    pub dropped_bytes: i64,
    pub started_at_ms: i64,
    pub ended_at_ms: Option<i64>,
    pub revision: i64,
}

/// 一次进程终态提交的入参。
#[derive(Debug, Clone)]
pub struct ProcessFinish<'a> {
    pub runtime_epoch: &'a str,
    pub process_session_id: u64,
    /// exited / stopped / timed_out / interrupted / failed
    pub state: &'a str,
    pub exit_code: Option<i32>,
    /// stopped / timed_out / interrupted；其余为 None。
    pub stop_reason: Option<&'a str>,
    pub snapshot: OutputSnapshot<'a>,
    /// 单调修订号（必须大于已存值）。
    pub revision: i64,
}

/// 登记一个新建进程（`state = starting`）。
#[derive(Debug, Clone)]
pub struct NewProcess<'a> {
    pub chat_session_id: &'a str,
    pub origin_call_id: &'a str,
    pub runtime_epoch: &'a str,
    pub process_session_id: u64,
    pub command: &'a str,
    pub cwd: &'a Path,
    pub tty: bool,
}

/// 一次输出快照（受控、有界）。
#[derive(Debug, Clone)]
pub struct OutputSnapshot<'a> {
    pub output_tail: &'a [u8],
    pub output_start: i64,
    pub output_end: i64,
    pub dropped_bytes: i64,
}

impl Database {
    /// 登记新进程；同一 `(runtime_epoch, process_session_id)` 重复登记返回冲突。
    ///
    /// 先校验会话属于该工作区（跨工作区写被拒），再插入。
    pub fn begin_process(&self, workspace: &Path, new: &NewProcess<'_>) -> Result<String> {
        let workspace_external = self
            .workspace_external_id(workspace)?
            .ok_or(DbError::WorkspaceMissing)?;
        let transaction = self.conn.unchecked_transaction()?;
        let owned: Option<i64> = transaction
            .query_row(
                "SELECT id FROM agent_session WHERE external_id = ?1 AND workspace_id = ?2",
                params![new.chat_session_id, workspace_external],
                |row| row.get(0),
            )
            .ok();
        if owned.is_none() {
            return Err(DbError::SessionMissing);
        }
        let external_id = new_external_id("process");
        transaction
            .execute(
                r#"INSERT INTO agent_process
                   (external_id, chat_session_id, origin_call_id, runtime_epoch,
                    process_session_id, state, command, cwd, tty, started_at_ms, revision)
               VALUES (?1, ?2, ?3, ?4, ?5, 'starting', ?6, ?7, ?8, ?9, 0)"#,
                params![
                    external_id,
                    new.chat_session_id,
                    new.origin_call_id,
                    new.runtime_epoch,
                    i64::try_from(new.process_session_id).unwrap_or(i64::MAX),
                    new.command,
                    new.cwd.display().to_string(),
                    i64::from(new.tty),
                    now_ms(),
                ],
            )
            .map_err(classify)?;
        transaction.commit()?;
        Ok(external_id)
    }

    /// 合并一次输出快照；`revision` 必须大于已存值，且该行仍是运行态。
    ///
    /// 返回是否真正写入（较旧 revision / 已终态时为 false，不覆盖终态）。
    pub fn update_process_output(
        &self,
        workspace: &Path,
        runtime_epoch: &str,
        process_session_id: u64,
        snapshot: &OutputSnapshot<'_>,
        revision: i64,
    ) -> Result<bool> {
        let workspace_external = self
            .workspace_external_id(workspace)?
            .ok_or(DbError::WorkspaceMissing)?;
        let updated = self.conn.execute(
            r#"UPDATE agent_process
               SET output_tail = ?1, output_start = ?2, output_end = ?3,
                   dropped_bytes = ?4, revision = ?5, state = 'running'
               WHERE runtime_epoch = ?6 AND process_session_id = ?7
                 AND revision < ?5 AND state IN ('starting', 'running')
                 AND chat_session_id IN (
                     SELECT external_id FROM agent_session WHERE workspace_id = ?8
                 )"#,
            params![
                snapshot.output_tail,
                snapshot.output_start,
                snapshot.output_end,
                snapshot.dropped_bytes,
                revision,
                runtime_epoch,
                i64::try_from(process_session_id).unwrap_or(i64::MAX),
                workspace_external,
            ],
        )?;
        Ok(updated > 0)
    }

    /// 提交终态；`revision` 必须大于已存值，重复终态幂等。
    ///
    /// 参数打包成 [`ProcessFinish`]（字段远超 8 个，逐个传会把调用点变成一串位置参数）。
    /// 返回是否真正写入。
    pub fn finish_process(&self, workspace: &Path, finish: &ProcessFinish<'_>) -> Result<bool> {
        let ProcessFinish {
            runtime_epoch,
            process_session_id,
            state,
            exit_code,
            stop_reason,
            snapshot,
            revision,
        } = finish.clone();
        let workspace_external = self
            .workspace_external_id(workspace)?
            .ok_or(DbError::WorkspaceMissing)?;
        let updated = self.conn.execute(
            r#"UPDATE agent_process
               SET state = ?1, exit_code = ?2, stop_reason = ?3,
                   output_tail = ?4, output_start = ?5, output_end = ?6,
                   dropped_bytes = ?7, revision = ?8, ended_at_ms = ?9
               WHERE runtime_epoch = ?10 AND process_session_id = ?11
                 AND revision < ?8
                 AND chat_session_id IN (
                     SELECT external_id FROM agent_session WHERE workspace_id = ?12
                 )"#,
            params![
                state,
                exit_code.map(i64::from),
                stop_reason,
                snapshot.output_tail,
                snapshot.output_start,
                snapshot.output_end,
                snapshot.dropped_bytes,
                revision,
                now_ms(),
                runtime_epoch,
                i64::try_from(process_session_id).unwrap_or(i64::MAX),
                workspace_external,
            ],
        )?;
        Ok(updated > 0)
    }

    /// 按会话批量载入进程记录（不做每卡片一次查询）。
    pub fn load_processes_for_session(
        &self,
        workspace: &Path,
        chat_session_id: &str,
    ) -> Result<Vec<ProcessRecord>> {
        let workspace_external = self
            .workspace_external_id(workspace)?
            .ok_or(DbError::WorkspaceMissing)?;
        let owned: Option<i64> = self
            .conn
            .query_row(
                "SELECT id FROM agent_session WHERE external_id = ?1 AND workspace_id = ?2",
                params![chat_session_id, workspace_external],
                |row| row.get(0),
            )
            .ok();
        if owned.is_none() {
            return Err(DbError::SessionMissing);
        }
        let mut statement = self.conn.prepare(
            r#"SELECT external_id, chat_session_id, origin_call_id, runtime_epoch,
                      process_session_id, state, exit_code, stop_reason, command, cwd, tty,
                      output_tail, output_start, output_end, dropped_bytes,
                      started_at_ms, ended_at_ms, revision
               FROM agent_process
               WHERE chat_session_id = ?1
               ORDER BY started_at_ms, id"#,
        )?;
        let rows = statement.query_map(params![chat_session_id], |row| {
            let tail: Option<Vec<u8>> = row.get(11)?;
            Ok(ProcessRecord {
                external_id: row.get(0)?,
                chat_session_id: row.get(1)?,
                origin_call_id: row.get(2)?,
                runtime_epoch: row.get(3)?,
                process_session_id: row.get(4)?,
                state: row.get(5)?,
                exit_code: row.get(6)?,
                stop_reason: row.get(7)?,
                command: row.get(8)?,
                cwd: row.get(9)?,
                tty: row.get::<_, i64>(10)? != 0,
                output_tail: tail
                    .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
                    .unwrap_or_default(),
                output_start: row.get(12)?,
                output_end: row.get(13)?,
                dropped_bytes: row.get(14)?,
                started_at_ms: row.get(15)?,
                ended_at_ms: row.get(16)?,
                revision: row.get(17)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(DbError::from)
    }

    /// 启动恢复：遗留 starting/running 一律标 interrupted（不重跑、不重连）。
    ///
    /// 幂等：第二次调用返回 0。
    pub fn recover_interrupted_processes(&self) -> Result<usize> {
        let now = now_ms();
        let updated = self.conn.execute(
            r#"UPDATE agent_process
               SET state = 'interrupted', stop_reason = 'lx exited before the process finished',
                   ended_at_ms = ?1, revision = revision + 1
               WHERE state IN ('starting', 'running')"#,
            params![now],
        )?;
        Ok(updated)
    }

    /// 会话级联清理进程记录（会话删除前调用；外键也会级联，这里显式返回清理条数）。
    pub fn delete_processes_for_session(&self, chat_session_id: &str) -> Result<usize> {
        let removed = self.conn.execute(
            "DELETE FROM agent_process WHERE chat_session_id = ?1",
            params![chat_session_id],
        )?;
        Ok(removed)
    }

    /// 该会话的归属工作区（事件层按 workspace 校验用）。
    pub fn session_workspace(&self, chat_session_id: &str) -> Result<Option<PathBuf>> {
        let mut statement = self.conn.prepare(
            r#"SELECT w.path FROM agent_session s
               JOIN workspace w ON w.external_id = s.workspace_id
               WHERE s.external_id = ?1"#,
        )?;
        let mut rows = statement.query(params![chat_session_id])?;
        let row = rows.next()?;
        Ok(row.and_then(|row| {
            row.get::<_, Option<String>>(0)
                .ok()
                .flatten()
                .map(PathBuf::from)
        }))
    }
}

/// 唯一约束冲突转成可区分错误（重复进程身份）。
fn classify(error: rusqlite::Error) -> DbError {
    if error.to_string().contains("UNIQUE constraint failed") {
        return DbError::ProcessConflict;
    }
    DbError::Sqlite(error)
}

#[cfg(test)]
mod tests;
