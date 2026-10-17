//! 流式聊天请求执行：reqwest 客户端、超时、取消与事件回流。
//!
//! 网络在独立 Tokio 任务运行；输出经 channel 送回事件循环。
//! 取消通过 `CancellationToken` 触发；超时包括连接超时与流空闲超时
//! （活动流不受固定总时长误杀）。Key 只出现在请求 header/URL，
//! 不进入错误消息与日志。

use std::time::Duration;

use tokio::sync::{mpsc, watch};

use super::events::{ChatDelta, ChatRequest, MAX_FRAME_BYTES, MAX_TOTAL_TEXT_BYTES, StopReason};
use super::sse::{SseFrame, SseParser};
use super::transport::tools::{TOOLS_UNSUPPORTED_CODE, is_tools_unsupported_status};
use super::transport::{self, TransportRequest};

/// 一次流式请求的最终结果（恰好一个终态）。
#[derive(Debug, Clone, PartialEq)]
pub struct StreamOutcome {
    /// 全部增量（含终态事件）。
    pub deltas: Vec<ChatDelta>,
    /// 最终结束原因（Cancelled / Failed / Complete）。
    pub stop: StopReason,
}

/// 连接超时。
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// 首字节超时（TTFB）。
const FIRST_BYTE_TIMEOUT: Duration = Duration::from_secs(60);
/// 最大重试次数：对齐 opencode（`schedule.recurs(10)`）。
const MAX_RETRIES: u32 = 10;
/// 指数退避基线与单次间隔上限：对齐 opencode 的 `exponential(2s)` + `spaced(10s)`。
const RETRY_BASE_DELAY: Duration = Duration::from_secs(2);
const RETRY_MAX_DELAY: Duration = Duration::from_secs(10);

/// 执行一次流式请求。
///
/// `cancel` 关闭时请求立即中止，已收 partial 保留在返回值中。
/// 事件层按 request key 过滤过期结果；本函数不触碰应用状态。
/// 每个新产出的增量批经 `on_delta` 即时回调（打字机效果：内容随网络分块实时到达），
/// 同时仍完整累计在返回值里供终态/usage 使用。
/// 仅在**尚未产出任何内容、也没有任何工具输入事件**时重试（最多 10 次，指数退避
/// 2s→10s，对齐 opencode）。一旦本响应出现过 `ToolInputStarted/Delta/Ended`，
/// 说明模型已经规划了文件操作，重试可能造成重复副作用：该响应绝不重试。
/// 半截流同样不重试，避免重复内容。
pub async fn run_stream(
    transport_kind: &str,
    base_url: &str,
    api_key: Option<&str>,
    idle_timeout: Duration,
    request: ChatRequest,
    mut cancel: watch::Receiver<bool>,
    mut on_delta: impl FnMut(&[ChatDelta]),
) -> StreamOutcome {
    let built = match transport::build_request(transport_kind, base_url, api_key, &request) {
        Ok(built) => built,
        Err(code) => {
            let stop = StopReason::Failed { code };
            return StreamOutcome {
                deltas: vec![ChatDelta::Finished {
                    reason: stop.clone(),
                }],
                stop,
            };
        }
    };
    let mut deltas = Vec::new();
    let mut attempt = 0u32;
    let stop = loop {
        let reason = stream_once(
            &built,
            transport_kind,
            idle_timeout,
            &mut cancel,
            &mut deltas,
            &mut on_delta,
        )
        .await;
        // 只在“本响应零增量、零工具输入”时重试：已有 partial 说明请求已被接受，
        // 重试会导致重复内容；已出现工具输入说明可能有文件副作用，绝不重试。
        let retryable = can_retry(&deltas, &reason) && attempt < MAX_RETRIES;
        if !retryable {
            break reason;
        }
        attempt += 1;
        // 指数退避（2s、4s、8s，随后每次 10s），与 opencode 的节奏一致。
        let backoff = RETRY_BASE_DELAY
            .saturating_mul(1u32 << (attempt - 1).min(4))
            .min(RETRY_MAX_DELAY);
        tokio::select! {
            _ = tokio::time::sleep(backoff) => {}
            _ = cancel.changed() => break StopReason::Cancelled,
        }
    };
    // 保证 deltas 恰好含一个终态（stream_once 中途路径可能尚未追加）。
    if !deltas
        .iter()
        .any(|delta| matches!(delta, ChatDelta::Finished { .. }))
    {
        deltas.push(ChatDelta::Finished {
            reason: stop.clone(),
        });
    }
    StreamOutcome { deltas, stop }
}

/// 本响应是否仍可安全重试：零增量、零工具输入，且错误属于临时故障。
///
/// `deltas` 是本次调用**累计**的增量（跨尝试累积），因此任一工具输入事件都会
/// 永久关闭重试；`ResponseFinished` 也视为已产出内容（模型已给出结论）。
fn can_retry(deltas: &[ChatDelta], reason: &StopReason) -> bool {
    if deltas.iter().any(has_tool_input) {
        return false;
    }
    let produced_content = deltas
        .iter()
        .any(|delta| !matches!(delta, ChatDelta::Finished { .. }));
    if produced_content {
        return false;
    }
    matches!(reason, StopReason::Failed { code } if is_transient(code))
}

/// 是否为工具输入类增量（Started/Delta/Ended）。
fn has_tool_input(delta: &ChatDelta) -> bool {
    matches!(
        delta,
        ChatDelta::ToolInputStarted { .. }
            | ChatDelta::ToolInputDelta { .. }
            | ChatDelta::ToolInputEnded { .. }
    )
}

/// 错误码是否属于可安全重试的临时故障（连接失败/超时/限流/5xx）。
fn is_transient(code: &str) -> bool {
    if code == "timeout" || code == "connection failed" {
        return true;
    }
    // 错误码形如 `http 503` 或 `http 503 · <detail>`：取状态码前缀。
    if let Some(rest) = code.strip_prefix("http ")
        && let Some(status) = rest
            .split_whitespace()
            .next()
            .and_then(|token| token.parse::<u16>().ok())
    {
        return status == 429 || (500..=599).contains(&status);
    }
    false
}

/// 把 HTTP 失败归类成稳定错误码。
///
/// 本轮请求带了 `tools` 且状态码属于能力类 4xx/501 时，错误码前缀明确为
/// [`TOOLS_UNSUPPORTED_CODE`]（“该接口不支持工具调用”），**不**静默移除 tools 重试；
/// 认证/限流/超时仍按 `http <status>` 原样上报，保留既有重试与诊断语义。
fn http_error_code(status: u16, detail: Option<&str>, tools_requested: bool) -> String {
    let base = if tools_requested && is_tools_unsupported_status(status) {
        format!("{TOOLS_UNSUPPORTED_CODE} · http {status}")
    } else {
        format!("http {status}")
    };
    match detail {
        Some(detail) => format!("{base} · {detail}"),
        None => base,
    }
}

/// 测试用：暴露 HTTP 错误归类函数（保持 `http_error_code` 私有 + 可断言）。
#[cfg(test)]
pub(crate) fn http_error_code_for_tests(status: u16, detail: Option<&str>, tools: bool) -> String {
    http_error_code(status, detail, tools)
}

/// 把 HTTP 错误体裁剪成一行、单行、限长的可读说明（脱敏后进入 UI/DB）。
fn sanitize_error_detail(body: &str) -> String {
    let single: String = body
        .chars()
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .collect();
    single
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(160)
        .collect()
}

/// 单次尝试执行流：返回结束原因（已收 partial 在 `deltas`）。
///
/// 每次解析出新增量后立即把**新增切片**交给 `on_delta`（打字机效果），
/// 同时累计进 `deltas` 供终态与 usage 使用。
async fn stream_once(
    built: &TransportRequest,
    transport_kind: &str,
    idle_timeout: Duration,
    cancel: &mut watch::Receiver<bool>,
    deltas: &mut Vec<ChatDelta>,
    on_delta: &mut impl FnMut(&[ChatDelta]),
) -> StopReason {
    let client = match reqwest::Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .build()
    {
        Ok(client) => client,
        Err(error) => {
            return StopReason::Failed {
                code: format!("client build failed: {error}"),
            };
        }
    };
    let mut builder = client
        .post(&built.url)
        .header("Accept", "text/event-stream")
        .timeout(idle_timeout.max(FIRST_BYTE_TIMEOUT));
    for (name, value) in &built.headers {
        builder = builder.header(name.as_str(), value.as_str());
    }
    let response = tokio::select! {
        response = builder
            .header("Content-Type", "application/json")
            .body(built.body.clone())
            .send() => match response {
                Ok(response) => response,
                Err(error) => return StopReason::Failed { code: request_error_code(&error) },
            },
        _ = cancel.changed() => return StopReason::Cancelled,
    };
    let status = response.status();
    if !status.is_success() {
        // 读取错误体用于诊断，但只取截断/脱敏后的一段说明（设计允许：错误体经裁剪
        // 后可进入 UI/DB，且不得含密钥——Key 只在 header）。
        let detail = response
            .text()
            .await
            .ok()
            .map(|body| sanitize_error_detail(&body))
            .filter(|detail| !detail.is_empty());
        return StopReason::Failed {
            code: http_error_code(status.as_u16(), detail.as_deref(), built.tools_requested),
        };
    }
    // 逐 chunk 喂入 SSE 解析器。
    let mut stream = response.bytes_stream();
    let mut parser = SseParser::new();
    let mut decoder = transport::decoder_for(transport_kind);
    let mut total_text = 0_usize;
    use tokio_stream::StreamExt;
    loop {
        let chunk = tokio::select! {
            chunk = stream.next() => match chunk {
                Some(Ok(bytes)) => bytes,
                Some(Err(error)) => {
                    return finalize_partial(
                        decoder.as_mut(),
                        deltas,
                        StopReason::Failed {
                            code: request_error_code(&error),
                        },
                        on_delta,
                    );
                }
                None => break,
            },
            _ = cancel.changed() => {
                return finalize_partial(
                    decoder.as_mut(),
                    deltas,
                    StopReason::Cancelled,
                    on_delta,
                );
            }
        };
        if let Err(_too_large) = parser.feed(&chunk, MAX_FRAME_BYTES) {
            return finalize_partial(
                decoder.as_mut(),
                deltas,
                StopReason::Failed {
                    code: "frame exceeds size limit".into(),
                },
                on_delta,
            );
        }
        for frame in parser.take_frames() {
            match decode_frame(decoder.as_mut(), &frame, deltas, &mut total_text, on_delta) {
                Ok(()) => {}
                Err(code) => {
                    return finalize_partial(
                        decoder.as_mut(),
                        deltas,
                        StopReason::Failed { code },
                        on_delta,
                    );
                }
            }
        }
    }
    // EOF：半帧 flush（协议层判定完成性）。
    if let Some(frame) = parser.flush()
        && let Err(code) = decode_frame(decoder.as_mut(), &frame, deltas, &mut total_text, on_delta)
    {
        return finalize_partial(
            decoder.as_mut(),
            deltas,
            StopReason::Failed { code },
            on_delta,
        );
    }
    let residual = decoder.finish();
    if !residual.is_empty() {
        on_delta(&residual);
        deltas.extend(residual);
    }
    // EOF 时工具参数仍未闭合：半截 JSON 不得执行，按协议错误结束（不补造 Ended）。
    if decoder.has_open_tool_input() {
        return StopReason::Failed {
            code: super::transport::tools::TOOL_INPUT_TRUNCATED_CODE.to_string(),
        };
    }
    // 已有协议终态（finish_reason / message_stop / response.completed 等）则沿用；
    // 缺完成信号按设计视为中断/协议错误，不擅自标记完成。
    if let Some(reason) = deltas.iter().rev().find_map(|delta| match delta {
        ChatDelta::Finished { reason } => Some(reason.clone()),
        _ => None,
    }) {
        return reason;
    }
    if decoder.has_completion() {
        StopReason::Complete
    } else {
        StopReason::Failed {
            code: "stream ended without completion".into(),
        }
    }
}

/// 解码一帧并累计正文大小；新增量即时回调 `on_delta`（打字机效果）。
fn decode_frame(
    decoder: &mut dyn transport::StreamDecoder,
    frame: &SseFrame,
    deltas: &mut Vec<ChatDelta>,
    total_text: &mut usize,
    on_delta: &mut impl FnMut(&[ChatDelta]),
) -> Result<(), String> {
    // [DONE] 标记：OpenAI 兼容流的正常结束信号，等价于完成（EOF 后据此判定完成，
    // 不再误报 "stream ended without completion"）。跳过 JSON 解析。
    if frame.data.trim() == "[DONE]" {
        decoder.mark_done();
        return Ok(());
    }
    let produced = decoder.decode_frame(&frame.data)?;
    let mut fresh: Vec<ChatDelta> = Vec::new();
    for delta in produced {
        if let ChatDelta::Part(part) = &delta {
            *total_text += part.text.len();
            if *total_text > MAX_TOTAL_TEXT_BYTES {
                return Err("response exceeds size limit".into());
            }
        }
        // 整轮终态由 `Finished` 事件单独回投，不进增量批（避免重复/顺序歧义）；
        // 其余（含 `ResponseFinished`）都是状态层需要即时消费的增量。
        if delta.is_incremental() {
            fresh.push(delta.clone());
        }
        deltas.push(delta);
    }
    if !fresh.is_empty() {
        on_delta(&fresh);
    }
    Ok(())
}

/// 流中断时：冲刷解码器残余内容（不产生终态），再追加**恰好一个**终态。
fn finalize_partial(
    decoder: &mut dyn transport::StreamDecoder,
    deltas: &mut Vec<ChatDelta>,
    stop: StopReason,
    on_delta: &mut impl FnMut(&[ChatDelta]),
) -> StopReason {
    let residual = decoder.finish();
    if !residual.is_empty() {
        on_delta(&residual);
        deltas.extend(residual);
    }
    deltas.push(ChatDelta::Finished {
        reason: stop.clone(),
    });
    stop
}

/// reqwest 错误转稳定错误码（不含 URL 与 Key）。
fn request_error_code(error: &reqwest::Error) -> String {
    if error.is_timeout() {
        "timeout".into()
    } else if error.is_connect() {
        "connection failed".into()
    } else {
        format!("request failed: {error}")
    }
}

/// 后台流式任务的启动辅助：spawn 到 Tokio 并把结果送回 channel。
///
/// 事件层用 request key 过滤过期结果；此处只负责数据搬运。
pub fn spawn_stream(
    transport_kind: String,
    base_url: String,
    api_key: Option<String>,
    idle_timeout: Duration,
    request: ChatRequest,
    cancel: watch::Receiver<bool>,
    sender: mpsc::UnboundedSender<StreamOutcome>,
) {
    tokio::spawn(async move {
        let outcome = run_stream(
            &transport_kind,
            &base_url,
            api_key.as_deref(),
            idle_timeout,
            request,
            cancel,
            |_| {},
        )
        .await;
        let _ = sender.send(outcome);
    });
}

#[cfg(test)]
mod tests;
