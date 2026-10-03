//! 终端生命周期：原始模式、备用屏幕、括号粘贴与恢复。

use std::io::{self, Stdout};

use crossterm::cursor::Show;
use crossterm::event::{DisableBracketedPaste, EnableBracketedPaste};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

/// 持有终端句柄；Drop 时兜底恢复终端状态。
pub struct Tui {
    terminal: Terminal<CrosstermBackend<Stdout>>,
    restored: bool,
}

impl Tui {
    /// 进入原始模式与备用屏幕，并安装 panic 恢复钩子。
    pub fn init() -> io::Result<Self> {
        install_panic_hook();
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableBracketedPaste)?;
        let terminal = Terminal::new(CrosstermBackend::new(stdout))?;
        Ok(Self {
            terminal,
            restored: false,
        })
    }

    /// 终端句柄。
    pub fn terminal(&mut self) -> &mut Terminal<CrosstermBackend<Stdout>> {
        &mut self.terminal
    }

    /// 恢复终端；重复调用安全。
    pub fn restore(&mut self) -> io::Result<()> {
        if self.restored {
            return Ok(());
        }
        self.restored = true;
        disable_raw_mode()?;
        execute!(
            self.terminal.backend_mut(),
            LeaveAlternateScreen,
            DisableBracketedPaste,
            Show
        )?;
        self.terminal.show_cursor()
    }
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
    disable_raw_mode()?;
    execute!(
        io::stdout(),
        LeaveAlternateScreen,
        DisableBracketedPaste,
        Show
    )
}
