//! MCP 客户端：连接管理、工具命名、schema 清洗与结果转换。
//!
//! 唯一引入 `rmcp` 的模块；不依赖 `chat`/`app`/`ui`/`event`/`settings`。

pub mod manager;
pub mod naming;
pub mod oauth;
pub mod result;
pub mod schema;

pub use manager::{
    McpEvent, McpManager, McpToolSnapshot, ServerSnapshot, ServerStatus, manager as global_manager,
};
pub use naming::{MAX_TOOL_NAME_LEN, MCP_DELIMITER, MCP_PREFIX, McpToolEntry};
pub use result::{
    ArchivedImage, MAX_RESULT_IMAGES, MAX_TOOL_RESULT_BYTES, McpCallOutcome, McpFailureKind,
};
pub use schema::{MAX_SCHEMA_BYTES, SCHEMA_OMITTED_NOTE, sanitize_json_schema, trim_for_gemini};

/// 该工具名是否属于 MCP 工具。
pub fn is_mcp_tool(name: &str) -> bool {
    name.starts_with(MCP_PREFIX)
}
