//! 引用文件夹领域模型：项目级磁盘目录引用、内容扫描条目、模糊筛选与新增校验，纯数据无 IO。
//!
//! 与提示词树文件夹完全无关：引用文件夹指向磁盘目录，只用于复制 Markdown 引用与内容搜索；
//! 删除只移除数据库记录，不动磁盘内容或 Prompt 正文。文件系统遍历在事件层后台执行，
//! 本模块只做内存筛选与结果上限保护。

use std::path::PathBuf;

/// 引用文件夹内容扫描的条目上限（性能保护；超限标记截断）。
pub const MAX_SCAN_ENTRIES: usize = 20_000;
/// 搜索结果展示上限（性能保护；超限标记截断）。
pub const MAX_RESULTS: usize = 100;

/// 一条引用文件夹记录：项目级共享，路径为规范化后的绝对路径。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReferencedFolder {
    pub id: i64,
    pub path: PathBuf,
}

impl ReferencedFolder {
    /// 展示名：路径末段；无末段时回退完整路径。
    pub fn display_name(&self) -> String {
        self.path
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| !name.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| self.path.display().to_string())
    }
}

/// 显示中项目的引用文件夹快照；与 prompt 键同步，写成功后整体重载。
#[derive(Debug, Default)]
pub struct ReferencedFolders {
    /// 快照对应的 prompt 键；None 表示尚无归属键。
    pub key: Option<PathBuf>,
    /// 是否已尝试载入（含无持久化或查询失败时的空表）。
    pub loaded: bool,
    /// 载入失败（数据库不可用或查询错误）；菜单应据此展示不可用状态。
    pub failed: bool,
    pub folders: Vec<ReferencedFolder>,
}

impl ReferencedFolders {
    /// 切换到新的归属键：键变化时作废旧快照并标记未载入。
    pub fn reset(&mut self, key: Option<PathBuf>) {
        self.key = key;
        self.loaded = false;
        self.failed = false;
        self.folders.clear();
    }

    /// 用数据库记录替换快照。
    pub fn apply(&mut self, key: PathBuf, folders: Vec<ReferencedFolder>, failed: bool) {
        self.key = Some(key);
        self.loaded = true;
        self.failed = failed;
        self.folders = folders;
    }

    /// 键是否与当前归属一致。
    pub fn matches(&self, key: &std::path::Path) -> bool {
        self.key.as_deref() == Some(key)
    }
}

/// 扫描到的一条文件系统条目：绝对路径、相对根的 `/` 分隔路径、名称与类型。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReferencedEntry {
    pub path: PathBuf,
    /// 相对搜索根的路径（`/` 分隔）；根自身不产生条目。
    pub relative: String,
    /// 路径末段名（用于名称优先匹配）。
    pub name: String,
    pub is_directory: bool,
}

/// 一次目录扫描的纯结果（由事件层后台产生）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReferencedScanOutcome {
    pub entries: Vec<ReferencedEntry>,
    /// 命中扫描条目上限（结果被截断）。
    pub truncated: bool,
}

/// 新增路径的后台校验结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathValidation {
    /// 规范化成功；`canonical` 为稳定身份与复制值。
    Ok { canonical: PathBuf },
    /// 空输入（去空白后为空）。
    Empty,
    /// 非绝对路径。
    NotAbsolute,
    /// 路径不存在或不可访问。
    NotFound,
    /// 路径存在但不是目录。
    NotDirectory,
    /// 无法以引用语法无损表达（含换行或 NUL）。
    NotRepresentable,
    /// 已存在同一条目（规范化后路径重复）。
    Duplicate,
}

/// 一次待执行的后台请求（文件系统侧；DB 写入走 `Effect`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReferencedRequest {
    /// 扫描指定根（打开弹窗或进入子目录）。
    Scan { id: u64, folder: i64, root: PathBuf },
    /// 校验新增路径（需项目快照判重，故不在此携带键）。
    ValidateAdd { id: u64, input: String },
}

/// 关键字 `id` 是否仍为当前活动请求。
pub fn request_is_current(id: u64, current: Option<u64>) -> bool {
    current == Some(id)
}

/// 在内存中按输入筛选扫描条目：大小写不敏感，先名称后路径，稳定排序，同分按路径排序。
///
/// 空查询按路径排序展示全部条目；结果截取展示上限并返回是否溢出。
pub fn filter_entries(entries: &[ReferencedEntry], query: &str) -> (Vec<ReferencedEntry>, bool) {
    let query = query.trim().to_lowercase();
    let mut scored: Vec<(u32, &ReferencedEntry)> = entries
        .iter()
        .filter_map(|entry| {
            let score = entry_score(entry, &query);
            (score > 0).then_some((score, entry))
        })
        .collect();
    scored.sort_by(|left, right| {
        right
            .0
            .cmp(&left.0)
            .then_with(|| left.1.relative.cmp(&right.1.relative))
    });
    let overflow = scored.len() > MAX_RESULTS;
    let items = scored
        .into_iter()
        .take(MAX_RESULTS)
        .map(|(_, entry)| entry.clone())
        .collect();
    (items, overflow)
}

/// 单条目的匹配分；空查询统一给 1（仅排序不筛选），未命中返回 0。
fn entry_score(entry: &ReferencedEntry, query: &str) -> u32 {
    if query.is_empty() {
        return 1;
    }
    let name = entry.name.to_lowercase();
    let relative = entry.relative.to_lowercase();
    if name == query {
        return 6000;
    }
    if name.starts_with(query) {
        return 5000;
    }
    if relative.starts_with(query) {
        return 4000;
    }
    if name.contains(query) {
        return 3000;
    }
    if let Some(span) = subsequence_span(&name, query) {
        return 2000_u32.saturating_sub(span);
    }
    if relative.contains(query) {
        return 1000;
    }
    if let Some(span) = subsequence_span(&relative, query) {
        return 500_u32.saturating_sub(span);
    }
    0
}

/// 子序列匹配的字符跨度（首末距离 + 1）；未命中返回 None。
fn subsequence_span(haystack: &str, needle: &str) -> Option<u32> {
    let mut expected = needle.chars();
    let mut current = expected.next()?;
    let mut first: Option<u32> = None;
    for (position, ch) in haystack.chars().enumerate() {
        if ch != current {
            continue;
        }
        let position = u32::try_from(position).ok()?;
        let start = *first.get_or_insert(position);
        match expected.next() {
            Some(next) => current = next,
            None => return Some(position - start + 1),
        }
    }
    None
}

/// 菜单窗口起点：让选中行落在可见窗口内（不截断记录，仅滚动窗口）。
pub fn menu_window_start(selected: usize, visible: usize, len: usize) -> usize {
    if visible == 0 || len <= visible {
        return 0;
    }
    selected
        .saturating_add(1)
        .saturating_sub(visible)
        .min(len - visible)
}

#[cfg(test)]
mod tests;
