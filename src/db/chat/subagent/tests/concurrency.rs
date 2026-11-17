//! SQLite WAL 并发写回归：`BEGIN IMMEDIATE` 避免先读后写事务撞 `SQLITE_BUSY_SNAPSHOT`。
//!
//! 复现场景（来自 `sess-18e8d0bd85ac7960-2`）：主循环在 `begin_subagent_turn_with_full_options`
//! 内先查 `ensure_session_owned` 再写 `agent_session`，与此同时后台线程（`usage_recorder`）
//! 打开独立连接写入 `usage_log`。旧 `BEGIN DEFERRED` 会直接返回 `SQLITE_BUSY_SNAPSHOT`
//! （绕过 `busy_timeout`）；改用 `write_transaction()`（`BEGIN IMMEDIATE`）后，两方有序排队。

use std::sync::{Arc, Barrier};

use crate::db::usage::{UsageLogInsert, UsagePurpose, UsageRates, UsageStatus, UsageTokens};

use super::*;

#[test]
fn concurrent_usage_log_and_subagent_spawn_never_hit_busy_snapshot() {
    let fixture = Fixture::new("wal-concurrent-spawn");
    let db_path = fixture.root.join("lx.db");
    let parent = fixture.parent();
    let workspace = fixture.workspace().to_path_buf();

    // 使用真实文件库再开一条并发写连接，模拟 `usage_recorder` 后台写线程。
    let writer_usage = Database::open(&db_path).expect("open second connection");
    let barrier = Arc::new(Barrier::new(2));
    let worker_barrier = Arc::clone(&barrier);
    let worker_path = db_path.clone();
    let worker_parent = parent.clone();

    let rounds = 40usize;
    let usage_thread = std::thread::spawn(move || {
        // 同步测试线程与额外打开连接均须能跨线程有序写库。
        let db = writer_usage;
        worker_barrier.wait();
        for i in 0..rounds {
            // 穿插重新打开连接（验证 `Database::open` 在已开启 WAL 时不破坏锁行为）。
            let handle = if i % 10 == 0 {
                Some(Database::open(&worker_path).expect("reopen concurrent db"))
            } else {
                None
            };
            let target = handle.as_ref().unwrap_or(&db);
            let entry = UsageLogInsert {
                external_id: format!("ulog-concurrent-{i}"),
                session_id: Some(worker_parent.clone()),
                workspace_id: None,
                purpose: UsagePurpose::Chat,
                provider: "provider".into(),
                model: "model".into(),
                tokens: UsageTokens {
                    input: 100,
                    output: 20,
                    cache_read: 0,
                    cache_write: 0,
                },
                rates: UsageRates::default(),
                duration_ms: Some(10),
                status: UsageStatus::Success,
                error_message: None,
                created_at_ms: i as i64 + 1,
            };
            target
                .record_usage_log(&entry)
                .expect("concurrent usage_log write must succeed");
        }
    });

    barrier.wait();
    for i in 0..rounds {
        let task = format!("concurrent probe {i}");
        let items = [PersistedInputItem::Text { text: task.clone() }];
        let committed = fixture
            .database
            .begin_subagent_turn_with_full_options(
                &workspace,
                &parent,
                None,
                None,
                AgentRole::Explorer,
                &format!("probe {i}"),
                &task,
                &items,
                &Fixture::model(),
                None,
                Some(&format!("dedupe-concurrent-{i}")),
                Some(&format!("call-concurrent-{i}")),
                false,
                None,
                None,
                &None,
            )
            .expect("concurrent subagent spawn must not hit SQLITE_BUSY_SNAPSHOT");
        fixture.finish_child(
            &committed,
            TurnOutcome::Complete {
                stop_reason: Some("complete".into()),
            },
            None,
        );
    }

    usage_thread.join().expect("usage thread");
    let count: i64 = fixture
        .database
        .query_count("SELECT COUNT(*) FROM agent_subagent")
        .expect("count subagents");
    assert_eq!(count, rounds as i64);
    cleanup(fixture.root);
}

#[test]
fn write_transaction_waits_past_concurrent_immediate_commit() {
    let fixture = Fixture::new("wal-immediate-wait");
    let db_path = fixture.root.join("lx.db");
    let parent = fixture.parent();
    let workspace = fixture.workspace().to_path_buf();

    let writer_two = Database::open(&db_path).expect("open second connection");
    let barrier = Arc::new(Barrier::new(2));
    let worker_barrier = Arc::clone(&barrier);
    let worker_parent = parent.clone();
    let handle = std::thread::spawn(move || {
        let tx_other = writer_two
            .write_transaction()
            .expect("begin other immediate");
        tx_other
            .execute(
                "INSERT INTO usage_log (external_id, session_id, purpose, provider, model, created_at_ms) VALUES ('ulog-1', ?1, 'chat', 'p', 'm', 1)",
                params![worker_parent],
            )
            .expect("insert usage");
        worker_barrier.wait();
        // 持锁 60ms，强迫主线程的 `begin_subagent_turn_with_full_options` 在 `BEGIN IMMEDIATE` 上等待并重试。
        std::thread::sleep(std::time::Duration::from_millis(60));
        tx_other.commit().expect("commit usage");
    });

    barrier.wait();
    let items = [PersistedInputItem::Text {
        text: "after other write".into(),
    }];
    let committed = fixture
        .database
        .begin_subagent_turn_with_full_options(
            &workspace,
            &parent,
            None,
            None,
            AgentRole::Explorer,
            "after other write",
            "after other write",
            &items,
            &Fixture::model(),
            None,
            Some("dedupe-after-write"),
            Some("call-after-write"),
            false,
            None,
            None,
            &None,
        )
        .expect("immediate write transaction succeeds after waiting on busy_timeout");
    handle.join().expect("worker");
    assert!(!committed.agent_id.is_empty());
    cleanup(fixture.root);
}
