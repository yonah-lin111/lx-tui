//! `lx-tui session` 子命令与跨进程会话锁单元测试。

use crate::chat::agent::AgentRole;
use crate::db::chat::{ChatPart, CompactCommit, ModelSnapshot, TurnOutcome, Usage};

use super::{CliTestEnv, assert_exit_ok};

fn sample_model() -> ModelSnapshot {
    ModelSnapshot {
        provider: "openai".to_string(),
        model: "gpt-5.4".to_string(),
        variant: Some("high".to_string()),
    }
}

fn seed_completed_turn(
    env: &CliTestEnv,
    existing_session: Option<&str>,
    user_text: &str,
    assistant_text: &str,
) -> (String, String, String) {
    let db = env.open_db();
    let turn = db
        .begin_turn(&env.workspace, existing_session, user_text, &sample_model())
        .expect("begin turn");
    let parts = [ChatPart {
        position: 0,
        kind: "text".to_string(),
        content: assistant_text.to_string(),
        started_at_ms: Some(100),
        completed_at_ms: Some(200),
        ended_at_ms: Some(200),
        provider_metadata_json: None,
    }];
    db.finish(
        &env.workspace,
        &turn.assistant_message_id,
        &parts,
        &TurnOutcome::Complete {
            stop_reason: Some("stop".to_string()),
        },
        Usage {
            input: Some(120),
            output: Some(45),
            cache_read: None,
            cache_write: None,
        },
    )
    .expect("finish turn");
    (
        turn.session_id,
        turn.user_message_id,
        turn.assistant_message_id,
    )
}

#[test]
fn session_list_show_rename_fork_and_delete_work_with_id_prefix_and_last() {
    let env = CliTestEnv::new("session-crud");
    let (session_id, _u1, _a1) =
        seed_completed_turn(&env, None, "First question", "First response");
    let (_, u2, _a2) = seed_completed_turn(
        &env,
        Some(&session_id),
        "Second question",
        "Second response",
    );

    assert_exit_ok(env.exec(&["session", "list"]));
    assert_exit_ok(env.exec(&["session", "list", "--json"]));
    assert_exit_ok(env.exec(&["session", "show", "--last", "--tools", "--thinking"]));
    assert_exit_ok(env.exec(&["session", "show", &session_id, "--json"]));

    // Rename using --last
    assert_exit_ok(env.exec(&["session", "rename", "--last", "Renamed CLI Session"]));
    let db = env.open_db();
    assert_eq!(
        db.chat_session_title(&env.workspace, &session_id)
            .expect("title")
            .as_deref(),
        Some("Renamed CLI Session")
    );

    // Fork before second user message first (while `--last` is still `session_id`), then fork full session
    assert_exit_ok(env.exec(&["session", "fork", "--last", "--before", &u2, "--json"]));
    assert_exit_ok(env.exec(&["session", "fork", &session_id]));
    let all_sessions = db
        .list_chat_sessions(&env.workspace, 0, 20)
        .expect("list sessions");
    assert_eq!(all_sessions.len(), 3);

    // Delete original session
    assert_exit_ok(env.exec(&["session", "delete", &session_id]));
    assert!(
        db.load_chat_session(&env.workspace, &session_id)
            .expect("load deleted")
            .is_none()
    );
}

#[test]
fn session_lock_blocks_delete_rewind_and_compact_when_active() {
    let env = CliTestEnv::new("session-lock");
    let (session_id, _, _) = seed_completed_turn(&env, None, "Locked turn", "Locked answer");

    // Hold the cross-process session lock to simulate an active turn in TUI or exec
    let held_lock = crate::platform::try_lock_session(&session_id)
        .expect("lock")
        .expect("acquire session lock");

    // Read-only and safe derived commands must still succeed while locked
    assert_exit_ok(env.exec(&["session", "list"]));
    assert_exit_ok(env.exec(&["session", "show", &session_id]));
    assert_exit_ok(env.exec(&["session", "rename", &session_id, "Still Renamable"]));

    // Destructive/mutating commands must fail immediately while locked
    let del_err = env
        .exec(&["session", "delete", &session_id])
        .expect_err("delete must fail when locked");
    assert!(del_err.contains("currently active"));

    let rewind_err = env
        .exec(&["session", "rewind", &session_id, "--mode", "delete-only"])
        .expect_err("rewind must fail when locked");
    assert!(
        rewind_err.contains("running this session") || rewind_err.contains("currently active"),
        "unexpected rewind error: {rewind_err}"
    );

    let compact_err = env
        .exec(&["session", "compact", &session_id, "--revert"])
        .expect_err("compact must fail when locked");
    assert!(compact_err.contains("currently active"));

    drop(held_lock);

    // Once released, delete succeeds
    assert_exit_ok(env.exec(&["session", "delete", &session_id]));
}

#[test]
fn session_rewind_dry_run_delete_only_and_compact_revert_work() {
    let env = CliTestEnv::new("session-rewind-compact");
    let (session_id, _, _) = seed_completed_turn(&env, None, "Turn 1", "Reply 1");
    let (_, _, _) = seed_completed_turn(&env, Some(&session_id), "Turn 2", "Reply 2");

    // Dry-run rewind inspects latest user turn without deleting
    assert_exit_ok(env.exec(&["session", "rewind", "--last", "--dry-run"]));
    assert_exit_ok(env.exec(&["session", "rewind", "--last", "--dry-run", "--json"]));

    let db = env.open_db();
    assert_eq!(
        db.load_chat_session(&env.workspace, &session_id)
            .expect("load")
            .expect("exists")
            .len(),
        4
    );

    // Commit a compact checkpoint and test `session compact --revert` and `rewind --mode restore-compact`
    let rev = db.session_revision(&session_id).expect("revision");
    db.commit_compact(
        &env.workspace,
        &CompactCommit {
            session_id: session_id.clone(),
            expected_revision: rev,
            base_checkpoint_id: None,
            start_sequence: 1,
            end_sequence: 2,
            summary_text: "Summary of turn 1".to_string(),
            model_label: "gpt-5.4".to_string(),
            operation_id: "op-compact-1".to_string(),
        },
    )
    .expect("commit compact");
    assert!(
        db.active_checkpoint(&env.workspace, &session_id)
            .expect("active")
            .is_some()
    );

    assert_exit_ok(env.exec(&["session", "compact", &session_id, "--revert"]));
    assert!(
        db.active_checkpoint(&env.workspace, &session_id)
            .expect("reverted")
            .is_none()
    );

    // Execute delete-only rewind on latest turn
    assert_exit_ok(env.exec(&[
        "session",
        "rewind",
        &session_id,
        "--mode",
        "delete-only",
        "--json",
    ]));
    assert_eq!(
        db.load_chat_session(&env.workspace, &session_id)
            .expect("load after rewind")
            .expect("exists")
            .len(),
        2
    );
}

#[test]
fn session_list_rejects_zero_limit_and_rename_last_requires_title_for_session_ids() {
    let env = CliTestEnv::new("session-limit-rename");
    let (session_id, _, _) = seed_completed_turn(&env, None, "Question", "Answer");

    let err = env
        .exec(&["session", "list", "--limit", "0"])
        .expect_err("zero limit must be rejected");
    assert!(err.contains("at least 1"), "unexpected error: {err}");
    let err = env
        .exec(&["session", "list", "--limit=0"])
        .expect_err("zero limit must be rejected");
    assert!(err.contains("at least 1"), "unexpected error: {err}");

    // A session-id-shaped positional after `--last` is ambiguous, not a title.
    let err = env
        .exec(&["session", "rename", "--last", "sess-abc"])
        .expect_err("session-id-shaped title must be rejected");
    assert!(err.contains("--title"), "unexpected error: {err}");

    // `--title` disambiguates and succeeds without touching the session ID.
    assert_exit_ok(env.exec(&["session", "rename", "--last", "--title", "New Title"]));
    let db = env.open_db();
    assert_eq!(
        db.chat_session_title(&env.workspace, &session_id)
            .expect("title")
            .as_deref(),
        Some("New Title")
    );
}

#[test]
fn rewind_dry_run_restore_compact_requires_active_checkpoint() {
    let env = CliTestEnv::new("session-rewind-precheck");
    let (session_id, _, _) = seed_completed_turn(&env, None, "Turn", "Reply");

    let err = env
        .exec(&[
            "session",
            "rewind",
            "--last",
            "--dry-run",
            "--mode",
            "restore-compact",
        ])
        .expect_err("missing checkpoint must be rejected");
    assert!(
        err.contains("no active compact checkpoint"),
        "unexpected error: {err}"
    );

    let db = env.open_db();
    let rev = db.session_revision(&session_id).expect("revision");
    db.commit_compact(
        &env.workspace,
        &CompactCommit {
            session_id: session_id.clone(),
            expected_revision: rev,
            base_checkpoint_id: None,
            start_sequence: 1,
            end_sequence: 2,
            summary_text: "Compact summary".to_string(),
            model_label: "gpt-5.4".to_string(),
            operation_id: "op-rewind-dry-run".to_string(),
        },
    )
    .expect("commit compact");

    // Dry run only inspects the checkpoint and must not revert it.
    assert_exit_ok(env.exec(&[
        "session",
        "rewind",
        "--last",
        "--dry-run",
        "--mode",
        "restore-compact",
    ]));
    assert_exit_ok(env.exec(&[
        "session",
        "rewind",
        "--last",
        "--dry-run",
        "--mode",
        "restore-compact",
        "--json",
    ]));
    assert!(
        db.active_checkpoint(&env.workspace, &session_id)
            .expect("active checkpoint")
            .is_some()
    );
}

#[test]
fn session_agents_and_processes_list_subagents_and_process_records() {
    let env = CliTestEnv::new("session-agents");
    let (session_id, _, assistant_id) =
        seed_completed_turn(&env, None, "Spawn explorer", "Done spawning");

    let db = env.open_db();
    let sub = db
        .begin_subagent_turn(
            &env.workspace,
            &session_id,
            AgentRole::Explorer,
            "Inspect CLI module",
            "Check src/cli files",
            &sample_model(),
            Some(&assistant_id),
        )
        .expect("begin subagent");
    db.finish_subagent_turn(
        &env.workspace,
        &sub.agent_id,
        &sub.child_assistant_id,
        &[],
        &TurnOutcome::Complete { stop_reason: None },
        Usage::default(),
        Usage::default(),
        None,
    )
    .expect("finish subagent");

    assert_exit_ok(env.exec(&["session", "agents", &session_id]));
    assert_exit_ok(env.exec(&[
        "session", "agents", "--last", "--tree", "--filter", "done", "--json",
    ]));
    assert_exit_ok(env.exec(&["session", "processes", &session_id, "--json"]));
}
