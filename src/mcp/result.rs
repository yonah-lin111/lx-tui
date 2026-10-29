//! MCP 工具结果转换：文本拼接、图片归档、占位与截断。

use std::path::PathBuf;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use rmcp::model::ContentBlock;

use crate::platform::{archive_processed_image, process_image_bytes};

/// 工具结果文本上限（64 KiB）。
pub const MAX_TOOL_RESULT_BYTES: usize = 64 * 1024;
/// 单次结果最多发给模型的图片数。
pub const MAX_RESULT_IMAGES: usize = 4;

/// 已归档的工具图片（不含字节；字节在组装请求时按路径加载）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchivedImage {
    pub path: PathBuf,
    pub mime: String,
    pub width: u32,
    pub height: u32,
}

/// MCP 调用结果。
#[derive(Debug, Clone, PartialEq)]
pub enum McpCallOutcome {
    Success {
        text: String,
        images: Vec<ArchivedImage>,
        images_omitted: usize,
        truncated: bool,
    },
    Failure {
        kind: McpFailureKind,
        text: String,
    },
    Cancelled,
}

/// 失败分类（由执行层映射为稳定工具错误码）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpFailureKind {
    /// server 返回 `isError: true`。
    ToolError,
    /// 调用超时。
    Timeout,
    /// OAuth token 缺失或刷新失败。
    AuthRequired,
    /// 快照里的工具已被移除 / server 已停。
    ToolMissing,
    /// 传输失败、server 断开或协议错误。
    Server,
    /// 参数不是 JSON 对象 / 缺少必填参数。
    InvalidArguments,
}

/// 把 `CallToolResult` 转成结果；图片立即归档到会话目录。
pub fn convert_result(result: &rmcp::model::CallToolResult, session_id: &str) -> McpCallOutcome {
    let mut text_parts: Vec<String> = Vec::new();
    if let Some(structured) = &result.structured_content
        && !structured.is_null()
    {
        text_parts
            .push(serde_json::to_string(structured).unwrap_or_else(|_| structured.to_string()));
    }
    let mut images = Vec::new();
    let mut omitted = 0usize;
    for block in &result.content {
        match block {
            ContentBlock::Text(content) => text_parts.push(content.text.clone()),
            ContentBlock::Image(image) => {
                if images.len() >= MAX_RESULT_IMAGES {
                    omitted += 1;
                    continue;
                }
                match archive_image(&image.data, &image.mime_type, session_id) {
                    Ok(image) => images.push(image),
                    Err(error) => {
                        omitted += 1;
                        tracing::warn!(target: "lx::mcp", %error, "tool image dropped");
                    }
                }
            }
            ContentBlock::Audio(audio) => {
                let bytes = decoded_len(&audio.data);
                text_parts.push(format!(
                    "[audio {}, {bytes} bytes omitted]",
                    audio.mime_type
                ));
            }
            ContentBlock::Resource(resource) => match &resource.resource {
                rmcp::model::ResourceContents::TextResourceContents {
                    uri,
                    text,
                    mime_type,
                    ..
                } => {
                    let _ = mime_type;
                    text_parts.push(format!("{uri}:\n{text}"));
                }
                rmcp::model::ResourceContents::BlobResourceContents {
                    uri,
                    blob,
                    mime_type,
                    ..
                } => {
                    let mime = mime_type
                        .clone()
                        .unwrap_or_else(|| "application/octet-stream".into());
                    if mime.starts_with("image/") {
                        if images.len() >= MAX_RESULT_IMAGES {
                            omitted += 1;
                        } else {
                            match archive_image(blob, &mime, session_id) {
                                Ok(image) => images.push(image),
                                Err(error) => {
                                    omitted += 1;
                                    tracing::warn!(target: "lx::mcp", %error, "resource image dropped");
                                }
                            }
                        }
                    } else {
                        let bytes = decoded_len(blob);
                        text_parts.push(format!("[resource {uri} {mime}, {bytes} bytes omitted]"));
                    }
                }
                _ => {}
            },
            ContentBlock::ResourceLink(resource) => {
                text_parts.push(format!(
                    "[resource link {}: {}]",
                    resource.name, resource.uri
                ));
            }
            other => text_parts.push(
                serde_json::to_string(other).unwrap_or_else(|_| "[unsupported content]".into()),
            ),
        }
    }
    if omitted > 0 {
        text_parts.push(format!("[{omitted} more image(s) omitted]"));
    }
    let joined = text_parts
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    if result.is_error.unwrap_or(false) {
        let text = if joined.trim().is_empty() {
            "MCP tool returned an error".to_string()
        } else {
            joined
        };
        return McpCallOutcome::Failure {
            kind: McpFailureKind::ToolError,
            text,
        };
    }
    let (text, truncated) = truncate_text(&joined, MAX_TOOL_RESULT_BYTES);
    McpCallOutcome::Success {
        text,
        images,
        images_omitted: omitted,
        truncated,
    }
}

pub(crate) fn archive_image(
    data: &str,
    mime: &str,
    session_id: &str,
) -> Result<ArchivedImage, String> {
    let bytes = BASE64
        .decode(data)
        .map_err(|error| format!("invalid base64: {error}"))?;
    let processed = process_image_bytes(&bytes, &format!("tool.{}", extension(mime)), None)
        .map_err(|error| error.code().to_string())?;
    let archived = archive_processed_image(session_id, &processed)
        .map_err(|error| error.code().to_string())?;
    Ok(ArchivedImage {
        path: archived.archive_path,
        mime: archived.mime,
        width: archived.width,
        height: archived.height,
    })
}

fn extension(mime: &str) -> &'static str {
    match mime {
        "image/jpeg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        _ => "png",
    }
}

pub(crate) fn decoded_len(data: &str) -> usize {
    BASE64.decode(data).map(|bytes| bytes.len()).unwrap_or(0)
}

/// 截断到 UTF-8 字符边界并显式标记。
pub fn truncate_text(text: &str, max_bytes: usize) -> (String, bool) {
    if text.len() <= max_bytes {
        return (text.to_string(), false);
    }
    let mut end = max_bytes;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    (
        format!("{}\n[truncated at {max_bytes} bytes]", &text[..end]),
        true,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::model::CallToolResult;

    fn result(json: serde_json::Value) -> CallToolResult {
        serde_json::from_value(json).expect("call tool result")
    }

    #[test]
    fn text_blocks_are_joined() {
        let result = result(serde_json::json!({
            "content": [{"type": "text", "text": "hello"}, {"type": "text", "text": "world"}]
        }));
        match convert_result(&result, "sess-test") {
            McpCallOutcome::Success { text, .. } => assert_eq!(text, "hello\nworld"),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn structured_content_is_kept_first() {
        let result = result(serde_json::json!({
            "content": [{"type": "text", "text": "raw"}],
            "structuredContent": {"answer": 42}
        }));
        match convert_result(&result, "sess-test") {
            McpCallOutcome::Success { text, .. } => {
                assert!(text.starts_with("{\"answer\":42}"));
                assert!(text.contains("raw"));
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn is_error_becomes_tool_failure_with_fallback_text() {
        let result = result(serde_json::json!({"content": [], "isError": true}));
        assert_eq!(
            convert_result(&result, "sess-test"),
            McpCallOutcome::Failure {
                kind: McpFailureKind::ToolError,
                text: "MCP tool returned an error".into()
            }
        );
    }

    #[test]
    fn audio_gets_a_placeholder() {
        let result = result(serde_json::json!({
            "content": [{"type": "audio", "data": "aGVsbG8=", "mimeType": "audio/wav"}]
        }));
        match convert_result(&result, "sess-test") {
            McpCallOutcome::Success { text, .. } => assert!(text.contains("audio/wav")),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn truncation_marks_the_result() {
        let (text, truncated) = truncate_text(&"a".repeat(100), 10);
        assert!(truncated);
        assert!(text.contains("truncated at 10 bytes"));
    }
}
