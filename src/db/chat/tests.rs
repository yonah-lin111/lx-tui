//! 聊天持久化测试：轮次事务、checkpoint 覆盖、终态幂等、恢复与跨工作区隔离。

use std::path::PathBuf;

use super::*;

/// 独立临时目录中的数据库路径。
fn temp_db_path(label: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos());
    std::env::temp_dir().join(format!(
        "lx-tui-chat-test-{label}-{}",
        nanos.map(|value| format!("{value:x}")).unwrap_or_default()
    ))
}

fn open_test_database(label: &str) -> Database {
    Database::open(&temp_db_path(label)).expect("database opens")
}

fn workspace_path(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!("lx-tui-chat-ws-{label}"))
}

/// 测试模型快照。
fn model() -> ModelSnapshot {
    ModelSnapshot {
        provider: "example".into(),
        model: "example-chat".into(),
        variant: Some("high".into()),
    }
}

fn part(position: i64, kind: &str, content: &str) -> ChatPart {
    ChatPart {
        position,
        kind: kind.into(),
        content: content.into(),
        provider_metadata_json: None,
    }
}

/// 建库并信任两个工作区；返回（数据库，工作区 A，工作区 B）。
fn database_with_two_workspaces(label: &str) -> (Database, PathBuf, PathBuf) {
    let database = open_test_database(label);
    let a = workspace_path(&format!("{label}-a"));
    let b = workspace_path(&format!("{label}-b"));
    database.trust_workspace(&a, "a").expect("trust a");
    database.trust_workspace(&b, "b").expect("trust b");
    (database, a, b)
}

#[test]
fn first_turn_creates_session_user_assistant_and_preview() {
    let (database, workspace, _) = database_with_two_workspaces("first-turn");
    let committed = database
        .begin_turn(&workspace, None, "你好，世界", &model())
        .expect("begin turn");
    assert_eq!(committed.turn_index, 0);
    let messages = database
        .load_chat_session(&workspace, &committed.session_id)
        .expect("load")
        .expect("session exists");
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].role, "user");
    assert_eq!(messages[0].status, "complete");
    assert_eq!(messages[0].parts.len(), 1);
    assert_eq!(messages[0].parts[0].content, "你好，世界");
    assert_eq!(messages[0].parts[0].kind, "text");
    assert_eq!(messages[1].role, "assistant");
    assert_eq!(messages[1].status, "streaming");
    assert_eq!(
        messages[1].parts,
        Vec::new(),
        "streaming assistant 尚无 parts"
    );
    // session 标题取首条用户文本的单行截断。
    let sessions = database
        .list_chat_sessions(&workspace, 0, 10)
        .expect("list");
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].title, "你好，世界");
    assert_eq!(sessions[0].first_user_message, "你好，世界");
    assert_eq!(sessions[0].model_provider, "example");
}

#[test]
fn consecutive_turns_keep_sequence_and_turn_index_monotonic() {
    let (database, workspace, _) = database_with_two_workspaces("turns");
    let first = database
        .begin_turn(&workspace, None, "第一轮", &model())
        .expect("first");
    database
        .finish(
            &workspace,
            &first.assistant_message_id,
            &[part(0, "text", "好的")],
            &TurnOutcome::Complete {
                stop_reason: Some("stop".into()),
            },
            Usage {
                input: Some(10),
                output: Some(5),
                ..Default::default()
            },
        )
        .expect("finish first");
    let second = database
        .begin_turn(&workspace, Some(&first.session_id), "第二轮", &model())
        .expect("second");
    let messages = database
        .load_chat_session(&workspace, &first.session_id)
        .expect("load")
        .expect("exists");
    // 验收样例：sequence 0,1,2,3；turn_index 0,0,1,1。
    assert_eq!(
        messages
            .iter()
            .map(|message| message.sequence)
            .collect::<Vec<_>>(),
        vec![0, 1, 2, 3]
    );
    assert_eq!(
        messages
            .iter()
            .map(|message| message.turn_index)
            .collect::<Vec<_>>(),
        vec![0, 0, 1, 1]
    );
    assert_eq!(second.turn_index, 1);
}

#[test]
fn interrupted_first_turn_still_has_four_rows_and_second_turn_resumes() {
    let (database, workspace, _) = database_with_two_workspaces("partial");
    let first = database
        .begin_turn(&workspace, None, "第一轮", &model())
        .expect("first");
    // 第一轮停止：仍保留 user + partial assistant。
    database
        .checkpoint(
            &workspace,
            &first.assistant_message_id,
            &[
                part(0, "reasoning", "思考片段"),
                part(1, "text", "部分回复"),
            ],
        )
        .expect("checkpoint");
    database
        .finish(
            &workspace,
            &first.assistant_message_id,
            &[
                part(0, "reasoning", "思考片段"),
                part(1, "text", "部分回复"),
            ],
            &TurnOutcome::Interrupted,
            Usage::default(),
        )
        .expect("finish interrupted");
    let messages = database
        .load_chat_session(&workspace, &first.session_id)
        .expect("load")
        .expect("exists");
    assert_eq!(messages.len(), 2, "第一轮停止后仍有两条 message");
    assert_eq!(messages[1].status, "interrupted");
    // 第二轮照常进入。
    let second = database
        .begin_turn(&workspace, Some(&first.session_id), "第二轮", &model())
        .expect("second");
    assert_eq!(second.turn_index, 1);
    let all = database
        .load_chat_session(&workspace, &first.session_id)
        .expect("load")
        .expect("exists");
    assert_eq!(all.len(), 4);
}

#[test]
fn checkpoint_overwrites_parts_and_preserves_interleaved_order() {
    let (database, workspace, _) = database_with_two_workspaces("checkpoint");
    let committed = database
        .begin_turn(&workspace, None, "hello", &model())
        .expect("begin");
    // 交错 text -> reasoning -> text。
    database
        .checkpoint(
            &workspace,
            &committed.assistant_message_id,
            &[
                part(0, "text", "a"),
                part(1, "reasoning", "b"),
                part(2, "text", "c"),
            ],
        )
        .expect("checkpoint 1");
    // 后续 delta 合并到相邻同 kind part，快照覆盖后仍是交错顺序。
    database
        .checkpoint(
            &workspace,
            &committed.assistant_message_id,
            &[
                part(0, "text", "ab"),
                part(1, "reasoning", "bc"),
                part(2, "text", "cd"),
            ],
        )
        .expect("checkpoint 2");
    let messages = database
        .load_chat_session(&workspace, &committed.session_id)
        .expect("load")
        .expect("exists");
    let parts = &messages[1].parts;
    assert_eq!(
        parts
            .iter()
            .map(|part| (part.position, part.kind.as_str()))
            .collect::<Vec<_>>(),
        vec![(0, "text"), (1, "reasoning"), (2, "text")]
    );
    assert_eq!(parts[0].content, "ab");
    assert_eq!(parts[1].content, "bc");
    assert_eq!(parts[2].content, "cd");
}

#[test]
fn late_checkpoint_cannot_override_final_state() {
    let (database, workspace, _) = database_with_two_workspaces("late-checkpoint");
    let committed = database
        .begin_turn(&workspace, None, "hi", &model())
        .expect("begin");
    database
        .finish(
            &workspace,
            &committed.assistant_message_id,
            &[part(0, "text", "final")],
            &TurnOutcome::Complete { stop_reason: None },
            Usage::default(),
        )
        .expect("finish");
    // 迟到 checkpoint 必须被拒绝，不覆盖终态内容。
    let error = database
        .checkpoint(
            &workspace,
            &committed.assistant_message_id,
            &[part(0, "text", "stale")],
        )
        .expect_err("late checkpoint rejected");
    assert!(matches!(error, DbError::MessageNotStreaming));
    let messages = database
        .load_chat_session(&workspace, &committed.session_id)
        .expect("load")
        .expect("exists");
    assert_eq!(messages[1].parts[0].content, "final");
    assert_eq!(messages[1].status, "complete");
}

#[test]
fn duplicate_finish_is_idempotent_and_does_not_double_count_usage() {
    let (database, workspace, _) = database_with_two_workspaces("idempotent-finish");
    let committed = database
        .begin_turn(&workspace, None, "hi", &model())
        .expect("begin");
    let usage = Usage {
        input: Some(42),
        output: Some(7),
        ..Default::default()
    };
    for _ in 0..2 {
        database
            .finish(
                &workspace,
                &committed.assistant_message_id,
                &[part(0, "text", "done")],
                &TurnOutcome::Complete { stop_reason: None },
                usage,
            )
            .expect("finish idempotent");
    }
    let sessions = database
        .list_chat_sessions(&workspace, 0, 10)
        .expect("list");
    assert_eq!(sessions[0].total_input_tokens, 42, "usage 不重复累计");
    assert_eq!(sessions[0].total_output_tokens, 7);
}

#[test]
fn unknown_usage_stays_null_not_fake_zero() {
    let (database, workspace, _) = database_with_two_workspaces("unknown-usage");
    let committed = database
        .begin_turn(&workspace, None, "hi", &model())
        .expect("begin");
    database
        .finish(
            &workspace,
            &committed.assistant_message_id,
            &[part(0, "text", "done")],
            &TurnOutcome::Complete { stop_reason: None },
            Usage::default(),
        )
        .expect("finish");
    let messages = database
        .load_chat_session(&workspace, &committed.session_id)
        .expect("load")
        .expect("exists");
    assert_eq!(messages[1].input_tokens, None, "未知 usage 保持 NULL");
    assert_eq!(messages[1].output_tokens, None);
    let sessions = database
        .list_chat_sessions(&workspace, 0, 10)
        .expect("list");
    assert_eq!(sessions[0].total_input_tokens, 0);
}

#[test]
fn recover_interrupted_marks_only_streaming_rows() {
    let (database, workspace, _) = database_with_two_workspaces("recover");
    let first = database
        .begin_turn(&workspace, None, "one", &model())
        .expect("first");
    database
        .finish(
            &workspace,
            &first.assistant_message_id,
            &[part(0, "text", "ok")],
            &TurnOutcome::Complete { stop_reason: None },
            Usage::default(),
        )
        .expect("finish first");
    let second = database
        .begin_turn(&workspace, Some(&first.session_id), "two", &model())
        .expect("second");
    // 模拟异常退出后重开：遗留 streaming 改为 interrupted。
    let recovered = database.recover_interrupted_streams().expect("recover");
    assert_eq!(recovered, 1);
    let messages = database
        .load_chat_session(&workspace, &first.session_id)
        .expect("load")
        .expect("exists");
    assert_eq!(messages[1].status, "complete", "已完成的行不改动");
    assert_eq!(
        messages[3].status, "interrupted",
        "assistant external id = {}",
        second.assistant_message_id
    );
    // 幂等：第二次恢复无行可改。
    assert_eq!(database.recover_interrupted_streams().expect("again"), 0);
}

#[test]
fn cross_workspace_access_is_rejected() {
    let (database, workspace_a, workspace_b) = database_with_two_workspaces("isolation");
    let committed = database
        .begin_turn(&workspace_a, None, "secret", &model())
        .expect("begin in a");
    // B 工作区读不到 A 的会话。
    assert!(
        database
            .load_chat_session(&workspace_b, &committed.session_id)
            .expect("query ok")
            .is_none()
    );
    assert!(
        database
            .list_chat_sessions(&workspace_b, 0, 10)
            .expect("query ok")
            .is_empty()
    );
    // B 工作区不能 checkpoint/finish/rename/delete A 的消息。
    assert!(matches!(
        database.checkpoint(&workspace_b, &committed.assistant_message_id, &[]),
        Err(DbError::SessionMissing)
    ));
    assert!(matches!(
        database.finish(
            &workspace_b,
            &committed.assistant_message_id,
            &[],
            &TurnOutcome::Interrupted,
            Usage::default(),
        ),
        Err(DbError::SessionMissing)
    ));
    assert!(matches!(
        database.rename_chat_session(&workspace_b, &committed.session_id, "hack"),
        Err(DbError::SessionMissing)
    ));
    assert!(matches!(
        database.delete_chat_session(&workspace_b, &committed.session_id),
        Err(DbError::SessionMissing)
    ));
    // B 不能把消息续轮到 A 的 session。
    assert!(matches!(
        database.begin_turn(&workspace_b, Some(&committed.session_id), "x", &model()),
        Err(DbError::SessionMissing)
    ));
    // A 的数据原样保留。
    let sessions = database
        .list_chat_sessions(&workspace_a, 0, 10)
        .expect("list a");
    assert_eq!(sessions.len(), 1);
}

#[test]
fn begin_turn_rejects_blank_and_oversized_drafts() {
    let (database, workspace, _) = database_with_two_workspaces("draft-guard");
    assert!(matches!(
        database.begin_turn(&workspace, None, "   \n  ", &model()),
        Err(DbError::InvalidDraft)
    ));
    let oversized = "x".repeat(2 * 1024 * 1024 + 1);
    assert!(matches!(
        database.begin_turn(&workspace, None, &oversized, &model()),
        Err(DbError::MessageTooLarge)
    ));
    // 空草稿不产生 session 行。
    assert!(
        database
            .list_chat_sessions(&workspace, 0, 10)
            .expect("list")
            .is_empty()
    );
}

#[test]
fn begin_turn_on_unknown_session_fails_without_half_turn() {
    let (database, workspace, _) = database_with_two_workspaces("unknown-session");
    let error = database
        .begin_turn(&workspace, Some("sess-nonexistent"), "hi", &model())
        .expect_err("unknown session rejected");
    assert!(matches!(error, DbError::SessionMissing));
    assert!(
        database
            .list_chat_sessions(&workspace, 0, 10)
            .expect("list")
            .is_empty(),
        "不产生半轮"
    );
}

#[test]
fn delete_cascades_messages_parts_and_leaves_no_orphans() {
    let (database, workspace, _) = database_with_two_workspaces("cascade");
    let committed = database
        .begin_turn(&workspace, None, "hi", &model())
        .expect("begin");
    database
        .checkpoint(
            &workspace,
            &committed.assistant_message_id,
            &[part(0, "text", "partial")],
        )
        .expect("checkpoint");
    let session_id = committed.session_id.clone();
    drop(committed);
    database
        .delete_chat_session(&workspace, &session_id)
        .expect("delete");
    for table in ["agent_session", "agent_message", "agent_message_part"] {
        let count: i64 = database
            .connection()
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .expect("count");
        assert_eq!(count, 0, "{table} 无孤儿记录");
    }
    assert!(
        database
            .list_chat_sessions(&workspace, 0, 10)
            .expect("list")
            .is_empty()
    );
}

#[test]
fn rename_updates_title_but_not_session_reference() {
    let (database, workspace, _) = database_with_two_workspaces("rename");
    let committed = database
        .begin_turn(&workspace, None, "original question", &model())
        .expect("begin");
    database
        .rename_chat_session(&workspace, &committed.session_id, "自定义标题")
        .expect("rename");
    let sessions = database
        .list_chat_sessions(&workspace, 0, 10)
        .expect("list");
    assert_eq!(sessions[0].title, "自定义标题");
    assert_eq!(
        sessions[0].first_user_message, "original question",
        "预览不改写"
    );
    // external id 稳定：session 引用不因改名失效。
    assert_eq!(sessions[0].external_id, committed.session_id);
}

#[test]
fn set_chat_session_model_updates_selection_columns() {
    let (database, workspace, _) = database_with_two_workspaces("set-model");
    let committed = database
        .begin_turn(&workspace, None, "hi", &model())
        .expect("begin");
    let next = ModelSnapshot {
        provider: "other".into(),
        model: "other-model".into(),
        variant: None,
    };
    database
        .set_chat_session_model(&workspace, &committed.session_id, &next)
        .expect("set model");
    let loaded = database
        .chat_session_model(&workspace, &committed.session_id)
        .expect("query")
        .expect("exists");
    assert_eq!(loaded, next);
    // 历史 message 的模型快照不被篡改。
    let messages = database
        .load_chat_session(&workspace, &committed.session_id)
        .expect("load")
        .expect("exists");
    assert_eq!(
        messages[1].model_provider.as_deref(),
        Some("example"),
        "message 始终保存该轮实际模型"
    );
}

#[test]
fn list_sessions_orders_by_recent_activity() {
    let (database, workspace, _) = database_with_two_workspaces("ordering");
    for text in ["first", "second", "third"] {
        let committed = database
            .begin_turn(&workspace, None, text, &model())
            .expect("begin");
        database
            .finish(
                &workspace,
                &committed.assistant_message_id,
                &[part(0, "text", "ok")],
                &TurnOutcome::Complete { stop_reason: None },
                Usage::default(),
            )
            .expect("finish");
    }
    let sessions = database
        .list_chat_sessions(&workspace, 0, 10)
        .expect("list");
    let titles: Vec<&str> = sessions.iter().map(|s| s.title.as_str()).collect();
    assert_eq!(titles, vec!["third", "second", "first"]);
    // 分页。
    let page = database.list_chat_sessions(&workspace, 1, 2).expect("page");
    assert_eq!(page.len(), 2);
    assert_eq!(page[0].title, "second");
}

#[test]
fn provider_metadata_round_trips() {
    let (database, workspace, _) = database_with_two_workspaces("metadata");
    let committed = database
        .begin_turn(&workspace, None, "hi", &model())
        .expect("begin");
    // Anthropic signature 附加到对应 thinking part 的元数据。
    database
        .checkpoint(
            &workspace,
            &committed.assistant_message_id,
            &[ChatPart {
                position: 0,
                kind: "reasoning".into(),
                content: "thought".into(),
                provider_metadata_json: Some(r#"{"signature":"sig-abc"}"#.into()),
            }],
        )
        .expect("checkpoint");
    let messages = database
        .load_chat_session(&workspace, &committed.session_id)
        .expect("load")
        .expect("exists");
    assert_eq!(
        messages[1].parts[0].provider_metadata_json.as_deref(),
        Some(r#"{"signature":"sig-abc"}"#),
        "签名原样往返"
    );
}
