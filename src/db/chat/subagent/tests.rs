//! 子 agent 持久化测试：spawn 事务、列表过滤、报告行回放、恰好一次、级联与恢复。

use std::path::{Path, PathBuf};

use super::*;
use crate::chat::agent::{AgentRole, BaseRole};
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
        model_identity: None,
        environment: None,
        instructions: None,
        skills: None,
        mcp: None,
        model_switches: &[],
        skill_blocks: &[],
        supports_image_input: true,
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

#[test]
fn v18_upgrade_migrates_legacy_reports_and_creates_queue_tables() {
    let fixture = Fixture::new("v18-upgrade");
    let parent = fixture.parent();
    let first = fixture.spawn(&parent, AgentRole::Explorer, "first task");
    let second = fixture.spawn(&parent, AgentRole::Executor, "second task");
    fixture.finish_child(
        &first,
        TurnOutcome::Complete { stop_reason: None },
        Some(&report()),
    );
    fixture.finish_child(
        &second,
        TurnOutcome::Complete { stop_reason: None },
        Some(&report()),
    );
    fixture
        .database
        .mark_report_delivered(fixture.workspace(), &first.agent_id)
        .expect("mark first delivered");
    // 模拟 v18 库：删掉 v19 新增三表并回退 user_version = 18。
    fixture
        .database
        .connection()
        .execute_batch(
            "DROP TABLE agent_subagent_input_item;
             DROP TABLE agent_subagent_input;
             DROP TABLE agent_subagent_report;
             PRAGMA user_version = 18;",
        )
        .expect("downgrade to v18");
    let Fixture {
        root,
        workspace,
        database,
    } = fixture;
    drop(database);

    let database = Database::open(&root.join("lx.db")).expect("reopen v18");
    let version: i64 = database
        .connection()
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .expect("version");
    assert_eq!(version, crate::db::SCHEMA_VERSION_FOR_TESTS);

    let pending = database
        .pending_reports(&workspace, &parent)
        .expect("pending reports");
    assert_eq!(
        pending.len(),
        1,
        "已投递的 legacy 报告不得重复进入 pending，未投递的保留"
    );
    assert_eq!(pending[0].agent_id, second.agent_id);
    cleanup(root);
}

#[test]
fn enqueue_claim_cancel_and_interrupt_priority_ordering() {
    let fixture = Fixture::new("queue-order");
    let parent = fixture.parent();
    let committed = fixture.spawn(&parent, AgentRole::Executor, "initial task");
    fixture.finish_child(
        &committed,
        TurnOutcome::Complete { stop_reason: None },
        Some(&report()),
    );

    // 同一 dedupe_key 重复提交幂等，只生成一条输入记录
    let items_a = vec![PersistedInputItem::Text {
        text: "normal 1".into(),
    }];
    let first = fixture
        .database
        .enqueue_input(
            fixture.workspace(),
            &parent,
            &committed.agent_id,
            "dedupe-1",
            Some("call-1"),
            None,
            &items_a,
            false,
        )
        .expect("enqueue 1");
    assert!(first.created);
    assert_eq!(first.queued_count, 1);

    let duplicate = fixture
        .database
        .enqueue_input(
            fixture.workspace(),
            &parent,
            &committed.agent_id,
            "dedupe-1",
            Some("call-1"),
            None,
            &items_a,
            false,
        )
        .expect("enqueue dup");
    assert!(!duplicate.created);
    assert_eq!(duplicate.submission_id, first.submission_id);
    assert_eq!(duplicate.queued_count, 1);

    // 普通排队 #2
    let second = fixture
        .database
        .enqueue_input(
            fixture.workspace(),
            &parent,
            &committed.agent_id,
            "dedupe-2",
            Some("call-2"),
            None,
            &[PersistedInputItem::Text {
                text: "normal 2".into(),
            }],
            false,
        )
        .expect("enqueue 2");
    assert_eq!(second.queued_count, 2);

    // 连续两条 interrupt=true 输入：按提交序排在普通队列之前
    let int_1 = fixture
        .database
        .enqueue_input(
            fixture.workspace(),
            &parent,
            &committed.agent_id,
            "dedupe-int-1",
            Some("call-3"),
            None,
            &[PersistedInputItem::Text {
                text: "interrupt 1".into(),
            }],
            true,
        )
        .expect("enqueue int 1");
    let int_2 = fixture
        .database
        .enqueue_input(
            fixture.workspace(),
            &parent,
            &committed.agent_id,
            "dedupe-int-2",
            Some("call-4"),
            None,
            &[PersistedInputItem::Text {
                text: "interrupt 2".into(),
            }],
            true,
        )
        .expect("enqueue int 2");
    assert_eq!(int_2.queued_count, 4);

    // 取消 second（queued -> cancelled），重复取消幂等
    let (_, status, remaining) = fixture
        .database
        .cancel_input(fixture.workspace(), &parent, &second.submission_id)
        .expect("cancel")
        .expect("found");
    assert_eq!(status, "cancelled");
    assert_eq!(remaining, 3);
    let (_, status_again, _) = fixture
        .database
        .cancel_input(fixture.workspace(), &parent, &second.submission_id)
        .expect("cancel again")
        .expect("found");
    assert_eq!(status_again, "cancelled");

    // 依次取队首并认领：顺序必须是 int_1 -> int_2 -> first
    for expected_id in [
        &int_1.submission_id,
        &int_2.submission_id,
        &first.submission_id,
    ] {
        let head = fixture
            .database
            .next_queued_input(fixture.workspace(), &parent, &committed.agent_id)
            .expect("next")
            .expect("some");
        assert_eq!(&head.submission_id, expected_id);
        let claimed = fixture
            .database
            .claim_input(
                fixture.workspace(),
                &parent,
                &committed.agent_id,
                &head.submission_id,
                &Fixture::model(),
            )
            .expect("claim");
        // running 状态下调用 cancel_input 不回滚，返回当前状态 running
        let (_, running_status, _) = fixture
            .database
            .cancel_input(fixture.workspace(), &parent, &claimed.submission_id)
            .expect("cancel running")
            .expect("found");
        assert_eq!(running_status, "running");

        fixture
            .database
            .finish_subagent_turn(
                fixture.workspace(),
                &committed.agent_id,
                &claimed.child_assistant_id,
                &[part(0, "turn done")],
                &TurnOutcome::Complete { stop_reason: None },
                Usage::default(),
                Usage::default(),
                Some(&report()),
            )
            .expect("finish turn");
    }
    assert!(
        fixture
            .database
            .next_queued_input(fixture.workspace(), &parent, &committed.agent_id)
            .expect("empty")
            .is_none()
    );
    // 4 轮完成共生成 4 条独立报告
    let reports = fixture
        .database
        .pending_reports(fixture.workspace(), &parent)
        .expect("pending reports");
    assert_eq!(reports.len(), 4);
    cleanup(fixture.root);
}

#[test]
fn queued_input_images_are_included_in_referenced_attachment_paths() {
    let fixture = Fixture::new("ref-paths");
    let parent = fixture.parent();
    let committed = fixture.spawn(&parent, AgentRole::Explorer, "look");
    let archive_path = "/tmp/lx-test-archived-subagent-img.png".to_string();
    fixture
        .database
        .enqueue_input(
            fixture.workspace(),
            &parent,
            &committed.agent_id,
            "dedupe-img",
            Some("call-img"),
            None,
            &[
                PersistedInputItem::Text {
                    text: "check this".into(),
                },
                PersistedInputItem::Image {
                    label: "bug.png".into(),
                    archive_path: archive_path.clone(),
                    mime: "image/png".into(),
                    content_hash: "0123456789abcdef".into(),
                    width: 16,
                    height: 16,
                },
            ],
            false,
        )
        .expect("enqueue image");
    let referenced = fixture
        .database
        .referenced_attachment_paths()
        .expect("referenced");
    assert!(
        referenced.contains(&archive_path),
        "未处理队列中的归档图片必须纳入 GC 引用集"
    );
    cleanup(fixture.root);
}

#[test]
fn begin_with_options_persists_nickname_and_custom_role_and_resolves_by_nickname() {
    let fixture = Fixture::new("nickname-role");
    let parent = fixture.parent();
    let role =
        AgentRole::from_name_and_base("reviewer", Some(BaseRole::Explorer)).expect("reviewer");
    let items = [PersistedInputItem::Text {
        text: "review the diff".into(),
    }];
    let committed = fixture
        .database
        .begin_subagent_turn_with_options(
            fixture.workspace(),
            &parent,
            None,
            Some("Turing"),
            role,
            "review diff",
            "review the diff",
            &items,
            &Fixture::model(),
            None,
            None,
            None,
            false,
            &None,
        )
        .expect("spawn");

    let record = fixture
        .database
        .list_subagents(fixture.workspace(), &parent)
        .expect("list")[0]
        .clone();
    assert_eq!(record.nickname.as_deref(), Some("Turing"));
    assert_eq!(record.role, role);
    assert_eq!(record.role.base(), BaseRole::Explorer);
    assert!(!record.role.is_builtin());

    let (nickname, role_name, agent_path): (Option<String>, Option<String>, Option<String>) =
        fixture
            .database
            .connection()
            .query_row(
                "SELECT agent_nickname, agent_role, agent_path FROM agent_session \
                 WHERE external_id = ?1",
                params![committed.child_session_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("child session identity");
    assert_eq!(nickname.as_deref(), Some("Turing"));
    assert_eq!(role_name.as_deref(), Some("reviewer"));
    assert_eq!(agent_path.as_deref(), Some("explorer"));

    let by_id = fixture
        .database
        .resolve_owned_subagent(fixture.workspace(), &parent, &committed.agent_id)
        .expect("resolve by id")
        .expect("found by id");
    assert_eq!(by_id.agent_id, committed.agent_id);

    let by_nickname = fixture
        .database
        .resolve_owned_subagent(fixture.workspace(), &parent, "turing")
        .expect("resolve by nickname")
        .expect("found by nickname");
    assert_eq!(by_nickname.agent_id, committed.agent_id);
    assert_eq!(by_nickname.nickname.as_deref(), Some("Turing"));

    assert!(
        fixture
            .database
            .resolve_owned_subagent(fixture.workspace(), &parent, "missing")
            .expect("resolve missing")
            .is_none()
    );
    cleanup(fixture.root);
}

#[test]
fn fork_context_clones_only_completed_turns_before_the_streaming_turn() {
    let fixture = Fixture::new("fork-context");
    let parent = fixture.parent();
    // 父会话第二轮仍在 streaming：fork 只能看到上一轮已完成历史。
    fixture
        .database
        .begin_turn(
            fixture.workspace(),
            Some(&parent),
            "streaming question",
            &Fixture::model(),
        )
        .expect("streaming parent turn");
    let items = [PersistedInputItem::Text {
        text: "child task".into(),
    }];
    let committed = fixture
        .database
        .begin_subagent_turn_with_options(
            fixture.workspace(),
            &parent,
            None,
            Some("Curie"),
            AgentRole::Executor,
            "child task",
            "child task",
            &items,
            &Fixture::model(),
            None,
            None,
            None,
            true,
            &None,
        )
        .expect("fork spawn");

    let messages = fixture
        .database
        .load_chat_session(fixture.workspace(), &committed.child_session_id)
        .expect("load")
        .expect("child exists");
    let summary: Vec<(i64, i64, &str, &str)> = messages
        .iter()
        .map(|message| {
            (
                message.sequence,
                message.turn_index,
                message.role.as_str(),
                message
                    .parts
                    .first()
                    .map(|part| part.content.as_str())
                    .unwrap_or_default(),
            )
        })
        .collect();
    assert_eq!(
        summary,
        vec![
            (0, 0, "user", "parent question"),
            (1, 0, "assistant", "parent answer"),
            (2, 1, "user", "child task"),
            (3, 1, "assistant", ""),
        ]
    );
    assert!(messages.iter().all(|message| {
        message
            .parts
            .iter()
            .all(|part| part.content != "streaming question")
    }));
    cleanup(fixture.root);
}

#[test]
fn dev_database_copy_migrates_to_current_version_and_passes_integrity_check() {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return;
    };
    let dev_db = home.join(".lx-t-dev/db/lx.db");
    if !dev_db.is_file() {
        return;
    }
    let temp_dir = std::env::temp_dir().join(format!(
        "lx-dev-db-copy-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&temp_dir).expect("temp dir");
    let copy_path = temp_dir.join("lx.db");
    std::fs::copy(&dev_db, &copy_path).expect("copy dev db");

    let before_counts: (i64, i64, i64) = {
        let raw = rusqlite::Connection::open(&copy_path).expect("open raw copy");
        let sessions: i64 = raw
            .query_row("SELECT COUNT(*) FROM agent_session", [], |r| r.get(0))
            .unwrap_or(0);
        let messages: i64 = raw
            .query_row("SELECT COUNT(*) FROM agent_message", [], |r| r.get(0))
            .unwrap_or(0);
        let subagents: i64 = raw
            .query_row("SELECT COUNT(*) FROM agent_subagent", [], |r| r.get(0))
            .unwrap_or(0);
        (sessions, messages, subagents)
    };

    let migrated = Database::open(&copy_path).expect("migrate dev db copy");
    let version: i64 = migrated
        .connection()
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .expect("user_version");
    assert_eq!(version, crate::db::SCHEMA_VERSION_FOR_TESTS);

    let after_sessions: i64 = migrated
        .connection()
        .query_row("SELECT COUNT(*) FROM agent_session", [], |r| r.get(0))
        .expect("sessions");
    let after_messages: i64 = migrated
        .connection()
        .query_row("SELECT COUNT(*) FROM agent_message", [], |r| r.get(0))
        .expect("messages");
    let after_subagents: i64 = migrated
        .connection()
        .query_row("SELECT COUNT(*) FROM agent_subagent", [], |r| r.get(0))
        .expect("subagents");
    assert_eq!(
        (after_sessions, after_messages, after_subagents),
        before_counts
    );

    let integrity: String = migrated
        .connection()
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .expect("integrity_check");
    assert_eq!(integrity, "ok");
    let fk_violations: i64 = migrated
        .connection()
        .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |r| {
            r.get(0)
        })
        .expect("fk check");
    assert_eq!(fk_violations, 0);

    drop(migrated);
    cleanup(temp_dir);
}
