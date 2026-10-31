//! Schema 迁移：v4 提示词分页、v5 去状态列、v7 聊天五表、v8 part 时刻、v9 工具时间线、v21 待办日程化。
//!
//! 迁移与 `PRAGMA user_version` 推进同事务完成：任一步失败整体回滚，旧数据与版本号
//! 保持不变，下次打开重试。历史迁移文本（v4/v5/v7/v8）不得改写，只能在尾部追加新版本；
//! 迁移期不执行任何文件操作。

mod schema;

use rusqlite::{Connection, params};
use schema::*;

use super::{Database, DbError, FIRST_PAGE_NAME, Result, SCHEMA_VERSION};

impl Database {
    /// 迁移到当前 Schema：建表与版本推进在同一事务中完成，幂等。
    ///
    /// 开发期的资产/附件表已随截图粘贴功能移除，老库中遗留的这些表不再被读写；
    /// 已有数据（工作区、提示词）原样保留。版本高于程序支持值时返回错误。
    ///
    /// v4 迁移把每条旧提示词的正文原样复制为唯一 `Page 1` 并指向当前页；
    /// v5 迁移重建提示词表移除状态列（在关闭外键的独立事务中完成，不删页表数据）。
    /// 已升级库不再回填（版本号门控），空库得到同一结构。
    pub(crate) fn migrate(&mut self) -> Result<()> {
        let version: i64 = self
            .conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if version > SCHEMA_VERSION {
            return Err(DbError::UnsupportedSchema(version));
        }
        if version < SCHEMA_VERSION {
            if version < 4 {
                let transaction = self.conn.transaction()?;
                transaction.execute_batch(SCHEMA)?;
                transaction.execute_batch(SCHEMA_V4)?;
                // 老库的 workspace_prompt 由历史 SCHEMA 建立，缺 active_page_id 列。
                if !column_exists(&transaction, "workspace_prompt", "active_page_id")? {
                    transaction.execute_batch(
                        "ALTER TABLE workspace_prompt ADD COLUMN active_page_id INTEGER;",
                    )?;
                }
                // 旧正文逐字进入唯一 Page 1；已有页的提示词（重复迁移）不重复生成。
                transaction.execute(
                    r#"INSERT INTO workspace_prompt_page
                           (prompt_id, position, name, content, created_at, updated_at)
                       SELECT p.id, 0, ?1, p.content, datetime('now'), datetime('now')
                       FROM workspace_prompt p
                       WHERE NOT EXISTS (
                           SELECT 1 FROM workspace_prompt_page pg WHERE pg.prompt_id = p.id
                       )"#,
                    params![FIRST_PAGE_NAME],
                )?;
                transaction.execute(
                    r#"UPDATE workspace_prompt
                       SET active_page_id = (
                           SELECT pg.id FROM workspace_prompt_page pg
                           WHERE pg.prompt_id = workspace_prompt.id
                           ORDER BY pg.position LIMIT 1
                       )
                       WHERE active_page_id IS NULL"#,
                    [],
                )?;
                transaction.pragma_update(None, "user_version", 4)?;
                transaction.commit()?;
            }
            // 旧库（v1..v4）带状态列：整表重建去列，必须在事务外关闭外键，
            // 否则 `DROP TABLE workspace_prompt` 会级联删除页表数据。
            if column_exists(&self.conn, "workspace_prompt", "status")? {
                self.rebuild_prompt_without_status()?;
            }
            let transaction = self.conn.transaction()?;
            transaction.execute_batch(SCHEMA)?;
            // v7：聊天会话五表；建表与索引成功后才推进 user_version。
            if version < 7 {
                transaction.execute_batch(SCHEMA_V7)?;
                // 打开外键时的完整性自检：v7 只增表，发现违规说明旧数据已坏。
                ensure_no_foreign_key_violation(&transaction)?;
            }
            // v8：消息 part 记录流中起止时刻（思考耗时展示）；老库补列（新库由 SCHEMA 建出）。
            if version < 8 && !column_exists(&transaction, "agent_message_part", "started_at_ms")? {
                transaction.execute_batch(
                    "ALTER TABLE agent_message_part ADD COLUMN started_at_ms INTEGER;
                     ALTER TABLE agent_message_part ADD COLUMN ended_at_ms INTEGER;",
                )?;
            }
            // v9：part 独立完成时刻 + 工具调用重建（position / interrupted）。
            if version < 9 {
                migrate_v9(&transaction)?;
            }
            // v10：全局 prompt 历史表（只增表，不触碰既有数据）。
            if version < 10 {
                transaction.execute_batch(SCHEMA_V10)?;
                ensure_no_foreign_key_violation(&transaction)?;
            }
            // v11：会话进程元数据表（只增表，不触碰既有数据）。
            if version < 11 {
                transaction.execute_batch(SCHEMA_V11)?;
                ensure_no_foreign_key_violation(&transaction)?;
            }
            // v12：撤销 journal 与文件快照四表 + 会话 revision 列。
            if version < 12 {
                if !column_exists(&transaction, "agent_session", "revision")? {
                    transaction.execute_batch(
                        "ALTER TABLE agent_session ADD COLUMN revision INTEGER NOT NULL DEFAULT 0
                         CHECK (revision >= 0);",
                    )?;
                }
                transaction.execute_batch(SCHEMA_V12)?;
                ensure_no_foreign_key_violation(&transaction)?;
            }
            // 先推进到 v12：v13 重建会话表必须关闭外键，只能在独立连接态执行。
            // 若此处直接写 SCHEMA_VERSION 而随后 v13 失败，库会被标成 v13 却仍是 NOT NULL 旧表。
            if version < 12 {
                transaction.pragma_update(None, "user_version", 12)?;
            }
            transaction.commit()?;
        }
        if version < 13 {
            self.migrate_v13()?;
        }
        if version < 14 {
            // 只增表：不触碰既有消息与会话数据，失败整体回滚（版本保持 v13）。
            let transaction = self.conn.unchecked_transaction()?;
            transaction.execute_batch(SCHEMA_V14)?;
            ensure_no_foreign_key_violation(&transaction)?;
            // 只推进到本分支的版本：后续分支失败时不能把库标成更高版本，
            // 否则下次启动会跳过缺失的迁移（v15/v16 都依赖这一点）。
            transaction.pragma_update(None, "user_version", 14)?;
            transaction.commit()?;
        }
        if version < 15 {
            // 只增表：全局 todo 列表，不触碰既有数据，失败整体回滚（版本不推进）。
            let transaction = self.conn.unchecked_transaction()?;
            transaction.execute_batch(SCHEMA_V15_TODO)?;
            ensure_no_foreign_key_violation(&transaction)?;
            transaction.pragma_update(None, "user_version", 15)?;
            transaction.commit()?;
        }
        if version < 16 {
            // 只加列：subagent 报告与报告行标记。不重建任何表；`column_exists` 守卫
            // 让重复迁移幂等（旧库可能已被更早的二进制补过其中一列）。
            let transaction = self.conn.unchecked_transaction()?;
            for (table, column, ddl) in [
                (
                    "agent_subagent",
                    "report_json",
                    "ALTER TABLE agent_subagent ADD COLUMN report_json TEXT;",
                ),
                (
                    "agent_subagent",
                    "report_delivered",
                    "ALTER TABLE agent_subagent ADD COLUMN report_delivered INTEGER NOT NULL DEFAULT 0;",
                ),
                (
                    "agent_message",
                    "subagent_report_json",
                    "ALTER TABLE agent_message ADD COLUMN subagent_report_json TEXT;",
                ),
            ] {
                if !column_exists(&transaction, table, column)? {
                    transaction.execute_batch(ddl)?;
                }
            }
            ensure_no_foreign_key_violation(&transaction)?;
            // 只推进到本分支版本，v17 仍需执行（不能直接写 SCHEMA_VERSION）。
            transaction.pragma_update(None, "user_version", 16)?;
            transaction.commit()?;
        }
        if version < 17 {
            // 只加列：subagent 的短标题。旧记录留空，UI 不拿 task 首行冒充标题。
            let transaction = self.conn.unchecked_transaction()?;
            if !column_exists(&transaction, "agent_subagent", "title")? {
                transaction.execute_batch(
                    "ALTER TABLE agent_subagent ADD COLUMN title TEXT NOT NULL DEFAULT '';",
                )?;
            }
            ensure_no_foreign_key_violation(&transaction)?;
            transaction.pragma_update(None, "user_version", 17)?;
            transaction.commit()?;
        }
        if version < 18 {
            // 只增表：消息附件（可见引用元数据，图片本体存本地路径）。
            let transaction = self.conn.unchecked_transaction()?;
            transaction.execute_batch(SCHEMA_V18)?;
            ensure_no_foreign_key_violation(&transaction)?;
            transaction.pragma_update(None, "user_version", 18)?;
            transaction.commit()?;
        }
        // v18 开发库兼容：若已建含 `image_bytes` BLOB 列的旧版附件表，重建为仅存路径的当前结构。
        if column_exists(&self.conn, "agent_message_attachment", "image_bytes")? {
            self.conn.execute_batch("PRAGMA foreign_keys = OFF;")?;
            let result = (|| -> Result<()> {
                let transaction = self.conn.unchecked_transaction()?;
                transaction.execute_batch("DROP TABLE IF EXISTS agent_message_attachment;")?;
                transaction.execute_batch(SCHEMA_V18)?;
                ensure_no_foreign_key_violation(&transaction)?;
                transaction.commit()?;
                Ok(())
            })();
            self.conn.execute_batch("PRAGMA foreign_keys = ON;")?;
            result?;
        }
        if version < 19 {
            // 只增表 + 旧单条报告数据迁移：不重建旧表。
            let transaction = self.conn.unchecked_transaction()?;
            transaction.execute_batch(SCHEMA_V19)?;
            let legacy_count: i64 = transaction.query_row(
                "SELECT COUNT(*) FROM agent_subagent WHERE report_json IS NOT NULL AND report_json <> ''",
                [],
                |row| row.get(0),
            )?;
            transaction.execute(
                r#"INSERT OR IGNORE INTO agent_subagent_report
                       (external_id, agent_id, child_user_message_id, status, report_json,
                        completed_at_ms, delivered_at_ms)
                   SELECT
                       'rep-legacy-' || a.external_id,
                       a.external_id,
                       (SELECT m.external_id FROM agent_message m
                        WHERE m.session_id = a.child_session_id AND m.role = 'user'
                        ORDER BY m.sequence DESC LIMIT 1),
                       CASE WHEN a.status = 'failed' THEN 'failed' ELSE 'completed' END,
                       a.report_json,
                       COALESCE(a.completed_at_ms, a.updated_at_ms, a.created_at_ms),
                       CASE WHEN a.report_delivered <> 0
                            THEN COALESCE(a.completed_at_ms, a.updated_at_ms, a.created_at_ms)
                            ELSE NULL
                       END
                   FROM agent_subagent a
                   WHERE a.report_json IS NOT NULL AND a.report_json <> ''"#,
                [],
            )?;
            let migrated_count: i64 = transaction.query_row(
                "SELECT COUNT(*) FROM agent_subagent_report WHERE external_id LIKE 'rep-legacy-%'",
                [],
                |row| row.get(0),
            )?;
            if migrated_count < legacy_count {
                return Err(DbError::ForeignKeyViolation);
            }
            ensure_no_foreign_key_violation(&transaction)?;
            // 只推进到 19：v20 整表重建失败时不能把库标成已升级。
            transaction.pragma_update(None, "user_version", 19)?;
            transaction.commit()?;
        }
        if version < 20 {
            self.rebuild_attachment_with_skill_kind()?;
        }
        if version < 21 {
            self.migrate_v21_todo_schedule()?;
        }
        Ok(())
    }

    /// v21：todo 表扩展日期与优先级字段（`entry_date`, `priority`, `completed_date`），
    /// 并将唯一位置索引从全局 `(position)` 升级为按日 `(entry_date, position)`。
    fn migrate_v21_todo_schedule(&mut self) -> Result<()> {
        let transaction = self.conn.unchecked_transaction()?;
        if !column_exists(&transaction, "todo", "entry_date")? {
            transaction.execute_batch(
                "ALTER TABLE todo ADD COLUMN entry_date TEXT NOT NULL DEFAULT '';",
            )?;
        }
        if !column_exists(&transaction, "todo", "priority")? {
            transaction.execute_batch(
                "ALTER TABLE todo ADD COLUMN priority TEXT NOT NULL DEFAULT 'P1';",
            )?;
        }
        if !column_exists(&transaction, "todo", "completed_date")? {
            transaction.execute_batch("ALTER TABLE todo ADD COLUMN completed_date TEXT;")?;
        }
        let today = super::todo::today_date();
        transaction.execute(
            "UPDATE todo SET entry_date = ?1 WHERE entry_date = ''",
            params![today.as_str()],
        )?;
        transaction.execute(
            "UPDATE todo SET completed_date = entry_date WHERE status = 'done' AND completed_date IS NULL",
            [],
        )?;
        transaction.execute_batch(SCHEMA_V21_TODO_INDEXES)?;
        ensure_no_foreign_key_violation(&transaction)?;
        transaction.pragma_update(None, "user_version", 21)?;
        transaction.commit()?;
        Ok(())
    }

    /// v20：重建消息附件表，加入 `skill` kind。
    ///
    /// 关闭外键后重建（`DROP TABLE` 在打开外键时会级联删除消息），
    /// 行数与索引在事务内校验，失败不推进版本。
    fn rebuild_attachment_with_skill_kind(&mut self) -> Result<()> {
        self.conn.execute_batch("PRAGMA foreign_keys = OFF;")?;
        let result = (|| -> Result<()> {
            let transaction = self.conn.unchecked_transaction()?;
            let before: i64 = transaction.query_row(
                "SELECT COUNT(*) FROM agent_message_attachment",
                [],
                |row| row.get(0),
            )?;
            transaction.execute_batch(SCHEMA_V20_ATTACHMENT)?;
            let after: i64 = transaction.query_row(
                "SELECT COUNT(*) FROM agent_message_attachment",
                [],
                |row| row.get(0),
            )?;
            if before != after {
                return Err(DbError::ForeignKeyViolation);
            }
            ensure_no_foreign_key_violation(&transaction)?;
            transaction.pragma_update(None, "user_version", 20)?;
            transaction.commit()?;
            Ok(())
        })();
        self.conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        result
    }

    /// v13：会话模型三列可空 + 消息 kind/命令 JSON。
    ///
    /// 重建 `agent_session` 时关闭外键（否则 DROP 会级联清空消息、工具与进程），
    /// 完成后自检外键并在同一事务内把版本推到当前值；失败保持 v12 便于重试。
    fn migrate_v13(&mut self) -> Result<()> {
        self.conn.execute_batch("PRAGMA foreign_keys = OFF;")?;
        let result = (|| -> Result<()> {
            let transaction = self.conn.transaction()?;
            // 旧 shell v12 库没有 revision 列（该分支的 v12 早于 rewind 合并）。
            // 必须先补列：下面的整表重建会 `SELECT revision FROM agent_session`。
            if !column_exists(&transaction, "agent_session", "revision")? {
                transaction.execute_batch(
                    "ALTER TABLE agent_session ADD COLUMN revision INTEGER NOT NULL DEFAULT 0
                     CHECK (revision >= 0);",
                )?;
            }
            if session_model_not_null(&transaction)? {
                transaction.execute_batch(SCHEMA_V13_SESSION)?;
            }
            if !column_exists(&transaction, "agent_message", "kind")? {
                transaction.execute_batch(
                    "ALTER TABLE agent_message
                         ADD COLUMN kind TEXT NOT NULL DEFAULT 'chat'
                         CHECK (kind IN ('chat', 'user_shell'));",
                )?;
            }
            if !column_exists(&transaction, "agent_message", "user_shell_json")? {
                transaction
                    .execute_batch("ALTER TABLE agent_message ADD COLUMN user_shell_json TEXT;")?;
            }
            ensure_no_foreign_key_violation(&transaction)?;
            // 只推进到 13：后续分支失败时不能让库看起来已是当前版本。
            transaction.pragma_update(None, "user_version", 13)?;
            transaction.commit()?;
            Ok(())
        })();
        self.conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        result
    }

    /// v5：重建 `workspace_prompt` 去 `status` 列（页表外键指向重建后的同名表）。
    ///
    /// `DROP TABLE` 在打开外键时会级联清空页表，因此重建必须在关闭外键的
    /// 独立事务内完成；失败不推进版本，下次打开会重试（结构幂等）。
    fn rebuild_prompt_without_status(&mut self) -> Result<()> {
        self.conn.execute_batch("PRAGMA foreign_keys = OFF;")?;
        let result = (|| -> Result<()> {
            let transaction = self.conn.transaction()?;
            transaction.execute_batch(SCHEMA_V5_PROMPT)?;
            ensure_no_foreign_key_violation(&transaction)?;
            transaction.commit()?;
            Ok(())
        })();
        self.conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        result
    }
}

/// v9 迁移主体：补 `completed_at_ms`、回填历史思考完成时刻、重建工具调用表。
///
/// 调用方已开启事务，本函数不再提交；任一步失败由外层回滚，`user_version` 与旧数据不变。
fn migrate_v9(transaction: &Connection) -> Result<()> {
    if !column_exists(transaction, "agent_message_part", "completed_at_ms")? {
        transaction
            .execute_batch("ALTER TABLE agent_message_part ADD COLUMN completed_at_ms INTEGER;")?;
    }
    // 已终态消息的思考块：以最后增量时刻为准，缺失时用消息完成时刻。
    // streaming 消息的思考块留给启动恢复路径（`recover_interrupted_streams`）补齐。
    transaction.execute_batch(
        r#"UPDATE agent_message_part
           SET completed_at_ms = COALESCE(
                   ended_at_ms,
                   (SELECT m.completed_at_ms FROM agent_message m
                    WHERE m.external_id = agent_message_part.message_id))
           WHERE kind = 'reasoning' AND completed_at_ms IS NULL
             AND message_id IN (SELECT external_id FROM agent_message WHERE status <> 'streaming');"#,
    )?;
    let before: i64 =
        transaction.query_row("SELECT COUNT(*) FROM agent_tool_call", [], |row| row.get(0))?;
    transaction.execute_batch(SCHEMA_V9_TOOL_CALL_CREATE)?;
    let after: i64 =
        transaction.query_row("SELECT COUNT(*) FROM agent_tool_call_v9", [], |row| {
            row.get(0)
        })?;
    if before != after {
        return Err(DbError::ForeignKeyViolation);
    }
    ensure_no_foreign_key_violation(transaction)?;
    transaction.execute_batch(SCHEMA_V9_TOOL_CALL_FINISH)?;
    Ok(())
}

/// 外键自检：重建/搬运后不允许悬空引用。
fn ensure_no_foreign_key_violation(connection: &Connection) -> Result<()> {
    let violations: i64 =
        connection.query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
            row.get(0)
        })?;
    if violations != 0 {
        return Err(DbError::ForeignKeyViolation);
    }
    Ok(())
}

/// `agent_session.model_provider` 是否仍带 NOT NULL（需要 v12 重建）。
///
/// 表不存在（全新库尚未建表）时返回 false：建表后由同一套列检查补齐。
fn session_model_not_null(connection: &Connection) -> Result<bool> {
    let mut statement = connection.prepare("PRAGMA table_info(agent_session)")?;
    let mut rows = statement.query_map([], |row| {
        Ok((row.get::<_, String>(1)?, row.get::<_, i64>(3)?))
    })?;
    Ok(rows.any(|row| row.is_ok_and(|(name, not_null)| name == "model_provider" && not_null == 1)))
}

/// 表是否已有指定列（`PRAGMA table_info`）；仅迁移期使用。
fn column_exists(connection: &Connection, table: &str, column: &str) -> Result<bool> {
    let mut statement = connection.prepare(&format!("PRAGMA table_info({table})"))?;
    let mut names = statement.query_map([], |row| row.get::<_, String>(1))?;
    Ok(names.any(|name| name.is_ok_and(|name| name == column)))
}

#[cfg(test)]
mod tests;
