//! crossterm 按键到终端字节序列的编码；方向键形态由 DECCKM 模式决定。

use alacritty_terminal::term::TermMode;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEventKind};

/// 把按键编码为写入 PTY 的字节；不支持的按键返回 None。
///
/// Super（Cmd）组合只属于应用级快捷键，绝不写入 PTY；kitty 键盘协议下尤其重要，
/// 否则 Cmd+C 会被当作普通 `c` 打进 shell。
pub fn encode_key(key: KeyEvent, mode: TermMode) -> Option<Vec<u8>> {
    if key.modifiers.contains(KeyModifiers::SUPER) {
        return None;
    }
    let modifiers = key.modifiers;
    match key.code {
        KeyCode::Char(c) => Some(encode_char(c, modifiers)),
        KeyCode::Enter => Some(vec![b'\r']),
        KeyCode::Backspace => Some(vec![0x7f]),
        KeyCode::Tab => Some(vec![b'\t']),
        KeyCode::BackTab => Some(b"\x1b[Z".to_vec()),
        KeyCode::Esc => Some(vec![0x1b]),
        KeyCode::Up => Some(cursor_key(b'A', modifiers, mode)),
        KeyCode::Down => Some(cursor_key(b'B', modifiers, mode)),
        KeyCode::Right => Some(cursor_key(b'C', modifiers, mode)),
        KeyCode::Left => Some(cursor_key(b'D', modifiers, mode)),
        KeyCode::Home => Some(cursor_key(b'H', modifiers, mode)),
        KeyCode::End => Some(cursor_key(b'F', modifiers, mode)),
        KeyCode::Insert => Some(tilde_key(2, modifiers)),
        KeyCode::Delete => Some(tilde_key(3, modifiers)),
        KeyCode::PageUp => Some(tilde_key(5, modifiers)),
        KeyCode::PageDown => Some(tilde_key(6, modifiers)),
        KeyCode::F(number) => function_key(number, modifiers),
        _ => None,
    }
}

/// 编码滚轮为终端鼠标序列；协议由 `mode` 的 SGR/UTF8 位选择，旧式协议兜底。
///
/// `column`/`row` 为窗格内容区 0 基坐标；三类协议各自换算成 1 基并加固定偏移；
/// 旧式协议坐标超出单字节上限时返回 None（应用已在真正拖动，忽略一个滚轮事件可接受）。
pub fn encode_mouse_wheel(
    kind: MouseEventKind,
    column: u16,
    row: u16,
    modifiers: KeyModifiers,
    mode: TermMode,
) -> Option<Vec<u8>> {
    let mut code = match kind {
        MouseEventKind::ScrollUp => 64u16,
        MouseEventKind::ScrollDown => 65,
        MouseEventKind::ScrollLeft => 66,
        MouseEventKind::ScrollRight => 67,
        _ => return None,
    };
    if modifiers.contains(KeyModifiers::SHIFT) {
        code += 4;
    }
    if modifiers.contains(KeyModifiers::ALT) {
        code += 8;
    }
    if modifiers.contains(KeyModifiers::CONTROL) {
        code += 16;
    }

    if mode.contains(TermMode::SGR_MOUSE) {
        return Some(format!("\x1b[<{code};{};{}M", column + 1, row + 1).into_bytes());
    }

    let mut bytes = b"\x1b[M".to_vec();
    if mode.contains(TermMode::UTF8_MOUSE) {
        push_mouse_codepoint(&mut bytes, u32::from(code) + 32)?;
        push_mouse_codepoint(&mut bytes, u32::from(column) + 33)?;
        push_mouse_codepoint(&mut bytes, u32::from(row) + 33)?;
    } else {
        bytes.push(u8::try_from(code + 32).ok()?);
        bytes.push(u8::try_from(column + 33).ok()?);
        bytes.push(u8::try_from(row + 33).ok()?);
    }
    Some(bytes)
}

/// UTF-8 编码一个鼠标协议码点；超出合法码点返回 None。
fn push_mouse_codepoint(bytes: &mut Vec<u8>, value: u32) -> Option<()> {
    let ch = char::from_u32(value)?;
    let mut buffer = [0u8; 4];
    bytes.extend_from_slice(ch.encode_utf8(&mut buffer).as_bytes());
    Some(())
}

fn encode_char(c: char, modifiers: KeyModifiers) -> Vec<u8> {
    let mut bytes = Vec::new();
    if modifiers.contains(KeyModifiers::ALT) {
        bytes.push(0x1b);
    }
    if modifiers.contains(KeyModifiers::CONTROL) {
        bytes.push(control_byte(c));
    } else {
        let mut buffer = [0u8; 4];
        bytes.extend_from_slice(c.encode_utf8(&mut buffer).as_bytes());
    }
    bytes
}

fn control_byte(c: char) -> u8 {
    match c.to_ascii_uppercase() {
        ' ' | '@' => 0x00,
        'A'..='Z' => (c.to_ascii_uppercase() as u8) - 0x40,
        '[' => 0x1b,
        '\\' => 0x1c,
        ']' => 0x1d,
        '^' => 0x1e,
        '_' => 0x1f,
        '?' => 0x7f,
        other => other as u8,
    }
}

fn modifier_param(modifiers: KeyModifiers) -> u8 {
    let mut value = 1;
    if modifiers.contains(KeyModifiers::SHIFT) {
        value += 1;
    }
    if modifiers.contains(KeyModifiers::ALT) {
        value += 2;
    }
    if modifiers.contains(KeyModifiers::CONTROL) {
        value += 4;
    }
    value
}

fn cursor_key(final_byte: u8, modifiers: KeyModifiers, mode: TermMode) -> Vec<u8> {
    let param = modifier_param(modifiers);
    if param == 1 {
        if mode.contains(TermMode::APP_CURSOR) {
            vec![0x1b, b'O', final_byte]
        } else {
            vec![0x1b, b'[', final_byte]
        }
    } else {
        format!("\x1b[1;{param}{}", final_byte as char).into_bytes()
    }
}

fn tilde_key(code: u8, modifiers: KeyModifiers) -> Vec<u8> {
    let param = modifier_param(modifiers);
    if param == 1 {
        format!("\x1b[{code}~").into_bytes()
    } else {
        format!("\x1b[{code};{param}~").into_bytes()
    }
}

fn function_key(number: u8, modifiers: KeyModifiers) -> Option<Vec<u8>> {
    let param = modifier_param(modifiers);
    let ss3 = matches!(number, 1..=4);
    let code = match number {
        1 => 'P',
        2 => 'Q',
        3 => 'R',
        4 => 'S',
        5 => return Some(tilde_key(15, modifiers)),
        6 => return Some(tilde_key(17, modifiers)),
        7 => return Some(tilde_key(18, modifiers)),
        8 => return Some(tilde_key(19, modifiers)),
        9 => return Some(tilde_key(20, modifiers)),
        10 => return Some(tilde_key(21, modifiers)),
        11 => return Some(tilde_key(23, modifiers)),
        12 => return Some(tilde_key(24, modifiers)),
        _ => return None,
    };
    if param == 1 {
        let prefix = if ss3 { "O" } else { "[" };
        Some(format!("\x1b{prefix}{code}").into_bytes())
    } else {
        Some(format!("\x1b[1;{param}{code}").into_bytes())
    }
}

#[cfg(test)]
mod tests;
