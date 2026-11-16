//! v30 沙箱列/表与规则种类解析。

use super::*;

#[test]
fn v30_adds_session_mode_column_and_rule_table() {
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
