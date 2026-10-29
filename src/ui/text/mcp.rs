//! MCP 用户可见文案（中文）。

/// 首轮启动进度（lx 活动标签）。
pub fn startup_label(done: usize, total: usize) -> String {
    format!("正在启动 MCP 服务 ({done}/{total})…")
}

/// server 启动失败 toast。
pub fn start_failed(name: &str, error: &str) -> String {
    format!("MCP 服务 \"{name}\" 启动失败：{error}")
}

/// OAuth 未登录 toast。
pub fn needs_login(name: &str) -> String {
    format!("MCP 服务 \"{name}\" 需要登录，请在设置页 MCP 中登录")
}

/// 卡片展开区的参数标题。
pub const DETAIL_ARGUMENTS: &str = "Arguments";
/// 卡片展开区的结果标题。
pub const DETAIL_RESULT: &str = "Result";
/// 图片占位行前缀。
pub const IMAGE_LINE_PREFIX: &str = "[image";
/// 结果被截断的标题状态。
pub const RESULT_TRUNCATED: &str = "truncated";
/// 卡片标题里的图片张数。
pub fn images_label(count: usize) -> String {
    format!("{count} image(s)")
}
/// 展开区的图片占位行。
pub fn image_line(mime: &str, width: u32, height: u32, path: &str) -> String {
    format!("[image {mime} {width}×{height}] {path}")
}
/// 超出 20 行时的提示。
pub fn more_lines(count: usize) -> String {
    format!("… {count} more line(s)")
}
