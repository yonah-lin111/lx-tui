//! 键盘路由：Ctrl+Q 退出；浮层打开时按键按浮层种类过滤；
//! prompt 聚焦时按键映射为编辑命令，否则原样编码进焦点窗格。

pub mod encode;

use alacritty_terminal::term::TermMode;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::app::actions::{Action, EditorCommand, OverlayKey};
use crate::app::overlay::OverlayKind;

/// 路由结果。
#[derive(Debug, PartialEq, Eq)]
pub enum Routed {
    /// 应用行为。
    Action(Action),
    /// prompt 编辑器命令。
    Editor(EditorCommand),
    /// 浮层按键。
    Overlay(OverlayKey),
    /// 复制 prompt 选区（无选区为全文）到系统剪贴板。
    Copy,
    /// 复制光标所在模板块正文到系统剪贴板。
    CopyTemplateBlock,
    /// 原样写入焦点窗格的字节。
    Pane(Vec<u8>),
}

/// 路由一次按键；Release 事件与无法编码的按键返回 None。
///
/// `Ctrl+Q` 始终优先；浮层打开时只放行该浮层支持的键，其余吞掉；
/// `prompt_focused` 为真时按键只进入编辑器：`Ctrl/Cmd+C` 复制选区（无选区全文）、
/// `Ctrl/Cmd+Shift+C` 复制光标所在模板块正文，未映射的按键被吞掉，绝不写入 PTY；
/// `pane_lx` 为真（活动窗格显示 lx 页）时除 `Ctrl+Q` 外的按键一并吞掉，不写入隐藏终端。
pub fn route(
    key: KeyEvent,
    mode: TermMode,
    prompt_focused: bool,
    overlay: Option<OverlayKind>,
    pane_lx: bool,
) -> Option<Routed> {
    if key.kind == KeyEventKind::Release {
        return None;
    }
    if key.code == KeyCode::Char('q') && key.modifiers.contains(KeyModifiers::CONTROL) {
        return Some(Routed::Action(Action::Quit));
    }
    if let Some(kind) = overlay {
        return overlay_key(key, kind).map(Routed::Overlay);
    }
    if prompt_focused {
        let copy_chord = matches!(key.code, KeyCode::Char('c' | 'C'))
            && (key.modifiers.contains(KeyModifiers::CONTROL)
                || key.modifiers.contains(KeyModifiers::SUPER))
            && !key.modifiers.contains(KeyModifiers::ALT);
        if copy_chord {
            if key.modifiers.contains(KeyModifiers::SHIFT) {
                return Some(Routed::CopyTemplateBlock);
            }
            return Some(Routed::Copy);
        }
        return editor_command(key).map(Routed::Editor);
    }
    if pane_lx {
        return None;
    }
    encode::encode_key(key, mode).map(Routed::Pane)
}

/// 浮层按键映射：只放行该浮层支持的键，其余返回 None（吞掉）。
fn overlay_key(key: KeyEvent, kind: OverlayKind) -> Option<OverlayKey> {
    let plain = !key
        .modifiers
        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT);
    match kind {
        OverlayKind::Menu => match key.code {
            KeyCode::Esc => Some(OverlayKey::Esc),
            KeyCode::Up => Some(OverlayKey::Up),
            KeyCode::Down => Some(OverlayKey::Down),
            KeyCode::Enter => Some(OverlayKey::Enter),
            _ => None,
        },
        OverlayKind::ConfirmClose
        | OverlayKind::ConfirmSwitchCwd
        | OverlayKind::ConfirmSyncWorkspaceCwd => match key.code {
            KeyCode::Esc => Some(OverlayKey::Esc),
            KeyCode::Enter => Some(OverlayKey::Enter),
            _ => None,
        },
        OverlayKind::Rename | OverlayKind::NewWorkspace => match key.code {
            KeyCode::Esc => Some(OverlayKey::Esc),
            KeyCode::Enter => Some(OverlayKey::Enter),
            KeyCode::Char('c' | 'C') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                Some(OverlayKey::Clear)
            }
            KeyCode::Backspace => Some(OverlayKey::Backspace),
            KeyCode::Delete => Some(OverlayKey::Delete),
            KeyCode::Left if plain => Some(OverlayKey::Left),
            KeyCode::Right if plain => Some(OverlayKey::Right),
            KeyCode::Home => Some(OverlayKey::Home),
            KeyCode::End => Some(OverlayKey::End),
            KeyCode::Char(ch) if plain => Some(OverlayKey::Char(ch)),
            _ => None,
        },
        OverlayKind::WorktreeOpen => match key.code {
            KeyCode::Esc => Some(OverlayKey::Esc),
            KeyCode::Enter => Some(OverlayKey::Enter),
            KeyCode::Up => Some(OverlayKey::Up),
            KeyCode::Down => Some(OverlayKey::Down),
            KeyCode::Char('c' | 'C') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                Some(OverlayKey::Clear)
            }
            KeyCode::Backspace => Some(OverlayKey::Backspace),
            KeyCode::Delete => Some(OverlayKey::Delete),
            KeyCode::Left if plain => Some(OverlayKey::Left),
            KeyCode::Right if plain => Some(OverlayKey::Right),
            KeyCode::Home => Some(OverlayKey::Home),
            KeyCode::End => Some(OverlayKey::End),
            KeyCode::Char(ch) if plain => Some(OverlayKey::Char(ch)),
            _ => None,
        },
    }
}

/// 编辑键映射；Shift 只用于字符输入与 @ 面板目录导航，导航键要求无修饰符。
///
/// `Shift+Enter` 映射为 @ 面板目录进入命令；面板未消费时回落为行尾另起一行
/// （见 `update::apply_editor`）。@ 面板的目录回退走编辑器撤销（`Ctrl+Z` / `Cmd+Z`）。
/// Ctrl/Alt 组合同 opencode/readline：Ctrl+U 删除到逻辑行首（已在行首时删除前一个换行）、
/// Ctrl+K 删除到行尾、Ctrl+W 向前删词、
/// Ctrl+A/E 逻辑行首尾、Ctrl+D 正向删除、Ctrl+←/→ 按词移动；撤销/重做用
/// Ctrl+Z / Ctrl+Y（legacy 终端可靠），并对齐 opencode 接受 Ctrl+Shift+Z、Ctrl+-、
/// Ctrl+. 与 Super 组合（终端支持才收得到）；Alt+B/F、Alt+D 作为别名；
/// Tab/BackTab 缩进/反缩进；其余 Ctrl/Alt 组合吞掉。
fn editor_command(key: KeyEvent) -> Option<EditorCommand> {
    let modifiers = key.modifiers;
    let ctrl = modifiers.contains(KeyModifiers::CONTROL);
    let alt = modifiers.contains(KeyModifiers::ALT);
    let shift = modifiers.contains(KeyModifiers::SHIFT);
    if modifiers.contains(KeyModifiers::SUPER) {
        return match key.code {
            KeyCode::Enter if !shift => Some(EditorCommand::NewlineBelow),
            KeyCode::Char('z' | 'Z') if shift => Some(EditorCommand::Redo),
            KeyCode::Char('z' | 'Z') => Some(EditorCommand::Undo),
            KeyCode::Char('y' | 'Y') => Some(EditorCommand::Redo),
            _ => None,
        };
    }
    let command = match (ctrl, alt, key.code) {
        (true, false, KeyCode::Enter) if !shift => Some(EditorCommand::NewlineBelow),
        (true, false, KeyCode::Char(ch)) => match ch {
            'u' | 'U' => Some(EditorCommand::DeleteToLineStart),
            'k' | 'K' => Some(EditorCommand::DeleteToLineEnd),
            'w' | 'W' => Some(EditorCommand::DeleteWordBackward),
            'a' | 'A' => Some(EditorCommand::LineStart),
            'e' | 'E' => Some(EditorCommand::LineEnd),
            'd' | 'D' => Some(EditorCommand::Delete),
            'z' | '-' => Some(EditorCommand::Undo),
            'Z' | 'y' | 'Y' | '.' => Some(EditorCommand::Redo),
            _ => None,
        },
        (true, false, KeyCode::Backspace) => Some(EditorCommand::DeleteWordBackward),
        (true, false, KeyCode::Delete) => Some(EditorCommand::DeleteWordForward),
        (true, false, KeyCode::Left) => Some(EditorCommand::WordLeft),
        (true, false, KeyCode::Right) => Some(EditorCommand::WordRight),
        (false, true, KeyCode::Char('b')) => Some(EditorCommand::WordLeft),
        (false, true, KeyCode::Char('f')) => Some(EditorCommand::WordRight),
        (false, true, KeyCode::Char('d')) => Some(EditorCommand::DeleteWordForward),
        (false, true, KeyCode::Backspace) => Some(EditorCommand::DeleteWordBackward),
        (false, true, KeyCode::Left) => Some(EditorCommand::WordLeft),
        (false, true, KeyCode::Right) => Some(EditorCommand::WordRight),
        _ => None,
    };
    if command.is_some() {
        return command;
    }
    if ctrl || alt {
        return None;
    }
    match key.code {
        KeyCode::Char(ch) => Some(EditorCommand::InsertChar(ch)),
        KeyCode::Enter if shift => Some(EditorCommand::EnterFolder),
        KeyCode::Enter => Some(EditorCommand::Newline),
        KeyCode::Backspace => Some(EditorCommand::Backspace),
        KeyCode::Delete => Some(EditorCommand::Delete),
        KeyCode::Esc if modifiers.is_empty() => Some(EditorCommand::Escape),
        KeyCode::Tab if modifiers.is_empty() => Some(EditorCommand::Indent),
        KeyCode::Tab if modifiers == KeyModifiers::SHIFT => Some(EditorCommand::Outdent),
        KeyCode::BackTab => Some(EditorCommand::Outdent),
        KeyCode::Left if modifiers.is_empty() => Some(EditorCommand::Left),
        KeyCode::Right if modifiers.is_empty() => Some(EditorCommand::Right),
        KeyCode::Up if modifiers.is_empty() => Some(EditorCommand::Up),
        KeyCode::Down if modifiers.is_empty() => Some(EditorCommand::Down),
        KeyCode::Home if modifiers.is_empty() => Some(EditorCommand::Home),
        KeyCode::End if modifiers.is_empty() => Some(EditorCommand::End),
        _ => None,
    }
}

#[cfg(test)]
mod tests;
