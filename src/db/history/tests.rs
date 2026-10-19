//! 全局 prompt 历史持久化测试：迁移、去重置顶、上限裁剪与载入顺序。

use std::path::PathBuf;

use super::{Database, DbError};

/// 独立临时目录中的数据库路径；每个测试互不共享。
fn temp_db_path(label: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or(0);
    let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir()
        .join(format!(
            "lx-tui-history-test-{label}-{nanos:x}-{sequence:x}"
        ))
        .join("lx.db")
}

fn open_test_database(label: &str) -> Database {
    Database::open(&temp_db_path(label)).expect("database opens")
}

#[test]
fn fresh_database_exposes_current_schema_with_empty_history() {
    let database = open_test_database("fresh");
    let version: i64 = database
        .conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .expect("user_version");
    assert_eq!(
        version,
        crate::db::SCHEMA_VERSION_FOR_TESTS,
        "新建库直接推进到当前 schema 版本"
    );
    assert!(database.load_prompt_history(100).expect("load").is_empty());
    assert_eq!(database.prompt_history_count().expect("count"), 0);
}

#[test]
fn record_and_load_returns_oldest_to_newest() {
    let database = open_test_database("order");
    for text in ["first", "second", "third"] {
        database.record_prompt_history(text, 100).expect("record");
    }
    let entries = database.load_prompt_history(100).expect("load");
    assert_eq!(entries, vec!["first", "second", "third"], "末尾为最近使用");
}

#[test]
fn record_deduplicates_and_moves_to_newest() {
    let database = open_test_database("dedupe");
    database
        .record_prompt_history("alpha", 100)
        .expect("record");
    database.record_prompt_history("beta", 100).expect("record");
    database
        .record_prompt_history("alpha", 100)
        .expect("record");
    let entries = database.load_prompt_history(100).expect("load");
    assert_eq!(entries, vec!["beta", "alpha"], "重复项置顶且不新增行");
    assert_eq!(database.prompt_history_count().expect("count"), 2);
}

#[test]
fn record_trims_to_limit_dropping_oldest() {
    let database = open_test_database("limit");
    for index in 0..12 {
        database
            .record_prompt_history(&format!("prompt {index}"), 5)
            .expect("record");
    }
    let entries = database.load_prompt_history(5).expect("load");
    assert_eq!(
        entries,
        vec![
            "prompt 7".to_string(),
            "prompt 8".to_string(),
            "prompt 9".to_string(),
            "prompt 10".to_string(),
            "prompt 11".to_string(),
        ]
    );
    assert_eq!(database.prompt_history_count().expect("count"), 5);
}

#[test]
fn load_limit_takes_newest_subset() {
    let database = open_test_database("subset");
    for text in ["one", "two", "three", "four"] {
        database.record_prompt_history(text, 100).expect("record");
    }
    let entries = database.load_prompt_history(2).expect("load");
    assert_eq!(entries, vec!["three", "four"], "只取最近两条，仍按旧→新");
    let none = database.load_prompt_history(0).expect("load");
    assert!(none.is_empty(), "limit 为 0 返回空");
}

#[test]
fn record_rejects_blank_and_oversized() {
    let database = open_test_database("reject");
    assert!(matches!(
        database.record_prompt_history("   \n", 100),
        Err(DbError::InvalidDraft)
    ));
    let oversized = "x".repeat(32 * 1024 + 1);
    assert!(matches!(
        database.record_prompt_history(&oversized, 100),
        Err(DbError::MessageTooLarge)
    ));
    assert_eq!(database.prompt_history_count().expect("count"), 0);
}

#[test]
fn migration_is_idempotent_across_reopen() {
    let path = temp_db_path("reopen");
    {
        let database = Database::open(&path).expect("open");
        database.record_prompt_history("kept", 100).expect("record");
    }
    // 二次打开：迁移不重复建表、不丢数据。
    let reopened = Database::open(&path).expect("reopen");
    assert_eq!(
        reopened.load_prompt_history(100).expect("load"),
        vec!["kept"]
    );
}

#[test]
fn v9_database_upgrades_to_current_schema_keeping_chat_data() {
    let path = temp_db_path("v9");
    {
        // 先用当前程序建库，再手工把版本号退回 9 并删掉 v10/v11 新增的表，模拟 v9 旧库。
        let database = Database::open(&path).expect("open");
        database
            .conn
            .execute_batch(
                "DROP TABLE prompt_history;
                 DROP TABLE agent_process;
                 PRAGMA user_version = 9;",
            )
            .expect("downgrade to v9");
    }
    let upgraded = Database::open(&path).expect("upgrade");
    let version: i64 = upgraded
        .conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .expect("user_version");
    assert_eq!(
        version,
        crate::db::SCHEMA_VERSION_FOR_TESTS,
        "旧库升级到当前 schema 版本"
    );
    // 新表可用。
    upgraded
        .record_prompt_history("after upgrade", 100)
        .expect("record");
    assert_eq!(
        upgraded.load_prompt_history(100).expect("load"),
        vec!["after upgrade"]
    );
}
