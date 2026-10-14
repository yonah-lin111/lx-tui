//! 侧栏工作区与 Agents 列表滚动、悬停与拖拽排序。

use crate::app::state::{AppState, Workspace};

/// 工作区列表最大滚动偏移（按可见行计）；列表放得下时恒为 0。
pub fn workspace_scroll_max(state: &AppState, visible: usize) -> usize {
    state.workspace_rows().len().saturating_sub(visible.max(1))
}

/// 滚动工作区列表；越界钳制；返回是否变化。
pub fn scroll_workspace_list(state: &mut AppState, delta: isize, visible: usize) -> bool {
    let max = workspace_scroll_max(state, visible);
    let target = (state.workspace_scroll.min(max) as isize).saturating_add(delta);
    let offset = target.clamp(0, max as isize) as usize;
    if offset == state.workspace_scroll {
        return false;
    }
    state.workspace_scroll = offset;
    true
}

/// 直接设置滚动偏移（滚动条点击/拖拽）；越界钳制；返回是否变化。
pub fn set_workspace_scroll(state: &mut AppState, offset: usize, visible: usize) -> bool {
    let offset = offset.min(workspace_scroll_max(state, visible));
    if offset == state.workspace_scroll {
        return false;
    }
    state.workspace_scroll = offset;
    true
}

/// Agents 列表滚动上限。
fn agent_scroll_max(state: &AppState, visible: usize) -> usize {
    state.agent_panes().len().saturating_sub(visible.max(1))
}

/// 滚动 Agents 列表；越界钳制；返回是否变化。
pub fn scroll_agent_list(state: &mut AppState, delta: isize, visible: usize) -> bool {
    let max = agent_scroll_max(state, visible);
    let target = (state.agent_scroll.min(max) as isize).saturating_add(delta);
    let offset = target.clamp(0, max as isize) as usize;
    if offset == state.agent_scroll {
        return false;
    }
    state.agent_scroll = offset;
    true
}

/// 直接设置 Agents 滚动偏移（滚动条点击/拖拽）；越界钳制；返回是否变化。
pub fn set_agent_scroll(state: &mut AppState, offset: usize, visible: usize) -> bool {
    let offset = offset.min(agent_scroll_max(state, visible));
    if offset == state.agent_scroll {
        return false;
    }
    state.agent_scroll = offset;
    true
}

/// 钳制 Agents 滚动偏移（几何或列表变化后调用）；返回是否变化。
pub fn clamp_agent_scroll(state: &mut AppState, visible: usize) -> bool {
    let max = agent_scroll_max(state, visible);
    if state.agent_scroll <= max {
        return false;
    }
    state.agent_scroll = max;
    true
}

/// 更新 Agents 悬停槽位；返回是否变化。
pub fn set_agent_hover(state: &mut AppState, hover: Option<usize>) -> bool {
    if state.agent_hover == hover {
        return false;
    }
    state.agent_hover = hover;
    true
}

/// 直接设置 prompt 视口偏移（滚动条点击/拖拽）；越界钳制；返回是否变化。
pub fn set_prompt_scroll(state: &mut AppState, offset: usize) -> bool {
    let before = state.prompt.scroll();
    state.prompt.scroll_to(offset);
    state.prompt.scroll() != before
}

/// 开始拖动排序：记录被拖工作区索引；按下时已切换激活；整块反显等指针移动后再显示。
///
/// 拖动期间 hover 反显优先，清空悬停高亮直到指针再次移动。
pub fn begin_workspace_drag(state: &mut AppState, index: usize) {
    if index < state.workspaces.len() {
        state.workspace_drag = Some(index);
        state.workspace_dragging = false;
        state.workspace_hover = None;
    }
}

/// 拖动排序：把被拖工作区所在的组（根 + linked 子项）整体移动到落点，
/// 落点落在别的组上时吸附到该组边界；重复主 checkout 等独立项单项移动；
/// 激活项与拖动索引跟随新位置；返回是否变化。
pub fn drag_workspace_to(state: &mut AppState, target: usize) -> bool {
    let Some(from) = state.workspace_drag else {
        return false;
    };
    let len = state.workspaces.len();
    if len == 0 || from >= len {
        return false;
    }
    state.workspace_dragging = true;
    let target = target.min(len - 1);
    let block = state.workspace_block(from);
    let hovered = state.workspace_block(target);
    if hovered.iter().any(|member| block.contains(member)) {
        return false;
    }
    // 落点坐标：向下拖插到目标块之后，向上拖插到目标块之前。
    let at = if target > from {
        hovered.last().map_or(len, |last| last.saturating_add(1))
    } else {
        hovered.first().copied().unwrap_or(0)
    };
    let removed_before =
        |position: usize| block.iter().filter(|member| **member < position).count();
    let insert_at = at - removed_before(at);
    let kept: Vec<usize> = (0..len).filter(|index| !block.contains(index)).collect();
    let mut order = kept;
    for (offset, member) in block.iter().enumerate() {
        order.insert(insert_at + offset, *member);
    }
    if order.iter().enumerate().all(|(index, old)| index == *old) {
        return false;
    }
    let new_index = |old: usize| match block.iter().position(|member| *member == old) {
        Some(offset) => insert_at + offset,
        None => {
            let rank = old - removed_before(old);
            if rank < insert_at {
                rank
            } else {
                rank + block.len()
            }
        }
    };
    state.active_workspace = new_index(state.active_workspace);
    if let Some(pinned) = state.prompt_pinned {
        state.prompt_pinned = Some(new_index(pinned));
    }
    state.workspace_drag = Some(new_index(from));
    let mut slots: Vec<Option<Workspace>> = state.workspaces.drain(..).map(Some).collect();
    state.workspaces = order
        .into_iter()
        .filter_map(|old| slots[old].take())
        .collect();
    true
}

/// 结束拖动排序。
pub fn end_workspace_drag(state: &mut AppState) {
    state.workspace_drag = None;
    state.workspace_dragging = false;
}

/// 保证当前工作区可见并钳制偏移；列表长度或可见行变化后调用；返回是否变化。
pub fn ensure_workspace_visible(state: &mut AppState, visible: usize) -> bool {
    let visible = visible.max(1);
    let max = workspace_scroll_max(state, visible);
    let rows = state.workspace_rows();
    let active = rows
        .iter()
        .position(|row| row.index == state.active_workspace)
        .unwrap_or(0);
    let mut offset = state.workspace_scroll.min(max);
    if active < offset {
        offset = active;
    } else if active >= offset.saturating_add(visible) {
        offset = active + 1 - visible;
    }
    if offset == state.workspace_scroll {
        return false;
    }
    state.workspace_scroll = offset;
    true
}

/// 仅把滚动偏移钳制到合法范围（窗口缩放后调用，不强制跟随 active）；返回是否变化。
pub fn clamp_workspace_scroll(state: &mut AppState, visible: usize) -> bool {
    let max = workspace_scroll_max(state, visible);
    if state.workspace_scroll <= max {
        return false;
    }
    state.workspace_scroll = max;
    true
}
