//! 终端生命周期：原始模式、备用屏幕、鼠标上报与恢复。

mod cursor;

use std::io::{self, Stdout, Write};
use std::time::{Duration, Instant};

use crossterm::cursor::Show;
use crossterm::event::{DisableBracketedPaste, EnableBracketedPaste};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

use crate::tui::cursor::QuietCursor;

/// 退出清空输入时的静默窗口与等待上限（覆盖禁用序列到达终端前的在途上报）。
const DRAIN_QUIET: Duration = Duration::from_millis(10);
const DRAIN_LIMIT: Duration = Duration::from_millis(100);

/// 关闭硬件光标闪烁（DEC 私有模式 12）：只改闪烁、不改终端配置的光标形状。
///
/// 动画重绘会重置部分终端（Ghostty 等）的闪烁相位，硬件光标闪烁无法在重绘下
/// 稳定维持，只能整体关掉；不支持的终端静默忽略。
const CURSOR_BLINK_OFF: &[u8] = b"\x1b[?12l";
/// 恢复硬件光标闪烁，退出时还原 TUI 进入前的常态。
const CURSOR_BLINK_ON: &[u8] = b"\x1b[?12h";

/// 持有终端句柄；Drop 时兜底恢复终端状态。
pub struct Tui {
    terminal: Terminal<QuietCursor<CrosstermBackend<Stdout>>>,
    restored: bool,
}

impl Tui {
    /// 进入原始模式与备用屏幕，并安装 panic 恢复钩子。
    pub fn init() -> io::Result<Self> {
        install_panic_hook();
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableBracketedPaste)?;
        crate::platform::enable_mouse_capture(&mut stdout)?;
        crate::platform::enable_keyboard_enhancement(&mut stdout)?;
        stdout.write_all(CURSOR_BLINK_OFF)?;
        stdout.flush()?;
        let terminal = Terminal::new(QuietCursor::new(CrosstermBackend::new(stdout)))?;
        Ok(Self {
            terminal,
            restored: false,
        })
    }

    /// 终端句柄。
    pub(crate) fn terminal(&mut self) -> &mut Terminal<QuietCursor<CrosstermBackend<Stdout>>> {
        &mut self.terminal
    }

    /// 恢复终端；重复调用安全。
    pub fn restore(&mut self) -> io::Result<()> {
        if self.restored {
            return Ok(());
        }
        self.restored = true;
        let _ = crate::platform::set_pointer_shape(crate::platform::PointerShape::Default);
        restore_terminal(
            self.terminal.backend_mut(),
            &mut drain_pending_input,
            &mut disable_raw_mode,
        )?;
        self.terminal.show_cursor()
    }
}

/// 退出恢复顺序：先关鼠标上报，仍在原始模式下丢弃在途上报，再弹出键盘增强并离开备用屏，
/// 最后恢复 cooked 模式。
///
/// 顺序不可交换：cooked 模式下未成行的残留上报会被终端回显并留在行缓冲，泄漏进后续 shell。
fn restore_terminal(
    writer: &mut impl Write,
    drain: &mut dyn FnMut(),
    restore_cooked: &mut dyn FnMut() -> io::Result<()>,
) -> io::Result<()> {
    crate::platform::disable_mouse_capture(writer)?;
    drain();
    crate::platform::disable_keyboard_enhancement(writer)?;
    writer.write_all(CURSOR_BLINK_ON)?;
    execute!(writer, LeaveAlternateScreen, DisableBracketedPaste, Show)?;
    restore_cooked()
}

impl Drop for Tui {
    fn drop(&mut self) {
        // 正常退出已恢复；Drop 只兜底，忽略错误。
        let _ = self.restore();
    }
}

/// panic 时先恢复终端，再交回默认钩子。
fn install_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        let _ = restore_raw();
        default_hook(panic_info);
    }));
}

fn restore_raw() -> io::Result<()> {
    let _ = crate::platform::set_pointer_shape(crate::platform::PointerShape::Default);
    restore_terminal(
        &mut io::stdout(),
        &mut drain_pending_input,
        &mut disable_raw_mode,
    )
}

/// 丢弃已排队的终端输入：先给在途鼠标上报留出到达窗口，静默一段后结束。
fn drain_pending_input() {
    let deadline = Instant::now() + DRAIN_LIMIT;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        match crossterm::event::poll(DRAIN_QUIET.min(remaining)) {
            Ok(true) => {
                if crossterm::event::read().is_err() {
                    break;
                }
            }
            _ => break,
        }
    }
}

#[cfg(test)]
mod tests;
