//! 路径与数据库 TEXT 之间的无损编码。
//!
//! SQLite 列是 TEXT；Unix 路径不保证是合法 UTF-8。直接 `to_string_lossy` 会把
//! 非 UTF-8 字节替换成 U+FFFD，恢复时拼出一个**不同的路径**。这里对非 UTF-8 路径
//! 使用带标记的 base64 编码，UTF-8 路径保持原样，读写两侧共用同一套转换。

use std::path::{Path, PathBuf};

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;

/// 作用域目录名：清洗后的目录名 + 路径哈希（同名目录不互相覆盖）。
///
/// 开发库隔离目录、项目级 MCP 配置与项目级 skill 目录共用这一份命名，
/// 保证同一项目在所有作用域里的目录名一致。
pub fn scope_dir_name(root: &Path) -> Option<String> {
    let name = root.file_name()?.to_str().map(scope_name)?;
    Some(format!("{name}-{:08x}", path_hash(root) as u32))
}

/// 目录名清洗：保留 ASCII 字母数字与 `-_.`，其余替换为 `-`；空结果回退 `worktree`。
pub fn scope_name(raw: &str) -> String {
    let name: String = raw
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                ch
            } else {
                '-'
            }
        })
        .collect();
    let name = name.trim_matches('.');
    if name.is_empty() {
        "worktree".to_string()
    } else {
        name.to_string()
    }
}

/// FNV-1a 64 位；作用域与内容指纹只需稳定，不用于安全判定。
pub fn path_hash(path: &Path) -> u64 {
    bytes_hash(path.to_string_lossy().as_bytes())
}

/// 字节级 FNV-1a 64 位。
pub fn bytes_hash(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// 非 UTF-8 路径的标记前缀（U+001F 控制字符 + 固定 ASCII 标记）。
const TAG: &str = "\u{1f}lx-b64:";

/// 路径 → 数据库文本。
pub fn encode_path(path: &Path) -> String {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        match path.to_str() {
            Some(text) => text.to_string(),
            None => format!("{TAG}{}", BASE64.encode(path.as_os_str().as_bytes())),
        }
    }
    #[cfg(not(unix))]
    {
        path.to_string_lossy().into_owned()
    }
}

/// 数据库文本 → 路径；识别不了编码时按普通文本处理。
pub fn decode_path(text: &str) -> PathBuf {
    #[cfg(unix)]
    {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;
        if let Some(encoded) = text.strip_prefix(TAG)
            && let Ok(bytes) = BASE64.decode(encoded)
        {
            return PathBuf::from(OsString::from_vec(bytes));
        }
        PathBuf::from(text)
    }
    #[cfg(not(unix))]
    {
        PathBuf::from(text)
    }
}

/// 目标的现存祖先：返回 `(词法祖先, 规范化祖先)`。
///
/// 目标自身可以尚不存在；取它的父目录向上找第一个存在的目录。词法祖先用于在恢复时
/// 重新走一遍路径链，规范化结果用于识别“祖先目录本身被换成了链接”。
pub fn path_anchor(path: &Path) -> Option<(PathBuf, PathBuf)> {
    let mut current = path.parent()?.to_path_buf();
    loop {
        if current.is_dir() {
            let canonical = std::fs::canonicalize(&current).ok()?;
            return Some((current, canonical));
        }
        if !current.pop() {
            return None;
        }
    }
}

/// 校验目标路径链：祖先未被替换，且中间目录都是真实目录（不是符号链接）。
///
/// 目标本身（最后一个 component）不检查：delete/move 的合法对象就是链接目录项。
/// 中间目录尚不存在（新建文件的父目录）时视为安全：写入会创建真实目录。
pub fn verify_path_chain(
    anchor_path: &Path,
    anchor_canonical: &Path,
    target: &Path,
) -> std::io::Result<()> {
    let current_anchor = std::fs::canonicalize(anchor_path)?;
    if current_anchor != anchor_canonical {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "path ancestor was redirected",
        ));
    }
    let Ok(relative) = target.strip_prefix(anchor_path) else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "target is outside its recorded ancestor",
        ));
    };
    let mut current = anchor_path.to_path_buf();
    let components: Vec<_> = relative.components().collect();
    for component in components.iter().take(components.len().saturating_sub(1)) {
        current.push(component);
        match std::fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "path component was replaced by a symlink",
                ));
            }
            Ok(metadata) if !metadata.is_dir() => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "path component is not a directory",
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod tests;
