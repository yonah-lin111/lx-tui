//! `workspace` / `mcp` / `skill` / `command` / `model` 的 P0–P2 回归测试。

use std::path::Path;
use std::process::Command;

use serde_json::Value;

use crate::config::mcp::{McpHttpAuth, McpTransportConfig, load_mcp, user_mcp_path};
use crate::settings::commands::CommandKind;

use super::{CliTestEnv, assert_exit_ok, path_str};

fn git_available() -> bool {
    Command::new("git")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
}

fn git_init(dir: &Path) {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["init", "--quiet"])
        .output()
        .expect("run git init");
    assert!(
        output.status.success(),
        "git init failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn same_path(left: &Path, right: &Path) -> bool {
    left == right
        || std::fs::canonicalize(left)
            .is_ok_and(|left| std::fs::canonicalize(right).is_ok_and(|right| left == right))
}

#[test]
fn workspace_delete_include_worktrees_only_removes_subdirectory_target() {
    let env = CliTestEnv::new("workspace-delete-subdir");
    let repo = env.create_subdir("repo");
    if git_available() {
        git_init(&repo);
    }
    let sub = repo.join("sub");
    std::fs::create_dir_all(&sub).expect("create repo subdir");
    let sub = std::fs::canonicalize(&sub).unwrap_or(sub);

    let db = env.open_db();
    db.record_workspace_open(&repo, "repo")
        .expect("register repo");
    db.record_workspace_open(&sub, "repo-sub")
        .expect("register sub");

    // P0-1 regression: `<repo>/sub` is not a main worktree root, so expanding
    // `--include-worktrees` must never pull in `<repo>` (or its linked worktrees).
    assert_exit_ok(env.exec(&["workspace", "delete", path_str(&sub), "--include-worktrees"]));

    let records = db.list_workspaces().expect("workspaces");
    assert!(
        records.iter().any(|record| same_path(&record.path, &repo)),
        "parent repository record must survive subdirectory deletion"
    );
    assert!(
        !records.iter().any(|record| same_path(&record.path, &sub)),
        "registered subdirectory record must be deleted"
    );
    assert!(repo.is_dir(), "parent repository directory must remain");
}

#[test]
fn workspace_add_and_rename_reject_unknown_paths() {
    let env = CliTestEnv::new("workspace-validation");
    let missing = env.workspace.join("missing-dir");

    let err = env
        .exec(&["workspace", "add", path_str(&missing)])
        .expect_err("non-existent path must be rejected");
    assert!(
        err.contains("does not exist") || err.contains("not a directory"),
        "unexpected error: {err}"
    );

    let err = env
        .exec(&["workspace", "rename", "Unregistered"])
        .expect_err("unregistered workspace rename must be rejected");
    assert!(err.contains("not registered"), "unexpected error: {err}");
}

#[test]
fn workspace_reorder_appends_unlisted_workspaces_without_duplicate_positions() {
    let env = CliTestEnv::new("workspace-reorder");
    let first = env.create_subdir("first");
    let second = env.create_subdir("second");
    let third = env.create_subdir("third");
    for path in [&first, &second, &third] {
        assert_exit_ok(env.exec(&["workspace", "add", path_str(path)]));
    }

    // Move only the last entry to the front; unlisted entries keep their relative order.
    assert_exit_ok(env.exec(&["workspace", "reorder", path_str(&third)]));
    let db = env.open_db();
    let records = db.list_workspaces().expect("workspaces");
    let paths: Vec<&Path> = records.iter().map(|record| record.path.as_path()).collect();
    assert_eq!(paths.len(), 3);
    assert!(same_path(paths[0], &third));
    assert!(same_path(paths[1], &first));
    assert!(same_path(paths[2], &second));

    let mut positions: Vec<i64> = records.iter().map(|record| record.position).collect();
    positions.sort_unstable();
    assert_eq!(positions, vec![0, 1, 2], "positions must stay contiguous");
}

fn user_mcp_server(name: &str) -> crate::config::mcp::McpServerConfig {
    let path = user_mcp_path().expect("user mcp path");
    let file = load_mcp(&path).expect("load mcp").expect("mcp file exists");
    file.mcp.servers.get(name).cloned().expect("server exists")
}

#[test]
fn mcp_add_updates_preserve_omitted_http_auth_headers_and_stdio_fields() {
    let env = CliTestEnv::new("mcp-preserve");
    assert_exit_ok(env.exec(&[
        "mcp",
        "add-http",
        "remote",
        "--url",
        "https://mcp.example.com/sse",
        "--auth",
        "bearer",
        "--bearer-token",
        "tok123",
        "--header",
        "X-Key=sec1",
    ]));
    // P1-6 regression: updating only the URL must keep auth, token and headers.
    assert_exit_ok(env.exec(&[
        "mcp",
        "add-http",
        "remote",
        "--url",
        "https://mcp.example.com/v2",
    ]));
    let server = user_mcp_server("remote");
    let McpTransportConfig::Http {
        url,
        auth,
        bearer_token,
        http_headers,
        ..
    } = &server.transport
    else {
        panic!("expected HTTP transport");
    };
    assert_eq!(url, "https://mcp.example.com/v2");
    assert_eq!(*auth, McpHttpAuth::Bearer);
    assert_eq!(bearer_token.as_deref(), Some("tok123"));
    assert_eq!(http_headers.get("X-Key").map(String::as_str), Some("sec1"));

    assert_exit_ok(env.exec(&[
        "mcp",
        "add-stdio",
        "local",
        "--command",
        "npx",
        "--arg",
        "-y",
        "--arg",
        "@modelcontextprotocol/server-filesystem",
        "--env",
        "NODE_ENV=production",
    ]));
    // Updating only the command must keep args and env.
    assert_exit_ok(env.exec(&["mcp", "add-stdio", "local", "--command", "node"]));
    let server = user_mcp_server("local");
    let McpTransportConfig::Stdio {
        command, args, env, ..
    } = &server.transport
    else {
        panic!("expected stdio transport");
    };
    assert_eq!(command, "node");
    assert_eq!(
        args,
        &vec![
            "-y".to_string(),
            "@modelcontextprotocol/server-filesystem".to_string()
        ]
    );
    assert_eq!(env.get("NODE_ENV").map(String::as_str), Some("production"));
}

#[test]
fn skill_and_command_edit_preserve_body_on_disk() {
    let env = CliTestEnv::new("edit-preserve-body");
    assert_exit_ok(env.exec(&[
        "skill",
        "create",
        "rust-audit",
        "--description",
        "Original description",
        "--content",
        "Keep this skill body.",
    ]));
    // P1-5 regression: metadata-only edit in a non-interactive process must not
    // read an empty stdin and overwrite the existing body.
    assert_exit_ok(env.exec(&[
        "skill",
        "edit",
        "rust-audit",
        "--description",
        "Updated description",
    ]));
    let skills_root = crate::config::skills::user_skills_root().expect("skills root");
    let skill_path =
        crate::skills::find_skill_file(&skills_root.join("rust-audit")).expect("skill file");
    let (_, description, body) =
        crate::skills::store::read_skill_at(&skill_path).expect("read skill");
    assert_eq!(description, "Updated description");
    assert!(
        body.contains("Keep this skill body."),
        "skill body was cleared: {body:?}"
    );

    assert_exit_ok(env.exec(&[
        "command",
        "create",
        "review-pr",
        "--kind",
        "agentMD",
        "--description",
        "Original command description",
        "--content",
        "Keep this command body.",
    ]));
    assert_exit_ok(env.exec(&[
        "command",
        "edit",
        "review-pr",
        "--description",
        "Updated command description",
    ]));
    let command_path = crate::config::user_data_root()
        .expect("data root")
        .join("command")
        .join(CommandKind::AgentMd.dir_name())
        .join("review-pr.md");
    let text = std::fs::read_to_string(&command_path).expect("read command file");
    assert!(
        text.contains("Keep this command body."),
        "command body was cleared: {text:?}"
    );
    assert!(text.contains("Updated command description"));
}

fn command_list_json(stdout: &str) -> Value {
    // `--json` emits pretty-printed JSON; the surrounding lines are test-harness noise.
    let start = stdout.find('{').expect("JSON object start");
    let end = stdout.rfind('}').expect("JSON object end");
    let value: Value = serde_json::from_str(&stdout[start..=end])
        .unwrap_or_else(|error| panic!("invalid command list JSON ({error}); stdout:\n{stdout}"));
    assert!(value.get("commands").is_some(), "stdout:\n{stdout}");
    value
}

#[test]
fn command_list_kind_keeps_user_command_shadowed_by_other_project_kind() {
    let env = CliTestEnv::new("command-kind-list");
    env.trust_workspace();
    assert_exit_ok(env.exec(&[
        "command",
        "create",
        "shared-name",
        "--kind",
        "agentMD",
        "--description",
        "User markdown command",
        "--content",
        "User body",
    ]));
    assert_exit_ok(env.exec(&[
        "command",
        "create",
        "shared-name",
        "--kind",
        "agentBlock",
        "--description",
        "Project block command",
        "--block",
        "template",
        "--content",
        "Project body",
        "--scope",
        "project",
    ]));

    // P2-4 regression: filter by kind before merging scopes, otherwise the
    // project agentBlock hides the same-named user agentMD from the listing.
    let stdout = env
        .exec_captured(&[
            "command",
            "-C",
            env.workspace_str(),
            "list",
            "--kind",
            "agentMD",
            "--json",
        ])
        .expect("captured command list");
    let value = command_list_json(&stdout);
    let commands = value["commands"].as_array().expect("commands array");
    assert!(
        commands
            .iter()
            .any(|command| command["name"] == "shared-name"
                && command["kind"] == "agentMD"
                && command["scope"] == "user"),
        "user agentMD must remain listed: {value}"
    );
    assert!(
        commands.iter().all(|command| command["kind"] == "agentMD"),
        "kind filter must exclude agentBlock: {value}"
    );
}

#[test]
fn model_rejects_empty_provider_and_model_ids() {
    let env = CliTestEnv::new("model-empty-ids");
    let err = env
        .exec_raw(&["model", "provider-set", ""])
        .expect_err("empty provider ID must be rejected");
    assert!(
        err.contains("provider ID cannot be empty"),
        "unexpected error: {err}"
    );
    let err = env
        .exec_raw(&["model", "model-set", "acme/"])
        .expect_err("empty model ID must be rejected");
    assert!(
        err.contains("model ID cannot be empty"),
        "unexpected error: {err}"
    );
}
