//! 完整文件系统权限档案：条目模型、基准条目与路径解析（B 批）。
//!
//! 解析规则对齐 Codex：字面路径按组件边界前缀匹配、glob 按静态前缀计具体度，
//! 同具体度按 `Deny > Write > Read`。基准已含 `(/, Read)`，因此默认只读。

use std::fmt;
use std::path::{Path, PathBuf};

use globset::{GlobBuilder, GlobMatcher};

use crate::config::sandbox::SandboxMode;
use crate::platform::sandbox::roots;

/// 单个 `additional_permissions.file_system` 列表的最大条目数。
pub const MAX_REQUEST_ENTRIES: usize = 16;
/// 单个列表的条目总字节上限。
pub const MAX_REQUEST_BYTES: usize = 4096;
/// glob 扫描深度上限（对齐 Codex）。
pub const DEFAULT_GLOB_SCAN_MAX_DEPTH: usize = 16;

/// 条目的有效权限。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FileSystemAccessMode {
    /// 只读。
    Read,
    /// 可读写。
    Write,
    /// 读写都拒绝。
    Deny,
}

impl FileSystemAccessMode {
    /// 优先级：Deny > Write > Read。
    pub const fn rank(self) -> u8 {
        match self {
            Self::Read => 0,
            Self::Write => 1,
            Self::Deny => 2,
        }
    }
}

/// 条目路径：绝对字面路径或绝对路径开头的 glob。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileSystemPath {
    /// 绝对路径（目录或文件）。
    Path(PathBuf),
    /// git 风格 glob（`*` `?` `**` `[...]` `{a,b}`）。
    Glob(String),
}

impl FileSystemPath {
    /// 展示文本。
    pub fn display(&self) -> String {
        match self {
            Self::Path(path) => path.display().to_string(),
            Self::Glob(pattern) => pattern.clone(),
        }
    }

    /// 用于具体度比较的静态前缀长度（字节）。
    fn specificity(&self) -> usize {
        match self {
            Self::Path(path) => path.as_os_str().len(),
            Self::Glob(pattern) => static_prefix_len(pattern),
        }
    }

    /// 路径匹配时的具体度；不匹配返回 None。
    fn match_specificity(&self, path: &Path, matcher: Option<&GlobMatcher>) -> Option<usize> {
        match self {
            Self::Path(entry) => path.starts_with(entry).then(|| self.specificity()),
            Self::Glob(_) => matcher
                .filter(|matcher| matcher.is_match(path))
                .map(|_| self.specificity()),
        }
    }
}

/// 一条权限条目。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileSystemEntry {
    /// 绝对路径或 glob。
    pub path: FileSystemPath,
    /// 生效权限。
    pub access: FileSystemAccessMode,
}

/// 冻结的文件系统策略。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileSystemPolicy {
    /// 全部条目（基准 + 本次命令追加）。
    pub entries: Vec<FileSystemEntry>,
    /// glob 扫描深度上限。
    pub glob_scan_max_depth: usize,
}

/// `additional_permissions.file_system` 的请求内容。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FileSystemRequest {
    /// 追加只读。
    pub read: Vec<String>,
    /// 追加可写。
    pub write: Vec<String>,
    /// 追加拒绝（读写都禁止）。
    pub deny: Vec<String>,
}

impl FileSystemRequest {
    /// 请求是否为空。
    pub fn is_empty(&self) -> bool {
        self.read.is_empty() && self.write.is_empty() && self.deny.is_empty()
    }

    /// 是否只包含收缩权限的 `deny`（无需审批）。
    pub fn denies_only(&self) -> bool {
        !self.deny.is_empty() && self.read.is_empty() && self.write.is_empty()
    }

    /// 校验列表长度、总字节数与条目格式。
    pub fn validate(&self) -> Result<(), PermissionError> {
        for (name, list, globs_allowed) in [
            ("read", &self.read, false),
            ("write", &self.write, false),
            ("deny", &self.deny, true),
        ] {
            if list.len() > MAX_REQUEST_ENTRIES {
                return Err(PermissionError::Invalid(format!(
                    "'{name}' must contain at most {MAX_REQUEST_ENTRIES} entries"
                )));
            }
            let bytes: usize = list.iter().map(String::len).sum();
            if bytes > MAX_REQUEST_BYTES {
                return Err(PermissionError::Invalid(format!(
                    "'{name}' entries must total at most {MAX_REQUEST_BYTES} bytes"
                )));
            }
            for raw in list {
                let entry = parse_entry(raw)?;
                if !globs_allowed && matches!(entry, FileSystemPath::Glob(_)) {
                    return Err(PermissionError::Invalid(format!(
                        "'{name}' entries must be literal paths; globs are supported for 'deny'"
                    )));
                }
            }
        }
        Ok(())
    }

    /// 权限请求的可读摘要（审批 strip 展示）。
    pub fn summary(&self) -> String {
        let mut parts = Vec::new();
        for (label, list) in [
            ("read", &self.read),
            ("write", &self.write),
            ("deny", &self.deny),
        ] {
            if !list.is_empty() {
                parts.push(format!("{label}: {}", list.join(", ")));
            }
        }
        parts.join(" · ")
    }
}

/// `with_additional_permissions` 的请求内容。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AdditionalPermissions {
    /// 是否允许网络（None = 不改变）。
    pub network: Option<bool>,
    /// 文件系统请求。
    pub file_system: Option<FileSystemRequest>,
}

impl AdditionalPermissions {
    /// 是否为空。
    pub fn is_empty(&self) -> bool {
        self.network.is_none() && self.file_system.as_ref().is_none_or(|fs| fs.is_empty())
    }

    /// 是否只包含 `deny`（无需审批）。
    pub fn denies_only(&self) -> bool {
        matches!(self.network, None | Some(false))
            && self
                .file_system
                .as_ref()
                .is_some_and(FileSystemRequest::denies_only)
    }

    /// 审批 strip 展示文本。
    pub fn summary(&self) -> String {
        let mut parts = Vec::new();
        if self.network == Some(true) {
            parts.push("network: allowed".to_string());
        }
        if let Some(file_system) = &self.file_system
            && !file_system.is_empty()
        {
            parts.push(file_system.summary());
        }
        parts.join(" · ")
    }
}

/// 权限档案错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionError {
    /// 条目或请求非法。
    Invalid(String),
}

impl fmt::Display for PermissionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(message) => {
                write!(formatter, "invalid file system permission: {message}")
            }
        }
    }
}

impl std::error::Error for PermissionError {}

impl FileSystemPolicy {
    /// 按模式生成基准条目。
    pub fn baseline(mode: SandboxMode, cwd: &Path, extra_roots: &[PathBuf]) -> Self {
        let mut policy = Self {
            entries: vec![entry(
                FileSystemPath::Path(PathBuf::from("/")),
                FileSystemAccessMode::Read,
            )],
            glob_scan_max_depth: DEFAULT_GLOB_SCAN_MAX_DEPTH,
        };
        if mode != SandboxMode::WorkspaceWrite {
            return policy;
        }
        let roots = roots::writable_roots(cwd, extra_roots);
        for root in &roots {
            policy.push(
                FileSystemPath::Path(root.clone()),
                FileSystemAccessMode::Write,
            );
        }
        for protected in roots::protected_paths(&roots) {
            policy.push(FileSystemPath::Path(protected), FileSystemAccessMode::Read);
        }
        policy
    }

    /// 空策略（`danger-full-access` 与无沙箱平台）。
    pub fn empty() -> Self {
        Self {
            entries: Vec::new(),
            glob_scan_max_depth: DEFAULT_GLOB_SCAN_MAX_DEPTH,
        }
    }

    /// 追加一次命令的请求条目；重复条目幂等。
    pub fn with_request(&self, request: &FileSystemRequest) -> Result<Self, PermissionError> {
        request.validate()?;
        let mut policy = self.clone();
        for (list, access) in [
            (&request.read, FileSystemAccessMode::Read),
            (&request.write, FileSystemAccessMode::Write),
            (&request.deny, FileSystemAccessMode::Deny),
        ] {
            for raw in list {
                policy.push(parse_entry(raw)?, access);
            }
        }
        Ok(policy)
    }

    /// 对绝对路径计算有效权限；无匹配时继承基准只读。
    pub fn access_for(&self, path: &Path) -> FileSystemAccessMode {
        self.explain(path)
            .map(|(access, _)| access)
            .unwrap_or(FileSystemAccessMode::Read)
    }

    /// 计算有效权限并给出命中的最具体条目。
    ///
    /// `Deny` 是收缩型安全规则：任何命中都优先于 Read/Write，避免后端投影
    /// （deny 规则无法被更具体的 allow 覆盖）与文件工具闸门给出不同结果。
    pub fn explain(&self, path: &Path) -> Option<(FileSystemAccessMode, FileSystemPath)> {
        let mut best: Option<(usize, FileSystemAccessMode, FileSystemPath)> = None;
        let mut deny: Option<(usize, FileSystemPath)> = None;
        for entry in &self.entries {
            let matcher = match &entry.path {
                FileSystemPath::Glob(pattern) => match compile_glob(pattern) {
                    Ok(matcher) => Some(matcher),
                    Err(_) => continue,
                },
                FileSystemPath::Path(_) => None,
            };
            let Some(specificity) = entry.path.match_specificity(path, matcher.as_ref()) else {
                continue;
            };
            if entry.access == FileSystemAccessMode::Deny {
                if deny
                    .as_ref()
                    .is_none_or(|(current, _)| specificity > *current)
                {
                    deny = Some((specificity, entry.path.clone()));
                }
                continue;
            }
            let better = match &best {
                None => true,
                Some((current, access, _)) => {
                    specificity > *current
                        || (specificity == *current && entry.access.rank() > access.rank())
                }
            };
            if better {
                best = Some((specificity, entry.access, entry.path.clone()));
            }
        }
        deny.map(|(_, path)| (FileSystemAccessMode::Deny, path))
            .or_else(|| best.map(|(_, access, path)| (access, path)))
    }

    /// 可用于写绑定/写规则的路径（非 glob 的 `Write` 条目）。
    pub fn writable_roots(&self) -> Vec<PathBuf> {
        let mut roots: Vec<PathBuf> = self
            .entries
            .iter()
            .filter(|entry| entry.access == FileSystemAccessMode::Write)
            .filter_map(|entry| match &entry.path {
                FileSystemPath::Path(path)
                    if self.access_for(path) == FileSystemAccessMode::Write =>
                {
                    Some(path.clone())
                }
                _ => None,
            })
            .collect();
        roots.sort_by_key(|path| path.components().count());
        roots.dedup();
        roots
    }

    /// 写根内的只读保护路径（`.git` 等基准保护与显式 `read` carveout）。
    pub fn protected_paths(&self) -> Vec<PathBuf> {
        let roots = self.writable_roots();
        let mut paths: Vec<PathBuf> = self
            .entries
            .iter()
            .filter(|entry| entry.access == FileSystemAccessMode::Read)
            .filter_map(|entry| match &entry.path {
                FileSystemPath::Path(path)
                    if roots
                        .iter()
                        .any(|root| path != root && path.starts_with(root)) =>
                {
                    Some(path.clone())
                }
                _ => None,
            })
            .collect();
        paths.sort();
        paths.dedup();
        paths
    }

    /// 显式 `Deny` 字面路径。
    pub fn deny_paths(&self) -> Vec<PathBuf> {
        let mut paths: Vec<PathBuf> = self
            .entries
            .iter()
            .filter(|entry| entry.access == FileSystemAccessMode::Deny)
            .filter_map(|entry| match &entry.path {
                FileSystemPath::Path(path) => Some(path.clone()),
                _ => None,
            })
            .collect();
        paths.sort();
        paths.dedup();
        paths
    }

    /// 显式 `Deny` glob。
    pub fn deny_globs(&self) -> Vec<String> {
        let mut globs: Vec<String> = self
            .entries
            .iter()
            .filter(|entry| entry.access == FileSystemAccessMode::Deny)
            .filter_map(|entry| match &entry.path {
                FileSystemPath::Glob(pattern) => Some(pattern.clone()),
                _ => None,
            })
            .collect();
        globs.sort();
        globs.dedup();
        globs
    }

    fn push(&mut self, path: FileSystemPath, access: FileSystemAccessMode) {
        let entry = FileSystemEntry { path, access };
        if !self.entries.contains(&entry) {
            self.entries.push(entry);
        }
    }
}

fn entry(path: FileSystemPath, access: FileSystemAccessMode) -> FileSystemEntry {
    FileSystemEntry { path, access }
}

/// 解析并校验一条请求条目：绝对路径或绝对路径开头的 glob，禁止 `~` 与控制字符。
pub fn parse_entry(raw: &str) -> Result<FileSystemPath, PermissionError> {
    let text = raw.trim();
    if text.is_empty() {
        return Err(PermissionError::Invalid(
            "path entries must not be empty".into(),
        ));
    }
    if text.chars().any(char::is_control) {
        return Err(PermissionError::Invalid(
            "path entries must not contain control characters".into(),
        ));
    }
    if text == "~" || text.starts_with("~/") || text.starts_with("~\\") {
        return Err(PermissionError::Invalid(
            "path entries must be absolute; `~` is not allowed".into(),
        ));
    }
    if !Path::new(text).is_absolute() {
        return Err(PermissionError::Invalid(format!(
            "path entry '{text}' must be absolute"
        )));
    }
    if is_glob(text) {
        compile_glob(text)?;
        Ok(FileSystemPath::Glob(text.to_string()))
    } else {
        Ok(FileSystemPath::Path(PathBuf::from(text)))
    }
}

/// 编译 glob（`literal_separator(true)`：`*` 不跨目录）。
pub fn compile_glob(pattern: &str) -> Result<GlobMatcher, PermissionError> {
    GlobBuilder::new(pattern)
        .literal_separator(true)
        .backslash_escape(true)
        .build()
        .map(|glob| glob.compile_matcher())
        .map_err(|error| PermissionError::Invalid(format!("invalid glob '{pattern}': {error}")))
}

/// 路径是否含 glob 元字符。
pub fn is_glob(text: &str) -> bool {
    text.chars()
        .any(|ch| matches!(ch, '*' | '?' | '[' | ']' | '{' | '}'))
}

/// glob 第一个元字符之前的静态前缀长度（字节）。
fn static_prefix_len(pattern: &str) -> usize {
    pattern
        .char_indices()
        .find_map(|(index, ch)| {
            matches!(ch, '*' | '?' | '[' | ']' | '{' | '}' | '\\').then_some(index)
        })
        .unwrap_or(pattern.len())
}

#[cfg(test)]
mod tests;
