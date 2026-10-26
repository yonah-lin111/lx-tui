//! todo 持久层测试：UPSERT 语义、位置顺移、删除重排与撤销恢复。

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use super::*;
use crate::db::DbError;

/// 独立临时目录中的数据库路径；每个测试互不共享。
fn open_test_database() -> Database {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or(0);
    let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let path: PathBuf = std::env::temp_dir()
        .join(format!("lx-tui-todo-test-{nanos:x}-{sequence:x}"))
        .join("lx.db");
    Database::open(&path).expect("database opens")
}

/// 位置序列（断言密集性）。
fn positions(items: &[TodoRecord]) -> Vec<i64> {
    items.iter().map(|item| item.position).collect()
}

/// 文本序列（断言顺序）。
fn texts(items: &[TodoRecord]) -> Vec<&str> {
    items.iter().map(|item| item.text.as_str()).collect()
}

/// 造一条位于末尾的新 todo。
fn append(db: &Database, text: &str) -> TodoRecord {
    let position = i64::try_from(db.todo_count().expect("count")).expect("fits");
    let record = TodoRecord::new(text, TodoStatus::Todo, position);
    db.save_todo(&record).expect("save");
    record
}

#[test]
fn save_todo_insert_shifts_positions() {
    let db = open_test_database();
    let first = append(&db, "first");
    let second = append(&db, "second");
    assert_eq!(positions(&db.list_todos().expect("list")), vec![0, 1]);

    // 插到中间：原第 1 条顺移，其余不动。
    let middle = TodoRecord::new("middle", TodoStatus::Todo, 1);
    db.save_todo(&middle).expect("insert middle");

    let items = db.list_todos().expect("list");
    assert_eq!(texts(&items), vec!["first", "middle", "second"]);
    assert_eq!(positions(&items), vec![0, 1, 2]);
    assert_eq!(items[1].id, middle.id);
    assert_eq!(items[2].id, second.id);
    assert_eq!(items[0].id, first.id);
}

#[test]
fn save_todo_keeps_unique_positions() {
    let db = open_test_database();
    for index in 0..8 {
        // 全部插在 0：每次都把已有行整体顺移一格。
        let record = TodoRecord::new(format!("item-{index}"), TodoStatus::Todo, 0);
        db.save_todo(&record).expect("insert at head");
    }
    let items = db.list_todos().expect("list");
    assert_eq!(positions(&items), (0..8).collect::<Vec<_>>());
    // 每次都插在头部：最新一条排第一，最早一条被挤到末尾。
    assert_eq!(items[0].text, "item-7");
    assert_eq!(items[7].text, "item-0");
    assert_eq!(db.todo_count().expect("count"), 8);
}

#[test]
fn save_todo_is_idempotent() {
    let db = open_test_database();
    let record = append(&db, "stable");
    db.save_todo(&record).expect("repeat save");
    db.save_todo(&record).expect("repeat save again");
    let items = db.list_todos().expect("list");
    assert_eq!(items.len(), 1, "重复写入不新增行");
    assert_eq!(items[0].position, 0);
}

#[test]
fn save_todo_inserts_missing_row_at_position() {
    let db = open_test_database();
    append(&db, "a");
    append(&db, "b");
    // 一条从未写进库的记录（模拟"首次插入失败、之后又编辑"）：位置 1 落库并顺移。
    let ghost = TodoRecord::new("ghost", TodoStatus::InProgress, 1);
    db.save_todo(&ghost).expect("upsert inserts");

    let items = db.list_todos().expect("list");
    assert_eq!(texts(&items), vec!["a", "ghost", "b"]);
    assert_eq!(items[1].status, TodoStatus::InProgress);
}

#[test]
fn save_todo_updates_existing_without_moving_position() {
    let db = open_test_database();
    append(&db, "a");
    let target = append(&db, "b");
    append(&db, "c");

    let mut edited = target.clone();
    edited.text = "b-edited".into();
    edited.status = TodoStatus::Done;
    // 位置字段故意给错：已存在的行不允许通过 UPSERT 挪位。
    edited.position = 0;
    db.save_todo(&edited).expect("update");

    let items = db.list_todos().expect("list");
    assert_eq!(texts(&items), vec!["a", "b-edited", "c"]);
    assert_eq!(positions(&items), vec![0, 1, 2]);
    assert_eq!(items[1].status, TodoStatus::Done);
}

#[test]
fn save_todo_trims_text() {
    let db = open_test_database();
    let record = TodoRecord::new("   padded  ", TodoStatus::Todo, 0);
    db.save_todo(&record).expect("save");
    assert_eq!(db.list_todos().expect("list")[0].text, "padded");
}

#[test]
fn delete_todo_is_idempotent() {
    let db = open_test_database();
    let record = append(&db, "only");
    db.delete_todo(&record.id).expect("delete");
    db.delete_todo(&record.id).expect("delete missing is ok");
    assert_eq!(db.todo_count().expect("count"), 0);
}

#[test]
fn delete_todo_renumbers_remaining() {
    let db = open_test_database();
    let first = append(&db, "a");
    append(&db, "b");
    append(&db, "c");
    db.delete_todo(&first.id).expect("delete head");

    let items = db.list_todos().expect("list");
    assert_eq!(texts(&items), vec!["b", "c"]);
    assert_eq!(positions(&items), vec![0, 1]);
}

#[test]
fn delete_todos_with_status_reports_removed_records() {
    let db = open_test_database();
    let done = append(&db, "done-1");
    append(&db, "open");
    let done_two = append(&db, "done-2");
    for id in [&done.id, &done_two.id] {
        let mut record = db.todo_by_id(id).expect("read").expect("exists");
        record.status = TodoStatus::Done;
        db.save_todo(&record).expect("mark done");
    }

    let removed = db
        .delete_todos_with_status(TodoStatus::Done)
        .expect("clear done");
    assert_eq!(texts(&removed), vec!["done-1", "done-2"], "按原顺序返回");
    assert_eq!(removed[0].position, 0);
    assert_eq!(removed[1].position, 2);

    let items = db.list_todos().expect("list");
    assert_eq!(texts(&items), vec!["open"]);
    assert_eq!(positions(&items), vec![0], "剩余位置重排为密集");
}

#[test]
fn restore_todos_reinserts_at_original_positions() {
    let db = open_test_database();
    append(&db, "a");
    let removed_target = append(&db, "b");
    append(&db, "c");
    db.delete_todo(&removed_target.id).expect("delete middle");
    assert_eq!(db.todo_count().expect("count"), 2);

    db.restore_todos(std::slice::from_ref(&removed_target))
        .expect("restore");
    let items = db.list_todos().expect("list");
    assert_eq!(texts(&items), vec!["a", "b", "c"]);
    assert_eq!(positions(&items), vec![0, 1, 2]);
    assert_eq!(items[1].id, removed_target.id);
}

#[test]
fn restore_todos_inserts_back_into_shrunken_list() {
    let db = open_test_database();
    append(&db, "a");
    let tail = append(&db, "b");
    db.delete_todo(&tail.id).expect("delete tail");
    append(&db, "c");
    // 恢复位置 1 在当前长度 2 之内：插回中间。
    db.restore_todos(std::slice::from_ref(&tail))
        .expect("restore");
    let items = db.list_todos().expect("list");
    assert_eq!(texts(&items), vec!["a", "b", "c"]);
    assert_eq!(positions(&items), vec![0, 1, 2]);
}

#[test]
fn restore_todos_clamps_position_beyond_end() {
    let db = open_test_database();
    append(&db, "a");
    let removed = append(&db, "b");
    append(&db, "c");
    db.delete_todo(&removed.id).expect("delete middle");
    assert_eq!(texts(&db.list_todos().expect("list")), vec!["a", "c"]);

    // 原位置 1 已超出当前长度 2？不，仍在范围内；把位置改成远超长度验证钳制。
    let mut far = removed;
    far.position = 99;
    db.restore_todos(&[far]).expect("restore");
    let items = db.list_todos().expect("list");
    assert_eq!(texts(&items), vec!["a", "c", "b"]);
    assert_eq!(positions(&items), vec![0, 1, 2]);
}

#[test]
fn restore_todos_empty_is_noop() {
    let db = open_test_database();
    append(&db, "a");
    db.restore_todos(&[]).expect("noop");
    assert_eq!(db.todo_count().expect("count"), 1);
}

#[test]
fn empty_text_rejected() {
    let db = open_test_database();
    for text in ["", "   ", "\t\n"] {
        let record = TodoRecord::new(text, TodoStatus::Todo, 0);
        assert!(
            matches!(db.save_todo(&record), Err(DbError::InvalidDraft)),
            "{text:?} 应被拒绝"
        );
    }
    assert_eq!(db.todo_count().expect("count"), 0);
}

#[test]
fn oversized_text_rejected() {
    let db = open_test_database();
    // 按字符计数：500 个汉字合法（1500 字节），501 个越界。
    let ok = "字".repeat(500);
    db.save_todo(&TodoRecord::new(ok, TodoStatus::Todo, 0))
        .expect("500 chars is within limit");
    let too_long = "字".repeat(501);
    assert!(matches!(
        db.save_todo(&TodoRecord::new(too_long, TodoStatus::Todo, 1)),
        Err(DbError::MessageTooLarge)
    ));
}

#[test]
fn status_roundtrips_all_three_values() {
    let db = open_test_database();
    for (index, status) in [TodoStatus::Todo, TodoStatus::InProgress, TodoStatus::Done]
        .into_iter()
        .enumerate()
    {
        let record = TodoRecord::new(format!("s{index}"), status, index as i64);
        db.save_todo(&record).expect("save");
    }
    let items = db.list_todos().expect("list");
    assert_eq!(
        items.iter().map(|item| item.status).collect::<Vec<_>>(),
        vec![TodoStatus::Todo, TodoStatus::InProgress, TodoStatus::Done]
    );
}

#[test]
fn unknown_status_text_falls_back_to_todo() {
    assert_eq!(TodoStatus::from_db("done"), TodoStatus::Done);
    assert_eq!(TodoStatus::from_db("in_progress"), TodoStatus::InProgress);
    assert_eq!(TodoStatus::from_db("todo"), TodoStatus::Todo);
    assert_eq!(TodoStatus::from_db("garbage"), TodoStatus::Todo);
}

#[test]
fn status_cycle_walks_three_states() {
    assert_eq!(TodoStatus::Todo.cycle(), TodoStatus::InProgress);
    assert_eq!(TodoStatus::InProgress.cycle(), TodoStatus::Done);
    assert_eq!(TodoStatus::Done.cycle(), TodoStatus::Todo);
}
