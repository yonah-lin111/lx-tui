//! crossterm 按键到终端字节序列的编码；方向键形态由 DECCKM 模式决定。

use alacritty_terminal::term::TermMode;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEventKind};

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
    let code = match kind {
        MouseEventKind::ScrollUp => 64u16,
        MouseEventKind::ScrollDown => 65,
        MouseEventKind::ScrollLeft => 66,
        MouseEventKind::ScrollRight => 67,
        _ => return None,
    };
    encode_mouse_sequence(code, column, row, modifiers, false, mode)
}

/// 编码鼠标按键/拖拽/移动为终端鼠标序列；协议选择与坐标规则同 `encode_mouse_wheel`。
///
/// `Drag` 与 `Moved` 分别是加了 32 的按键移动位与无按键移动位；`Up` 在 SGR 下以小写 `m`
/// 收尾，旧式协议用按钮 3 表示释放。
pub fn encode_mouse_button(
    kind: MouseEventKind,
    column: u16,
    row: u16,
    modifiers: KeyModifiers,
    mode: TermMode,
) -> Option<Vec<u8>> {
    let (code, release) = match kind {
        MouseEventKind::Down(button) => (mouse_button_code(button), false),
        MouseEventKind::Up(button) => (mouse_button_code(button), true),
        MouseEventKind::Drag(button) => (mouse_button_code(button) + 32, false),
        MouseEventKind::Moved => (35, false),
        _ => return None,
    };
    encode_mouse_sequence(code, column, row, modifiers, release, mode)
}

/// 鼠标按键的基础协议码：左 0、中 1、右 2。
fn mouse_button_code(button: MouseButton) -> u16 {
    match button {
        MouseButton::Left => 0,
        MouseButton::Middle => 1,
        MouseButton::Right => 2,
    }
}

/// 修饰键协议偏移：Shift +4、Alt +8、Control +16。
fn modifier_offset(modifiers: KeyModifiers) -> u16 {
    let mut offset = 0;
    if modifiers.contains(KeyModifiers::SHIFT) {
        offset += 4;
    }
    if modifiers.contains(KeyModifiers::ALT) {
        offset += 8;
    }
    if modifiers.contains(KeyModifiers::CONTROL) {
        offset += 16;
    }
    offset
}

/// 编码鼠标协议序列：SGR 优先，其次 UTF8，最后旧式；坐标均为 1 基。
fn encode_mouse_sequence(
    code: u16,
    column: u16,
    row: u16,
    modifiers: KeyModifiers,
    release: bool,
    mode: TermMode,
) -> Option<Vec<u8>> {
    let offset = modifier_offset(modifiers);
    if mode.contains(TermMode::SGR_MOUSE) {
        let terminator = if release { 'm' } else { 'M' };
        return Some(
            format!(
                "\x1b[<{};{};{}{terminator}",
                code + offset,
                column + 1,
                row + 1
            )
            .into_bytes(),
        );
    }

    let code = if release { 3 + offset } else { code + offset };
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
