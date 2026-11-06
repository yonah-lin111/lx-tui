//! `/mcp` 面板打开、原地切换与重连分派。

use std::path::PathBuf;

use crate::app::effects::Effect;
use crate::app::state::lx::mcp::{McpPanel, McpPanelEntry, McpRuntimeState, McpServerState};
use crate::app::state::lx::slash::SlashPanel;
use crate::app::state::{AppState, InputHintLevel};
use crate::layout::PaneId;
use crate::ui::text;

use super::session::show_lx_hint;

/// 解析窗格所属的 MCP 配置根（仓库主 checkout 优先，回退工作区根）。
pub fn mcp_root_for_pane(state: &AppState, pane: PaneId) -> Option<PathBuf> {
    state
        .chat_session_key_for_pane(pane)
        .or_else(|| state.active_workspace().repo_key().map(PathBuf::from))
}

/// 按当前 `state.mcp_catalog` 与窗格禁用集构建 `McpPanelEntry` 列表。
pub fn build_panel_entries(state: &AppState, pane: PaneId) -> Vec<McpPanelEntry> {
    let Some(root) = mcp_root_for_pane(state, pane) else {
        return Vec::new();
    };
    let Some(catalog) = state.mcp_catalog.get(&root) else {
        return Vec::new();
    };
    let Some(target) = state.pane_anywhere(pane) else {
        return Vec::new();
    };
    let disabled = target.lx.disabled_mcp_servers();
    catalog
        .iter()
        .map(|entry| McpPanelEntry::from_catalog(entry, disabled))
        .collect()
}

/// `/mcp` 打开窗格内联 MCP 管理面板。
pub fn open_mcp_panel(state: &mut AppState, pane: PaneId) -> bool {
    if !state
        .pane_anywhere(pane)
        .is_some_and(|target| target.view == crate::app::state::PaneView::Lx)
    {
        return false;
    }
    let entries = build_panel_entries(state, pane);
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    target.lx.slash = Some(SlashPanel::Mcp(Box::new(McpPanel::new(entries))));
    state.effects.push(Effect::RefreshMcpCatalog { pane });
    true
}

/// 在 `/mcp` 面板中对当前高亮项执行切换或重连（Enter / Space / 鼠标点击），面板保持打开。
pub fn toggle_mcp_entry(state: &mut AppState, pane: PaneId) -> bool {
    let Some(active) = state
        .pane_anywhere(pane)
        .and_then(|target| match &target.lx.slash {
            Some(SlashPanel::Mcp(panel)) => panel.active_entry().cloned(),
            _ => None,
        })
    else {
        return true;
    };

    match active.state {
        McpServerState::DisabledInSettings => {
            show_lx_hint(
                state,
                pane,
                InputHintLevel::Warning,
                text::LX_MCP_DISABLED_IN_SETTINGS,
            );
            return true;
        }
        McpServerState::Failed { .. } => {
            // 当前窗格已启用但底层连接失败：触发后台重连，状态立即切为 Connecting。
            mark_catalog_connecting(state, pane, &active.name);
            state.effects.push(Effect::RestartMcpServer {
                pane,
                server: active.name,
            });
        }
        McpServerState::DisabledInPane => {
            // 重新在当前窗格启用；若底层此前处于 Failed，顺带触发后台重连。
            if let Some(target) = state.pane_mut_anywhere(pane) {
                target.lx.set_mcp_server_disabled(&active.name, false);
            }
            if is_catalog_failed(state, pane, &active.name) {
                mark_catalog_connecting(state, pane, &active.name);
                state.effects.push(Effect::RestartMcpServer {
                    pane,
                    server: active.name,
                });
            }
        }
        McpServerState::Connected { .. }
        | McpServerState::Connecting
        | McpServerState::NeedsLogin => {
            // 在当前窗格禁用（不杀后台连接）。
            if let Some(target) = state.pane_mut_anywhere(pane) {
                target.lx.set_mcp_server_disabled(&active.name, true);
            }
        }
    }

    refresh_pane_mcp_panel(state, pane);
    state.sync_lx_inventory();
    true
}

fn is_catalog_failed(state: &AppState, pane: PaneId, server: &str) -> bool {
    let Some(root) = mcp_root_for_pane(state, pane) else {
        return false;
    };
    state
        .mcp_catalog
        .get(&root)
        .and_then(|entries| entries.iter().find(|entry| entry.name == server))
        .is_some_and(|entry| matches!(entry.runtime, McpRuntimeState::Failed { .. }))
}

fn mark_catalog_connecting(state: &mut AppState, pane: PaneId, server: &str) {
    let Some(root) = mcp_root_for_pane(state, pane) else {
        return;
    };
    if let Some(entries) = state.mcp_catalog.get_mut(&root)
        && let Some(entry) = entries.iter_mut().find(|entry| entry.name == server)
    {
        entry.runtime = McpRuntimeState::Connecting;
    }
}

/// 刷新指定窗格已打开的 `SlashPanel::Mcp` 条目（保持高亮项与过滤词）。
pub fn refresh_pane_mcp_panel(state: &mut AppState, pane: PaneId) -> bool {
    let is_mcp_open = state
        .pane_anywhere(pane)
        .is_some_and(|target| matches!(target.lx.slash, Some(SlashPanel::Mcp(_))));
    if !is_mcp_open {
        return false;
    }
    let entries = build_panel_entries(state, pane);
    if let Some(target) = state.pane_mut_anywhere(pane)
        && let Some(SlashPanel::Mcp(panel)) = target.lx.slash.as_mut()
    {
        panel.sync_entries(entries);
        return true;
    }
    false
}

/// 刷新所有打开了 `SlashPanel::Mcp` 的窗格。
pub fn refresh_all_open_mcp_panels(state: &mut AppState) -> bool {
    let panes = state.all_pane_ids();
    let mut changed = false;
    for pane in panes {
        changed |= refresh_pane_mcp_panel(state, pane);
    }
    changed
}
