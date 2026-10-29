//! Skills 用户可见文案（中文）。

/// 目录超出 8000 字符预算时省略了若干 skill。
pub fn catalog_omitted(count: usize) -> String {
    format!("可用 skill 过多，{count} 个未列入模型目录")
}

/// 注入正文被截断。
pub fn truncated(name: &str) -> String {
    format!("skill \"{name}\" 超过 8000 字节，已截断注入")
}

/// 注入前读取失败。
pub fn read_failed(name: &str, reason: &str) -> String {
    format!("skill \"{name}\" 读取失败：{reason}")
}
