//! 粘贴路径解析：单行/多行绝对路径与 `file://` URI，以及图片文件签名探测。
//!
//! 纯解析放在这里，核心模块不出现 `cfg(target_os)`；引用标记的格式与解析在 `app/markdown/`。

use std::path::{Path, PathBuf};

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

/// 单行路径解析：`file://` URI 或本机绝对路径。
fn parse_path_line(line: &str) -> Option<PathBuf> {
    if line.contains('\0') {
        return None;
    }
    if let Some(rest) = line.strip_prefix("file://") {
        return parse_file_uri(rest);
    }
    let path = Path::new(line);
    path.is_absolute().then(|| path.to_path_buf())
}

/// `file://` URI 解析：仅本机 host（空或 `localhost`）；百分号序列按 UTF-8 解码。
fn parse_file_uri(rest: &str) -> Option<PathBuf> {
    let (host, path) = match rest.find('/') {
        Some(0) => ("", rest),
        Some(index) => (&rest[..index], &rest[index..]),
        None => return None,
    };
    if !host.is_empty() && host != "localhost" {
        return None;
    }
    let decoded = percent_decode(path)?;
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

/// 百分号解码；非法序列或非 UTF-8 结果返回 None。
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
    String::from_utf8(bytes).ok()
}

#[cfg(test)]
mod tests;
