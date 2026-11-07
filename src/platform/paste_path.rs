//! 粘贴路径解析：单行/多行绝对路径与 `file://` URI，以及图片文件签名探测。
//!
//! 纯解析放在这里，核心模块不出现 `cfg(target_os)`；引用标记的格式与解析在 `app/markdown/`。

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// 解析当前用户家目录：优先 `HOME`，回退 Windows `USERPROFILE` 及 `HOMEDRIVE` + `HOMEPATH`。
pub fn home_dir() -> Option<PathBuf> {
    home_dir_from_vars(
        std::env::var_os("HOME"),
        std::env::var_os("USERPROFILE"),
        std::env::var_os("HOMEDRIVE"),
        std::env::var_os("HOMEPATH"),
    )
}

/// `home_dir` 的纯函数版本：空变量视为缺失，便于跨平台单测。
pub(crate) fn home_dir_from_vars(
    home: Option<OsString>,
    userprofile: Option<OsString>,
    homedrive: Option<OsString>,
    homepath: Option<OsString>,
) -> Option<PathBuf> {
    if let Some(dir) = home.filter(|value| !value.is_empty()) {
        return Some(PathBuf::from(dir));
    }
    if let Some(dir) = userprofile.filter(|value| !value.is_empty()) {
        return Some(PathBuf::from(dir));
    }
    match (
        homedrive.filter(|value| !value.is_empty()),
        homepath.filter(|value| !value.is_empty()),
    ) {
        (Some(drive), Some(path)) => {
            let mut combined = drive;
            combined.push(path);
            Some(PathBuf::from(combined))
        }
        _ => None,
    }
}

/// 展开开头为 `~` 的路径：`~` 与 `~/...`（Windows 另支持 `~\...`）。
///
/// 外层 `None` 表示不是家目录形式；内层 `Err(())` 表示无法解析家目录。
pub fn expand_tilde_path(input: &str) -> Option<Result<PathBuf, ()>> {
    if input == "~" {
        return Some(home_dir().ok_or(()));
    }
    if let Some(rest) = input.strip_prefix("~/") {
        return Some(home_dir().map(|home| home.join(rest)).ok_or(()));
    }
    #[cfg(windows)]
    if let Some(rest) = input.strip_prefix("~\\") {
        return Some(home_dir().map(|home| home.join(rest)).ok_or(()));
    }
    None
}

/// 粘贴文本中的路径 → 实际存在的路径：优先原文；否则按跨平台粘贴规则归一化后再试。
///
/// 终端拖拽/部分复制会带 shell 转义的反斜杠（`ai_agent_api\\(1\\).md`）、外层引号或
/// 拖拽尾随空格；原文不是合法文件时归一化再判定，避免把转义文本当路径插进正文。
pub fn resolve_pasted_path(path: &Path) -> Option<PathBuf> {
    if path.exists() {
        return Some(path.to_path_buf());
    }
    let text = path.to_str()?;
    let normalized = normalize_pasted_path_input(text);
    if normalized != text && Path::new(&normalized).exists() {
        return Some(PathBuf::from(normalized));
    }
    // 原文含反斜杠但归一化未命中（例如多行拼接）时，保留一次纯去转义兜底。
    if text.contains('\\') {
        let unescaped = unescape_backslashes(text);
        let candidate = Path::new(&unescaped);
        if candidate.exists() {
            return Some(candidate.to_path_buf());
        }
    }
    None
}

/// 把粘贴/键入的单个路径归一化为可直接使用的文本。
///
/// 兼容 macOS / Linux / Windows 的终端与文件管理器：去掉首尾噪声（含拖拽附加空格）、
/// 剥离单双引号、解码 `file://` URI，并把 shell 反斜杠转义还原成真实字符；
/// Windows 原生反斜杠路径保持分隔符不变。返回空串表示没有有效内容。
pub fn normalize_pasted_path_input(raw: &str) -> String {
    let line = raw.trim_start().trim_end_matches(['\r', '\n']);
    // 已是磁盘上真实存在的原样路径（例如文件名本身带尾随空格）时不再裁剪，
    // 保证多次归一化结果稳定。
    if !line.is_empty()
        && !line.starts_with('\'')
        && !line.starts_with('"')
        && !line.starts_with("file://")
        && !line.contains('\\')
        && Path::new(line).exists()
    {
        return line.to_string();
    }
    let trimmed = trim_trailing_unescaped_spaces(line);
    if trimmed.is_empty() {
        return String::new();
    }
    if trimmed.len() >= 2 && trimmed.starts_with('\'') && trimmed.ends_with('\'') {
        let inner = trimmed[1..trimmed.len() - 1].replace("'\\''", "'");
        return decode_file_uri_or(inner);
    }
    if trimmed.len() >= 2 && trimmed.starts_with('"') && trimmed.ends_with('"') {
        let inner = &trimmed[1..trimmed.len() - 1];
        let text = unescape_double_quoted(inner, is_windows_backslash_path(inner));
        return decode_file_uri_or(text);
    }
    if let Some(rest) = trimmed.strip_prefix("file://")
        && let Some(path) = parse_file_uri(rest)
    {
        return path.to_string_lossy().into_owned();
    }
    if !is_windows_backslash_path(trimmed) && trimmed.contains('\\') {
        // 文件名真含反斜杠（Unix 允许）时优先原文，否则按 shell 转义还原。
        if Path::new(trimmed).exists() {
            return trimmed.to_string();
        }
        return unescape_backslashes(trimmed);
    }
    trimmed.to_string()
}

/// 去掉末尾未被转义的空格/制表符（终端拖拽路径常附带一个尾随空格）。
///
/// 空格前有奇数个反斜杠表示该空格被 shell 转义（`test\ `），必须保留。
fn trim_trailing_unescaped_spaces(text: &str) -> &str {
    let bytes = text.as_bytes();
    let mut end = bytes.len();
    while end > 0 && matches!(bytes[end - 1], b' ' | b'\t') {
        let mut backslashes = 0_usize;
        let mut index = end - 1;
        while index > 0 && bytes[index - 1] == b'\\' {
            backslashes += 1;
            index -= 1;
        }
        if !backslashes.is_multiple_of(2) {
            break;
        }
        end -= 1;
    }
    &text[..end]
}

/// 是否是 Windows 原生反斜杠路径：盘符 `C:\...` 或 UNC `\\server\...`。
fn is_windows_backslash_path(text: &str) -> bool {
    let bytes = text.as_bytes();
    if bytes.starts_with(b"\\\\") {
        return true;
    }
    bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'\\'
}

/// 双引号内容去转义：`\"` → `"`；`keep_backslashes=false` 时 `\\` → `\`。
fn unescape_double_quoted(text: &str, keep_backslashes: bool) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match chars.peek().copied() {
            Some('"') => {
                chars.next();
                out.push('"');
            }
            Some('\\') if !keep_backslashes => {
                chars.next();
                out.push('\\');
            }
            _ => out.push('\\'),
        }
    }
    out
}

/// 引号内文本若是 `file://` URI 则解码，否则原样返回。
fn decode_file_uri_or(text: String) -> String {
    if let Some(rest) = text.strip_prefix("file://")
        && let Some(path) = parse_file_uri(rest)
    {
        return path.to_string_lossy().into_owned();
    }
    text
}

/// shell 风格反斜杠去转义：`\\(` → `(`、`\\ ` → ` `、`\\\\` → `\\`；末尾孤立反斜杠保留。
fn unescape_backslashes(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match chars.next() {
            Some(next) => out.push(next),
            None => out.push('\\'),
        }
    }
    out
}

/// 图片文件签名探测：读文件头，读不到或不认识按非图片处理。
pub fn sniff_image_file(path: &Path) -> bool {
    use std::io::Read;
    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    let mut header = [0_u8; 16];
    let read = file.read(&mut header).unwrap_or(0);
    is_image(&header[..read])
}

/// 图片头部签名：PNG/JPEG/GIF/WebP/BMP/TIFF。
fn is_image(bytes: &[u8]) -> bool {
    const PREFIXES: &[&[u8]] = &[
        &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A],
        &[0xFF, 0xD8, 0xFF],
        b"GIF87a",
        b"GIF89a",
        b"BM",
        b"II\x2a\x00",
        b"MM\x00\x2a",
    ];
    PREFIXES.iter().any(|prefix| bytes.starts_with(prefix))
        || (bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP")
}

/// 路径文本解析：单行/多行绝对路径或 `file://` URI，逐行去掉外层换行。
///
/// 任一行不是存在的本机绝对路径形式时整段返回 None（调用方回退普通文本）；
/// 不 `trim` 路径首尾合法空格，空行不参与。
pub fn parse_path_text(text: &str) -> Option<Vec<PathBuf>> {
    if text.is_empty() {
        return None;
    }
    // 末尾换行是外层行结束符，不构成空行；内部空行仍使整段回退普通文本。
    let text = text
        .strip_suffix('\n')
        .map(|rest| rest.strip_suffix('\r').unwrap_or(rest))
        .unwrap_or(text);
    if text.is_empty() {
        return None;
    }
    let mut paths = Vec::new();
    for line in text.split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.is_empty() {
            return None;
        }
        paths.push(parse_path_line(line)?);
    }
    (!paths.is_empty()).then_some(paths)
}

/// 单行路径解析：`file://` URI、本机绝对路径或可归一化的粘贴形式（引号/转义/拖拽空格）。
fn parse_path_line(line: &str) -> Option<PathBuf> {
    if line.contains('\0') {
        return None;
    }
    if let Some(rest) = line.strip_prefix("file://") {
        return parse_file_uri(rest);
    }
    if Path::new(line).is_absolute() {
        return Some(PathBuf::from(line));
    }
    let normalized = normalize_pasted_path_input(line);
    if !normalized.is_empty() && Path::new(&normalized).is_absolute() {
        return Some(PathBuf::from(normalized));
    }
    None
}

/// `file://` URI 解析：仅本机 host（空或大小写不敏感的 `localhost`）；百分号序列按 UTF-8 解码。
fn parse_file_uri(rest: &str) -> Option<PathBuf> {
    let path_part = if let Some(stripped) = rest.strip_prefix('/') {
        format!("/{stripped}")
    } else if rest
        .get(..10)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("localhost/"))
    {
        // `localhost` 为 9 个 ASCII 字节，第 10 个字节必是路径起始的 `/`。
        rest[9..].to_string()
    } else {
        return None;
    };
    let decoded = percent_decode(&path_part)?;
    #[cfg(windows)]
    {
        // `/C:/Users/x` → `C:/Users/x`；UNC 与非法盘符拒绝。
        let bytes = decoded.as_bytes();
        if bytes.len() >= 3
            && bytes[0] == b'/'
            && bytes[1].is_ascii_alphabetic()
            && bytes[2] == b':'
        {
            return Some(PathBuf::from(&decoded[1..]));
        }
        return None;
    }
    #[cfg(not(windows))]
    {
        if decoded.starts_with('/') {
            Some(PathBuf::from(decoded))
        } else {
            None
        }
    }
}

/// 百分号解码；非法序列、非 UTF-8 结果或解码出 NUL 字符返回 None。
fn percent_decode(input: &str) -> Option<String> {
    let mut bytes = Vec::with_capacity(input.len());
    let raw = input.as_bytes();
    let mut index = 0;
    while index < raw.len() {
        if raw[index] == b'%' {
            let hex = raw.get(index + 1..index + 3)?;
            let value = u8::from_str_radix(std::str::from_utf8(hex).ok()?, 16).ok()?;
            bytes.push(value);
            index += 3;
        } else {
            bytes.push(raw[index]);
            index += 1;
        }
    }
    String::from_utf8(bytes)
        .ok()
        .filter(|text| !text.contains('\0'))
}

#[cfg(test)]
mod tests;
