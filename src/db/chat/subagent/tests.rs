//! 子 agent 持久化测试：spawn 事务、列表过滤、报告行回放、恰好一次、级联与恢复。

use std::path::{Path, PathBuf};

use super::*;
use crate::chat::agent::AgentRole;
use crate::db::Database;
use crate::db::chat::SESSION_PAGE_SIZE;
use crate::db::chat::{ChatPart, ModelSnapshot, TurnOutcome};
use rusqlite::params;

/// 独立临时数据库 + 工作区。
struct Fixture {
    root: PathBuf,
    workspace: PathBuf,
    database: Database,
}

impl Fixture {
    fn new(label: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "lx-subagent-db-{}-{label}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|value| value.as_nanos())
                .unwrap_or_default()
        ));
        let workspace = root.join("ws");
        std::fs::create_dir_all(&workspace).expect("workspace");
        let database = Database::open(&root.join("lx.db")).expect("open db");
        database
            .trust_workspace(&workspace, "ws")
            .expect("trust workspace");
        Self {
            root,
            workspace,
            database,
        }
    }

    fn workspace(&self) -> &Path {
        &self.workspace
    }

    fn model() -> ModelSnapshot {
        ModelSnapshot {
            provider: "p".into(),
            model: "m".into(),
            variant: None,
        }
    }

    /// 建一个父会话并返回其 id（首轮已完整结束，便于测试回放）。
    fn parent(&self) -> String {
        let committed = self
            .database
            .begin_turn(self.workspace(), None, "parent question", &Self::model())
            .expect("begin parent");
        self.database
            .finish(
                self.workspace(),
                &committed.assistant_message_id,
                &[part(0, "parent answer")],
                &TurnOutcome::Complete {
                    stop_reason: Some("complete".into()),
                },
                Usage::default(),
            )
            .expect("finish parent");
        committed.session_id
    }

    fn spawn(&self, parent: &str, role: AgentRole, task: &str) -> SubagentCommitted {
        self.database
            .begin_subagent_turn(
                self.workspace(),
                parent,
                role,
                task.lines().next().unwrap_or(task),
                task,
                &Self::model(),
                None,
            )
            .expect("begin subagent")
    }

    fn finish_child(
        &self,
        committed: &SubagentCommitted,
        outcome: TurnOutcome,
        report: Option<&ReportPayload>,
    ) {
        self.database
            .finish_subagent_turn(
                self.workspace(),
                &committed.agent_id,
                &committed.child_assistant_id,
                &[part(0, "child done")],
                &outcome,
                Usage {
                    input: Some(10),
                    output: Some(5),
                    ..Usage::default()
                },
                report,
            )
            .expect("finish subagent");
    }
}

fn cleanup(root: PathBuf) {
    let _ = std::fs::remove_dir_all(root);
}

fn part(position: i64, text: &str) -> ChatPart {
    ChatPart {
        position,
        kind: "text".into(),
        content: text.into(),
        provider_metadata_json: None,
        started_at_ms: None,
        ended_at_ms: None,
        completed_at_ms: None,
    }
}

fn report() -> ReportPayload {
    ReportPayload {
        title: "compact entry".into(),
        body: "found the entry point".into(),
        files_changed: 2,
        duration_ms: Some(1_200),
    }
}

#[test]
fn begin_subagent_turn_writes_child_session_and_relation() {
    let fixture = Fixture::new("begin");
    let spawn_message = fixture
        .database
        .begin_turn(fixture.workspace(), None, "spawn it", &Fixture::model())
        .expect("parent turn")
        .assistant_message_id;
    let parent = fixture.parent();
    let committed = fixture
        .database
        .begin_subagent_turn(
            fixture.workspace(),
            &parent,
            AgentRole::Explorer,
            "find the compact entry",
            "find the compact entry\nwith details",
            &Fixture::model(),
            Some(&spawn_message),
        )
        .expect("spawn");
    assert!(committed.agent_id.starts_with("sub"));
    assert_ne!(committed.child_session_id, parent);

    let records = fixture
        .database
        .list_subagents(fixture.workspace(), &parent)
        .expect("list");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].agent_id, committed.agent_id);
    assert_eq!(records[0].role, AgentRole::Explorer);
    assert_eq!(records[0].status, "running");
    assert_eq!(
        records[0].spawn_message_id.as_deref(),
        Some(spawn_message.as_str())
    );
    assert_eq!(records[0].title, "find the compact entry");

    let messages = fixture
        .database
        .load_chat_session(fixture.workspace(), &committed.child_session_id)
        .expect("load")
        .expect("child exists");
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].role, "user");
    assert_eq!(
        messages[0].parts[0].content,
        "find the compact entry\nwith details"
    );
    assert_eq!(messages[1].status, "streaming");
}

#[test]
fn list_chat_sessions_excludes_subagent_sessions() {
    let fixture = Fixture::new("list");
    let parent = fixture.parent();
    fixture.spawn(&parent, AgentRole::Explorer, "look around");
    let sessions = fixture
        .database
        .list_chat_sessions(fixture.workspace(), 0, SESSION_PAGE_SIZE)
        .expect("list sessions");
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].external_id, parent);
}

#[test]
fn report_message_and_turn_replay_in_one_turn() {
    let fixture = Fixture::new("report");
    let parent = fixture.parent();
    let committed = fixture.spawn(&parent, AgentRole::Explorer, "investigate");
    fixture.finish_child(
        &committed,
        TurnOutcome::Complete {
            stop_reason: Some("complete".into()),
        },
        Some(&report()),
    );
    let view = fixture
        .database
        .list_subagents(fixture.workspace(), &parent)
        .expect("list")[0]
        .report_json
        .as_deref()
        .and_then(crate::chat::agent::SubagentReportView::from_json)
        .expect("report view");
    let reports = vec![view];
    let envelope = format!(
        "[local runtime] subagent reports:\n{}",
        reports
            .iter()
            .map(crate::chat::agent::format_report_envelope)
            .collect::<Vec<_>>()
            .join("\n\n")
    );
    let turn = fixture
        .database
        .begin_report_batch(
            fixture.workspace(),
            &parent,
            &envelope,
            &crate::chat::agent::reports_to_json(&reports),
            std::slice::from_ref(&committed.agent_id),
            &Fixture::model(),
        )
        .expect("begin report batch");

    let messages = fixture
        .database
        .load_chat_session(fixture.workspace(), &parent)
        .expect("load")
        .expect("parent");
    let report_row = messages
        .iter()
        .find(|message| message.external_id == turn.user_message_id)
        .expect("report row");
    let stored = report_row
        .subagent_reports
        .as_deref()
        .expect("reports json");
    let parsed = crate::chat::agent::reports_from_json(stored);
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].agent_id, committed.agent_id);
    assert_eq!(report_row.turn_index, turn.turn_index);
    assert!(
        fixture
            .database
            .list_subagents(fixture.workspace(), &parent)
            .expect("list")[0]
            .report_delivered,
        "批次提交即视为已投递，失败重试不会重复生成卡片"
    );
    let assistant = messages
        .iter()
        .find(|message| message.external_id == turn.assistant_message_id)
        .expect("assistant");
    assert_eq!(assistant.turn_index, turn.turn_index);

    // 只补 assistant 还没结束时不可回放；结束后同 turn_index 的 user+assistant 一起回放。
    fixture
        .database
        .finish(
            fixture.workspace(),
            &turn.assistant_message_id,
            &[part(0, "report received")],
            &TurnOutcome::Complete {
                stop_reason: Some("complete".into()),
            },
            Usage::default(),
        )
        .expect("finish report turn");
    let messages = fixture
        .database
        .load_chat_session(fixture.workspace(), &parent)
        .expect("load")
        .expect("parent");
    let assembly = crate::chat::context::assemble(&crate::chat::context::AssemblyInput {
        messages: &messages,
        draft: "next question",
        draft_attachments: &[],
        model: &Fixture::model(),
        context_limit: None,
        output_limit: None,
        variant_params: None,
        compact: None,
        environment: None,
    });
    let text = format!("{:?}", assembly.request.items);
    assert!(text.contains("subagent_report"), "{text}");
    assert!(text.contains("report received"), "{text}");
}

/// 子轮必须有自己的文件 scope：executor 的 mutation 依赖它，缺了会被整批拒绝。
#[test]
fn child_turn_has_a_file_scope() {
    let fixture = Fixture::new("scope");
    let parent = fixture.parent();
    let committed = fixture.spawn(&parent, AgentRole::Executor, "fix it");
    let state: String = fixture
        .database
        .connection()
        .query_row(
            "SELECT state FROM agent_turn_file_scope WHERE user_message_id = ?1",
            params![committed.child_user_message_id],
            |row| row.get(0),
        )
        .expect("scope row");
    assert_eq!(state, "capturing");
}

#[test]
fn tokens_do_not_leak_from_child_to_parent() {
    let fixture = Fixture::new("tokens");
    let parent = fixture.parent();
    let committed = fixture.spawn(&parent, AgentRole::Explorer, "look");
    fixture.finish_child(
        &committed,
        TurnOutcome::Complete { stop_reason: None },
        None,
    );
    let parent_tokens = fixture
        .database
        .list_chat_sessions(fixture.workspace(), 0, SESSION_PAGE_SIZE)
        .expect("list")[0]
        .total_input_tokens;
    assert_eq!(parent_tokens, 0);
    let child_tokens: i64 = fixture
        .database
        .connection()
        .query_row(
            "SELECT total_input_tokens FROM agent_session WHERE external_id = ?1",
            params![committed.child_session_id],
            |row| row.get(0),
        )
        .expect("child tokens");
    assert_eq!(child_tokens, 10);
}

#[test]
fn mark_report_delivered_is_idempotent_and_keeps_report() {
    let fixture = Fixture::new("delivered");
    let parent = fixture.parent();
    let committed = fixture.spawn(&parent, AgentRole::Explorer, "look");
    fixture.finish_child(
        &committed,
        TurnOutcome::Complete { stop_reason: None },
        Some(&report()),
    );
    let before = fixture
        .database
        .list_subagents(fixture.workspace(), &parent)
        .expect("list")[0]
        .report_json
        .clone();
    assert!(
        fixture
            .database
            .mark_report_delivered(fixture.workspace(), &committed.agent_id)
            .expect("first")
    );
    assert!(
        !fixture
            .database
            .mark_report_delivered(fixture.workspace(), &committed.agent_id)
            .expect("second")
    );
    let record = fixture
        .database
        .list_subagents(fixture.workspace(), &parent)
        .expect("list")[0]
        .clone();
    assert!(record.report_delivered);
    assert_eq!(record.report_json, before);
    assert!(
        fixture
            .database
            .pending_subagent_reports(fixture.workspace(), &parent)
            .expect("pending")
            .is_empty()
    );
}

#[test]
fn deleting_parent_cascades_to_child_and_relation() {
    let fixture = Fixture::new("cascade");
    let parent = fixture.parent();
    let committed = fixture.spawn(&parent, AgentRole::Executor, "fix");
    fixture
        .database
        .delete_chat_session(fixture.workspace(), &parent)
        .expect("delete");
    let child_sessions: i64 = fixture
        .database
        .connection()
        .query_row(
            "SELECT COUNT(*) FROM agent_session WHERE external_id = ?1",
            params![committed.child_session_id],
            |row| row.get(0),
        )
        .expect("count sessions");
    assert_eq!(child_sessions, 0);
    let relations: i64 = fixture
        .database
        .connection()
        .query_row("SELECT COUNT(*) FROM agent_subagent", [], |row| row.get(0))
        .expect("count relations");
    assert_eq!(relations, 0);
    let messages: i64 = fixture
        .database
        .connection()
        .query_row(
            "SELECT COUNT(*) FROM agent_message WHERE session_id = ?1",
            params![committed.child_session_id],
            |row| row.get(0),
        )
        .expect("count messages");
    assert_eq!(messages, 0);
}

#[test]
fn recover_running_subagents_cancels_and_interrupts() {
    let fixture = Fixture::new("recover");
    let parent = fixture.parent();
    let committed = fixture.spawn(&parent, AgentRole::Explorer, "look");
    assert_eq!(
        fixture
            .database
            .recover_running_subagents()
            .expect("recover"),
        1
    );
    assert_eq!(
        fixture
            .database
            .recover_running_subagents()
            .expect("idempotent"),
        0
    );
    let record = fixture
        .database
        .list_subagents(fixture.workspace(), &parent)
        .expect("list")[0]
        .clone();
    assert_eq!(record.status, "cancelled");
    let messages = fixture
        .database
        .load_chat_session(fixture.workspace(), &committed.child_session_id)
        .expect("load")
        .expect("child");
    assert_eq!(messages[1].status, "interrupted");
}

#[test]
fn v15_upgrade_adds_columns_and_keeps_data() {
    let fixture = Fixture::new("upgrade");
    let parent = fixture.parent();
    let committed = fixture.spawn(&parent, AgentRole::Explorer, "look");
    // 模拟 v15 库：去掉 v16 三列并回退版本号。
    fixture
        .database
        .connection()
        .execute_batch(
            "ALTER TABLE agent_subagent DROP COLUMN report_json;
             ALTER TABLE agent_subagent DROP COLUMN report_delivered;
             ALTER TABLE agent_subagent DROP COLUMN title;
             ALTER TABLE agent_message DROP COLUMN subagent_report_json;
             PRAGMA user_version = 15;",
        )
        .expect("downgrade");
    let Fixture {
        root,
        workspace,
        database,
    } = fixture;
    drop(database);
    let database = Database::open(&root.join("lx.db")).expect("reopen");
    let version: i64 = database
        .connection()
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .expect("version");
    assert_eq!(version, crate::db::SCHEMA_VERSION_FOR_TESTS);
    let records = database.list_subagents(&workspace, &parent).expect("list");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].agent_id, committed.agent_id);
    assert_eq!(records[0].title, "", "旧记录不回填 task 首行");
    let integrity: String = database
        .connection()
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .expect("integrity");
    assert_eq!(integrity, "ok");
    cleanup(root);
}
