use std::path::PathBuf;

use super::*;
use crate::db::chat::RewindCommit;

fn open_test_db(label: &str) -> (Database, PathBuf) {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "lx-model-switch-db-{label}-{}-{nanos}",
        std::process::id()
    ));
    let workspace = root.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace");
    let db = Database::open(&root.join("lx.db")).expect("open db");
    db.ensure_workspace(&workspace).expect("ensure workspace");
    (db, workspace)
}

/// 构造切换提交所需的模型身份（测试只关心提示词重算，不关心展示细节）。
fn identity(model: &ModelSnapshot, display_name: &str) -> crate::chat::agent::ModelIdentity {
    crate::chat::agent::ModelIdentity {
        display_name: display_name.into(),
        provider_id: model.provider.clone(),
        model_id: model.model.clone(),
        variant: model.variant.clone(),
        supports_image_input: false,
    }
}

#[test]
fn record_model_switch_allocates_shared_sequence_and_skips_unchanged() {
    let (db, workspace) = open_test_db("sequence");
    let initial_model = ModelSnapshot {
        provider: "anthropic".into(),
        model: "claude-sonnet-4".into(),
        variant: Some("high".into()),
    };
    let turn1 = db
        .begin_turn(&workspace, None, "hello", &initial_model)
        .expect("turn1");
    assert_eq!(turn1.user_sequence, 0);
    assert_eq!(turn1.assistant_sequence, 1);

    // 同一模型与等级重复设置不产生切换记录
    let noop = db
        .record_model_switch(
            &workspace,
            &ModelSwitchCommit {
                session_id: turn1.session_id.clone(),
                model: initial_model.clone(),
                from_label: Some("Claude Sonnet 4 (high)".into()),
                to_label: "Claude Sonnet 4".into(),
                updated_sources: vec!["core/model".into()],
                injected_prompt: String::new(),
                cwd: None,
                to_identity: identity(&initial_model, "Claude Sonnet 4"),
            },
        )
        .expect("noop switch");
    assert!(noop.is_none());

    // 切换到新模型：sequence 接在 assistant_sequence(1) 之后为 2
    let next_model = ModelSnapshot {
        provider: "9router".into(),
        model: "deepseek-v4.1-flash".into(),
        variant: Some("max".into()),
    };
    let sw1 = db
        .record_model_switch(
            &workspace,
            &ModelSwitchCommit {
                session_id: turn1.session_id.clone(),
                model: next_model.clone(),
                from_label: Some("Claude Sonnet 4 (high)".into()),
                to_label: "DeepSeek V4.1 Flash".into(),
                updated_sources: vec!["core/model".into(), "core/agents_md".into()],
                injected_prompt: "# Your Model\n- Name: DeepSeek V4.1 Flash".into(),
                cwd: None,
                to_identity: identity(&next_model, "DeepSeek V4.1 Flash"),
            },
        )
        .expect("record switch")
        .expect("created");
    assert_eq!(sw1.sequence, 2);
    assert!(!sw1.is_variant_only());
    assert_eq!(sw1.from_provider.as_deref(), Some("anthropic"));
    assert_eq!(sw1.to_provider, "9router");

    // 仅切换思考等级：sequence 递增为 3，且识别为 variant_only
    let variant_only_model = ModelSnapshot {
        provider: "9router".into(),
        model: "deepseek-v4.1-flash".into(),
        variant: Some("high".into()),
    };
    let sw2 = db
        .record_model_switch(
            &workspace,
            &ModelSwitchCommit {
                session_id: turn1.session_id.clone(),
                model: variant_only_model.clone(),
                from_label: Some("DeepSeek V4.1 Flash (max)".into()),
                to_label: "DeepSeek V4.1 Flash".into(),
                updated_sources: vec!["core/reasoning_effort".into()],
                injected_prompt: "# Your Model\n- Name: DeepSeek V4.1 Flash".into(),
                cwd: None,
                to_identity: identity(&variant_only_model, "DeepSeek V4.1 Flash"),
            },
        )
        .expect("record variant switch")
        .expect("created");
    assert_eq!(sw2.sequence, 3);
    assert!(sw2.is_variant_only());

    // 下一轮消息的 user_sequence 必须跨过模型切换 sequence(3) 分配为 4
    let turn2 = db
        .begin_turn(
            &workspace,
            Some(&turn1.session_id),
            "follow up",
            &variant_only_model,
        )
        .expect("turn2");
    assert_eq!(turn2.user_sequence, 4);
    assert_eq!(turn2.assistant_sequence, 5);

    let switches = db
        .model_switches(&workspace, &turn1.session_id)
        .expect("load switches");
    assert_eq!(switches.len(), 2);
    assert_eq!(switches[0].sequence, 2);
    assert_eq!(switches[1].sequence, 3);
}

#[test]
fn rewind_deletes_model_switches_at_or_after_rewound_turn() {
    let (db, workspace) = open_test_db("rewind");
    let model_a = ModelSnapshot {
        provider: "anthropic".into(),
        model: "claude-sonnet-4".into(),
        variant: Some("high".into()),
    };
    let model_b = ModelSnapshot {
        provider: "9router".into(),
        model: "deepseek-v4.1-flash".into(),
        variant: Some("max".into()),
    };
    let model_c = ModelSnapshot {
        provider: "openai".into(),
        model: "gpt-5".into(),
        variant: None,
    };

    let turn1 = db
        .begin_turn(&workspace, None, "turn 1", &model_a)
        .expect("turn1");
    let sw_before_turn2 = db
        .record_model_switch(
            &workspace,
            &ModelSwitchCommit {
                session_id: turn1.session_id.clone(),
                model: model_b.clone(),
                from_label: Some("Claude".into()),
                to_label: "DeepSeek".into(),
                updated_sources: vec!["core/model".into()],
                injected_prompt: "prompt b".into(),
                cwd: None,
                to_identity: identity(&model_b, "DeepSeek"),
            },
        )
        .expect("sw1")
        .expect("created");
    let turn2 = db
        .begin_turn(&workspace, Some(&turn1.session_id), "turn 2", &model_b)
        .expect("turn2");
    let _sw_after_turn2 = db
        .record_model_switch(
            &workspace,
            &ModelSwitchCommit {
                session_id: turn1.session_id.clone(),
                model: model_c.clone(),
                from_label: Some("DeepSeek".into()),
                to_label: "GPT-5".into(),
                updated_sources: vec!["core/model".into()],
                injected_prompt: "prompt c".into(),
                cwd: None,
                to_identity: identity(&model_c, "GPT-5"),
            },
        )
        .expect("sw2")
        .expect("created");

    // 回退第 2 轮（sequence >= turn2.user_sequence）：第 2 轮之后的切换应被清理，第 1 轮之后的切换保留
    db.begin_operation(
        &workspace,
        "op-rewind-2",
        &turn1.session_id,
        &turn2.user_message_id,
        turn2.revision,
        "delete_only",
        "turn 2",
    )
    .expect("begin_operation");
    db.commit_rewind(
        &workspace,
        &RewindCommit {
            operation_id: "op-rewind-2".into(),
            session_id: turn1.session_id.clone(),
            target_user_message_id: turn2.user_message_id,
            expected_revision: turn2.revision,
            delete_conversation: true,
            revert_files: false,
            kind: "delete_only".into(),
            restored_file_count: 0,
            files_already_restored: false,
        },
    )
    .expect("rewind turn2");

    let remaining = db
        .model_switches(&workspace, &turn1.session_id)
        .expect("remaining switches");
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].id, sw_before_turn2.id);
}

#[test]
fn rewind_restores_session_model_to_surviving_state() {
    let (db, workspace) = open_test_db("rewind-restore-model");
    let model_a = ModelSnapshot {
        provider: "anthropic".into(),
        model: "claude-sonnet-4".into(),
        variant: Some("high".into()),
    };
    let model_b = ModelSnapshot {
        provider: "9router".into(),
        model: "deepseek-v4.1-flash".into(),
        variant: Some("max".into()),
    };
    let model_c = ModelSnapshot {
        provider: "openai".into(),
        model: "gpt-5".into(),
        variant: None,
    };

    let turn1 = db
        .begin_turn(&workspace, None, "turn 1", &model_a)
        .expect("turn1");
    db.record_model_switch(
        &workspace,
        &ModelSwitchCommit {
            session_id: turn1.session_id.clone(),
            model: model_b.clone(),
            from_label: Some("Claude".into()),
            to_label: "DeepSeek".into(),
            updated_sources: vec!["core/model".into()],
            injected_prompt: "prompt b".into(),
            cwd: None,
            to_identity: identity(&model_b, "DeepSeek"),
        },
    )
    .expect("switch b")
    .expect("created");
    let turn2 = db
        .begin_turn(&workspace, Some(&turn1.session_id), "turn 2", &model_b)
        .expect("turn2");
    // 切到 C 但未产生消息：回退第 2 轮后该切换应被删除，模型回落到 B。
    db.record_model_switch(
        &workspace,
        &ModelSwitchCommit {
            session_id: turn1.session_id.clone(),
            model: model_c.clone(),
            from_label: Some("DeepSeek".into()),
            to_label: "GPT-5".into(),
            updated_sources: vec!["core/model".into()],
            injected_prompt: "prompt c".into(),
            cwd: None,
            to_identity: identity(&model_c, "GPT-5"),
        },
    )
    .expect("switch c")
    .expect("created");
    assert_eq!(
        db.chat_session_model(&workspace, &turn1.session_id)
            .expect("model before rewind"),
        Some(model_c.clone())
    );

    db.begin_operation(
        &workspace,
        "op-rewind-restore-model",
        &turn1.session_id,
        &turn2.user_message_id,
        turn2.revision,
        "delete_only",
        "turn 2",
    )
    .expect("begin_operation");
    db.commit_rewind(
        &workspace,
        &RewindCommit {
            operation_id: "op-rewind-restore-model".into(),
            session_id: turn1.session_id.clone(),
            target_user_message_id: turn2.user_message_id,
            expected_revision: turn2.revision,
            delete_conversation: true,
            revert_files: false,
            kind: "delete_only".into(),
            restored_file_count: 0,
            files_already_restored: false,
        },
    )
    .expect("rewind turn2");

    assert_eq!(
        db.chat_session_model(&workspace, &turn1.session_id)
            .expect("model after rewind"),
        Some(model_b.clone()),
        "回退后必须回滚到最后一条存活切换的目标模型"
    );

    // 没有切换记录时回落到最后一条存活 assistant 消息的模型。
    let turn3 = db
        .begin_turn(&workspace, None, "plain 1", &model_a)
        .expect("turn3");
    let turn4 = db
        .begin_turn(&workspace, Some(&turn3.session_id), "plain 2", &model_b)
        .expect("turn4");
    assert_eq!(
        db.chat_session_model(&workspace, &turn3.session_id)
            .expect("model before plain rewind"),
        Some(model_b.clone())
    );
    db.begin_operation(
        &workspace,
        "op-rewind-plain",
        &turn3.session_id,
        &turn4.user_message_id,
        turn4.revision,
        "delete_only",
        "plain 2",
    )
    .expect("begin_operation plain");
    db.commit_rewind(
        &workspace,
        &RewindCommit {
            operation_id: "op-rewind-plain".into(),
            session_id: turn3.session_id.clone(),
            target_user_message_id: turn4.user_message_id,
            expected_revision: turn4.revision,
            delete_conversation: true,
            revert_files: false,
            kind: "delete_only".into(),
            restored_file_count: 0,
            files_already_restored: false,
        },
    )
    .expect("rewind plain");

    assert_eq!(
        db.chat_session_model(&workspace, &turn3.session_id)
            .expect("model after plain rewind"),
        Some(model_a),
        "无切换记录时回落到最后一条存活 assistant 消息的模型"
    );
}

#[test]
fn v21_upgrade_creates_agent_model_switch_table_and_cascades() {
    let (db, workspace) = open_test_db("v22-upgrade");
    let model_a = ModelSnapshot {
        provider: "anthropic".into(),
        model: "claude-sonnet-4".into(),
        variant: None,
    };
    let model_b = ModelSnapshot {
        provider: "9router".into(),
        model: "deepseek-v4.1-flash".into(),
        variant: Some("max".into()),
    };
    let model_c = ModelSnapshot {
        provider: "openai".into(),
        model: "gpt-5".into(),
        variant: None,
    };
    let turn = db
        .begin_turn(&workspace, None, "hello", &model_a)
        .expect("turn");
    let db_path = workspace.parent().expect("root").join("lx.db");

    // 模拟 v21 库：v22 表尚不存在。
    db.connection()
        .execute_batch("DROP TABLE agent_model_switch; PRAGMA user_version = 21;")
        .expect("downgrade to v21");
    drop(db);

    let database = Database::open(&db_path).expect("reopen v21");
    let version: i64 = database
        .connection()
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .expect("version");
    assert_eq!(version, crate::db::SCHEMA_VERSION_FOR_TESTS);
    let created: i64 = database
        .connection()
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'agent_model_switch'",
            [],
            |row| row.get(0),
        )
        .expect("table lookup");
    assert_eq!(created, 1, "v21 -> v22 迁移必须补建 agent_model_switch");

    // 级联删除：会话删除时切换记录随之清理。
    let switch = database
        .record_model_switch(
            &workspace,
            &ModelSwitchCommit {
                session_id: turn.session_id.clone(),
                model: model_b,
                from_label: Some("Claude".into()),
                to_label: "DeepSeek".into(),
                updated_sources: vec!["core/model".into()],
                injected_prompt: "prompt".into(),
                cwd: None,
                to_identity: identity(&model_c, "GPT-5"),
            },
        )
        .expect("record switch")
        .expect("created");
    assert_eq!(switch.to_provider, "9router");
    database
        .delete_chat_session(&workspace, &turn.session_id)
        .expect("delete session");
    let remaining: i64 = database
        .connection()
        .query_row(
            "SELECT COUNT(*) FROM agent_model_switch WHERE session_id = ?1",
            rusqlite::params![turn.session_id],
            |row| row.get(0),
        )
        .expect("count switches");
    assert_eq!(remaining, 0, "删除会话必须级联删除切换记录");
}
