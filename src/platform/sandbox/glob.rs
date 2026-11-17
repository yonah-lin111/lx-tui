//! git 风格 glob 的共享实现：SBPL 正则编译与 bwrap 侧有界扫描。
//!
//! 移植自 OpenAI Codex `sandboxing/src/seatbelt.rs`（Apache-2.0），去掉与 lx
//! 无关的大小写/别名逻辑；`seatbelt` 用正则，`bwrap` 用 `globset` 本地匹配。

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::path::Path;
use std::path::PathBuf;

use globset::{GlobBuilder, GlobMatcher};

use super::SandboxError;

/// 单次 glob 展开的匹配上限（超过即 fail-closed）。
pub const MAX_GLOB_MATCHES: usize = 4096;

/// 把 git 风格 glob 编译成 SBPL `(regex #"...")` 可用的正则。
///
/// `subtree = true` 时字面模式同时匹配后代（对齐 Codex unreadable glob 语义）。
pub fn to_regex(pattern: &str, subtree: bool) -> Option<String> {
    if pattern.is_empty() {
        return None;
    }
    let mut regex = String::from("^");
    let mut chars = pattern.chars().collect::<std::collections::VecDeque<_>>();
    let mut alternate_depth = 0;

    while let Some(ch) = chars.pop_front() {
        match ch {
            '*' => {
                if chars.front() == Some(&'*') {
                    chars.pop_front();
                    if chars.front() == Some(&'/') {
                        chars.pop_front();
                        regex.push_str("(.*/)?");
                    } else {
                        regex.push_str(".*");
                    }
                } else {
                    regex.push_str("[^/]*");
                }
            }
            '?' => {
                regex.push_str("[^/]");
            }
            '\\' => {
                if let Some(escaped) = chars.pop_front() {
                    regex.push_str(&regex::escape(&escaped.to_string()));
                } else {
                    regex.push_str("\\\\");
                }
            }
            '{' => {
                alternate_depth += 1;
                regex.push('(');
            }
            '}' if alternate_depth > 0 => {
                alternate_depth -= 1;
                regex.push(')');
            }
            ',' if alternate_depth > 0 => regex.push('|'),
            '[' => {
                let mut class = Vec::new();
                let mut closed = false;
                while let Some(class_ch) = chars.pop_front() {
                    if class_ch == ']' {
                        closed = true;
                        break;
                    }
                    class.push(class_ch);
                }
                if !closed {
                    regex.push_str("\\[");
                    for class_ch in class.into_iter().rev() {
                        chars.push_front(class_ch);
                    }
                    continue;
                }
                regex.push('[');
                let mut class_chars = class.into_iter();
                if let Some(first) = class_chars.next() {
                    match first {
                        '!' => regex.push('^'),
                        '^' => regex.push_str("\\^"),
                        _ => regex.push(first),
                    }
                }
                for class_ch in class_chars {
                    match class_ch {
                        '\\' => regex.push_str("\\\\"),
                        _ => regex.push(class_ch),
                    }
                }
                regex.push(']');
            }
            ']' => {
                regex.push_str("\\]");
            }
            _ => regex.push_str(&regex::escape(&ch.to_string())),
        }
    }

    // 花括号分支在分隔符处会被截断（`/a/{b/c,d}` -> `/a/{b`），补齐右括号。
    for _ in 0..alternate_depth {
        regex.push(')');
    }
    if subtree && !regex.ends_with(".*") {
        regex.push_str("(/.*)?");
    }
    regex.push('$');
    Some(regex)
}

/// 把 glob 转义成 SBPL 原始字符串可用的文本（转义 `"`）。
pub fn sbpl_regex_literal(regex: &str) -> String {
    regex.replace('"', "\\\"")
}

/// glob 第一个元字符之前的静态目录（扫描起点）；根目录或空前缀返回 None。
pub fn scan_root(pattern: &str) -> Option<PathBuf> {
    let first = pattern.char_indices().find_map(|(index, ch)| {
        matches!(ch, '*' | '?' | '[' | ']' | '{' | '}' | '\\').then_some(index)
    });
    let prefix = match first {
        Some(index) => &pattern[..index],
        None => pattern,
    };
    let root = if prefix.ends_with('/') {
        prefix.trim_end_matches('/')
    } else {
        prefix.rsplit_once('/').map(|(root, _)| root).unwrap_or("")
    };
    if root.is_empty() || root == "/" {
        return None;
    }
    Some(PathBuf::from(root))
}

/// 把 glob 静态目录前缀规范化（支持存在目录或最近存在祖先）。
///
/// 无变化、全局根 glob 或静态目录含 `..` 时返回 None。
pub fn canonical_glob_alias(pattern: &str) -> Option<String> {
    let first = pattern.char_indices().find_map(|(index, ch)| {
        matches!(ch, '*' | '?' | '[' | ']' | '{' | '}' | '\\').then_some(index)
    });
    let static_part = match first {
        Some(index) => &pattern[..index],
        None => pattern,
    };
    let slash_idx = static_part.rfind('/')?;
    if slash_idx == 0 {
        return None;
    }
    let static_dir = Path::new(&pattern[..slash_idx]);
    let tail = &pattern[slash_idx..];
    let canonical_dir = super::roots::resolve_write_target(static_dir)
        .or_else(|| super::roots::canonical_parent_or_ancestor(static_dir))?;
    if canonical_dir == static_dir {
        return None;
    }
    let dir_str = canonical_dir.to_string_lossy();
    let mut escaped = String::with_capacity(dir_str.len());
    for ch in dir_str.chars() {
        if matches!(ch, '*' | '?' | '[' | ']' | '{' | '}' | '\\') {
            escaped.push('\\');
        }
        escaped.push(ch);
    }
    escaped.push_str(tail);
    Some(escaped)
}

/// 在写根下按深度上限扫描 glob 匹配项（含符号链接的规范目标）。
pub fn scan_matches(
    patterns: &[String],
    max_depth: usize,
) -> Result<BTreeSet<PathBuf>, SandboxError> {
    if patterns.is_empty() || max_depth == 0 {
        return Ok(BTreeSet::new());
    }
    let mut by_root: BTreeMap<PathBuf, Vec<(String, GlobMatcher)>> = BTreeMap::new();
    for pattern in patterns {
        let Some(root) = scan_root(pattern) else {
            return Err(SandboxError::Invalid(format!(
                "deny glob '{pattern}' must start with a non-root directory prefix"
            )));
        };
        // 扫描根不存在或不是目录：没有匹配，安全跳过。
        match std::fs::metadata(&root) {
            Ok(metadata) if metadata.is_dir() => {}
            Ok(_) => continue,
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
                ) =>
            {
                continue;
            }
            Err(error) => {
                return Err(SandboxError::Invalid(format!(
                    "failed to inspect deny glob root {}: {error}",
                    root.display()
                )));
            }
        }
        let matcher = GlobBuilder::new(pattern)
            .literal_separator(true)
            .backslash_escape(true)
            .build()
            .map(|glob| glob.compile_matcher())
            .map_err(|error| {
                SandboxError::Invalid(format!("invalid deny glob '{pattern}': {error}"))
            })?;
        by_root
            .entry(root)
            .or_default()
            .push((pattern.clone(), matcher));
    }

    let mut matches = BTreeSet::new();
    for (root, patterns) in by_root {
        walk(&root, &patterns, max_depth, 0, &mut matches)?;
        if matches.len() > MAX_GLOB_MATCHES {
            return Err(SandboxError::Invalid(format!(
                "deny glob expansion matched more than {MAX_GLOB_MATCHES} paths"
            )));
        }
    }
    Ok(matches)
}

fn walk(
    dir: &Path,
    patterns: &[(String, GlobMatcher)],
    max_depth: usize,
    depth: usize,
    matches: &mut BTreeSet<PathBuf>,
) -> Result<(), SandboxError> {
    if depth > max_depth {
        return Ok(());
    }
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::NotFound
                    | std::io::ErrorKind::NotADirectory
                    | std::io::ErrorKind::PermissionDenied
            ) =>
        {
            return Ok(());
        }
        Err(error) => {
            return Err(SandboxError::Invalid(format!(
                "failed to scan deny glob root {}: {error}",
                dir.display()
            )));
        }
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let matched = patterns
            .iter()
            .any(|(_, matcher)| matcher.is_match(path.as_path()));
        if matched {
            if matches.len() >= MAX_GLOB_MATCHES {
                return Err(SandboxError::Invalid(format!(
                    "deny glob expansion matched more than {MAX_GLOB_MATCHES} paths"
                )));
            }
            if let Ok(target) = std::fs::canonicalize(&path)
                && target != path
            {
                matches.insert(target);
            }
            matches.insert(path.clone());
        }
        let is_dir = std::fs::symlink_metadata(&path)
            .map(|metadata| metadata.is_dir())
            .unwrap_or(false);
        if is_dir {
            walk(&path, patterns, max_depth, depth + 1, matches)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
