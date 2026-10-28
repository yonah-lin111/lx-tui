//! `/resume` 会话编辑模态：改名与删除（`Shift+Enter` 从历史面板打开）。
//!
//! 只改纯状态、只入队效应；落库由事件层执行，成功回执后再更新列表。

use crate::app::effects::Effect;
use crate::app::overlay::{Overlay, ResumeSessionEdit, TextInput};
use crate::app::state::AppState;
use crate::app::state::lx::slash::SlashPanel;
use crate::layout::PaneId;
use crate::ui::text;

use super::menu::close_overlay;

/// 从 `/resume` 历史面板打开编辑模态；高亮行即目标会话。
///
/// 面板没有条目（空态占位）时不打开。返回是否打开。
pub fn open_resume_session_edit(state: &mut AppState, pane: PaneId) -> bool {
    let Some(target) = state.pane_anywhere(pane) else {
        return false;
    };
    let Some(SlashPanel::History(panel)) = target.lx.slash.as_ref() else {
        return false;
    };
    // 高亮位置是过滤视图内的位置，换算回原始条目。
    let Some(entry) = panel.active_entry() else {
        return false;
    };
    let session_id = entry.session_id.clone();
    let title = entry.title.clone();
    let Some(workspace) = state.chat_session_key_for_pane(pane) else {
        return false;
    };
    state.overlay = Some(Overlay::ResumeSessionEdit(ResumeSessionEdit {
        pane,
        workspace,
        session_id,
        input: TextInput::new(title),
        confirming_delete: false,
        hint: None,
    }));
    true
}

/// 复制 `/resume` 历史面板高亮行的会话 id 到剪贴板；返回是否产生了复制效应。
///
/// 剪贴板内容为**裸 id**（无前缀、无引号、无换行），便于直接粘给 `sqlite3`/`git` 等外部工具。
/// 面板未打开、列表为空（空态占位行）或高亮越界时返回 false 且**不产生任何效应**：
/// 调用方据此把这个按键静默吞掉（不清空输入框、不报错、不关面板）。
pub fn copy_resume_session_id(state: &mut AppState, pane: PaneId) -> bool {
    let Some(target) = state.pane_anywhere(pane) else {
        return false;
    };
    let Some(SlashPanel::History(panel)) = target.lx.slash.as_ref() else {
        return false;
    };
    let Some(entry) = panel.active_entry() else {
        return false;
    };
    let session_id = entry.session_id.clone();
    // 唯一剪贴板出口：剪贴板写入与 toast 由事件层按 Effect 出箱，app 层不做 IO。
    // `anchor` 锚到当前窗格：toast 落在 main content 里，而不是屏幕顶部居中——
    // 复制发生在 lx 页，反馈就该出现在同一块区域。
    state.effects.push(Effect::CopyToClipboard {
        text: session_id,
        anchor: Some(pane),
    });
    true
}

/// 编辑模态按键分派；返回是否产生了需要写库的效应（由调用方照常 pump）。
///
/// - `Enter`：保存标题（确认删除态下等同确认删除）。
/// - `^D`：进入删除确认；确认态下再按即删除。
/// - `Esc`：确认态下退回编辑态，否则关闭模态。
pub fn apply_resume_session_edit_key(
    state: &mut AppState,
    key: crate::app::actions::OverlayKey,
) -> bool {
    use crate::app::actions::OverlayKey;
    let Some(Overlay::ResumeSessionEdit(edit)) = state.overlay.as_ref() else {
        return false;
    };
    let confirming = edit.confirming_delete;
    match key {
        OverlayKey::Esc => {
            if confirming {
                if let Some(Overlay::ResumeSessionEdit(edit)) = state.overlay.as_mut() {
                    edit.confirming_delete = false;
                    edit.hint = None;
                }
                return false;
            }
            close_overlay(state);
            false
        }
        OverlayKey::Enter => {
            if confirming {
                return delete_target(state);
            }
            commit_resume_session_rename(state)
        }
        OverlayKey::Remove => {
            if confirming {
                return delete_target(state);
            }
            if let Some(Overlay::ResumeSessionEdit(edit)) = state.overlay.as_mut() {
                edit.confirming_delete = true;
                edit.hint = None;
            }
            false
        }
        other => {
            // 确认态不允许编辑标题：避免"要删的那个"与"正在改的标题"混淆。
            if confirming {
                return false;
            }
            edit_input(state, other);
            false
        }
    }
}

/// 编辑单行标题（与重命名浮层同一套文本编辑语义）。
fn edit_input(state: &mut AppState, key: crate::app::actions::OverlayKey) {
    use crate::app::actions::OverlayKey;
    let Some(Overlay::ResumeSessionEdit(edit)) = state.overlay.as_mut() else {
        return;
    };
    match key {
        OverlayKey::Char(ch) => edit.input.insert_char(ch),
        OverlayKey::Clear => edit.input.clear(),
        OverlayKey::Backspace => edit.input.backspace(),
        OverlayKey::Delete => edit.input.delete(),
        OverlayKey::Left => edit.input.move_left(),
        OverlayKey::Right => edit.input.move_right(),
        OverlayKey::Home | OverlayKey::LineStart => edit.input.move_home(),
        OverlayKey::End | OverlayKey::LineEnd => edit.input.move_end(),
        OverlayKey::DeleteToLineStart => edit.input.delete_to_line_start(),
        OverlayKey::DeleteToLineEnd => edit.input.delete_to_line_end(),
        OverlayKey::Undo => {
            edit.input.undo();
        }
        OverlayKey::Redo => {
            edit.input.redo();
        }
        _ => {}
    }
    // 编辑标题即退出删除确认：用户改变主意了。
    edit.confirming_delete = false;
    edit.hint = None;
}

/// 保存标题：去空白后为空则拒绝（模态内提示、不关窗、不落库）。
fn commit_resume_session_rename(state: &mut AppState) -> bool {
    let Some(Overlay::ResumeSessionEdit(edit)) = state.overlay.as_ref() else {
        return false;
    };
    let title = edit.input.text().trim().to_string();
    if title.is_empty() {
        set_edit_hint(state, text::RESUME_EDIT_EMPTY_TITLE);
        return false;
    }
    let workspace = edit.workspace.clone();
    let session_id = edit.session_id.clone();
    let pane = edit.pane;
    close_overlay(state);
    state.effects.push(Effect::RenameChatSession {
        workspace,
        session_id,
        title,
    });
    crate::app::update::refresh_lx_status(state, pane);
    true
}

/// 删除高亮会话；被其它窗格占用（正在生成）时拒绝并在模态内提示。
fn delete_target(state: &mut AppState) -> bool {
    let Some(Overlay::ResumeSessionEdit(edit)) = state.overlay.as_ref() else {
        return false;
    };
    let workspace = edit.workspace.clone();
    let session_id = edit.session_id.clone();
    let pane = edit.pane;
    if session_busy_elsewhere(state, pane, &session_id) {
        set_edit_hint(state, text::LX_CHAT_SESSION_BUSY);
        return false;
    }
    close_overlay(state);
    state.effects.push(Effect::DeleteChatSession {
        workspace,
        session_id,
    });
    true
}

/// 该会话是否正被**其它**窗格占用（生成中）；本窗格自己的会话按既有语义先停流再删。
fn session_busy_elsewhere(state: &AppState, pane: PaneId, session_id: &str) -> bool {
    state
        .all_pane_ids()
        .into_iter()
        .filter(|other| *other != pane)
        .any(|other| {
            state.pane_anywhere(other).is_some_and(|target| {
                target.lx.chat.session_id.as_deref() == Some(session_id)
                    && target.lx.chat.request.is_busy()
            })
        })
}

/// 模态内提示（红字，显示在按钮行上方）。
fn set_edit_hint(state: &mut AppState, message: &str) {
    if let Some(Overlay::ResumeSessionEdit(edit)) = state.overlay.as_mut() {
        edit.hint = Some(message.to_string());
    }
}

/// 改名成功回执：就地更新 `/resume` 列表该行标题（不关闭面板、不动高亮）。
pub fn on_session_renamed(
    state: &mut AppState,
    pane: PaneId,
    session_id: &str,
    title: &str,
) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    let Some(SlashPanel::History(panel)) = target.lx.slash.as_mut() else {
        return false;
    };
    let Some(entry) = panel
        .entries
        .iter_mut()
        .find(|entry| entry.session_id == session_id)
    else {
        return false;
    };
    if entry.title == title {
        return false;
    }
    entry.title = title.to_string();
    true
}

/// 删除成功回执：从 `/resume` 列表就地移除该行，高亮顺位落到下一行（末尾则回退）。
pub fn on_session_removed(state: &mut AppState, pane: PaneId, session_id: &str) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    let Some(SlashPanel::History(panel)) = target.lx.slash.as_mut() else {
        return false;
    };
    let Some(index) = panel
        .entries
        .iter()
        .position(|entry| entry.session_id == session_id)
    else {
        return false;
    };
    panel.entries.remove(index);
    if panel.current == Some(index) {
        panel.current = None;
    } else if let Some(current) = panel.current
        && current > index
    {
        panel.current = Some(current - 1);
    }
    panel.window.clamp(panel.entries.len());
    true
}

/// 模态归属校验：工作区或焦点窗格变化即作废（丢弃未保存标题）。
///
/// 由事件循环每轮调用；返回是否关闭了模态。
pub fn discard_stale_resume_edit(state: &mut AppState) -> bool {
    let Some(Overlay::ResumeSessionEdit(edit)) = state.overlay.as_ref() else {
        return false;
    };
    let focused = state.active_tab().layout.focus();
    let workspace = state.chat_session_key_for_pane(edit.pane);
    let stale = focused != edit.pane || workspace.as_deref() != Some(edit.workspace.as_path());
    if !stale {
        return false;
    }
    state.overlay = None;
    true
}
