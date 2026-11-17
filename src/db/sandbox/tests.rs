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
                 DROP TABLE IF EXISTS agent_permission_switch;
                 ALTER TABLE agent_session DROP COLUMN sandbox_mode;
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

#[test]
fn v32_database_without_report_kind_upgrades_to_v33() {
    let path = std::env::temp_dir().join(format!(
        "lx-db-v33-{}-{}.db",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default()
    ));
    {
        let database = Database::open(&path).expect("open latest");
        // 模拟只在沙箱分支上推进过、缺少 dev v30 `kind` 列的 v32 库。
        database
            .conn
            .execute_batch(
                "ALTER TABLE agent_compact_checkpoint DROP COLUMN kind;
                 PRAGMA user_version = 32;",
            )
            .expect("simulate v32");
    }

    let database = Database::open(&path).expect("upgrade to v33");
    let version: i64 = database
        .conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .expect("version");
    assert_eq!(version, super::super::SCHEMA_VERSION);
    let report_column: i64 = database
        .conn
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('agent_compact_checkpoint') WHERE name = 'kind'",
            [],
            |row| row.get(0),
        )
        .expect("report column");
    assert_eq!(report_column, 1, "v33 必须补齐缺失的 dev v30 列");
    let sandbox_column: i64 = database
        .conn
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('agent_session') WHERE name = 'sandbox_mode'",
            [],
            |row| row.get(0),
        )
        .expect("sandbox column");
    assert_eq!(sandbox_column, 1);
    let _ = std::fs::remove_file(path);
}

#[test]
fn sandbox_rule_round_trips_empty_tokens_as_json_and_reads_legacy_nul_format() {
    use crate::db::chat::{ModelSnapshot, TurnOutcome, Usage};
    let root = std::env::temp_dir().join(format!(
        "lx-db-sandbox-rule-json-{}-{}",
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
    let turn = database
        .begin_turn(&root, None, "turn 1", &model)
        .expect("turn");
    database
        .finish(
            &root,
            &turn.assistant_message_id,
            &[],
            &TurnOutcome::Complete {
                stop_reason: Some("stop".into()),
            },
            Usage::default(),
        )
        .expect("finish");

    let empty_arg_rule = SessionSandboxRule {
        kind: SandboxRuleKind::Exact,
        tokens: vec!["git".into(), "commit".into(), "-m".into(), String::new()],
    };
    let prefix_rule = SessionSandboxRule {
        kind: SandboxRuleKind::Prefix,
        tokens: vec!["sh".into(), "-c".into(), String::new()],
    };
    database
        .insert_sandbox_rule(&root, &turn.session_id, &empty_arg_rule)
        .expect("insert exact");
    database
        .insert_sandbox_rule(&root, &turn.session_id, &prefix_rule)
        .expect("insert prefix");

    let raw: String = database
        .conn
        .query_row(
            "SELECT prefix_json FROM agent_session_sandbox_rule WHERE session_id = ?1 AND kind = 'exact'",
            params![turn.session_id],
            |row| row.get(0),
        )
        .expect("raw json");
    assert_eq!(raw, r#"["git","commit","-m",""]"#);

    // 模拟历史旧库以 `\u{0}` 拼接且末尾带空参数的数据，读取不得过滤空串。
    database
        .conn
        .execute(
            "INSERT INTO agent_session_sandbox_rule (session_id, kind, prefix_json, created_at_ms)
             VALUES (?1, 'exact', ?2, 1)",
            params![turn.session_id, "printf\u{0}%s\u{0}"],
        )
        .expect("insert legacy");

    let loaded = database
        .sandbox_rules(&root, &turn.session_id)
        .expect("load rules");
    assert_eq!(
        loaded,
        vec![
            empty_arg_rule,
            prefix_rule,
            SessionSandboxRule {
                kind: SandboxRuleKind::Exact,
                tokens: vec!["printf".into(), "%s".into(), String::new()],
            },
        ]
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn fork_before_boundary_resolves_sandbox_mode_at_boundary_and_subagent_skips_switches() {
    use crate::chat::agent::AgentRole;
    use crate::db::chat::{ModelSnapshot, TurnOutcome, Usage};
    let root = std::env::temp_dir().join(format!(
        "lx-db-fork-perm-{}-{}",
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

    let t1 = database
        .begin_turn(&root, None, "turn 1", &model)
        .expect("t1");
    database
        .finish(
            &root,
            &t1.assistant_message_id,
            &[],
            &TurnOutcome::Complete {
                stop_reason: Some("stop".into()),
            },
            Usage::default(),
        )
        .expect("f1");

    let t2 = database
        .begin_turn(&root, Some(&t1.session_id), "turn 2", &model)
        .expect("t2");
    database
        .finish(
            &root,
            &t2.assistant_message_id,
            &[],
            &TurnOutcome::Complete {
                stop_reason: Some("stop".into()),
            },
            Usage::default(),
        )
        .expect("f2");

    // 切换 1（t2 之后、t3 之前）：workspace-write -> danger-full-access
    database
        .record_permission_switch(
            &root,
            &PermissionSwitchCommit {
                session_id: t1.session_id.clone(),
                from_mode: "workspace-write".into(),
                to_mode: "danger-full-access".into(),
                injected_prompt: "Switched to Full".into(),
            },
        )
        .expect("sw1");

    let t3 = database
        .begin_turn(&root, Some(&t1.session_id), "turn 3", &model)
        .expect("t3");
    database
        .finish(
            &root,
            &t3.assistant_message_id,
            &[],
            &TurnOutcome::Complete {
                stop_reason: Some("stop".into()),
            },
            Usage::default(),
        )
        .expect("f3");

    // 切换 2（t3 之后、t4 之前）：danger-full-access -> read-only
    database
        .record_permission_switch(
            &root,
            &PermissionSwitchCommit {
                session_id: t1.session_id.clone(),
                from_mode: "danger-full-access".into(),
                to_mode: "read-only".into(),
                injected_prompt: "Switched to Read-only".into(),
            },
        )
        .expect("sw2");

    let t4 = database
        .begin_turn(&root, Some(&t1.session_id), "turn 4", &model)
        .expect("t4");
    database
        .finish(
            &root,
            &t4.assistant_message_id,
            &[],
            &TurnOutcome::Complete {
                stop_reason: Some("stop".into()),
            },
            Usage::default(),
        )
        .expect("f4");

    // 在 t2 之前分叉：处于切换 1 之前，应取切换 1 的 from_mode = "workspace-write" 且无切换记录。
    let forked_before_t2 = database
        .fork_chat_session(&root, &t1.session_id, Some(&t2.user_message_id))
        .expect("fork before t2");
    assert_eq!(
        database
            .session_sandbox_mode(&root, &forked_before_t2.session_id)
            .expect("mode"),
        Some("workspace-write".into())
    );
    assert!(
        database
            .permission_switches(&root, &forked_before_t2.session_id)
            .expect("switches")
            .is_empty()
    );

    // 在 t3 之前分叉：处于切换 1 之后、切换 2 之前，应取切换 1 的 to_mode = "danger-full-access"。
    let forked_before_t3 = database
        .fork_chat_session(&root, &t1.session_id, Some(&t3.user_message_id))
        .expect("fork before t3");
    assert_eq!(
        database
            .session_sandbox_mode(&root, &forked_before_t3.session_id)
            .expect("mode"),
        Some("danger-full-access".into())
    );
    assert_eq!(
        database
            .permission_switches(&root, &forked_before_t3.session_id)
            .expect("switches")
            .len(),
        1
    );

    // P2-9：子 agent fork_context=true 不应复制父会话的 agent_permission_switch 行。
    let child = database
        .begin_subagent_turn_with_full_options(
            &root,
            &t1.session_id,
            None,
            Some("Turing"),
            AgentRole::Explorer,
            "Forked child",
            "inspect",
            &[],
            &model,
            None,
            None,
            None,
            true,
            None,
            None,
            &None,
        )
        .expect("spawn child");
    assert!(
        database
            .permission_switches(&root, &child.child_session_id)
            .expect("child switches")
            .is_empty(),
        "子会话不得继承父会话的 agent_permission_switch 行"
    );

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn commit_rewind_rolls_back_sandbox_mode_and_clears_session_rules() {
    use crate::db::chat::{ModelSnapshot, RewindCommit, TurnOutcome, Usage};
    let root = std::env::temp_dir().join(format!(
        "lx-db-rewind-perm-{}-{}",
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

    let t1 = database
        .begin_turn(&root, None, "turn 1", &model)
        .expect("t1");
    database
        .finish(
            &root,
            &t1.assistant_message_id,
            &[],
            &TurnOutcome::Complete {
                stop_reason: Some("stop".into()),
            },
            Usage::default(),
        )
        .expect("f1");

    // 切换 1：workspace-write -> read-only
    database
        .record_permission_switch(
            &root,
            &PermissionSwitchCommit {
                session_id: t1.session_id.clone(),
                from_mode: "workspace-write".into(),
                to_mode: "read-only".into(),
                injected_prompt: "Switched to Read-only".into(),
            },
        )
        .expect("sw1");

    let t2 = database
        .begin_turn(&root, Some(&t1.session_id), "turn 2", &model)
        .expect("t2");
    database
        .finish(
            &root,
            &t2.assistant_message_id,
            &[],
            &TurnOutcome::Complete {
                stop_reason: Some("stop".into()),
            },
            Usage::default(),
        )
        .expect("f2");

    // 切换 2：read-only -> danger-full-access，并写入一条会话规则
    database
        .record_permission_switch(
            &root,
            &PermissionSwitchCommit {
                session_id: t1.session_id.clone(),
                from_mode: "read-only".into(),
                to_mode: "danger-full-access".into(),
                injected_prompt: "Switched to Full".into(),
            },
        )
        .expect("sw2");
    let t3 = database
        .begin_turn(&root, Some(&t1.session_id), "turn 3", &model)
        .expect("t3");
    database
        .finish(
            &root,
            &t3.assistant_message_id,
            &[],
            &TurnOutcome::Complete {
                stop_reason: Some("stop".into()),
            },
            Usage::default(),
        )
        .expect("f3");
    database
        .insert_sandbox_rule(
            &root,
            &t1.session_id,
            &SessionSandboxRule {
                kind: SandboxRuleKind::Exact,
                tokens: vec!["echo".into(), "ok".into()],
            },
        )
        .expect("rule");

    // 1) 先撤销第 3 轮（最新轮）：sw2 在 t3 之前，仍在存活范围内，模式保持 sw2 的 "danger-full-access"，且会话规则被清理。
    let rev = database.session_revision(&t1.session_id).expect("rev");
    database
        .begin_operation(
            &root,
            "op-rw-t3",
            &t1.session_id,
            &t3.user_message_id,
            rev,
            "delete_only",
            "turn 3",
        )
        .expect("begin op 3");
    let committed_t3 = database
        .commit_rewind(
            &root,
            &RewindCommit {
                operation_id: "op-rw-t3".into(),
                session_id: t1.session_id.clone(),
                target_user_message_id: t3.user_message_id.clone(),
                expected_revision: rev,
                kind: "delete_only".into(),
                delete_conversation: true,
                revert_files: false,
                restored_file_count: 0,
                files_already_restored: false,
            },
        )
        .expect("commit t3");
    assert!(!committed_t3.session_deleted);
    assert_eq!(
        database
            .session_sandbox_mode(&root, &t1.session_id)
            .expect("mode after t3 rewind"),
        Some("danger-full-access".into())
    );
    assert!(
        database
            .sandbox_rules(&root, &t1.session_id)
            .expect("rules cleared")
            .is_empty()
    );

    // 2) 再撤销第 2 轮（连同之后的切换 2）：应删掉切换 2 并回滚到存活切换 1 的 "read-only"，同时清空会话规则。
    database
        .insert_sandbox_rule(
            &root,
            &t1.session_id,
            &SessionSandboxRule {
                kind: SandboxRuleKind::Exact,
                tokens: vec!["echo".into(), "again".into()],
            },
        )
        .expect("rule t2");
    let rev_t2 = database.session_revision(&t1.session_id).expect("rev_t2");
    database
        .begin_operation(
            &root,
            "op-rw-t2",
            &t1.session_id,
            &t2.user_message_id,
            rev_t2,
            "delete_only",
            "turn 2",
        )
        .expect("begin op");
    let committed = database
        .commit_rewind(
            &root,
            &RewindCommit {
                operation_id: "op-rw-t2".into(),
                session_id: t1.session_id.clone(),
                target_user_message_id: t2.user_message_id.clone(),
                expected_revision: rev_t2,
                kind: "delete_only".into(),
                delete_conversation: true,
                revert_files: false,
                restored_file_count: 0,
                files_already_restored: false,
            },
        )
        .expect("commit t2");
    assert!(!committed.session_deleted);
    assert_eq!(
        database
            .session_sandbox_mode(&root, &t1.session_id)
            .expect("mode after t2 rewind"),
        Some("read-only".into())
    );
    assert!(
        database
            .sandbox_rules(&root, &t1.session_id)
            .expect("rules cleared")
            .is_empty()
    );

    // 另一会话：t_base_1 -> t_base_2 -> sw_base (workspace-write -> danger-full-access)，
    // 撤销 t_base_2 后全部切换被删，应恢复为 sw_base 的 from_mode = "workspace-write"。
    let t_base_1 = database
        .begin_turn(&root, None, "base 1", &model)
        .expect("tb1");
    database
        .finish(
            &root,
            &t_base_1.assistant_message_id,
            &[],
            &TurnOutcome::Complete {
                stop_reason: Some("stop".into()),
            },
            Usage::default(),
        )
        .expect("fb1");
    let t_base_2 = database
        .begin_turn(&root, Some(&t_base_1.session_id), "base 2", &model)
        .expect("tb2");
    database
        .finish(
            &root,
            &t_base_2.assistant_message_id,
            &[],
            &TurnOutcome::Complete {
                stop_reason: Some("stop".into()),
            },
            Usage::default(),
        )
        .expect("fb2");
    database
        .record_permission_switch(
            &root,
            &PermissionSwitchCommit {
                session_id: t_base_1.session_id.clone(),
                from_mode: "workspace-write".into(),
                to_mode: "danger-full-access".into(),
                injected_prompt: "Switched to Full".into(),
            },
        )
        .expect("sw_base");
    database
        .insert_sandbox_rule(
            &root,
            &t_base_1.session_id,
            &SessionSandboxRule {
                kind: SandboxRuleKind::Prefix,
                tokens: vec!["cargo".into(), "test".into()],
            },
        )
        .expect("rule 2");
    let rev2 = database
        .session_revision(&t_base_1.session_id)
        .expect("rev2");
    database
        .begin_operation(
            &root,
            "op-rw-tb2",
            &t_base_1.session_id,
            &t_base_2.user_message_id,
            rev2,
            "delete_only",
            "base 2",
        )
        .expect("begin op 2");
    let committed2 = database
        .commit_rewind(
            &root,
            &RewindCommit {
                operation_id: "op-rw-tb2".into(),
                session_id: t_base_1.session_id.clone(),
                target_user_message_id: t_base_2.user_message_id.clone(),
                expected_revision: rev2,
                kind: "delete_only".into(),
                delete_conversation: true,
                revert_files: false,
                restored_file_count: 0,
                files_already_restored: false,
            },
        )
        .expect("commit tb2");
    assert!(!committed2.session_deleted);
    assert_eq!(
        database
            .session_sandbox_mode(&root, &t_base_1.session_id)
            .expect("mode after all switches deleted"),
        Some("workspace-write".into())
    );
    assert!(
        database
            .permission_switches(&root, &t_base_1.session_id)
            .expect("switches")
            .is_empty()
    );
    assert!(
        database
            .sandbox_rules(&root, &t_base_1.session_id)
            .expect("rules cleared 2")
            .is_empty()
    );

    let _ = std::fs::remove_dir_all(root);
}
