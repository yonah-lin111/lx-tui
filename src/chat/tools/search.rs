//! `search` 工具：在目录树中按正则或字面量搜索文本，尊重忽略规则。
//!
//! 默认从冻结的工作区 cwd 出发；可用 `path` 指定其他目录（显式绝对路径允许跨工作区）。
//! 遍历复用 `ignore` crate 的 `WalkBuilder`（与项目已有的文件扫描同源），不依赖外部 `rg`。
//! 结果按 `(path, line)` 稳定排序，命中行超长按字符截断并显式标记。
//!
//! 时限与规模都有硬上限：超过 [`MAX_SEARCH_RESULTS`]、[`MAX_SEARCH_FILES`] 或
//! [`SEARCH_TIMEOUT`] 时立即停止遍历并置 `truncated`，绝不无限遍历。

use std::path::{Path, PathBuf};
use std::time::Instant;

use ignore::WalkBuilder;
use regex::Regex;

use super::args::{Args, clamp_limit};
use super::path;
use super::text::split_lines;
use super::types::{
    SearchMatch, ToolContext, ToolErrorCode, ToolExecutionResult, ToolFailure, ToolResult,
};
use super::{
    MAX_SEARCH_FILE_BYTES, MAX_SEARCH_FILES, MAX_SEARCH_LINE_CHARS, MAX_SEARCH_RESULTS,
    SEARCH_TIMEOUT,
};

/// `search` 允许的参数集合。
const ALLOWED: [&str; 6] = [
    "pattern",
    "path",
    "include",
    "literal",
    "case_sensitive",
    "limit",
];

/// 遍历深度上限：显式指定广域路径时避免无限下钻。
const MAX_DEPTH: usize = 32;

/// 编译后的匹配器：正则或字面量（大小写开关已折叠进匹配器）。
enum Matcher {
    /// 已编译正则。
    Regex(Regex),
    /// 字面量子串（不敏感模式已转小写）。
    Literal {
        needle: String,
        case_sensitive: bool,
    },
}

impl Matcher {
    /// 文本是否匹配。
    fn is_match(&self, text: &str) -> bool {
        match self {
            Self::Regex(regex) => regex.is_match(text),
            Self::Literal {
                needle,
                case_sensitive,
            } => {
                if *case_sensitive {
                    text.contains(needle.as_str())
                } else {
                    text.to_lowercase().contains(needle.as_str())
                }
            }
        }
    }
}

/// 执行 `search`。
pub(crate) fn run(ctx: &ToolContext, arguments_json: &str) -> ToolResult<ToolExecutionResult> {
    let args = Args::parse(arguments_json, &ALLOWED)?;
    let pattern = args.required_str("pattern")?;
    let root_raw = args.optional_str("path")?.unwrap_or(".");
    let include = args.optional_str("include")?;
    let literal = args.optional_bool("literal")?.unwrap_or(false);
    let case_sensitive = args.optional_bool("case_sensitive")?.unwrap_or(false);
    let limit = clamp_limit(args.optional_positive("limit")?, MAX_SEARCH_RESULTS);

    let matcher = build_matcher(pattern, literal, case_sensitive)?;
    let root = path::resolve(ctx, root_raw)?;
    let caps = Caps {
        results: limit,
        ..Caps::default()
    };
    let metadata = std::fs::metadata(&root).map_err(|error| match error.kind() {
        std::io::ErrorKind::NotFound => ToolFailure::new(
            ToolErrorCode::NotFound,
            format!("{} does not exist", path::display(&root)),
        ),
        _ => path::map_io_error(&error, "stat", &root),
    })?;
    if metadata.is_file() {
        // 单文件搜索：直接扫该文件（不受 include 与忽略规则约束）。
        let single = [root.clone()];
        return scan_files(&matcher, &single, &caps);
    }
    if !metadata.is_dir() {
        return Err(ToolFailure::new(
            ToolErrorCode::NotADirectory,
            format!("{} is not a directory", path::display(&root)),
        ));
    }

    let glob = build_include(&root, include)?;
    let (files, truncated) = collect_files(&root, glob.as_ref(), &caps);
    let mut result = scan_files(&matcher, &files, &caps)?;
    if let ToolExecutionResult::Search {
        truncated: inner, ..
    } = &mut result
    {
        *inner |= truncated;
    }
    Ok(result)
}

/// 搜索资源上限快照；生产值即模块常量，测试可注入更小的上限。
#[derive(Debug, Clone, Copy)]
struct Caps {
    /// 命中条数上限。
    results: usize,
    /// 候选文件数上限。
    files: usize,
    /// 遍历总时长上限。
    timeout: std::time::Duration,
    /// 单文件字节上限。
    file_bytes: u64,
}

impl Default for Caps {
    fn default() -> Self {
        Self {
            results: MAX_SEARCH_RESULTS,
            files: MAX_SEARCH_FILES,
            timeout: SEARCH_TIMEOUT,
            file_bytes: MAX_SEARCH_FILE_BYTES,
        }
    }
}

/// 遍历目录收集候选文件；返回文件列表与是否因上限/超时截断。
fn collect_files(
    root: &Path,
    glob: Option<&ignore::overrides::Override>,
    caps: &Caps,
) -> (Vec<PathBuf>, bool) {
    let deadline = Instant::now() + caps.timeout;
    let mut files: Vec<PathBuf> = Vec::new();
    let mut truncated = false;
    let walker = WalkBuilder::new(root)
        .hidden(false)
        .ignore(true)
        .git_ignore(true)
        .git_global(false)
        .git_exclude(false)
        .parents(false)
        .require_git(false)
        .follow_links(false)
        .max_depth(Some(MAX_DEPTH))
        .filter_entry(|entry| {
            // 只排除符号链接；目录本身由 ignore 规则决定是否下钻。
            entry.depth() == 0 || entry.file_type().is_some_and(|kind| !kind.is_symlink())
        })
        .build();
    for entry in walker {
        if files.len() >= caps.files {
            truncated = true;
            break;
        }
        if Instant::now() >= deadline {
            truncated = true;
            break;
        }
        let Ok(entry) = entry else { continue };
        if !entry.file_type().is_some_and(|kind| kind.is_file()) {
            continue;
        }
        if let Some(glob) = glob {
            // include 是白名单：未命中（含 `None`）一律跳过。
            if !glob.matched(entry.path(), false).is_whitelist() {
                continue;
            }
        }
        files.push(entry.into_path());
    }
    (files, truncated)
}

/// 扫描给定文件集合，返回按 `(path, line)` 排序的命中。
fn scan_files(
    matcher: &Matcher,
    files: &[PathBuf],
    caps: &Caps,
) -> ToolResult<ToolExecutionResult> {
    let deadline = Instant::now() + caps.timeout;
    let mut matches: Vec<SearchMatch> = Vec::new();
    let mut truncated = false;
    let mut files_scanned = 0usize;
    'files: for file in files {
        if Instant::now() >= deadline {
            truncated = true;
            break;
        }
        files_scanned += 1;
        let Ok(metadata) = std::fs::metadata(file) else {
            continue;
        };
        if metadata.len() > caps.file_bytes {
            // 超过单文件上限：跳过并显式标记，避免误读巨型文件。
            truncated = true;
            continue;
        }
        let Ok(bytes) = std::fs::read(file) else {
            continue;
        };
        // 含 NUL 判定为二进制，不搜索（与 `read` 的判定一致）。
        if bytes.contains(&0) {
            continue;
        }
        let Ok(text) = std::str::from_utf8(&bytes) else {
            continue;
        };
        for (index, line) in split_lines(text).iter().enumerate() {
            if !matcher.is_match(line) {
                continue;
            }
            if matches.len() >= caps.results {
                truncated = true;
                break 'files;
            }
            let (rendered, line_truncated) = truncate_line(line);
            matches.push(SearchMatch {
                path: file.clone(),
                line: index + 1,
                text: rendered,
                line_truncated,
            });
        }
    }
    matches.sort_by(|left, right| {
        crate::platform::path_key(&left.path)
            .cmp(&crate::platform::path_key(&right.path))
            .then_with(|| left.line.cmp(&right.line))
    });
    Ok(ToolExecutionResult::Search {
        matches,
        truncated,
        files_scanned,
    })
}

/// 命中行超长截断（按字符，避免切断 UTF-8）。
fn truncate_line(line: &str) -> (String, bool) {
    if line.chars().count() <= MAX_SEARCH_LINE_CHARS {
        return (line.to_string(), false);
    }
    let head: String = line.chars().take(MAX_SEARCH_LINE_CHARS).collect();
    (format!("{head}…[truncated]"), true)
}

/// 编译匹配器；正则非法返回 `PatternInvalid`。
fn build_matcher(pattern: &str, literal: bool, case_sensitive: bool) -> ToolResult<Matcher> {
    if literal {
        return Ok(Matcher::Literal {
            needle: if case_sensitive {
                pattern.to_string()
            } else {
                pattern.to_lowercase()
            },
            case_sensitive,
        });
    }
    let regex = regex::RegexBuilder::new(pattern)
        .case_insensitive(!case_sensitive)
        .size_limit(4 * 1024 * 1024)
        .build()
        .map_err(|error| {
            ToolFailure::new(
                ToolErrorCode::PatternInvalid,
                format!("invalid regular expression: {error}"),
            )
        })?;
    Ok(Matcher::Regex(regex))
}

/// 编译 `include` glob（复用 `ignore` 的 override 语义，不引入额外 crate）。
///
/// glob 相对于**搜索根**匹配（与 `rg --glob` 体感一致）：`*.rs` 命中任意深度，
/// `src/**/*.rs` 只命中搜索根下 `src/` 子树。
fn build_include(
    root: &Path,
    include: Option<&str>,
) -> ToolResult<Option<ignore::overrides::Override>> {
    let Some(include) = include else {
        return Ok(None);
    };
    let mut builder = ignore::overrides::OverrideBuilder::new(root);
    builder.add(include).map_err(|error| {
        ToolFailure::new(
            ToolErrorCode::PatternInvalid,
            format!("invalid include glob: {error}"),
        )
    })?;
    let built = builder.build().map_err(|error| {
        ToolFailure::new(
            ToolErrorCode::PatternInvalid,
            format!("invalid include glob: {error}"),
        )
    })?;
    Ok(Some(built))
}

#[cfg(test)]
mod tests;
