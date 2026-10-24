//! Google Gemini 适配测试：请求形状（contents/tools/systemInstruction/Key 位置）、
//! thought part、functionCall 去重与合成 call ID、usage。

use serde_json::{Value, json};

use super::*;
use crate::chat::events::{
    ChatDelta, ChatRequest, ChatTurn, ModelItem, ModelStepStop, PartKind, StopReason,
    TOOL_ARGUMENTS_INVALID_CODE,
};
use crate::chat::transport::tools::TOOL_PROTOCOL_ERROR_CODE;

fn request() -> ChatRequest {
    ChatRequest {
        turns: vec![
            ChatTurn {
                role: "user".into(),
                text: "问".into(),
                reasoning_metadata: None,
            },
            ChatTurn {
                role: "assistant".into(),
                text: "答".into(),
                reasoning_metadata: None,
            },
            ChatTurn {
                role: "user".into(),
                text: "续问".into(),
                reasoning_metadata: None,
            },
        ],
        items: Vec::new(),
        model: crate::db::chat::ModelSnapshot {
            provider: "gemini".into(),
            model: "gemini-flash".into(),
            variant: None,
        },
        variant_params: None,
        purpose: crate::chat::events::RequestPurpose::Chat,
        system_prompt: None,
    }
}

/// 带工具调用的结构化历史（成功 + 失败各一次）。
fn tool_request() -> ChatRequest {
    let mut req = request();
    req.turns = Vec::new();
    req.items = vec![
        ModelItem::UserText("读 a.rs".into()),
        ModelItem::AssistantToolCall {
            call_id: "gemini-7-0-0-read".into(),
            name: "read".into(),
            input: json!({"path": "a.rs"}),
        },
        ModelItem::ToolResult {
            call_id: "gemini-7-0-0-read".into(),
            content: "1: fn main() {}".into(),
            is_error: false,
        },
        ModelItem::AssistantToolCall {
            call_id: "gemini-7-0-1-write".into(),
            name: "write".into(),
            input: json!({"path": "b.rs", "content": ""}),
        },
        ModelItem::ToolResult {
            call_id: "gemini-7-0-1-write".into(),
            content: "permission_denied".into(),
            is_error: true,
        },
    ];
    req
}

fn body_of(req: &ChatRequest) -> Value {
    let built = super::build("https://example.invalid", Some("g-key"), req).expect("build ok");
    serde_json::from_str(&built.body).expect("json")
}

fn decode_frames(decoder: &mut dyn super::super::StreamDecoder, frames: &[&str]) -> Vec<ChatDelta> {
    let mut out = Vec::new();
    for frame in frames {
        out.extend(decoder.decode_frame(frame).expect("frame decodes"));
    }
    out.extend(decoder.finish());
    out
}

fn decode_all(decoder: &mut dyn super::super::StreamDecoder, frames: &[&str]) -> Vec<ChatDelta> {
    let mut out = decode_frames(decoder, frames);
    if !out
        .iter()
        .any(|delta| matches!(delta, ChatDelta::Finished { .. }))
    {
        out.push(ChatDelta::Finished {
            reason: if decoder.has_completion() {
                StopReason::Complete
            } else {
                StopReason::Failed {
                    code: "stream ended without completion".into(),
                }
            },
        });
    }
    out
}

fn trace(deltas: &[ChatDelta]) -> Vec<String> {
    deltas
        .iter()
        .filter_map(|delta| match delta {
            ChatDelta::ToolInputStarted {
                call_id,
                name,
                ordinal,
            } => Some(format!("started:{call_id}:{name}:{ordinal}")),
            ChatDelta::ToolInputDelta { call_id, chunk } => {
                Some(format!("delta:{call_id}:{chunk}"))
            }
            ChatDelta::ToolInputEnded {
                call_id,
                arguments_json,
            } => Some(format!("ended:{call_id}:{arguments_json}")),
            ChatDelta::ResponseFinished { stop } => Some(format!("response:{stop:?}")),
            ChatDelta::Finished { reason } => Some(format!("finished:{}", reason.as_str())),
            _ => None,
        })
        .collect()
}

// ---------------------------------------------------------------- 请求编码

#[test]
fn url_places_key_in_query_and_body_uses_contents() {
    let built =
        super::build("https://example.invalid", Some("g-key"), &request()).expect("build ok");
    assert!(
        built
            .url
            .ends_with("/models/gemini-flash:streamGenerateContent?alt=sse&key=g-key"),
        "Key 只按该协议放置在查询参数：{}",
        built.url
    );
    // 模型名重复 models/ 前缀去重。
    let built = super::build("https://example.invalid", Some("g-key"), &{
        let mut req = request();
        req.model.model = "models/gemini-flash".into();
        req
    })
    .expect("build ok");
    assert!(built.url.contains("/models/gemini-flash:"));
    assert!(!built.url.contains("models/models/"));
    let body: Value = serde_json::from_str(&built.body).expect("json");
    let contents = body
        .get("contents")
        .and_then(Value::as_array)
        .expect("contents");
    assert_eq!(contents.len(), 3);
    // assistant 历史 role 映射为 model。
    assert_eq!(contents[0]["role"], "user");
    assert_eq!(contents[1]["role"], "model");
    assert_eq!(contents[2]["role"], "user");
    // Key 不进 body。
    assert!(!built.body.contains("g-key"));
}

/// 系统指令走 `systemInstruction.parts[0].text`。
#[test]
fn system_instruction_carries_lx_prompt() {
    let body = body_of(&request());
    assert_eq!(
        body["systemInstruction"]["parts"][0]["text"],
        crate::chat::prompt::system_prompt()
    );
}

/// `tools[0].functionDeclarations` 与 `available_tools()` 一一对应。
#[test]
fn tools_use_function_declarations() {
    let body = body_of(&request());
    let declarations = body["tools"][0]["functionDeclarations"]
        .as_array()
        .expect("declarations");
    let specs = crate::chat::tools::available_tools();
    assert_eq!(declarations.len(), specs.len());
    for (wire, spec) in declarations.iter().zip(specs) {
        assert_eq!(wire["name"], spec.name);
        assert_eq!(wire["description"], spec.description);
        assert_eq!(wire["parameters"], spec.parameters());
    }
}

/// 结构化历史：model functionCall part 与 user functionResponse part 相邻成对。
#[test]
fn tool_history_encodes_function_call_and_response_parts() {
    let body = body_of(&tool_request());
    let contents = body["contents"].as_array().expect("contents");
    assert_eq!(contents.len(), 5, "user → model → user → model → user");
    assert_eq!(contents[0]["parts"][0]["text"], "读 a.rs");
    // model functionCall：args 是对象，不写入合成 call ID。
    let call = &contents[1]["parts"][0]["functionCall"];
    assert_eq!(contents[1]["role"], "model");
    assert_eq!(call["name"], "read");
    assert_eq!(call["args"], json!({"path": "a.rs"}));
    assert!(call.get("id").is_none(), "不得把合成 ID 写进供应商字段");
    // user functionResponse：name 由配对调用解析，response 包装结构化结果。
    assert_eq!(contents[2]["role"], "user");
    let response = &contents[2]["parts"][0]["functionResponse"];
    assert_eq!(response["name"], "read");
    assert_eq!(response["response"]["result"], "1: fn main() {}");
    // 失败结果用 error 键标注，不与成功结果混淆。
    let failed = &contents[4]["parts"][0]["functionResponse"];
    assert_eq!(failed["name"], "write");
    assert_eq!(failed["response"]["error"], "permission_denied");
}

/// **不污染请求**：`functionResponse` 只含 `name` / `response`，
/// 合成 call ID（含响应序号）不得出现在发送给供应商的任何字段里。
#[test]
fn synthesized_call_ids_never_reach_the_wire() {
    let mut req = tool_request();
    // 用真实合成形状（含响应序号）的历史，确保序号也不会泄漏到请求体。
    req.items[1] = ModelItem::AssistantToolCall {
        call_id: "gemini-42-0-0-read".into(),
        name: "read".into(),
        input: json!({"path": "a.rs"}),
    };
    req.items[2] = ModelItem::ToolResult {
        call_id: "gemini-42-0-0-read".into(),
        content: "1: fn main() {}".into(),
        is_error: false,
    };
    let body = body_of(&req);
    let contents = body["contents"].as_array().expect("contents");
    // functionCall：只有 name / args 两个键。
    let call = contents[1]["parts"][0]["functionCall"]
        .as_object()
        .expect("functionCall object");
    let mut call_keys: Vec<&str> = call.keys().map(String::as_str).collect();
    call_keys.sort_unstable();
    assert_eq!(call_keys, vec!["args", "name"], "functionCall 不得带 id");
    // functionResponse：只有 name / response 两个键。
    for index in [2_usize, 4] {
        let response = contents[index]["parts"][0]["functionResponse"]
            .as_object()
            .expect("functionResponse object");
        let mut keys: Vec<&str> = response.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            vec!["name", "response"],
            "functionResponse 只能带 name/response：{response:?}"
        );
    }
    // 整个请求体里不得出现合成 ID 的任何痕迹。
    assert!(
        !body.to_string().contains("gemini-42"),
        "合成 call ID 泄漏进请求体：{body}"
    );
    assert!(!body.to_string().contains("call_id"));
}

#[test]
fn thinking_budget_maps_to_generation_config() {
    let mut req = request();
    req.variant_params = Some(json!({"thinkingBudget": 1024}));
    let body = body_of(&req);
    assert_eq!(
        body["generationConfig"]["thinkingConfig"]["thinkingBudget"],
        1024
    );
}

// ---------------------------------------------------------------- 解码

/// 测试用响应序号：ID 内容随序号变化，断言固定序号下的完整形状。
const SEQ: u64 = 7;

/// 构造一个使用固定响应序号的解码器。
fn decoder() -> Decoder {
    Decoder::new(SEQ)
}

#[test]
fn thought_part_is_reasoning_and_order_preserved() {
    let mut decoder = decoder();
    let deltas = decode_all(
        &mut decoder,
        &[
            r#"{"candidates":[{"content":{"parts":[{"text":"思考内容","thought":true},{"text":"正文"}],"role":"model"},"finishReason":"STOP"}],"usageMetadata":{"promptTokenCount":12,"candidatesTokenCount":3}}"#,
        ],
    );
    let parts: Vec<(PartKind, String)> = deltas
        .iter()
        .filter_map(|delta| match delta {
            ChatDelta::Part(part) => Some((part.kind, part.text.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(
        parts,
        vec![
            (PartKind::Reasoning, "思考内容".into()),
            (PartKind::Text, "正文".into()),
        ]
    );
    assert!(deltas.iter().any(|delta| matches!(
        delta,
        ChatDelta::Usage(usage) if usage.input == Some(12) && usage.output == Some(3)
    )));
    // 无 functionCall：ResponseFinished{Final} + Finished。
    assert!(deltas.contains(&ChatDelta::ResponseFinished {
        stop: ModelStepStop::Final
    }));
    assert_eq!(
        deltas.last(),
        Some(&ChatDelta::Finished {
            reason: StopReason::Complete
        })
    );
}

/// 单 functionCall：Started + Ended（args 已是完整对象）+ ResponseFinished{ToolCalls}。
#[test]
fn single_function_call_emits_started_and_ended() {
    let mut decoder = decoder();
    let deltas = decode_all(
        &mut decoder,
        &[
            r#"{"candidates":[{"content":{"parts":[{"functionCall":{"name":"read","args":{"path":"a.rs"}}}],"role":"model"},"finishReason":"STOP","index":0}]}"#,
        ],
    );
    assert_eq!(
        trace(&deltas),
        vec![
            // 合成 ID 用 (response seq, candidate, part, name)，不含时间戳。
            "started:gemini-7-0-0-read:read:0".to_string(),
            "ended:gemini-7-0-0-read:{\"path\":\"a.rs\"}".to_string(),
            "response:ToolCalls".to_string(),
            "finished:complete".to_string(),
        ]
    );
}

/// 累计快照重复给同一 functionCall：只执行一次。
#[test]
fn repeated_cumulative_snapshot_is_deduped() {
    let mut decoder = decoder();
    let deltas = decode_all(
        &mut decoder,
        &[
            // 第一帧只有 thought + functionCall。
            r#"{"candidates":[{"content":{"parts":[{"text":"想","thought":true},{"functionCall":{"name":"read","args":{"path":"a.rs"}}}],"role":"model"},"index":0}]}"#,
            // 第二帧是累计快照：thought 全文 + 同一个 functionCall（重复）。
            r#"{"candidates":[{"content":{"parts":[{"text":"想要读文件","thought":true},{"functionCall":{"name":"read","args":{"path":"a.rs"}}}],"role":"model"},"finishReason":"STOP","index":0}]}"#,
        ],
    );
    let lines = trace(&deltas);
    let started: Vec<&String> = lines
        .iter()
        .filter(|line| line.starts_with("started:"))
        .collect();
    assert_eq!(started.len(), 1, "同一 functionCall 只执行一次：{deltas:?}");
    assert_eq!(
        lines
            .iter()
            .filter(|line| line.starts_with("ended:"))
            .count(),
        1
    );
}

/// 同一帧两个 functionCall：按 part index 顺序，ID 各自稳定。
#[test]
fn two_function_calls_in_one_frame_keep_part_order() {
    let mut decoder = decoder();
    let deltas = decode_all(
        &mut decoder,
        &[
            r#"{"candidates":[{"content":{"parts":[{"functionCall":{"name":"read","args":{"path":"a"}}},{"functionCall":{"name":"write","args":{"path":"b","content":""}}}],"role":"model"},"finishReason":"STOP","index":0}]}"#,
        ],
    );
    assert_eq!(
        trace(&deltas),
        vec![
            "started:gemini-7-0-0-read:read:0".to_string(),
            "ended:gemini-7-0-0-read:{\"path\":\"a\"}".to_string(),
            "started:gemini-7-0-1-write:write:1".to_string(),
            "ended:gemini-7-0-1-write:{\"content\":\"\",\"path\":\"b\"}".to_string(),
            "response:ToolCalls".to_string(),
            "finished:complete".to_string(),
        ]
    );
}

/// reasoning → tool → reasoning → text 交错：thought part 与 functionCall 顺序保持。
#[test]
fn reasoning_tool_reasoning_text_interleaving() {
    let mut decoder = decoder();
    let deltas = decode_all(
        &mut decoder,
        &[
            r#"{"candidates":[{"content":{"parts":[{"text":"先想","thought":true}],"role":"model"},"index":0}]}"#,
            r#"{"candidates":[{"content":{"parts":[{"functionCall":{"name":"read","args":{"path":"a"}}}],"role":"model"},"index":0}]}"#,
            r#"{"candidates":[{"content":{"parts":[{"text":"再想","thought":true},{"text":"正文"}],"role":"model"},"finishReason":"STOP","index":0}]}"#,
        ],
    );
    let trace: Vec<String> = deltas
        .iter()
        .filter_map(|delta| match delta {
            ChatDelta::Part(part) => Some(format!("part:{:?}:{}", part.kind, part.text)),
            ChatDelta::ToolInputStarted { call_id, .. } => Some(format!("started:{call_id}")),
            ChatDelta::ToolInputEnded { call_id, .. } => Some(format!("ended:{call_id}")),
            ChatDelta::ResponseFinished { stop } => Some(format!("response:{stop:?}")),
            ChatDelta::Finished { reason } => Some(format!("finished:{}", reason.as_str())),
            _ => None,
        })
        .collect();
    assert_eq!(
        trace,
        vec![
            "part:Reasoning:先想",
            "started:gemini-7-0-0-read",
            "ended:gemini-7-0-0-read",
            "part:Reasoning:再想",
            "part:Text:正文",
            "response:ToolCalls",
            "finished:complete",
        ]
    );
}

/// 缺 args 的 functionCall：按空对象处理（协议允许无参调用）。
#[test]
fn function_call_without_args_uses_empty_object() {
    let mut decoder = decoder();
    let deltas = decode_all(
        &mut decoder,
        &[
            r#"{"candidates":[{"content":{"parts":[{"functionCall":{"name":"search"}}],"role":"model"},"finishReason":"STOP","index":0}]}"#,
        ],
    );
    assert_eq!(
        trace(&deltas),
        vec![
            "started:gemini-7-0-0-search:search:0".to_string(),
            "ended:gemini-7-0-0-search:{}".to_string(),
            "response:ToolCalls".to_string(),
            "finished:complete".to_string(),
        ]
    );
}

/// 缺 name 的 functionCall：协议错误，不执行。
#[test]
fn function_call_without_name_is_protocol_error() {
    let mut decoder = decoder();
    let error = decoder
        .decode_frame(
            r#"{"candidates":[{"content":{"parts":[{"functionCall":{"args":{"path":"a"}}}],"role":"model"},"index":0}]}"#,
        )
        .expect_err("missing name rejected");
    assert!(error.contains(TOOL_PROTOCOL_ERROR_CODE), "{error}");
}

/// 回归：纯文本 part 带 `"functionCall": null` 占位时属于普通正文，
/// 不能被当成“缺 name 的调用”误判成协议错误。
#[test]
fn null_function_call_placeholder_in_text_part_is_not_an_error() {
    let mut decoder = decoder();
    let deltas = decoder
        .decode_frame(
            r#"{"candidates":[{"content":{"parts":[{"text":"我是 lx。","functionCall":null}],"role":"model"},"finishReason":"STOP","index":0}]}"#,
        )
        .expect("null placeholder is not a call");
    let text: String = deltas
        .iter()
        .filter_map(|delta| match delta {
            ChatDelta::Part(part) if part.kind == PartKind::Text => Some(part.text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(text, "我是 lx。");
    assert!(
        !deltas
            .iter()
            .any(|delta| matches!(delta, ChatDelta::ToolInputStarted { .. })),
        "null 占位不得造出工具调用"
    );
}

/// 非对象类型的 args：序列化后校验失败（参数必须可被工具层解析）。
#[test]
fn function_call_with_non_object_args_is_invalid() {
    let mut decoder = decoder();
    let error = decoder
        .decode_frame(
            r#"{"candidates":[{"content":{"parts":[{"functionCall":{"name":"read","args":"not-an-object"}}],"role":"model"},"index":0}]}"#,
        )
        .expect_err("non-object args rejected");
    assert!(error.contains(TOOL_ARGUMENTS_INVALID_CODE), "{error}");
}

/// 未知 finishReason（如 MAX_TOKENS 之外的值）不产生终态，EOF 由 client 判失败。
#[test]
fn unknown_finish_reason_waits_for_client() {
    let mut decoder = decoder();
    let deltas = decode_frames(
        &mut decoder,
        &[
            r#"{"candidates":[{"content":{"parts":[{"text":"x"}],"role":"model"},"finishReason":"OTHER"}]}"#,
        ],
    );
    assert!(!decoder.has_completion());
    assert!(
        !deltas
            .iter()
            .any(|delta| matches!(delta, ChatDelta::Finished { .. }))
    );
}

/// `MAX_TOKENS` 也视为本次响应结束（内容已截断但响应本身完成）。
#[test]
fn max_tokens_finish_reason_completes_response() {
    let mut decoder = decoder();
    let deltas = decode_all(
        &mut decoder,
        &[
            r#"{"candidates":[{"content":{"parts":[{"text":"x"}],"role":"model"},"finishReason":"MAX_TOKENS"}]}"#,
        ],
    );
    assert!(deltas.contains(&ChatDelta::ResponseFinished {
        stop: ModelStepStop::Final
    }));
    assert!(deltas.contains(&ChatDelta::Finished {
        reason: StopReason::Complete
    }));
}

#[test]
fn error_response_is_rejected() {
    let mut decoder = decoder();
    let error = decoder
        .decode_frame(r#"{"error":{"message":"quota exceeded"}}"#)
        .expect_err("error rejected");
    assert!(error.contains("quota exceeded"));
}

#[test]
fn eof_without_stop_finish_reason_is_protocol_error() {
    let mut decoder = decoder();
    let deltas = decode_all(
        &mut decoder,
        &[r#"{"candidates":[{"content":{"parts":[{"text":"x"}],"role":"model"}}]}"#],
    );
    assert!(matches!(
        deltas.last(),
        Some(ChatDelta::Finished {
            reason: StopReason::Failed { .. }
        })
    ));
}

#[test]
fn empty_base_url_uses_official_endpoint() {
    let built = super::build("", Some("g-key"), &request()).expect("build ok");
    assert!(
        built
            .url
            .starts_with("https://generativelanguage.googleapis.com/v1beta/")
    );
}

/// 用量只报**本响应**的实际值：两次采样各自解码，由上层累加。
#[test]
fn usage_is_reported_per_response_only() {
    let mut first = Decoder::default();
    let first_deltas = decode_all(
        &mut first,
        &[
            r#"{"candidates":[{"content":{"parts":[{"text":"a"}],"role":"model"},"finishReason":"STOP"}],"usageMetadata":{"promptTokenCount":10,"candidatesTokenCount":2}}"#,
        ],
    );
    let mut second = Decoder::default();
    let second_deltas = decode_all(
        &mut second,
        &[
            r#"{"candidates":[{"content":{"parts":[{"text":"b"}],"role":"model"},"finishReason":"STOP"}],"usageMetadata":{"promptTokenCount":40,"candidatesTokenCount":5}}"#,
        ],
    );
    let usages = |deltas: &[ChatDelta]| {
        deltas
            .iter()
            .filter_map(|delta| match delta {
                ChatDelta::Usage(usage) => Some((usage.input, usage.output)),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(usages(&first_deltas), vec![(Some(10), Some(2))]);
    assert_eq!(usages(&second_deltas), vec![(Some(40), Some(5))]);
}

// ------------------------------------------------- 跨响应 call ID 唯一性（G8 回归）

/// 一次响应只含一个 `read` 调用的真实帧（G8 缺陷的最小复现素材）。
const ONE_READ_FRAME: &str = r#"{"candidates":[{"content":{"parts":[{"functionCall":{"name":"read","args":{"path":"a.rs"}}}],"role":"model"},"finishReason":"STOP"}]}"#;

/// 提取一帧解码出的全部 `ToolInputStarted.call_id`。
fn started_ids(decoder: &mut dyn super::super::StreamDecoder, frame: &str) -> Vec<String> {
    decoder
        .decode_frame(frame)
        .expect("frame decodes")
        .into_iter()
        .filter_map(|delta| match delta {
            ChatDelta::ToolInputStarted { call_id, .. } => Some(call_id),
            _ => None,
        })
        .collect()
}

/// **G8 回归**：同一用户轮次的第 2 次及之后的模型采样，同一个 `read` 调用
/// 必须拿到**不同**的 call ID。
///
/// 缺陷根因：解码器实例按“一次模型响应”创建，旧 ID 只用
/// `(candidate, part, name)`，于是每次采样都合成出同一个
/// `gemini-0-0-read`，撞上 DB 的 `UNIQUE(session_id, call_id)`，
/// 多步工具链（read → edit → read）在第 2 次同名调用时整轮失败。
#[test]
fn call_ids_are_unique_across_model_responses() {
    let mut ids = Vec::new();
    for _ in 0..3 {
        // 每次采样都由 client 新建一个解码器（序号来自进程内计数器）。
        let seq = super::super::next_response_seq();
        let mut decoder = super::super::decoder_for("google", seq);
        let started = started_ids(decoder.as_mut(), ONE_READ_FRAME);
        assert_eq!(started.len(), 1, "每帧一个 functionCall");
        ids.extend(started);
    }
    assert_eq!(ids.len(), 3);
    for (index, id) in ids.iter().enumerate() {
        assert!(!id.is_empty(), "合成 ID 不得为空");
        assert_eq!(id, id.trim(), "合成 ID 不得含空白：{id:?}");
        // 不含时间戳：只由序号与位置/工具名组成。
        assert_eq!(
            id.split('-').count(),
            5,
            "ID 形状应为 gemini-{{seq}}-{{candidate}}-{{part}}-{{name}}：{id}"
        );
        for other in ids.iter().skip(index + 1) {
            assert_ne!(id, other, "跨响应同名调用必须拿到不同 call ID");
        }
    }
}

/// 显式序号也能保证跨响应唯一，且同一序号下结果**确定**（无随机、无时钟）。
#[test]
fn explicit_response_seq_is_deterministic_and_distinct() {
    let first = started_ids(&mut Decoder::new(1), ONE_READ_FRAME);
    let first_again = started_ids(&mut Decoder::new(1), ONE_READ_FRAME);
    let second = started_ids(&mut Decoder::new(2), ONE_READ_FRAME);
    assert_eq!(first, first_again, "同一序号必须得到同一 ID");
    assert_ne!(first, second, "不同序号必须得到不同 ID");
    assert_eq!(first, vec!["gemini-1-0-0-read".to_string()]);
    assert_eq!(second, vec!["gemini-2-0-0-read".to_string()]);
}

/// `next_response_seq` 严格单调：同一进程内连续两次响应不会拿到同一序号。
#[test]
fn response_seq_is_strictly_increasing() {
    let first = super::super::next_response_seq();
    let second = super::super::next_response_seq();
    let third = super::super::next_response_seq();
    assert!(first < second, "{first} < {second}");
    assert!(second < third, "{second} < {third}");
}

/// **同响应去重不变**：同一 decoder 连续喂入同一累计快照两帧，
/// 仍只产生一次 Started/Ended，且 ID 与序号绑定（不会被第二帧改写）。
#[test]
fn cumulative_snapshot_still_dedupes_within_one_response() {
    let mut decoder = Decoder::new(SEQ);
    let deltas = decode_all(
        &mut decoder,
        &[
            ONE_READ_FRAME,
            // 累计快照：完全相同的 functionCall 再来一次。
            ONE_READ_FRAME,
        ],
    );
    let started: Vec<&String> = deltas
        .iter()
        .filter_map(|delta| match delta {
            ChatDelta::ToolInputStarted { call_id, .. } => Some(call_id),
            _ => None,
        })
        .collect();
    let ended: Vec<&String> = deltas
        .iter()
        .filter_map(|delta| match delta {
            ChatDelta::ToolInputEnded { call_id, .. } => Some(call_id),
            _ => None,
        })
        .collect();
    assert_eq!(started, vec!["gemini-7-0-0-read"], "去重后只执行一次");
    assert_eq!(ended, started, "Started/Ended 成对且 ID 一致");
    assert_eq!(
        deltas
            .iter()
            .filter(|delta| matches!(delta, ChatDelta::ResponseFinished { .. }))
            .count(),
        1,
        "重复 finishReason 幂等"
    );
}

/// **同响应多调用唯一**：一帧内两个 functionCall（不同 part）ID 互不相同，
/// 且候选之间存在区别（不同 candidate 也不撞）。
#[test]
fn multiple_calls_in_one_response_get_distinct_ids() {
    let mut decoder = Decoder::new(SEQ);
    let deltas = decode_all(
        &mut decoder,
        &[
            r#"{"candidates":[{"content":{"parts":[{"functionCall":{"name":"read","args":{"path":"a"}}},{"functionCall":{"name":"read","args":{"path":"b"}}}],"role":"model"},"finishReason":"STOP","index":0}]}"#,
        ],
    );
    let ids: Vec<&String> = deltas
        .iter()
        .filter_map(|delta| match delta {
            ChatDelta::ToolInputStarted { call_id, .. } => Some(call_id),
            _ => None,
        })
        .collect();
    assert_eq!(ids.len(), 2);
    assert_ne!(ids[0], ids[1], "同响应不同 part 不得共用 call ID");
    // 同一工具名的两次调用靠 part 区分（不是靠名字）。
    assert!(ids[0].ends_with("-0-read"), "{:?}", ids[0]);
    assert!(ids[1].ends_with("-1-read"), "{:?}", ids[1]);
}

/// 摘要请求：不声明工具，系统提示使用请求覆写值（四协议同口径）。
#[test]
fn compact_request_omits_tools_and_overrides_the_system_prompt() {
    let request = crate::chat::events::ChatRequest {
        turns: vec![crate::chat::events::ChatTurn {
            role: "user".into(),
            text: "compress the transcript".into(),
            reasoning_metadata: None,
        }],
        items: vec![crate::chat::events::ModelItem::UserText(
            "compress the transcript".into(),
        )],
        model: crate::db::chat::ModelSnapshot {
            provider: "p".into(),
            model: "m".into(),
            variant: None,
        },
        variant_params: None,
        purpose: crate::chat::events::RequestPurpose::CompactSummary,
        system_prompt: Some("COMPACT INSTRUCTIONS".into()),
    };
    let built = super::build("https://api.example.invalid", Some("key"), &request).expect("build");
    let body: serde_json::Value = serde_json::from_str(&built.body).expect("json");
    assert!(!built.tools_requested, "摘要请求不得声明工具能力");
    assert!(
        !built.body.contains("\"tools\""),
        "body 不得含 tools：{body}"
    );
    let prompt = serde_json::to_string(&body).expect("json");
    assert!(prompt.contains("COMPACT INSTRUCTIONS"), "系统提示被覆写");
}
