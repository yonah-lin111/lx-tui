//! Skills user-visible copy.

/// 目录超出 8000 字符预算时省略了若干 skill。
pub fn catalog_omitted(count: usize) -> String {
    format!("too many skills; {count} omitted from the model catalog")
}

/// 注入正文被截断。
pub fn truncated(name: &str) -> String {
    format!("skill \"{name}\" exceeds 8000 bytes; injection truncated")
}

/// 注入前读取失败。
pub fn read_failed(name: &str, reason: &str) -> String {
    format!("failed to read skill \"{name}\": {reason}")
}
