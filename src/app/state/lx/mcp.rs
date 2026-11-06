//! `/mcp` 命令面板状态与工作区 MCP 目录快照。
//!
//! 纯状态层：不直接读取磁盘配置或访问 `McpManager`；
//! 由事件层在启动、配置变更与 `McpEvent::Status` 到达时同步 `McpCatalogEntry`，
//! 并结合每个窗格的 `Lx::disabled_mcp_servers` 投影为面板条目。

use std::collections::BTreeSet;

use super::slash::{PanelWindow, best_score};
use crate::ui::text;

/// 工作区单个 MCP server 的底层运行状态（跨同项目窗格共享）。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum McpRuntimeState {
    /// 正在启动或重连中。
    #[default]
    Connecting,
    /// 已连接就绪（携带可用工具数量）。
    Ready { tools: usize },
    /// 启动或运行失败（携带错误原因）。
    Failed { error: String },
    /// 需要 OAuth 登录授权。
    NeedsLogin,
}

/// 工作区生效配置中的一条 MCP server 目录项（存入 `AppState::mcp_catalog`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpCatalogEntry {
    /// MCP server 唯一名称。
    pub name: String,
    /// 传输摘要（如 `stdio: npx …` 或 `https://…`）。
    pub summary: String,
    /// 配置文件（`mcp.json`）中的 `enabled` 字段。
    pub config_enabled: bool,
    /// 底层运行状态（仅 `config_enabled == true` 时有意义）。
    pub runtime: McpRuntimeState,
}

/// 单个 MCP server 在特定窗格 `/mcp` 面板中的展示状态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpServerState {
    /// 在本窗格启用且底层已就绪。
    Connected { tools: usize },
    /// 在本窗格启用且底层正在连接。
    Connecting,
    /// 在本窗格启用但底层连接失败。
    Failed { error: String },
    /// 在本窗格启用但需要 OAuth 登录。
    NeedsLogin,
    /// 在当前窗格被手动禁用（底层连接保持不变）。
    DisabledInPane,
    /// 在全局/项目设置中被禁用（`mcp.json` 的 `enabled: false`），本窗格只读置灰。
    DisabledInSettings,
}

/// `/mcp` 面板中的单行条目。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpPanelEntry {
    pub name: String,
    pub summary: String,
    pub preview: String,
    pub state: McpServerState,
}

impl McpPanelEntry {
    /// 由工作区目录项与当前窗格禁用集合构造面板条目。
    pub fn from_catalog(entry: &McpCatalogEntry, pane_disabled: &BTreeSet<String>) -> Self {
        let state = if !entry.config_enabled {
            McpServerState::DisabledInSettings
        } else if pane_disabled.contains(&entry.name) {
            McpServerState::DisabledInPane
        } else {
            match &entry.runtime {
                McpRuntimeState::Ready { tools } => McpServerState::Connected { tools: *tools },
                McpRuntimeState::Connecting => McpServerState::Connecting,
                McpRuntimeState::Failed { error } => McpServerState::Failed {
                    error: sanitize_single_line(error),
                },
                McpRuntimeState::NeedsLogin => McpServerState::NeedsLogin,
            }
        };
        let preview = build_preview(&entry.summary, &state);
        Self {
            name: entry.name.clone(),
            summary: entry.summary.clone(),
            preview,
            state,
        }
    }

    /// 是否因全局/项目设置禁用而在面板中置灰只读。
    pub fn is_settings_disabled(&self) -> bool {
        matches!(self.state, McpServerState::DisabledInSettings)
    }

    /// 右对齐状态标签文案。
    pub fn status_tag(&self) -> &'static str {
        match &self.state {
            McpServerState::Connected { .. } => text::LX_MCP_STATUS_CONNECTED,
            McpServerState::Connecting => text::LX_MCP_STATUS_CONNECTING,
            McpServerState::Failed { .. } => text::LX_MCP_STATUS_FAILED,
            McpServerState::NeedsLogin => text::LX_MCP_STATUS_NEEDS_LOGIN,
            McpServerState::DisabledInPane => text::LX_MCP_STATUS_DISABLED,
            McpServerState::DisabledInSettings => text::LX_MCP_STATUS_DISABLED_SETTINGS,
        }
    }
}

fn sanitize_single_line(raw: &str) -> String {
    let stripped = strip_ansi_and_controls(raw);
    stripped
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or_default()
        .to_string()
}

fn strip_ansi_and_controls(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' {
            if chars.peek() == Some(&'[') {
                let _ = chars.next();
                for next in chars.by_ref() {
                    if ('@'..='~').contains(&next) {
                        break;
                    }
                }
            }
            continue;
        }
        if ch == '\n' || ch == '\r' {
            out.push('\n');
        } else if ch == '\t' {
            out.push(' ');
        } else if !ch.is_control() {
            out.push(ch);
        }
    }
    out
}

fn build_preview(summary: &str, state: &McpServerState) -> String {
    match state {
        McpServerState::Connected { tools } => {
            let tool_label = if *tools == 1 {
                "1 tool".to_string()
            } else {
                format!("{tools} tools")
            };
            if summary.is_empty() {
                tool_label
            } else {
                format!("{tool_label} · {summary}")
            }
        }
        McpServerState::Failed { error } if !error.is_empty() => error.clone(),
        _ => summary.to_string(),
    }
}

/// `/mcp` 窗格内联面板状态。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct McpPanel {
    /// 当前工作区的全部 MCP 条目（按名称字典序）。
    pub entries: Vec<McpPanelEntry>,
    /// 滚动与高亮窗口。
    pub window: PanelWindow,
    /// 当前过滤词（来自输入框全文）。
    pub filter: String,
    /// 过滤后的条目在 `entries` 中的索引。
    pub filtered: Vec<usize>,
}

impl McpPanel {
    /// 以初始条目列表创建面板（默认高亮第 0 项，不过滤）。
    pub fn new(entries: Vec<McpPanelEntry>) -> Self {
        let len = entries.len();
        Self {
            entries,
            window: PanelWindow::new(0),
            filter: String::new(),
            filtered: (0..len).collect(),
        }
    }

    /// 同步最新条目（原地切换或后台事件到达时调用），保留当前高亮的 server 名与过滤词。
    pub fn sync_entries(&mut self, entries: Vec<McpPanelEntry>) {
        let active_name = self.active_entry().map(|entry| entry.name.clone());
        self.entries = entries;
        let query = self.filter.clone();
        self.apply_filter(&query);
        if let Some(name) = active_name
            && let Some(pos) = self
                .filtered
                .iter()
                .position(|&idx| self.entries.get(idx).is_some_and(|e| e.name == name))
        {
            self.window.set_active(pos);
        } else {
            self.window = PanelWindow::default();
        }
        self.clamp_window();
    }

    /// 按输入框全文重算过滤结果。
    pub fn refilter(&mut self, query: &str) {
        if self.filter != query {
            self.filter = query.to_string();
            self.window = PanelWindow::default();
        }
        self.apply_filter(query);
    }

    fn apply_filter(&mut self, query: &str) {
        if query.trim().is_empty() {
            self.filtered = (0..self.entries.len()).collect();
            self.clamp_window();
            return;
        }
        let query = query.trim().to_lowercase();
        let mut scored: Vec<(u32, usize)> = self
            .entries
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| {
                best_score(&[&entry.name, &entry.summary, entry.status_tag()], &query)
                    .map(|score| (score, index))
            })
            .collect();
        scored.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.cmp(&right.1)));
        self.filtered = scored.into_iter().map(|(_, index)| index).collect();
        self.clamp_window();
    }

    /// 过滤后可见条目列表（供渲染与命中使用）。
    pub fn visible(&self) -> Vec<&McpPanelEntry> {
        self.filtered
            .iter()
            .filter_map(|index| self.entries.get(*index))
            .collect()
    }

    /// 当前高亮条目。
    pub fn active_entry(&self) -> Option<&McpPanelEntry> {
        self.filtered
            .get(self.window.active)
            .and_then(|index| self.entries.get(*index))
    }

    /// 钳制窗口范围。
    pub fn clamp_window(&mut self) {
        self.window.clamp(self.filtered.len());
    }

    /// 是否有过滤词但无匹配结果。
    pub fn is_filter_empty(&self) -> bool {
        !self.filter.trim().is_empty() && self.filtered.is_empty()
    }
}

#[cfg(test)]
mod tests;
