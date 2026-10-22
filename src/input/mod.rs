//! 键盘路由：Ctrl+Q 退出；浮层打开时按键按浮层种类过滤；
//! prompt 聚焦时按键映射为编辑命令，否则原样编码进焦点窗格。

pub mod encode;

use alacritty_terminal::term::TermMode;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::app::actions::{Action, EditorCommand, LxInput, OverlayKey};
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
    /// 立即保存 prompt（Ctrl/Cmd+S）：条目落库；临时提示词走保存选择器。
    SavePrompt,
    /// 复制光标所在模板块/补充块正文到系统剪贴板。
    CopyTemplateBlock,
    /// lx 页单行输入命令。
    Lx(LxInput),
    /// 原样写入焦点窗格的字节。
    Pane(Vec<u8>),
}

/// 路由一次按键；Release 事件与无法编码的按键返回 None。
///
/// `Ctrl+Q` 始终优先；浮层打开时只放行该浮层支持的键，其余吞掉；
/// `prompt_focused` 为真时按键只进入编辑器：`Ctrl/Cmd+C` 复制选区（无选区全文）、
/// `Ctrl/Cmd+Shift+C` 复制光标所在模板块/补充块正文，未映射的按键被吞掉，绝不写入 PTY；
/// `pane_lx` 为真（活动窗格显示 lx 页）时按键进入 lx 单行输入框，未映射键吞掉，不写入隐藏终端。
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
        let save_chord = matches!(key.code, KeyCode::Char('s' | 'S'))
            && (key.modifiers.contains(KeyModifiers::CONTROL)
                || key.modifiers.contains(KeyModifiers::SUPER))
            && !key.modifiers.contains(KeyModifiers::ALT);
        if save_chord {
            return Some(Routed::SavePrompt);
        }
        return match editor_command(key) {
            Some(command) => Some(Routed::Editor(command)),
            None => {
                // 终端编码差异排查用：prompt 聚焦时被吞掉的按键（仅 debug 级）。
                tracing::debug!(?key, "prompt key swallowed");
                None
            }
        };
    }
    if pane_lx {
        // 长按 Esc/Enter 的 Repeat 不推进撤销待命与二次确认，也不重复发送。
        if key.kind == KeyEventKind::Repeat && matches!(key.code, KeyCode::Esc | KeyCode::Enter) {
            return None;
        }
        return lx_input_key(key).map(Routed::Lx);
    }
    encode::encode_key(key, mode).map(Routed::Pane)
}

/// lx 页输入键映射：Shift+Enter 换行、单行编辑键、md 面板键；其余（含 Enter）吞掉。
fn lx_input_key(key: KeyEvent) -> Option<LxInput> {
    let modifiers = key.modifiers;
    let ctrl = modifiers.contains(KeyModifiers::CONTROL);
    let alt = modifiers.contains(KeyModifiers::ALT);
    let shift = modifiers.contains(KeyModifiers::SHIFT);
    if modifiers.contains(KeyModifiers::SUPER) {
        return match key.code {
            KeyCode::Char('z' | 'Z') if shift => Some(LxInput::Redo),
            KeyCode::Char('z' | 'Z') => Some(LxInput::Undo),
            KeyCode::Char('y' | 'Y') => Some(LxInput::Redo),
            // Cmd+C 与 Ctrl+C 落到同一命令：是否「清空」还是「复制」由 update 层按
            // 当前面板决定，输入层不感知状态。
            KeyCode::Char('c' | 'C') => Some(LxInput::Clear),
            _ => None,
        };
    }
    let mapped = match (ctrl, alt, key.code) {
        (true, false, KeyCode::Enter) => Some(LxInput::Newline),
        (true, false, KeyCode::Char(ch)) => match ch {
            'o' | 'O' => Some(LxInput::ToggleToolDetail),
            'u' | 'U' => Some(LxInput::DeleteToLineStart),
            'k' | 'K' => Some(LxInput::DeleteToLineEnd),
            'w' | 'W' => Some(LxInput::DeleteWordBackward),
            'a' | 'A' => Some(LxInput::LineStart),
            'e' | 'E' => Some(LxInput::LineEnd),
            'd' | 'D' => Some(LxInput::StopProcess),
            'c' | 'C' => Some(LxInput::Clear),
            't' | 'T' => Some(LxInput::ToggleThinking),
            'z' | '-' => Some(LxInput::Undo),
            'Z' | 'y' | 'Y' | '.' => Some(LxInput::Redo),
            _ => None,
        },
        (true, false, KeyCode::Backspace) => Some(LxInput::DeleteWordBackward),
        (true, false, KeyCode::Delete) => Some(LxInput::DeleteWordForward),
        (true, false, KeyCode::Left) => Some(LxInput::WordLeft),
        (true, false, KeyCode::Right) => Some(LxInput::WordRight),
        // Alt+↑/↓：在可展开工具卡片之间移动键盘选择（草稿与光标保持不变）。
        (false, true, KeyCode::Up) => Some(LxInput::SelectPrevTool),
        (false, true, KeyCode::Down) => Some(LxInput::SelectNextTool),
        (false, true, KeyCode::Char('b')) => Some(LxInput::WordLeft),
        (false, true, KeyCode::Char('f')) => Some(LxInput::WordRight),
        (false, true, KeyCode::Char('d')) => Some(LxInput::DeleteWordForward),
        (false, true, KeyCode::Backspace) => Some(LxInput::DeleteWordBackward),
        (false, true, KeyCode::Left) => Some(LxInput::WordLeft),
        (false, true, KeyCode::Right) => Some(LxInput::WordRight),
        _ => None,
    };
    if mapped.is_some() {
        return mapped;
    }
    if ctrl || alt {
        return None;
    }
    match key.code {
        // Shift+Enter：提及面板高亮目录时进入；未消费时在 update 层回落为换行。
        KeyCode::Enter if shift => Some(LxInput::EnterFolder),
        // Enter 无面板时预留给发送，交给 update 判定。
        KeyCode::Enter => Some(LxInput::Enter),
        KeyCode::Backspace => Some(LxInput::Backspace),
        KeyCode::Delete => Some(LxInput::Delete),
        KeyCode::Left => Some(LxInput::Left),
        KeyCode::Right => Some(LxInput::Right),
        KeyCode::Up => Some(LxInput::Up),
        KeyCode::Down => Some(LxInput::Down),
        KeyCode::Home => Some(LxInput::Home),
        KeyCode::End => Some(LxInput::End),
        KeyCode::Tab => Some(LxInput::Tab),
        KeyCode::PageUp => Some(LxInput::PageUp),
        KeyCode::PageDown => Some(LxInput::PageDown),
        KeyCode::Esc => Some(LxInput::Escape),
        KeyCode::Char(ch) => Some(LxInput::Char(ch)),
        _ => None,
    }
}

/// 页重命名输入键映射：Enter/Esc 结束、单行编辑键、Ctrl+C 清空；其余返回 None（吞掉）。
///
/// 与重命名浮层同一套语义，但不依赖 `Overlay`（页重命名是标题行内联输入）。
pub fn page_rename_key(key: KeyEvent) -> Option<OverlayKey> {
    let plain = !key
        .modifiers
        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT);
    match key.code {
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
    }
}

/// 浮层按键映射：只放行该浮层支持的键，其余返回 None（吞掉）。
fn overlay_key(key: KeyEvent, kind: OverlayKind) -> Option<OverlayKey> {
    // Super/Cmd 组合同样视为非纯文本输入（终端支持 kitty 协议时可达）。
    let plain = !key
        .modifiers
        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER);
    let shift = key.modifiers.contains(KeyModifiers::SHIFT);
    match kind {
        OverlayKind::Menu | OverlayKind::PageMenu => match key.code {
            KeyCode::Esc => Some(OverlayKey::Esc),
            KeyCode::Up => Some(OverlayKey::Up),
            KeyCode::Down => Some(OverlayKey::Down),
            KeyCode::Enter => Some(OverlayKey::Enter),
            _ => None,
        },
        // 引用文件夹目录弹窗：上下移动、回车打开；Ctrl/Cmd+C 复制引用、
        // Ctrl/Cmd+D 或 Del 移除记录（沿用弹窗内快捷键语义，行内不再放动作按钮）。
        OverlayKind::ReferencedFolderMenu => match key.code {
            KeyCode::Esc => Some(OverlayKey::Esc),
            KeyCode::Up => Some(OverlayKey::Up),
            KeyCode::Down => Some(OverlayKey::Down),
            KeyCode::Enter => Some(OverlayKey::Enter),
            KeyCode::Delete => Some(OverlayKey::Remove),
            KeyCode::Char('c' | 'C')
                if key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::SUPER) =>
            {
                Some(OverlayKey::Copy)
            }
            KeyCode::Char('d' | 'D')
                if key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::SUPER) =>
            {
                Some(OverlayKey::Remove)
            }
            _ => None,
        },
        OverlayKind::ReferencedFolderDelete => match key.code {
            KeyCode::Esc => Some(OverlayKey::Esc),
            KeyCode::Enter => Some(OverlayKey::Enter),
            _ => None,
        },
        OverlayKind::ReferencedFolderInput => match key.code {
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
        OverlayKind::ReferencedFolderSearch => match key.code {
            KeyCode::Esc => Some(OverlayKey::Esc),
            KeyCode::Enter if shift => Some(OverlayKey::EnterFolder),
            KeyCode::Enter => Some(OverlayKey::Enter),
            KeyCode::Up => Some(OverlayKey::Up),
            KeyCode::Down => Some(OverlayKey::Down),
            // Ctrl/Cmd+Z：回退上层搜索范围（终端支持 kitty 协议时 Cmd 组合可达）。
            KeyCode::Char('z' | 'Z')
                if key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::SUPER) =>
            {
                Some(OverlayKey::Back)
            }
            // Ctrl/Cmd+C：沿用参考面板语义复制高亮项引用（单行搜索框不另设选区模型）。
            KeyCode::Char('c' | 'C')
                if key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::SUPER) =>
            {
                Some(OverlayKey::Copy)
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
        OverlayKind::ConfirmClose
        | OverlayKind::ConfirmSwitchCwd
        | OverlayKind::ConfirmSyncWorkspaceCwd
        | OverlayKind::TrustWorkspace => match key.code {
            KeyCode::Esc => Some(OverlayKey::Esc),
            KeyCode::Enter => Some(OverlayKey::Enter),
            _ => None,
        },
        OverlayKind::Rename
        | OverlayKind::NewWorkspace
        | OverlayKind::NewNode
        | OverlayKind::ResumeSessionEdit => match key.code {
            KeyCode::Esc => Some(OverlayKey::Esc),
            KeyCode::Enter => Some(OverlayKey::Enter),
            KeyCode::Char('c' | 'C') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                Some(OverlayKey::Clear)
            }
            // `^D` 进入删除确认 / 确认删除（`/resume` 编辑模态）。
            KeyCode::Char('d' | 'D') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                Some(OverlayKey::Remove)
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
        OverlayKind::SaveDraft => match key.code {
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

/// 编辑键映射；Shift 用于字符输入、@ 面板目录导航与 `Ctrl/Cmd+Enter` 变体。
///
/// `Shift+Enter` 映射为 @ 面板目录进入命令；面板未消费时回落为行尾另起一行
/// （见 `update::apply_editor`）。@ 面板的目录回退走编辑器撤销（`Ctrl+Z` / `Cmd+Z`）。
/// `Ctrl/Cmd+Shift+Enter` 与 `Ctrl/Cmd+Enter` 同为行尾另起一行并保留行首缩进
/// （对齐 lx-agent 的 `Ctrl/Cmd+Shift+Enter`）。
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
            KeyCode::Enter => Some(EditorCommand::NewlineBelow),
            KeyCode::Char('z' | 'Z') if shift => Some(EditorCommand::Redo),
            KeyCode::Char('z' | 'Z') => Some(EditorCommand::Undo),
            KeyCode::Char('y' | 'Y') => Some(EditorCommand::Redo),
            _ => None,
        };
    }
    let command = match (ctrl, alt, key.code) {
        (true, false, KeyCode::Enter) => Some(EditorCommand::NewlineBelow),
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

#[cfg(test)]
mod process_keys;
