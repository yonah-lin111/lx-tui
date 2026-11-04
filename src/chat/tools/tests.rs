//! 工具执行层入口测试：分发、契约、路径解析、格式化、摘要、落库 JSON 与安全性。

use super::test_support::*;
use super::*;

/// 工具名字固定且英文（内置工具 + 子 agent 控制工具）。
#[test]
fn available_tools_are_the_seventeen_stable_names() {
    let names: Vec<&str> = available_tools().iter().map(|spec| spec.name).collect();
    let expected: Vec<&str> = crate::chat::tools::TOOL_NAMES
        .iter()
        .chain(crate::chat::tools::SUBAGENT_TOOL_NAMES.iter())
        .copied()
        .collect();
    assert_eq!(names, expected);
    for spec in available_tools() {
        assert!(
            !spec.description.is_empty(),
            "{} needs a description",
            spec.name
        );
    }
}

/// 每个 schema 都是严格对象：required 完整、`additionalProperties=false`、
/// `edit` 只有 `patch` 字段。
#[test]
fn schemas_are_strict_objects() {
    for spec in available_tools() {
        let parameters = spec.parameters();
        assert_eq!(parameters["type"], "object", "{} type", spec.name);
        assert_eq!(
            parameters["additionalProperties"], false,
            "{} must reject extra fields",
            spec.name
        );
        let required = parameters["required"]
            .as_array()
            .unwrap_or_else(|| panic!("{} required", spec.name));
        let properties = parameters["properties"]
            .as_object()
            .unwrap_or_else(|| panic!("{} properties", spec.name));
        for field in required {
            let name = field.as_str().expect("required is string");
            assert!(
                properties.contains_key(name),
                "{} missing {name}",
                spec.name
            );
        }
    }
    let edit = available_tools()
        .iter()
        .find(|spec| spec.name == "edit")
        .expect("edit spec");
    let edit_parameters = edit.parameters();
    let keys: Vec<&str> = edit_parameters["properties"]
        .as_object()
        .expect("edit properties")
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(keys, vec!["patch", "read_versions"]);
    // read 的 offset 必须写明 1-based。
    let read = available_tools()
        .iter()
        .find(|spec| spec.name == "read")
        .expect("read spec");
    let read_parameters = read.parameters();
    let offset = read_parameters["properties"]["offset"]["description"]
        .as_str()
        .expect("offset description");
    assert!(offset.contains("1-based"), "offset description: {offset}");
}

/// 未知工具名返回结构化失败，且不产生副作用。
#[test]
fn unknown_tool_is_a_structured_failure() {
    let fixture = Fixture::new("unknown-tool");
    fixture.write("a.txt", "keep\n");
    let result = execute(&fixture.ctx(), "shell", r#"{"path":"a.txt"}"#);
    assert_eq!(expect_failure(&result), ToolErrorCode::UnknownTool);
    assert_eq!(fixture.read("a.txt"), "keep\n");
    assert!(format_for_model(&result).contains("unknown_tool"));
}

/// 多余字段被拒绝，且目标文件前后一致。
#[test]
fn extra_arguments_are_rejected_without_side_effects() {
    let fixture = Fixture::new("extra-args");
    fixture.write("a.txt", "original\n");
    let before = fixture.read_bytes("a.txt");
    let result = execute(
        &fixture.ctx(),
        "write",
        r#"{"path":"a.txt","content":"x","append":true}"#,
    );
    assert_eq!(expect_failure(&result), ToolErrorCode::InvalidArguments);
    assert_eq!(fixture.read_bytes("a.txt"), before);
    assert!(!format_for_model(&result).contains("append\":true"));
}

/// 无效 JSON 同样失败且无副作用。
#[test]
fn invalid_json_is_rejected_without_side_effects() {
    let fixture = Fixture::new("invalid-json");
    fixture.write("a.txt", "original\n");
    let result = execute(&fixture.ctx(), "edit", "{not json");
    assert_eq!(expect_failure(&result), ToolErrorCode::InvalidArguments);
    assert_eq!(fixture.read("a.txt"), "original\n");
}

/// 非对象 JSON、错误类型与 0 值参数都被拒绝。
#[test]
fn argument_types_are_validated() {
    let fixture = Fixture::new("arg-types");
    fixture.write("a.txt", "x\n");
    let ctx = fixture.ctx();
    for arguments in [
        "[]",
        r#"{"path":42}"#,
        r#"{"path":"a.txt","offset":0}"#,
        r#"{"path":"a.txt","offset":-1}"#,
        r#"{"path":"a.txt","offset":"2"}"#,
        r#"{"path":"a.txt","limit":1.5}"#,
        r#"{"path":""}"#,
    ] {
        let result = execute(&ctx, "read", arguments);
        assert_eq!(
            expect_failure(&result),
            ToolErrorCode::InvalidArguments,
            "arguments {arguments} must be rejected"
        );
    }
}

/// 相对路径以 `ctx.cwd` 解析，不受进程 cwd 影响。
#[test]
fn relative_paths_resolve_against_context_not_process_cwd() {
    let fixture = Fixture::new("cwd");
    fixture.write("nested/a.txt", "hello\n");
    let ctx = fixture.ctx();
    let result = execute(&ctx, "read", r#"{"path":"nested/a.txt"}"#);
    let (content, _, _, _, truncated, total) = expect_read(&result);
    assert!(content.contains("hello"));
    assert!(!truncated);
    assert_eq!(total, 1);

    // 进程 cwd 指向别处时结果不变：解析只用 ctx.cwd。
    let elsewhere = Fixture::new("elsewhere");
    let absolute = fixture.join("nested/a.txt");
    let result = execute(
        &ToolContext::new(elsewhere.path()),
        "read",
        &format!(r#"{{"path":"{}"}}"#, absolute.display()),
    );
    assert!(expect_read(&result).0.contains("hello"));
}

/// cwd 为空或不存在时返回 `PathUnresolvable`，绝不退回进程 cwd。
#[test]
fn unresolvable_cwd_is_reported() {
    let empty = ToolContext::new("");
    let result = execute(&empty, "read", r#"{"path":"a.txt"}"#);
    assert_eq!(expect_failure(&result), ToolErrorCode::PathUnresolvable);

    let missing = ToolContext::new("/definitely/not/here/lx-tools");
    let result = execute(&missing, "read", r#"{"path":"a.txt"}"#);
    assert_eq!(expect_failure(&result), ToolErrorCode::PathUnresolvable);
}

/// 符号链接目标可读写（允许链接指向任意位置）。
#[cfg(unix)]
#[test]
fn symlink_targets_are_readable_and_writable() {
    let fixture = Fixture::new("symlink");
    let target = fixture.write("real.txt", "one\n");
    let link = fixture.join("link.txt");
    std::os::unix::fs::symlink(&target, &link).expect("symlink");
    let ctx = fixture.ctx();
    let result = execute(&ctx, "read", r#"{"path":"link.txt"}"#);
    assert!(expect_read(&result).0.contains("one"));

    let result = execute(&ctx, "write", r#"{"path":"link.txt","content":"two\n"}"#);
    let (created, _, _) = expect_write(&result);
    assert!(
        !created,
        "writing through a symlink must not claim creation"
    );
    assert_eq!(fixture.read("real.txt"), "two\n");
}

/// `format_for_model` 覆盖四类成功结果与失败结果。
#[test]
fn format_for_model_covers_every_variant() {
    let fixture = Fixture::new("format");
    fixture.write("a.txt", "one\ntwo\n");
    let ctx = fixture.ctx();
    let read = execute(&ctx, "read", r#"{"path":"a.txt"}"#);
    let text = format_for_model(&read);
    assert!(text.contains("1: one"), "read text: {text}");
    assert!(text.contains("2: two"));

    let write = execute(&ctx, "write", r#"{"path":"b.txt","content":"x\n"}"#);
    let text = format_for_model(&write);
    assert!(text.contains("Created"), "write text: {text}");

    let search = execute(&ctx, "search", r#"{"pattern":"one","literal":true}"#);
    let text = format_for_model(&search);
    assert!(text.contains("a.txt:1: one"), "search text: {text}");

    let failure = execute(&ctx, "read", r#"{"path":"missing.txt"}"#);
    let text = format_for_model(&failure);
    assert!(text.contains("not_found"), "failure text: {text}");
    assert!(text.contains("no files were changed"));
}

/// 摘要只含路径/范围/计数，绝不包含正文。
#[test]
fn summary_never_contains_file_content() {
    let fixture = Fixture::new("summary");
    fixture.write("secret.txt", "TOP-SECRET-PAYLOAD\nline2\n");
    let ctx = fixture.ctx();
    let read = execute(&ctx, "read", r#"{"path":"secret.txt"}"#);
    let summary = to_summary(&read);
    let rendered = format!("{summary:?}");
    assert!(!rendered.contains("TOP-SECRET-PAYLOAD"), "{rendered}");
    assert_eq!(summary.range, Some((1, 2)));
    assert_eq!(
        summary.path.as_deref(),
        Some(fixture.join("secret.txt").to_string_lossy().as_ref())
    );

    let search = execute(
        &ctx,
        "search",
        r#"{"pattern":"TOP-SECRET","case_sensitive":true}"#,
    );
    let summary = to_summary(&search);
    assert_eq!(summary.matches, Some(1));
    assert!(!format!("{summary:?}").contains("PAYLOAD"));
}

/// 落库 JSON 是规范化结构；超限时截断并显式标记。
#[test]
fn result_json_is_normalized_and_flags_truncation() {
    let fixture = Fixture::new("result-json");
    let result = execute(
        &fixture.ctx(),
        "write",
        r#"{"path":"a.txt","content":"one\ntwo\n"}"#,
    );
    let json = to_result_json(&result).expect("json");
    let value: serde_json::Value = serde_json::from_str(&json).expect("valid json");
    assert_eq!(value["tool"], "write");
    assert_eq!(value["created"], true);
    assert_eq!(value["added"], 2);

    // 超限：一次读取超大文件，结果 JSON 必须被截断且带显式标志。
    let big = "x".repeat(4000);
    let mut content = String::new();
    for _ in 0..MAX_READ_LINES {
        content.push_str(&big);
        content.push('\n');
    }
    fixture.write("big.txt", &content);
    let result = execute(&fixture.ctx(), "read", r#"{"path":"big.txt"}"#);
    let json = to_result_json(&result).expect("json");
    assert!(
        json.len() <= MAX_TOOL_RESULT_BYTES,
        "json length {} exceeds limit",
        json.len()
    );
    let value: serde_json::Value = serde_json::from_str(&json).expect("valid json");
    assert_eq!(value["truncated"], true);
}

/// 参数体超限直接失败（不解析、无副作用）。
#[test]
fn oversized_arguments_are_rejected() {
    let fixture = Fixture::new("oversized-args");
    let payload = "y".repeat(crate::chat::events::MAX_TOOL_ARGUMENTS_BYTES + 1);
    let arguments = format!(r#"{{"path":"a.txt","content":"{payload}"}}"#);
    let result = execute(&fixture.ctx(), "write", &arguments);
    assert_eq!(expect_failure(&result), ToolErrorCode::TooLarge);
    assert!(!fixture.exists("a.txt"));
}

/// `curr_time` 返回实时或注入的精确 UTC 格式，拒绝任何未知字段。
#[test]
fn curr_time_returns_exact_utc_format_and_rejects_extra_fields() {
    let fixture = Fixture::new("curr-time");
    // 2026-10-29 08:15:30 UTC = 1_793_261_730
    let ctx = fixture.ctx().with_clock_secs(1_793_261_730);
    let result = execute(&ctx, "curr_time", "{}");
    assert!(!result.is_failure());
    assert_eq!(
        format_for_model(&result),
        "current_time: 2026-10-29 08:15:30 UTC\n"
    );
    let summary = to_summary(&result);
    assert_eq!(summary.path.as_deref(), Some("2026-10-29 08:15:30 UTC"));
    let json_text = to_result_json(&result).expect("json");
    let json_val: serde_json::Value = serde_json::from_str(&json_text).expect("parse");
    assert_eq!(json_val["tool"], "curr_time");
    assert_eq!(json_val["current_time"], "2026-10-29 08:15:30 UTC");

    // 实时时钟格式校验
    let live = execute(&fixture.ctx(), "curr_time", "{}");
    let live_text = format_for_model(&live);
    assert!(live_text.starts_with("current_time: 20"));
    assert!(live_text.ends_with(" UTC\n"));

    // 非空对象或非对象一律拒绝
    for bad in [r#"{"tz":"UTC"}"#, "[]", r#"{"format":"iso"}"#] {
        let failed = execute(&ctx, "curr_time", bad);
        assert_eq!(
            expect_failure(&failed),
            ToolErrorCode::InvalidArguments,
            "curr_time must reject {bad}"
        );
    }
}

/// 新控制参数出现在 schema 中，且数字/枚举与常量同源。
#[test]
fn subagent_schemas_declare_hierarchy_and_budget_fields() {
    use crate::chat::agent::MAX_FORK_TURNS;
    use crate::config::subagents::MIN_SUBAGENT_MAX_TOKENS;

    let spec = |name: &str| {
        available_tools()
            .iter()
            .find(|spec| spec.name == name)
            .map(|spec| spec.parameters())
            .unwrap_or_else(|| panic!("{name}"))
    };

    let spawn = spec("spawn_agent");
    assert_eq!(
        spawn["properties"]["task_name"]["maxLength"],
        crate::chat::agent::MAX_PATH_SEGMENT_LEN
    );
    assert_eq!(spawn["properties"]["fork_turns"]["maximum"], MAX_FORK_TURNS);
    assert_eq!(
        spawn["properties"]["max_tokens"]["minimum"],
        MIN_SUBAGENT_MAX_TOKENS
    );

    let send = spec("send_input");
    assert_eq!(send["properties"]["trigger_turn"]["type"], "boolean");
    assert_eq!(send["properties"]["trigger_turn"]["default"], true);

    let list = spec("list_agents");
    assert_eq!(
        list["properties"]["path_prefix"]["type"], "string",
        "list_agents must expose path_prefix"
    );

    for tool in ["send_input", "interrupt_agent", "close_agent"] {
        let tool_spec = spec(tool);
        let target = tool_spec["properties"]["target"]["description"]
            .as_str()
            .expect("target description");
        assert!(
            target.contains("agent_path"),
            "{tool} target description must mention agent_path: {target}"
        );
    }
    let wait_spec = spec("wait_agent");
    let wait_targets = wait_spec["properties"]["targets"]["description"]
        .as_str()
        .expect("wait targets description");
    assert!(wait_targets.contains("agent_path"));
}

/// 未达深度上限的 executor 工具面包含控制工具，达到上限后收紧。
#[test]
fn executor_coordinator_surface_contains_control_tools() {
    use crate::chat::agent::ToolSurface;

    let names: Vec<&str> = tools_for(ToolSurface::ExecutorCoordinator)
        .iter()
        .map(|spec| spec.name)
        .collect();
    assert_eq!(names.len(), 15);
    for expected in [
        "read",
        "search",
        "web_search",
        "edit",
        "write",
        "exec_command",
        "write_stdin",
        "spawn_agent",
        "send_input",
        "interrupt_agent",
        "resume_agent",
        "wait_agent",
        "close_agent",
        "list_agents",
        "cancel_agent_input",
    ] {
        assert!(names.contains(&expected), "missing {expected}");
    }
    for absent in ["ask", "update_plan", "curr_time"] {
        assert!(!names.contains(&absent), "unexpected {absent}");
    }
    let declarations = builtin_tools(ToolSurface::ExecutorCoordinator);
    assert_eq!(declarations.len(), 15);
}

/// 跨工具校验：schema 声明的属性与执行层白名单校验严格一致。
#[test]
fn schema_declarations_match_execution_layer_validation() {
    let fixture = Fixture::new("schema-exec-consistency");
    let ctx = fixture.ctx();
    for spec in available_tools() {
        let params = spec.parameters();
        let props = params["properties"].as_object().expect("properties object");
        // 往每个工具塞入一个未在 schema.properties 中声明的字段，执行层或 parse_control 必须拒绝
        let mut payload = serde_json::Map::new();
        payload.insert("__undeclared_field__".into(), serde_json::json!(1));
        let raw = serde_json::Value::Object(payload).to_string();
        if let Some(control) = crate::chat::agent::parse_control(spec.name, &raw) {
            let err = control.expect_err("control tool must reject undeclared field");
            assert_eq!(
                err.code,
                ToolErrorCode::InvalidArguments,
                "tool {} failed with wrong code",
                spec.name
            );
            // 同时确认 schema 声明过的每个字段名在 parse_control 中不会被判为 `unknown field`
            for declared_key in props.keys() {
                let mut single = serde_json::Map::new();
                single.insert(declared_key.clone(), serde_json::Value::Null);
                let single_raw = serde_json::Value::Object(single).to_string();
                if let Some(Err(failure)) =
                    crate::chat::agent::parse_control(spec.name, &single_raw)
                {
                    assert!(
                        !failure.message.contains("unknown field"),
                        "tool {} rejected its own declared property `{declared_key}` as unknown: {}",
                        spec.name,
                        failure.message
                    );
                }
            }
        } else if matches!(
            spec.name,
            "read" | "search" | "edit" | "write" | "curr_time"
        ) {
            let res = execute(&ctx, spec.name, &raw);
            assert_eq!(
                expect_failure(&res),
                ToolErrorCode::InvalidArguments,
                "tool {} must reject undeclared field",
                spec.name
            );
        }
    }
}
