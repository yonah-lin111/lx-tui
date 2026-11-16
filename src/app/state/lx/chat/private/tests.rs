use super::*;
use crate::db::chat::{ChatMessage, ChatPart};

fn message(id: &str, role: &str, text: &str) -> ChatMessage {
    ChatMessage {
        external_id: id.into(),
        sequence: 0,
        turn_index: 0,
        kind: "chat".into(),
        role: role.into(),
        status: "complete".into(),
        model_provider: None,
        model_id: None,
        variant: None,
        stop_reason: None,
        error_code: None,
        input_tokens: None,
        output_tokens: None,
        cache_read_tokens: None,
        cache_write_tokens: None,
        created_at_ms: 0,
        completed_at_ms: None,
        user_shell: None,
        subagent_reports: None,
        parts: vec![ChatPart {
            position: 0,
            kind: "text".into(),
            content: text.into(),
            provider_metadata_json: None,
            started_at_ms: None,
            ended_at_ms: None,
            completed_at_ms: None,
        }],
        tools: Vec::new(),
        attachments: Vec::new(),
    }
}

#[test]
fn mark_and_query_private_user_message() {
    let mut chat = LxChat::new();
    assert!(!chat.is_private_user_message("u1"));
    chat.mark_private_user_message("u1");
    assert!(chat.is_private_user_message("u1"));
}

#[test]
fn rebuild_skips_steer_report_and_missing_ids() {
    let messages = vec![
        message("spawn", "user", "spawn task"),
        message("private", "user", "private question"),
        message("report", "user", "[Report for main agent]\nstatus"),
        message("steer", "user", "mid-turn steer"),
        message("assistant", "assistant", "answer"),
    ];
    let mut chat = LxChat::new();
    let mut steer_messages = messages.clone();
    if let Some(steer) = steer_messages.iter_mut().find(|m| m.external_id == "steer") {
        steer.stop_reason = Some("steer".into());
    }
    chat.rebuild_steered_user_messages(&steer_messages);
    chat.rebuild_private_user_messages(
        &messages,
        &[
            "private".into(),
            "report".into(),
            "steer".into(),
            "missing".into(),
        ],
    );
    assert!(chat.is_private_user_message("private"), "面板追问必须标记");
    assert!(
        !chat.is_private_user_message("report"),
        "报告请求有独立来源标签"
    );
    assert!(!chat.is_private_user_message("steer"), "steer 已有独立底色");
    assert!(!chat.is_private_user_message("spawn"), "spawn 任务不是私聊");
    assert!(!chat.is_private_user_message("missing"));
}

#[test]
fn rebuild_without_candidates_clears_previous_marks() {
    let messages = vec![message("private", "user", "private")];
    let mut chat = LxChat::new();
    chat.mark_private_user_message("private");
    chat.rebuild_private_user_messages(&messages, &[]);
    assert!(!chat.is_private_user_message("private"));
}
