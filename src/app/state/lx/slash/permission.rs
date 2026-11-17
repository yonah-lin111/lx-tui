//! `/permission` 一级命令选择面板状态（与 `/model` 对齐：初始定位当前生效项、输入过滤）。

use crate::config::sandbox::SandboxMode;

use super::{PanelWindow, command_score};

/// `/permission` 的固定候选（简短展示名 + 说明）。
pub fn permission_candidates() -> [(SandboxMode, &'static str, &'static str); 3] {
    [
        (
            SandboxMode::ReadOnly,
            SandboxMode::ReadOnly.short_label(),
            crate::ui::text::LX_PERMISSION_READ_ONLY_DESC,
        ),
        (
            SandboxMode::WorkspaceWrite,
            SandboxMode::WorkspaceWrite.short_label(),
            crate::ui::text::LX_PERMISSION_WORKSPACE_DESC,
        ),
        (
            SandboxMode::DangerFullAccess,
            SandboxMode::DangerFullAccess.short_label(),
            crate::ui::text::LX_PERMISSION_FULL_DESC,
        ),
    ]
}

/// `/permission` 选择面板。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionPicker {
    /// 全局默认模式（用于展示 `[Default]` 标签）。
    pub global: SandboxMode,
    /// 当前会话覆盖；None = 跟随全局默认。
    pub session: Option<SandboxMode>,
    /// 当前生效模式在候选列表中的原始索引（`0..3`）。
    pub current: usize,
    /// 高亮与滚动窗口。
    pub window: PanelWindow,
    /// 过滤词（输入框全文快照）；空表示不过滤。
    pub filter: String,
    /// 过滤后的候选索引视图。
    pub filtered: Vec<usize>,
}

impl PermissionPicker {
    /// 构造面板并自动高亮当前生效的权限模式。
    pub fn new(global: SandboxMode, session: Option<SandboxMode>) -> Self {
        let effective = session.unwrap_or(global);
        let candidates = permission_candidates();
        let current = candidates
            .iter()
            .position(|(mode, _, _)| *mode == effective)
            .unwrap_or(1);
        Self {
            global,
            session,
            current,
            window: PanelWindow::new(current),
            filter: String::new(),
            filtered: (0..candidates.len()).collect(),
        }
    }

    /// 当前生效模式。
    pub fn effective(&self) -> SandboxMode {
        self.session.unwrap_or(self.global)
    }

    /// 同步全局默认与会话覆盖模式（设置页热更新或同会话多窗格同步时使用）。
    pub fn sync_modes(&mut self, global: SandboxMode, session: Option<SandboxMode>) -> bool {
        let effective = session.unwrap_or(global);
        let next_current = permission_candidates()
            .iter()
            .position(|(mode, _, _)| *mode == effective)
            .unwrap_or(1);
        if self.global == global && self.session == session && self.current == next_current {
            return false;
        }
        self.global = global;
        self.session = session;
        self.current = next_current;
        true
    }

    /// 按输入框全文过滤候选；空过滤恢复全部并定位回当前生效项。
    pub fn refilter(&mut self, query: &str) {
        let query = query.trim().to_lowercase();
        if self.filter == query {
            return;
        }
        self.filter = query;
        let candidates = permission_candidates();
        if self.filter.is_empty() {
            self.filtered = (0..candidates.len()).collect();
            self.window = PanelWindow::new(self.current);
            return;
        }
        let mut scored: Vec<(u32, usize)> = candidates
            .iter()
            .enumerate()
            .filter_map(|(index, (mode, label, desc))| {
                let best = [
                    command_score(&label.to_lowercase(), &self.filter),
                    command_score(mode.as_str(), &self.filter),
                    command_score(&desc.to_lowercase(), &self.filter),
                ]
                .into_iter()
                .flatten()
                .max()?;
                Some((best, index))
            })
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
        self.filtered = scored.into_iter().map(|(_, index)| index).collect();
        self.window = PanelWindow::new(0);
    }

    /// 当前有过滤词但没有匹配项（空态占位）。
    pub fn is_filter_empty(&self) -> bool {
        !self.filter.is_empty() && self.filtered.is_empty()
    }

    /// 当前高亮模式（按过滤视图换算回原始模式）。
    pub fn active_mode(&self) -> Option<SandboxMode> {
        let real = self.filtered.get(self.window.active).copied()?;
        permission_candidates().get(real).map(|(mode, _, _)| *mode)
    }

    /// 当前生效项在过滤视图中的索引（无匹配时为 None）。
    pub fn current_index(&self) -> Option<usize> {
        self.filtered
            .iter()
            .position(|index| *index == self.current)
    }

    /// 过滤后可见候选数量。
    pub fn candidate_count(&self) -> usize {
        self.filtered.len()
    }
}
