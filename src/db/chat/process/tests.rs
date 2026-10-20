//! 进程元数据持久化测试：迁移、登记、revision 乱序、终态幂等、级联清理与恢复。

use std::path::{Path, PathBuf};

#[allow(unused_imports)]
use super::*;

use crate::db::Database;
use crate::db::chat::{NewProcess, OutputSnapshot};

/// 独立临时数据库 + 工作区。
struct Fixture {
    root: PathBuf,
    workspace: PathBuf,
    database: Database,
}

impl Fixture {
    fn new(label: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "lx-process-db-{}-{label}-{}",
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

    /// 建一个会话（进程记录必须挂在真实会话上）。
    fn session(&self) -> String {
        let snapshot = crate::db::chat::ModelSnapshot {
            provider: "p".into(),
            model: "m".into(),
            variant: None,
        };
        self.database
            .begin_turn(self.workspace(), None, "hello", &snapshot)
            .expect("begin turn")
            .session_id
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// 快照构造（默认空输出）。
fn empty_snapshot() -> OutputSnapshot<'static> {
    OutputSnapshot {
        output_tail: b"",
        output_start: 0,
        output_end: 0,
        dropped_bytes: 0,
    }
}

#[test]
fn v11_creates_the_process_table() {
    let fixture = Fixture::new("schema");
    let exists: i64 = fixture
        .database
        .conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='agent_process'",
            [],
            |row| row.get(0),
        )
        .expect("query");
    assert_eq!(exists, 1, "v11 建立 agent_process 表");
}

#[test]
fn begin_process_records_starting_state() {
    let fixture = Fixture::new("begin");
    let session = fixture.session();
    fixture
        .database
        .begin_process(
            fixture.workspace(),
            &NewProcess {
                chat_session_id: &session,
                origin_call_id: "call-1",
                runtime_epoch: "epoch-1",
                process_session_id: 7,
                command: "cargo test",
                cwd: fixture.workspace(),
                tty: false,
            },
        )
        .expect("begin process");
    let records = fixture
        .database
        .load_processes_for_session(fixture.workspace(), &session)
        .expect("load");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].state, "starting");
    assert_eq!(records[0].process_session_id, 7);
    assert_eq!(records[0].origin_call_id, "call-1");
    assert_eq!(records[0].command, "cargo test");
}

#[test]
fn duplicate_process_identity_is_rejected() {
    let fixture = Fixture::new("duplicate");
    let session = fixture.session();
    let new = NewProcess {
        chat_session_id: &session,
        origin_call_id: "call-1",
        runtime_epoch: "epoch-1",
        process_session_id: 7,
        command: "cmd",
        cwd: fixture.workspace(),
        tty: false,
    };
    fixture
        .database
        .begin_process(fixture.workspace(), &new)
        .expect("first");
    let error = fixture
        .database
        .begin_process(fixture.workspace(), &new)
        .expect_err("duplicate");
    assert_eq!(error.code(), "process conflict");
}

#[test]
fn cross_session_process_is_not_visible() {
    let fixture = Fixture::new("cross");
    let session = fixture.session();
    fixture
        .database
        .begin_process(
            fixture.workspace(),
            &NewProcess {
                chat_session_id: &session,
                origin_call_id: "call-1",
                runtime_epoch: "epoch-1",
                process_session_id: 1,
                command: "cmd",
                cwd: fixture.workspace(),
                tty: false,
            },
        )
        .expect("begin");
    // 另一个未知会话：返回 SessionMissing，不泄露别的会话的进程。
    let error = fixture
        .database
        .load_processes_for_session(fixture.workspace(), "other-session")
        .expect_err("missing session");
    assert_eq!(error.code(), "session missing");
}

#[test]
fn older_revisions_cannot_overwrite_newer_output() {
    let fixture = Fixture::new("revision");
    let session = fixture.session();
    fixture
        .database
        .begin_process(
            fixture.workspace(),
            &NewProcess {
                chat_session_id: &session,
                origin_call_id: "call-1",
                runtime_epoch: "epoch-1",
                process_session_id: 3,
                command: "cmd",
                cwd: fixture.workspace(),
                tty: false,
            },
        )
        .expect("begin");
    let newer = OutputSnapshot {
        output_tail: b"new",
        output_start: 0,
        output_end: 3,
        dropped_bytes: 0,
    };
    assert!(
        fixture
            .database
            .update_process_output(fixture.workspace(), "epoch-1", 3, &newer, 5)
            .expect("write v5")
    );
    let older = OutputSnapshot {
        output_tail: b"old",
        output_start: 0,
        output_end: 3,
        dropped_bytes: 0,
    };
    assert!(
        !fixture
            .database
            .update_process_output(fixture.workspace(), "epoch-1", 3, &older, 4)
            .expect("write v4"),
        "旧 revision 必须零行"
    );
    let records = fixture
        .database
        .load_processes_for_session(fixture.workspace(), &session)
        .expect("load");
    assert_eq!(records[0].output_tail, "new");
}

#[test]
fn terminal_state_is_not_overwritten_by_later_output() {
    let fixture = Fixture::new("terminal");
    let session = fixture.session();
    fixture
        .database
        .begin_process(
            fixture.workspace(),
            &NewProcess {
                chat_session_id: &session,
                origin_call_id: "call-1",
                runtime_epoch: "epoch-1",
                process_session_id: 4,
                command: "cmd",
                cwd: fixture.workspace(),
                tty: false,
            },
        )
        .expect("begin");
    let snapshot = OutputSnapshot {
        output_tail: b"done",
        output_start: 0,
        output_end: 4,
        dropped_bytes: 0,
    };
    let finish = ProcessFinish {
        runtime_epoch: "epoch-1",
        process_session_id: 4,
        state: "exited",
        exit_code: Some(0),
        stop_reason: None,
        snapshot: snapshot.clone(),
        revision: 10,
    };
    assert!(
        fixture
            .database
            .finish_process(fixture.workspace(), &finish)
            .expect("finish")
    );
    // 迟到的输出快照不能把终态改回 running。
    assert!(
        !fixture
            .database
            .update_process_output(fixture.workspace(), "epoch-1", 4, &snapshot, 11)
            .expect("late update")
    );
    let records = fixture
        .database
        .load_processes_for_session(fixture.workspace(), &session)
        .expect("load");
    assert_eq!(records[0].state, "exited");
    assert_eq!(records[0].exit_code, Some(0));
}

#[test]
fn restart_marks_running_processes_interrupted_without_rerun() {
    let fixture = Fixture::new("recover");
    let session = fixture.session();
    fixture
        .database
        .begin_process(
            fixture.workspace(),
            &NewProcess {
                chat_session_id: &session,
                origin_call_id: "call-1",
                runtime_epoch: "epoch-1",
                process_session_id: 9,
                command: "sleep 999",
                cwd: fixture.workspace(),
                tty: false,
            },
        )
        .expect("begin");
    let recovered = fixture
        .database
        .recover_interrupted_processes()
        .expect("recover");
    assert_eq!(recovered, 1);
    let records = fixture
        .database
        .load_processes_for_session(fixture.workspace(), &session)
        .expect("load");
    assert_eq!(records[0].state, "interrupted", "不重跑、不伪报已退出");
    assert!(records[0].ended_at_ms.is_some());
    // 幂等：第二次恢复不再命中。
    assert_eq!(
        fixture
            .database
            .recover_interrupted_processes()
            .expect("again"),
        0
    );
}

#[test]
fn deleting_a_session_cascades_process_records() {
    let fixture = Fixture::new("cascade");
    let session = fixture.session();
    fixture
        .database
        .begin_process(
            fixture.workspace(),
            &NewProcess {
                chat_session_id: &session,
                origin_call_id: "call-1",
                runtime_epoch: "epoch-1",
                process_session_id: 2,
                command: "cmd",
                cwd: fixture.workspace(),
                tty: false,
            },
        )
        .expect("begin");
    let removed = fixture
        .database
        .delete_processes_for_session(&session)
        .expect("delete");
    assert_eq!(removed, 1);
    let records = fixture
        .database
        .load_processes_for_session(fixture.workspace(), &session)
        .expect("load");
    assert!(records.is_empty());
    let _ = empty_snapshot();
}
