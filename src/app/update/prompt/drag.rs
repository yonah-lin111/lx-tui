//! 提示词列表拖拽：起点登记、拖动阈值、落点解析与移动效应登记。
//!
//! 纯状态：不写库、不产生 IO，移动只登记 `Effect` 由持久化服务执行。
//! 标记 `dragging` 后，Up 走移动；未跨行则本次 Down→Up 仍是一次点击。

use std::time::Instant;

use crate::app::effects::Effect;
use crate::app::prompt::{PromptDropHit, PromptRowKind};
use crate::app::state::{AppState, PromptDrag, PromptDragSource, PromptDropMark};
use crate::app::toast::{Toast, ToastKind};
use crate::ui::text;

use super::tree_view::{
    click_prompt_row, open_prompt_row, prompt_row_double_click, select_prompt_row,
};

/// 移动被拒绝的原因（决定 toast 文案）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptMoveReject {
    /// 目标会形成第 3 层，或该层不收文件夹。
    Depth,
    /// 被拖文件夹含子文件夹，整体移入会掉出可视范围。
    Nested,
    /// 无可移动（拖到自己、已是同一容器、落点不可解释）。
    Noop,
}

/// 按下：只选中并登记起点；折叠/展开与双击判定全部推迟到 Up。
///
/// 分区标签行不登记；`Temp` 行没有持久身份（`source = None`），可点击但不可拖。
/// 搜索过滤态下拖动结果与所见不符，直接禁用拖动。返回选中是否变化。
pub fn begin_prompt_drag(state: &mut AppState, row: usize) -> bool {
    let workspace = state.prompt_workspace();
    let Some((row_data, searching)) = state.workspaces.get(workspace).and_then(|workspace| {
        let panel = &workspace.prompt_panel;
        panel
            .rows()
            .get(row)
            .copied()
            .map(|row| (row, !panel.search.text().trim().is_empty()))
    }) else {
        return false;
    };
    if !row_data.kind.selectable() {
        return false;
    }
    let source = match row_data.kind {
        PromptRowKind::Folder => state
            .workspaces
            .get(workspace)
            .and_then(|workspace| workspace.prompt_panel.tree.folder_of_row(row_data))
            .map(|folder| PromptDragSource::Folder(folder.id)),
        PromptRowKind::Item => state
            .workspaces
            .get(workspace)
            .and_then(|workspace| workspace.prompt_panel.tree.item_of_row(row_data))
            .map(|item| PromptDragSource::Item(item.id)),
        PromptRowKind::Temp | PromptRowKind::Section(_) => None,
    };
    let enabled = source.is_some() && !searching;
    let changed = select_prompt_row(state, row);
    state.prompt_drag = Some(PromptDrag {
        workspace,
        source,
        from_row: row,
        pressed_at: Instant::now(),
        dragging: false,
        drop_mark: PromptDropMark::None,
        enabled,
    });
    state.prompt_tree_hover = None;
    changed
}

/// 拖动：跨到与起点不同的可见行、或落到末行以下空白才置 `dragging`；返回是否变化。
///
/// 空白（`Root`）也算跨行：它是"拖出文件夹回根层"的落点，若只在行上置位，
/// 末行条目往下拖出到空白将永远退化成一次点击，该路径不可达。
///
/// 置臂后每次移动都重算落点反馈：指针换行时高亮跟着走，回到会被拒绝的落点
/// （自己、同一容器）或移出树时高亮立刻消失——不给"按下去会移动"的假承诺。
pub fn drag_prompt_to(state: &mut AppState, hit: PromptDropHit) -> bool {
    let Some(drag) = state.prompt_drag else {
        return false;
    };
    if !drag.enabled {
        return false;
    }
    let crossed = match hit {
        PromptDropHit::Row(row) => row != drag.from_row,
        // 空白在末行以下：到达它必然已离开起点行，同属"跨行"。
        PromptDropHit::Root => true,
        PromptDropHit::Outside => false,
    };
    // 已置臂后即便指针回到起点行（或移出树）也保持拖动状态：此时是"取消"而非"没在拖"。
    let armed = drag.dragging || crossed;
    let drop_mark = if armed {
        prompt_drop_mark(state, &drag, hit)
    } else {
        PromptDropMark::None
    };
    let changed = armed != drag.dragging || drop_mark != drag.drop_mark;
    state.prompt_drag = Some(PromptDrag {
        dragging: armed,
        drop_mark,
        ..drag
    });
    changed
}

/// 落点反馈：只对**会真正产生移动**的落点给出高亮。
///
/// 复用 [`prompt_drop_container`] 的判定，保证高亮与松手后的行为永远一致；
/// 被拒绝的落点（第 3 层、含子文件夹、拖到自己、同一容器）一律不亮，
/// 由松手时的 toast 说明原因。
///
/// 亮哪一行：容器是文件夹时亮**该文件夹行**（而不是指针下的条目行——条目行的容器是
/// 它所在的文件夹，亮条目行会被读成"插到这条旁边"）；容器是根层时，指针若在某一行上，
/// 就亮那一行（那一行的容器本来就是根层，松手确实落在这里），只有指针落在末行以下
/// 空白才退回 `Root`，由渲染层画 `→ move to root` 提示。
fn prompt_drop_mark(state: &AppState, drag: &PromptDrag, hit: PromptDropHit) -> PromptDropMark {
    let Ok(target) = prompt_drop_container(state, drag, hit) else {
        return PromptDropMark::None;
    };
    let Some(folder_id) = target else {
        return match hit {
            PromptDropHit::Row(row) => PromptDropMark::Row(row),
            PromptDropHit::Root => PromptDropMark::Root,
            PromptDropHit::Outside => PromptDropMark::None,
        };
    };
    let Some(workspace) = state.workspaces.get(drag.workspace) else {
        return PromptDropMark::None;
    };
    let panel = &workspace.prompt_panel;
    panel
        .rows()
        .iter()
        .position(|row| {
            panel
                .tree
                .folder_of_row(*row)
                .is_some_and(|folder| folder.id == folder_id)
        })
        .map_or(PromptDropMark::None, PromptDropMark::Row)
}

/// 松开：未拖动按点击/双击处理；拖动则解析落点并登记移动效应。
///
/// 返回是否产生可见变化（供事件层置脏）。
pub fn finish_prompt_drag(state: &mut AppState, hit: PromptDropHit) -> bool {
    let Some(drag) = state.prompt_drag.take() else {
        return false;
    };
    state.prompt_tree_hover = None;
    if drag.workspace != state.prompt_workspace() {
        return false;
    }
    if !drag.dragging {
        if prompt_row_double_click(state, drag.from_row, drag.pressed_at) {
            return open_prompt_row(state, drag.from_row);
        }
        return click_prompt_row(state, drag.from_row);
    }
    if hit == PromptDropHit::Outside {
        return false;
    }
    match prompt_drop_container(state, &drag, hit) {
        Ok(target) => match drag.source {
            Some(source) => queue_move(state, drag.workspace, source, target),
            None => false,
        },
        Err(PromptMoveReject::Noop) => false,
        Err(reason) => {
            show_reject_toast(state, reason);
            true
        }
    }
}

/// 解析落点对应的容器（`None` 为根层）；非法落点给出拒绝原因。
pub fn prompt_drop_container(
    state: &AppState,
    drag: &PromptDrag,
    hit: PromptDropHit,
) -> Result<Option<i64>, PromptMoveReject> {
    let Some(source) = drag.source else {
        return Err(PromptMoveReject::Noop);
    };
    let Some(workspace) = state.workspaces.get(drag.workspace) else {
        return Err(PromptMoveReject::Noop);
    };
    let panel = &workspace.prompt_panel;
    let target = match hit {
        PromptDropHit::Outside => return Err(PromptMoveReject::Noop),
        PromptDropHit::Root => None,
        PromptDropHit::Row(row) => {
            let Some(row_data) = panel.rows().get(row).copied() else {
                return Err(PromptMoveReject::Noop);
            };
            match row_data.kind {
                PromptRowKind::Section(_) => return Err(PromptMoveReject::Noop),
                PromptRowKind::Temp => None,
                PromptRowKind::Folder => match panel.tree.folder_of_row(row_data) {
                    Some(folder) => Some(folder.id),
                    None => return Err(PromptMoveReject::Noop),
                },
                PromptRowKind::Item => match panel.tree.item_of_row(row_data) {
                    Some(item) => item.folder_id,
                    None => return Err(PromptMoveReject::Noop),
                },
            }
        }
    };
    match source {
        PromptDragSource::Item(id) => {
            let Some(item) = panel.tree.item(id) else {
                return Err(PromptMoveReject::Noop);
            };
            if target == item.folder_id {
                return Err(PromptMoveReject::Noop);
            }
            Ok(target)
        }
        PromptDragSource::Folder(id) => {
            let Some(folder) = panel.tree.folder(id) else {
                return Err(PromptMoveReject::Noop);
            };
            if target == folder.parent_id {
                return Err(PromptMoveReject::Noop);
            }
            let Some(target) = target else {
                return Ok(None);
            };
            if target == id {
                return Err(PromptMoveReject::Noop);
            }
            if panel
                .tree
                .folders()
                .iter()
                .any(|sub| sub.parent_id == Some(id))
            {
                return Err(PromptMoveReject::Nested);
            }
            if folder.parent_id.is_some() {
                return Err(PromptMoveReject::Depth);
            }
            let Some(target_folder) = panel.tree.folder(target) else {
                return Err(PromptMoveReject::Noop);
            };
            if target_folder.parent_id.is_some() {
                return Err(PromptMoveReject::Depth);
            }
            Ok(Some(target))
        }
    }
}

/// 登记移动效应；取不到工作区路径时不发。返回是否登记。
fn queue_move(
    state: &mut AppState,
    workspace: usize,
    source: PromptDragSource,
    target: Option<i64>,
) -> bool {
    let Some(cwd) = super::workspace_cwd(state, workspace) else {
        return false;
    };
    let effect = match source {
        PromptDragSource::Item(id) => Effect::MovePrompt {
            workspace: cwd,
            id,
            folder_id: target,
        },
        PromptDragSource::Folder(id) => Effect::MoveFolder {
            workspace: cwd,
            id,
            parent_id: target,
        },
    };
    state.effects.push(effect);
    true
}

/// 非法落点反馈：锚在显示中的 prompt 面板上，不跨面板打扰。
fn show_reject_toast(state: &mut AppState, reason: PromptMoveReject) {
    let message = match reason {
        PromptMoveReject::Depth => text::TOAST_PROMPT_MOVE_DEPTH,
        PromptMoveReject::Nested => text::TOAST_PROMPT_MOVE_NESTED,
        PromptMoveReject::Noop => return,
    };
    let toast = Toast::new(
        ToastKind::Danger,
        message,
        Some(state.prompt.id()),
        Instant::now(),
    );
    crate::app::update::show_toast(state, toast);
}
