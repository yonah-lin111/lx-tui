//! MCP user-visible copy.

/// server 启动失败 toast。
pub fn start_failed(name: &str, error: &str) -> String {
    format!("MCP server \"{name}\" failed to start: {error}")
}

/// OAuth 未登录 toast。
pub fn needs_login(name: &str) -> String {
    format!("MCP server \"{name}\" needs login; open Settings → MCP")
}

/// 卡片标题里的图片张数。
pub fn images_label(count: usize) -> String {
    format!("{count} image(s)")
}
