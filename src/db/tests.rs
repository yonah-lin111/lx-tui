//! DB 单元测试：迁移、信任、层级约束、级联删除与工作区隔离。

use std::path::PathBuf;
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
        .create_prompt(&workspace, Some(root), "in root", "", &[])
        .expect("root prompt");
    database
        .create_prompt(&workspace, Some(sub), "in sub", "", &[])
        .expect("sub prompt");
    database
        .create_prompt(&workspace, None, "loose", "", &[])
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
        .create_prompt(&left, Some(left_folder), "draft", "", &[])
        .expect("prompt ok");
    database
        .create_prompt(&right, None, "other", "", &[])
        .expect("other prompt ok");

    database
        .update_prompt_document(prompt, "hello\nworld", &[])
        .expect("content ok");
    database
        .update_prompt_status(prompt, "in_progress")
        .expect("status ok");
    database
        .rename_prompt(prompt, "renamed")
        .expect("rename ok");

    let left_prompts = database.list_prompts(&left).expect("list ok");
    assert_eq!(left_prompts.len(), 1);
    assert_eq!(left_prompts[0].name, "renamed");
    assert_eq!(left_prompts[0].content, "hello\nworld");
    assert_eq!(left_prompts[0].status, "in_progress");
    assert_eq!(left_prompts[0].folder_id, Some(left_folder));

    let right_prompts = database.list_prompts(&right).expect("list ok");
    assert_eq!(right_prompts.len(), 1, "workspace isolation");
    assert_eq!(right_prompts[0].name, "other");
    assert_eq!(right_prompts[0].status, "todo");

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
        .create_prompt(&workspace, Some(999), "orphan", "", &[])
        .expect_err("missing folder rejected");
    assert!(matches!(error, DbError::FolderMissing));
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
    assert_eq!(database_path(false).expect("prod path"), prod.join("lx.db"));
}

// ===== v2：附件与 managed 资产 =====

/// 简单文件附件写入参数。
fn file_write(id: &str, path: &str) -> AttachmentWrite {
    AttachmentWrite {
        id: id.to_string(),
        managed_asset_id: None,
        external_path: Some(PathBuf::from(path)),
        kind: "file".to_string(),
        name: path.rsplit('/').next().unwrap_or(path).to_string(),
    }
}

/// 简单 managed 截图写入参数。
fn image_write(id: &str, asset_id: &str) -> AttachmentWrite {
    AttachmentWrite {
        id: id.to_string(),
        managed_asset_id: Some(asset_id.to_string()),
        external_path: None,
        kind: "image".to_string(),
        name: "shot.png".to_string(),
    }
}

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
fn migrates_v1_database_adding_attachment_tables() {
    let path = temp_db_path("v1-upgrade");
    let workspace = workspace_path("v1-upgrade");
    std::fs::create_dir_all(path.parent().expect("parent")).expect("dir");
    {
        let conn = Connection::open(&path).expect("open");
        conn.execute_batch("PRAGMA foreign_keys = ON;").expect("fk");
        conn.execute_batch(SCHEMA_V1).expect("v1 schema");
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
    let database = Database::open(&path).expect("v1 upgrades to v2");
    let version: i64 = database
        .conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .expect("version");
    assert_eq!(version, 2);
    for table in ["managed_attachment_asset", "workspace_prompt_attachment"] {
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
    let prompts = database.list_prompts(&workspace).expect("list");
    assert_eq!(prompts.len(), 1);
    assert_eq!(prompts[0].content, "legacy body", "旧正文不改写");
    assert!(prompts[0].attachments.is_empty(), "旧记录读取为空附件");
    drop(database);
    // 再次打开不重复迁移。
    let reopened = Database::open(&path).expect("reopen ok");
    let version: i64 = reopened
        .conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .expect("version");
    assert_eq!(version, 2);
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
fn create_and_list_prompts_preserve_attachment_order() {
    let (database, _) = open_test_database("attach-order");
    let workspace = workspace_path("attach-order");
    let writes = vec![
        file_write("att-1", "/tmp/a.txt"),
        file_write("att-2", "/tmp/b.txt"),
    ];
    database
        .create_prompt(&workspace, None, "p", "body", &writes)
        .expect("create");
    let prompts = database.list_prompts(&workspace).expect("list");
    let ids: Vec<&str> = prompts[0]
        .attachments
        .iter()
        .map(|attachment| attachment.id.as_str())
        .collect();
    assert_eq!(ids, vec!["att-1", "att-2"], "按 position 顺序恢复");
    assert_eq!(prompts[0].attachments[0].kind, "file");
    assert_eq!(prompts[0].attachments[0].path, PathBuf::from("/tmp/a.txt"));
}

#[test]
fn update_prompt_document_replaces_attachments_and_rolls_back_on_failure() {
    let (database, _) = open_test_database("attach-rollback");
    let workspace = workspace_path("attach-rollback");
    let id = database
        .create_prompt(
            &workspace,
            None,
            "p",
            "old",
            &[file_write("att-1", "/tmp/a.txt")],
        )
        .expect("create");
    database
        .update_prompt_document(id, "new", &[file_write("att-2", "/tmp/c.txt")])
        .expect("update");
    let prompts = database.list_prompts(&workspace).expect("list");
    assert_eq!(prompts[0].content, "new");
    assert_eq!(prompts[0].attachments.len(), 1);
    assert_eq!(prompts[0].attachments[0].id, "att-2");
    // 重复 external_id 触发唯一约束：整笔事务回滚，正文与附件保持更新前状态。
    let broken = vec![
        file_write("att-3", "/tmp/d.txt"),
        file_write("att-3", "/tmp/e.txt"),
    ];
    assert!(
        database
            .update_prompt_document(id, "broken", &broken)
            .is_err()
    );
    let prompts = database.list_prompts(&workspace).expect("list");
    assert_eq!(prompts[0].content, "new", "失败回滚保留旧正文");
    assert_eq!(prompts[0].attachments.len(), 1);
    assert_eq!(prompts[0].attachments[0].id, "att-2");
}

#[test]
fn deleting_folder_cascades_attachment_rows() {
    let (database, _) = open_test_database("attach-cascade");
    let workspace = workspace_path("attach-cascade");
    let folder = database
        .create_folder(&workspace, None, "root")
        .expect("folder");
    let id = database
        .create_prompt(
            &workspace,
            Some(folder),
            "p",
            "",
            &[file_write("att-1", "/tmp/a.txt")],
        )
        .expect("create");
    let _ = id;
    database.delete_folder(folder).expect("delete folder");
    let count: i64 = database
        .conn
        .query_row(
            "SELECT COUNT(*) FROM workspace_prompt_attachment",
            [],
            |row| row.get(0),
        )
        .expect("count");
    assert_eq!(count, 0, "级联删除附件记录");
}

#[test]
fn managed_assets_require_registration_and_block_delete_while_referenced() {
    let (database, _) = open_test_database("managed-asset");
    let workspace = workspace_path("managed-asset");
    let asset_path = PathBuf::from("/tmp/lx-tui-assets/0123456789abcdef0123456789abcdef.png");
    database
        .register_managed_asset("0123456789abcdef0123456789abcdef", &asset_path)
        .expect("register");
    let id = database
        .create_prompt(
            &workspace,
            None,
            "p",
            "",
            &[image_write("att-1", "0123456789abcdef0123456789abcdef")],
        )
        .expect("create");
    let prompts = database.list_prompts(&workspace).expect("list");
    assert_eq!(
        prompts[0].attachments[0].path, asset_path,
        "路径从资产表 JOIN"
    );
    assert_eq!(
        database.managed_attachment_ids().expect("ids"),
        vec!["0123456789abcdef0123456789abcdef".to_string()]
    );
    assert!(
        database
            .delete_managed_asset("0123456789abcdef0123456789abcdef")
            .is_err(),
        "仍被附件引用时拒绝删除资产行"
    );
    // 未登记资产写入被外键拒绝。
    assert!(
        database
            .create_prompt(
                &workspace,
                None,
                "q",
                "",
                &[image_write("att-2", "missing")]
            )
            .is_err()
    );
    database.delete_prompt(id).expect("delete prompt");
    assert!(database.managed_attachment_ids().expect("ids").is_empty());
    database
        .delete_managed_asset("0123456789abcdef0123456789abcdef")
        .expect("delete asset after release");
    assert!(database.list_managed_assets().expect("assets").is_empty());
}

#[test]
fn attachment_check_rejects_double_source() {
    let (database, _) = open_test_database("attach-check");
    let workspace = workspace_path("attach-check");
    let broken = AttachmentWrite {
        id: "att-1".to_string(),
        managed_asset_id: Some("asset".to_string()),
        external_path: Some(PathBuf::from("/tmp/a.txt")),
        kind: "image".to_string(),
        name: "a".to_string(),
    };
    assert!(
        database
            .create_prompt(&workspace, None, "p", "", &[broken])
            .is_err()
    );
}
