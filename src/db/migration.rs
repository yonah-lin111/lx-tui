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

/// v12 最新轮次撤销：会话 revision、独立提示、轮次文件 scope、镜像、mutation 与操作日志。
///
/// `user_message_id` 是稳定业务键：journal 与 scope 不随目标消息删除级联消失，
/// 因此只有 session 外键；提示也不外键到 message（撤销自身不能把 tip 删掉）。
/// 镜像以原始 BLOB 保存，不区分文本/二进制，也不依赖 diff 正文。
const SCHEMA_V12: &str = r#"
CREATE TABLE IF NOT EXISTS agent_session_notice (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL REFERENCES agent_session(external_id) ON DELETE CASCADE,
    sequence INTEGER NOT NULL CHECK (sequence >= 0),
    operation_id TEXT NOT NULL UNIQUE,
    kind TEXT NOT NULL CHECK (kind IN ('revert_and_delete', 'delete_only', 'revert_only')),
    target_user_message_id TEXT NOT NULL,
    restored_file_count INTEGER NOT NULL DEFAULT 0 CHECK (restored_file_count >= 0),
    files_already_restored INTEGER NOT NULL DEFAULT 0 CHECK (files_already_restored IN (0, 1)),
    created_at_ms INTEGER NOT NULL,
    UNIQUE(session_id, sequence)
);
CREATE INDEX IF NOT EXISTS idx_agent_session_notice_session
    ON agent_session_notice(session_id, sequence);

CREATE TABLE IF NOT EXISTS agent_turn_file_scope (
    session_id TEXT NOT NULL REFERENCES agent_session(external_id) ON DELETE CASCADE,
    user_message_id TEXT NOT NULL UNIQUE,
    capture_version INTEGER NOT NULL CHECK (capture_version >= 0),
    state TEXT NOT NULL CHECK (state IN ('capturing', 'complete', 'incomplete')),
    snapshot_bytes INTEGER NOT NULL DEFAULT 0 CHECK (snapshot_bytes >= 0),
    files_restored INTEGER NOT NULL DEFAULT 0 CHECK (files_restored IN (0, 1)),
    created_at_ms INTEGER NOT NULL,
    PRIMARY KEY (session_id, user_message_id)
);
CREATE INDEX IF NOT EXISTS idx_agent_turn_file_scope_session
    ON agent_turn_file_scope(session_id, created_at_ms);

CREATE TABLE IF NOT EXISTS agent_file_image (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    external_id TEXT NOT NULL UNIQUE,
    session_id TEXT NOT NULL REFERENCES agent_session(external_id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK (kind IN ('missing', 'regular', 'symlink')),
    bytes BLOB,
    mode INTEGER,
    link_target TEXT,
    git_oid TEXT,
    worktree_root TEXT,
    git_dir TEXT,
    head_ref TEXT,
    head_oid TEXT,
    size INTEGER NOT NULL DEFAULT 0 CHECK (size >= 0),
    created_at_ms INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_agent_file_image_session
    ON agent_file_image(session_id, id);

CREATE TABLE IF NOT EXISTS agent_file_mutation (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    external_id TEXT NOT NULL UNIQUE,
    session_id TEXT NOT NULL REFERENCES agent_session(external_id) ON DELETE CASCADE,
    user_message_id TEXT NOT NULL,
    tool_call_id TEXT NOT NULL,
    ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
    operation TEXT NOT NULL CHECK (operation IN
        ('create', 'update', 'delete', 'move_src', 'move_dst')),
    path TEXT NOT NULL,
    target_path TEXT NOT NULL,
    anchor_path TEXT,
    anchor_canonical TEXT,
    before_image_id INTEGER NOT NULL,
    planned_after_image_id INTEGER NOT NULL,
    actual_after_image_id INTEGER,
    worktree_root TEXT,
    git_dir TEXT,
    head_ref TEXT,
    head_oid TEXT,
    state TEXT NOT NULL CHECK (state IN ('prepared', 'committed')),
    success INTEGER NOT NULL DEFAULT 1 CHECK (success IN (0, 1)),
    created_at_ms INTEGER NOT NULL,
    committed_at_ms INTEGER
);
CREATE INDEX IF NOT EXISTS idx_agent_file_mutation_turn
    ON agent_file_mutation(session_id, user_message_id, id);

CREATE TABLE IF NOT EXISTS agent_rewind_operation (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    external_id TEXT NOT NULL UNIQUE,
    operation_id TEXT NOT NULL UNIQUE,
    session_id TEXT NOT NULL REFERENCES agent_session(external_id) ON DELETE CASCADE,
    action TEXT NOT NULL,
    target_user_message_id TEXT NOT NULL,
    expected_revision INTEGER NOT NULL CHECK (expected_revision >= 0),
    phase TEXT NOT NULL,
    error_code TEXT,
    user_text TEXT NOT NULL DEFAULT '',
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_agent_rewind_operation_session
    ON agent_rewind_operation(session_id, created_at_ms);
"#;

/// v13：会话模型列改为可空（纯本地命令会话无模型），消息增加 kind 与命令记录。
///
/// SQLite 无法移除 NOT NULL，只能整表重建；与 v5 一样在关闭外键的独立事务中完成，
/// 保留主键、external ID、revision、索引与全部外键数据。消息列用 `ALTER TABLE` 追加。
const SCHEMA_V13_SESSION: &str = r#"
DROP TABLE IF EXISTS agent_session_v13;

CREATE TABLE agent_session_v13 (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    external_id TEXT NOT NULL UNIQUE,
    workspace_id TEXT NOT NULL REFERENCES workspace(external_id) ON DELETE CASCADE,
    source TEXT NOT NULL DEFAULT 'native' CHECK (source IN ('native', 'subagent')),
    title TEXT NOT NULL,
    first_user_message TEXT NOT NULL DEFAULT '',
    model_provider TEXT,
    model_id TEXT,
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
    total_output_tokens INTEGER NOT NULL DEFAULT 0 CHECK (total_output_tokens >= 0),
    revision INTEGER NOT NULL DEFAULT 0 CHECK (revision >= 0)
);

INSERT INTO agent_session_v13
    (id, external_id, workspace_id, source, title, first_user_message,
     model_provider, model_id, variant, status, archived, agent_nickname, agent_role,
     agent_path, created_at_ms, updated_at_ms, last_message_at_ms,
     total_input_tokens, total_output_tokens, revision)
SELECT id, external_id, workspace_id, source, title, first_user_message,
       model_provider, model_id, variant, status, archived, agent_nickname, agent_role,
       agent_path, created_at_ms, updated_at_ms, last_message_at_ms,
       total_input_tokens, total_output_tokens, revision
FROM agent_session;

DROP TABLE agent_session;

ALTER TABLE agent_session_v13 RENAME TO agent_session;

CREATE INDEX IF NOT EXISTS idx_agent_session_workspace_recent
    ON agent_session(workspace_id, archived, updated_at_ms DESC, id DESC);
"#;

/// v14：上下文压缩检查点（`/compact`）与撤销关联。
///
/// 独立表而不是复用 `agent_session_notice`：notice 是“撤销提示”（kind 与文件计数语义），
/// 检查点需要保存摘要正文、模型标签、覆盖范围与生效状态，且可被撤销/恢复多次。
/// `sequence` 与消息、撤销提示共用同一编号空间（时间线合并的前提）；`operation_id`
/// 只与 `agent_rewind_operation` 关联，不建立平行的撤销系统。
const SCHEMA_V14: &str = r#"
CREATE TABLE IF NOT EXISTS agent_compact_checkpoint (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    external_id TEXT NOT NULL UNIQUE,
    session_id TEXT NOT NULL REFERENCES agent_session(external_id) ON DELETE CASCADE,
    sequence INTEGER NOT NULL CHECK (sequence >= 0),
    operation_id TEXT NOT NULL UNIQUE,
    base_checkpoint_id TEXT,
    start_sequence INTEGER NOT NULL CHECK (start_sequence >= 0),
    end_sequence INTEGER NOT NULL CHECK (end_sequence >= 0),
    summary_text TEXT NOT NULL,
    model_label TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('active', 'reverted')),
    reverted_by_operation_id TEXT,
    created_at_ms INTEGER NOT NULL,
    UNIQUE(session_id, sequence),
    CHECK (end_sequence >= start_sequence)
);

CREATE INDEX IF NOT EXISTS idx_agent_compact_checkpoint_session
    ON agent_compact_checkpoint(session_id, sequence);

CREATE INDEX IF NOT EXISTS idx_agent_compact_checkpoint_state
    ON agent_compact_checkpoint(session_id, state, id);
"#;

/// v15 全局 todo 表：不按工作区/会话分区（同 `prompt_history` 的全局口径）。
///
/// `position` 为密集递增排序位，插入时由应用层在单事务内顺移（见 `db::todo`）；
/// `id` 为稳定业务键，UPSERT 与删除都以它为准。
/// `position` 上的唯一索引是顺移正确性的最后一道闸门：漏顺移会直接报错，
/// 而不是悄悄出现两条同位置条目。
const SCHEMA_V15_TODO: &str = r#"
CREATE TABLE IF NOT EXISTS todo (
    id TEXT PRIMARY KEY,
    text TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'todo' CHECK (status IN ('todo', 'in_progress', 'done')),
    position INTEGER NOT NULL CHECK (position >= 0),
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_todo_position ON todo(position);
"#;

impl Database {
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
            // v12：最新轮次撤销的 journal、镜像、操作日志与独立提示。
            if version < 12 && !column_exists(&transaction, "agent_session", "revision")? {
                transaction.execute_batch(
                    "ALTER TABLE agent_session ADD COLUMN revision INTEGER NOT NULL DEFAULT 0
                     CHECK (revision >= 0);",
                )?;
            }
            // 合并前另一分支曾把 v12 指向另一套迁移（无这些表）。停在 12 的旧库不会走
            // `version < 12` 分支，后续启动恢复会因缺表整体失败；这里无条件补齐。
            // 语句全部是 `CREATE ... IF NOT EXISTS`，重复执行幂等。
            transaction.execute_batch(SCHEMA_V12)?;
            // v13 需要关闭外键重建表，不能在本事务内完成：这里只推进到 v12。
            transaction.pragma_update(None, "user_version", 12)?;
            transaction.commit()?;
        }
        // v12 发布前补列：已经停在 v12 的开发库也要拿到路径锚点列（不推进版本）。
        if !column_exists(&self.conn, "agent_file_mutation", "anchor_path")? {
            let transaction = self.conn.unchecked_transaction()?;
            transaction.execute_batch(
                "ALTER TABLE agent_file_mutation ADD COLUMN anchor_path TEXT;
                 ALTER TABLE agent_file_mutation ADD COLUMN anchor_canonical TEXT;",
            )?;
            transaction.commit()?;
        }
        if version < 13 {
            self.migrate_v13()?;
        }
        if version < 14 {
            // 只增表：不触碰既有数据，失败整体回滚（版本不推进）。
            let transaction = self.conn.unchecked_transaction()?;
            transaction.execute_batch(SCHEMA_V14)?;
            ensure_no_foreign_key_violation(&transaction)?;
            transaction.pragma_update(None, "user_version", SCHEMA_VERSION)?;
            transaction.commit()?;
        }
        if version < 15 {
            // 只增表：全局 todo 列表，不触碰既有数据，失败整体回滚（版本不推进）。
            let transaction = self.conn.unchecked_transaction()?;
            transaction.execute_batch(SCHEMA_V15_TODO)?;
            ensure_no_foreign_key_violation(&transaction)?;
            transaction.pragma_update(None, "user_version", SCHEMA_VERSION)?;
            transaction.commit()?;
        }
        Ok(())
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
            transaction.pragma_update(None, "user_version", SCHEMA_VERSION)?;
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
