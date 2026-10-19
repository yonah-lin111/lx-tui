//! Schema 迁移：v4 提示词分页、v5 去状态列、v7 聊天五表、v8 part 时刻、v9 工具时间线。
//!
//! 迁移与 `PRAGMA user_version` 推进同事务完成：任一步失败整体回滚，旧数据与版本号
//! 保持不变，下次打开重试。历史迁移文本（v4/v5/v7/v8）不得改写，只能在尾部追加新版本；
//! 迁移期不执行任何文件操作。

use rusqlite::{Connection, params};

use super::{Database, DbError, FIRST_PAGE_NAME, Result, SCHEMA_VERSION};

/// 建表语句（workspace 语义，对齐 lx-agent 的 external_id + 级联外键设计）。
const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS workspace (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    external_id TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    "type" TEXT NOT NULL DEFAULT 'filesystem',
    path TEXT UNIQUE,
    is_trusted INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS workspace_folder (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    external_id TEXT NOT NULL UNIQUE,
    workspace_id TEXT NOT NULL,
    parent_folder_id TEXT,
    name TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (workspace_id) REFERENCES workspace(external_id) ON DELETE CASCADE,
    FOREIGN KEY (parent_folder_id) REFERENCES workspace_folder(external_id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_workspace_folder_workspace_id ON workspace_folder(workspace_id);
CREATE INDEX IF NOT EXISTS idx_workspace_folder_parent ON workspace_folder(parent_folder_id);

CREATE TABLE IF NOT EXISTS workspace_prompt (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    external_id TEXT NOT NULL UNIQUE,
    workspace_id TEXT NOT NULL,
    folder_id TEXT,
    name TEXT NOT NULL,
    content TEXT NOT NULL DEFAULT '',
    active_page_id INTEGER,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (workspace_id) REFERENCES workspace(external_id) ON DELETE CASCADE,
    FOREIGN KEY (folder_id) REFERENCES workspace_folder(external_id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_workspace_prompt_workspace_id ON workspace_prompt(workspace_id);
CREATE INDEX IF NOT EXISTS idx_workspace_prompt_folder_id ON workspace_prompt(folder_id);

CREATE TABLE IF NOT EXISTS workspace_referenced_folder (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    workspace_id TEXT NOT NULL,
    path TEXT NOT NULL,
    created_at TEXT NOT NULL,
    FOREIGN KEY (workspace_id) REFERENCES workspace(external_id) ON DELETE CASCADE,
    UNIQUE (workspace_id, path)
);

CREATE INDEX IF NOT EXISTS idx_workspace_referenced_folder_workspace_id
    ON workspace_referenced_folder(workspace_id);
"#;

/// v4 分页建表语句：页表以 `(prompt_id, position)` 唯一约束保证创建顺序；
/// `workspace_prompt.content` 保留为第一页镜像（兼容既有读取者）。
const SCHEMA_V4: &str = r#"
CREATE TABLE IF NOT EXISTS workspace_prompt_page (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    prompt_id INTEGER NOT NULL,
    position INTEGER NOT NULL,
    name TEXT NOT NULL,
    content TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE (prompt_id, position),
    FOREIGN KEY (prompt_id) REFERENCES workspace_prompt(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_workspace_prompt_page_prompt_id ON workspace_prompt_page(prompt_id);
"#;

/// v5 迁移的提示词表重建语句：移除 `status` 列（SQLite 不支持带 CHECK 的 DROP COLUMN）。
/// 重建必须在关闭外键的独立事务中执行（见 `rebuild_prompt_without_status`）。
const SCHEMA_V5_PROMPT: &str = r#"
CREATE TABLE workspace_prompt_v5 (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    external_id TEXT NOT NULL UNIQUE,
    workspace_id TEXT NOT NULL,
    folder_id TEXT,
    name TEXT NOT NULL,
    content TEXT NOT NULL DEFAULT '',
    active_page_id INTEGER,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (workspace_id) REFERENCES workspace(external_id) ON DELETE CASCADE,
    FOREIGN KEY (folder_id) REFERENCES workspace_folder(external_id) ON DELETE CASCADE
);

INSERT INTO workspace_prompt_v5 (id, external_id, workspace_id, folder_id, name, content, active_page_id, created_at, updated_at)
    SELECT id, external_id, workspace_id, folder_id, name, content, active_page_id, created_at, updated_at
    FROM workspace_prompt;

DROP TABLE workspace_prompt;

ALTER TABLE workspace_prompt_v5 RENAME TO workspace_prompt;

CREATE INDEX IF NOT EXISTS idx_workspace_prompt_workspace_id ON workspace_prompt(workspace_id);
CREATE INDEX IF NOT EXISTS idx_workspace_prompt_folder_id ON workspace_prompt(folder_id);
"#;

/// v7 聊天会话建表语句：会话、消息、内容块、工具调用与子 Agent 关系五表。
///
/// - 内部 `id` 与稳定 `external_id` 双键：`id` 只作本地排序，跨表外键引用
///   `external_id`（对齐 workspace 表约定），避免易变的 `PaneId` 进库。
/// - 时间统一 UTC 毫秒整数；同毫秒排序交给 `id`。
/// - 工具调用与子 Agent 表本期只建结构、不写行；预建覆盖 Codex item 的
///   名称、参数、状态与结果语义。
const SCHEMA_V7: &str = r#"
CREATE TABLE IF NOT EXISTS agent_session (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    external_id TEXT NOT NULL UNIQUE,
    workspace_id TEXT NOT NULL REFERENCES workspace(external_id) ON DELETE CASCADE,
    source TEXT NOT NULL DEFAULT 'native' CHECK (source IN ('native', 'subagent')),
    title TEXT NOT NULL,
    first_user_message TEXT NOT NULL DEFAULT '',
    model_provider TEXT NOT NULL,
    model_id TEXT NOT NULL,
    variant TEXT,
    status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'closed')),
    archived INTEGER NOT NULL DEFAULT 0 CHECK (archived IN (0, 1)),
    agent_nickname TEXT,
    agent_role TEXT,
    agent_path TEXT,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    last_message_at_ms INTEGER NOT NULL,
    total_input_tokens INTEGER NOT NULL DEFAULT 0 CHECK (total_input_tokens >= 0),
    total_output_tokens INTEGER NOT NULL DEFAULT 0 CHECK (total_output_tokens >= 0)
);
CREATE INDEX IF NOT EXISTS idx_agent_session_workspace_recent
    ON agent_session(workspace_id, archived, updated_at_ms DESC, id DESC);

CREATE TABLE IF NOT EXISTS agent_message (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    external_id TEXT NOT NULL UNIQUE,
    session_id TEXT NOT NULL REFERENCES agent_session(external_id) ON DELETE CASCADE,
    sequence INTEGER NOT NULL CHECK (sequence >= 0),
    turn_index INTEGER NOT NULL CHECK (turn_index >= 0),
    role TEXT NOT NULL CHECK (role IN ('user', 'assistant')),
    status TEXT NOT NULL CHECK (status IN ('complete', 'streaming', 'interrupted', 'failed')),
    model_provider TEXT,
    model_id TEXT,
    variant TEXT,
    stop_reason TEXT,
    input_tokens INTEGER CHECK (input_tokens >= 0),
    output_tokens INTEGER CHECK (output_tokens >= 0),
    cache_read_tokens INTEGER CHECK (cache_read_tokens >= 0),
    cache_write_tokens INTEGER CHECK (cache_write_tokens >= 0),
    error_code TEXT,
    created_at_ms INTEGER NOT NULL,
    completed_at_ms INTEGER,
    UNIQUE(session_id, sequence)
);
CREATE INDEX IF NOT EXISTS idx_agent_message_session_sequence
    ON agent_message(session_id, sequence);

CREATE TABLE IF NOT EXISTS agent_message_part (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    external_id TEXT NOT NULL UNIQUE,
    message_id TEXT NOT NULL REFERENCES agent_message(external_id) ON DELETE CASCADE,
    position INTEGER NOT NULL CHECK (position >= 0),
    kind TEXT NOT NULL CHECK (kind IN ('text', 'reasoning')),
    content TEXT NOT NULL DEFAULT '',
    provider_metadata_json TEXT,
    created_at_ms INTEGER NOT NULL,
    started_at_ms INTEGER,
    ended_at_ms INTEGER,
    UNIQUE(message_id, position)
);

CREATE TABLE IF NOT EXISTS agent_tool_call (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    external_id TEXT NOT NULL UNIQUE,
    session_id TEXT NOT NULL REFERENCES agent_session(external_id) ON DELETE CASCADE,
    message_id TEXT NOT NULL REFERENCES agent_message(external_id) ON DELETE CASCADE,
    call_id TEXT NOT NULL,
    namespace TEXT,
    name TEXT NOT NULL,
    arguments_json TEXT NOT NULL DEFAULT '{}',
    status TEXT NOT NULL CHECK (status IN ('pending', 'running', 'completed', 'failed', 'cancelled')),
    result_json TEXT,
    error_text TEXT,
    started_at_ms INTEGER NOT NULL,
    completed_at_ms INTEGER,
    duration_ms INTEGER CHECK (duration_ms >= 0),
    UNIQUE(session_id, call_id)
);
CREATE INDEX IF NOT EXISTS idx_agent_tool_call_session_started
    ON agent_tool_call(session_id, started_at_ms, id);

CREATE TABLE IF NOT EXISTS agent_subagent (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    external_id TEXT NOT NULL UNIQUE,
    parent_session_id TEXT NOT NULL REFERENCES agent_session(external_id) ON DELETE CASCADE,
    child_session_id TEXT NOT NULL UNIQUE REFERENCES agent_session(external_id) ON DELETE CASCADE,
    spawn_message_id TEXT REFERENCES agent_message(external_id) ON DELETE SET NULL,
    status TEXT NOT NULL CHECK (status IN ('pending', 'running', 'blocked', 'completed', 'failed', 'cancelled')),
    nickname TEXT,
    role TEXT,
    agent_path TEXT,
    model_provider TEXT,
    model_id TEXT,
    variant TEXT,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    completed_at_ms INTEGER,
    CHECK (parent_session_id <> child_session_id)
);
CREATE INDEX IF NOT EXISTS idx_agent_subagent_parent_status
    ON agent_subagent(parent_session_id, status);
"#;

/// v9 工具调用表重建：新表完整复制 v8 全部列，新增时间线 `position` 与 `interrupted` 终态。
///
/// SQLite 无法修改 CHECK 约束，只能整表重建；重建期间数据先写新表、核对行数与
/// 外键后再删旧表。旧行 position 从该消息 `agent_message_part.MAX(position)+1` 起，
/// 同消息内按 `(started_at_ms, id)` 稳定排序递增，保证不冲突且顺序可回放。
const SCHEMA_V9_TOOL_CALL_CREATE: &str = r#"
DROP TABLE IF EXISTS agent_tool_call_v9;

CREATE TABLE agent_tool_call_v9 (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    external_id TEXT NOT NULL UNIQUE,
    session_id TEXT NOT NULL REFERENCES agent_session(external_id) ON DELETE CASCADE,
    message_id TEXT NOT NULL REFERENCES agent_message(external_id) ON DELETE CASCADE,
    call_id TEXT NOT NULL,
    namespace TEXT,
    name TEXT NOT NULL,
    arguments_json TEXT NOT NULL DEFAULT '{}',
    position INTEGER NOT NULL CHECK (position >= 0),
    status TEXT NOT NULL CHECK
        (status IN ('pending', 'running', 'completed', 'failed', 'cancelled', 'interrupted')),
    result_json TEXT,
    error_text TEXT,
    started_at_ms INTEGER NOT NULL,
    completed_at_ms INTEGER,
    duration_ms INTEGER CHECK (duration_ms >= 0),
    UNIQUE(session_id, call_id),
    UNIQUE(message_id, position)
);

INSERT INTO agent_tool_call_v9
    (id, external_id, session_id, message_id, call_id, namespace, name, arguments_json,
     position, status, result_json, error_text, started_at_ms, completed_at_ms, duration_ms)
SELECT
    t.id, t.external_id, t.session_id, t.message_id, t.call_id, t.namespace, t.name,
    t.arguments_json,
    -- 起点：该消息既有 part 的 MAX(position) + 1（无 part 则 0）；
    -- 序号按 (started_at_ms, id) 稳定递增，保证同消息内不冲突且顺序可回放。
    COALESCE((SELECT MAX(p.position) FROM agent_message_part p WHERE p.message_id = t.message_id), -1)
        + ROW_NUMBER() OVER (PARTITION BY t.message_id ORDER BY t.started_at_ms, t.id),
    t.status, t.result_json, t.error_text, t.started_at_ms, t.completed_at_ms, t.duration_ms
FROM agent_tool_call t
ORDER BY t.id;
"#;

/// v9 工具调用表替换：删旧表、改名新表并重建索引。
const SCHEMA_V9_TOOL_CALL_FINISH: &str = r#"
DROP TABLE agent_tool_call;

ALTER TABLE agent_tool_call_v9 RENAME TO agent_tool_call;

CREATE INDEX IF NOT EXISTS idx_agent_tool_call_session_started
    ON agent_tool_call(session_id, started_at_ms, id);
CREATE INDEX IF NOT EXISTS idx_agent_tool_call_session_message_position
    ON agent_tool_call(session_id, message_id, position);
"#;

/// v10 全局 prompt 历史表：只存输入框原文，`text` 唯一以支持去重置顶。
///
/// 不按 workspace/session 分区：历史是全局共享的（跨窗格、跨工作区、跨会话）。
/// `updated_at_ms` 表示最近使用时刻，裁剪与加载都按它倒序；同毫秒排序交给 `id`。
const SCHEMA_V10: &str = r#"
CREATE TABLE IF NOT EXISTS prompt_history (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    text TEXT NOT NULL UNIQUE,
    updated_at_ms INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_prompt_history_updated
    ON prompt_history(updated_at_ms DESC, id DESC);
"#;

/// v11 会话进程元数据表：保存原 exec 工具返回后的进程变化。
///
/// 工具原始结果（`agent_tool_call.result_json`）**不可变**；进程后续输出与退出写在
/// 这里，二者不互相改写。`process_session_id` 是运行实例内身份，不接受 OS PID 作为
/// 恢复入口；`revision` 用单调整数分配，较旧 revision 与不同运行代次的更新零行。
const SCHEMA_V11: &str = r#"
CREATE TABLE IF NOT EXISTS agent_process (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    external_id TEXT NOT NULL UNIQUE,
    chat_session_id TEXT NOT NULL REFERENCES agent_session(external_id) ON DELETE CASCADE,
    origin_call_id TEXT NOT NULL,
    runtime_epoch TEXT NOT NULL,
    process_session_id INTEGER NOT NULL CHECK (process_session_id > 0),
    state TEXT NOT NULL CHECK
        (state IN ('starting', 'running', 'exited', 'stopped', 'timed_out', 'interrupted', 'failed')),
    exit_code INTEGER,
    stop_reason TEXT,
    command TEXT NOT NULL,
    cwd TEXT NOT NULL,
    tty INTEGER NOT NULL DEFAULT 0 CHECK (tty IN (0, 1)),
    output_tail BLOB,
    output_start INTEGER NOT NULL DEFAULT 0 CHECK (output_start >= 0),
    output_end INTEGER NOT NULL DEFAULT 0 CHECK (output_end >= 0),
    dropped_bytes INTEGER NOT NULL DEFAULT 0 CHECK (dropped_bytes >= 0),
    started_at_ms INTEGER NOT NULL,
    ended_at_ms INTEGER,
    revision INTEGER NOT NULL DEFAULT 0 CHECK (revision >= 0),
    UNIQUE(runtime_epoch, process_session_id)
);

CREATE INDEX IF NOT EXISTS idx_agent_process_session
    ON agent_process(chat_session_id, started_at_ms, id);
"#;

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
            // v10：全局 prompt 历史表；只增表，老数据不受影响。
            if version < 10 {
                transaction.execute_batch(SCHEMA_V10)?;
            }
            // v11：会话进程元数据表；只增表，工具结果体仍不可变。
            if version < 11 {
                transaction.execute_batch(SCHEMA_V11)?;
            }
            transaction.pragma_update(None, "user_version", SCHEMA_VERSION)?;
            transaction.commit()?;
        }
        Ok(())
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

/// 表是否已有指定列（`PRAGMA table_info`）；仅迁移期使用。
fn column_exists(connection: &Connection, table: &str, column: &str) -> Result<bool> {
    let mut statement = connection.prepare(&format!("PRAGMA table_info({table})"))?;
    let mut names = statement.query_map([], |row| row.get::<_, String>(1))?;
    Ok(names.any(|name| name.is_ok_and(|name| name == column)))
}

#[cfg(test)]
mod tests;
