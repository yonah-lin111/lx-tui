//! v9 迁移测试：v7/v8 旧库升级、数据保留、回填口径、幂等与失败回滚。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use super::*;
use crate::db::chat::{ChatPart, ModelSnapshot, TurnOutcome, Usage};
use crate::db::{DbError, SCHEMA_VERSION, now_ms};

fn temp_db_path(label: &str) -> PathBuf {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or(0);
    let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir()
        .join(format!("lx-tui-v9-test-{label}-{nanos:x}-{sequence:x}"))
        .join("lx.db")
}

fn workspace_path(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!("lx-tui-v9-ws-{label}"))
}

fn model() -> ModelSnapshot {
    ModelSnapshot {
        provider: "example".into(),
        model: "example-chat".into(),
        variant: None,
    }
}

/// v8 的 `agent_tool_call` 与 `agent_message_part` 定义（无 position / completed_at_ms）。
const V8_TOOL_CALL: &str = r#"
DROP TABLE IF EXISTS agent_tool_call;
CREATE TABLE agent_tool_call (
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
CREATE INDEX idx_agent_tool_call_session_started
    ON agent_tool_call(session_id, started_at_ms, id);
"#;

const V8_MESSAGE_PART: &str = r#"
DROP TABLE IF EXISTS agent_message_part;
CREATE TABLE agent_message_part (
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
"#;

/// 构造 v8 现场：完整库建好后把两张表换回 v8 定义并回退版本号，同时写入旧数据。
///
/// 返回（库路径，工作区）。旧数据含：
/// - 一条已终态 assistant 消息，其 reasoning part 有 `ended_at_ms`；
/// - 一条 streaming assistant 消息，其 reasoning part 无 `ended_at_ms`；
/// - 该终态消息上两条旧工具行（started_at_ms 逆序插入，验证按时间稳定排序）。
fn seed_v8_database(label: &str) -> (PathBuf, PathBuf) {
    let path = temp_db_path(label);
    let workspace = workspace_path(label);
    std::fs::create_dir_all(path.parent().expect("parent")).expect("dir");
    {
        let database = Database::open(&path).expect("open to v9");
        database.trust_workspace(&workspace, "ws").expect("trust");
        database
            .conn
            .execute_batch(V8_TOOL_CALL)
            .expect("v8 tool call");
        database
            .conn
            .execute_batch(V8_MESSAGE_PART)
            .expect("v8 message part");
        // 内部 id 直接指定，便于稳定断言。
        database
            .conn
            .execute_batch(
                r#"INSERT INTO agent_session
                       (external_id, workspace_id, title, first_user_message, model_provider,
                        model_id, created_at_ms, updated_at_ms, last_message_at_ms)
                   VALUES ('sess-1', (SELECT external_id FROM workspace WHERE path IS NOT NULL),
                           'v8 title', '问', 'p', 'm', 1, 2, 2);
                   INSERT INTO agent_message
                       (external_id, session_id, sequence, turn_index, role, status,
                        model_provider, model_id, created_at_ms, completed_at_ms)
                   VALUES ('msg-user', 'sess-1', 0, 0, 'user', 'complete', NULL, NULL, 10, 10),
                          ('msg-done', 'sess-1', 1, 0, 'assistant', 'complete', 'p', 'm', 20, 90),
                          ('msg-stream', 'sess-1', 2, 1, 'assistant', 'streaming', 'p', 'm', 30, NULL);
                   INSERT INTO agent_message_part
                       (external_id, message_id, position, kind, content, created_at_ms,
                        started_at_ms, ended_at_ms)
                   VALUES ('part-done-reasoning', 'msg-done', 0, 'reasoning', '旧思考', 20, 21, 55),
                          ('part-done-text', 'msg-done', 1, 'text', '旧正文', 20, NULL, NULL),
                          ('part-stream-reasoning', 'msg-stream', 0, 'reasoning', '半截', 30, 31, NULL);
                   INSERT INTO agent_tool_call
                       (external_id, session_id, message_id, call_id, namespace, name,
                        arguments_json, status, result_json, error_text, started_at_ms,
                        completed_at_ms, duration_ms)
                   VALUES ('tc-late', 'sess-1', 'msg-done', 'call-late', NULL, 'read',
                           '{"path":"b"}', 'completed', '{"content":"b"}', NULL, 80, 85, 5),
                          ('tc-early', 'sess-1', 'msg-done', 'call-early', 'ns', 'search',
                           '{"pattern":"x"}', 'pending', NULL, NULL, 40, NULL, NULL);
                   PRAGMA user_version = 8;"#,
            )
            .expect("seed v8 data");
    }
    (path, workspace)
}

fn columns(database: &Database, table: &str) -> Vec<String> {
    let mut statement = database
        .conn
        .prepare(&format!("SELECT name FROM pragma_table_info('{table}')"))
        .expect("prepare");
    let rows = statement
        .query_map([], |row| row.get::<_, String>(0))
        .expect("query");
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .expect("collect")
}

#[test]
fn v8_database_upgrades_to_v9_keeping_all_rows() {
    let (path, workspace) = seed_v8_database("upgrade");
    let database = Database::open(&path).expect("v8 upgrades to v9");
    assert_eq!(
        database
            .conn
            .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .expect("version"),
        SCHEMA_VERSION
    );
    let part_columns = columns(&database, "agent_message_part");
    assert!(
        part_columns.contains(&"completed_at_ms".to_string()),
        "{part_columns:?}"
    );
    let tool_columns = columns(&database, "agent_tool_call");
    for column in [
        "position",
        "namespace",
        "arguments_json",
        "result_json",
        "error_text",
        "duration_ms",
        "completed_at_ms",
        "started_at_ms",
    ] {
        assert!(
            tool_columns.contains(&column.to_string()),
            "缺列 {column}: {tool_columns:?}"
        );
    }
    // 旧会话/消息/part/工具行全部保留。
    let messages = database
        .load_chat_session(&workspace, "sess-1")
        .expect("load")
        .expect("session exists");
    assert_eq!(messages.len(), 3);
    assert_eq!(messages[1].role, "assistant");
    assert_eq!(messages[1].status, "complete");
    assert_eq!(messages[1].parts.len(), 2);
    assert_eq!(messages[1].parts[0].content, "旧思考");
    assert_eq!(messages[1].parts[1].content, "旧正文");
    assert_eq!(messages[1].tools.len(), 2, "旧工具行保留");
    // 位置：part 占 0、1；工具按 (started_at_ms, id) 排在后面。
    let positions: Vec<(String, i64)> = messages[1]
        .tools
        .iter()
        .map(|call| (call.call_id.clone(), call.position))
        .collect();
    assert_eq!(
        positions,
        vec![("call-early".into(), 2), ("call-late".into(), 3)]
    );
    // 工具原有字段不丢。
    let late = messages[1]
        .tools
        .iter()
        .find(|call| call.call_id == "call-late")
        .expect("late");
    assert_eq!(late.name, "read");
    assert_eq!(late.arguments_json, r#"{"path":"b"}"#);
    assert_eq!(late.result_json.as_deref(), Some(r#"{"content":"b"}"#));
    assert_eq!(late.status, "completed");
    assert_eq!(late.duration_ms, Some(5));
    assert_eq!(late.completed_at_ms, Some(85));
    let early = messages[1]
        .tools
        .iter()
        .find(|call| call.call_id == "call-early")
        .expect("early");
    assert_eq!(early.status, "pending");
    // namespace 等 v8 列完整搬运（记录类型不暴露 namespace，直接查库核对）。
    let namespace: Option<String> = database
        .conn
        .query_row(
            "SELECT namespace FROM agent_tool_call WHERE call_id = 'call-early'",
            [],
            |row| row.get(0),
        )
        .expect("namespace");
    assert_eq!(namespace.as_deref(), Some("ns"));
    // 终态消息的 reasoning 完成时刻 = ended_at_ms。
    assert_eq!(messages[1].parts[0].completed_at_ms, Some(55));
    assert_eq!(messages[1].parts[1].completed_at_ms, None, "text 不回填");
    // streaming 消息的 reasoning 留待恢复路径。
    assert_eq!(messages[2].parts[0].completed_at_ms, None);
    // 外键干净、索引重建。
    let violations: i64 = database
        .conn
        .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
            row.get(0)
        })
        .expect("fk check");
    assert_eq!(violations, 0);
    let index: i64 = database
        .conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' AND name = 'idx_agent_tool_call_session_started'",
            [],
            |row| row.get(0),
        )
        .expect("index query");
    assert_eq!(index, 1);
    // 重开幂等。
    drop(database);
    let reopened = Database::open(&path).expect("reopen");
    assert_eq!(
        reopened
            .conn
            .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .expect("version"),
        SCHEMA_VERSION
    );
    assert_eq!(
        reopened
            .load_tool_calls_for_session(&workspace, "sess-1")
            .expect("load tools")
            .len(),
        2
    );
}

#[test]
fn v7_database_upgrades_to_v9_with_full_schema() {
    let path = temp_db_path("v7-upgrade");
    let workspace = workspace_path("v7-upgrade");
    std::fs::create_dir_all(path.parent().expect("parent")).expect("dir");
    {
        // 完整库建好后删掉 v9 才有的结构并回退版本号到 7。
        let database = Database::open(&path).expect("open");
        database.trust_workspace(&workspace, "ws").expect("trust");
        database
            .conn
            .execute_batch(&format!(
                "PRAGMA foreign_keys = OFF;{} {V8_MESSAGE_PART} {V8_TOOL_CALL}
                 PRAGMA foreign_keys = ON; PRAGMA user_version = 7;",
                ""
            ))
            .expect("back to v7");
    }
    let database = Database::open(&path).expect("v7 upgrades to v9");
    assert_eq!(
        database
            .conn
            .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .expect("version"),
        SCHEMA_VERSION
    );
    assert!(columns(&database, "agent_message_part").contains(&"completed_at_ms".to_string()));
    assert!(columns(&database, "agent_tool_call").contains(&"position".to_string()));
    // v9 结构可用：迁移后可以正常落一行工具。
    let committed = database
        .begin_turn(&workspace, None, "hi", &model())
        .expect("begin");
    database
        .begin_tool_call(
            &workspace,
            &committed.session_id,
            &committed.assistant_message_id,
            "call-1",
            "read",
            "{}",
        )
        .expect("tool call works after upgrade");
}

#[test]
fn interrupted_status_is_accepted_after_migration() {
    let (path, workspace) = seed_v8_database("interrupted");
    let database = Database::open(&path).expect("upgrade");
    let calls = database
        .load_tool_calls_for_session(&workspace, "sess-1")
        .expect("load");
    assert_eq!(calls.len(), 2);
    // v8 的 pending 行在启动恢复后变成 interrupted（新 CHECK 允许写入）。
    assert_eq!(
        database.recover_interrupted_tool_calls().expect("recover"),
        1
    );
    let calls = database
        .load_tool_calls_for_session(&workspace, "sess-1")
        .expect("load");
    let early = calls
        .iter()
        .find(|call| call.call_id == "call-early")
        .expect("early");
    assert_eq!(early.status, "interrupted");
}

#[test]
fn recovery_backfills_streaming_reasoning_completion() {
    let (path, workspace) = seed_v8_database("streaming-reasoning");
    let database = Database::open(&path).expect("upgrade");
    assert_eq!(database.recover_interrupted_streams().expect("recover"), 1);
    let messages = database
        .load_chat_session(&workspace, "sess-1")
        .expect("load")
        .expect("session");
    let streamed = messages.last().expect("streaming message");
    assert_eq!(streamed.status, "interrupted");
    let completed = streamed.parts[0]
        .completed_at_ms
        .expect("reasoning completed after recovery");
    assert!(completed > 0, "回填当前时刻");
    // 幂等：第二次无行可改。
    assert_eq!(database.recover_interrupted_streams().expect("again"), 0);
}

#[test]
fn migration_is_idempotent_and_keeps_foreign_keys_clean() {
    let (path, workspace) = seed_v8_database("idempotent");
    for _ in 0..3 {
        let database = Database::open(&path).expect("open");
        let violations: i64 = database
            .conn
            .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
                row.get(0)
            })
            .expect("fk check");
        assert_eq!(violations, 0);
        assert_eq!(
            database
                .load_tool_calls_for_session(&workspace, "sess-1")
                .expect("load")
                .len(),
            2
        );
    }
}

#[test]
fn failed_migration_rolls_back_version_and_data() {
    let (path, workspace) = seed_v8_database("rollback");
    // 制造迁移中途失败：让 v9 的 INSERT 因 CHECK 失败（旧行 status 不在新集合内），
    // 通过临时插入一行非法状态不可行；改用同名临时表占位触发 CREATE 失败。
    {
        let connection = rusqlite::Connection::open(&path).expect("open raw");
        connection
            .execute_batch("CREATE VIEW agent_tool_call_v9 AS SELECT 1 AS blocked;")
            .expect("blocking view");
    }
    let error = match Database::open(&path) {
        Ok(_) => panic!("迁移应失败"),
        Err(error) => error,
    };
    assert!(
        matches!(error, DbError::Sqlite(_)),
        "unexpected error: {error}"
    );
    let connection = rusqlite::Connection::open(&path).expect("reopen raw");
    let version: i64 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .expect("version");
    assert_eq!(version, 8, "失败不推进 user_version");
    let tools: i64 = connection
        .query_row("SELECT COUNT(*) FROM agent_tool_call", [], |row| row.get(0))
        .expect("tools");
    assert_eq!(tools, 2, "旧工具行不变");
    let parts: i64 = connection
        .query_row("SELECT COUNT(*) FROM agent_message_part", [], |row| {
            row.get(0)
        })
        .expect("parts");
    assert_eq!(parts, 3, "旧 part 行不变");
    let part_columns: Vec<String> = {
        let mut statement = connection
            .prepare("SELECT name FROM pragma_table_info('agent_message_part')")
            .expect("prepare");
        let rows = statement
            .query_map([], |row| row.get::<_, String>(0))
            .expect("query");
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .expect("collect")
    };
    assert!(
        !part_columns.contains(&"completed_at_ms".to_string()),
        "失败迁移不留下半成品列: {part_columns:?}"
    );
    drop(connection);
    // 解除障碍后重试可成功（结构幂等 + 版本门控不误判）。
    {
        let connection = rusqlite::Connection::open(&path).expect("open raw");
        connection
            .execute_batch("DROP VIEW agent_tool_call_v9;")
            .expect("drop view");
    }
    let database = Database::open(&path).expect("retry succeeds");
    assert_eq!(
        database
            .load_tool_calls_for_session(&workspace, "sess-1")
            .expect("load")
            .len(),
        2
    );
}

#[test]
fn tool_position_and_ticket_survive_reopen() {
    let (path, workspace) = seed_v8_database("ticket-reopen");
    let database = Database::open(&path).expect("upgrade");
    let messages = database
        .load_chat_session(&workspace, "sess-1")
        .expect("load")
        .expect("session");
    let done = &messages[1];
    // 迁移后继续在同一消息上追加工具：位置接在既有 part/tool 之后。
    let ticket = database
        .begin_tool_call(
            &workspace,
            "sess-1",
            &done.external_id,
            "call-new",
            "write",
            "{}",
        )
        .expect("begin after migration");
    assert_eq!(ticket.position, 4, "既有 0..3 之后");
    assert!(ticket.started_at_ms <= now_ms());
}

#[test]
fn fresh_database_has_v9_structure() {
    let path = temp_db_path("fresh");
    let database = Database::open(&path).expect("open fresh");
    assert!(columns(&database, "agent_message_part").contains(&"completed_at_ms".to_string()));
    let tool_columns = columns(&database, "agent_tool_call");
    assert!(tool_columns.contains(&"position".to_string()));
    let definition: String = database
        .conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'agent_tool_call'",
            [],
            |row| row.get(0),
        )
        .expect("definition");
    assert!(definition.contains("interrupted"), "{definition}");
    assert!(
        definition.contains("UNIQUE(message_id, position)"),
        "{definition}"
    );
    assert!(
        definition.contains("UNIQUE(session_id, call_id)"),
        "{definition}"
    );
    // 直接写 interrupted 行被接受（v8 CHECK 会拒绝）。
    let workspace = workspace_path("fresh");
    database.trust_workspace(&workspace, "ws").expect("trust");
    let committed = database
        .begin_turn(&workspace, None, "hi", &model())
        .expect("begin");
    database
        .conn
        .execute(
            r#"INSERT INTO agent_tool_call
                   (external_id, session_id, message_id, call_id, name, position, status,
                    started_at_ms)
               VALUES ('tc-int', ?1, ?2, 'call-int', 'read', 0, 'interrupted', 0)"#,
            rusqlite::params![committed.session_id, committed.assistant_message_id],
        )
        .expect("interrupted accepted");
}

/// 旧库（v7 无时刻列）经 v8 -> v9 两级迁移后，part 列齐全。
#[test]
fn v7_part_table_gains_timing_and_completion_columns() {
    let path = temp_db_path("v7-part-columns");
    std::fs::create_dir_all(path.parent().expect("parent")).expect("dir");
    let v7_part = r#"
        DROP TABLE IF EXISTS agent_message_part;
        CREATE TABLE agent_message_part (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            external_id TEXT NOT NULL UNIQUE,
            message_id TEXT NOT NULL REFERENCES agent_message(external_id) ON DELETE CASCADE,
            position INTEGER NOT NULL CHECK (position >= 0),
            kind TEXT NOT NULL CHECK (kind IN ('text', 'reasoning')),
            content TEXT NOT NULL DEFAULT '',
            provider_metadata_json TEXT,
            created_at_ms INTEGER NOT NULL,
            UNIQUE(message_id, position)
        );
    "#;
    {
        let database = Database::open(&path).expect("open");
        database
            .conn
            .execute_batch(&format!(
                "PRAGMA foreign_keys = OFF; {v7_part} {V8_TOOL_CALL}
                 PRAGMA foreign_keys = ON; PRAGMA user_version = 7;"
            ))
            .expect("back to v7");
    }
    let database = Database::open(&path).expect("upgrade");
    let part_columns = columns(&database, "agent_message_part");
    for column in ["started_at_ms", "ended_at_ms", "completed_at_ms"] {
        assert!(
            part_columns.contains(&column.to_string()),
            "缺列 {column}: {part_columns:?}"
        );
    }
}

/// 迁移后 checkpoint / finish 仍可用，且工具行不被删除。
#[test]
fn migration_then_chat_writes_keep_tools() {
    let (path, _workspace) = seed_v8_database("chat-after-migration");
    let database = Database::open(&path).expect("upgrade");
    let workspace = workspace_path("chat-after-migration-2");
    database.trust_workspace(&workspace, "ws2").expect("trust");
    let committed = database
        .begin_turn(&workspace, None, "hi", &model())
        .expect("begin");
    let ticket = database
        .begin_tool_call(
            &workspace,
            &committed.session_id,
            &committed.assistant_message_id,
            "call-1",
            "read",
            "{}",
        )
        .expect("begin tool");
    database
        .mark_tool_running(&workspace, &ticket)
        .expect("running");
    database
        .finish_tool_call(
            &workspace,
            &ticket,
            &crate::db::chat::ToolCallResult::completed(None),
        )
        .expect("finish tool");
    let part = ChatPart {
        position: 1,
        kind: "text".into(),
        content: "done".into(),
        provider_metadata_json: None,
        started_at_ms: None,
        ended_at_ms: None,
        completed_at_ms: None,
    };
    database
        .finish(
            &workspace,
            &committed.assistant_message_id,
            &[part],
            &TurnOutcome::Complete { stop_reason: None },
            Usage::default(),
        )
        .expect("finish");
    let calls = database
        .load_tool_calls_for_session(&workspace, &committed.session_id)
        .expect("load tools");
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].status, "completed");
}

/// 迁移期不执行文件操作：临时库路径旁不产生额外文件。
#[test]
fn migration_touches_no_files_besides_database() {
    let (path, _workspace) = seed_v8_database("no-side-effects");
    let before = directory_entries(path.parent().expect("parent"));
    let database = Database::open(&path).expect("upgrade");
    drop(database);
    let after = directory_entries(path.parent().expect("parent"));
    // WAL/SHM 由 SQLite 管理，只允许数据库自身相关文件。
    for entry in after {
        assert!(
            matches!(
                entry.as_str(),
                "lx.db" | "lx.db-wal" | "lx.db-shm" | "lx.db-journal"
            ),
            "迁移产生了意外文件: {entry}"
        );
    }
    assert!(!before.is_empty());
}

fn directory_entries(dir: &Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(|entry| entry.ok())
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default()
}

/// v11 → v12：模型列可空、消息 kind/JSON、全部历史与外键数据保留。
#[test]
fn v11_upgrade_keeps_history_and_adds_shell_columns() {
    let path = temp_db_path("v11-upgrade");
    let workspace = workspace_path("v11-upgrade");
    {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("temp dir");
        }
        let conn = rusqlite::Connection::open(&path).expect("raw open");
        conn.execute_batch(SCHEMA).expect("base schema");
        conn.execute_batch(SCHEMA_V7).expect("v7 schema");
        // SCHEMA_V7 已含 v8 的起止时刻列（新装库口径），v9 补齐完成时刻与工具时间线。
        migrate_v9(&conn).expect("v9 schema");
        conn.execute_batch(SCHEMA_V10).expect("v10");
        conn.execute_batch(SCHEMA_V11).expect("v11");
        let ws = workspace.to_string_lossy().into_owned();
        conn.execute(
            r#"INSERT INTO workspace (external_id, name, "type", path, is_trusted, created_at, updated_at)
               VALUES ('ws1', 'ws', 'filesystem', ?1, 1, datetime('now'), datetime('now'))"#,
            rusqlite::params![ws],
        )
        .expect("workspace");
        conn.execute(
            r#"INSERT INTO agent_session
               (external_id, workspace_id, source, title, first_user_message,
                model_provider, model_id, variant, status, archived,
                created_at_ms, updated_at_ms, last_message_at_ms)
               VALUES ('sess1', 'ws1', 'native', '旧会话', '旧消息', 'p', 'm', NULL, 'active', 0, 1, 1, 1)"#,
            [],
        )
        .expect("session");
        conn.execute(
            r#"INSERT INTO agent_message
               (external_id, session_id, sequence, turn_index, role, status,
                model_provider, model_id, created_at_ms, completed_at_ms)
               VALUES ('msg1', 'sess1', 0, 0, 'user', 'complete', NULL, NULL, 1, 1)"#,
            [],
        )
        .expect("message");
        conn.execute(
            r#"INSERT INTO agent_message_part
               (external_id, message_id, position, kind, content, created_at_ms)
               VALUES ('part1', 'msg1', 0, 'text', '旧消息', 1)"#,
            [],
        )
        .expect("part");
        conn.execute(
            r#"INSERT INTO agent_tool_call
               (external_id, session_id, message_id, call_id, name, arguments_json,
                position, status, started_at_ms)
               VALUES ('tool1', 'sess1', 'msg1', 'c1', 'read', '{}', 1, 'completed', 1)"#,
            [],
        )
        .expect("tool");
        conn.execute(
            r#"INSERT INTO agent_process
               (external_id, chat_session_id, origin_call_id, runtime_epoch,
                process_session_id, state, command, cwd, started_at_ms)
               VALUES ('proc1', 'sess1', 'c1', 'e1', 1, 'exited', 'pwd', '/tmp', 1)"#,
            [],
        )
        .expect("process");
        conn.execute_batch("PRAGMA user_version = 11;")
            .expect("version 11");
    }
    let database = Database::open(&path).expect("migrate v11 -> v12");
    let version: i64 = database
        .conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .expect("version");
    assert_eq!(version, SCHEMA_VERSION);
    // 模型列可空（notnull = 0），旧数据原样保留。
    let mut statement = database
        .conn
        .prepare("PRAGMA table_info(agent_session)")
        .expect("table info");
    let nullables: Vec<(String, i64)> = statement
        .query_map([], |row| Ok((row.get(1)?, row.get(3)?)))
        .expect("rows")
        .collect::<std::result::Result<_, _>>()
        .expect("collect");
    for column in ["model_provider", "model_id"] {
        assert_eq!(
            nullables
                .iter()
                .find(|(name, _)| name == column)
                .map(|(_, not_null)| *not_null),
            Some(0),
            "{column} 必须可空"
        );
    }
    let messages = database
        .load_chat_session(&workspace, "sess1")
        .expect("load")
        .expect("session exists");
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].kind, "chat", "旧记录默认 chat");
    assert_eq!(messages[0].parts[0].content, "旧消息");
    let tools = database
        .load_tool_calls_for_session(&workspace, "sess1")
        .expect("tools");
    assert_eq!(tools.len(), 1);
    let processes = database
        .load_processes_for_session(&workspace, "sess1")
        .expect("processes");
    assert_eq!(processes.len(), 1);
    let violations: i64 = database
        .conn
        .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
            row.get(0)
        })
        .expect("fk check");
    assert_eq!(violations, 0);
    // 升级后可直接创建纯命令会话。
    let shell = database
        .begin_user_shell(
            &workspace,
            None,
            &crate::db::chat::UserShellRecord::starting("pwd", "/tmp", "/bin/zsh"),
        )
        .expect("shell-only session after upgrade");
    assert_eq!(
        database
            .chat_session_model(&workspace, &shell.session_id)
            .expect("model"),
        None
    );
}
