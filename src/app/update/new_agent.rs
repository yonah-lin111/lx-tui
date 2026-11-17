//! 侧栏 Agents 新建入口：已安装 CLI Agent 弹窗的状态、按键与创建。

use std::time::Instant;

use crate::app::actions::OverlayKey;
use crate::app::overlay::{NewAgentDialog, Overlay};
use crate::app::state::AppState;

use super::menu::close_overlay;
use super::workspace::create_agent_tab;

/// 打开 New agent 弹窗：校验目标工作区并快照当前安装列表。
///
/// 安装列表由事件层在右键 Agents 分区时扫描；`app/` 不做 IO。
pub fn open_new_agent_dialog(state: &mut AppState, workspace: usize) {
    if state.workspaces.get(workspace).is_none() {
        return;
    }
    state.new_agent_click = None;
    state.new_agent_scroll_drag = None;
    state.overlay = Some(Overlay::NewAgent(NewAgentDialog::new(
        workspace,
        state.installed_agents.clone(),
    )));
}

/// 上下移动弹窗高亮（按搜索过滤后的列表）；无匹配 no-op。返回是否变化。
pub fn move_new_agent_selection(state: &mut AppState, delta: isize) -> bool {
    let Some(Overlay::NewAgent(dialog)) = state.overlay.as_mut() else {
        return false;
    };
    dialog.move_selection(delta)
}

/// 点选弹窗某一行；越界 no-op。返回是否变化。
pub fn set_new_agent_selection(state: &mut AppState, index: usize) -> bool {
    let Some(Overlay::NewAgent(dialog)) = state.overlay.as_mut() else {
        return false;
    };
    dialog.select(index)
}

/// 弹窗行双击判定：同一行在窗口内第二次点击返回 true，并清空现场避免三击连发。
pub fn new_agent_double_click(state: &mut AppState, index: usize, now: Instant) -> bool {
    if !matches!(state.overlay, Some(Overlay::NewAgent(_))) {
        return false;
    }
    let double = state.new_agent_click.is_some_and(|(last, at)| {
        last == index
            && now.saturating_duration_since(at) <= crate::app::update::PROMPT_DOUBLE_CLICK_WINDOW
    });
    state.new_agent_click = if double { None } else { Some((index, now)) };
    double
}

/// 创建当前高亮的 Agent：关闭弹窗并新建终端标签，PTY 就绪后写入启动命令。
///
/// 空列表或目标工作区已消失时不留下半成品。
pub fn create_selected_agent(state: &mut AppState) {
    let Some(Overlay::NewAgent(dialog)) = state.overlay.as_ref() else {
        return;
    };
    let Some(agent) = dialog.selected_agent().cloned() else {
        return;
    };
    let workspace = dialog.workspace;
    state.overlay = None;
    state.new_agent_click = None;
    state.new_agent_scroll_drag = None;
    create_agent_tab(state, workspace, &agent.command);
}

/// 滚轮滚动 New agent 弹窗列表视口；返回是否变化。
pub fn scroll_new_agent(state: &mut AppState, delta: isize, start: usize, visible: usize) -> bool {
    let Some(Overlay::NewAgent(dialog)) = state.overlay.as_mut() else {
        return false;
    };
    dialog.scroll_viewport(delta, start, visible)
}

/// 直接设置 New agent 弹窗列表视口起点；返回是否变化。
pub fn set_new_agent_viewport(state: &mut AppState, offset: usize, visible: usize) -> bool {
    let Some(Overlay::NewAgent(dialog)) = state.overlay.as_mut() else {
        return false;
    };
    dialog.set_viewport(offset, visible)
}

/// New agent 弹窗按键：Esc 关闭、上下移动、Enter 创建；其余按键编辑搜索框，
/// 文本变化后高亮回到过滤结果首行并清除双击判定。
pub fn apply_new_agent_key(state: &mut AppState, key: OverlayKey) {
    match key {
        OverlayKey::Esc => close_overlay(state),
        OverlayKey::Up => {
            move_new_agent_selection(state, -1);
        }
        OverlayKey::Down => {
            move_new_agent_selection(state, 1);
        }
        OverlayKey::Enter => create_selected_agent(state),
        _ => {
            let changed = {
                let Some(Overlay::NewAgent(dialog)) = state.overlay.as_mut() else {
                    return;
                };
                let before = dialog.query.text().to_string();
                if dialog.query.apply_edit_key(key) && dialog.query.text() != before {
                    dialog.normalize_selection();
                    true
                } else {
                    false
                }
            };
            if changed {
                state.new_agent_click = None;
                state.new_agent_scroll_drag = None;
            }
        }
    }
}

#[cfg(test)]
mod tests;
