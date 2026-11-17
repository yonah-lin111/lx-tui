//! 旁路上下文边界：只读到所属 QA、不泄漏后续主 QA。

use std::path::PathBuf;

use super::*;
use crate::chat::events::ModelItem;
use crate::db::Database;
use crate::db::chat::{BtwTargetSpec, ModelSnapshot, TurnOutcome, Usage};

fn workspace(label: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "lx-btw-ctx-{label}-{}-{}",
        std::process::id(),
        crate::db::now_ms()
    ));
    std::fs::create_dir_all(&path).expect("workspace dir");
    path
}

fn model() -> ModelSnapshot {
    ModelSnapshot {
        provider: "test".into(),
        model: "model".into(),
        variant: None,
    }
}

fn finish(database: &Database, workspace: &std::path::Path, assistant: &str) {
    database
        .finish(
            workspace,
            assistant,
            &[],
            &TurnOutcome::Complete {
                stop_reason: Some("complete".into()),
            },
            Usage::default(),
        )
        .expect("finish");
}

fn text_of(item: &ModelItem) -> &str {
    match item {
        ModelItem::UserText(text) => text,
        _ => "",
    }
}

#[test]
fn btw_context_stops_at_the_owning_qa() {
    let workspace = workspace("cutoff");
    let database = Database::open_in_memory().expect("db");
    database.ensure_workspace(&workspace).expect("ws");
    let first = database
        .begin_turn(&workspace, None, "first question", &model())
        .expect("qa1");
    finish(&database, &workspace, &first.assistant_message_id);
    let second = database
        .begin_turn(
            &workspace,
            Some(&first.session_id),
            "later question",
            &model(),
        )
        .expect("qa2");
    finish(&database, &workspace, &second.assistant_message_id);

    let accepted = database
        .accept_btw_batch(
            &workspace,
            &first.session_id,
            &first.user_message_id,
            "side",
            "btw-ctx",
            "why did it fail?",
            &[BtwTargetSpec::default_assistant(model())],
        )
        .expect("accept");
    let start = database
        .start_btw_recipient(&workspace, &accepted.recipients[0].id)
        .expect("start")
        .expect("claim");
    let mcp = crate::mcp::McpToolSnapshot::default();
    let context = assemble_btw_context(BtwContextInput {
        database: &database,
        workspace: &workspace,
        parent_session_id: &first.session_id,
        qa_anchor_id: &first.user_message_id,
        child_session_id: &start.child_session_id,
        current_user_message_id: &start.user_message_id,
        question: "why did it fail?",
        answerer_label: "lx",
        references: &[],
        model: &model(),
        variant_params: None,
        cwd: &workspace,
        context_limit: None,
        mcp: &mcp,
        sandbox: &crate::config::sandbox::SandboxSettings::default(),
    })
    .expect("context");

    let items = context.items();
    assert_eq!(items.len(), 2);
    let reference = text_of(&items[0]);
    assert!(reference.contains("<main-reference>"));
    assert!(reference.contains("first question"));
    assert!(
        !reference.contains("later question"),
        "later main QA must not leak into the side thread"
    );
    assert_eq!(text_of(&items[1]), "why did it fail?");
    assert!(reference.contains("answerer=\"lx\""));
}

#[test]
fn btw_context_renders_prior_side_turns_but_not_the_current_one() {
    let workspace = workspace("history");
    let database = Database::open_in_memory().expect("db");
    database.ensure_workspace(&workspace).expect("ws");
    let qa = database
        .begin_turn(&workspace, None, "main question", &model())
        .expect("qa");
    finish(&database, &workspace, &qa.assistant_message_id);
    let first = database
        .accept_btw_batch(
            &workspace,
            &qa.session_id,
            &qa.user_message_id,
            "side",
            "btw-1",
            "first side question",
            &[BtwTargetSpec::default_assistant(model())],
        )
        .expect("accept 1");
    let start = database
        .start_btw_recipient(&workspace, &first.recipients[0].id)
        .expect("start")
        .expect("claim");
    database
        .finish_btw_recipient(
            &workspace,
            &first.recipients[0].id,
            &start.assistant_message_id,
            Some(&[]),
            &TurnOutcome::Complete {
                stop_reason: Some("complete".into()),
            },
            Usage::default(),
            Usage::default(),
        )
        .expect("settle");
    let second = database
        .accept_btw_batch(
            &workspace,
            &qa.session_id,
            &qa.user_message_id,
            "side",
            "btw-2",
            "second side question",
            &[BtwTargetSpec::default_assistant(model())],
        )
        .expect("accept 2");
    let start2 = database
        .start_btw_recipient(&workspace, &second.recipients[0].id)
        .expect("start 2")
        .expect("claim 2");
    let mcp = crate::mcp::McpToolSnapshot::default();
    let context = assemble_btw_context(BtwContextInput {
        database: &database,
        workspace: &workspace,
        parent_session_id: &qa.session_id,
        qa_anchor_id: &qa.user_message_id,
        child_session_id: &start2.child_session_id,
        current_user_message_id: &start2.user_message_id,
        question: "second side question",
        answerer_label: "lx",
        references: &[],
        model: &model(),
        variant_params: None,
        cwd: &workspace,
        context_limit: None,
        mcp: &mcp,
        sandbox: &crate::config::sandbox::SandboxSettings::default(),
    })
    .expect("context");
    let reference = text_of(&context.items()[0]);
    assert!(reference.contains("<btw-history>"));
    assert!(reference.contains("first side question"));
    assert!(
        !reference.contains("second side question"),
        "current batch must stay the last user item, not duplicated in history"
    );
}

#[test]
fn btw_context_inherits_parent_sandbox_settings() {
    let workspace = workspace("sandbox-btw");
    let database = Database::open_in_memory().expect("db");
    database.ensure_workspace(&workspace).expect("ws");
    let qa = database
        .begin_turn(&workspace, None, "main question", &model())
        .expect("qa");
    finish(&database, &workspace, &qa.assistant_message_id);
    let accepted = database
        .accept_btw_batch(
            &workspace,
            &qa.session_id,
            &qa.user_message_id,
            "side",
            "btw-sb",
            "what is my sandbox?",
            &[BtwTargetSpec::default_assistant(model())],
        )
        .expect("accept");
    let start = database
        .start_btw_recipient(&workspace, &accepted.recipients[0].id)
        .expect("start")
        .expect("claim");
    let mcp = crate::mcp::McpToolSnapshot::default();
    let ro = crate::config::sandbox::SandboxSettings {
        mode: crate::config::sandbox::SandboxMode::ReadOnly,
        ..Default::default()
    };
    let context = assemble_btw_context(BtwContextInput {
        database: &database,
        workspace: &workspace,
        parent_session_id: &qa.session_id,
        qa_anchor_id: &qa.user_message_id,
        child_session_id: &start.child_session_id,
        current_user_message_id: &start.user_message_id,
        question: "what is my sandbox?",
        answerer_label: "lx",
        references: &[],
        model: &model(),
        variant_params: None,
        cwd: &workspace,
        context_limit: None,
        mcp: &mcp,
        sandbox: &ro,
    })
    .expect("context");
    let system_prompt = context.system_prompt_text().expect("system prompt");
    assert!(
        system_prompt.contains(r#"<sandbox mode="read-only""#),
        "btw 环境快照应继承主会话沙箱模式: {system_prompt}"
    );
}
