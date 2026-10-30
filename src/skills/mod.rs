//! Skills：发现、解析、目录渲染与设置页编辑。
//!
//! 纯文件 IO 模块，不依赖 `chat`/`app`/`ui`/`event`/`settings`。
//! 一个 skill 是包含 `SKILL.md` 的目录；正文只在用户点名时注入当轮上下文。

pub mod discover;
pub mod parse;
pub mod render;
pub mod store;

use std::path::{Path, PathBuf};

pub use discover::{ProjectSkillRoots, catalog_signature, discover};
pub use parse::{ParsedSkill, load_skill_body, parse_skill};
pub use render::{
    render_catalog, render_catalog_with_omitted, render_injection, render_replay_line,
};
pub use store::{delete_skill, read_skill, read_skill_at, serialize_skill, write_skill};

/// skill 文件的标准文件名。
pub const SKILL_FILE_NAME: &str = "SKILL.md";

/// 是否为 skill 文件：允许 `SKILL.md` 大小写变体。
///
/// macOS/Windows 大小写不敏感，用户常写成 `skill.md`；`read_dir` 返回的实际
/// 文件名仍是写入时的 casing，因此比较必须忽略大小写，否则目录里明明有文件、
/// lx 却当成不存在。
pub fn is_skill_file_name(name: &std::ffi::OsStr) -> bool {
    name.to_str()
        .is_some_and(|name| name.eq_ignore_ascii_case(SKILL_FILE_NAME))
}

/// 在一个 skill 目录里找实际存在的 `SKILL.md`（优先标准大写名）。
pub fn find_skill_file(dir: &Path) -> Option<PathBuf> {
    let entries = std::fs::read_dir(dir).ok()?;
    let mut fallback = None;
    for entry in entries.flatten() {
        if !is_skill_file_name(&entry.file_name()) {
            continue;
        }
        // 跟随符号链接，与发现阶段的 `entry.metadata()` 保持同一口径。
        if !entry.metadata().map(|meta| meta.is_file()).unwrap_or(false) {
            continue;
        }
        let path = entry.path();
        if entry.file_name() == std::ffi::OsStr::new(SKILL_FILE_NAME) {
            return Some(path);
        }
        fallback.get_or_insert(path);
    }
    fallback
}

/// `SKILL.md` 读取上限（256 KiB）。
pub const MAX_SKILL_FILE_BYTES: usize = 256 * 1024;
/// 系统提示里 skill 目录的字符预算。
pub const MAX_CATALOG_CHARS: usize = 8000;
/// 单条 description 在目录里的截断长度。
pub const MAX_CATALOG_DESCRIPTION_CHARS: usize = 200;
/// 单个 skill 注入当轮上下文的字节上限。
pub const MAX_SKILL_INJECTION_BYTES: usize = 8000;
/// 发现时每个根最多遍历的目录数。
pub const MAX_DIRS_PER_ROOT: usize = 2000;
/// 发现时的最大目录深度。
pub const MAX_DEPTH: usize = 6;

/// skill 作用域。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SkillScope {
    Project,
    User,
}

impl SkillScope {
    /// 稳定字符串值（设置页路由）。
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Project => "project",
            Self::User => "user",
        }
    }
}

/// skill 来源：lx 自有目录（可编辑）或 `.agents` 目录（只读）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SkillSource {
    Lx,
    Agents,
}

impl SkillSource {
    /// 稳定字符串值（设置页展示）。
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Lx => "lx",
            Self::Agents => "agents",
        }
    }

    /// lx 自有目录可编辑；`.agents` 只读。
    pub const fn editable(self) -> bool {
        matches!(self, Self::Lx)
    }
}

/// 一个已发现的 skill。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillEntry {
    pub name: String,
    pub description: String,
    /// canonical 绝对路径（`SKILL.md`）。
    pub path: PathBuf,
    pub scope: SkillScope,
    pub source: SkillSource,
    pub enabled: bool,
    /// 同名条目被哪个更优先的 `SKILL.md` 覆盖。
    pub shadowed_by: Option<PathBuf>,
}

static CACHE: std::sync::RwLock<Vec<SkillEntry>> = std::sync::RwLock::new(Vec::new());
static CACHE_ERRORS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// 最近一次扫描的 skill 条目（纯数据；app 层只读它做 `@` 面板候选）。
pub fn cached_entries() -> Vec<SkillEntry> {
    CACHE
        .read()
        .map(|entries| entries.clone())
        .unwrap_or_default()
}

/// 最近一次扫描的解析错误数（首页计数用）。
pub fn cached_errors() -> usize {
    CACHE_ERRORS.load(std::sync::atomic::Ordering::Relaxed)
}

/// 事件层每次扫描后更新缓存。
pub fn set_cached_entries(entries: Vec<SkillEntry>) {
    set_cached_summary(entries, 0);
}

/// 事件层扫描后同时更新条目与解析错误数。
pub fn set_cached_summary(entries: Vec<SkillEntry>, errors: usize) {
    if let Ok(mut cache) = CACHE.write() {
        *cache = entries;
    }
    CACHE_ERRORS.store(errors, std::sync::atomic::Ordering::Relaxed);
}

/// 发现结果：可用条目 + 解析错误。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SkillCatalog {
    pub entries: Vec<SkillEntry>,
    pub errors: Vec<(PathBuf, String)>,
}

impl SkillCatalog {
    /// 可编辑的来源标记（设置页用）。
    pub fn editable(&self, entry: &SkillEntry) -> bool {
        entry.source.editable()
    }
}
