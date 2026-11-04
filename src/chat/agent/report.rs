//! 子 agent 汇报视图与 `<subagent_report>` XML 信封序列化。

use serde_json::{Value, json};

use super::{AgentRole, MAX_REPORT_BYTES};

/// 汇报块数据源（父消息 `subagent_report_json` 反序列化）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubagentReportView {
    /// 稳定 agent id。
    pub agent_id: String,
    /// 可选唯一昵称（如 `"Turing"`；旧记录为 None）。
    pub nickname: Option<String>,
    /// 面向用户的短标题；旧记录为空。
    pub title: String,
    /// 角色。
    pub role: AgentRole,
    /// `completed` / `failed` / `cancelled`。
    pub status: String,
    /// 去重后的改动文件数。
    pub files_changed: usize,
    /// 子 turn 耗时（毫秒）。
    pub duration_ms: Option<i64>,
    /// 报告正文（已按上限截断）。
    pub body: String,
    /// 报告生成时刻（UTC 毫秒）。
    pub delivered_at_ms: i64,
}

impl SubagentReportView {
    /// 落库 JSON（`agent_subagent.report_json` 共用；消息侧批量存数组）。
    pub fn to_json(&self) -> String {
        self.to_value().to_string()
    }

    fn to_value(&self) -> Value {
        let mut obj = json!({
            "agent_id": self.agent_id,
            "title": self.title,
            "role": self.role.as_str(),
            "status": self.status,
            "files_changed": self.files_changed,
            "duration_ms": self.duration_ms,
            "body": self.body,
            "delivered_at_ms": self.delivered_at_ms,
        });
        if let Some(nickname) = self.nickname.as_deref().filter(|s| !s.is_empty())
            && let Some(map) = obj.as_object_mut()
        {
            map.insert("nickname".to_string(), Value::String(nickname.to_string()));
        }
        obj
    }

    /// 从落库 JSON 还原单条报告；损坏返回 None。
    pub fn from_json(text: &str) -> Option<Self> {
        let value: Value = serde_json::from_str(text).ok()?;
        Self::from_value(&value)
    }

    fn from_value(value: &Value) -> Option<Self> {
        Some(Self {
            agent_id: value.get("agent_id")?.as_str()?.to_string(),
            nickname: value
                .get("nickname")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string),
            title: value
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            role: AgentRole::parse(value.get("role")?.as_str()?)?,
            status: value.get("status")?.as_str()?.to_string(),
            files_changed: value
                .get("files_changed")
                .and_then(Value::as_u64)
                .unwrap_or_default() as usize,
            duration_ms: value.get("duration_ms").and_then(Value::as_i64),
            body: value.get("body")?.as_str()?.to_string(),
            delivered_at_ms: value
                .get("delivered_at_ms")
                .and_then(Value::as_i64)
                .unwrap_or_default(),
        })
    }
}

/// 消息 `subagent_report_json` 反序列化：新格式为数组，旧格式为单对象。
pub fn reports_from_json(text: &str) -> Vec<SubagentReportView> {
    let Ok(value) = serde_json::from_str::<Value>(text) else {
        return Vec::new();
    };
    match value {
        Value::Array(items) => items
            .iter()
            .filter_map(SubagentReportView::from_value)
            .collect(),
        Value::Object(_) => SubagentReportView::from_value(&value).into_iter().collect(),
        _ => Vec::new(),
    }
}

/// 批量报告落库 JSON：统一存数组，单条也走数组。
pub fn reports_to_json(reports: &[SubagentReportView]) -> String {
    Value::Array(reports.iter().map(SubagentReportView::to_value).collect()).to_string()
}

/// 注入父会话的报告文本：纯文本 + XML 包裹 + 元字符转义。
pub fn format_report_envelope(report: &SubagentReportView) -> String {
    let body = truncate_report_body(&sanitize_control(&report.body));
    let duration = report
        .duration_ms
        .map(|value| format!("{value}"))
        .unwrap_or_else(|| "unknown".to_string());
    let nickname_attr = report
        .nickname
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .map(|nick| format!(" nickname=\"{}\"", escape_attribute(nick.trim())))
        .unwrap_or_default();
    format!(
        "<subagent_report agent_id=\"{}\"{} title=\"{}\" role=\"{}\" status=\"{}\" files_changed=\"{}\" duration_ms=\"{}\">\n{}\n</subagent_report>",
        escape_attribute(&report.agent_id),
        nickname_attr,
        escape_attribute(&report.title),
        report.role.as_str(),
        escape_attribute(&report.status),
        report.files_changed,
        duration,
        escape_text(&body),
    )
}

/// 报告正文截断（按字节上限，且不切断 UTF-8 字符）。
pub fn truncate_report_body(body: &str) -> String {
    if body.len() <= MAX_REPORT_BYTES {
        return body.to_string();
    }
    let mut end = MAX_REPORT_BYTES;
    while end > 0 && !body.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n[truncated]", &body[..end])
}

/// 剥离 ANSI/控制字符（保留换行与制表符）。
fn sanitize_control(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for next in chars.by_ref() {
                    if ('@'..='~').contains(&next) {
                        break;
                    }
                }
            } else {
                let _ = chars.next();
            }
            continue;
        }
        if ch == '\n' || ch == '\t' || !ch.is_control() {
            out.push(ch);
        }
    }
    out
}

/// 正文 XML 元字符转义（防止伪造信封边界）。
fn escape_text(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// 属性值转义（含引号）。
fn escape_attribute(text: &str) -> String {
    escape_text(text).replace('"', "&quot;")
}
