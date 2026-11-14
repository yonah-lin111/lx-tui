//! Todo 只读查询层测试：筛选、分页、汇总、按日聚合与聚合组统计。

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::db::Database;
use crate::db::todo::{TodoPriority, TodoRecord, TodoStatus};

use super::{TodoFilter, TodoPage, TodoSummary};

/// 独立临时目录中的数据库路径；每个测试互不共享。
fn open_test_database() -> Database {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or(0);
    let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let path: PathBuf = std::env::temp_dir()
        .join(format!("lx-tui-todo-query-test-{nanos:x}-{sequence:x}"))
        .join("lx.db");
    Database::open(&path).expect("database opens")
}

/// 追加一条独立待办（位置取当日末尾）。
fn save(
    db: &Database,
    date: &str,
    text: &str,
    status: TodoStatus,
    priority: TodoPriority,
) -> TodoRecord {
    let position = db.list_todos_by_date(date).expect("list date").len();
    let record = TodoRecord::new_on(
        date,
        text,
        status,
        priority,
        i64::try_from(position).unwrap_or(0),
    );
    db.save_todo(&record).expect("save todo");
    record
}

/// 写入某日全部条目并整体写回聚合组归属。
///
/// `save_todo` 每次写入都会清理单成员组，无法逐条拼出多成员组；这里先逐条落库，
/// 再用 `sync_date_todos` 在单事务内统一写回 `group_id`。
fn save_date(
    db: &Database,
    date: &str,
    drafts: &[(&str, TodoStatus, TodoPriority, Option<&str>)],
) -> Vec<TodoRecord> {
    let records: Vec<TodoRecord> = drafts
        .iter()
        .enumerate()
        .map(|(index, (text, status, priority, group))| {
            TodoRecord::new_on(
                date,
                *text,
                *status,
                *priority,
                i64::try_from(index).unwrap_or(0),
            )
            .with_group_id(group.map(str::to_string))
        })
        .collect();
    for record in &records {
        db.save_todo(record).expect("save todo");
    }
    db.sync_date_todos(date, &records).expect("sync date");
    records
}

/// 按默认分页读取一页。
fn query(db: &Database, filter: &TodoFilter) -> TodoPage {
    db.query_todos_page(filter, 1, 50).expect("query page")
}

/// 明细文本序列（断言顺序）。
fn page_texts(page: &TodoPage) -> Vec<&str> {
    page.rows
        .iter()
        .map(|row| row.record.text.as_str())
        .collect()
}

#[test]
fn range_filter_is_inclusive_by_entry_date() {
    let db = open_test_database();
    for date in ["2026-11-08", "2026-11-09", "2026-11-10", "2026-11-11"] {
        save(&db, date, date, TodoStatus::Todo, TodoPriority::P1);
    }

    let all = query(&db, &TodoFilter::default());
    assert_eq!(all.total, 4);
    assert_eq!(
        page_texts(&all),
        vec!["2026-11-11", "2026-11-10", "2026-11-09", "2026-11-08"]
    );

    let closed = TodoFilter {
        start_date: Some("2026-11-09".into()),
        end_date: Some("2026-11-10".into()),
        ..Default::default()
    };
    assert_eq!(
        page_texts(&query(&db, &closed)),
        vec!["2026-11-10", "2026-11-09"],
        "起止边界都包含在内"
    );

    let from = TodoFilter {
        start_date: Some("2026-11-10".into()),
        ..Default::default()
    };
    assert_eq!(query(&db, &from).total, 2);

    let until = TodoFilter {
        end_date: Some("2026-11-09".into()),
        ..Default::default()
    };
    assert_eq!(query(&db, &until).total, 2);

    // 起止倒置由服务层交换；DB 层照原样执行，结果为空。
    let inverted = TodoFilter {
        start_date: Some("2026-11-11".into()),
        end_date: Some("2026-11-08".into()),
        ..Default::default()
    };
    assert_eq!(query(&db, &inverted).total, 0);
}

#[test]
fn status_priority_and_search_filters_combine() {
    let db = open_test_database();
    save(
        &db,
        "2026-11-10",
        "write report",
        TodoStatus::Todo,
        TodoPriority::P1,
    );
    save(
        &db,
        "2026-11-10",
        "review pr",
        TodoStatus::InProgress,
        TodoPriority::P0,
    );
    save(
        &db,
        "2026-11-11",
        "write tests",
        TodoStatus::Done,
        TodoPriority::P1,
    );
    save(
        &db,
        "2026-11-11",
        "inbox",
        TodoStatus::Todo,
        TodoPriority::P3,
    );

    let status = TodoFilter {
        status: Some(TodoStatus::Todo),
        ..Default::default()
    };
    assert_eq!(
        page_texts(&query(&db, &status)),
        vec!["inbox", "write report"]
    );

    let priority = TodoFilter {
        priority: Some(TodoPriority::P1),
        ..Default::default()
    };
    assert_eq!(
        page_texts(&query(&db, &priority)),
        vec!["write tests", "write report"]
    );

    let impossible = TodoFilter {
        status: Some(TodoStatus::Todo),
        priority: Some(TodoPriority::P0),
        ..Default::default()
    };
    assert_eq!(query(&db, &impossible).total, 0);

    let search = TodoFilter {
        search: Some("write".into()),
        ..Default::default()
    };
    assert_eq!(
        page_texts(&query(&db, &search)),
        vec!["write tests", "write report"]
    );

    let done_search = TodoFilter {
        status: Some(TodoStatus::Done),
        search: Some("write".into()),
        ..Default::default()
    };
    assert_eq!(page_texts(&query(&db, &done_search)), vec!["write tests"]);

    let mismatched = TodoFilter {
        priority: Some(TodoPriority::P1),
        search: Some("review".into()),
        ..Default::default()
    };
    assert_eq!(query(&db, &mismatched).total, 0);
}

#[test]
fn search_is_ascii_case_insensitive_and_matches_chinese_substrings() {
    let db = open_test_database();
    save(
        &db,
        "2026-11-10",
        "Fix Login Bug",
        TodoStatus::Todo,
        TodoPriority::P1,
    );
    save(
        &db,
        "2026-11-10",
        "写周报总结",
        TodoStatus::Todo,
        TodoPriority::P2,
    );
    save(
        &db,
        "2026-11-11",
        "部署 login 服务",
        TodoStatus::InProgress,
        TodoPriority::P0,
    );

    let lower = TodoFilter {
        search: Some("login".into()),
        ..Default::default()
    };
    assert_eq!(query(&db, &lower).total, 2);

    let upper = TodoFilter {
        search: Some("LOGIN".into()),
        ..Default::default()
    };
    assert_eq!(
        page_texts(&query(&db, &upper)),
        vec!["部署 login 服务", "Fix Login Bug"],
        "ASCII 大小写不敏感"
    );

    let chinese = TodoFilter {
        search: Some("周报".into()),
        ..Default::default()
    };
    assert_eq!(page_texts(&query(&db, &chinese)), vec!["写周报总结"]);

    let chinese_partial = TodoFilter {
        search: Some("服务".into()),
        ..Default::default()
    };
    assert_eq!(
        page_texts(&query(&db, &chinese_partial)),
        vec!["部署 login 服务"]
    );

    let missing = TodoFilter {
        search: Some("absent".into()),
        ..Default::default()
    };
    assert_eq!(query(&db, &missing).total, 0);
}

#[test]
fn pagination_reports_total_and_clamps_bounds() {
    let db = open_test_database();
    for index in 0..5 {
        save(
            &db,
            "2026-11-10",
            &format!("item-{index}"),
            TodoStatus::Todo,
            TodoPriority::P1,
        );
    }
    let filter = TodoFilter::default();

    let first = db.query_todos_page(&filter, 1, 2).expect("page 1");
    assert_eq!(page_texts(&first), vec!["item-0", "item-1"]);
    assert_eq!((first.total, first.page, first.page_size), (5, 1, 2));

    let second = db.query_todos_page(&filter, 2, 2).expect("page 2");
    assert_eq!(page_texts(&second), vec!["item-2", "item-3"]);
    assert_eq!((second.total, second.page), (5, 2));

    let third = db.query_todos_page(&filter, 3, 2).expect("page 3");
    assert_eq!(page_texts(&third), vec!["item-4"]);

    let beyond = db.query_todos_page(&filter, 9, 2).expect("beyond");
    assert!(beyond.rows.is_empty());
    assert_eq!(beyond.total, 5);

    // page=0 视为 1；page_size=0 视为 1，超上限钳到 200。
    let zero_page = db.query_todos_page(&filter, 0, 2).expect("page 0");
    assert_eq!((zero_page.page, zero_page.page_size), (1, 2));
    let tiny = db.query_todos_page(&filter, 1, 0).expect("size 0");
    assert_eq!((tiny.page_size, tiny.rows.len()), (1, 1));
    let huge = db.query_todos_page(&filter, 1, 999).expect("size clamp");
    assert_eq!((huge.page_size, huge.rows.len()), (200, 5));
}

#[test]
fn items_order_by_date_desc_then_position_asc() {
    let db = open_test_database();
    save_date(
        &db,
        "2026-11-09",
        &[
            ("old-a", TodoStatus::Todo, TodoPriority::P1, None),
            ("old-b", TodoStatus::Todo, TodoPriority::P1, None),
        ],
    );
    save_date(
        &db,
        "2026-11-10",
        &[
            ("new-a", TodoStatus::Todo, TodoPriority::P1, None),
            ("new-b", TodoStatus::Todo, TodoPriority::P1, None),
        ],
    );

    assert_eq!(
        page_texts(&query(&db, &TodoFilter::default())),
        vec!["new-a", "new-b", "old-a", "old-b"]
    );
}

#[test]
fn items_resolve_group_label_from_earliest_member() {
    let db = open_test_database();
    save_date(
        &db,
        "2026-11-10",
        &[
            ("zebra", TodoStatus::Todo, TodoPriority::P1, Some("g1")),
            ("apple", TodoStatus::Todo, TodoPriority::P1, Some("g1")),
            ("solo", TodoStatus::Todo, TodoPriority::P1, None),
        ],
    );

    let page = query(&db, &TodoFilter::default());
    let labels: Vec<Option<&str>> = page
        .rows
        .iter()
        .map(|row| row.group_label.as_deref())
        .collect();
    // 组标签取最早条目（按 position），与字典序无关。
    assert_eq!(labels, vec![Some("zebra"), Some("zebra"), None]);
}

#[test]
fn summary_counts_statuses_and_empty_table_is_zero() {
    let db = open_test_database();
    assert_eq!(
        db.query_todo_summary(&TodoFilter::default(), "2026-11-14")
            .expect("empty summary"),
        TodoSummary::default()
    );

    save(&db, "2026-11-14", "a", TodoStatus::Todo, TodoPriority::P1);
    save(&db, "2026-11-14", "b", TodoStatus::Todo, TodoPriority::P3);
    save(
        &db,
        "2026-11-14",
        "c",
        TodoStatus::InProgress,
        TodoPriority::P0,
    );
    save(&db, "2026-11-14", "d", TodoStatus::Done, TodoPriority::P1);
    save(&db, "2026-11-15", "e", TodoStatus::Done, TodoPriority::P3);

    let summary = db
        .query_todo_summary(&TodoFilter::default(), "2026-11-14")
        .expect("summary");
    assert_eq!(
        summary,
        TodoSummary {
            total: 5,
            open: 2,
            in_progress: 1,
            done: 2,
            overdue: 0,
        }
    );

    let open_only = TodoFilter {
        status: Some(TodoStatus::Todo),
        ..Default::default()
    };
    assert_eq!(
        db.query_todo_summary(&open_only, "2026-11-14")
            .expect("open summary"),
        TodoSummary {
            total: 2,
            open: 2,
            in_progress: 0,
            done: 0,
            overdue: 0,
        }
    );

    let top = TodoFilter {
        priority: Some(TodoPriority::P0),
        ..Default::default()
    };
    assert_eq!(
        db.query_todo_summary(&top, "2026-11-14")
            .expect("p0 summary"),
        TodoSummary {
            total: 1,
            open: 0,
            in_progress: 1,
            done: 0,
            overdue: 0,
        }
    );
}

#[test]
fn overdue_ignores_date_range_but_respects_other_filters() {
    let db = open_test_database();
    save(
        &db,
        "2026-11-10",
        "old open",
        TodoStatus::Todo,
        TodoPriority::P1,
    );
    save(
        &db,
        "2026-11-11",
        "old doing",
        TodoStatus::InProgress,
        TodoPriority::P2,
    );
    save(
        &db,
        "2026-11-12",
        "old done",
        TodoStatus::Done,
        TodoPriority::P1,
    );
    save(
        &db,
        "2026-11-13",
        "old urgent",
        TodoStatus::Todo,
        TodoPriority::P0,
    );
    save(
        &db,
        "2026-11-14",
        "today open",
        TodoStatus::Todo,
        TodoPriority::P1,
    );
    save(
        &db,
        "2026-11-15",
        "future open",
        TodoStatus::Todo,
        TodoPriority::P1,
    );

    let today_only = TodoFilter {
        start_date: Some("2026-11-14".into()),
        end_date: Some("2026-11-14".into()),
        ..Default::default()
    };
    let summary = db
        .query_todo_summary(&today_only, "2026-11-14")
        .expect("today summary");
    assert_eq!(summary.total, 1, "日期范围只收窄 total 等当页计数");
    assert_eq!(summary.overdue, 3, "Overdue 突破日期范围统计历史欠账");

    let by_priority = TodoFilter {
        priority: Some(TodoPriority::P0),
        ..today_only.clone()
    };
    assert_eq!(
        db.query_todo_summary(&by_priority, "2026-11-14")
            .expect("p0 overdue")
            .overdue,
        1
    );

    let by_status = TodoFilter {
        status: Some(TodoStatus::InProgress),
        ..Default::default()
    };
    assert_eq!(
        db.query_todo_summary(&by_status, "2026-11-14")
            .expect("status overdue")
            .overdue,
        1
    );

    let done_only = TodoFilter {
        status: Some(TodoStatus::Done),
        ..Default::default()
    };
    assert_eq!(
        db.query_todo_summary(&done_only, "2026-11-14")
            .expect("done overdue")
            .overdue,
        0
    );

    let by_search = TodoFilter {
        search: Some("urgent".into()),
        ..Default::default()
    };
    assert_eq!(
        db.query_todo_summary(&by_search, "2026-11-14")
            .expect("search overdue")
            .overdue,
        1
    );

    let open_overdue = TodoFilter {
        status: Some(TodoStatus::Todo),
        ..Default::default()
    };
    assert_eq!(
        db.query_todo_summary(&open_overdue, "2026-11-14")
            .expect("open overdue")
            .overdue,
        2
    );
}

#[test]
fn daily_aggregates_planned_done_and_remaining() {
    let db = open_test_database();
    assert!(
        db.query_todo_daily(&TodoFilter::default())
            .expect("empty daily")
            .is_empty()
    );

    save_date(
        &db,
        "2026-11-09",
        &[
            ("a", TodoStatus::Todo, TodoPriority::P1, None),
            ("b", TodoStatus::Done, TodoPriority::P1, None),
        ],
    );
    save_date(
        &db,
        "2026-11-10",
        &[
            ("c", TodoStatus::Todo, TodoPriority::P1, None),
            ("d", TodoStatus::InProgress, TodoPriority::P1, None),
            ("e", TodoStatus::Todo, TodoPriority::P1, None),
        ],
    );
    save_date(
        &db,
        "2026-11-11",
        &[("f", TodoStatus::Done, TodoPriority::P1, None)],
    );

    let daily = db.query_todo_daily(&TodoFilter::default()).expect("daily");
    let rows: Vec<(&str, usize, usize, usize)> = daily
        .iter()
        .map(|row| (row.date.as_str(), row.planned, row.done, row.remaining))
        .collect();
    assert_eq!(
        rows,
        vec![
            ("2026-11-11", 1, 1, 0),
            ("2026-11-10", 3, 0, 3),
            ("2026-11-09", 2, 1, 1),
        ]
    );

    let done_only = TodoFilter {
        status: Some(TodoStatus::Done),
        ..Default::default()
    };
    let filtered = db.query_todo_daily(&done_only).expect("done daily");
    // 筛选后只统计命中条：11-09 的计划数降为 1，11-10 整天消失。
    assert_eq!(filtered.len(), 2);
    assert_eq!(filtered[0].date, "2026-11-11");
    assert_eq!((filtered[0].planned, filtered[0].done), (1, 1));
    assert_eq!(filtered[1].date, "2026-11-09");
    assert_eq!(
        (filtered[1].planned, filtered[1].done, filtered[1].remaining),
        (1, 1, 0)
    );
}

#[test]
fn groups_aggregate_members_and_skip_standalone_items() {
    let db = open_test_database();
    save_date(
        &db,
        "2026-11-10",
        &[
            ("alpha", TodoStatus::Todo, TodoPriority::P2, Some("g1")),
            ("beta", TodoStatus::Done, TodoPriority::P0, Some("g1")),
            (
                "gamma",
                TodoStatus::InProgress,
                TodoPriority::P3,
                Some("g1"),
            ),
            ("solo", TodoStatus::Todo, TodoPriority::P1, None),
        ],
    );
    save_date(
        &db,
        "2026-11-11",
        &[
            ("delta", TodoStatus::Todo, TodoPriority::P3, Some("g2")),
            ("epsilon", TodoStatus::Done, TodoPriority::P1, Some("g2")),
        ],
    );
    save_date(
        &db,
        "2026-11-12",
        &[
            ("zeta", TodoStatus::Todo, TodoPriority::P3, Some("g3")),
            ("eta", TodoStatus::Todo, TodoPriority::P2, Some("g3")),
        ],
    );

    let groups = db
        .query_todo_groups(&TodoFilter::default())
        .expect("groups");
    let ids: Vec<&str> = groups.iter().map(|group| group.group_id.as_str()).collect();
    assert_eq!(ids, vec!["g3", "g2", "g1"], "独立条目 solo 不出现在 Groups");

    let g1 = &groups[2];
    assert_eq!(g1.date, "2026-11-10");
    assert_eq!(g1.label, "alpha");
    assert_eq!((g1.items, g1.open, g1.in_progress, g1.done), (3, 1, 1, 1));
    assert_eq!(g1.top_priority, TodoPriority::P0, "P0 是组内最高优先级");

    let g2 = &groups[1];
    assert_eq!(g2.label, "delta");
    assert_eq!(g2.top_priority, TodoPriority::P1, "P3 与 P1 取更高的 P1");

    let g3 = &groups[0];
    assert_eq!(g3.top_priority, TodoPriority::P2, "P3 与 P2 取更高的 P2");
}

#[test]
fn groups_order_by_date_desc_then_first_position() {
    let db = open_test_database();
    save_date(
        &db,
        "2026-11-10",
        &[
            ("a1", TodoStatus::Todo, TodoPriority::P1, Some("ga")),
            ("a2", TodoStatus::Todo, TodoPriority::P1, Some("ga")),
            ("b1", TodoStatus::Todo, TodoPriority::P1, Some("gb")),
            ("b2", TodoStatus::Todo, TodoPriority::P1, Some("gb")),
        ],
    );
    save_date(
        &db,
        "2026-11-09",
        &[
            ("c1", TodoStatus::Todo, TodoPriority::P1, Some("gc")),
            ("c2", TodoStatus::Todo, TodoPriority::P1, Some("gc")),
        ],
    );

    let groups = db
        .query_todo_groups(&TodoFilter::default())
        .expect("groups");
    let ids: Vec<&str> = groups.iter().map(|group| group.group_id.as_str()).collect();
    assert_eq!(ids, vec!["ga", "gb", "gc"]);
    assert_eq!(groups[0].label, "a1");
    assert_eq!(groups[1].label, "b1");
}

#[test]
fn groups_respect_filters() {
    let db = open_test_database();
    save_date(
        &db,
        "2026-11-10",
        &[
            ("alpha", TodoStatus::Todo, TodoPriority::P2, Some("g1")),
            ("beta", TodoStatus::Done, TodoPriority::P0, Some("g1")),
        ],
    );

    let done_only = TodoFilter {
        status: Some(TodoStatus::Done),
        ..Default::default()
    };
    let groups = db.query_todo_groups(&done_only).expect("done groups");
    assert_eq!(groups.len(), 1);
    assert_eq!((groups[0].items, groups[0].open, groups[0].done), (1, 0, 1));

    let out_of_range = TodoFilter {
        start_date: Some("2026-11-11".into()),
        ..Default::default()
    };
    assert!(
        db.query_todo_groups(&out_of_range)
            .expect("range groups")
            .is_empty()
    );
}
