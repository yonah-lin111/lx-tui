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
        .create_prompt(&workspace, Some(root), "in root", "")
        .expect("root prompt");
    database
        .create_prompt(&workspace, Some(sub), "in sub", "")
        .expect("sub prompt");
    database
        .create_prompt(&workspace, None, "loose", "")
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
        .create_prompt(&left, Some(left_folder), "draft", "")
        .expect("prompt ok");
    database
        .create_prompt(&right, None, "other", "")
        .expect("other prompt ok");

    database
        .update_prompt_content(prompt, "hello\nworld")
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
        .create_prompt(&workspace, Some(999), "orphan", "")
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
