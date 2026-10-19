//! `search` 工具：在目录树中按正则或字面量搜索文本，尊重忽略规则。
//!
//! 默认从冻结的工作区 cwd 出发；可用 `path` 指定其他目录（显式绝对路径允许跨工作区）。
//! 遍历复用 `ignore` crate 的 `WalkBuilder`（与项目已有的文件扫描同源），不依赖外部 `rg`。
//! 结果按 `(path, line)` 稳定排序，命中行超长按字符截断并显式标记。
//!
//! 可达文件集合对齐 ripgrep 的默认语义：`.git/` 与 `.jj/` 是**always-ignore**（与是否搜隐藏
//! 文件无关）；`.gitignore` / `.ignore` 在真正的 git 仓库内生效（`require_git(true)`），且
//! 父级忽略规则不随搜索根漂移（`parents(true)`）。隐藏文件（`.env` 等）默认**会**被搜索。
//!
//! 时限与规模都有硬上限：超过 [`MAX_SEARCH_RESULTS`]、[`MAX_SEARCH_FILES`]、`MAX_DEPTH`
//! 或 [`SEARCH_TIMEOUT`] 时立即停止遍历并置 `truncated`，绝不无限遍历。遍历与扫描共用
//! **同一个** deadline，因此单次调用的总时长上限就是一个 [`SEARCH_TIMEOUT`]。

use std::ffi::OsStr;
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

/// 遍历深度上限（`Caps::default()` 的生产值）：显式指定广域路径时避免无限下钻。
///
/// 触发时**必须**置 `truncated`（见 [`collect_files`]），否则就是静默截断。
const MAX_DEPTH: usize = 32;

/// VCS 目录名：无论 `hidden` 设置如何都排除（ripgrep 的 always-ignore 硬规则）。
///
/// `.git/config` 含 remote URL 凭据、`.git/logs` 含操作历史，`.jj/` 同理，绝不能进上下文。
const VCS_DIRS: [&str; 2] = [".git", ".jj"];

#[cfg(test)]
thread_local! {
    /// 测试注入点：统计当前线程构造 deadline 的次数。
    ///
    /// 用 thread-local 而非全局计数器，避免并行测试互相干扰；断言「一次调用只花一个预算」。
    static DEADLINES_CREATED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

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
        let deadline = deadline(&caps);
        return scan_files(&matcher, &single, &caps, deadline);
    }
    if !metadata.is_dir() {
        return Err(ToolFailure::new(
            ToolErrorCode::NotADirectory,
            format!("{} is not a directory", path::display(&root)),
        ));
    }

    let glob = build_include(&root, include)?;
    // 遍历与扫描共用一个 deadline：单次调用的总时长上限就是一个 `caps.timeout`。
    let deadline = deadline(&caps);
    let (files, truncated) = collect_files(&root, glob.as_ref(), &caps, deadline);
    let mut result = scan_files(&matcher, &files, &caps, deadline)?;
    if let ToolExecutionResult::Search {
        truncated: inner, ..
    } = &mut result
    {
        *inner |= truncated;
    }
    Ok(result)
}

/// 本次调用的唯一 deadline（遍历与扫描共用，避免两段各算一次而变成 2× 预算）。
fn deadline(caps: &Caps) -> Instant {
    #[cfg(test)]
    DEADLINES_CREATED.with(|cell| cell.set(cell.get() + 1));
    Instant::now() + caps.timeout
}

/// 重置当前线程的 deadline 计数（测试用）。
#[cfg(test)]
fn reset_deadline_count() {
    DEADLINES_CREATED.with(|cell| cell.set(0));
}

/// 当前线程已创建的 deadline 数量（测试用）。
#[cfg(test)]
fn deadline_count() -> usize {
    DEADLINES_CREATED.with(std::cell::Cell::get)
}

/// 目录是否至少有一个子项（用 `read_dir` 只取一项，不排序也不收集）。
///
/// 只用于深度上限的截断判定：空目录被剪掉不算丢东西，不必报 `truncated`。
fn dir_has_children(path: &Path) -> bool {
    std::fs::read_dir(path)
        .map(|mut entries| entries.next().is_some())
        // 读不到目录（权限等）时按「可能有内容」处理：宁可多报截断，不静默丢结果。
        .unwrap_or(true)
}

/// 是否为 VCS 元数据目录名（`.git` / `.jj`）。
fn is_vcs_dir_name(name: &OsStr) -> bool {
    VCS_DIRS.iter().any(|vcs| name == OsStr::new(vcs))
}

/// 搜索资源上限快照；生产值即模块常量，测试可注入更小的上限。
#[derive(Debug, Clone, Copy)]
struct Caps {
    /// 命中条数上限。
    results: usize,
    /// 候选文件数上限。
    files: usize,
    /// 遍历总时长上限（遍历与扫描共用同一个 deadline）。
    timeout: std::time::Duration,
    /// 单文件字节上限。
    file_bytes: u64,
    /// 遍历深度上限；目录 entry 落在该深度即无法再下钻，置 `truncated`。
    depth: usize,
}

impl Default for Caps {
    fn default() -> Self {
        Self {
            results: MAX_SEARCH_RESULTS,
            files: MAX_SEARCH_FILES,
            timeout: SEARCH_TIMEOUT,
            file_bytes: MAX_SEARCH_FILE_BYTES,
            depth: MAX_DEPTH,
        }
    }
}

/// 遍历目录收集候选文件；返回文件列表与是否因上限/超时/深度截断。
///
/// `deadline` 由 [`run`] 统一计算，与 [`scan_files`] 共用同一个预算。
fn collect_files(
    root: &Path,
    glob: Option<&ignore::overrides::Override>,
    caps: &Caps,
    deadline: Instant,
) -> (Vec<PathBuf>, bool) {
    let mut files: Vec<PathBuf> = Vec::new();
    let mut truncated = false;
    // `parents(true)` 必须与 `require_git(true)` **成对**设置：只开 parents 会让父目录
    // （例如 `~/.gitignore` 里的 `*`）在非 git 目录里吞掉整棵子树，搜索结果变空；
    // require_git(true) 把 gitignore 限定在真正的仓库内，与 git/ripgrep 语义一致
    // （codex 在 `file-search/src/lib.rs` 记录过这个坑）。
    let walker = WalkBuilder::new(root)
        .hidden(false)
        .ignore(true)
        .git_ignore(true)
        .git_global(false)
        .git_exclude(false)
        .parents(true)
        .require_git(true)
        .follow_links(false)
        .max_depth(Some(caps.depth))
        .filter_entry(|entry| {
            // 搜索根本身始终放行：显式把 `path` 指到 `.git/` 内部时尊重显式意图。
            if entry.depth() == 0 {
                return true;
            }
            // 符号链接不跟随（剪掉整个子树），与 `follow_links(false)` 保持一致。
            if entry.file_type().is_some_and(|kind| kind.is_symlink()) {
                return false;
            }
            // VCS 目录是 always-ignore：与 `hidden` 设置无关，任何深度都剪掉。
            !is_vcs_dir_name(entry.file_name())
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
        if entry.file_type().is_some_and(|kind| kind.is_dir()) {
            // 落在深度上限上的目录无法继续下钻：walker 只是不再深入，不会报错，
            // 必须在这里显式标记，否则就是静默截断（review P2-2）。
            // 只在它确实还有子项时才算截断：空目录本来就没有东西可丢，报截断是假消息。
            if entry.depth() >= caps.depth && dir_has_children(entry.path()) {
                truncated = true;
            }
            continue;
        }
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
    deadline: Instant,
) -> ToolResult<ToolExecutionResult> {
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
