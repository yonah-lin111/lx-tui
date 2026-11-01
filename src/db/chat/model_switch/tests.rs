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
                model: model_c,
                from_label: Some("DeepSeek".into()),
                to_label: "GPT-5".into(),
                updated_sources: vec!["core/model".into()],
                injected_prompt: "prompt c".into(),
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
