//! Schema 测试：十七工具名、工具面过滤、严格对象与字段白名单。

use super::*;

#[test]
fn tool_order_and_names_are_fixed() {
    let names: Vec<&str> = available_tools().iter().map(|spec| spec.name).collect();
    assert_eq!(
        names,
        vec![
            "read",
            "search",
            "web_search",
            "edit",
            "write",
            "exec_command",
            "write_stdin",
            "update_plan",
            "ask",
            "curr_time",
            "spawn_agent",
            "send_input",
            "interrupt_agent",
            "resume_agent",
            "wait_agent",
            "close_agent",
            "list_agents",
            "cancel_agent_input",
        ]
    );
}

/// 主工具面顺序 == `TOOL_NAMES` + 控制工具；子角色只拿到各自白名单。
#[test]
fn tools_for_filters_by_surface() {
    use crate::chat::agent::ToolSurface;
    use crate::chat::tools::{SUBAGENT_TOOL_NAMES, TOOL_NAMES};
    let main: Vec<&str> = tools_for(ToolSurface::Main)
        .iter()
        .map(|s| s.name)
        .collect();
    let expected: Vec<&str> = TOOL_NAMES
        .iter()
        .chain(SUBAGENT_TOOL_NAMES.iter())
        .copied()
        .collect();
    assert_eq!(main, expected);
    let explorer: Vec<&str> = tools_for(ToolSurface::Explorer)
        .iter()
        .map(|s| s.name)
        .collect();
    assert_eq!(explorer, vec!["read", "search", "web_search"]);
    let executor: Vec<&str> = tools_for(ToolSurface::Executor)
        .iter()
        .map(|s| s.name)
        .collect();
    assert_eq!(
        executor,
        vec![
            "read",
            "search",
            "web_search",
            "edit",
            "write",
            "exec_command",
            "write_stdin"
        ]
    );
    for absent in ["ask", "update_plan", "spawn_agent"] {
        assert!(!executor.contains(&absent), "{absent}");
    }
}

/// 新 schema：严格对象、required 与常量同源。
#[test]
fn subagent_schemas_are_strict_and_use_constants() {
    use crate::chat::agent::{
        DEFAULT_WAIT_TIMEOUT_MS, MAX_TASK_CHARS, MAX_WAIT_TARGETS, MAX_WAIT_TIMEOUT_MS,
    };
    let spec = |name: &str| {
        available_tools()
            .iter()
            .find(|spec| spec.name == name)
            .map(|spec| spec.parameters())
            .unwrap_or_else(|| panic!("{name}"))
    };
    let spawn = spec("spawn_agent");
    assert_eq!(spawn["additionalProperties"], false);
    assert_eq!(spawn["required"], serde_json::json!(["role", "title"]));
    assert_eq!(
        spawn["properties"]["title"]["maxLength"],
        crate::chat::agent::MAX_TITLE_CHARS
    );
    assert_eq!(spawn["properties"]["role"]["type"], "string");
    assert_eq!(
        spawn["properties"]["role"]["maxLength"],
        crate::chat::agent::MAX_ROLE_NAME_LEN
    );
    assert!(spawn["properties"]["role"].get("enum").is_none());
    assert_eq!(spawn["properties"]["fork_context"]["type"], "boolean");
    assert_eq!(spawn["properties"]["fork_context"]["default"], false);
    assert_eq!(spawn["properties"]["model"]["maxLength"], 128);
    assert_eq!(spawn["properties"]["reasoning_effort"]["maxLength"], 64);
    assert_eq!(spawn["properties"]["task"]["maxLength"], MAX_TASK_CHARS);
    assert!(spawn["properties"]["items"].is_object());

    let interrupt = spec("interrupt_agent");
    assert_eq!(interrupt["additionalProperties"], false);
    assert_eq!(interrupt["required"], serde_json::json!(["target"]));

    let wait = spec("wait_agent");
    assert_eq!(wait["additionalProperties"], false);
    assert_eq!(wait["required"], serde_json::json!([]));
    assert_eq!(wait["properties"]["targets"]["maxItems"], MAX_WAIT_TARGETS);
    assert_eq!(
        wait["properties"]["wait_for"]["enum"],
        serde_json::json!(["all", "any"])
    );
    assert_eq!(wait["properties"]["wait_for"]["default"], "all");
    assert_eq!(
        wait["properties"]["timeout_ms"]["default"],
        DEFAULT_WAIT_TIMEOUT_MS
    );
    assert_eq!(
        wait["properties"]["timeout_ms"]["maximum"],
        MAX_WAIT_TIMEOUT_MS
    );

    let close = spec("close_agent");
    assert_eq!(close["additionalProperties"], false);
    assert_eq!(close["required"], serde_json::json!(["target"]));
}

#[test]
fn exec_command_schema_documents_limits() {
    let spec = available_tools()
        .iter()
        .find(|spec| spec.name == "exec_command")
        .expect("exec_command");
    let parameters = spec.parameters();
    assert_eq!(parameters["required"], serde_json::json!(["cmd"]));
    for field in [
        "cmd",
        "workdir",
        "shell",
        "login",
        "tty",
        "yield_time_ms",
        "timeout_ms",
        "max_output_tokens",
    ] {
        assert!(
            parameters["properties"].get(field).is_some(),
            "exec_command schema missing {field}"
        );
    }
    assert_eq!(
        parameters["properties"]["yield_time_ms"]["maximum"],
        crate::chat::process::MAX_YIELD_MS
    );
    assert_eq!(
        parameters["properties"]["timeout_ms"]["maximum"],
        crate::chat::process::MAX_TIMEOUT_MS
    );
}

#[test]
fn write_stdin_and_update_plan_schemas_are_strict() {
    let stdin = available_tools()
        .iter()
        .find(|spec| spec.name == "write_stdin")
        .expect("write_stdin");
    assert_eq!(
        stdin.parameters()["required"],
        serde_json::json!(["session_id"])
    );
    let plan = available_tools()
        .iter()
        .find(|spec| spec.name == "update_plan")
        .expect("update_plan");
    let parameters = plan.parameters();
    assert_eq!(parameters["required"], serde_json::json!(["plan"]));
    assert_eq!(
        parameters["properties"]["plan"]["items"]["additionalProperties"],
        false
    );
    assert_eq!(
        parameters["properties"]["plan"]["items"]["properties"]["status"]["enum"],
        serde_json::json!(["pending", "in_progress", "completed"])
    );
}

#[test]
fn all_schemas_reject_additional_properties() {
    for spec in available_tools() {
        let parameters = spec.parameters();
        assert_eq!(parameters["additionalProperties"], false, "{}", spec.name);
        assert_eq!(parameters["type"], "object", "{}", spec.name);
    }
}

#[test]
fn read_schema_documents_one_based_offset() {
    let read = available_tools()
        .iter()
        .find(|spec| spec.name == "read")
        .expect("read");
    let parameters = read.parameters();
    assert_eq!(parameters["properties"]["offset"]["minimum"], 1);
    let description = parameters["properties"]["offset"]["description"]
        .as_str()
        .expect("description");
    assert!(description.contains("1-based"));
    assert_eq!(parameters["required"], serde_json::json!(["path"]));
}

#[test]
fn edit_schema_has_patch_and_read_versions() {
    let edit = available_tools()
        .iter()
        .find(|spec| spec.name == "edit")
        .expect("edit");
    let parameters = edit.parameters();
    let properties = parameters["properties"].as_object().expect("properties");
    // `read_versions` 是读后版本保护的唯一入口：字符串映射，不能是数组/数字。
    assert_eq!(properties.len(), 2);
    assert!(properties.contains_key("patch"));
    assert!(properties.contains_key("read_versions"));
    assert_eq!(
        parameters["properties"]["read_versions"]["additionalProperties"],
        serde_json::json!({"type": "string"})
    );
    assert_eq!(parameters["required"], serde_json::json!(["patch"]));
}

#[test]
fn search_schema_optional_fields() {
    let search = available_tools()
        .iter()
        .find(|spec| spec.name == "search")
        .expect("search");
    let parameters = search.parameters();
    assert_eq!(parameters["required"], serde_json::json!(["pattern"]));
    for field in ["path", "include", "literal", "case_sensitive", "limit"] {
        assert!(
            parameters["properties"].get(field).is_some(),
            "search schema missing {field}"
        );
    }
}

#[test]
fn write_schema_requires_path_and_content() {
    let write = available_tools()
        .iter()
        .find(|spec| spec.name == "write")
        .expect("write");
    let parameters = write.parameters();
    assert_eq!(
        parameters["required"],
        serde_json::json!(["path", "content"])
    );
}

/// `ask` 的严格形状：必填 `questions`、题/选项都不接受额外字段、数字来自常量。
#[test]
fn ask_schema_is_strict_and_documents_limits() {
    let spec = available_tools()
        .iter()
        .find(|spec| spec.name == "ask")
        .expect("ask spec");
    assert!(!spec.description.is_empty(), "ask 必须有描述");
    assert!(spec.description.contains("1-3"), "{}", spec.description);
    let parameters = spec.parameters();
    assert_eq!(parameters["required"], serde_json::json!(["questions"]));
    assert_eq!(parameters["additionalProperties"], false);
    let questions = &parameters["properties"]["questions"];
    assert_eq!(questions["minItems"], 1);
    assert_eq!(
        questions["maxItems"],
        crate::chat::tools::ask::MAX_ASK_QUESTIONS
    );
    let question = &questions["items"];
    assert_eq!(
        question["required"],
        serde_json::json!(["id", "header", "question"])
    );
    assert_eq!(question["additionalProperties"], false);
    let options = &question["properties"]["options"];
    assert_eq!(
        options["maxItems"],
        crate::chat::tools::ask::MAX_ASK_OPTIONS
    );
    let option = &options["items"];
    assert_eq!(
        option["required"],
        serde_json::json!(["label", "description"])
    );
    assert_eq!(option["additionalProperties"], false);
    // 所有上限文案都由常量生成，不手写第二份数字。
    assert!(
        question["properties"]["header"]["description"]
            .as_str()
            .expect("description")
            .contains(&crate::chat::tools::ask::MAX_ASK_HEADER_CHARS.to_string())
    );
}

#[test]
fn turn_tools_with_subagents_injects_custom_roles_into_spawn() {
    use crate::config::subagents::{BaseRole, CustomRoleConfig, SubagentsSection};

    let section = SubagentsSection {
        roles: vec![CustomRoleConfig {
            name: "reviewer".into(),
            base_role: BaseRole::Explorer,
            description: "Strict security and regression reviewer.".into(),
            system_prompt: "You are a reviewer.".into(),
            model: None,
            nickname_candidates: Vec::new(),
            ..CustomRoleConfig::default()
        }],
        ..SubagentsSection::default()
    };
    let decls = super::turn_tools_with_subagents(
        crate::chat::agent::ToolSurface::Main,
        &crate::mcp::McpToolSnapshot::default(),
        Some(&section),
    );
    let spawn = decls
        .iter()
        .find(|decl| decl.name == "spawn_agent")
        .expect("spawn_agent");
    assert!(spawn.description.contains("`reviewer`"));
    assert!(
        spawn
            .description
            .contains("Strict security and regression reviewer.")
    );
    assert_eq!(
        spawn.parameters["properties"]["role"]["enum"],
        serde_json::json!(["explorer", "executor", "monitor", "reviewer"])
    );
    let read = decls.iter().find(|decl| decl.name == "read").expect("read");
    assert!(!read.description.contains("reviewer"));
}

#[test]
fn turn_tools_for_subagent_narrows_capabilities() {
    use crate::chat::agent::ToolSurface;
    use crate::config::subagents::RoleCapabilities;

    let snapshot = crate::mcp::McpToolSnapshot::default();
    let open = super::turn_tools_for_subagent(
        ToolSurface::Executor,
        RoleCapabilities::default(),
        &snapshot,
        None,
    );
    assert!(open.iter().any(|decl| decl.name == "web_search"));

    let narrowed = super::turn_tools_for_subagent(
        ToolSurface::Executor,
        RoleCapabilities {
            allow_web_search: false,
            allow_mcp: false,
            allow_skills: false,
        },
        &snapshot,
        None,
    );
    assert!(
        !narrowed.iter().any(|decl| decl.name == "web_search"),
        "web_search is removed"
    );
    assert!(
        !narrowed.iter().any(|decl| decl.name.starts_with("mcp__")),
        "MCP tools are not injected"
    );
    assert!(
        !narrowed
            .iter()
            .any(|decl| decl.name == "list_resources" || decl.name == "read_resource"),
        "MCP resource tools are not injected"
    );
    // 内置工具面不受能力收紧影响。
    for name in ["read", "search", "edit", "write", "exec_command"] {
        assert!(narrowed.iter().any(|decl| decl.name == name), "{name}");
    }
}

#[test]
fn build_spawn_role_guidance_shows_locked_model_and_restrictions() {
    use crate::chat::tools::build_spawn_role_guidance;
    use crate::config::ai::ModelSelection;
    use crate::config::subagents::{BaseRole, CustomRoleConfig, SubagentsSection};

    let section = SubagentsSection {
        roles: vec![CustomRoleConfig {
            name: "reviewer".into(),
            base_role: BaseRole::Explorer,
            description: "Strict security reviewer.".into(),
            system_prompt: "You review.".into(),
            model: Some(ModelSelection {
                provider: "openai".into(),
                model: "gpt-4.1-mini".into(),
                variant: Some("high".into()),
            }),
            lock_model: true,
            capabilities: crate::config::subagents::RoleCapabilities {
                allow_web_search: false,
                allow_mcp: false,
                allow_skills: true,
            },
            ..CustomRoleConfig::default()
        }],
        ..SubagentsSection::default()
    };
    let guidance = build_spawn_role_guidance(&section);
    assert!(guidance.contains("`reviewer`"));
    assert!(guidance.contains("locked model: openai/gpt-4.1-mini (high)"));
    assert!(guidance.contains("cannot be overridden"));
    assert!(guidance.contains("[no web_search, no mcp]"));
    assert!(guidance.contains("`monitor`"));
}

#[test]
fn turn_tools_falls_back_to_builtin_and_resources_are_named() {
    use crate::chat::agent::ToolSurface;
    let snapshot = crate::mcp::McpToolSnapshot::default();
    assert_eq!(
        super::turn_tools(ToolSurface::Explorer, &snapshot),
        super::builtin_tools(ToolSurface::Explorer)
    );
    let names: Vec<String> = super::resource_tools()
        .into_iter()
        .map(|tool| tool.name)
        .collect();
    assert_eq!(
        names,
        ["list_resources", "list_resource_templates", "read_resource"]
    );
}
