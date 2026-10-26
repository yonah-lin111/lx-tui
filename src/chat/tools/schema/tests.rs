//! Schema 测试：十一工具名、工具面过滤、严格对象与字段白名单。

use super::*;

#[test]
fn tool_order_and_names_are_fixed() {
    let names: Vec<&str> = available_tools().iter().map(|spec| spec.name).collect();
    assert_eq!(
        names,
        vec![
            "read",
            "search",
            "edit",
            "write",
            "exec_command",
            "write_stdin",
            "update_plan",
            "ask",
            "spawn_agent",
            "wait_agent",
            "close_agent"
        ]
    );
}

/// 主工具面顺序 == 现有 8 项 + 3 个控制工具；子角色只拿到各自白名单。
#[test]
fn tools_for_filters_by_surface() {
    use crate::chat::agent::ToolSurface;
    use crate::chat::tools::{SUBAGENT_TOOL_NAMES, TOOL_NAMES};
    let main: Vec<&str> = tools_for(ToolSurface::Main).iter().map(|s| s.name).collect();
    let expected: Vec<&str> = TOOL_NAMES
        .iter()
        .chain(SUBAGENT_TOOL_NAMES.iter())
        .copied()
        .collect();
    assert_eq!(main, expected);
    let explorer: Vec<&str> = tools_for(ToolSurface::Explorer).iter().map(|s| s.name).collect();
    assert_eq!(explorer, vec!["read", "search"]);
    let executor: Vec<&str> = tools_for(ToolSurface::Executor).iter().map(|s| s.name).collect();
    assert_eq!(
        executor,
        vec!["read", "search", "edit", "write", "exec_command", "write_stdin"]
    );
    for absent in ["ask", "update_plan", "spawn_agent"] {
        assert!(!executor.contains(&absent), "{absent}");
    }
}

/// 新 schema：严格对象、required 与常量同源。
#[test]
fn subagent_schemas_are_strict_and_use_constants() {
    use crate::chat::agent::{DEFAULT_WAIT_TIMEOUT_MS, MAX_TASK_CHARS, MAX_WAIT_TARGETS, MAX_WAIT_TIMEOUT_MS};
    let spec = |name: &str| {
        available_tools()
            .iter()
            .find(|spec| spec.name == name)
            .map(|spec| spec.parameters())
            .unwrap_or_else(|| panic!("{name}"))
    };
    let spawn = spec("spawn_agent");
    assert_eq!(spawn["additionalProperties"], false);
    assert_eq!(spawn["required"], serde_json::json!(["role", "task"]));
    assert_eq!(spawn["properties"]["role"]["enum"], serde_json::json!(["explorer", "executor"]));
    assert_eq!(spawn["properties"]["task"]["maxLength"], MAX_TASK_CHARS);

    let wait = spec("wait_agent");
    assert_eq!(wait["additionalProperties"], false);
    assert_eq!(wait["required"], serde_json::json!(["targets"]));
    assert_eq!(wait["properties"]["targets"]["maxItems"], MAX_WAIT_TARGETS);
    assert_eq!(wait["properties"]["timeout_ms"]["default"], DEFAULT_WAIT_TIMEOUT_MS);
    assert_eq!(wait["properties"]["timeout_ms"]["maximum"], MAX_WAIT_TIMEOUT_MS);

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
