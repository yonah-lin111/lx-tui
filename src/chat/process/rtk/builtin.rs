//! 内置输出过滤（外部 rtk 缺失时使用）。
//!
//! 规则只做**可解释的裁剪**：保留错误/失败/摘要与首尾，裁掉样板行，并在结果里写明
//! 省略了多少行。绝不改写命令、退出码或真实偏移；只影响进入模型/卡片的文本。

/// 单次输出超过该字符数才进入通用首尾裁剪。
pub const MAX_GENERIC_CHARS: usize = 2_000;
/// grep/rg 总量与单文件上限（对齐 rtk 默认配置）。
const GREP_MAX_RESULTS: usize = 200;
const GREP_MAX_PER_FILE: usize = 25;
/// git status 最多保留的已修改/未跟踪文件数。
const STATUS_MAX_CHANGED: usize = 15;
const STATUS_MAX_UNTRACKED: usize = 10;
/// 各类过滤的物理行上限。
const MAX_TEST_LINES: usize = 200;
const MAX_BUILD_LINES: usize = 200;
const MAX_DIFF_LINES: usize = 300;
const MAX_LOG_LINES: usize = 40;
const MAX_LIST_LINES: usize = 200;
/// 通用裁剪：超过行数时保留的首尾行。
const GENERIC_HEAD_LINES: usize = 120;
const GENERIC_TAIL_LINES: usize = 40;
/// 通用字符上限（防止单行巨型 JSON 撑爆）。
const GENERIC_MAX_BYTES: usize = 16 * 1024;

/// 按命令首段选择过滤器。
pub fn filter_output(command: &str, output: &str) -> String {
    let cleaned = strip_ansi(output);
    if cleaned.trim().is_empty() {
        return String::new();
    }
    match family(command) {
        Family::Test => filter_test(&cleaned),
        Family::Build => filter_build(&cleaned),
        Family::Status => filter_status(&cleaned),
        Family::Diff => filter_diff(&cleaned),
        Family::Log => filter_log(&cleaned),
        Family::Grep => filter_grep(&cleaned),
        Family::List => filter_list(&cleaned),
        Family::Plain => filter_generic(&cleaned),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Family {
    Test,
    Build,
    Status,
    Diff,
    Log,
    Grep,
    List,
    Plain,
}

/// 首段命令 → 过滤器类别。
fn family(command: &str) -> Family {
    let mut words = command.split_whitespace();
    let first = words.next().unwrap_or_default();
    let second = words.next().unwrap_or_default();
    match (first, second) {
        ("cargo", "test") | ("cargo", "nextest") => Family::Test,
        ("cargo", "build") | ("cargo", "check") | ("cargo", "clippy") | ("cargo", "install") => {
            Family::Build
        }
        ("npm", "test") | ("pnpm", "test") | ("vitest", _) | ("jest", _) => Family::Test,
        ("tsc", _) | ("dotnet", _) => Family::Build,
        ("git", "status") => Family::Status,
        ("git", "diff") | ("git", "show") => Family::Diff,
        ("git", "log") => Family::Log,
        ("grep", _) | ("rg", _) => Family::Grep,
        ("ls", _) | ("tree", _) | ("find", _) => Family::List,
        _ => Family::Plain,
    }
}

/// 失败/诊断行：保留该行及其后若干行上下文（代码片段/断言输出）。
fn filter_test(output: &str) -> String {
    let lines: Vec<&str> = output.lines().collect();
    let summary = lines
        .iter()
        .rev()
        .find(|line| line.trim_start().starts_with("test result:"));
    let mut kept: Vec<String> = Vec::new();
    let mut block = 0usize;
    for line in &lines {
        let trimmed = line.trim_start();
        let is_marker = trimmed.starts_with("FAILED")
            || trimmed.contains(" FAILED")
            || trimmed.starts_with("failures:")
            || trimmed.starts_with("---- ")
            || trimmed.starts_with("panicked")
            || trimmed.starts_with("thread '")
            || trimmed.starts_with("error")
            || trimmed.starts_with("warning:")
            || trimmed.starts_with("assertion")
            || trimmed.starts_with("left:")
            || trimmed.starts_with("right:")
            || trimmed.starts_with("test result:");
        if is_marker {
            kept.push((*line).to_string());
            block = if trimmed.starts_with("---- ") { 20 } else { 6 };
            continue;
        }
        if block > 0 && (line.starts_with(char::is_whitespace) || line.trim().is_empty()) {
            kept.push((*line).to_string());
            block -= 1;
        } else {
            block = 0;
        }
    }
    if kept.is_empty() {
        return match summary {
            Some(line) => format!("{line}\n"),
            None => "ok (no failures detected)\n".to_string(),
        };
    }
    cap_lines(kept, MAX_TEST_LINES, "lines")
}

/// 构建输出：去掉编译进度样板，保留警告/错误与完成行。
fn filter_build(output: &str) -> String {
    let mut kept: Vec<String> = Vec::new();
    let mut block = 0usize;
    for line in output.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("Compiling ")
            || trimmed.starts_with("Checking ")
            || trimmed.starts_with("Downloading ")
            || trimmed.starts_with("Downloaded ")
            || trimmed.starts_with("Updating ")
            || trimmed.starts_with("Locking ")
            || trimmed.starts_with("Adding ")
            || trimmed.starts_with("Running `")
        {
            block = 0;
            continue;
        }
        let is_marker = trimmed.starts_with("warning:")
            || trimmed.starts_with("error")
            || trimmed.starts_with("Finished ")
            || trimmed.starts_with("FAILED")
            || trimmed.starts_with("test result:");
        if is_marker {
            kept.push(line.to_string());
            block = 6;
            continue;
        }
        if block > 0 && (line.starts_with(char::is_whitespace) || line.trim().is_empty()) {
            kept.push(line.to_string());
            block -= 1;
        } else {
            block = 0;
        }
    }
    if kept.is_empty() {
        return "ok\n".to_string();
    }
    cap_lines(kept, MAX_BUILD_LINES, "lines")
}

/// git status：保留分支与状态分组，按上限列出文件并报告省略数。
fn filter_status(output: &str) -> String {
    let mut kept: Vec<String> = Vec::new();
    let mut changed = 0usize;
    let mut untracked = 0usize;
    let mut omitted_changed = 0usize;
    let mut omitted_untracked = 0usize;
    let mut in_untracked = false;
    for line in output.lines() {
        let trimmed = line.trim_start();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.starts_with("Untracked files:") {
            in_untracked = true;
            kept.push(line.to_string());
            continue;
        }
        if trimmed.starts_with("Changes not staged")
            || trimmed.starts_with("Changes to be committed")
            || trimmed.starts_with("Unmerged paths:")
            || trimmed.starts_with("On branch")
            || trimmed.starts_with("Your branch")
            || trimmed.starts_with("## ")
        {
            kept.push(line.to_string());
            continue;
        }
        // `(use "git add ...")` 之类的提示行不占文件额度。
        if trimmed.starts_with('(') {
            continue;
        }
        // 文件行：porcelain 两列状态，或长格式的 `modified:` / `new file:` 等。
        let prefix: Vec<char> = line.chars().take(2).collect();
        let status_char =
            |ch: char| matches!(ch, ' ' | 'M' | 'A' | 'D' | 'R' | 'C' | 'U' | '?' | '!');
        let porcelain = prefix.len() == 2
            && prefix.iter().all(|ch| status_char(*ch))
            && prefix.iter().any(|ch| *ch != ' ');
        let long_format = [
            "modified:",
            "new file:",
            "deleted:",
            "renamed:",
            "copied:",
            "both modified:",
            "both added:",
            "both deleted:",
            "typechange:",
            "untracked:",
        ]
        .iter()
        .any(|marker| trimmed.starts_with(marker));
        let is_file = if in_untracked {
            // 长格式的未跟踪文件是 TAB 缩进的裸路径。
            true
        } else {
            porcelain || long_format
        };
        if !is_file {
            kept.push(line.to_string());
            continue;
        }
        if in_untracked {
            if untracked < STATUS_MAX_UNTRACKED {
                kept.push(line.to_string());
                untracked += 1;
            } else {
                omitted_untracked += 1;
            }
        } else if changed < STATUS_MAX_CHANGED {
            kept.push(line.to_string());
            changed += 1;
        } else {
            omitted_changed += 1;
        }
    }
    if omitted_changed > 0 {
        kept.push(format!("…[{omitted_changed} more changed files]"));
    }
    if omitted_untracked > 0 {
        kept.push(format!("…[{omitted_untracked} more untracked files]"));
    }
    joined(kept)
}

/// diff/show：只保留文件头、hunk 头与增删行，丢弃上下文。
fn filter_diff(output: &str) -> String {
    let mut kept: Vec<String> = Vec::new();
    for line in output.lines() {
        let keep = line.starts_with("diff --git")
            || line.starts_with("index ")
            || line.starts_with("--- ")
            || line.starts_with("+++ ")
            || line.starts_with("@@")
            || line.starts_with('+')
            || line.starts_with('-')
            || line.starts_with("commit ")
            || line.starts_with("Author:")
            || line.starts_with("Date:")
            || line.starts_with("new file mode")
            || line.starts_with("deleted file mode")
            || line.starts_with("rename from")
            || line.starts_with("rename to");
        if keep {
            kept.push(line.to_string());
        }
    }
    if kept.is_empty() {
        return filter_generic(output);
    }
    cap_lines(kept, MAX_DIFF_LINES, "lines")
}

fn filter_log(output: &str) -> String {
    let mut lines: Vec<String> = output.lines().map(str::to_string).collect();
    if lines.len() <= MAX_LOG_LINES {
        return joined(lines);
    }
    let omitted = lines.len() - MAX_LOG_LINES;
    lines.truncate(MAX_LOG_LINES);
    lines.push(format!("…[{omitted} older commits omitted]"));
    joined(lines)
}

/// grep/rg：按文件限制条数、总条数，行内超长截断。
fn filter_grep(output: &str) -> String {
    let mut kept: Vec<String> = Vec::new();
    let mut per_file: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut omitted = 0usize;
    let mut total = 0usize;
    for line in output.lines() {
        if line.trim().is_empty() {
            continue;
        }
        if total >= GREP_MAX_RESULTS {
            omitted += 1;
            continue;
        }
        let file = line.split(':').next().unwrap_or_default().to_string();
        let count = per_file.entry(file).or_insert(0);
        if *count >= GREP_MAX_PER_FILE {
            omitted += 1;
            continue;
        }
        *count += 1;
        total += 1;
        kept.push(truncate_chars(line, 300));
    }
    if omitted > 0 {
        kept.push(format!("…[{omitted} matches omitted]"));
    }
    joined(kept)
}

fn filter_list(output: &str) -> String {
    let lines: Vec<String> = output.lines().map(str::to_string).collect();
    cap_lines(lines, MAX_LIST_LINES, "entries")
}

/// 通用：短输出原样；长输出保留行首尾并显式标注省略，最后再按字符兜底。
fn filter_generic(output: &str) -> String {
    let normalized = collapse_blank_lines(output);
    if normalized.chars().count() <= MAX_GENERIC_CHARS {
        return normalized;
    }
    let lines: Vec<String> = normalized.lines().map(str::to_string).collect();
    let mut text = if lines.len() > GENERIC_HEAD_LINES + GENERIC_TAIL_LINES {
        let omitted = lines.len() - GENERIC_HEAD_LINES - GENERIC_TAIL_LINES;
        let mut kept: Vec<String> = lines[..GENERIC_HEAD_LINES].to_vec();
        kept.push(format!("…[{omitted} lines omitted]"));
        kept.extend_from_slice(&lines[lines.len() - GENERIC_TAIL_LINES..]);
        joined(kept)
    } else {
        normalized
    };
    if text.len() > GENERIC_MAX_BYTES {
        let head = truncate_bytes(&text, GENERIC_MAX_BYTES / 2);
        let tail = tail_chars(&text, GENERIC_MAX_BYTES / 2);
        text = format!("{head}\n…[output truncated]\n{tail}");
    }
    text
}

/// 行级上限：保留前 `max-1` 行并报告省略数。
fn cap_lines(lines: Vec<String>, max: usize, unit: &str) -> String {
    if lines.len() <= max {
        return joined(lines);
    }
    let omitted = lines.len() - (max - 1);
    let mut kept: Vec<String> = lines.into_iter().take(max - 1).collect();
    kept.push(format!("…[{omitted} {unit} omitted]"));
    joined(kept)
}

fn joined(lines: Vec<String>) -> String {
    if lines.is_empty() {
        return String::new();
    }
    let mut out = lines.join("\n");
    out.push('\n');
    out
}

/// 连续空行折叠为一行空行。
fn collapse_blank_lines(output: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    let mut blank = false;
    for line in output.lines() {
        if line.trim().is_empty() {
            if !blank {
                out.push("");
            }
            blank = true;
        } else {
            out.push(line);
            blank = false;
        }
    }
    while out.last().is_some_and(|line| line.is_empty()) {
        out.pop();
    }
    let mut text = out.join("\n");
    if !text.is_empty() {
        text.push('\n');
    }
    text
}

/// 剥离 CSI/OSC 等 ANSI 转义序列（输出已经过进程层净化时是 no-op）。
fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' {
            match chars.peek() {
                Some('[') => {
                    chars.next();
                    for next in chars.by_ref() {
                        if ('@'..='~').contains(&next) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    chars.next();
                    for next in chars.by_ref() {
                        if next == '\u{7}' {
                            break;
                        }
                    }
                }
                _ => {
                    let _ = chars.next();
                }
            }
            continue;
        }
        if ch == '\r' {
            continue;
        }
        if ch == '\n' || ch == '\t' || !ch.is_control() {
            out.push(ch);
        }
    }
    out
}

/// 按字符截断并加省略号。
fn truncate_chars(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        return value.to_string();
    }
    let mut out: String = value.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// 按字节上限截断到 UTF-8 边界。
fn truncate_bytes(value: &str, max: usize) -> String {
    if value.len() <= max {
        return value.to_string();
    }
    let mut end = max;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_string()
}

/// 取末尾 N 字节（UTF-8 边界）。
fn tail_chars(value: &str, max: usize) -> String {
    if value.len() <= max {
        return value.to_string();
    }
    let mut start = value.len() - max;
    while start < value.len() && !value.is_char_boundary(start) {
        start += 1;
    }
    value[start..].to_string()
}
