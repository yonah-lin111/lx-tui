//! 紧凑布局的派生数据：进入前的折叠快照、全局 Agent 条目与切换器行模型。

use super::{AppState, RightTab, SidebarAgentEntry};

/// 进入紧凑布局前的桌面折叠状态；`Some` 表示当前处于紧凑布局。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompactRestore {
    pub prompt_collapsed: bool,
    pub panes_collapsed: bool,
}

/// 紧凑切换器分区。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompactSection {
    Agents,
    Spaces,
    Tabs,
    Panels,
    Actions,
}

/// 跨工作区的 Agent 条目：侧栏条目 + 所属工作区与可选标题。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactAgentRow {
    pub workspace: usize,
    pub entry: SidebarAgentEntry,
    pub title: Option<String>,
}

/// 紧凑切换器的一行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompactRow {
    Section(CompactSection),
    Agent(CompactAgentRow),
    Workspace(usize),
    Tab(usize),
    Panel(RightTab),
    NewWorkspace,
    NewTab,
}

impl CompactRow {
    /// 分区标题行不可选中。
    pub const fn is_selectable(&self) -> bool {
        !matches!(self, Self::Section(_))
    }
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

    /// 所有工作区的 Agent 条目；紧凑切换器使用。
    pub fn global_agent_entries(&self) -> Vec<CompactAgentRow> {
        let mut rows = Vec::new();
        for workspace in 0..self.workspaces.len() {
            for entry in self.agent_entries_for_workspace(workspace) {
                let title = self
                    .pane_anywhere(entry.pane_id)
                    .and_then(|pane| pane.agent_display_title(entry.kind))
                    .map(str::to_string);
                rows.push(CompactAgentRow {
                    workspace,
                    entry,
                    title,
                });
            }
        }
        rows
    }

    /// 紧凑切换器的完整行序：Agents（有条目时）→ Spaces → Tabs → Panels → Actions。
    pub fn compact_rows(&self) -> Vec<CompactRow> {
        let mut rows = Vec::new();

        let agents = self.global_agent_entries();
        if !agents.is_empty() {
            rows.push(CompactRow::Section(CompactSection::Agents));
            rows.extend(agents.into_iter().map(CompactRow::Agent));
        }

        rows.push(CompactRow::Section(CompactSection::Spaces));
        rows.extend((0..self.workspaces.len()).map(CompactRow::Workspace));

        rows.push(CompactRow::Section(CompactSection::Tabs));
        rows.extend((0..self.active_workspace().tabs.len()).map(CompactRow::Tab));

        rows.push(CompactRow::Section(CompactSection::Panels));
        rows.push(CompactRow::Panel(RightTab::Prompt));
        rows.push(CompactRow::Panel(RightTab::Todo));

        rows.push(CompactRow::Section(CompactSection::Actions));
        rows.push(CompactRow::NewWorkspace);
        rows.push(CompactRow::NewTab);

        rows
    }
}

#[cfg(test)]
mod tests;
