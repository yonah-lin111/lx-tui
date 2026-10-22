//! 路径与数据库 TEXT 之间的无损编码。
//!
//! SQLite 列是 TEXT；Unix 路径不保证是合法 UTF-8。直接 `to_string_lossy` 会把
//! 非 UTF-8 字节替换成 U+FFFD，恢复时拼出一个**不同的路径**。这里对非 UTF-8 路径
//! 使用带标记的 base64 编码，UTF-8 路径保持原样，读写两侧共用同一套转换。

use std::path::{Path, PathBuf};

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;

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
mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    #[test]
    fn utf8_paths_roundtrip_unchanged() {
        let path = Path::new("/tmp/目录/a.txt");
        let encoded = encode_path(path);
        assert_eq!(encoded, "/tmp/目录/a.txt");
        assert_eq!(decode_path(&encoded), path);
    }

    #[test]
    fn non_utf8_paths_roundtrip_without_loss() {
        let path = PathBuf::from(OsString::from_vec(b"/tmp/bad-\xff\xfe.txt".to_vec()));
        let encoded = encode_path(&path);
        assert!(encoded.starts_with(TAG), "必须带标记：{encoded}");
        assert_eq!(decode_path(&encoded), path);
    }
}

#[cfg(all(test, unix))]
mod chain_tests {
    use super::*;

    fn temp(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "lx-path-chain-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    #[test]
    fn chain_accepts_real_directories() {
        let root = temp("ok");
        std::fs::create_dir_all(root.join("sub")).expect("dirs");
        let target = root.join("sub/a.txt");
        let (anchor, canonical) = path_anchor(&target).expect("anchor");
        assert!(verify_path_chain(&anchor, &canonical, &target).is_ok());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn chain_rejects_symlinked_component() {
        let root = temp("link");
        let elsewhere = temp("elsewhere");
        std::fs::create_dir_all(root.join("sub")).expect("dirs");
        let target = root.join("sub/a.txt");
        let (anchor, canonical) = path_anchor(&target).expect("anchor");
        std::fs::remove_dir_all(root.join("sub")).expect("remove");
        std::os::unix::fs::symlink(&elsewhere, root.join("sub")).expect("symlink");
        assert!(
            verify_path_chain(&anchor, &canonical, &target).is_err(),
            "中间目录被换成链接必须拒绝"
        );
        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_dir_all(elsewhere);
    }
}
