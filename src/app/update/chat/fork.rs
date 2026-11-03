//! `/fork`：从当前会话的某条用户消息之前分叉出新会话。
//!
//! 应用层只负责节点选择与效应登记；DB 事务与视图装载由服务层完成。

use std::time::Instant;

use crate::app::state::{AppState, ForkEntry, ForkPanel, InputHintLevel, PaneView, SlashPanel};
use crate::app::toast::{Toast, ToastKind};
use crate::layout::PaneId;
use crate::ui::text;

/// 打开 `/fork` 节点面板。
///
/// 面板首项固定 `Full session`，其余为 `role=user && kind=chat` 的消息（最新在上）。
/// 流式生成、本地命令执行、无会话或没有用户聊天消息时只给窗格内提示，不打开面板。
pub fn open_fork_panel(state: &mut AppState, pane: PaneId) -> bool {
    let Some((session_id, mut entries)) = state.pane_anywhere(pane).and_then(|target| {
        if target.view != PaneView::Lx || target.lx.shell_mode() {
            return None;
        }
        let session_id = target.lx.chat.session_id.clone()?;
        let entries: Vec<ForkEntry> = target
            .lx
            .chat
            .messages
            .iter()
            .filter(|message| message.role == "user" && message.kind == "chat")
            .rev()
            .map(|message| {
                let text = message.text();
                ForkEntry {
                    message_id: Some(message.id.clone()),
                    label: fork_label(&text),
                }
            })
            .collect();
        Some((session_id, entries))
    }) else {
        if state
            .pane_anywhere(pane)
            .is_some_and(|target| target.view == PaneView::Lx && !target.lx.shell_mode())
        {
            crate::app::update::chat::show_lx_hint(
                state,
                pane,
                InputHintLevel::Warning,
                text::LX_FORK_NOTHING,
            );
            return true;
        }
        return false;
    };

    let occupied = state.all_pane_ids().into_iter().any(|other| {
        state.pane_anywhere(other).is_some_and(|target| {
            target.lx.chat.session_id.as_deref() == Some(session_id.as_str())
                && (target.lx.chat.request.is_busy() || target.lx.chat.has_running_shell())
        })
    });
    if occupied {
        crate::app::update::chat::show_lx_hint(
            state,
            pane,
            InputHintLevel::Warning,
            text::LX_FORK_BUSY,
        );
        return true;
    }
    if entries.is_empty() {
        crate::app::update::chat::show_lx_hint(
            state,
            pane,
            InputHintLevel::Warning,
            text::LX_FORK_NOTHING,
        );
        return true;
    }

    entries.insert(
        0,
        ForkEntry {
            message_id: None,
            label: String::new(),
        },
    );
    let filtered = (0..entries.len()).collect();
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    target.lx.slash = Some(SlashPanel::Fork(Box::new(ForkPanel {
        entries,
        window: crate::app::state::PanelWindow::default(),
        filter: String::new(),
        filtered,
    })));
    true
}

/// fork 完成：装好新会话后回填被选中消息的原文，并在窗格内提示。
pub fn on_fork_completed(state: &mut AppState, pane: PaneId, restore_text: Option<&str>) -> bool {
    if let Some(target) = state.pane_mut_anywhere(pane) {
        match restore_text {
            Some(text) if !text.is_empty() => target.lx.replace_input(text),
            _ => target.lx.clear(),
        }
    }
    crate::app::update::show_lx_toast(
        state,
        pane,
        Toast::new(
            ToastKind::Default,
            text::LX_FORKED,
            Some(pane),
            Instant::now(),
        ),
    );
    true
}

/// fork 失败：源会话与当前窗格保持原样。
pub fn on_fork_failed(state: &mut AppState, pane: PaneId) -> bool {
    crate::app::update::chat::show_lx_hint(
        state,
        pane,
        InputHintLevel::Error,
        text::LX_FORK_FAILED,
    );
    true
}

/// 节点展示文本：单行化、剔除控制字符、超过 120 字符截断。
fn fork_label(text: &str) -> String {
    const MAX_CHARS: usize = 120;
    let mut out = String::new();
    let mut truncated = false;
    for ch in text.chars() {
        if out.chars().count() >= MAX_CHARS {
            truncated = true;
            break;
        }
        match ch {
            '\n' | '\r' => out.push(' '),
            ch if (ch as u32) < 0x20 || ch == '\u{7f}' => {}
            ch => out.push(ch),
        }
    }
    let out = out.trim();
    if out.is_empty() {
        return text::LX_FORK_EMPTY_MESSAGE.to_string();
    }
    if truncated {
        format!("{out}…")
    } else {
        out.to_string()
    }
}
