//! Schema 测试：八工具名、严格对象与字段白名单。

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
            "ask"
        ]
    );
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
