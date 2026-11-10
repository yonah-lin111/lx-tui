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
fn prompt_move_and_folder_move_update_database_parents() {
    let env = CliTestEnv::new("prompt-move");
    assert_exit_ok(env.exec(&["prompt", "folder", "create", "Target"]));
    assert_exit_ok(env.exec(&["prompt", "folder", "create", "Other"]));
    let db = env.open_db();
    let folders = db.list_folders(&env.workspace).expect("folders");
    let target_id = folders
        .iter()
        .find(|folder| folder.name == "Target")
        .expect("target folder")
        .id;
    let other_id = folders
        .iter()
        .find(|folder| folder.name == "Other")
        .expect("other folder")
        .id;

    assert_exit_ok(env.exec(&["prompt", "folder", "create", "Child"]));
    let child_id = db
        .list_folders(&env.workspace)
        .expect("folders")
        .into_iter()
        .find(|folder| folder.name == "Child")
        .expect("child folder")
        .id;

    assert_exit_ok(env.exec(&["prompt", "create", "Movable Prompt", "--content", "Move me"]));
    let prompt_id = db.list_prompts(&env.workspace).expect("prompts")[0]
        .id
        .to_string();

    assert_exit_ok(env.exec(&[
        "prompt",
        "move",
        &prompt_id,
        "--folder",
        &target_id.to_string(),
    ]));
    let moved = db.list_prompts(&env.workspace).expect("moved prompt");
    assert_eq!(moved[0].folder_id, Some(target_id));

    // P1-3 regression: folder move must read `--parent`, not the prompt `--folder` slot.
    assert_exit_ok(env.exec(&[
        "prompt",
        "folder",
        "move",
        &child_id.to_string(),
        "--parent",
        &other_id.to_string(),
    ]));
    let folders = db.list_folders(&env.workspace).expect("folders after move");
    assert_eq!(
        folders
            .iter()
            .find(|folder| folder.id == child_id)
            .expect("child folder")
            .parent_id,
        Some(other_id)
    );
}

#[test]
fn prompt_writes_are_isolated_to_the_current_workspace() {
    let env = CliTestEnv::new("prompt-isolation");
    let workspace_b = env.create_subdir("workspace-b");
    let workspace_b = path_str(&workspace_b);

    assert_exit_ok(env.exec(&[
        "prompt",
        "create",
        "Workspace A Prompt",
        "--content",
        "Original A body",
    ]));
    let db = env.open_db();
    let prompt = db.list_prompts(&env.workspace).expect("prompts")[0].clone();
    let prompt_id = prompt.id.to_string();
    let page_id = prompt.pages[0].id.to_string();

    // P1-4 regression: workspace B must not be able to edit/delete A's page or prompt.
    let err = env
        .exec_raw(&[
            "prompt",
            "-C",
            workspace_b,
            "page",
            "edit",
            &page_id,
            "--content",
            "hijacked",
        ])
        .expect_err("cross-workspace page edit must be rejected");
    assert!(err.contains("not found"), "unexpected error: {err}");
    let err = env
        .exec_raw(&["prompt", "-C", workspace_b, "page", "delete", &page_id])
        .expect_err("cross-workspace page delete must be rejected");
    assert!(err.contains("not found"), "unexpected error: {err}");
    let err = env
        .exec_raw(&["prompt", "-C", workspace_b, "delete", &prompt_id])
        .expect_err("cross-workspace prompt delete must be rejected");
    assert!(err.contains("not found"), "unexpected error: {err}");

    let prompts = db.list_prompts(&env.workspace).expect("prompts intact");
    assert_eq!(prompts.len(), 1);
    assert_eq!(prompts[0].id, prompt.id);
    assert_eq!(prompts[0].pages.len(), 1);
    assert_eq!(prompts[0].pages[0].id, prompt.pages[0].id);
    assert_eq!(prompts[0].pages[0].content, "Original A body");
    assert!(
        db.list_prompts(std::path::Path::new(workspace_b))
            .expect("workspace b prompts")
            .is_empty()
    );
}

#[test]
fn todo_text_flag_preserves_completion_and_rejects_rollover_for_done() {
    let env = CliTestEnv::new("todo-regressions");

    // P2-3 regression: `--text` is a valid alternative to the positional TEXT.
    assert_exit_ok(env.exec_raw(&["todo", "add", "--text", "From flag"]));
    let db = env.open_db();
    let todos = db.list_todos().expect("todos");
    assert_eq!(todos.len(), 1);
    assert_eq!(todos[0].text, "From flag");
    let todo_id = todos[0].id.clone();

    // Mark done with a historical completion date, then run `done` again.
    let mut done = todos[0].clone();
    done.status = TodoStatus::Done;
    done.completed_date = Some("2000-01-01".to_string());
    db.save_todo(&done).expect("seed completed todo");
    assert_exit_ok(env.exec_raw(&["todo", "done", &todo_id]));
    let completed = db
        .list_todos()
        .expect("todos")
        .into_iter()
        .find(|todo| todo.id == todo_id)
        .expect("todo");
    assert_eq!(completed.status, TodoStatus::Done);
    assert_eq!(
        completed.completed_date.as_deref(),
        Some("2000-01-01"),
        "repeated `todo done` must preserve the original completed_date"
    );

    // P2-3 regression: completed todos cannot be rolled over.
    let err = env
        .exec_raw(&["todo", "rollover", "--id", &todo_id])
        .expect_err("rollover of a done todo must be rejected");
    assert!(
        err.contains("cannot roll over completed todo"),
        "unexpected error: {err}"
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

#[test]
fn cli_todo_group_ungroup_and_status_sync_across_group() {
    let env = CliTestEnv::new("todo-group-cli");
    let date = "2026-11-10";

    for (text, priority) in [
        ("Write login API", "P1"),
        ("Standalone task", "P2"),
        ("Write login unit tests", "P1"),
        ("Fix home page style", "P3"),
    ] {
        assert_exit_ok(env.exec_raw(&["todo", "add", text, "--date", date, "-p", priority]));
    }

    let db = env.open_db();
    let initial = db.list_todos_by_date(date).expect("initial todos");
    assert_eq!(initial.len(), 4);
    let id_api = initial[0].id.clone();
    let id_standalone = initial[1].id.clone();
    let id_test = initial[2].id.clone();
    let id_style = initial[3].id.clone();

    // Group non-adjacent items [id_api, id_test, id_style]: makes them contiguous and shares group_id.
    assert_exit_ok(env.exec_raw(&["todo", "group", &id_api, &id_test, &id_style]));
    let grouped = db.list_todos_by_date(date).expect("grouped todos");
    assert_eq!(
        grouped.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(),
        vec![
            id_api.as_str(),
            id_test.as_str(),
            id_style.as_str(),
            id_standalone.as_str(),
        ]
    );
    let gid = grouped[0].group_id.clone().expect("group_id assigned");
    assert_eq!(grouped[1].group_id.as_deref(), Some(gid.as_str()));
    assert_eq!(grouped[2].group_id.as_deref(), Some(gid.as_str()));
    assert_eq!(grouped[3].group_id, None);

    // Status changes (`start`, `done`, `reopen`, `update --status`) sync across the whole group.
    assert_exit_ok(env.exec_raw(&["todo", "start", &id_test]));
    for item in &db.list_todos_by_date(date).expect("after start")[..3] {
        assert_eq!(item.status, TodoStatus::InProgress);
    }
    assert_exit_ok(env.exec_raw(&["todo", "done", &id_api]));
    for item in &db.list_todos_by_date(date).expect("after done")[..3] {
        assert_eq!(item.status, TodoStatus::Done);
        assert!(item.completed_date.is_some());
    }
    assert_exit_ok(env.exec_raw(&["todo", "update", &id_style, "--status", "todo"]));
    for item in &db.list_todos_by_date(date).expect("after update status")[..3] {
        assert_eq!(item.status, TodoStatus::Todo);
        assert_eq!(item.completed_date, None);
    }

    // Split one item (`ungroup <ID>`) detaches it to immediately after the remaining 2-item group.
    assert_exit_ok(env.exec_raw(&["todo", "ungroup", &id_test]));
    let after_split = db.list_todos_by_date(date).expect("after split");
    assert_eq!(
        after_split
            .iter()
            .map(|t| (t.id.as_str(), t.group_id.as_deref()))
            .collect::<Vec<_>>(),
        vec![
            (id_api.as_str(), Some(gid.as_str())),
            (id_style.as_str(), Some(gid.as_str())),
            (id_test.as_str(), None),
            (id_standalone.as_str(), None),
        ]
    );

    // Dissolve the remaining group with `ungroup <ID> --all`.
    assert_exit_ok(env.exec_raw(&["todo", "ungroup", &id_api, "--all"]));
    for item in db.list_todos_by_date(date).expect("after ungroup all") {
        assert_eq!(item.group_id, None);
    }

    // Passing multiple IDs to `ungroup` must return a clear single-ID error.
    let multi_err = env
        .exec_raw(&["todo", "ungroup", "--id", &id_api, "--id", &id_style])
        .expect_err("multiple IDs in ungroup must fail");
    assert!(
        multi_err.contains("expected a single <TODO_ID>"),
        "unexpected error: {multi_err}"
    );
}

#[test]
fn cli_todo_group_preserves_existing_member_order_and_rollover_keeps_group() {
    let env = CliTestEnv::new("todo-group-order-rollover");
    let past_date = "2026-11-08";
    let target_date = "2026-11-10";

    for text in ["Step 1", "Step 2", "Step 3"] {
        assert_exit_ok(env.exec_raw(&["todo", "add", text, "--date", past_date]));
    }
    let db = env.open_db();
    let initial = db.list_todos_by_date(past_date).expect("initial todos");
    let id_1 = initial[0].id.clone();
    let id_2 = initial[1].id.clone();
    let id_3 = initial[2].id.clone();

    // Group [id_1, id_2], then merge id_3 into the existing group via `todo group id_1 id_3`.
    assert_exit_ok(env.exec_raw(&["todo", "group", &id_1, &id_2]));
    assert_exit_ok(env.exec_raw(&["todo", "group", &id_1, &id_3]));
    let grouped = db.list_todos_by_date(past_date).expect("after 2nd group");
    assert_eq!(
        grouped.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(),
        vec![id_1.as_str(), id_2.as_str(), id_3.as_str()],
        "merging into an existing group must preserve existing member order [id_1, id_2]"
    );
    let gid = grouped[0].group_id.clone().expect("group_id");
    assert!(
        grouped
            .iter()
            .all(|t| t.group_id.as_deref() == Some(gid.as_str()))
    );

    // Seed historical completed_date on id_1 and verify `update --status done` preserves and syncs it.
    let mut seeded = grouped.clone();
    for item in &mut seeded {
        item.status = TodoStatus::Done;
        item.completed_date = Some("2000-01-01".to_string());
    }
    db.sync_date_todos(past_date, &seeded).expect("seed done");
    assert_exit_ok(env.exec_raw(&["todo", "update", &id_1, "--status", "done"]));
    for item in db.list_todos_by_date(past_date).expect("after update done") {
        assert_eq!(item.status, TodoStatus::Done);
        assert_eq!(item.completed_date.as_deref(), Some("2000-01-01"));
    }

    // Reopen the group and roll over by specifying only one member `--id <id_2>`:
    // all incomplete groupmates on the same date must roll over together and keep their group_id.
    assert_exit_ok(env.exec_raw(&["todo", "reopen", &id_1]));
    assert_exit_ok(env.exec_raw(&["todo", "rollover", "--to", target_date, "--id", &id_2]));
    assert!(
        db.list_todos_by_date(past_date)
            .expect("past empty")
            .is_empty()
    );
    let rolled = db.list_todos_by_date(target_date).expect("target todos");
    assert_eq!(
        rolled.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(),
        vec![id_1.as_str(), id_2.as_str(), id_3.as_str()]
    );
    for item in &rolled {
        assert_eq!(item.group_id.as_deref(), Some(gid.as_str()));
    }
}
