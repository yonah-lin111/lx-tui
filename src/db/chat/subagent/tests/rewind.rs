//! 详情 `/rewind` 的可撤回判据：认人类面板私聊追问与新版面板 `/report`；
//! spawn / 父 `send_input` / 旧版自动投递报告 / steer 段拒绝。

use super::*;

fn text_item(text: &str) -> [PersistedInputItem; 1] {
    [PersistedInputItem::Text { text: text.into() }]
}

fn complete() -> TurnOutcome {
    TurnOutcome::Complete {
        stop_reason: Some("complete".into()),
    }
}

/// 人类面板私聊追问（position>0、无 parent_call_id、notify_parent=0）可撤回。
#[test]
fn latest_undoable_turn_requires_human_panel_follow_up() {
    let fixture = Fixture::new("undo-human");
    let parent = fixture.parent();
    let committed = fixture.spawn(&parent, AgentRole::Explorer, "task");
    fixture.finish_child(&committed, complete(), None);
    let input = fixture
        .database
        .enqueue_input(
            fixture.workspace(),
            &parent,
            &committed.agent_id,
            "private-1",
            None,
            None,
            &text_item("private question"),
            false,
            false,
        )
        .expect("enqueue private");
    let claimed = fixture
        .database
        .claim_input(
            fixture.workspace(),
            &parent,
            &committed.agent_id,
            &input.submission_id,
            &Fixture::model(),
        )
        .expect("claim private");
    let turn = fixture
        .database
        .latest_undoable_private_turn(fixture.workspace(), &committed.child_session_id)
        .expect("query")
        .expect("human follow-up is undoable");
    assert_eq!(turn.user_message_id, claimed.child_user_message_id);
    cleanup(fixture.root);
}

/// 新版面板 `/report` 轮（`notify_parent=0`）是可撤回的普通私有轮次。
#[test]
fn panel_report_turn_is_undoable() {
    let fixture = Fixture::new("undo-panel-report");
    let parent = fixture.parent();
    let committed = fixture.spawn(&parent, AgentRole::Explorer, "task");
    fixture.finish_child(&committed, complete(), None);
    let input = fixture
        .database
        .enqueue_input(
            fixture.workspace(),
            &parent,
            &committed.agent_id,
            "panel-report-1",
            None,
            None,
            &text_item("[Report for main agent]\nstatus"),
            false,
            false,
        )
        .expect("enqueue report");
    let claimed = fixture
        .database
        .claim_input(
            fixture.workspace(),
            &parent,
            &committed.agent_id,
            &input.submission_id,
            &Fixture::model(),
        )
        .expect("claim report");
    let turn = fixture
        .database
        .latest_undoable_private_turn(fixture.workspace(), &committed.child_session_id)
        .expect("query")
        .expect("panel report is undoable");
    assert_eq!(turn.user_message_id, claimed.child_user_message_id);
    cleanup(fixture.root);
}

/// spawn 初始任务、父 agent `send_input` 与旧版 `/report` 轮都不可撤回。
#[test]
fn spawn_input_and_parent_send_input_and_report_turns_are_not_undoable() {
    let fixture = Fixture::new("undo-nonhuman");
    let parent = fixture.parent();
    let committed = fixture.spawn(&parent, AgentRole::Explorer, "task");
    // spawn 输入的 position = 0。
    assert!(
        fixture
            .database
            .latest_undoable_private_turn(fixture.workspace(), &committed.child_session_id)
            .expect("query")
            .is_none(),
        "spawn turn is not undoable"
    );
    fixture.finish_child(&committed, complete(), None);

    // 模型 send_input：parent_call_id 非空。
    let tool = fixture
        .database
        .enqueue_input(
            fixture.workspace(),
            &parent,
            &committed.agent_id,
            "tool-1",
            Some("call-1"),
            None,
            &text_item("tool follow-up"),
            false,
            true,
        )
        .expect("enqueue tool");
    let tool_claimed = fixture
        .database
        .claim_input(
            fixture.workspace(),
            &parent,
            &committed.agent_id,
            &tool.submission_id,
            &Fixture::model(),
        )
        .expect("claim tool");
    assert!(
        fixture
            .database
            .latest_undoable_private_turn(fixture.workspace(), &committed.child_session_id)
            .expect("query")
            .is_none(),
        "parent send_input is not undoable"
    );
    fixture
        .database
        .finish_subagent_turn(
            fixture.workspace(),
            &committed.agent_id,
            &tool_claimed.child_assistant_id,
            &[part(0, "tool done")],
            &complete(),
            Usage::default(),
            Usage::default(),
            None,
        )
        .expect("finish tool");

    // 显式 /report：notify_parent = 1。
    let report = fixture
        .database
        .enqueue_input(
            fixture.workspace(),
            &parent,
            &committed.agent_id,
            "report-1",
            None,
            None,
            &text_item("[Report to parent agent]\nstatus"),
            false,
            true,
        )
        .expect("enqueue report");
    fixture
        .database
        .claim_input(
            fixture.workspace(),
            &parent,
            &committed.agent_id,
            &report.submission_id,
            &Fixture::model(),
        )
        .expect("claim report");
    assert!(
        fixture
            .database
            .latest_undoable_private_turn(fixture.workspace(), &committed.child_session_id)
            .expect("query")
            .is_none(),
        "report turn is not undoable"
    );
    cleanup(fixture.root);
}

/// steer 段只覆盖截断之后的内容，不能按“最后一轮”撤回。
#[test]
fn steered_turn_is_not_undoable() {
    let fixture = Fixture::new("undo-steer");
    let parent = fixture.parent();
    let committed = fixture.spawn(&parent, AgentRole::Explorer, "task");
    fixture.finish_child(&committed, complete(), None);
    let input = fixture
        .database
        .enqueue_input(
            fixture.workspace(),
            &parent,
            &committed.agent_id,
            "private-1",
            None,
            None,
            &text_item("private question"),
            false,
            false,
        )
        .expect("enqueue private");
    let claimed = fixture
        .database
        .claim_input(
            fixture.workspace(),
            &parent,
            &committed.agent_id,
            &input.submission_id,
            &Fixture::model(),
        )
        .expect("claim private");
    fixture
        .database
        .begin_steer_turn(
            fixture.workspace(),
            &committed.child_session_id,
            &claimed.child_assistant_id,
            &[],
            "steer this turn",
            &[],
            &Fixture::model(),
        )
        .expect("steer");
    assert!(
        fixture
            .database
            .latest_undoable_private_turn(fixture.workspace(), &committed.child_session_id)
            .expect("query")
            .is_none(),
        "steer segment is not undoable"
    );
    cleanup(fixture.root);
}
