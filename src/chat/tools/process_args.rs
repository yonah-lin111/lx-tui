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
