//! 紧凑布局的派生数据：进入前的折叠快照与全屏面板。

use super::{AppState, SidebarAgentEntry};

/// 进入紧凑布局前的桌面折叠状态；`Some` 表示当前处于紧凑布局。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompactRestore {
    pub sidebar_collapsed: bool,
    pub prompt_collapsed: bool,
    pub panes_collapsed: bool,
}

/// 紧凑布局下由焦点推导出的全屏面板。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompactPanel {
    /// 左侧栏：工作区与 Agents。
    Left,
    /// 右侧栏：Prompt 或 Todo（具体页由 `right_tab` 决定）。
    Right,
}

impl AppState {
    /// 指定工作区的 Agent 条目（按标签与窗格树序；同一窗格先 lx、再终端外部 Agent）。
    pub fn agent_entries_for_workspace(&self, workspace_index: usize) -> Vec<SidebarAgentEntry> {
        let Some(workspace) = self.workspaces.get(workspace_index) else {
            return Vec::new();
        };
        let mut entries = Vec::new();
        for (tab_index, tab) in workspace.tabs.iter().enumerate() {
            for (pane_index, pane_id) in tab.layout.pane_ids().into_iter().enumerate() {
                let Some(pane) = tab.pane(pane_id) else {
                    continue;
                };
                if pane.lx.has_active_session() {
                    entries.push(SidebarAgentEntry {
                        tab_index,
                        pane_index,
                        pane_id,
                        kind: super::SidebarAgentKind::Lx {
                            entry: pane.lx.entry,
                            ai_web_site: pane.lx.ai_web_site,
                        },
                        state: pane.lx.agent_state(),
                    });
                }
                if let Some(snapshot) = pane.agent.as_ref() {
                    entries.push(SidebarAgentEntry {
                        tab_index,
                        pane_index,
                        pane_id,
                        kind: super::SidebarAgentKind::Terminal(snapshot.kind),
                        state: snapshot.state,
                    });
                }
            }
        }
        entries
    }
}

#[cfg(test)]
mod tests;
