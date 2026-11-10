//! 流式聊天请求执行：reqwest 客户端、超时、取消与事件回流。
//!
//! 网络在独立 Tokio 任务运行；输出经 channel 送回事件循环。
//! 取消通过 `CancellationToken` 触发；超时包括连接超时与流空闲超时
//! （活动流不受固定总时长误杀）。Key 只出现在请求 header/URL，
//! 不进入错误消息与日志。

use std::time::{Duration, Instant};

use tokio::sync::{mpsc, watch};

use crate::logging::{PayloadCapture, PayloadMeta, PayloadRequest};

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

/// 等待「取消」信号；**发送端全部被丢弃不算取消**。
///
/// `watch::Receiver::changed()` 在发送端被丢弃时返回 `Err`。取消是「有人明确要求停止」，
/// 不是「没人再持有取消柄」——把 `Err` 当取消会让一个根本没人取消的请求自己中止
/// （`/compact` 摘要请求曾因此必然失败并报 `summary request failed: cancelled`）。
/// 这里只在**收到 `true`** 时返回；发送端消失后永久等待（请求按自身超时/EOF 收尾）。
async fn wait_cancel(cancel: &mut watch::Receiver<bool>) {
    loop {
        match cancel.changed().await {
            Ok(()) => {
                if *cancel.borrow_and_update() {
                    return;
                }
            }
            Err(_) => std::future::pending::<()>().await,
        }
    }
}

/// 不可取消请求的取消通道。
///
/// **两个返回值必须一起活到请求结束**：`watch::Sender` 一旦被丢弃，
/// `watch::Receiver::changed()` 会立刻返回 `Err`，而 [`run_stream`] 的 `select!` 把它
/// 当成用户取消——`/compact` 的摘要请求曾因此必然失败并报
/// `summary request failed: cancelled`。这里把「需要同时持有」写进签名与命名，
/// 避免调用方写成 `watch::channel(false).1`。
pub fn uncancellable() -> (watch::Sender<bool>, watch::Receiver<bool>) {
    watch::channel(false)
}

/// 网络流的关联快照；由调用点构造，供日志与 payload 使用。
#[derive(Debug, Clone)]
pub struct StreamLog {
    pub kind: &'static str,
    pub pane: Option<u32>,
    pub session_id: Option<String>,
    pub turn_index: Option<i64>,
    pub generation: Option<u64>,
    pub agent_id: Option<String>,
    pub parent_session: Option<String>,
}

impl Default for StreamLog {
    fn default() -> Self {
        Self {
            kind: "unknown",
            pane: None,
            session_id: None,
            turn_index: None,
            generation: None,
            agent_id: None,
            parent_session: None,
        }
    }
}

impl StreamLog {
    fn payload_meta(&self) -> PayloadMeta {
        PayloadMeta {
            kind: self.kind,
            pane: self.pane,
            session_id: self.session_id.clone(),
            turn_index: self.turn_index,
            generation: self.generation,
            agent_id: self.agent_id.clone(),
            parent_session: self.parent_session.clone(),
        }
    }

    fn session_str(&self) -> &str {
        self.session_id.as_deref().unwrap_or("-")
    }

    fn agent_str(&self) -> &str {
        self.agent_id.as_deref().unwrap_or("-")
    }

    fn parent_str(&self) -> &str {
        self.parent_session.as_deref().unwrap_or("-")
    }
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
/// `cancel` 收到 `true`（或以 `true` 创建）时请求立即中止，已收 partial 保留在返回值中。
/// **发送端全部被丢弃不算取消**：那种情况按请求自身超时/EOF 收尾（见 [`wait_cancel`]）。
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
    cancel: watch::Receiver<bool>,
    on_delta: impl FnMut(&[ChatDelta]),
) -> StreamOutcome {
    run_stream_with_log(
        transport_kind,
        base_url,
        api_key,
        idle_timeout,
        request,
        &StreamLog::default(),
        cancel,
        on_delta,
    )
    .await
}

/// 与 [`run_stream`] 相同，但带上会话/轮次关联字段用于日志与 payload。
pub async fn run_stream_with_log(
    transport_kind: &str,
    base_url: &str,
    api_key: Option<&str>,
    idle_timeout: Duration,
    request: ChatRequest,
    log: &StreamLog,
    mut cancel: watch::Receiver<bool>,
    mut on_delta: impl FnMut(&[ChatDelta]),
) -> StreamOutcome {
    let request_id = crate::logging::new_request_id();
    let started = Instant::now();
    let built = match transport::build_request(transport_kind, base_url, api_key, &request) {
        Ok(built) => built,
        Err(code) => {
            tracing::warn!(
                target: "lx::agent::request",
                request_id = %request_id,
                kind = log.kind,
                pane = log.pane,
                session = %log.session_str(),
                turn = log.turn_index,
                generation = log.generation,
                agent_id = %log.agent_str(),
                parent_session = %log.parent_str(),
                transport = transport_kind,
                provider = %request.model.provider,
                model = %request.model.model,
                code = %code,
                "agent request build failed"
            );
            let stop = StopReason::Failed { code };
            return StreamOutcome {
                deltas: vec![ChatDelta::Finished {
                    reason: stop.clone(),
                }],
                stop,
            };
        }
    };
    tracing::info!(
        target: "lx::agent::request",
        request_id = %request_id,
        kind = log.kind,
        pane = log.pane,
        session = %log.session_str(),
        turn = log.turn_index,
        generation = log.generation,
        agent_id = %log.agent_str(),
        parent_session = %log.parent_str(),
        transport = transport_kind,
        provider = %request.model.provider,
        model = %request.model.model,
        variant = ?request.model.variant,
        purpose = ?request.purpose,
        turns = request.turns.len(),
        items = request.items.len(),
        tools = request.tools.len(),
        body_bytes = built.body.len(),
        system_bytes = request.system_prompt.as_ref().map_or(0, String::len),
        url = %crate::logging::url_origin_and_path(&built.url),
        "agent request built"
    );
    let mut deltas = Vec::new();
    let mut attempt = 0u32;
    let stop = loop {
        if *cancel.borrow() {
            break StopReason::Cancelled;
        }
        let attempt_started = Instant::now();
        let before = deltas.len();
        let reason = stream_once(
            &built,
            transport_kind,
            idle_timeout,
            &mut cancel,
            &mut deltas,
            &mut on_delta,
            &request_id,
            log,
            attempt,
        )
        .await;
        tracing::debug!(
            target: "lx::agent::request",
            request_id = %request_id,
            session = %log.session_str(),
            attempt,
            reason = reason.as_str(),
            attempt_deltas = deltas.len().saturating_sub(before),
            elapsed_ms = attempt_started.elapsed().as_millis() as u64,
            "agent request attempt finished"
        );
        // 只在“本响应零增量、零工具输入”时重试：已有 partial 说明请求已被接受，
        // 重试会导致重复内容；已出现工具输入说明可能有文件副作用，绝不重试。
        let retryable = !*cancel.borrow() && can_retry(&deltas, &reason) && attempt < MAX_RETRIES;
        if !retryable {
            break reason;
        }
        deltas.retain(|delta| !matches!(delta, ChatDelta::Finished { .. }));
        attempt += 1;
        // 指数退避（2s、4s、8s，随后每次 10s），与 opencode 的节奏一致。
        let backoff = RETRY_BASE_DELAY
            .saturating_mul(1u32 << (attempt - 1).min(4))
            .min(RETRY_MAX_DELAY);
        tracing::warn!(
            target: "lx::agent::request",
            request_id = %request_id,
            session = %log.session_str(),
            attempt,
            reason = reason.as_str(),
            backoff_ms = backoff.as_millis() as u64,
            "agent request retrying"
        );
        tokio::select! {
            _ = tokio::time::sleep(backoff) => {}
            _ = wait_cancel(&mut cancel) => {
                tracing::debug!(
                    target: "lx::agent::request",
                    request_id = %request_id,
                    session = %log.session_str(),
                    stage = "backoff",
                    "agent request cancelled"
                );
                break StopReason::Cancelled;
            }
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
    let elapsed_ms = started.elapsed().as_millis();
    tracing::info!(
        target: "lx::agent::request",
        request_id = %request_id,
        kind = log.kind,
        pane = log.pane,
        session = %log.session_str(),
        turn = log.turn_index,
        generation = log.generation,
        agent_id = %log.agent_str(),
        parent_session = %log.parent_str(),
        attempts = attempt + 1,
        reason = stop.as_str(),
        deltas = deltas.len(),
        elapsed_ms = elapsed_ms as u64,
        "agent request finished"
    );
    crate::chat::usage_recorder::record_stream_finish(
        log,
        &request,
        &deltas,
        &stop,
        elapsed_ms as i64,
    );
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
    crate::logging::sanitize_text(body, 160)
}

/// 单次尝试执行流：返回结束原因（已收 partial 在 `deltas`）。
///
/// 每次解析出新增量后立即把**新增切片**交给 `on_delta`（打字机效果），
/// 同时累计进 `deltas` 供终态与 usage 使用。
// 参数承载请求、取消、增量与日志上下文，拆结构体反而掩盖执行顺序。
#[allow(clippy::too_many_arguments)]
async fn stream_once(
    built: &TransportRequest,
    transport_kind: &str,
    idle_timeout: Duration,
    cancel: &mut watch::Receiver<bool>,
    deltas: &mut Vec<ChatDelta>,
    on_delta: &mut impl FnMut(&[ChatDelta]),
    request_id: &str,
    log: &StreamLog,
    attempt: u32,
) -> StopReason {
    let mut capture = PayloadCapture::start(
        crate::logging::payload_enabled(),
        crate::logging::active_log_dir(),
        request_id,
        attempt,
        log.payload_meta(),
        PayloadRequest {
            method: "POST",
            url: &built.url,
            headers: &built.headers,
            body: &built.body,
            tools_requested: built.tools_requested,
        },
    );
    let reason = stream_once_inner(
        built,
        transport_kind,
        idle_timeout,
        cancel,
        deltas,
        on_delta,
        request_id,
        &mut capture,
    )
    .await;
    let error = (!matches!(reason, StopReason::Complete)).then(|| reason.as_str());
    capture.finish(error);
    reason
}

// 同上：与 stream_once 保持一一对应，避免额外抽象层。
#[allow(clippy::too_many_arguments)]
async fn stream_once_inner(
    built: &TransportRequest,
    transport_kind: &str,
    idle_timeout: Duration,
    cancel: &mut watch::Receiver<bool>,
    deltas: &mut Vec<ChatDelta>,
    on_delta: &mut impl FnMut(&[ChatDelta]),
    request_id: &str,
    capture: &mut PayloadCapture,
) -> StopReason {
    let client = match reqwest::Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .build()
    {
        Ok(client) => client,
        Err(error) => {
            tracing::warn!(
                target: "lx::agent::request",
                request_id,
                code = %format!("client build failed: {error}"),
                "agent http client build failed"
            );
            return StopReason::Failed {
                code: format!("client build failed: {error}"),
            };
        }
    };
    let mut builder = client
        .post(&built.url)
        .header("Accept", "text/event-stream");
    for (name, value) in &built.headers {
        builder = builder.header(name.as_str(), value.as_str());
    }
    // 首字节也按空闲超时处理（不是整请求总时长）：大上下文预填充可能长时间没有
    // 任何 SSE 字节，但一旦开始流式输出，持续产出不应被一个固定总时长杀掉。
    let first_byte_timeout = idle_timeout.max(FIRST_BYTE_TIMEOUT);
    let response = tokio::select! {
        response = tokio::time::timeout(
            first_byte_timeout,
            builder
                .header("Content-Type", "application/json")
                .body(built.body.clone())
                .send(),
        ) => match response {
            Ok(Ok(response)) => response,
            Ok(Err(error)) => {
                let code = request_error_code(error);
                tracing::warn!(
                    target: "lx::agent::request",
                    request_id,
                    code = %code,
                    "agent http request failed"
                );
                return StopReason::Failed { code };
            }
            Err(_) => {
                tracing::warn!(
                    target: "lx::agent::request",
                    request_id,
                    timeout_ms = first_byte_timeout.as_millis() as u64,
                    "agent http first byte timeout"
                );
                return StopReason::Failed {
                    code: "timeout".into(),
                };
            }
        },
        _ = wait_cancel(cancel) => {
            tracing::debug!(
                target: "lx::agent::request",
                request_id,
                stage = "send",
                "agent request cancelled"
            );
            return StopReason::Cancelled;
        }
    };
    let status = response.status();
    capture.response_status(status.as_u16());
    if !status.is_success() {
        // 读取错误体用于诊断，但只取截断/脱敏后的一段说明（设计允许：错误体经裁剪
        // 后可进入 UI/DB，且不得含密钥——Key 只在 header）。
        let raw_body = tokio::time::timeout(idle_timeout, response.text())
            .await
            .ok()
            .and_then(Result::ok);
        if let Some(body) = raw_body.as_deref() {
            capture.response_body(body);
        }
        let detail = raw_body
            .as_deref()
            .map(sanitize_error_detail)
            .filter(|detail| !detail.is_empty());
        let code = http_error_code(status.as_u16(), detail.as_deref(), built.tools_requested);
        tracing::warn!(
            target: "lx::agent::request",
            request_id,
            status = status.as_u16(),
            tools_requested = built.tools_requested,
            detail = %crate::logging::sanitize_text(
                raw_body.as_deref().unwrap_or_default(),
                2048
            ),
            "agent http error response"
        );
        return StopReason::Failed { code };
    }
    // 逐 chunk 喂入 SSE 解析器。
    let mut stream = response.bytes_stream();
    let mut parser = SseParser::new();
    // 解码器实例按“一次模型响应”创建：序号从进程内计数器取，
    // Google 用它合成跨响应唯一的 call ID（同轮第 2 次采样不得复用第 1 次）。
    let mut decoder = transport::decoder_for(transport_kind, transport::next_response_seq());
    let mut total_text = 0_usize;
    use tokio_stream::StreamExt;
    loop {
        let chunk = tokio::select! {
            chunk = tokio::time::timeout(idle_timeout, stream.next()) => match chunk {
                Ok(Some(Ok(bytes))) => bytes,
                Ok(Some(Err(error))) => {
                    let code = request_error_code(error);
                    tracing::warn!(
                        target: "lx::agent::request",
                        request_id,
                        code = %code,
                        "agent stream read failed"
                    );
                    return finalize_partial(
                        decoder.as_mut(),
                        deltas,
                        StopReason::Failed { code },
                        on_delta,
                    );
                }
                Ok(None) => break,
                Err(_) => {
                    tracing::warn!(
                        target: "lx::agent::request",
                        request_id,
                        timeout_ms = idle_timeout.as_millis() as u64,
                        "agent stream idle timeout"
                    );
                    return finalize_partial(
                        decoder.as_mut(),
                        deltas,
                        StopReason::Failed {
                            code: "timeout".into(),
                        },
                        on_delta,
                    );
                }
            },
            _ = wait_cancel(cancel) => {
                tracing::debug!(
                    target: "lx::agent::request",
                    request_id,
                    stage = "stream",
                    "agent request cancelled"
                );
                return finalize_partial(
                    decoder.as_mut(),
                    deltas,
                    StopReason::Cancelled,
                    on_delta,
                );
            }
        };
        if let Err(_too_large) = parser.feed(&chunk, MAX_FRAME_BYTES) {
            tracing::warn!(
                target: "lx::agent::request",
                request_id,
                max_frame_bytes = MAX_FRAME_BYTES,
                "agent stream frame exceeds size limit"
            );
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
            capture.record_frame(frame.event.as_deref(), &frame.data);
            match decode_frame(decoder.as_mut(), &frame, deltas, &mut total_text, on_delta) {
                Ok(()) => {}
                Err(code) => {
                    tracing::warn!(
                        target: "lx::agent::request",
                        request_id,
                        code = %code,
                        frame_event = ?frame.event,
                        frame_bytes = frame.data.len(),
                        "agent stream frame decode failed"
                    );
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
    if let Some(frame) = parser.flush() {
        capture.record_frame(frame.event.as_deref(), &frame.data);
        if let Err(code) = decode_frame(decoder.as_mut(), &frame, deltas, &mut total_text, on_delta)
        {
            tracing::warn!(
                target: "lx::agent::request",
                request_id,
                code = %code,
                frame_event = ?frame.event,
                frame_bytes = frame.data.len(),
                "agent stream frame decode failed"
            );
            return finalize_partial(
                decoder.as_mut(),
                deltas,
                StopReason::Failed { code },
                on_delta,
            );
        }
    }
    let residual = decoder.finish();
    if !residual.is_empty() {
        on_delta(&residual);
        deltas.extend(residual);
    }
    // EOF 时工具参数仍未闭合：半截 JSON 不得执行，按协议错误结束（不补造 Ended）。
    if decoder.has_open_tool_input() {
        tracing::warn!(
            target: "lx::agent::request",
            request_id,
            "agent stream ended with open tool input"
        );
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
    let reason = if decoder.has_completion() {
        StopReason::Complete
    } else {
        StopReason::Failed {
            code: "stream ended without completion".into(),
        }
    };
    tracing::debug!(
        target: "lx::agent::request",
        request_id,
        reason = reason.as_str(),
        "agent stream eof"
    );
    reason
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

/// reqwest 错误转稳定错误码（剥离 URL 与 query，绝不泄漏 `?key=`）。
fn request_error_code(error: reqwest::Error) -> String {
    if error.is_timeout() {
        "timeout".into()
    } else if error.is_connect() {
        "connection failed".into()
    } else {
        format!("request failed: {}", error.without_url())
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
