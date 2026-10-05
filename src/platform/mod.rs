//! 平台能力：剪贴板写入、鼠标捕获与默认 shell；OS 专属代码唯一落点。

use std::io::{self, IsTerminal, Write};
use std::time::{Duration, Instant};

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;

/// 原生复制命令的等待上限；超时终止，避免剪贴板被占用时无限阻塞。
const NATIVE_COPY_TIMEOUT: Duration = Duration::from_secs(2);

/// Unix 下关闭全部已知鼠标上报模式（含旧式模式，herdr `terminal_modes` 同款清理）。
#[cfg(not(windows))]
const DISABLE_MOUSE_REPORTING: &[u8] =
    b"\x1b[?1006l\x1b[?1016l\x1b[?1015l\x1b[?1005l\x1b[?1003l\x1b[?1002l\x1b[?1000l";
/// Unix 下上报按键、释放、拖动与无按键移动（SGR 1000/1002/1003/1006）；
/// 移动上报用于右栏分割线的悬停指针形状与高亮。
#[cfg(not(windows))]
const ENABLE_MOUSE_BUTTON_REPORTING: &[u8] = b"\x1b[?1000h\x1b[?1002h\x1b[?1003h\x1b[?1006h";

/// 鼠标指针形状（OSC 22）；不支持的终端静默忽略。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointerShape {
    Default,
    EwResize,
}

impl PointerShape {
    /// OSC 22 形状名；恢复默认必须显式发 `default`，空 payload 部分终端不认。
    fn name(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::EwResize => "ew-resize",
        }
    }
}

/// 设置鼠标指针形状；输出到 stdout，失败只影响形状提示。
pub fn set_pointer_shape(shape: PointerShape) -> io::Result<()> {
    let mut stdout = io::stdout();
    stdout.write_all(
        pointer_shape_sequence(
            shape,
            std::env::var_os("TMUX").is_some(),
            std::env::var_os("STY").is_some(),
        )
        .as_bytes(),
    )?;
    stdout.flush()
}

/// OSC 22 输出；tmux/screen 下按需加 passthrough 包装（与 OSC 52 同款）。
fn pointer_shape_sequence(shape: PointerShape, tmux: bool, screen: bool) -> String {
    let sequence = format!("\x1b]22;{}\x1b\\", shape.name());
    let passthrough = format!("\x1bPtmux;\x1b{sequence}\x1b\\");
    if tmux {
        format!("{sequence}{passthrough}")
    } else if screen {
        passthrough
    } else {
        sequence
    }
}

/// 写入系统剪贴板：OSC 52 立即写；原生工具放到后台线程并限时执行，绝不阻塞事件循环。
pub fn write_clipboard(text: &str) -> bool {
    let osc52 = write_osc52(text);
    let owned = text.to_string();
    let native = std::thread::Builder::new()
        .name("clipboard-write".to_string())
        .spawn(move || {
            if !native_copy(&owned) {
                tracing::debug!("native clipboard write failed");
            }
        })
        .is_ok();
    osc52 || native
}

/// 默认 shell：Unix 取 `$SHELL`，Windows 取 `%COMSPEC%`。
pub fn default_shell() -> String {
    #[cfg(windows)]
    let shell = std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".to_string());
    #[cfg(not(windows))]
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
    shell
}

/// 开启鼠标上报：Unix 用显式 VT 序列；Windows 走 crossterm 的 WinAPI 捕获。
pub fn enable_mouse_capture(writer: &mut impl Write) -> io::Result<()> {
    #[cfg(windows)]
    {
        crossterm::execute!(writer, crossterm::event::EnableMouseCapture)
    }
    #[cfg(not(windows))]
    {
        writer.write_all(ENABLE_MOUSE_BUTTON_REPORTING)?;
        writer.flush()
    }
}

/// 关闭鼠标上报；与开启路径一一对应。
pub fn disable_mouse_capture(writer: &mut impl Write) -> io::Result<()> {
    #[cfg(windows)]
    {
        crossterm::execute!(writer, crossterm::event::DisableMouseCapture)
    }
    #[cfg(not(windows))]
    {
        writer.write_all(DISABLE_MOUSE_REPORTING)?;
        writer.flush()
    }
}

/// OSC 52 输出：tmux/screen 下按需加 passthrough 包装（opencode 同款）。
fn osc52_output(text: &str, tmux: bool, screen: bool) -> String {
    let sequence = format!("\x1b]52;c;{}\x07", BASE64.encode(text.as_bytes()));
    let passthrough = format!("\x1bPtmux;\x1b{sequence}\x1b\\");
    if tmux {
        format!("{sequence}{passthrough}")
    } else if screen {
        passthrough
    } else {
        sequence
    }
}

fn write_osc52(text: &str) -> bool {
    let mut stdout = io::stdout();
    if !stdout.is_terminal() {
        return false;
    }
    let output = osc52_output(
        text,
        std::env::var_os("TMUX").is_some(),
        std::env::var_os("STY").is_some(),
    );
    stdout
        .write_all(output.as_bytes())
        .and_then(|()| stdout.flush())
        .is_ok()
}

/// 各平台原生复制命令；无可用工具或执行失败返回 false。
fn native_copy(text: &str) -> bool {
    #[cfg(target_os = "macos")]
    let copied = run_with_stdin("pbcopy", &[], text);
    #[cfg(windows)]
    let copied = run_with_stdin(
        "powershell.exe",
        &[
            "-NonInteractive",
            "-NoProfile",
            "-Command",
            "[Console]::InputEncoding = [System.Text.Encoding]::UTF8; \
             Set-Clipboard -Value ([Console]::In.ReadToEnd())",
        ],
        text,
    );
    #[cfg(target_os = "linux")]
    let copied = {
        let candidates: &[(&str, &[&str])] = &[
            ("wl-copy", &[]),
            ("xclip", &["-selection", "clipboard"]),
            ("xsel", &["--clipboard", "--input"]),
        ];
        candidates
            .iter()
            .any(|(program, args)| run_with_stdin(program, args, text))
    };
    #[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
    let copied = {
        let _ = text;
        false
    };
    copied
}

/// 以 stdin 传入文本执行复制命令；超时或失败返回 false。
fn run_with_stdin(program: &str, args: &[&str], text: &str) -> bool {
    use std::process::{Command, Stdio};

    let Ok(mut child) = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false;
    };
    let Some(mut stdin) = child.stdin.take() else {
        let _ = child.wait();
        return false;
    };
    if stdin.write_all(text.as_bytes()).is_err() {
        let _ = child.wait();
        return false;
    }
    drop(stdin);

    let deadline = Instant::now() + NATIVE_COPY_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return false;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn osc52_sequence_uses_bel_and_standard_base64() {
        assert_eq!(
            osc52_output("hello", false, false),
            "\x1b]52;c;aGVsbG8=\x07"
        );
        assert_eq!(osc52_output("", false, false), "\x1b]52;c;\x07");
    }

    #[test]
    fn osc52_sequence_encodes_multibyte_text() {
        assert_eq!(osc52_output("你好", false, false), "\x1b]52;c;5L2g5aW9\x07");
    }

    #[test]
    fn osc52_wraps_for_tmux_and_screen() {
        let plain = "\x1b]52;c;aGVsbG8=\x07";
        let passthrough = format!("\x1bPtmux;\x1b{plain}\x1b\\");
        assert_eq!(
            osc52_output("hello", true, false),
            format!("{plain}{passthrough}")
        );
        assert_eq!(osc52_output("hello", false, true), passthrough);
    }

    #[cfg(not(windows))]
    #[test]
    fn disable_sequence_covers_all_known_mouse_modes() {
        let Ok(text) = std::str::from_utf8(DISABLE_MOUSE_REPORTING) else {
            panic!("sequence is ascii");
        };
        for mode in ["1000", "1002", "1003", "1005", "1006", "1015", "1016"] {
            assert!(text.contains(&format!("\x1b[?{mode}l")), "missing {mode}");
        }
    }

    #[cfg(not(windows))]
    #[test]
    fn enable_sequence_requests_button_drag_and_move_events() {
        let Ok(text) = std::str::from_utf8(ENABLE_MOUSE_BUTTON_REPORTING) else {
            panic!("sequence is ascii");
        };
        for mode in ["1000", "1002", "1003", "1006"] {
            assert!(text.contains(&format!("\x1b[?{mode}h")), "missing {mode}");
        }
        for mode in ["1005", "1015", "1016"] {
            assert!(
                !text.contains(&format!("\x1b[?{mode}h")),
                "unexpected {mode}"
            );
        }
    }

    #[test]
    fn pointer_shape_sequence_sets_and_resets() {
        assert_eq!(
            pointer_shape_sequence(PointerShape::EwResize, false, false),
            "\x1b]22;ew-resize\x1b\\"
        );
        assert_eq!(
            pointer_shape_sequence(PointerShape::Default, false, false),
            "\x1b]22;default\x1b\\"
        );
    }

    #[test]
    fn pointer_shape_sequence_wraps_for_tmux_and_screen() {
        let plain = "\x1b]22;ew-resize\x1b\\";
        let passthrough = format!("\x1bPtmux;\x1b{plain}\x1b\\");
        assert_eq!(
            pointer_shape_sequence(PointerShape::EwResize, true, false),
            format!("{plain}{passthrough}")
        );
        assert_eq!(
            pointer_shape_sequence(PointerShape::EwResize, false, true),
            passthrough
        );
    }
}
