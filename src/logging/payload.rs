//! payload 捕获：显式开启后把一次请求的脱敏请求体与原始 SSE 载荷写入 JSONL。
//!
//! 未开启时所有方法都是 no-op；开启时单请求体、单条 SSE 与单请求总量均有上限。

use std::io::Write;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::redact::{omit_inline_images, redacted_headers, sanitize_text, url_origin_and_path};

/// 单请求体上限。
pub const MAX_REQUEST_BODY_BYTES: usize = 2 * 1024 * 1024;
/// 单条 SSE `data` 上限。
pub const MAX_SSE_LINE_BYTES: usize = 256 * 1024;
/// 单次请求 payload 总量上限。
pub const MAX_TOTAL_BYTES: usize = 20 * 1024 * 1024;

/// 请求的关联快照；与 `chat::client::StreamLog` 解耦，避免 logging 反向依赖 chat。
#[derive(Debug, Clone, Default)]
pub struct PayloadMeta {
    pub kind: &'static str,
    pub pane: Option<u32>,
    pub session_id: Option<String>,
    pub turn_index: Option<i64>,
    pub generation: Option<u64>,
    pub agent_id: Option<String>,
    pub parent_session: Option<String>,
}

/// 请求本身的只读字段。
pub struct PayloadRequest<'a> {
    pub method: &'a str,
    pub url: &'a str,
    pub headers: &'a [(String, String)],
    pub body: &'a str,
    pub tools_requested: bool,
}

/// 一次网络请求的 payload 捕获器。
pub struct PayloadCapture {
    enabled: bool,
    dir: Option<PathBuf>,
    request_id: String,
    attempt: u32,
    meta: PayloadMeta,
    request: Option<Value>,
    status: Option<u16>,
    error_body: Option<String>,
    sse: Vec<Value>,
    sse_bytes: usize,
    truncated: bool,
}

impl PayloadCapture {
    /// 构造捕获器；`enabled=false` 时后续方法全部 no-op。
    pub fn start(
        enabled: bool,
        dir: Option<PathBuf>,
        request_id: &str,
        attempt: u32,
        meta: PayloadMeta,
        request: PayloadRequest<'_>,
    ) -> Self {
        if !enabled || dir.is_none() {
            return Self::disabled();
        }
        let body = omit_inline_images(request.body, MAX_REQUEST_BODY_BYTES);
        let truncated = body.ends_with("…[truncated]");
        let body_value = serde_json::from_str::<Value>(&body).unwrap_or(Value::String(body));
        let headers: serde_json::Map<String, Value> = redacted_headers(request.headers)
            .into_iter()
            .map(|(name, value)| (name, Value::String(value)))
            .collect();
        let request_value = json!({
            "method": request.method,
            "url": url_origin_and_path(request.url),
            "headers": headers,
            "body": body_value,
            "tools_requested": request.tools_requested,
        });
        Self {
            enabled: true,
            dir,
            request_id: request_id.to_string(),
            attempt,
            meta,
            request: Some(request_value),
            status: None,
            error_body: None,
            sse: Vec::new(),
            sse_bytes: 0,
            truncated,
        }
    }

    fn disabled() -> Self {
        Self {
            enabled: false,
            dir: None,
            request_id: String::new(),
            attempt: 0,
            meta: PayloadMeta::default(),
            request: None,
            status: None,
            error_body: None,
            sse: Vec::new(),
            sse_bytes: 0,
            truncated: false,
        }
    }

    /// HTTP 响应状态码。
    pub fn response_status(&mut self, status: u16) {
        if self.enabled {
            self.status = Some(status);
        }
    }

    /// HTTP 非 2xx 的响应正文（内部再截断到 2KB）。
    pub fn response_body(&mut self, body: &str) {
        if self.enabled {
            self.error_body = Some(sanitize_text(body, 2048));
        }
    }

    /// 记录一帧 SSE；`data` 超长或总量超限时截断并标记。
    pub fn record_frame(&mut self, event: Option<&str>, data: &str) {
        if !self.enabled {
            return;
        }
        if self.sse_bytes >= MAX_TOTAL_BYTES {
            self.truncated = true;
            return;
        }
        let (data, line_truncated) = if data.len() > MAX_SSE_LINE_BYTES {
            (
                format!(
                    "{}…[truncated]",
                    super::redact::truncate_utf8(data, MAX_SSE_LINE_BYTES)
                ),
                true,
            )
        } else {
            (data.to_string(), false)
        };
        if line_truncated {
            self.truncated = true;
        }
        let remaining = MAX_TOTAL_BYTES.saturating_sub(self.sse_bytes);
        if data.len() > remaining {
            self.truncated = true;
            return;
        }
        self.sse_bytes += data.len() + event.map_or(0, str::len);
        self.sse.push(json!({"event": event, "data": data}));
    }

    /// 结束一次请求并写一行 JSONL；写失败只降级为 debug 日志。
    pub fn finish(self, error: Option<&str>) {
        if !self.enabled {
            return;
        }
        let Some(dir) = self.dir else {
            return;
        };
        let response = json!({
            "status": self.status,
            "sse": self.sse,
            "body": self.error_body,
        });
        let record = json!({
            "ts": super::utc_timestamp_now(),
            "pid": std::process::id(),
            "request_id": self.request_id,
            "kind": self.meta.kind,
            "pane": self.meta.pane,
            "session_id": self.meta.session_id,
            "turn_index": self.meta.turn_index,
            "generation": self.meta.generation,
            "agent_id": self.meta.agent_id,
            "parent_session": self.meta.parent_session,
            "attempt": self.attempt,
            "request": self.request,
            "response": response,
            "error": error.map(|text| sanitize_text(text, 2048)),
            "truncated": self.truncated,
        });
        let Ok(line) = serde_json::to_string(&record) else {
            return;
        };
        let path = payload_path(&dir);
        // 有意忽略：payload 是显式调试模式，写失败不影响请求结果。
        if let Err(error) = append_line(&path, &line) {
            tracing::debug!(
                target: "lx::logging",
                path = %path.display(),
                %error,
                "payload write failed"
            );
        }
    }
}

pub(crate) fn payload_path(dir: &Path) -> PathBuf {
    dir.join(format!(
        "payload-{}.jsonl",
        super::format_date(super::utc_today())
    ))
}

fn append_line(path: &Path, line: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
        let _ = crate::platform::restrict_dir_permissions(parent);
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    let _ = crate::platform::restrict_file_permissions(path);
    let mut buffer = Vec::with_capacity(line.len() + 1);
    buffer.extend_from_slice(line.as_bytes());
    buffer.push(b'\n');
    file.write_all(&buffer)
}

#[cfg(test)]
mod tests;
