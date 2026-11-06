//! AI Web 站点驱动：嵌入式 `ego-browser` 脚本与行协议解析。
//!
//! 脚本经 stdin 交给 `ego-browser nodejs`，用 `@@LX_AI_WEB@@` 前缀的 NDJSON 行回投
//! 增量与终态；本模块只做纯数据解析，不触碰 `AppState` 或子进程。

/// 嵌入式 DeepSeek 网页驱动脚本。
pub const DEEPSEEK_SCRIPT: &str = include_str!("deepseek_script.js");

/// 脚本输出行协议前缀。
pub const PROTOCOL_PREFIX: &str = "@@LX_AI_WEB@@";

/// 脚本回投的一条流事件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AiWebStreamEvent {
    /// 深度思考增量。
    Thinking(String),
    /// 回答正文增量。
    Text(String),
    /// 网页未登录，需要用户在 ego-browser 中完成登录。
    LoginRequired { url: String },
    /// 本轮回答生成完毕。
    Done,
    /// 页面或驱动异常。
    Error(String),
}

/// 解析脚本的一行标准输出；非协议行（如 `[ego-browser:notice]`）返回 `None`。
pub fn parse_script_line(line: &str) -> Option<AiWebStreamEvent> {
    let payload = line.trim().strip_prefix(PROTOCOL_PREFIX)?.trim();
    let value: serde_json::Value = serde_json::from_str(payload).ok()?;
    let event = value.get("event")?.as_str()?;
    let field = |name: &str| {
        value
            .get(name)
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    match event {
        "thinking" => Some(AiWebStreamEvent::Thinking(field("delta"))),
        "text" => Some(AiWebStreamEvent::Text(field("delta"))),
        "login_required" => Some(AiWebStreamEvent::LoginRequired { url: field("url") }),
        "done" => Some(AiWebStreamEvent::Done),
        "error" => Some(AiWebStreamEvent::Error(field("message"))),
        _ => None,
    }
}

/// 定位 `ego-browser`：环境变量优先，其次 `~/.local/bin`，最后交给 `PATH`。
pub fn resolve_ego_browser_bin() -> std::path::PathBuf {
    if let Some(bin) = std::env::var_os("LX_EGO_BROWSER_BIN") {
        return std::path::PathBuf::from(bin);
    }
    if let Some(home) = std::env::var_os("HOME") {
        let candidate = std::path::PathBuf::from(home)
            .join(".local")
            .join("bin")
            .join("ego-browser");
        if candidate.is_file() {
            return candidate;
        }
    }
    std::path::PathBuf::from("ego-browser")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(json: &str) -> Option<AiWebStreamEvent> {
        parse_script_line(&format!("{PROTOCOL_PREFIX}{json}"))
    }

    #[test]
    fn parses_thinking_and_text_deltas() {
        assert_eq!(
            parse(r#"{"event":"thinking","delta":"hmm"}"#),
            Some(AiWebStreamEvent::Thinking("hmm".into()))
        );
        assert_eq!(
            parse(r#"{"event":"text","delta":"hello"}"#),
            Some(AiWebStreamEvent::Text("hello".into()))
        );
    }

    #[test]
    fn parses_empty_delta() {
        assert_eq!(
            parse(r#"{"event":"text","delta":""}"#),
            Some(AiWebStreamEvent::Text(String::new()))
        );
        assert_eq!(
            parse(r#"{"event":"thinking","delta":""}"#),
            Some(AiWebStreamEvent::Thinking(String::new()))
        );
    }

    #[test]
    fn parses_login_required_with_url() {
        assert_eq!(
            parse(r#"{"event":"login_required","url":"https://chat.deepseek.com/sign_in"}"#),
            Some(AiWebStreamEvent::LoginRequired {
                url: "https://chat.deepseek.com/sign_in".into()
            })
        );
    }

    #[test]
    fn parses_done_and_error() {
        assert_eq!(parse(r#"{"event":"done"}"#), Some(AiWebStreamEvent::Done));
        assert_eq!(
            parse(r#"{"event":"error","message":"user is controlling"}"#),
            Some(AiWebStreamEvent::Error("user is controlling".into()))
        );
    }

    #[test]
    fn ignores_invalid_json_and_unknown_events() {
        assert_eq!(parse("not json"), None);
        assert_eq!(parse(r#"{"event":"unknown"}"#), None);
        assert_eq!(parse(r#"{"delta":"missing event"}"#), None);
    }

    #[test]
    fn ignores_non_protocol_noise() {
        assert_eq!(
            parse_script_line("[ego-browser:notice] Ego Lite update is available"),
            None
        );
        assert_eq!(parse_script_line(""), None);
        assert_eq!(parse_script_line("console output"), None);
    }
}
