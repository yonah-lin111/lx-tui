//! SQLite 各版本建表与重建 DDL 文本常量。

/// 建表语句（workspace 语义，对齐 lx-agent 的 external_id + 级联外键设计）。
pub(super) const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS workspace (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    external_id TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    "type" TEXT NOT NULL DEFAULT 'filesystem',
    path TEXT UNIQUE,
    is_trusted INTEGER NOT NULL DEFAULT 1,
    -- 全局展示顺序：活跃列表与历史列表共用，拖拽排序写回。
    position INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

-- 用户在历史列表显式删除、但磁盘上仍存在的 Git worktree 路径；重新登记/打开时解除。
CREATE TABLE IF NOT EXISTS workspace_hidden (
    path TEXT PRIMARY KEY,
    hidden_at TEXT NOT NULL
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
pub(super) const SCHEMA_V4: &str = r#"
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
pub(super) const SCHEMA_V5_PROMPT: &str = r#"
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
pub(super) const SCHEMA_V7: &str = r#"
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
pub(super) const SCHEMA_V9_TOOL_CALL_CREATE: &str = r#"
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
    COALESCE((SELECT MAX(p.position) FROM agent_message_part p WHERE p.message_id = t.message_id), -1)
        + ROW_NUMBER() OVER (PARTITION BY t.message_id ORDER BY t.started_at_ms, t.id),
    t.status, t.result_json, t.error_text, t.started_at_ms, t.completed_at_ms, t.duration_ms
FROM agent_tool_call t
ORDER BY t.id;
"#;

/// v9 工具调用表替换：删旧表、改名新表并重建索引。
pub(super) const SCHEMA_V9_TOOL_CALL_FINISH: &str = r#"
DROP TABLE agent_tool_call;

ALTER TABLE agent_tool_call_v9 RENAME TO agent_tool_call;

CREATE INDEX IF NOT EXISTS idx_agent_tool_call_session_started
    ON agent_tool_call(session_id, started_at_ms, id);
CREATE INDEX IF NOT EXISTS idx_agent_tool_call_session_message_position
    ON agent_tool_call(session_id, message_id, position);
"#;

/// v10 全局 prompt 历史表：只存输入框原文，`text` 唯一以支持去重置顶。
pub(super) const SCHEMA_V10: &str = r#"
CREATE TABLE IF NOT EXISTS prompt_history (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    text TEXT NOT NULL UNIQUE,
    updated_at_ms INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_prompt_history_updated
    ON prompt_history(updated_at_ms DESC, id DESC);
"#;

/// v11 会话进程元数据表：保存原 exec 工具返回后的进程变化。
pub(super) const SCHEMA_V11: &str = r#"
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
pub(super) const SCHEMA_V12: &str = r#"
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
pub(super) const SCHEMA_V13_SESSION: &str = r#"
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
pub(super) const SCHEMA_V14: &str = r#"
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
pub(super) const SCHEMA_V15_TODO: &str = r#"
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

/// v18：消息附件（文件/文件夹/图片可见引用元数据，图片本体存本地路径）。
pub(super) const SCHEMA_V18: &str = r#"
CREATE TABLE IF NOT EXISTS agent_message_attachment (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    external_id TEXT NOT NULL UNIQUE,
    message_id TEXT NOT NULL REFERENCES agent_message(external_id) ON DELETE CASCADE,
    position INTEGER NOT NULL CHECK (position >= 0),
    kind TEXT NOT NULL CHECK (kind IN ('file', 'folder', 'image')),
    label TEXT NOT NULL,
    marker_start INTEGER,
    marker_end INTEGER,
    source_path TEXT,
    mime TEXT,
    width INTEGER,
    height INTEGER,
    UNIQUE(message_id, position),
    CHECK ((marker_start IS NULL AND marker_end IS NULL) OR
           (marker_start >= 0 AND marker_end > marker_start)),
    CHECK (kind <> 'image' OR
           (mime IN ('image/png', 'image/jpeg') AND source_path IS NOT NULL
            AND width > 0 AND height > 0))
);
CREATE INDEX IF NOT EXISTS idx_agent_message_attachment_message
    ON agent_message_attachment(message_id, position);
"#;

/// v19：子 agent 输入队列、有序输入项与逐轮报告表。
pub(super) const SCHEMA_V19: &str = r#"
CREATE TABLE IF NOT EXISTS agent_subagent_input (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    external_id TEXT NOT NULL UNIQUE,
    agent_id TEXT NOT NULL REFERENCES agent_subagent(external_id) ON DELETE CASCADE,
    dedupe_key TEXT NOT NULL UNIQUE,
    parent_call_id TEXT,
    parent_message_id TEXT REFERENCES agent_message(external_id) ON DELETE SET NULL,
    position INTEGER NOT NULL,
    priority INTEGER NOT NULL DEFAULT 0,
    status TEXT NOT NULL CHECK (status IN ('queued','running','completed','failed','interrupted','cancelled')),
    child_user_message_id TEXT REFERENCES agent_message(external_id) ON DELETE SET NULL,
    created_at_ms INTEGER NOT NULL,
    started_at_ms INTEGER,
    completed_at_ms INTEGER,
    error_code TEXT,
    UNIQUE(agent_id, position)
);
CREATE INDEX IF NOT EXISTS idx_subagent_input_next
    ON agent_subagent_input(agent_id, status, priority DESC, position);

CREATE TABLE IF NOT EXISTS agent_subagent_input_item (
    input_id TEXT NOT NULL REFERENCES agent_subagent_input(external_id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('text','image')),
    text_content TEXT,
    label TEXT,
    archive_path TEXT,
    mime TEXT,
    content_hash TEXT,
    width INTEGER,
    height INTEGER,
    PRIMARY KEY(input_id, position)
);

CREATE TABLE IF NOT EXISTS agent_subagent_report (
    external_id TEXT PRIMARY KEY,
    agent_id TEXT NOT NULL REFERENCES agent_subagent(external_id) ON DELETE CASCADE,
    child_user_message_id TEXT REFERENCES agent_message(external_id) ON DELETE SET NULL,
    status TEXT NOT NULL CHECK (status IN ('completed','failed')),
    report_json TEXT NOT NULL,
    completed_at_ms INTEGER NOT NULL,
    delivered_at_ms INTEGER
);
CREATE INDEX IF NOT EXISTS idx_subagent_report_pending
    ON agent_subagent_report(agent_id, delivered_at_ms, completed_at_ms);
"#;

/// v20：附件表增加 `skill` kind（CHECK 约束无法 ALTER，必须整表重建）。
pub(super) const SCHEMA_V20_ATTACHMENT: &str = r#"
CREATE TABLE agent_message_attachment_v20 (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    external_id TEXT NOT NULL UNIQUE,
    message_id TEXT NOT NULL REFERENCES agent_message(external_id) ON DELETE CASCADE,
    position INTEGER NOT NULL CHECK (position >= 0),
    kind TEXT NOT NULL CHECK (kind IN ('file', 'folder', 'image', 'skill')),
    label TEXT NOT NULL,
    marker_start INTEGER,
    marker_end INTEGER,
    source_path TEXT,
    mime TEXT,
    width INTEGER,
    height INTEGER,
    UNIQUE(message_id, position),
    CHECK ((marker_start IS NULL AND marker_end IS NULL) OR
           (marker_start >= 0 AND marker_end > marker_start)),
    CHECK (kind <> 'image' OR
           (mime IN ('image/png', 'image/jpeg') AND source_path IS NOT NULL
            AND width > 0 AND height > 0))
);

INSERT INTO agent_message_attachment_v20
    (id, external_id, message_id, position, kind, label, marker_start, marker_end,
     source_path, mime, width, height)
    SELECT id, external_id, message_id, position, kind, label, marker_start, marker_end,
           source_path, mime, width, height
    FROM agent_message_attachment;

DROP TABLE agent_message_attachment;

ALTER TABLE agent_message_attachment_v20 RENAME TO agent_message_attachment;

CREATE INDEX IF NOT EXISTS idx_agent_message_attachment_message
    ON agent_message_attachment(message_id, position);
"#;

/// v21：todo 表索引重构——废弃全局 `idx_todo_position`，改为按日期 `(entry_date, position)` 唯一。
pub(super) const SCHEMA_V21_TODO_INDEXES: &str = r#"
DROP INDEX IF EXISTS idx_todo_position;
CREATE UNIQUE INDEX IF NOT EXISTS idx_todo_date_position ON todo(entry_date, position);
CREATE INDEX IF NOT EXISTS idx_todo_date_status ON todo(entry_date, status);
CREATE INDEX IF NOT EXISTS idx_todo_entry_date ON todo(entry_date);
"#;

/// v22：会话中途模型切换时间线与提示词注入记录表（与消息/notice/compact 共用 sequence）。
pub(super) const SCHEMA_V22_MODEL_SWITCH: &str = r#"
CREATE TABLE IF NOT EXISTS agent_model_switch (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    external_id TEXT NOT NULL UNIQUE,
    session_id TEXT NOT NULL REFERENCES agent_session(external_id) ON DELETE CASCADE,
    sequence INTEGER NOT NULL,
    from_provider TEXT,
    from_model TEXT,
    from_variant TEXT,
    to_provider TEXT NOT NULL,
    to_model TEXT NOT NULL,
    to_variant TEXT,
    from_label TEXT,
    to_label TEXT NOT NULL,
    updated_sources_json TEXT NOT NULL DEFAULT '[]',
    injected_prompt TEXT NOT NULL DEFAULT '',
    created_at_ms INTEGER NOT NULL,
    UNIQUE(session_id, sequence)
);
CREATE INDEX IF NOT EXISTS idx_agent_model_switch_session_seq
    ON agent_model_switch(session_id, sequence);
"#;

/// v24：历史列表隐藏路径（删除仍存在于磁盘的 worktree 后防止自动扫描复活）。
pub(super) const SCHEMA_V24: &str = r#"
CREATE TABLE IF NOT EXISTS workspace_hidden (
    path TEXT PRIMARY KEY,
    hidden_at TEXT NOT NULL
);
"#;
