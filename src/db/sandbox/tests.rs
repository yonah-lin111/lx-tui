//! v31 沙箱列/表与规则种类解析。

use super::*;

#[test]
fn v31_adds_session_mode_column_and_rule_table() {
    let database = Database::open_in_memory().expect("open");
    let version: i64 = database
        .conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .expect("version");
    assert_eq!(version, super::super::SCHEMA_VERSION);

    let mut statement = database
        .conn
        .prepare("PRAGMA table_info(agent_session)")
        .expect("table info");
    let columns: Vec<String> = statement
        .query_map([], |row| row.get::<_, String>(1))
        .expect("columns")
        .map(|row| row.expect("column"))
        .collect();
    assert!(columns.iter().any(|column| column == "sandbox_mode"));

    let tables: i64 = database
        .conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'agent_session_sandbox_rule'",
            [],
            |row| row.get(0),
        )
        .expect("rule table");
    assert_eq!(tables, 1);
}

#[test]
fn rule_kind_round_trips() {
    for kind in [SandboxRuleKind::Exact, SandboxRuleKind::Prefix] {
        assert_eq!(SandboxRuleKind::parse(kind.as_str()), Some(kind));
    }
    assert_eq!(SandboxRuleKind::parse("other"), None);
}

#[test]
fn v30_database_upgrades_to_v31_with_sandbox_schema() {
    let path = std::env::temp_dir().join(format!(
        "lx-db-v31-{}-{}.db",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default()
    ));
    {
        let database = Database::open(&path).expect("open latest");
        // 模拟 dev 的 v30：report checkpoint 列已存在，沙箱迁移尚未执行。
        database
            .conn
            .execute_batch(
                "DROP TABLE IF EXISTS agent_session_sandbox_rule;
                 ALTER TABLE agent_session DROP COLUMN sandbox_mode;
                 ALTER TABLE agent_compact_checkpoint
                     ADD COLUMN kind TEXT NOT NULL DEFAULT 'compact'
                     CHECK (kind IN ('compact', 'report'));
                 PRAGMA user_version = 30;",
            )
            .expect("simulate v30");
    }

    let database = Database::open(&path).expect("upgrade to v31");
    let version: i64 = database
        .conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .expect("version");
    assert_eq!(version, super::super::SCHEMA_VERSION);
    let sandbox_column: i64 = database
        .conn
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('agent_session') WHERE name = 'sandbox_mode'",
            [],
            |row| row.get(0),
        )
        .expect("column");
    assert_eq!(sandbox_column, 1);
    let report_column: i64 = database
        .conn
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('agent_compact_checkpoint') WHERE name = 'kind'",
            [],
            |row| row.get(0),
        )
        .expect("report column");
    assert_eq!(report_column, 1, "dev v30 迁移不得被跳过");
    let table: i64 = database
        .conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'agent_session_sandbox_rule'",
            [],
            |row| row.get(0),
        )
        .expect("rule table");
    assert_eq!(table, 1);
    let _ = std::fs::remove_file(path);
}
