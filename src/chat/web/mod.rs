//! AI Web 站点驱动：嵌入式 `ego-browser` 脚本与行协议解析。
//!
//! 脚本经 stdin 交给 `ego-browser nodejs`，用 `@@LX_AI_WEB@@` 前缀的 NDJSON 行回投
//! 增量与终态；本模块只做纯数据解析，不触碰 `AppState` 或子进程。

/// 嵌入式 DeepSeek 网页驱动辅助脚本（输出、新对话、CDP 文件上传与会话管理）。
pub const DEEPSEEK_HELPERS_SCRIPT: &str = include_str!("deepseek_helpers.js");

/// 嵌入式 DeepSeek 网页流式对话驱动主脚本。
pub const DEEPSEEK_SCRIPT: &str = include_str!("deepseek_script.js");

/// 嵌入式 Gemini 网页驱动辅助脚本（模式/思考等级切换、新对话、CDP 文件上传与会话管理）。
pub const GEMINI_HELPERS_SCRIPT: &str = include_str!("gemini_helpers.js");

/// 嵌入式 Gemini 网页流式对话驱动主脚本。
pub const GEMINI_SCRIPT: &str = include_str!("gemini_script.js");

/// 按目标站点返回 `(helpers_script, main_script)`。
pub fn scripts_for_site(site: crate::app::state::AiWebSite) -> (&'static str, &'static str) {
    match site {
        crate::app::state::AiWebSite::Deepseek => (DEEPSEEK_HELPERS_SCRIPT, DEEPSEEK_SCRIPT),
        crate::app::state::AiWebSite::Gemini => (GEMINI_HELPERS_SCRIPT, GEMINI_SCRIPT),
    }
}

/// 脚本输出行协议前缀。
pub const PROTOCOL_PREFIX: &str = "@@LX_AI_WEB@@";

/// 网页端单条历史会话摘要（供 `/resume` 列表展示）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AiWebSessionSummary {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub updated_at_ms: i64,
}

/// 网页端单条历史消息（供 `/resume` 恢复对话展示）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AiWebHistoryMessage {
    pub role: String,
    #[serde(default)]
    pub thinking: String,
    #[serde(default)]
    pub thinking_ms: Option<i64>,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub files: Vec<String>,
    #[serde(default)]
    pub created_at_ms: i64,
}

/// 脚本回投的一条流事件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AiWebStreamEvent {
    /// 深度思考增量。
    Thinking(String),
    /// 回答正文增量。
    Text(String),
    /// 当前对话已关联到云端会话（用于把本地临时 ID 升级为云端会话 ID 并同步标题）。
    Session { id: String, title: Option<String> },
    /// `/resume` 历史会话列表。
    Sessions(Vec<AiWebSessionSummary>),
    /// `/resume` 选中的历史会话完整消息。
    SessionLoaded {
        id: String,
        title: Option<String>,
        messages: Vec<AiWebHistoryMessage>,
    },
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
    let opt_str = |name: &str| {
        value
            .get(name)
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    match event {
        "thinking" => Some(AiWebStreamEvent::Thinking(field("delta"))),
        "text" => Some(AiWebStreamEvent::Text(field("delta"))),
        "session" => {
            let id = field("id");
            (!id.is_empty()).then(|| AiWebStreamEvent::Session {
                id,
                title: opt_str("title"),
            })
        }
        "sessions" => {
            let items = serde_json::from_value(value.get("items")?.clone()).ok()?;
            Some(AiWebStreamEvent::Sessions(items))
        }
        "session_loaded" => {
            let id = field("id");
            if id.is_empty() {
                return None;
            }
            let messages = serde_json::from_value(value.get("messages")?.clone()).ok()?;
            Some(AiWebStreamEvent::SessionLoaded {
                id,
                title: opt_str("title"),
                messages,
            })
        }
        "login_required" => Some(AiWebStreamEvent::LoginRequired { url: field("url") }),
        "done" => Some(AiWebStreamEvent::Done),
        "error" => Some(AiWebStreamEvent::Error(field("message"))),
        _ => None,
    }
}

/// 过滤 `ego-browser nodejs` 的非协议标准错误行：保留首条真实异常摘要，
/// 避免被后续调用栈（`at ...`）或通用退出尾注（`ego's nodejs process exited with code ...`）覆盖。
pub fn should_record_diag_line(trimmed: &str, current: Option<&str>) -> bool {
    if trimmed.is_empty()
        || trimmed.starts_with("[ego-browser:")
        || trimmed.starts_with("at ")
        || (trimmed.starts_with("ego's nodejs process exited with code") && current.is_some())
    {
        return false;
    }
    if current.is_some_and(|prev| {
        prev.starts_with("Error:")
            || prev.starts_with("TypeError:")
            || prev.starts_with("SyntaxError:")
            || prev.starts_with("ReferenceError:")
    }) {
        return false;
    }
    true
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

/// 注入 `ego-browser nodejs` 的 `__LX_PARAMS` 参数集。
#[derive(Debug, Clone, Default)]
pub struct ScriptParams<'a> {
    pub action: &'a str,
    pub model: Option<&'a str>,
    pub thinking: Option<&'a str>,
    pub prompt: Option<&'a str>,
    pub files: &'a [String],
    pub keep_files: &'a [String],
    pub first_turn: bool,
    pub stream_url: Option<&'a str>,
    pub session_id: Option<&'a str>,
    pub title: Option<&'a str>,
    pub edit_user_index: Option<usize>,
}

/// 组装交给 `ego-browser nodejs` 的脚本。
///
/// `ego-browser nodejs` 在守护进程内执行脚本，`process.env` 会跨调用残留；
/// 因此统一通过顶层块级常量 `const __LX_PARAMS = { ... };` 显式传入全部参数，避免跨轮串台。
pub fn script_source(site: crate::app::state::AiWebSite, params: ScriptParams<'_>) -> String {
    let (helpers_script, main_script) = scripts_for_site(site);
    let edit_index = params
        .edit_user_index
        .and_then(|idx| i64::try_from(idx).ok())
        .unwrap_or(-1);
    let files_json = serde_json::to_string(params.files).unwrap_or_else(|_| "[]".to_string());
    let keep_files_json =
        serde_json::to_string(params.keep_files).unwrap_or_else(|_| "[]".to_string());
    let mut source = String::from("const __LX_PARAMS = {\n");
    source.push_str("  action: ");
    source.push_str(&json_string(params.action));
    source.push_str(",\n  model: ");
    source.push_str(&json_string(params.model.unwrap_or("")));
    source.push_str(",\n  thinking: ");
    source.push_str(&json_string(params.thinking.unwrap_or("")));
    source.push_str(",\n  prompt: ");
    source.push_str(&json_string(params.prompt.unwrap_or("")));
    source.push_str(",\n  files: ");
    source.push_str(&files_json);
    source.push_str(",\n  keepFiles: ");
    source.push_str(&keep_files_json);
    source.push_str(",\n  firstTurn: ");
    source.push_str(if params.first_turn { "true" } else { "false" });
    source.push_str(",\n  streamUrl: ");
    source.push_str(&json_string(params.stream_url.unwrap_or("")));
    source.push_str(",\n  sessionId: ");
    source.push_str(&json_string(params.session_id.unwrap_or("")));
    source.push_str(",\n  title: ");
    source.push_str(&json_string(params.title.unwrap_or("")));
    source.push_str(&format!(",\n  editIndex: {edit_index},\n}};\n"));
    source.push_str(helpers_script);
    source.push('\n');
    source.push_str(main_script);
    source
}

/// JSON 字符串字面量；输入是任意用户文本，必须完整转义。
fn json_string(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_string())
}

#[cfg(test)]
mod tests;
