//! 共用工具请求定义与参数缓冲测试（离线：不访问任何真实 API）。

use serde_json::{Value, json};

use super::*;
use crate::chat::events::{ModelItem, TOOL_ARGUMENTS_TOO_LARGE_CODE};

/// 四种 wire 形状都必须与 `available_tools()` 一一对应，且 schema 逐字一致。
#[test]
fn chat_tools_wrap_function_and_match_schema() {
    let tools = openai_chat_tools();
    let tools = tools.as_array().expect("array");
    assert_eq!(tools.len(), available_tools().len());
    for (wire, spec) in tools.iter().zip(available_tools()) {
        assert_eq!(wire["type"], "function");
        assert_eq!(wire["function"]["name"], spec.name);
        assert_eq!(wire["function"]["description"], spec.description);
        assert_eq!(wire["function"]["parameters"], spec.parameters());
    }
}

#[test]
fn responses_tools_are_flat() {
    for (wire, spec) in openai_responses_tools()
        .as_array()
        .expect("array")
        .iter()
        .zip(available_tools())
    {
        assert_eq!(wire["type"], "function");
        // Responses 是扁平形状：name/description 在顶层，没有 function 包装。
        assert!(wire.get("function").is_none());
        assert_eq!(wire["name"], spec.name);
        assert_eq!(wire["parameters"], spec.parameters());
    }
}

#[test]
fn anthropic_tools_use_input_schema() {
    for (wire, spec) in anthropic_tools()
        .as_array()
        .expect("array")
        .iter()
        .zip(available_tools())
    {
        assert_eq!(wire["name"], spec.name);
        assert_eq!(wire["input_schema"], spec.parameters());
        assert!(wire.get("parameters").is_none());
    }
}

#[test]
fn google_tools_use_function_declarations() {
    let tools = google_tools();
    let declarations = tools[0]["functionDeclarations"]
        .as_array()
        .expect("declarations");
    assert_eq!(declarations.len(), available_tools().len());
    for (wire, spec) in declarations.iter().zip(available_tools()) {
        assert_eq!(wire["name"], spec.name);
        assert_eq!(wire["parameters"], spec.parameters());
    }
}

/// 能力类 4xx/501 才归类为“不支持工具”；认证/限流/超时保持原错误。
#[test]
fn only_capability_statuses_count_as_tools_unsupported() {
    for status in [400_u16, 404, 405, 409, 415, 422, 501] {
        assert!(is_tools_unsupported_status(status), "status {status}");
    }
    for status in [401_u16, 403, 408, 429, 500, 502, 503] {
        assert!(!is_tools_unsupported_status(status), "status {status}");
    }
}

/// 参数跨帧累计、结束补 `{}`、超限立即失败。
#[test]
fn buffer_accumulates_cross_frame_arguments() {
    let mut buffer = ToolCallBuffer::default();
    buffer.identify(0, "call_a", "read").expect("identify");
    assert_eq!(
        buffer.take_start(0),
        Some(("call_a".to_string(), "read".to_string()))
    );
    assert_eq!(buffer.take_start(0), None, "Started 只发一次");
    assert!(buffer.push_delta(0, "{\"path\":").is_ok());
    assert_eq!(buffer.take_emittable(0), "{\"path\":");
    assert!(buffer.push_delta(0, "\"a.rs\"}").is_ok());
    assert_eq!(buffer.take_emittable(0), "\"a.rs\"}");
    assert_eq!(buffer.take_emittable(0), "");
    let (call_id, arguments) = buffer.end(0).expect("end");
    assert_eq!(call_id, "call_a");
    assert_eq!(arguments, "{\"path\":\"a.rs\"}");
    assert!(!buffer.has_open());
    assert!(buffer.has_calls());
}

#[test]
fn buffer_empty_arguments_become_empty_object() {
    let mut buffer = ToolCallBuffer::default();
    buffer.identify(1, "call_b", "search").expect("identify");
    let (_, arguments) = buffer.end(1).expect("end");
    assert_eq!(arguments, "{}");
}

#[test]
fn buffer_rejects_duplicate_call_id_on_another_ordinal() {
    let mut buffer = ToolCallBuffer::default();
    buffer.identify(0, "call_a", "read").expect("identify");
    let error = buffer
        .identify(1, "call_a", "read")
        .expect_err("duplicate id rejected");
    assert!(error.contains(TOOL_PROTOCOL_ERROR_CODE), "{error}");
}

/// 缺 call ID 的未闭合调用不能在完成事件里被“补齐”一个假 ID。
#[test]
fn buffer_rejects_unclosed_call_without_id() {
    let mut buffer = ToolCallBuffer::default();
    buffer.bind_name(0, "read").expect("name bound");
    buffer.push_delta(0, "{}").expect("delta");
    let error = buffer.end_all_open().expect_err("missing id rejected");
    assert!(error.contains("no call id"), "{error}");
}

#[test]
fn buffer_rejects_changed_name_for_same_call() {
    let mut buffer = ToolCallBuffer::default();
    buffer.identify(0, "call_a", "read").expect("identify");
    let error = buffer
        .identify(0, "call_a", "write")
        .expect_err("name change rejected");
    assert!(error.contains("changed name"), "{error}");
}

#[test]
fn buffer_fails_fast_when_arguments_exceed_limit() {
    let mut buffer = ToolCallBuffer::default();
    buffer.identify(0, "call_a", "write").expect("identify");
    let chunk = "x".repeat(MAX_TOOL_ARGUMENTS_BYTES);
    let error = buffer
        .push_delta(0, &chunk)
        .and_then(|()| buffer.push_delta(0, "x"))
        .expect_err("limit enforced");
    assert_eq!(error, TOOL_ARGUMENTS_TOO_LARGE_CODE);
}

/// Gemini 的完整对象调用：同 ordinal 重复快照只登记一次（去重）。
#[test]
fn buffer_start_complete_dedupes_repeated_snapshot() {
    let mut buffer = ToolCallBuffer::default();
    let first = buffer
        .start_complete(0, "gemini-0-0-read", "read", "{\"path\":\"a\"}")
        .expect("start")
        .expect("first call");
    assert_eq!(first.0, "gemini-0-0-read");
    assert!(
        buffer
            .start_complete(0, "gemini-0-0-read", "read", "{\"path\":\"a\"}")
            .expect("dedupe")
            .is_none()
    );
}

/// `end_all_open` 按 ordinal 升序收尾（供应商响应内顺序，不是哈希顺序）。
#[test]
fn buffer_end_all_open_is_ordered_by_ordinal() {
    let mut buffer = ToolCallBuffer::default();
    buffer.identify(2, "call_c", "write").expect("identify");
    buffer.identify(0, "call_a", "read").expect("identify");
    buffer.identify(1, "call_b", "search").expect("identify");
    let ended = buffer.end_all_open().expect("end all");
    let ids: Vec<&str> = ended.iter().map(|(id, _)| id.as_str()).collect();
    assert_eq!(ids, vec!["call_a", "call_b", "call_c"]);
}

/// Responses 的 `arguments.done`：以权威完整串为准，且不再重发已发片段。
#[test]
fn buffer_replace_arguments_wins_over_deltas() {
    let mut buffer = ToolCallBuffer::default();
    buffer.identify(0, "call_a", "read").expect("identify");
    buffer.take_start(0);
    buffer.push_delta(0, "{\"path\"").expect("delta");
    assert_eq!(buffer.take_emittable(0), "{\"path\"");
    buffer
        .replace_arguments(0, "{\"path\":\"a.rs\"}")
        .expect("replace");
    assert_eq!(buffer.take_emittable(0), "", "done 后不再补发");
    let (_, arguments) = buffer.end(0).expect("end");
    assert_eq!(arguments, "{\"path\":\"a.rs\"}");
}

/// 历史归并：纯文本 items 与 turns 顺序一致，call/result 成对保留。
#[test]
fn history_blocks_preserve_order_and_pairs() {
    let items = vec![
        ModelItem::UserText("读一下".into()),
        ModelItem::AssistantToolCall {
            call_id: "c1".into(),
            name: "read".into(),
            input: json!({"path": "a.rs"}),
        },
        ModelItem::ToolResult {
            call_id: "c1".into(),
            content: "1: x".into(),
            is_error: false,
        },
        ModelItem::AssistantText("读完了".into()),
    ];
    let blocks = history_blocks(&items).expect("blocks");
    assert_eq!(blocks.len(), 4);
    assert_eq!(
        blocks[0],
        HistoryBlock::Text {
            role: Role::User,
            text: "读一下".into()
        }
    );
    assert!(matches!(&blocks[1], HistoryBlock::ToolCall { call_id, .. } if call_id == "c1"));
    // 工具结果的 name 由配对调用解析（Gemini 的 functionResponse 需要它）。
    assert!(
        matches!(&blocks[2], HistoryBlock::ToolResult { call_id, name, .. } if call_id == "c1" && name == "read")
    );
}

/// 孤儿工具结果（没有对应调用）不得进入请求。
#[test]
fn history_blocks_reject_orphan_result() {
    let items = vec![ModelItem::ToolResult {
        call_id: "missing".into(),
        content: "x".into(),
        is_error: false,
    }];
    let error = history_blocks(&items).expect_err("orphan rejected");
    assert!(error.contains("without a preceding call"), "{error}");
}

/// 非对象参数在编码前即失败（四工具参数都是 JSON object）。
#[test]
fn history_blocks_reject_non_object_arguments() {
    let items = vec![ModelItem::AssistantToolCall {
        call_id: "c1".into(),
        name: "read".into(),
        input: json!("not-an-object"),
    }];
    let error = history_blocks(&items).expect_err("non-object rejected");
    assert!(error.contains("not a JSON object"), "{error}");
}

/// 四种 wire 形状的名称集合必须等于四工具（防止漏发或错名）。
#[test]
fn all_wire_shapes_expose_the_tool_names() {
    let expected: Vec<&str> = available_tools().iter().map(|spec| spec.name).collect();
    assert_eq!(expected, crate::chat::tools::TOOL_NAMES.to_vec());

    let chat: Vec<String> = openai_chat_tools()
        .as_array()
        .expect("array")
        .iter()
        .map(|tool| {
            tool["function"]["name"]
                .as_str()
                .unwrap_or_default()
                .to_string()
        })
        .collect();
    let responses: Vec<String> = openai_responses_tools()
        .as_array()
        .expect("array")
        .iter()
        .map(|tool| tool["name"].as_str().unwrap_or_default().to_string())
        .collect();
    let anthropic: Vec<String> = anthropic_tools()
        .as_array()
        .expect("array")
        .iter()
        .map(|tool| tool["name"].as_str().unwrap_or_default().to_string())
        .collect();
    let google: Vec<String> = google_tools()[0]["functionDeclarations"]
        .as_array()
        .expect("declarations")
        .iter()
        .map(|tool| tool["name"].as_str().unwrap_or_default().to_string())
        .collect();
    let expected: Vec<String> = expected.into_iter().map(str::to_string).collect();
    assert_eq!(chat, expected);
    assert_eq!(responses, expected);
    assert_eq!(anthropic, expected);
    assert_eq!(google, expected);
}

/// schema 里必须保留 `additionalProperties: false`（模型可见契约的一部分）。
#[test]
fn emitted_schemas_keep_additional_properties_false() {
    for spec in available_tools() {
        let schema: Value = spec.parameters();
        assert_eq!(
            schema["additionalProperties"],
            json!(false),
            "工具 {} 的 schema 必须禁止未定义字段",
            spec.name
        );
    }
}
