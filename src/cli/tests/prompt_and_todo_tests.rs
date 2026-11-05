//! `lx-tui prompt` 与 `lx-tui todo` 全链路 CRUD 单元测试。

use crate::db::todo::{TodoPriority, TodoStatus, today_date};

use super::{CliTestEnv, assert_exit_ok, path_str};

#[test]
fn prompt_folder_entry_page_and_ref_crud_end_to_end() {
    let env = CliTestEnv::new("prompt-crud");

    // 1. Folder CRUD (root folder + child folder)
    assert_exit_ok(env.exec(&["prompt", "folder", "create", "Backend"]));
    let db = env.open_db();
    let folders = db.list_folders(&env.workspace).expect("folders");
    assert_eq!(folders.len(), 1);
    let root_folder_id = folders[0].id.to_string();

    assert_exit_ok(env.exec(&[
        "prompt",
        "folder",
        "create",
        "API",
        "--parent",
        &root_folder_id,
    ]));
    assert_exit_ok(env.exec(&[
        "prompt",
        "folder",
        "rename",
        &root_folder_id,
        "Core Backend",
    ]));
    assert_exit_ok(env.exec(&["prompt", "folder", "list", "--json"]));

    // 2. Prompt CRUD + pages
    assert_exit_ok(env.exec(&[
        "prompt",
        "create",
        "System Prompt",
        "--folder",
        &root_folder_id,
        "--page-name",
        "Overview",
        "--content",
        "Initial prompt content",
    ]));
    let prompts = db.list_prompts(&env.workspace).expect("prompts");
    assert_eq!(prompts.len(), 1);
    let prompt_id = prompts[0].id.to_string();
    assert_eq!(prompts[0].pages[0].name, "Overview");
    assert_eq!(prompts[0].pages[0].content, "Initial prompt content");

    assert_exit_ok(env.exec(&["prompt", "list"]));
    assert_exit_ok(env.exec(&["prompt", "show", &prompt_id, "--json"]));

    // Edit prompt via file input
    let body_file = env.write_file("updated_prompt.md", "Updated via -f file");
    assert_exit_ok(env.exec(&["prompt", "edit", &prompt_id, "-f", path_str(&body_file)]));
    let updated = db.list_prompts(&env.workspace).expect("prompts");
    assert_eq!(updated[0].pages[0].content, "Updated via -f file");

    // Create second page, switch active page, rename page, delete page
    assert_exit_ok(env.exec(&[
        "prompt",
        "page",
        "create",
        &prompt_id,
        "Examples",
        "--content",
        "Page 2 body",
    ]));
    let with_two_pages = db.list_prompts(&env.workspace).expect("prompts");
    assert_eq!(with_two_pages[0].pages.len(), 2);
    let second_page_id = with_two_pages[0].pages[1].id.to_string();

    assert_exit_ok(env.exec(&["prompt", "page", "active", &prompt_id, &second_page_id]));
    assert_exit_ok(env.exec(&[
        "prompt",
        "page",
        "rename",
        &second_page_id,
        "Few-shot Examples",
    ]));
    assert_exit_ok(env.exec(&["prompt", "page", "list", &prompt_id]));
    assert_exit_ok(env.exec(&["prompt", "page", "delete", &second_page_id]));

    // Deleting the last remaining page must be rejected
    let first_page_id = with_two_pages[0].pages[0].id.to_string();
    let last_page_err = env
        .exec(&["prompt", "page", "delete", &first_page_id])
        .expect_err("must reject deleting last page");
    assert!(last_page_err.contains("must keep at least one page"));

    // Move prompt to root, rename, and delete
    assert_exit_ok(env.exec(&["prompt", "move", &prompt_id, "--root"]));
    assert_exit_ok(env.exec(&["prompt", "rename", &prompt_id, "Root System Prompt"]));
    assert_exit_ok(env.exec(&["prompt", "delete", &prompt_id]));
    assert!(db.list_prompts(&env.workspace).expect("empty").is_empty());

    // 3. Referenced folder add / list / remove
    let ref_dir = env.create_subdir("shared-docs");
    assert_exit_ok(env.exec(&["prompt", "ref", "add", path_str(&ref_dir)]));
    let refs = db.list_referenced_folders(&env.workspace).expect("refs");
    assert_eq!(refs.len(), 1);
    let ref_id = refs[0].id.to_string();
    assert_exit_ok(env.exec(&["prompt", "ref", "list", "--json"]));
    assert_exit_ok(env.exec(&["prompt", "ref", "remove", &ref_id]));
    assert!(
        db.list_referenced_folders(&env.workspace)
            .expect("refs after remove")
            .is_empty()
    );
}

#[test]
fn todo_add_update_status_reorder_rollover_and_stats_end_to_end() {
    let env = CliTestEnv::new("todo-crud");
    let today = today_date();

    assert_exit_ok(env.exec_raw(&[
        "todo",
        "add",
        "Write CLI design doc",
        "--date",
        "2026-11-01",
        "-p",
        "P0",
    ]));
    assert_exit_ok(env.exec_raw(&[
        "todo",
        "add",
        "Implement CLI commands",
        "--date",
        &today,
        "-p",
        "P1",
    ]));
    assert_exit_ok(env.exec_raw(&[
        "todo",
        "add",
        "Run cargo test",
        "--date",
        &today,
        "-p",
        "P2",
    ]));

    let db = env.open_db();
    let all = db.list_todos().expect("list todos");
    assert_eq!(all.len(), 3);
    let past_id = all
        .iter()
        .find(|t| t.entry_date == "2026-11-01")
        .expect("past todo")
        .id
        .clone();
    let today_ids: Vec<String> = all
        .iter()
        .filter(|t| t.entry_date == today)
        .map(|t| t.id.clone())
        .collect();
    assert_eq!(today_ids.len(), 2);

    // Update using unique prefix of ID (trim 1 char from end so sequence suffix stays unique)
    let prefix = &today_ids[0][..today_ids[0].len() - 1];
    assert_exit_ok(env.exec_raw(&[
        "todo",
        "update",
        prefix,
        "--text",
        "Implement full CLI commands",
        "-p",
        "P0",
    ]));
    assert_exit_ok(env.exec_raw(&["todo", "start", &today_ids[0]]));
    assert_exit_ok(env.exec_raw(&["todo", "done", &today_ids[0]]));

    let updated = db
        .list_todos()
        .expect("todos")
        .into_iter()
        .find(|t| t.id == today_ids[0])
        .expect("found");
    assert_eq!(updated.status, TodoStatus::Done);
    assert_eq!(updated.priority, TodoPriority::P0);
    assert_eq!(updated.completed_date.as_deref(), Some(today.as_str()));

    // Reopen and reorder
    assert_exit_ok(env.exec_raw(&["todo", "reopen", &today_ids[0]]));
    assert_exit_ok(env.exec_raw(&[
        "todo",
        "reorder",
        "--date",
        &today,
        &today_ids[1],
        &today_ids[0],
    ]));

    // Rollover past incomplete todo to today
    assert_exit_ok(env.exec_raw(&["todo", "rollover", "--to", &today]));
    let rolled = db
        .list_todos()
        .expect("todos")
        .into_iter()
        .find(|t| t.id == past_id)
        .expect("past");
    assert_eq!(rolled.entry_date, today);

    // List and stats in human and JSON modes
    assert_exit_ok(env.exec_raw(&["todo", "list", "--all"]));
    assert_exit_ok(env.exec_raw(&["todo", "list", "--json"]));
    assert_exit_ok(env.exec_raw(&["todo", "stats", "--days", "7", "--json"]));

    // Delete
    assert_exit_ok(env.exec_raw(&["todo", "delete", &past_id]));
    assert_eq!(db.list_todos().expect("after delete").len(), 2);
}
