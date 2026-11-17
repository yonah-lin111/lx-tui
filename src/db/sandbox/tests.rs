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

    let switch_tables: i64 = database
        .conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'agent_permission_switch'",
            [],
            |row| row.get(0),
        )
        .expect("switch table");
    assert_eq!(switch_tables, 1);
}

#[test]
fn permission_switch_records_follow_sequence_and_survive_fork_and_rewind() {
    use crate::db::chat::{ModelSnapshot, TurnOutcome, Usage};
    let root = std::env::temp_dir().join(format!(
        "lx-db-perm-switch-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default()
    ));
    std::fs::create_dir_all(&root).expect("temp dir");
    let database = Database::open(&root.join("lx.db")).expect("open");
    database.trust_workspace(&root, "ws").expect("trust");
    let model = ModelSnapshot {
        provider: "p".into(),
        model: "m".into(),
        variant: None,
    };
    let turn1 = database
        .begin_turn(&root, None, "turn 1", &model)
        .expect("turn1");
    database
        .finish(
            &root,
            &turn1.assistant_message_id,
            &[],
            &TurnOutcome::Complete {
                stop_reason: Some("stop".into()),
            },
            Usage::default(),
        )
        .expect("finish1");
    let recorded = database
        .record_permission_switch(
            &root,
            &PermissionSwitchCommit {
                session_id: turn1.session_id.clone(),
                from_mode: "workspace-write".into(),
                to_mode: "danger-full-access".into(),
                injected_prompt: "Switched to Full".into(),
            },
        )
        .expect("record")
        .expect("created");
    assert_eq!(recorded.to_mode, "danger-full-access");
    let loaded = database
        .permission_switches(&root, &turn1.session_id)
        .expect("load");
    assert_eq!(loaded.len(), 1);
    assert_eq!(
        database
            .session_sandbox_mode(&root, &turn1.session_id)
            .expect("mode"),
        Some("danger-full-access".into())
    );
    let forked = database
        .fork_chat_session(&root, &turn1.session_id, None)
        .expect("fork");
    assert_eq!(
        database
            .session_sandbox_mode(&root, &forked.session_id)
            .expect("fork mode"),
        Some("danger-full-access".into())
    );
    assert_eq!(
        database
            .permission_switches(&root, &forked.session_id)
            .expect("fork switches")
            .len(),
        1
    );
    let _ = std::fs::remove_dir_all(root);
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
