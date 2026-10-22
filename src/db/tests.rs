//! DB 单元测试：迁移、信任、层级约束、级联删除与工作区隔离。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use super::*;

/// 独立临时目录中的数据库路径；每个测试互不共享。
fn temp_db_path(label: &str) -> PathBuf {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or(0);
    let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir()
        .join(format!("lx-tui-db-test-{label}-{nanos:x}-{sequence:x}"))
        .join("lx.db")
}

fn open_test_database(label: &str) -> (Database, PathBuf) {
    let path = temp_db_path(label);
    let database = Database::open(&path).expect("database opens");
    (database, path)
}

fn workspace_path(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!("lx-tui-ws-{label}"))
}

/// v5 之前的旧库 Schema（提示词表含状态列）；迁移测试构造旧库用。
const LEGACY_SCHEMA: &str = r#"
CREATE TABLE workspace (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    external_id TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    "type" TEXT NOT NULL DEFAULT 'filesystem',
    path TEXT UNIQUE,
    is_trusted INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE workspace_folder (
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

CREATE TABLE workspace_prompt (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    external_id TEXT NOT NULL UNIQUE,
    workspace_id TEXT NOT NULL,
    folder_id TEXT,
    name TEXT NOT NULL,
    content TEXT NOT NULL DEFAULT '',
    status TEXT NOT NULL DEFAULT 'todo' CHECK (status IN ('todo', 'in_progress', 'completed')),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (workspace_id) REFERENCES workspace(external_id) ON DELETE CASCADE,
    FOREIGN KEY (folder_id) REFERENCES workspace_folder(external_id) ON DELETE CASCADE
);
"#;

#[test]
fn migration_creates_schema_and_is_idempotent() {
    let (database, path) = open_test_database("migrate");
    let version: i64 = database
        .conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .expect("user_version readable");
    assert_eq!(version, SCHEMA_VERSION);
    for table in ["workspace", "workspace_folder", "workspace_prompt"] {
        let count: i64 = database
            .conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
                params![table],
                |row| row.get(0),
            )
            .expect("schema query");
        assert_eq!(count, 1, "table {table} exists");
    }
    drop(database);
    // 重开已迁移库不重复建表、不报错。
    let reopened = Database::open(&path).expect("reopen ok");
    let version: i64 = reopened
        .conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .expect("user_version readable");
    assert_eq!(version, SCHEMA_VERSION);
}

#[test]
fn trust_is_scoped_per_workspace_path() {
    let (database, _) = open_test_database("trust");
    let trusted = workspace_path("trusted");
    let other = workspace_path("other");
    assert!(!database.is_trusted(&trusted).expect("query ok"));
    database
        .trust_workspace(&trusted, "trusted")
        .expect("trust ok");
    assert!(database.is_trusted(&trusted).expect("query ok"));
    assert!(!database.is_trusted(&other).expect("query ok"));
    // 重复信任幂等。
    database
        .trust_workspace(&trusted, "renamed")
        .expect("trust again ok");
    assert!(database.is_trusted(&trusted).expect("query ok"));
}

#[test]
fn folder_nesting_is_limited_to_two_levels() {
    let (database, _) = open_test_database("depth");
    let workspace = workspace_path("depth");
    let root = database
        .create_folder(&workspace, None, "root")
        .expect("root folder");
    let sub = database
        .create_folder(&workspace, Some(root), "sub")
        .expect("sub folder");
    let error = database
        .create_folder(&workspace, Some(sub), "too-deep")
        .expect_err("third level rejected");
    assert!(matches!(error, DbError::DepthExceeded));
    let folders = database.list_folders(&workspace).expect("list ok");
    assert_eq!(folders.len(), 2, "no rows created by rejected insert");
}

#[test]
fn deleting_folder_cascades_subfolders_and_prompts() {
    let (database, _) = open_test_database("cascade");
    let workspace = workspace_path("cascade");
    let root = database
        .create_folder(&workspace, None, "root")
        .expect("root folder");
    let sub = database
        .create_folder(&workspace, Some(root), "sub")
        .expect("sub folder");
    database
        .create_prompt(&workspace, Some(root), "in root", &[])
        .expect("root prompt");
    database
        .create_prompt(&workspace, Some(sub), "in sub", &[])
        .expect("sub prompt");
    database
        .create_prompt(&workspace, None, "loose", &[])
        .expect("loose prompt");

    database.delete_folder(root).expect("delete root");

    assert!(
        database
            .list_folders(&workspace)
            .expect("list ok")
            .is_empty()
    );
    let prompts = database.list_prompts(&workspace).expect("list ok");
    assert_eq!(prompts.len(), 1, "only root-level prompt survives");
    assert_eq!(prompts[0].name, "loose");
}

#[test]
fn prompt_crud_persists_and_isolates_workspaces() {
    let (database, _) = open_test_database("prompt-crud");
    let left = workspace_path("left");
    let right = workspace_path("right");
    let left_folder = database
        .create_folder(&left, None, "folder")
        .expect("folder ok");
    let prompt = database
        .create_prompt(&left, Some(left_folder), "draft", &[])
        .expect("prompt ok");
    database
        .create_prompt(&right, None, "other", &[])
        .expect("other prompt ok");

    let pages = database.list_prompts(&left).expect("list ok");
    let page_id = pages[0].pages[0].id;
    database
        .update_page_content(page_id, "hello\nworld")
        .expect("content ok");
    database
        .rename_prompt(prompt, "renamed")
        .expect("rename ok");

    let left_prompts = database.list_prompts(&left).expect("list ok");
    assert_eq!(left_prompts.len(), 1);
    assert_eq!(left_prompts[0].name, "renamed");
    assert_eq!(left_prompts[0].content, "hello\nworld");
    assert_eq!(left_prompts[0].folder_id, Some(left_folder));

    let right_prompts = database.list_prompts(&right).expect("list ok");
    assert_eq!(right_prompts.len(), 1, "workspace isolation");
    assert_eq!(right_prompts[0].name, "other");

    database.delete_prompt(prompt).expect("delete ok");
    assert!(database.list_prompts(&left).expect("list ok").is_empty());
}

#[test]
fn rename_folder_updates_record() {
    let (database, _) = open_test_database("rename-folder");
    let workspace = workspace_path("rename-folder");
    let folder = database
        .create_folder(&workspace, None, "before")
        .expect("folder ok");
    database.rename_folder(folder, "after").expect("rename ok");
    let folders = database.list_folders(&workspace).expect("list ok");
    assert_eq!(folders.len(), 1);
    assert_eq!(folders[0].name, "after");
}

#[test]
fn prompt_in_missing_folder_is_rejected() {
    let (database, _) = open_test_database("missing-folder");
    let workspace = workspace_path("missing-folder");
    let error = database
        .create_prompt(&workspace, Some(999), "orphan", &[])
        .expect_err("missing folder rejected");
    assert!(matches!(error, DbError::FolderMissing));
}

#[test]
fn referenced_folders_are_scoped_persisted_and_deduplicated() {
    let (database, path) = open_test_database("referenced-folders");
    let left = workspace_path("referenced-left");
    let right = workspace_path("referenced-right");
    assert!(
        database
            .list_referenced_folders(&left)
            .expect("list ok")
            .is_empty(),
        "空列表"
    );
    let first = database
        .add_referenced_folder(&left, "/tmp/one")
        .expect("add ok");
    let second = database
        .add_referenced_folder(&left, "/tmp/two")
        .expect("add ok");
    assert_ne!(first, second);
    let again = database
        .add_referenced_folder(&left, "/tmp/one")
        .expect("duplicate ok");
    assert_eq!(again, first, "重复添加幂等且返回既有 id");
    let list = database.list_referenced_folders(&left).expect("list ok");
    assert_eq!(list.len(), 2, "重复路径不生成第二项");
    assert_eq!(list[0].id, first);
    assert_eq!(list[0].path, "/tmp/one");
    assert_eq!(list[1].path, "/tmp/two", "按添加顺序稳定");
    assert!(
        database
            .list_referenced_folders(&right)
            .expect("list ok")
            .is_empty(),
        "跨项目隔离"
    );

    database
        .delete_referenced_folder(&right, first)
        .expect("cross-project delete ignored");
    assert_eq!(
        database
            .list_referenced_folders(&left)
            .expect("list ok")
            .len(),
        2,
        "错误归属的删除不影响原项目"
    );
    database
        .delete_referenced_folder(&left, first)
        .expect("delete ok");
    let list = database.list_referenced_folders(&left).expect("list ok");
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].path, "/tmp/two");
    // 磁盘不存在时记录保留（不自动清理）。
    database
        .add_referenced_folder(&left, "/tmp/missing-does-not-matter")
        .expect("add stale path ok");
    assert_eq!(
        database
            .list_referenced_folders(&left)
            .expect("list ok")
            .len(),
        2
    );

    drop(database);
    let reopened = Database::open(&path).expect("reopen ok");
    let list = reopened.list_referenced_folders(&left).expect("list ok");
    assert_eq!(list.len(), 2, "重启后列表仍在");
    assert_eq!(list[0].path, "/tmp/two", "顺序保持");
}

#[test]
fn deleting_workspace_row_cascades_referenced_folders() {
    let (database, _) = open_test_database("referenced-cascade");
    let workspace = workspace_path("referenced-cascade");
    database
        .add_referenced_folder(&workspace, "/tmp/cascade")
        .expect("add ok");
    database
        .conn
        .execute(
            "DELETE FROM workspace WHERE path = ?1",
            params![path_string(&workspace)],
        )
        .expect("delete workspace row");
    // 工作区行删除后，新登记的工作区是另一行；旧记录已随外键级联删除。
    let total: i64 = database
        .conn
        .query_row(
            "SELECT COUNT(*) FROM workspace_referenced_folder",
            [],
            |row| row.get(0),
        )
        .expect("count");
    assert_eq!(total, 0, "外键级联删除引用记录");
}

#[test]
fn v5_database_upgrades_to_v6_with_referenced_folder_table() {
    let workspace = workspace_path("v5-referenced-upgrade");
    let path = seed_v4_database("v5-referenced-upgrade", &workspace);
    let database = Database::open(&path).expect("v4 upgrades to v6");
    assert_eq!(schema_version(&database), SCHEMA_VERSION);
    let count: i64 = database
        .conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'workspace_referenced_folder'",
            [],
            |row| row.get(0),
        )
        .expect("schema query");
    assert_eq!(count, 1, "新表随迁移建立");
    // 旧正文保持原样，且新表可写。
    assert_eq!(
        database.list_prompts(&workspace).expect("list")[0].content,
        "page body"
    );
    database
        .add_referenced_folder(&workspace, "/tmp/upgraded")
        .expect("add after upgrade ok");
    drop(database);
    let reopened = Database::open(&path).expect("reopen ok");
    assert_eq!(schema_version(&reopened), SCHEMA_VERSION);
    assert_eq!(
        reopened
            .list_referenced_folders(&workspace)
            .expect("list ok")
            .len(),
        1
    );
}

#[test]
fn dev_environment_detection_prioritizes_explicit_value() {
    assert!(dev_environment(Some("dev")));
    assert!(dev_environment(Some("DEV")));
    assert!(dev_environment(Some(" development ")));
    assert!(dev_environment(Some("local")));
    assert!(!dev_environment(Some("prod")));
    assert!(!dev_environment(Some("")));
    assert_eq!(dev_environment(None), cfg!(debug_assertions));
}

#[test]
fn database_dir_follows_environment_flag() {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let Some(home) = home else {
        return;
    };
    let dev = database_dir(true).expect("dev dir");
    let prod = database_dir(false).expect("prod dir");
    assert_eq!(dev, home.join(".lx-t-dev").join("db"));
    assert_eq!(prod, home.join(".lx-t").join("db"));
    assert_eq!(
        database_path(false, None).expect("prod path"),
        prod.join("lx.db")
    );
    assert_eq!(
        database_path(true, None).expect("dev path"),
        dev.join("lx.db")
    );

    // 开发环境 linked worktree 使用独立文件；生产忽略 worktree 根。
    let worktree = Path::new("/repo/.worktrees/lx-rewind");
    let scoped = database_path(true, Some(worktree)).expect("scoped dev path");
    assert!(scoped.starts_with(dev.join("worktrees")));
    assert_eq!(
        scoped.file_name().and_then(|name| name.to_str()),
        Some("lx.db")
    );
    assert_eq!(
        database_path(false, Some(worktree)).expect("prod path"),
        prod.join("lx.db")
    );
}

#[test]
fn worktree_scope_is_readable_and_path_specific() {
    let root = Path::new("/repo/.worktrees/lx-rewind");
    let scope = worktree_scope_for(root).expect("scope");
    assert!(scope.starts_with("lx-rewind-"));
    assert_eq!(scope, worktree_scope_for(root).expect("stable"));
    assert_ne!(
        worktree_scope_for(Path::new("/other/.worktrees/lx-rewind")).expect("other"),
        scope,
        "同名 worktree 目录不能共用作用域"
    );
}

#[test]
fn worktree_scope_sanitizes_directory_names() {
    assert_eq!(scope_name("feature/x y"), "feature-x-y");
    assert_eq!(scope_name(".."), "worktree");
    assert!(scope_name("lx-rewind").starts_with("lx-rewind"));
}

// ===== 迁移与 Schema =====

#[test]
fn foreign_keys_are_enabled_on_connection() {
    let (database, _) = open_test_database("foreign-keys");
    let enabled: i64 = database
        .conn
        .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
        .expect("pragma readable");
    assert_eq!(enabled, 1);
}

#[test]
fn migrates_v1_database_to_current_schema() {
    let path = temp_db_path("v1-upgrade");
    let workspace = workspace_path("v1-upgrade");
    std::fs::create_dir_all(path.parent().expect("parent")).expect("dir");
    {
        let conn = Connection::open(&path).expect("open");
        conn.execute_batch("PRAGMA foreign_keys = ON;").expect("fk");
        conn.execute_batch(LEGACY_SCHEMA).expect("v1 schema");
        conn.execute_batch("PRAGMA user_version = 1;")
            .expect("version");
        conn.execute(
            "INSERT INTO workspace (external_id, name, \"type\", path, is_trusted, created_at, updated_at) \
             VALUES ('ws-1', 'w', 'filesystem', ?1, 1, datetime('now'), datetime('now'))",
            params![path_string(&workspace)],
        )
        .expect("workspace row");
        conn.execute(
            "INSERT INTO workspace_prompt (external_id, workspace_id, folder_id, name, content, status, created_at, updated_at) \
             VALUES ('p-1', 'ws-1', NULL, 'old', 'legacy body', 'todo', datetime('now'), datetime('now'))",
            [],
        )
        .expect("prompt row");
    }
    let database = Database::open(&path).expect("v1 upgrades to latest");
    assert_eq!(schema_version(&database), SCHEMA_VERSION);
    let prompts = database.list_prompts(&workspace).expect("list");
    assert_eq!(prompts.len(), 1);
    assert_eq!(prompts[0].content, "legacy body", "旧正文不改写");
    assert_no_asset_tables(&database);
    drop(database);
    // 再次打开不重复迁移。
    let reopened = Database::open(&path).expect("reopen ok");
    assert_eq!(schema_version(&reopened), SCHEMA_VERSION);
}

/// 开发期 v2 的库（遗留附件表）：打开后数据保留，遗留表不再被读写。
#[test]
fn migrates_development_v2_database_keeping_data() {
    let path = temp_db_path("v2-legacy");
    let workspace = workspace_path("v2-legacy");
    std::fs::create_dir_all(path.parent().expect("parent")).expect("dir");
    {
        let conn = Connection::open(&path).expect("open");
        conn.execute_batch(LEGACY_SCHEMA).expect("v1 schema");
        conn.execute_batch(
            "CREATE TABLE managed_attachment_asset (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 external_id TEXT NOT NULL UNIQUE,
                 path TEXT NOT NULL UNIQUE,
                 created_at TEXT NOT NULL,
                 updated_at TEXT NOT NULL
             );
             CREATE TABLE workspace_prompt_attachment (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 external_id TEXT NOT NULL UNIQUE,
                 prompt_id TEXT NOT NULL,
                 managed_asset_id TEXT,
                 external_path TEXT,
                 position INTEGER NOT NULL,
                 \"type\" TEXT NOT NULL,
                 name TEXT NOT NULL,
                 created_at TEXT NOT NULL,
                 updated_at TEXT NOT NULL
             );",
        )
        .expect("legacy attachment tables");
        conn.execute(
            "INSERT INTO workspace (external_id, name, \"type\", path, is_trusted, created_at, updated_at) \
             VALUES ('ws-2', 'w', 'filesystem', ?1, 1, datetime('now'), datetime('now'))",
            params![path_string(&workspace)],
        )
        .expect("workspace row");
        conn.execute(
            "INSERT INTO workspace_prompt (external_id, workspace_id, folder_id, name, content, status, created_at, updated_at) \
             VALUES ('p-2', 'ws-2', NULL, 'old', 'v2 body', 'todo', datetime('now'), datetime('now'))",
            [],
        )
        .expect("prompt row");
        conn.execute_batch("PRAGMA user_version = 2;")
            .expect("version");
    }
    let database = Database::open(&path).expect("v2 upgrades to current");
    assert_eq!(schema_version(&database), SCHEMA_VERSION);
    let prompts = database.list_prompts(&workspace).expect("list");
    assert_eq!(prompts.len(), 1);
    assert_eq!(prompts[0].content, "v2 body", "旧正文不改写");
}

/// 当前 Schema 的版本号。
fn schema_version(database: &Database) -> i64 {
    database
        .conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .expect("version")
}

/// 资产/附件表不存在：截图粘贴功能的表已从 Schema 移除。
fn assert_no_asset_tables(database: &Database) {
    for table in [
        "managed_asset",
        "managed_attachment_asset",
        "workspace_prompt_attachment",
    ] {
        let count: i64 = database
            .conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
                params![table],
                |row| row.get(0),
            )
            .expect("schema query");
        assert_eq!(count, 0, "{table} table removed");
    }
}

// ===== v7 聊天会话迁移 =====

/// 构造 v6 库：迁移到 v6 后手工回退版本并去掉聊天表，模拟 v6 旧库。
fn seed_v6_database(label: &str, workspace: &Path) -> PathBuf {
    let path = temp_db_path(label);
    std::fs::create_dir_all(path.parent().expect("parent")).expect("dir");
    {
        let database = Database::open(&path).expect("open to v7");
        database.trust_workspace(workspace, "ws").expect("trust");
        // 制造 v6 现场：删掉 v7 聊天表并回退版本号。
        database
            .conn
            .execute_batch(
                "PRAGMA foreign_keys = OFF;
                 DROP TABLE agent_subagent;
                 DROP TABLE agent_tool_call;
                 DROP TABLE agent_message_part;
                 DROP TABLE agent_message;
                 DROP TABLE agent_session;
                 PRAGMA foreign_keys = ON;
                 PRAGMA user_version = 6;",
            )
            .expect("back to v6");
    }
    path
}

#[test]
fn v6_database_upgrades_to_v7_with_chat_tables() {
    let workspace = workspace_path("v6-chat-upgrade");
    let path = seed_v6_database("v6-chat-upgrade", &workspace);
    let database = Database::open(&path).expect("v6 upgrades to v7");
    assert_eq!(schema_version(&database), SCHEMA_VERSION);
    for table in [
        "agent_session",
        "agent_message",
        "agent_message_part",
        "agent_tool_call",
        "agent_subagent",
    ] {
        let count: i64 = database
            .conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
                params![table],
                |row| row.get(0),
            )
            .expect("schema query");
        assert_eq!(count, 1, "{table} 随 v7 迁移建立");
    }
    // 迁移后外键检查干净。
    let violations: i64 = database
        .conn
        .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
            row.get(0)
        })
        .expect("fk check");
    assert_eq!(violations, 0);
    // 迁移后聊天 CRUD 可用。
    let committed = database
        .begin_turn(&workspace, None, "hello", &v7_model_snapshot())
        .expect("chat works after upgrade");
    let messages = database
        .load_chat_session(&workspace, &committed.session_id)
        .expect("load")
        .expect("exists");
    assert_eq!(messages.len(), 2);
    drop(database);
    // 重开幂等。
    let reopened = Database::open(&path).expect("reopen ok");
    assert_eq!(schema_version(&reopened), SCHEMA_VERSION);
}

/// v7 测试用模型快照。
fn v7_model_snapshot() -> chat::ModelSnapshot {
    chat::ModelSnapshot {
        provider: "example".into(),
        model: "example-chat".into(),
        variant: Some("high".into()),
    }
}

#[test]
fn v6_upgrade_keeps_existing_prompt_and_trust_data() {
    let workspace = workspace_path("v6-keeps-data");
    let path = seed_v6_database("v6-keeps-data", &workspace);
    {
        let database = Database::open(&path).expect("open");
        database
            .create_folder(&workspace, None, "folder")
            .expect("folder");
        database
            .create_prompt(
                &workspace,
                None,
                "prompt",
                &[("Page 1".into(), "body".into())],
            )
            .expect("prompt");
    }
    let database = Database::open(&path).expect("upgrade");
    assert_eq!(schema_version(&database), SCHEMA_VERSION);
    assert!(database.is_trusted(&workspace).expect("trust kept"));
    let prompts = database.list_prompts(&workspace).expect("list");
    assert_eq!(prompts.len(), 1);
    assert_eq!(prompts[0].content, "body");
    let folders = database.list_folders(&workspace).expect("folders");
    assert_eq!(folders.len(), 1);
}

#[test]
fn chat_tables_reject_foreign_key_and_check_violations() {
    let (database, _) = open_test_database("chat-constraints");
    let workspace = workspace_path("chat-constraints");
    database.trust_workspace(&workspace, "ws").expect("trust");
    // 未知 workspace 外键拒绝。
    let result = database.conn.execute(
        r#"INSERT INTO agent_session
               (external_id, workspace_id, title, model_provider, model_id,
                created_at_ms, updated_at_ms, last_message_at_ms)
           VALUES ('sess-x', 'ws-missing', 't', 'p', 'm', 0, 0, 0)"#,
        [],
    );
    assert!(result.is_err(), "外键拒绝");
    let external: String = database
        .conn
        .query_row(
            "SELECT external_id FROM workspace WHERE path = ?1",
            params![path_string(&workspace)],
            |row| row.get(0),
        )
        .optional()
        .expect("workspace external")
        .expect("workspace external id missing");
    database
        .conn
        .execute(
            r#"INSERT INTO agent_session
                   (external_id, workspace_id, title, model_provider, model_id,
                    created_at_ms, updated_at_ms, last_message_at_ms)
               VALUES ('sess-c', ?1, 't', 'p', 'm', 0, 0, 0)"#,
            params![external],
        )
        .expect("seed session");
    // 非法 role 拒绝。
    database
        .conn
        .execute(
            r#"INSERT INTO agent_message
                   (external_id, session_id, sequence, turn_index, role, status, created_at_ms)
               VALUES ('msg-bad', 'sess-c', 0, 0, 'system', 'complete', 0)"#,
            [],
        )
        .expect_err("非法 role 被 CHECK 拒绝");
    database
        .conn
        .execute(
            r#"INSERT INTO agent_message
                   (external_id, session_id, sequence, turn_index, role, status, created_at_ms)
               VALUES ('msg-ok', 'sess-c', 0, 0, 'user', 'complete', 0)"#,
            [],
        )
        .expect("valid message");
    // 自指 subagent 拒绝。
    assert!(
        database
            .conn
            .execute(
                r#"INSERT INTO agent_subagent
                       (parent_session_id, child_session_id, status, created_at_ms, updated_at_ms)
                   VALUES ('sess-c', 'sess-c', 'pending', 0, 0)"#,
                [],
            )
            .is_err(),
        "自指子 Agent 被 CHECK 拒绝"
    );
}

#[test]
fn unique_sequence_and_position_constraints_hold() {
    let (database, _) = open_test_database("chat-unique");
    let workspace = workspace_path("chat-unique");
    database.trust_workspace(&workspace, "ws").expect("trust");
    let committed = database
        .begin_turn(&workspace, None, "hi", &v7_model_snapshot())
        .expect("begin");
    // 同 session 同 sequence 拒绝。
    database
        .conn
        .execute(
            r#"INSERT INTO agent_message
                   (external_id, session_id, sequence, turn_index, role, status, created_at_ms)
               VALUES ('msg-dup', ?1, 0, 0, 'user', 'complete', 0)"#,
            params![committed.session_id],
        )
        .expect_err("UNIQUE(session_id, sequence) 拒绝重复 sequence");
    // 同 message 同 position 拒绝。
    database
        .checkpoint(
            &workspace,
            &committed.assistant_message_id,
            &[chat::ChatPart {
                position: 0,
                kind: "text".into(),
                content: "a".into(),
                provider_metadata_json: None,
                started_at_ms: None,
                ended_at_ms: None,
                completed_at_ms: None,
            }],
        )
        .expect("checkpoint");
    let message_id = committed.assistant_message_id.clone();
    database
        .conn
        .execute(
            r#"INSERT INTO agent_message_part
                   (external_id, message_id, position, kind, content, created_at_ms)
               VALUES ('part-dup', ?1, 0, 'text', 'x', 0)"#,
            params![message_id],
        )
        .expect_err("UNIQUE(message_id, position) 拒绝重复 position");
}

#[test]
fn v0_empty_database_gets_full_v7_schema() {
    let (database, _) = open_test_database("v0-full");
    assert_eq!(schema_version(&database), SCHEMA_VERSION);
    // 新库与升级库结构一致：同一批表存在。
    let workspace = workspace_path("v0-full");
    database.trust_workspace(&workspace, "ws").expect("trust");
    let committed = database
        .begin_turn(&workspace, None, "hello", &v7_model_snapshot())
        .expect("chat works on fresh db");
    assert!(!committed.session_id.is_empty());
}
#[test]
fn chat_tool_and_subagent_tables_accept_valid_rows() {
    // 表结构本期不写行，但必须能接受符合约束的行（结构正确性验证）。
    let (database, _) = open_test_database("chat-tool-schema");
    let workspace = workspace_path("chat-tool-schema");
    database.trust_workspace(&workspace, "ws").expect("trust");
    let committed = database
        .begin_turn(&workspace, None, "hi", &v7_model_snapshot())
        .expect("begin");
    database
        .conn
        .execute(
            r#"INSERT INTO agent_tool_call
                   (external_id, session_id, message_id, call_id, name, position, status,
                    started_at_ms)
               VALUES ('tc-1', ?1, ?2, 'call-1', 'read', 0, 'pending', 0)"#,
            params![committed.session_id, committed.assistant_message_id],
        )
        .expect("tool call row accepted");
    // 创建第二个会话作为 child，验证 subagent 关系约束。
    let child = database
        .begin_turn(&workspace, None, "child turn", &v7_model_snapshot())
        .expect("child session");
    database
        .conn
        .execute(
            r#"INSERT INTO agent_subagent
                   (external_id, parent_session_id, child_session_id, spawn_message_id, status,
                    created_at_ms, updated_at_ms)
               VALUES ('sub-1', ?1, ?2, ?3, 'running', 0, 0)"#,
            params![
                committed.session_id,
                child.session_id,
                committed.assistant_message_id
            ],
        )
        .expect("subagent row accepted");
    // child 唯一性：同一 child 不能再挂另一个父。
    let other = database
        .begin_turn(&workspace, None, "other", &v7_model_snapshot())
        .expect("other session");
    assert!(
        database
            .conn
            .execute(
                r#"INSERT INTO agent_subagent
                       (external_id, parent_session_id, child_session_id, status,
                        created_at_ms, updated_at_ms)
                   VALUES ('sub-2', ?1, ?2, 'pending', 0, 0)"#,
                params![other.session_id, child.session_id],
            )
            .is_err(),
        "child_session_id UNIQUE 拒绝二次挂载"
    );
}

#[test]
fn rejects_database_newer_than_supported() {
    let path = temp_db_path("newer-schema");
    std::fs::create_dir_all(path.parent().expect("parent")).expect("dir");
    {
        let conn = Connection::open(&path).expect("open");
        conn.execute_batch("PRAGMA user_version = 99;")
            .expect("version");
    }
    let error = match Database::open(&path) {
        Ok(_) => panic!("newer schema must be rejected"),
        Err(error) => error,
    };
    assert!(matches!(error, DbError::UnsupportedSchema(99)));
}

#[test]
fn update_page_content_writes_content_and_rejects_unknown_row() {
    let (database, _) = open_test_database("document-update");
    let workspace = workspace_path("document-update");
    let id = database
        .create_prompt(&workspace, None, "p", &[("Page 1".into(), "old".into())])
        .expect("create");
    let page_id = database.list_prompts(&workspace).expect("list")[0].pages[0].id;
    database
        .update_page_content(page_id, "new")
        .expect("update content");
    let prompts = database.list_prompts(&workspace).expect("list");
    assert_eq!(prompts[0].content, "new", "第一页镜像同步");
    assert_eq!(prompts[0].pages[0].content, "new");
    assert!(database.update_page_content(id + 999, "x").is_err());
}

#[test]
fn deleting_folder_cascades_prompts() {
    let (database, _) = open_test_database("folder-cascade");
    let workspace = workspace_path("folder-cascade");
    let folder = database
        .create_folder(&workspace, None, "root")
        .expect("folder");
    database
        .create_prompt(&workspace, Some(folder), "p", &[])
        .expect("create");
    database.delete_folder(folder).expect("delete folder");
    let count: i64 = database
        .conn
        .query_row("SELECT COUNT(*) FROM workspace_prompt", [], |row| {
            row.get(0)
        })
        .expect("count");
    assert_eq!(count, 0, "级联删除条目");
}

// ===== v4 分页迁移与页 CRUD =====

/// 构造 v3 库：当前基础表 + 旧式提示词行（无页表、无 active_page_id 列）。
fn seed_v3_database(label: &str, workspace: &Path, content: &str) -> PathBuf {
    let path = temp_db_path(label);
    std::fs::create_dir_all(path.parent().expect("parent")).expect("dir");
    let conn = Connection::open(&path).expect("open");
    conn.execute_batch("PRAGMA foreign_keys = ON;").expect("fk");
    conn.execute_batch(
        r#"
        CREATE TABLE workspace (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            external_id TEXT NOT NULL UNIQUE,
            name TEXT NOT NULL,
            "type" TEXT NOT NULL DEFAULT 'filesystem',
            path TEXT UNIQUE,
            is_trusted INTEGER NOT NULL DEFAULT 1,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );
        CREATE TABLE workspace_prompt (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            external_id TEXT NOT NULL UNIQUE,
            workspace_id TEXT NOT NULL,
            folder_id TEXT,
            name TEXT NOT NULL,
            content TEXT NOT NULL DEFAULT '',
            status TEXT NOT NULL DEFAULT 'todo' CHECK (status IN ('todo', 'in_progress', 'completed')),
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            FOREIGN KEY (workspace_id) REFERENCES workspace(external_id) ON DELETE CASCADE
        );
        "#,
    )
    .expect("v3 schema");
    conn.execute(
        "INSERT INTO workspace (external_id, name, \"type\", path, is_trusted, created_at, updated_at) \
         VALUES ('ws-v3', 'w', 'filesystem', ?1, 1, datetime('now'), datetime('now'))",
        params![path_string(workspace)],
    )
    .expect("workspace row");
    conn.execute(
        "INSERT INTO workspace_prompt (external_id, workspace_id, folder_id, name, content, status, created_at, updated_at) \
         VALUES ('p-v3', 'ws-v3', NULL, 'old', ?1, 'todo', datetime('now'), datetime('now'))",
        params![content],
    )
    .expect("prompt row");
    conn.execute_batch("PRAGMA user_version = 3;")
        .expect("version");
    drop(conn);
    path
}

#[test]
fn v5_migration_drops_status_column_and_keeps_pages() {
    let workspace = workspace_path("v5-drop-status");
    let path = seed_v4_database("v5-drop-status", &workspace);
    let database = Database::open(&path).expect("v4 upgrades");
    assert_eq!(schema_version(&database), SCHEMA_VERSION);
    let status_columns: i64 = database
        .conn
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('workspace_prompt') WHERE name = 'status'",
            [],
            |row| row.get(0),
        )
        .expect("table info");
    assert_eq!(status_columns, 0, "v5 重建移除状态列");
    let prompts = database.list_prompts(&workspace).expect("list");
    assert_eq!(prompts.len(), 1);
    assert_eq!(prompts[0].name, "paged");
    assert_eq!(prompts[0].pages.len(), 1, "重建不丢页表数据");
    assert_eq!(prompts[0].pages[0].content, "page body");
    assert_eq!(prompts[0].active_page_id, Some(prompts[0].pages[0].id));
    drop(database);
    // 重复打开幂等：不再重建，版本保持当前值。
    let reopened = Database::open(&path).expect("reopen ok");
    assert_eq!(schema_version(&reopened), SCHEMA_VERSION);
    assert_eq!(
        reopened.list_prompts(&workspace).expect("list")[0]
            .pages
            .len(),
        1
    );
}

/// 构造 v4 库：旧基础表（含状态列）+ `active_page_id` 列与页表 + 一条两页数据。
fn seed_v4_database(label: &str, workspace: &Path) -> PathBuf {
    let path = temp_db_path(label);
    std::fs::create_dir_all(path.parent().expect("parent")).expect("dir");
    let conn = Connection::open(&path).expect("open");
    conn.execute_batch("PRAGMA foreign_keys = ON;").expect("fk");
    conn.execute_batch(LEGACY_SCHEMA).expect("legacy schema");
    conn.execute_batch(
        r#"
        ALTER TABLE workspace_prompt ADD COLUMN active_page_id INTEGER;
        CREATE TABLE workspace_prompt_page (
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
        "#,
    )
    .expect("v4 schema");
    conn.execute(
        "INSERT INTO workspace (external_id, name, \"type\", path, is_trusted, created_at, updated_at) \
         VALUES ('ws-v4', 'w', 'filesystem', ?1, 1, datetime('now'), datetime('now'))",
        params![path_string(workspace)],
    )
    .expect("workspace row");
    conn.execute(
        "INSERT INTO workspace_prompt (external_id, workspace_id, folder_id, name, content, status, active_page_id, created_at, updated_at) \
         VALUES ('p-v4', 'ws-v4', NULL, 'paged', 'page body', 'in_progress', 1, datetime('now'), datetime('now'))",
        [],
    )
    .expect("prompt row");
    conn.execute(
        "INSERT INTO workspace_prompt_page (id, prompt_id, position, name, content, created_at, updated_at) \
         VALUES (1, 1, 0, 'Page 1', 'page body', datetime('now'), datetime('now'))",
        [],
    )
    .expect("page row");
    conn.execute_batch("PRAGMA user_version = 4;")
        .expect("version");
    drop(conn);
    path
}

#[test]
fn v4_migration_backfills_single_first_page_idempotently() {
    let workspace = workspace_path("v4-backfill");
    let path = seed_v3_database("v4-backfill", &workspace, "legacy\nbody");
    let database = Database::open(&path).expect("v3 upgrades");
    assert_eq!(schema_version(&database), SCHEMA_VERSION);
    let prompts = database.list_prompts(&workspace).expect("list");
    assert_eq!(prompts.len(), 1);
    assert_eq!(prompts[0].content, "legacy\nbody", "镜像正文逐字不变");
    assert_eq!(prompts[0].pages.len(), 1, "旧正文成为唯一 Page 1");
    assert_eq!(prompts[0].pages[0].name, FIRST_PAGE_NAME);
    assert_eq!(prompts[0].pages[0].content, "legacy\nbody");
    assert_eq!(prompts[0].active_page_id, Some(prompts[0].pages[0].id));
    drop(database);
    // 重复打开不重复生成页。
    let reopened = Database::open(&path).expect("reopen ok");
    let prompts = reopened.list_prompts(&workspace).expect("list");
    assert_eq!(prompts[0].pages.len(), 1);
    assert_eq!(schema_version(&reopened), SCHEMA_VERSION);
}

#[test]
fn v4_migration_failure_rolls_back_and_keeps_old_data() {
    let workspace = workspace_path("v4-rollback");
    let path = seed_v3_database("v4-rollback", &workspace, "keep me");
    // 用同名视图占据页表名，令 v4 迁移建表失败。
    {
        let conn = Connection::open(&path).expect("open");
        conn.execute_batch("CREATE VIEW workspace_prompt_page AS SELECT 1 AS id;")
            .expect("blocking view");
        drop(conn);
    }
    let error = match Database::open(&path) {
        Ok(_) => panic!("migration must fail"),
        Err(error) => error,
    };
    assert!(matches!(error, DbError::Sqlite(_)), "{error:?}");
    // 版本与旧正文保持原样，迁移未部分生效。
    let conn = Connection::open(&path).expect("reopen raw");
    let version: i64 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .expect("version");
    assert_eq!(version, 3, "失败整体回滚，版本不推进");
    let content: String = conn
        .query_row(
            "SELECT content FROM workspace_prompt WHERE external_id = 'p-v3'",
            [],
            |row| row.get(0),
        )
        .expect("old content readable");
    assert_eq!(content, "keep me");
}

#[test]
fn page_crud_appends_renames_and_deletes_with_next_page_fallback() {
    let (database, _) = open_test_database("page-crud");
    let workspace = workspace_path("page-crud");
    let prompt = database
        .create_prompt(
            &workspace,
            None,
            "paged",
            &[
                ("Page 1".into(), "one".into()),
                ("Page 2".into(), "two".into()),
            ],
        )
        .expect("create");
    let pages = database.list_prompts(&workspace).expect("list")[0]
        .pages
        .clone();
    assert_eq!(pages.len(), 2);
    assert_eq!(pages[0].content, "one");
    assert_eq!(pages[1].content, "two");
    let first_page = pages[0].id;

    // 第一页正文写入镜像；第二页不动镜像。
    database
        .update_page_content(first_page, "one!")
        .expect("update first");
    database
        .update_page_content(pages[1].id, "two!")
        .expect("update second");
    let record = &database.list_prompts(&workspace).expect("list")[0];
    assert_eq!(record.content, "one!", "镜像跟随第一页");
    assert_eq!(record.pages[1].content, "two!");

    // 追加页：position 取最大加一，并设为当前页。
    let appended = database
        .create_prompt_page(prompt, "Page 3")
        .expect("append");
    let record = &database.list_prompts(&workspace).expect("list")[0];
    assert_eq!(record.pages.len(), 3);
    assert_eq!(record.pages[2].id, appended);
    assert_eq!(record.active_page_id, Some(appended), "新页立即成为当前页");

    // 重命名；空名由应用层拒绝，数据库只要求非空字符串。
    database
        .rename_prompt_page(appended, "Notes")
        .expect("rename");
    assert_eq!(
        database.list_prompts(&workspace).expect("list")[0].pages[2].name,
        "Notes"
    );

    // 删除当前页（末页）：回退前一页。
    let fallback = database.delete_prompt_page(appended).expect("delete");
    assert_eq!(fallback, Some(pages[1].id), "末页删除后回退前一页");
    let record = &database.list_prompts(&workspace).expect("list")[0];
    assert_eq!(record.pages.len(), 2);
    assert_eq!(record.active_page_id, Some(pages[1].id));

    // 删除当前页（首页）：优先下一页。
    database
        .set_prompt_active_page(prompt, first_page)
        .expect("focus first");
    let fallback = database.delete_prompt_page(first_page).expect("delete");
    assert_eq!(fallback, Some(pages[1].id), "优先显示原下一页");
    let record = &database.list_prompts(&workspace).expect("list")[0];
    assert_eq!(record.active_page_id, Some(pages[1].id));
    assert_eq!(record.pages.len(), 1, "最后一页保留");

    // 删除不存在的页报错；库层允许删空（应用层阻止最后一页删除）。
    assert!(matches!(
        database.delete_prompt_page(9999),
        Err(DbError::PageMissing)
    ));
    let fallback = database.delete_prompt_page(pages[1].id).expect("delete");
    assert_eq!(fallback, None, "无可回退页时返回 None");
    let record = &database.list_prompts(&workspace).expect("list")[0];
    assert!(record.pages.is_empty());
    assert_eq!(record.active_page_id, None);
}

#[test]
fn set_active_page_rejects_foreign_page_and_prompt_delete_cascades_pages() {
    let (database, _) = open_test_database("page-active");
    let left = workspace_path("page-active-left");
    let right = workspace_path("page-active-right");
    let first = database
        .create_prompt(&left, None, "a", &[])
        .expect("prompt a");
    let second = database
        .create_prompt(&left, None, "b", &[])
        .expect("prompt b");
    let foreign = database.list_prompts(&left).expect("list")[1].pages[0].id;
    assert!(matches!(
        database.set_prompt_active_page(first, foreign),
        Err(DbError::PageMissing)
    ));
    database
        .set_prompt_active_page(second, foreign)
        .expect("own page accepted");
    assert_eq!(
        database.list_prompts(&left).expect("list")[1].active_page_id,
        Some(foreign)
    );

    database.delete_prompt(first).expect("delete prompt");
    let pages: i64 = database
        .conn
        .query_row(
            "SELECT COUNT(*) FROM workspace_prompt_page WHERE prompt_id = ?1",
            params![first],
            |row| row.get(0),
        )
        .expect("count pages");
    assert_eq!(pages, 0, "删除条目级联删除页");
    assert!(database.list_prompts(&right).expect("list").is_empty());
}

#[test]
fn create_prompt_with_pages_is_atomic_and_mirrors_first_page() {
    let (database, _) = open_test_database("page-create");
    let workspace = workspace_path("page-create");
    let id = database
        .create_prompt(
            &workspace,
            None,
            "multi",
            &[
                ("Intro".into(), "alpha".into()),
                ("Body".into(), "beta".into()),
            ],
        )
        .expect("create");
    let record = &database.list_prompts(&workspace).expect("list")[0];
    assert_eq!(record.id, id);
    assert_eq!(record.content, "alpha", "镜像取第一页正文");
    assert_eq!(record.pages.len(), 2);
    assert_eq!(record.pages[1].name, "Body");
    assert_eq!(record.active_page_id, Some(record.pages[0].id));

    // 空页集合兜底为唯一空 Page 1。
    let plain = database
        .create_prompt(&workspace, None, "plain", &[])
        .expect("create plain");
    let record = database
        .list_prompts(&workspace)
        .expect("list")
        .into_iter()
        .find(|record| record.id == plain)
        .expect("record");
    assert_eq!(record.pages.len(), 1);
    assert_eq!(record.pages[0].content, "");
}

// ===== v8 消息 part 起止时刻迁移 =====

/// 构造 v7 现场：完整库建好后把 `agent_message_part` 换回无起止时刻的 v7 定义并回退版本号。
fn seed_v7_database(label: &str, workspace: &Path) -> PathBuf {
    let path = temp_db_path(label);
    std::fs::create_dir_all(path.parent().expect("parent")).expect("dir");
    {
        let database = Database::open(&path).expect("open to v8");
        database.trust_workspace(workspace, "ws").expect("trust");
        database
            .conn
            .execute_batch(
                "PRAGMA foreign_keys = OFF;
                 DROP TABLE agent_message_part;
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
                 PRAGMA foreign_keys = ON;
                 PRAGMA user_version = 7;",
            )
            .expect("back to v7");
    }
    path
}

#[test]
fn v7_database_upgrades_to_v8_with_part_timing_columns() {
    let workspace = workspace_path("v7-part-timing");
    let path = seed_v7_database("v7-part-timing", &workspace);
    let database = Database::open(&path).expect("v7 upgrades to v8");
    assert_eq!(schema_version(&database), SCHEMA_VERSION);
    let columns: Vec<String> = {
        let mut statement = database
            .conn
            .prepare("SELECT name FROM pragma_table_info('agent_message_part')")
            .expect("prepare");
        let rows = statement
            .query_map([], |row| row.get::<_, String>(0))
            .expect("query");
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .expect("collect")
    };
    assert!(
        columns.contains(&"started_at_ms".to_string()),
        "{columns:?}"
    );
    assert!(columns.contains(&"ended_at_ms".to_string()), "{columns:?}");
    // 迁移后旧行（无时刻）仍可读；新写入带时刻。
    let committed = database
        .begin_turn(
            &workspace,
            None,
            "问",
            &chat::ModelSnapshot {
                provider: "p".into(),
                model: "m".into(),
                variant: None,
            },
        )
        .expect("begin");
    database
        .checkpoint(
            &workspace,
            &committed.assistant_message_id,
            &[chat::ChatPart {
                position: 0,
                kind: "reasoning".into(),
                content: "思考".into(),
                provider_metadata_json: None,
                started_at_ms: Some(10),
                ended_at_ms: Some(6_310),
                completed_at_ms: None,
            }],
        )
        .expect("checkpoint");
    let loaded = database
        .load_chat_session(&workspace, &committed.session_id)
        .expect("load")
        .expect("exists");
    let assistant = loaded.last().expect("assistant");
    assert_eq!(assistant.parts[0].started_at_ms, Some(10));
    assert_eq!(assistant.parts[0].ended_at_ms, Some(6_310));
}
