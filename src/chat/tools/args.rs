//! 工具参数解析：严格白名单字段校验，不信任模型遵守 JSON Schema。
//!
//! 只接受 `{"key": value, ...}` 对象；未知字段、类型错误、越界整数都返回
//! `InvalidArguments`，并且**不产生任何文件 IO**。错误消息只包含字段名与期望类型，
//! 绝不回显完整参数（可能含大块内容或敏感文本）。

use serde_json::Value;

use super::types::{ToolErrorCode, ToolFailure, ToolResult};

/// 已校验参数的可变视图：按字段名取值，取不到时报错。
#[derive(Debug, Clone)]
pub(crate) struct Args {
    object: serde_json::Map<String, Value>,
}

impl Args {
    /// 解析并校验字段白名单。
    pub(crate) fn parse(arguments_json: &str, allowed: &[&str]) -> ToolResult<Self> {
        let trimmed = arguments_json.trim();
        if trimmed.is_empty() {
            return Err(ToolFailure::invalid("arguments must be a JSON object"));
        }
        let value: Value = serde_json::from_str(trimmed).map_err(|error| {
            ToolFailure::invalid(format!("arguments are not valid JSON: {error}"))
        })?;
        let Value::Object(object) = value else {
            return Err(ToolFailure::invalid("arguments must be a JSON object"));
        };
        if let Some(unknown) = object.keys().find(|key| !allowed.contains(&key.as_str())) {
            return Err(ToolFailure::invalid(format!(
                "unknown argument '{unknown}'; allowed: {}",
                allowed.join(", ")
            )));
        }
        Ok(Self { object })
    }

    /// 必填字符串字段；类型不符或为空白时报错。
    pub(crate) fn required_str(&self, key: &str) -> ToolResult<&str> {
        match self.object.get(key) {
            Some(Value::String(text)) if !text.trim().is_empty() => Ok(text),
            Some(Value::String(_)) => {
                Err(ToolFailure::invalid(format!("'{key}' must not be empty")))
            }
            Some(_) => Err(ToolFailure::invalid(format!("'{key}' must be a string"))),
            None => Err(ToolFailure::invalid(format!(
                "missing required argument '{key}'"
            ))),
        }
    }

    /// 必填字符串字段，允许空字符串（如 `write` 的清空内容）。
    pub(crate) fn required_text(&self, key: &str) -> ToolResult<&str> {
        match self.object.get(key) {
            Some(Value::String(text)) => Ok(text),
            Some(_) => Err(ToolFailure::invalid(format!("'{key}' must be a string"))),
            None => Err(ToolFailure::invalid(format!(
                "missing required argument '{key}'"
            ))),
        }
    }

    /// 可选字符串字段；空白字符串视为未提供。
    pub(crate) fn optional_str(&self, key: &str) -> ToolResult<Option<&str>> {
        match self.object.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(text)) if text.trim().is_empty() => Ok(None),
            Some(Value::String(text)) => Ok(Some(text)),
            Some(_) => Err(ToolFailure::invalid(format!("'{key}' must be a string"))),
        }
    }

    /// 可选布尔字段；类型不符时报错。
    pub(crate) fn optional_bool(&self, key: &str) -> ToolResult<Option<bool>> {
        match self.object.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::Bool(flag)) => Ok(Some(*flag)),
            Some(_) => Err(ToolFailure::invalid(format!("'{key}' must be a boolean"))),
        }
    }

    /// 可选字符串映射（值必须是字符串）；用于 `read_versions` 这类路径 → token 表。
    ///
    /// 键与值都只做类型校验，语义由调用方解释；空对象等价于未提供。
    pub(crate) fn optional_string_map(
        &self,
        key: &str,
    ) -> ToolResult<Option<std::collections::HashMap<String, String>>> {
        match self.object.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::Object(map)) => {
                let mut out = std::collections::HashMap::with_capacity(map.len());
                for (entry_key, value) in map {
                    let Some(text) = value.as_str() else {
                        return Err(ToolFailure::invalid(format!(
                            "'{key}.{entry_key}' must be a string"
                        )));
                    };
                    out.insert(entry_key.clone(), text.to_string());
                }
                Ok((!out.is_empty()).then_some(out))
            }
            Some(_) => Err(ToolFailure::invalid(format!(
                "'{key}' must be an object of string values"
            ))),
        }
    }

    /// 可选正整数（≥1）字段；0、负数、浮点与小数字符串一律拒绝。
    pub(crate) fn optional_positive(&self, key: &str) -> ToolResult<Option<usize>> {
        match self.object.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::Number(number)) => {
                let Some(value) = number.as_u64() else {
                    return Err(ToolFailure::invalid(format!(
                        "'{key}' must be a positive integer"
                    )));
                };
                if value == 0 {
                    return Err(ToolFailure::invalid(format!("'{key}' must be >= 1")));
                }
                usize::try_from(value).map(Some).map_err(|_| {
                    ToolFailure::new(
                        ToolErrorCode::TooLarge,
                        format!("'{key}' is too large for this platform"),
                    )
                })
            }
            Some(_) => Err(ToolFailure::invalid(format!(
                "'{key}' must be a positive integer"
            ))),
        }
    }
}

/// 把正整数参数钳到上限，返回原始值与被钳后的值。
pub(crate) fn clamp_limit(requested: Option<usize>, max: usize) -> usize {
    match requested {
        Some(value) => value.min(max),
        None => max,
    }
}

#[cfg(test)]
mod tests;
