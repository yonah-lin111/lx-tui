//! `AGENTS.md` / `agent.md` 双层系统提示词发现、默认文件初始化与 `<INSTRUCTIONS>` 格式化。
//!
//! 发现规则对齐 Codex (`codex-rs/codex-home/src/instructions/mod.rs` 与 `codex-rs/core/src/agents_md.rs`)：
//! 1. 全局层：用户数据根（生产 `~/.lx-t`，开发 `~/.lx-t-dev`）。若不存在任何候选文件，
//!    自动初始化空文件 `AGENTS.md`。
//! 2. 项目层：仅在工作区已信任（`workspace_trusted == true`）时，从 `project_root`（或最近
//!    的 `.git` 边界）向下到当前 `cwd` 逐级扫描，共享 32 KiB 字节预算。
//! 3. 目录内优先级：`AGENTS.override.md` > `AGENTS.md` > `agent.md` > `agents.md`，取第一个
//!    非空普通文件。

use std::io::Read;
use std::path::{Path, PathBuf};

/// 目录内最高优先级的本地覆盖指令文件名（对齐 Codex）。
pub const LOCAL_AGENTS_MD_FILENAME: &str = "AGENTS.override.md";
/// 标准指令文件名（也是全局目录默认初始化的文件名）。
pub const DEFAULT_AGENTS_MD_FILENAME: &str = "AGENTS.md";
/// 目录内按优先级探测的候选文件名列表。
pub const CANDIDATE_AGENT_FILENAMES: [&str; 4] =
    ["AGENTS.override.md", "AGENTS.md", "agent.md", "agents.md"];
/// 项目级文档共享字节预算上限（32 KiB，对齐 Codex `DEFAULT_PROJECT_DOC_MAX_BYTES`）。
pub const MAX_PROJECT_DOC_BYTES: usize = 32 * 1024;
/// 全局用户级文档字节预算上限（16 KiB，独立于项目预算）。
pub const MAX_GLOBAL_DOC_BYTES: usize = 16 * 1024;
/// 全局与项目级文档之间的分隔符（对齐 Codex `AGENTS_MD_SEPARATOR`）。
pub const AGENTS_MD_SEPARATOR: &str = "\n\n--- project-doc ---\n\n";
/// 文档超出字节预算被截断时追加的显式标记（与工具结果回放同一形态）。
const TRUNCATED_MARKER: &str = "\n…[truncated]";

/// 已加载并合并的 `AGENTS.md` 指令快照（在 turn 起点冻结）。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LoadedInstructions {
    /// 命中的全局指令文件路径（非空时记录）。
    pub global_source: Option<PathBuf>,
    /// 命中的项目级指令文件路径列表（按从根到 `cwd` 顺序）。
    pub project_sources: Vec<PathBuf>,
    /// 当前工作目录（用于标题行 `# AGENTS.md instructions for <directory>`）。
    pub directory: Option<PathBuf>,
    /// 合并后的指令正文（未包裹外层 `# AGENTS.md instructions` 与 `<INSTRUCTIONS>`）。
    pub body: String,
}

impl LoadedInstructions {
    /// 是否没有任何有效指令内容。
    pub fn is_empty(&self) -> bool {
        self.body.trim().is_empty()
    }

    /// 命中的来源简短标签列表（用于模型切换提示词与详情展示）。
    pub fn source_labels(&self) -> Vec<String> {
        let mut labels = Vec::new();
        if let Some(path) = &self.global_source {
            labels.push(format!("global ({})", path.display()));
        }
        for path in &self.project_sources {
            labels.push(path.display().to_string());
        }
        labels
    }

    /// 渲染为注入系统提示词的完整英文段落（对齐 Codex `UserInstructions`）。
    pub fn render_prompt_section(&self) -> Option<String> {
        if self.is_empty() {
            return None;
        }
        let header = match &self.directory {
            Some(dir) if !self.project_sources.is_empty() => {
                format!("# AGENTS.md instructions for {}", dir.display())
            }
            _ => "# AGENTS.md instructions".to_string(),
        };
        Some(format!(
            "{header}\n\n<INSTRUCTIONS>\n{}\n</INSTRUCTIONS>",
            self.body.trim()
        ))
    }
}

/// 确保用户数据根目录（`~/.lx-t` 或 `~/.lx-t-dev`）下存在指令文件。
///
/// 若目录下已存在任一候选文件（含空文件或大小写变体），直接返回其路径，不覆盖用户文件；
/// 若均不存在，原子创建空文件 `AGENTS.md`。
pub fn ensure_global_agents_md(user_root: &Path) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(user_root)?;
    let existing = list_candidate_paths_in_dir(user_root);
    if let Some(first) = existing.into_iter().next() {
        return Ok(first);
    }
    let target = user_root.join(DEFAULT_AGENTS_MD_FILENAME);
    if !target.exists() {
        crate::platform::write_file_atomic(&target, b"")?;
    }
    Ok(target)
}

/// 加载并合并全局 `~/.lx-t[-dev]` 与项目级（`project_root -> cwd`）的 `AGENTS.md` / `agent.md` 指令。
pub fn load_instructions(
    user_root: Option<&Path>,
    project_root: Option<&Path>,
    cwd: Option<&Path>,
    workspace_trusted: bool,
) -> LoadedInstructions {
    let mut global_source = None;
    let mut global_text = String::new();

    if let Some(root) = user_root {
        let _ = ensure_global_agents_md(root);
        if let Some((path, text, _)) = read_first_non_empty_in_dir(root, MAX_GLOBAL_DOC_BYTES) {
            global_source = Some(path);
            global_text = text;
        }
    }

    let mut project_sources = Vec::new();
    let mut project_texts = Vec::new();
    if workspace_trusted {
        let dirs = collect_search_dirs(project_root, cwd);
        let mut remaining = MAX_PROJECT_DOC_BYTES;
        for dir in dirs {
            if remaining == 0 {
                break;
            }
            if let Some((path, text, used)) = read_first_non_empty_in_dir(&dir, remaining) {
                remaining = remaining.saturating_sub(used);
                project_sources.push(path);
                project_texts.push(text);
            }
        }
    }

    let mut body = String::new();
    if !global_text.is_empty() {
        body.push_str(&global_text);
    }
    for (idx, text) in project_texts.iter().enumerate() {
        if !body.is_empty() {
            if idx == 0 && !global_text.is_empty() {
                body.push_str(AGENTS_MD_SEPARATOR);
            } else {
                body.push_str("\n\n");
            }
        }
        body.push_str(text);
    }

    LoadedInstructions {
        global_source,
        project_sources,
        directory: cwd.or(project_root).map(Path::to_path_buf),
        body,
    }
}

/// 收集从项目根（或最近 `.git` 边界）向下到 `cwd` 的目录链。
fn collect_search_dirs(project_root: Option<&Path>, cwd: Option<&Path>) -> Vec<PathBuf> {
    let Some(start) = cwd.or(project_root) else {
        return Vec::new();
    };
    let Some(root) = project_root else {
        return vec![start.to_path_buf()];
    };
    // 软链（如 macOS `/tmp` -> `/private/tmp`）会让字面前缀比较失败：两侧规范化后重试，
    // 仍然不在祖先链上才退化为只扫描 `cwd` 单目录。
    let (walk_start, walk_root) = if start.starts_with(root) {
        (start.to_path_buf(), root.to_path_buf())
    } else {
        match (std::fs::canonicalize(start), std::fs::canonicalize(root)) {
            (Ok(canon_start), Ok(canon_root)) if canon_start.starts_with(&canon_root) => {
                (canon_start, canon_root)
            }
            _ => return vec![start.to_path_buf()],
        }
    };

    let mut chain = Vec::new();
    let mut current = Some(walk_start.as_path());
    while let Some(dir) = current {
        chain.push(dir.to_path_buf());
        if dir == walk_root || dir.join(".git").exists() {
            break;
        }
        current = dir.parent();
    }
    chain.reverse();
    chain
}

/// 按候选文件名优先级列出目录内实际存在的普通文件路径（保留磁盘真实大小写文件名）。
fn list_candidate_paths_in_dir(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = Vec::new();
    for entry in entries.flatten() {
        if let Some(name) = entry.file_name().to_str() {
            let path = dir.join(name);
            if std::fs::metadata(&path).is_ok_and(|meta| meta.is_file()) {
                names.push(name.to_string());
            }
        }
    }

    let mut ordered = Vec::new();
    // 第一遍：按精确大小写优先级匹配。
    for candidate in CANDIDATE_AGENT_FILENAMES {
        if let Some(found) = names.iter().find(|name| name.as_str() == candidate) {
            let path = dir.join(found);
            if !ordered.contains(&path) {
                ordered.push(path);
            }
        }
    }
    // 第二遍：大小写不敏感兜底（如 `Agents.md` / `Agent.md`）。
    for candidate in CANDIDATE_AGENT_FILENAMES {
        for found in &names {
            if found.eq_ignore_ascii_case(candidate) {
                let path = dir.join(found);
                if !ordered.contains(&path) {
                    ordered.push(path);
                }
            }
        }
    }
    ordered
}

/// 在单个目录中按优先级读取第一个非空指令文件，最多消耗 `budget` 字节。
fn read_first_non_empty_in_dir(dir: &Path, budget: usize) -> Option<(PathBuf, String, usize)> {
    if budget == 0 {
        return None;
    }
    for path in list_candidate_paths_in_dir(dir) {
        if let Some((text, used)) = read_file_within_budget(&path, budget) {
            return Some((path, text, used));
        }
    }
    None
}

/// 读取文件前 `budget` 字节并在 UTF-8 字符边界安全截断；空白文件返回 `None`。
///
/// 多读 1 字节只用于判定「文件确实超出预算」：超出时必须显式追加 `…[truncated]`，
/// 不能把截断伪装成完整文件交给模型。
fn read_file_within_budget(path: &Path, budget: usize) -> Option<(String, usize)> {
    let file = std::fs::File::open(path).ok()?;
    let mut buffer = Vec::new();
    file.take(budget.saturating_add(1) as u64)
        .read_to_end(&mut buffer)
        .ok()?;
    if buffer.is_empty() {
        return None;
    }
    let truncated = buffer.len() > budget;
    if truncated {
        buffer.truncate(budget);
    }
    let used = buffer.len();
    let valid_text = match std::str::from_utf8(&buffer) {
        Ok(text) => text.to_string(),
        Err(err) => {
            let valid_up_to = err.valid_up_to();
            std::str::from_utf8(&buffer[..valid_up_to])
                .ok()?
                .to_string()
        }
    };
    let trimmed = valid_text.trim();
    if trimmed.is_empty() {
        return None;
    }
    let mut text = trimmed.to_string();
    if truncated {
        text.push_str(TRUNCATED_MARKER);
    }
    Some((text, used))
}
