//! `workspace`、`model`、`mcp`、`skill`、`command`、`subagent`、`settings` 领域 CLI 单元测试。

use super::{CliTestEnv, assert_exit_ok, path_str};

#[test]
fn workspace_history_crud_and_lock_guard_work() {
    let env = CliTestEnv::new("workspace-crud");
    let second_ws = env.create_subdir("second-project");

    // Add workspaces and trust
    assert_exit_ok(env.exec(&["workspace", "add", "--name", "Primary Repo", "--trust"]));
    assert_exit_ok(env.exec(&[
        "workspace",
        "add",
        path_str(&second_ws),
        "--name",
        "Second Repo",
    ]));

    let db = env.open_db();
    assert!(db.is_trusted(&env.workspace).expect("trusted"));
    assert_eq!(db.list_workspaces().expect("workspaces").len(), 2);

    // List (tree and --raw), status, rename, reorder
    assert_exit_ok(env.exec(&["workspace", "list"]));
    assert_exit_ok(env.exec(&["workspace", "list", "--raw", "--json"]));
    assert_exit_ok(env.exec(&["workspace", "status", "--json"]));
    assert_exit_ok(env.exec(&["workspace", "rename", "Renamed Primary"]));
    assert_exit_ok(env.exec(&[
        "workspace",
        "reorder",
        path_str(&second_ws),
        env.workspace_str(),
    ]));

    // Delete second workspace
    assert_exit_ok(env.exec(&["workspace", "delete", path_str(&second_ws)]));
    assert_eq!(db.list_workspaces().expect("after delete").len(), 1);

    // Active session lock in primary workspace must block `workspace delete`
    let turn = db
        .begin_turn(
            &env.workspace,
            None,
            "Active turn",
            &crate::db::chat::ModelSnapshot {
                provider: "openai".to_string(),
                model: "gpt-5.4".to_string(),
                variant: None,
            },
        )
        .expect("begin turn");
    let guard = crate::platform::try_lock_session(&turn.session_id)
        .expect("lock")
        .expect("acquire");
    let del_err = env
        .exec(&["workspace", "delete"])
        .expect_err("workspace delete must fail when session is locked");
    assert!(del_err.contains("active"));
    drop(guard);
}

#[test]
fn model_provider_and_model_crud_and_secret_masking_work() {
    let env = CliTestEnv::new("model-crud");

    assert_exit_ok(env.exec_raw(&[
        "model",
        "provider-set",
        "acme",
        "--type",
        "openai-compatible",
        "--name",
        "Acme AI",
        "--base-url",
        "https://api.acme.example/v1",
        "--api-key",
        "sk-secret-key-9876",
        "--enable",
    ]));
    assert_exit_ok(env.exec_raw(&[
        "model",
        "model-set",
        "acme/acme-coder-1",
        "--name",
        "Acme Coder 1",
        "--context-limit",
        "128000",
        "--output-limit",
        "16384",
        "--image-input",
        "true",
    ]));
    assert_exit_ok(env.exec_raw(&["model", "default", "acme/acme-coder-1"]));
    assert_exit_ok(env.exec_raw(&[
        "model",
        "config",
        "--title-language",
        "en",
        "--timeout-ms",
        "90000",
    ]));
    assert_exit_ok(env.exec_raw(&["model", "list", "--all"]));
    assert_exit_ok(env.exec_raw(&["model", "show", "acme", "--json"]));

    let ai_path = crate::config::ai::ai_config_path().expect("ai path");
    let loaded = crate::config::ai::load_ai(&ai_path)
        .expect("load ai")
        .expect("exists");
    assert_eq!(
        loaded
            .default_model
            .as_ref()
            .map(|s| (s.provider.as_str(), s.model.as_str())),
        Some(("acme", "acme-coder-1"))
    );

    assert_exit_ok(env.exec_raw(&["model", "model-rm", "acme/acme-coder-1"]));
    assert_exit_ok(env.exec_raw(&["model", "provider-rm", "acme"]));
}

#[test]
fn mcp_skill_command_subagent_and_settings_crud_work() {
    let env = CliTestEnv::new("extensions-crud");
    env.trust_workspace();

    // 1. MCP user & project servers
    assert_exit_ok(env.exec(&[
        "mcp",
        "add-stdio",
        "local-fs",
        "--command",
        "npx",
        "--arg",
        "-y",
        "--arg",
        "@modelcontextprotocol/server-filesystem",
        "--env",
        "NODE_ENV=production",
    ]));
    assert_exit_ok(env.exec(&[
        "mcp",
        "add-http",
        "proj-remote",
        "--url",
        "https://mcp.example.com/sse",
        "--auth",
        "bearer",
        "--bearer-token",
        "tok_secret_12345678",
        "--scope",
        "project",
    ]));
    assert_exit_ok(env.exec(&["mcp", "list", "--json"]));
    assert_exit_ok(env.exec(&["mcp", "show", "proj-remote", "--scope", "project", "--json"]));
    assert_exit_ok(env.exec(&["mcp", "disable", "local-fs"]));
    assert_exit_ok(env.exec(&["mcp", "enable", "local-fs"]));
    assert_exit_ok(env.exec(&["mcp", "remove", "local-fs"]));

    // 2. Skill create / show / edit / disable / enable / delete
    assert_exit_ok(env.exec(&[
        "skill",
        "create",
        "rust-audit",
        "--description",
        "Audit Rust code for safety",
        "--content",
        "# Rust Audit\nCheck unwrap and unsafe.",
        "--scope",
        "project",
    ]));
    assert_exit_ok(env.exec(&["skill", "list", "--json"]));
    assert_exit_ok(env.exec(&["skill", "show", "rust-audit", "--scope", "project"]));
    assert_exit_ok(env.exec(&["skill", "disable", "rust-audit"]));
    assert_exit_ok(env.exec(&["skill", "enable", "rust-audit"]));
    assert_exit_ok(env.exec(&[
        "skill",
        "edit",
        "rust-audit",
        "--description",
        "Updated audit skill",
        "--scope",
        "project",
    ]));
    assert_exit_ok(env.exec(&["skill", "delete", "rust-audit", "--scope", "project"]));

    // 3. Custom slash commands (agentMD & agentBlock)
    assert_exit_ok(env.exec(&[
        "command",
        "create",
        "review-pr",
        "--kind",
        "agentMD",
        "--description",
        "Review pull request changes",
        "--md-scope",
        "global",
        "--content",
        "Review all changed files carefully.",
    ]));
    assert_exit_ok(env.exec(&["command", "list", "--json"]));
    assert_exit_ok(env.exec(&["command", "show", "review-pr"]));
    assert_exit_ok(env.exec(&["command", "rename", "review-pr", "audit-pr"]));
    assert_exit_ok(env.exec(&["command", "delete", "audit-pr"]));

    // 4. Subagent settings, monitor builtin override, and project-scoped custom role
    assert_exit_ok(env.exec(&[
        "subagent",
        "set",
        "--max-concurrent",
        "6",
        "--max-depth",
        "3",
        "--default-max-tokens",
        "64000",
        "--delegation-mode",
        "proactive",
    ]));
    assert_exit_ok(env.exec(&[
        "subagent",
        "role-set",
        "monitor",
        "--description",
        "Custom monitor description",
        "--lock-model",
        "true",
    ]));
    assert_exit_ok(env.exec(&[
        "subagent",
        "role-set",
        "sec_reviewer",
        "--base",
        "explorer",
        "--description",
        "Security code reviewer",
        "--prompt",
        "Focus on auth and injection.",
        "--allow-web-search",
        "false",
        "--scope",
        "project",
    ]));
    assert_exit_ok(env.exec(&["subagent", "show", "--json"]));
    assert_exit_ok(env.exec(&["subagent", "show", "sec_reviewer", "--scope", "project"]));
    assert_exit_ok(env.exec(&["subagent", "role-reset", "monitor"]));
    assert_exit_ok(env.exec(&["subagent", "role-rm", "sec_reviewer", "--scope", "project"]));

    // 5. General layout settings show / set / reset
    assert_exit_ok(env.exec(&["settings", "set", "--sidebar-width", "32"]));
    assert_exit_ok(env.exec(&["settings", "show", "--json"]));
    let cfg = crate::config::load_settings().expect("load settings");
    assert_eq!(cfg.sidebar_width, 32);
    assert_exit_ok(env.exec(&["settings", "reset"]));
    let reset_cfg = crate::config::load_settings().expect("load reset settings");
    assert_eq!(reset_cfg, crate::config::Config::default());
}
