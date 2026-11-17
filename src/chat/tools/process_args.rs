//! `exec_command` / `write_stdin` 的参数规范化：严格白名单 + 范围校验。
//!
//! 校验先于任何进程副作用：这里返回错误时不会 spawn、不会写 stdin、不会停止任何进程。
//! 非法取值**拒绝而不静默修正**（越界的 `yield_time_ms` 不会被悄悄钳到上限）。

use serde_json::Value;

use crate::chat::process::{
    DEFAULT_STDIN_YIELD_MS, DEFAULT_YIELD_MS, ExecArgs, MAX_OUTPUT_TOKENS, MAX_STDIN_BYTES,
    MAX_TIMEOUT_MS, MAX_YIELD_MS, StdinArgs,
};

use super::types::{ToolErrorCode, ToolExecutionResult, ToolFailure, ToolResultPublic};

/// 解析 `exec_command` 参数。
pub fn parse_exec_args(arguments_json: &str) -> ToolResultPublic<ExecArgs> {
    let value = object_of(arguments_json)?;
    let allowed = [
        "cmd",
        "workdir",
        "shell",
        "login",
        "tty",
        "yield_time_ms",
        "timeout_ms",
        "max_output_tokens",
    ];
    reject_unknown(&value, &allowed)?;
    let cmd = value
        .get("cmd")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|cmd| !cmd.is_empty())
        .ok_or_else(|| ToolFailure::invalid("'cmd' must be a non-empty string"))?
        .to_string();
    let workdir = optional_string(&value, "workdir")?;
    let shell = optional_string(&value, "shell")?;
    // 对齐 codex：默认使用登录 shell（读到用户的 PATH/别名），显式 `login: false` 才关闭。
    let login = optional_bool(&value, "login")?.unwrap_or(true);
    let tty = optional_bool(&value, "tty")?.unwrap_or(false);
    let yield_time_ms = optional_int(&value, "yield_time_ms", 0, MAX_YIELD_MS as i64)?
        .unwrap_or(DEFAULT_YIELD_MS as i64) as u64;
    let timeout_ms =
        optional_int(&value, "timeout_ms", 1, MAX_TIMEOUT_MS as i64)?.map(|ms| ms as u64);
    let max_output_tokens = optional_int(&value, "max_output_tokens", 1, MAX_OUTPUT_TOKENS as i64)?
        .map(|tokens| tokens as usize)
        .unwrap_or(MAX_OUTPUT_TOKENS);
    Ok(ExecArgs {
        cmd,
        workdir,
        shell,
        login,
        tty,
        yield_time_ms,
        timeout_ms,
        max_output_tokens,
    })
}

/// 解析 `write_stdin` 参数。
pub fn parse_stdin_args(arguments_json: &str) -> ToolResultPublic<StdinArgs> {
    let value = object_of(arguments_json)?;
    let allowed = [
        "session_id",
        "chars",
        "terminate",
        "yield_time_ms",
        "max_output_tokens",
    ];
    reject_unknown(&value, &allowed)?;
    let session_id = value
        .get("session_id")
        .and_then(Value::as_u64)
        .filter(|id| *id > 0)
        .ok_or_else(|| ToolFailure::invalid("'session_id' must be a positive integer"))?;
    let chars = match value.get("chars") {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(text)) => text.clone(),
        Some(_) => return Err(ToolFailure::invalid("'chars' must be a string")),
    };
    if chars.len() > MAX_STDIN_BYTES {
        return Err(ToolFailure::new(
            ToolErrorCode::TooLarge,
            format!("'chars' exceeds the {MAX_STDIN_BYTES} byte limit"),
        ));
    }
    let terminate = optional_bool(&value, "terminate")?.unwrap_or(false);
    if terminate && !chars.is_empty() {
        return Err(ToolFailure::invalid(
            "'terminate' cannot be combined with a non-empty 'chars'",
        ));
    }
    let yield_time_ms = optional_int(&value, "yield_time_ms", 0, MAX_YIELD_MS as i64)?
        .unwrap_or(DEFAULT_STDIN_YIELD_MS as i64) as u64;
    let max_output_tokens = optional_int(&value, "max_output_tokens", 1, MAX_OUTPUT_TOKENS as i64)?
        .map(|tokens| tokens as usize)
        .unwrap_or(MAX_OUTPUT_TOKENS);
    Ok(StdinArgs {
        session_id,
        chars,
        terminate,
        yield_time_ms,
        max_output_tokens,
    })
}

/// 进程操作失败 → 工具失败结果（`changes` 恒为空：进程工具不涉及文件）。
pub fn process_failure(error: crate::chat::process::ProcessError) -> ToolExecutionResult {
    let code = match error.code {
        "resource_limit" => ToolErrorCode::ResourceLimit,
        "process_not_found" => ToolErrorCode::ProcessNotFound,
        "process_expired" => ToolErrorCode::ProcessExpired,
        "process_exited" | "stdin_failed" => ToolErrorCode::StdinFailed,
        "too_large" => ToolErrorCode::TooLarge,
        "invalid_arguments" => ToolErrorCode::InvalidArguments,
        _ => ToolErrorCode::ProcessFailed,
    };
    ToolFailure::new(code, error.message).into_result(Vec::new())
}

/// 参数必须是 JSON 对象。
fn object_of(arguments_json: &str) -> ToolResultPublic<Value> {
    let trimmed = arguments_json.trim();
    if trimmed.is_empty() {
        return Err(ToolFailure::invalid("arguments must be a JSON object"));
    }
    let value: Value = serde_json::from_str(trimmed)
        .map_err(|error| ToolFailure::invalid(format!("arguments are not valid JSON: {error}")))?;
    if !value.is_object() {
        return Err(ToolFailure::invalid("arguments must be a JSON object"));
    }
    Ok(value)
}

/// 未知字段一律拒绝（错误消息只列字段名，不回显参数全文）。
fn reject_unknown(value: &Value, allowed: &[&str]) -> ToolResultPublic<()> {
    let Some(object) = value.as_object() else {
        return Ok(());
    };
    if let Some(unknown) = object.keys().find(|key| !allowed.contains(&key.as_str())) {
        return Err(ToolFailure::invalid(format!(
            "unknown argument '{unknown}'; allowed: {}",
            allowed.join(", ")
        )));
    }
    Ok(())
}

/// 可选字符串字段（空白视为未提供）。
fn optional_string(value: &Value, key: &str) -> ToolResultPublic<Option<String>> {
    match value.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) if text.trim().is_empty() => Ok(None),
        Some(Value::String(text)) => Ok(Some(text.clone())),
        Some(_) => Err(ToolFailure::invalid(format!("'{key}' must be a string"))),
    }
}

/// 可选布尔字段。
fn optional_bool(value: &Value, key: &str) -> ToolResultPublic<Option<bool>> {
    match value.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(flag)) => Ok(Some(*flag)),
        Some(_) => Err(ToolFailure::invalid(format!("'{key}' must be a boolean"))),
    }
}

/// 可选整数：超出 `[min, max]` 直接拒绝（不静默钳制）。
fn optional_int(value: &Value, key: &str, min: i64, max: i64) -> ToolResultPublic<Option<i64>> {
    match value.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(number)) => {
            let Some(parsed) = number.as_i64() else {
                return Err(ToolFailure::invalid(format!(
                    "'{key}' must be an integer between {min} and {max}"
                )));
            };
            if parsed < min || parsed > max {
                return Err(ToolFailure::invalid(format!(
                    "'{key}' must be between {min} and {max}"
                )));
            }
            Ok(Some(parsed))
        }
        Some(_) => Err(ToolFailure::invalid(format!(
            "'{key}' must be an integer between {min} and {max}"
        ))),
    }
}

#[cfg(test)]
mod tests;

/// `exec_command` 的完整请求：进程参数 + 可选提权申请。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecRequest {
    /// 进程参数。
    pub args: ExecArgs,
    /// 提权申请（默认无）。
    pub escalation: Option<SandboxEscalation>,
}

/// 提权种类。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EscalationKind {
    /// 仅本次命令追加网络/可写根。
    Additional,
    /// 完全绕过沙箱。
    Unsandboxed,
}

/// 模型显式提权申请。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxEscalation {
    /// 提权种类。
    pub kind: EscalationKind,
    /// 面向用户的理由（非空，≤500 字符；不回显给模型）。
    pub justification: String,
    /// 会话级前缀规则（仅 `Unsandboxed`）。
    pub prefix_rule: Option<Vec<String>>,
    /// 本次命令的附加权限（仅 `Additional`）。
    pub additional: Option<AdditionalPermissions>,
}

pub use crate::chat::sandbox::AdditionalPermissions;

/// 解析 `exec_command` 请求（含提权字段）。
pub fn parse_exec_request(arguments_json: &str) -> ToolResultPublic<ExecRequest> {
    let value = object_of(arguments_json)?;
    let allowed = [
        "cmd",
        "workdir",
        "shell",
        "login",
        "tty",
        "yield_time_ms",
        "timeout_ms",
        "max_output_tokens",
        "sandbox_permissions",
        "justification",
        "prefix_rule",
        "additional_permissions",
    ];
    reject_unknown(&value, &allowed)?;
    let args = parse_exec_args_from(&value)?;
    let permissions = optional_string(&value, "sandbox_permissions")?
        .unwrap_or_else(|| "use_default".to_string());
    match permissions.as_str() {
        "use_default" => {
            for key in ["justification", "prefix_rule", "additional_permissions"] {
                if value.get(key).is_some_and(|field| !field.is_null()) {
                    return Err(ToolFailure::invalid(format!(
                        "'{key}' is only valid with an explicit sandbox permission request"
                    )));
                }
            }
            Ok(ExecRequest {
                args,
                escalation: None,
            })
        }
        "require_escalated" => {
            let justification = required_justification(&value)?;
            if value
                .get("additional_permissions")
                .is_some_and(|v| !v.is_null())
            {
                return Err(ToolFailure::invalid(
                    "'additional_permissions' is only valid with 'with_additional_permissions'",
                ));
            }
            let prefix_rule = parse_prefix_rule(&value)?;
            Ok(ExecRequest {
                args,
                escalation: Some(SandboxEscalation {
                    kind: EscalationKind::Unsandboxed,
                    justification,
                    prefix_rule,
                    additional: None,
                }),
            })
        }
        "with_additional_permissions" => {
            let justification = required_justification(&value)?;
            if value.get("prefix_rule").is_some_and(|v| !v.is_null()) {
                return Err(ToolFailure::invalid(
                    "'prefix_rule' is only valid with 'require_escalated'",
                ));
            }
            let Some(additional) = value.get("additional_permissions") else {
                return Err(ToolFailure::invalid(
                    "'additional_permissions' is required with 'with_additional_permissions'",
                ));
            };
            let additional = parse_additional_permissions(additional)?;
            Ok(ExecRequest {
                args,
                escalation: Some(SandboxEscalation {
                    kind: EscalationKind::Additional,
                    justification,
                    prefix_rule: None,
                    additional: Some(additional),
                }),
            })
        }
        other => Err(ToolFailure::invalid(format!(
            "'sandbox_permissions' must be use_default, with_additional_permissions, or require_escalated (got '{other}')"
        ))),
    }
}

fn parse_exec_args_from(value: &Value) -> ToolResultPublic<ExecArgs> {
    // 复用同一套字段校验：构造只含进程字段的对象，避免两套默认值漂移。
    let mut process_only = serde_json::Map::new();
    for key in [
        "cmd",
        "workdir",
        "shell",
        "login",
        "tty",
        "yield_time_ms",
        "timeout_ms",
        "max_output_tokens",
    ] {
        if let Some(field) = value.get(key) {
            process_only.insert(key.to_string(), field.clone());
        }
    }
    parse_exec_args(&Value::Object(process_only).to_string())
}

fn required_justification(value: &Value) -> ToolResultPublic<String> {
    let text = value
        .get("justification")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .ok_or_else(|| ToolFailure::invalid("'justification' must be a non-empty string"))?;
    if text.chars().count() > 500 {
        return Err(ToolFailure::invalid(
            "'justification' must be at most 500 characters",
        ));
    }
    if text.chars().any(char::is_control) {
        return Err(ToolFailure::invalid(
            "'justification' must not contain control characters",
        ));
    }
    Ok(text.to_string())
}

fn parse_prefix_rule(value: &Value) -> ToolResultPublic<Option<Vec<String>>> {
    let Some(raw) = value.get("prefix_rule") else {
        return Ok(None);
    };
    if raw.is_null() {
        return Ok(None);
    }
    let Some(items) = raw.as_array() else {
        return Err(ToolFailure::invalid(
            "'prefix_rule' must be an array of strings",
        ));
    };
    if items.is_empty() || items.len() > 6 {
        return Err(ToolFailure::invalid(
            "'prefix_rule' must contain between 1 and 6 tokens",
        ));
    }
    let mut tokens = Vec::with_capacity(items.len());
    for item in items {
        let Some(token) = item.as_str() else {
            return Err(ToolFailure::invalid(
                "'prefix_rule' must contain only strings",
            ));
        };
        if token.is_empty() || token.chars().any(char::is_control) {
            return Err(ToolFailure::invalid(
                "'prefix_rule' tokens must be non-empty without control characters",
            ));
        }
        if token.chars().any(|ch| {
            matches!(
                ch,
                '|' | ';' | '&' | '$' | '`' | '(' | ')' | '{' | '}' | '<' | '>'
            )
        }) {
            return Err(ToolFailure::invalid(
                "'prefix_rule' tokens must not contain shell metacharacters",
            ));
        }
        tokens.push(token.to_string());
    }
    if banned_prefix(&tokens) {
        return Err(ToolFailure::invalid(
            "'prefix_rule' must not name a shell, interpreter, git, package runner, rm, or sudo",
        ));
    }
    Ok(Some(tokens))
}

fn parse_additional_permissions(raw: &Value) -> ToolResultPublic<AdditionalPermissions> {
    let Some(object) = raw.as_object() else {
        return Err(ToolFailure::invalid(
            "'additional_permissions' must be an object",
        ));
    };
    if let Some(unknown) = object
        .keys()
        .find(|key| !matches!(key.as_str(), "network" | "file_system"))
    {
        return Err(ToolFailure::invalid(format!(
            "unknown additional permission '{unknown}'; allowed: network, file_system"
        )));
    }
    let mut permissions = AdditionalPermissions::default();
    if let Some(network) = object.get("network") {
        let Some(network) = network.as_object() else {
            return Err(ToolFailure::invalid("'network' must be an object"));
        };
        if let Some(unknown) = network.keys().find(|key| key.as_str() != "enabled") {
            return Err(ToolFailure::invalid(format!(
                "unknown network permission '{unknown}'"
            )));
        }
        let enabled = network
            .get("enabled")
            .map(|value| {
                value
                    .as_bool()
                    .ok_or_else(|| ToolFailure::invalid("'network.enabled' must be a boolean"))
            })
            .transpose()?
            .unwrap_or(false);
        permissions.network = Some(enabled);
    }
    if let Some(file_system) = object.get("file_system") {
        let Some(file_system) = file_system.as_object() else {
            return Err(ToolFailure::invalid("'file_system' must be an object"));
        };
        if let Some(unknown) = file_system
            .keys()
            .find(|key| !matches!(key.as_str(), "read" | "write" | "deny"))
        {
            return Err(ToolFailure::invalid(format!(
                "unknown file_system permission '{unknown}'; allowed: read, write, deny"
            )));
        }
        let mut request = crate::chat::sandbox::FileSystemRequest::default();
        for (key, target) in [
            ("read", &mut request.read),
            ("write", &mut request.write),
            ("deny", &mut request.deny),
        ] {
            let Some(items) = file_system.get(key) else {
                continue;
            };
            let Some(items) = items.as_array() else {
                return Err(ToolFailure::invalid(format!(
                    "'file_system.{key}' must be an array"
                )));
            };
            for item in items {
                let Some(raw) = item.as_str() else {
                    return Err(ToolFailure::invalid(format!(
                        "'file_system.{key}' must contain only strings"
                    )));
                };
                target.push(raw.to_string());
            }
        }
        request
            .validate()
            .map_err(|error| ToolFailure::invalid(error.to_string()))?;
        permissions.file_system = Some(request);
    }
    let grants = permissions.network == Some(true)
        || permissions
            .file_system
            .as_ref()
            .is_some_and(|request| !request.is_empty());
    if !grants {
        return Err(ToolFailure::invalid(
            "'additional_permissions' must request network or at least one file system entry",
        ));
    }
    Ok(permissions)
}

fn banned_prefix(tokens: &[String]) -> bool {
    const BANNED: &[&[&str]] = &[
        &["bash"],
        &["bash", "-c"],
        &["bash", "-lc"],
        &["sh"],
        &["sh", "-c"],
        &["sh", "-lc"],
        &["zsh"],
        &["zsh", "-c"],
        &["zsh", "-lc"],
        &["dash"],
        &["dash", "-c"],
        &["fish"],
        &["fish", "-c"],
        &["ksh"],
        &["ksh", "-c"],
        &["cmd"],
        &["cmd.exe"],
        &["powershell"],
        &["pwsh"],
        &["node"],
        &["node", "-e"],
        &["python"],
        &["python3"],
        &["py"],
        &["ruby"],
        &["perl"],
        &["php"],
        &["lua"],
        &["deno"],
        &["bun"],
        &["julia"],
        &["Rscript"],
        &["git"],
        &["npm", "run"],
        &["pnpm", "run"],
        &["yarn", "run"],
        &["env"],
        &["sudo"],
        &["rm"],
    ];
    BANNED.iter().any(|prefix| {
        tokens.len() >= prefix.len()
            && tokens
                .iter()
                .zip(*prefix)
                .all(|(token, expected)| token.as_str() == *expected)
    })
}
