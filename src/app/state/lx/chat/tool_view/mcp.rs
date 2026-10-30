//! MCP 工具卡片详情：参数与结果的有限行窗口（纯数据，渲染层只读）。

use serde_json::Value;

/// MCP 工具结果里的已归档图片（只展示元数据，不读字节）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpImageView {
    pub path: String,
    pub mime: String,
    pub width: u32,
    pub height: u32,
}

/// MCP 工具卡片详情：参数与结果的有限行窗口。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpDetailView {
    pub server: String,
    pub tool: String,
    /// 历史参数摘要（保留供旧数据/兼容；卡片不再在标题下重复展示）。
    pub summary: Option<String>,
    /// 格式化后的参数行（最多 20 行）。
    pub arguments: Vec<String>,
    pub arguments_more: usize,
    /// 结果文本行（最多 20 行）。
    pub result: Vec<String>,
    pub result_more: usize,
    pub images: Vec<McpImageView>,
    pub images_omitted: usize,
    /// 调用耗时（毫秒）；旧记录可能缺失。
    pub duration_ms: Option<u64>,
}

impl McpDetailView {
    /// 只由工具名与参数构造：工具处于 Pending/Running 或失败/取消无 `"kind":"mcp"` 结果体时使用。
    pub fn from_arguments(name: &str, arguments_json: &str) -> Option<Self> {
        let (server, tool) = if let Some(rest) = name.strip_prefix(crate::mcp::MCP_PREFIX) {
            let (server, tool) = rest.split_once(crate::mcp::MCP_DELIMITER)?;
            if server.is_empty() || tool.is_empty() {
                return None;
            }
            (server.to_string(), tool.to_string())
        } else if matches!(
            name,
            "list_resources" | "list_resource_templates" | "read_resource"
        ) {
            let server = serde_json::from_str::<Value>(arguments_json)
                .ok()
                .and_then(|value| {
                    value
                        .get("server")
                        .and_then(Value::as_str)
                        .filter(|s| !s.is_empty())
                        .map(str::to_string)
                })
                .unwrap_or_else(|| "all servers".to_string());
            (server, name.to_string())
        } else {
            return None;
        };
        Some(Self {
            server,
            tool,
            summary: argument_summary_of(arguments_json),
            arguments: argument_lines(arguments_json),
            arguments_more: argument_more(arguments_json),
            result: Vec::new(),
            result_more: 0,
            images: Vec::new(),
            images_omitted: 0,
            duration_ms: None,
        })
    }

    /// 由工具名 +（可选）落库结果 JSON + 参数 JSON 构造（历史重载与结算兜底共用）。
    pub fn from_tool(name: &str, result_json: Option<&str>, arguments_json: &str) -> Option<Self> {
        Self::from_parts(result_json, arguments_json)
            .or_else(|| Self::from_arguments(name, arguments_json))
    }

    /// 历史重载：结果体（落库 JSON）+ 调用参数。
    pub fn from_parts(result_json: Option<&str>, arguments_json: &str) -> Option<Self> {
        let value: Value = serde_json::from_str(result_json?).ok()?;
        if value.get("kind").and_then(Value::as_str)? != "mcp" {
            return None;
        }
        let server = value.get("server")?.as_str()?.to_string();
        let tool = value.get("tool")?.as_str()?.to_string();
        let text = value.get("text").and_then(Value::as_str).unwrap_or("");
        let images = value
            .get("images")
            .and_then(Value::as_array)
            .map(|images| {
                images
                    .iter()
                    .filter_map(|image| {
                        Some(McpImageView {
                            path: image.get("path")?.as_str()?.to_string(),
                            mime: image
                                .get("mime")
                                .and_then(Value::as_str)
                                .unwrap_or("image/png")
                                .to_string(),
                            width: image.get("width")?.as_u64().unwrap_or(0) as u32,
                            height: image.get("height")?.as_u64().unwrap_or(0) as u32,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        Some(Self {
            server,
            tool,
            summary: argument_summary_of(arguments_json),
            arguments: argument_lines(arguments_json),
            arguments_more: argument_more(arguments_json),
            result: text_lines(text),
            result_more: text_more(text),
            images,
            images_omitted: value
                .get("images_omitted")
                .and_then(Value::as_u64)
                .unwrap_or(0) as usize,
            duration_ms: value.get("duration_ms").and_then(Value::as_u64),
        })
    }

    /// 实时结算：直接由执行结果构造（与落库 JSON 同一口径）。
    pub fn from_result(result: &crate::chat::tools::McpToolResult, arguments_json: &str) -> Self {
        Self {
            server: result.server.clone(),
            tool: result.tool.clone(),
            summary: argument_summary_of(arguments_json),
            arguments: argument_lines(arguments_json),
            arguments_more: argument_more(arguments_json),
            result: text_lines(&result.text),
            result_more: text_more(&result.text),
            images: result
                .images
                .iter()
                .map(|image| McpImageView {
                    path: image.path.display().to_string(),
                    mime: image.mime.clone(),
                    width: image.width,
                    height: image.height,
                })
                .collect(),
            images_omitted: result.images_omitted,
            duration_ms: result.duration_ms,
        }
    }

    /// 折叠标题里的参数摘要。
    pub fn argument_summary(&self) -> Option<String> {
        self.summary.clone()
    }
}

/// 第一个字符串参数（单行、最多 60 字符）。
fn argument_summary_of(arguments_json: &str) -> Option<String> {
    let value: Value = serde_json::from_str(arguments_json).ok()?;
    let object = value.as_object()?;
    for value in object.values() {
        let Some(text) = value.as_str() else {
            continue;
        };
        let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
        if !text.is_empty() {
            return Some(text.chars().take(60).collect());
        }
    }
    None
}

/// 参数 JSON 格式化行（最多 20 行）。
fn argument_lines(arguments_json: &str) -> Vec<String> {
    let value: Value = serde_json::from_str(arguments_json).unwrap_or(Value::Null);
    let text = if value.is_null() {
        arguments_json.to_string()
    } else {
        serde_json::to_string_pretty(&value).unwrap_or_else(|_| arguments_json.to_string())
    };
    text.lines().take(20).map(str::to_string).collect()
}

fn argument_more(arguments_json: &str) -> usize {
    let value: Value = serde_json::from_str(arguments_json).unwrap_or(Value::Null);
    let text = if value.is_null() {
        arguments_json.to_string()
    } else {
        serde_json::to_string_pretty(&value).unwrap_or_else(|_| arguments_json.to_string())
    };
    text.lines().count().saturating_sub(20)
}

fn text_lines(text: &str) -> Vec<String> {
    text.lines().take(20).map(str::to_string).collect()
}

fn text_more(text: &str) -> usize {
    text.lines().count().saturating_sub(20)
}
