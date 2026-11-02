//! crossterm 按键到终端字节序列的编码；方向键形态由 DECCKM 模式决定。

use alacritty_terminal::term::TermMode;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEventKind};

/// 把按键编码为写入 PTY 的字节；不支持的按键返回 None。
///
/// Legacy 终端下拦截全部 Super（Cmd）组合，防止把 Cmd+C/Cmd+Z 当成普通字符打进 shell；
/// Kitty 键盘协议（`DISAMBIGUATE_ESC_CODES` / `REPORT_ALL_KEYS_AS_ESC`）激活时，除纯 Super+C
/// 保留给宿主复制选区外，其余 Super 组合（如 Super+Z 撤销、Super+Shift+Z 重做、Super+A 全选）
/// 按 `CSI u` 编码转发给子 TUI。
pub fn encode_key(key: KeyEvent, mode: TermMode) -> Option<Vec<u8>> {
    let kitty =
        mode.intersects(TermMode::DISAMBIGUATE_ESC_CODES | TermMode::REPORT_ALL_KEYS_AS_ESC);
    let report_all = mode.contains(TermMode::REPORT_ALL_KEYS_AS_ESC);

    if key.modifiers.contains(KeyModifiers::SUPER) {
        // Legacy 终端无法表达 Super（Cmd），直接丢弃以免把 Cmd+C/Cmd+Z 当成普通字符打进 shell；
        // Kitty 协议下除纯 Super+C（保留给宿主复制选区兜底、避免在 claude 中误插入 `c`）外，
        // 其余 Super 组合（如 Super+Z 撤销、Super+Shift+Z 重做、Super+A 全选）按 CSI u 编码转发。
        if !kitty
            || (key.modifiers == KeyModifiers::SUPER
                && matches!(key.code, KeyCode::Char('c' | 'C')))
        {
            return None;
        }
    }

    let (code, modifiers) = match key.code {
        KeyCode::BackTab => (KeyCode::Tab, key.modifiers | KeyModifiers::SHIFT),
        other => (other, key.modifiers),
    };

    match code {
        KeyCode::Char(c) => Some(encode_char(c, modifiers, kitty, report_all)),
        KeyCode::Enter => Some(encode_enter(modifiers, kitty, report_all)),
        KeyCode::Backspace => Some(encode_backspace(modifiers, kitty, report_all)),
        KeyCode::Tab => Some(encode_tab(modifiers, kitty, report_all)),
        KeyCode::Esc => Some(encode_esc(modifiers, kitty, report_all)),
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

fn csi_u(codepoint: u32, modifiers: KeyModifiers) -> Vec<u8> {
    let param = modifier_param(modifiers);
    if param == 1 {
        format!("\x1b[{codepoint}u").into_bytes()
    } else {
        format!("\x1b[{codepoint};{param}u").into_bytes()
    }
}

fn modify_other_key(codepoint: u32, modifiers: KeyModifiers) -> Vec<u8> {
    let param = modifier_param(modifiers);
    format!("\x1b[27;{param};{codepoint}~").into_bytes()
}

fn encode_enter(modifiers: KeyModifiers, kitty: bool, report_all: bool) -> Vec<u8> {
    if modifiers.is_empty() {
        return if report_all {
            csi_u(13, modifiers)
        } else {
            vec![b'\r']
        };
    }
    if kitty {
        return csi_u(13, modifiers);
    }
    if modifiers == KeyModifiers::ALT {
        vec![0x1b, b'\r']
    } else {
        modify_other_key(13, modifiers)
    }
}

fn encode_tab(modifiers: KeyModifiers, kitty: bool, report_all: bool) -> Vec<u8> {
    if modifiers.is_empty() {
        return if report_all {
            csi_u(9, modifiers)
        } else {
            vec![b'\t']
        };
    }
    if kitty {
        return csi_u(9, modifiers);
    }
    if modifiers == KeyModifiers::SHIFT {
        b"\x1b[Z".to_vec()
    } else if modifiers == KeyModifiers::ALT {
        vec![0x1b, b'\t']
    } else {
        modify_other_key(9, modifiers)
    }
}

fn encode_backspace(modifiers: KeyModifiers, kitty: bool, report_all: bool) -> Vec<u8> {
    if modifiers.is_empty() {
        return if report_all {
            csi_u(127, modifiers)
        } else {
            vec![0x7f]
        };
    }
    if kitty {
        return csi_u(127, modifiers);
    }
    if modifiers == KeyModifiers::ALT {
        vec![0x1b, 0x7f]
    } else if modifiers == KeyModifiers::CONTROL {
        vec![0x08]
    } else if modifiers == (KeyModifiers::ALT | KeyModifiers::CONTROL) {
        vec![0x1b, 0x08]
    } else {
        modify_other_key(127, modifiers)
    }
}

fn encode_esc(modifiers: KeyModifiers, kitty: bool, report_all: bool) -> Vec<u8> {
    if kitty || report_all {
        return csi_u(27, modifiers);
    }
    if modifiers.is_empty() {
        vec![0x1b]
    } else if modifiers == KeyModifiers::ALT {
        vec![0x1b, 0x1b]
    } else {
        modify_other_key(27, modifiers)
    }
}

fn encode_char(c: char, modifiers: KeyModifiers, kitty: bool, report_all: bool) -> Vec<u8> {
    let has_ctrl = modifiers.contains(KeyModifiers::CONTROL);
    let has_alt = modifiers.contains(KeyModifiers::ALT);
    let has_super = modifiers.contains(KeyModifiers::SUPER);
    let has_shift = modifiers.contains(KeyModifiers::SHIFT);

    // 无 Ctrl/Alt/Super 的普通或 Shift 可打印字符：除非开启 REPORT_ALL_KEYS_AS_ESC，一律发 UTF-8 文本。
    if !has_ctrl && !has_alt && !has_super {
        if report_all {
            let base = if has_shift && c.is_ascii_uppercase() {
                c.to_ascii_lowercase() as u32
            } else {
                c as u32
            };
            return csi_u(base, modifiers);
        }
        let mut buffer = [0u8; 4];
        return c.encode_utf8(&mut buffer).as_bytes().to_vec();
    }

    let base_codepoint = if c.is_ascii_alphabetic() {
        c.to_ascii_lowercase() as u32
    } else {
        c as u32
    };

    // Kitty 协议激活时（或 Legacy 下的 Ctrl+Shift+<char>）：对齐 Ghostty 与 herdr，
    // 所有带 Ctrl / Alt / Super 的字符键统一编码为 CSI u（如 Super+Z -> \x1b[122;9u，
    // Ctrl+Z -> \x1b[122;5u，Ctrl+- -> \x1b[45;5u）。
    if kitty || (has_ctrl && has_shift) {
        return csi_u(base_codepoint, modifiers);
    }

    let mut bytes = Vec::new();
    if has_alt {
        bytes.push(0x1b);
    }
    if has_ctrl {
        let Some(ctrl) = control_byte(c) else {
            return if c.is_ascii() {
                csi_u(base_codepoint, modifiers)
            } else {
                Vec::new()
            };
        };
        bytes.push(ctrl);
    } else {
        let mut buffer = [0u8; 4];
        bytes.extend_from_slice(c.encode_utf8(&mut buffer).as_bytes());
    }
    bytes
}

fn control_byte(c: char) -> Option<u8> {
    Some(match c.to_ascii_uppercase() {
        ' ' | '@' | '`' | '2' => 0x00,
        'A'..='Z' => (c.to_ascii_uppercase() as u8) - 0x40,
        '[' | '3' => 0x1b,
        '\\' | '4' => 0x1c,
        ']' | '5' => 0x1d,
        '^' | '~' | '6' => 0x1e,
        '_' | '-' | '/' | '7' => 0x1f,
        '?' | '8' => 0x7f,
        _ => return None,
    })
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
    if modifiers.contains(KeyModifiers::SUPER) {
        value += 8;
    }
    if modifiers.contains(KeyModifiers::HYPER) {
        value += 16;
    }
    if modifiers.contains(KeyModifiers::META) {
        value += 32;
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
