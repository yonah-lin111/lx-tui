//! 启动信任页：进入应用前的控制台信任门（logo + 边框对话框 + 键鼠选择）。
//!
//! 在 `Tui::init` 之后、主事件循环之前运行一个独立的阻塞式小循环：
//! `↑/↓` 移动选中、`Enter` 确认、`Esc` 取消、`y`/`n` 快捷选择，鼠标悬停跟随、点击确认；
//! 未信任路径才进入该页面，已信任直接返回。拒绝时不进入应用（由入口输出提示并退出）。

use std::io;

use std::path::Path;

use crossterm::event::{Event, KeyCode, KeyEventKind, MouseButton, MouseEventKind, read};
use ratatui::layout::Rect;
use ratatui::widgets::Clear;

use crate::app::state::AppState;
use crate::db::service::Persistence;
use crate::tui::Tui;
use crate::ui;

/// 启动信任门：返回是否放行（已信任或用户确认）；确认后登记信任。
pub fn startup_trust(
    tui: &mut Tui,
    persistence: &Persistence,
    state: &mut AppState,
) -> io::Result<bool> {
    let Some(path) = persistence.untrusted_startup_path(state) else {
        return Ok(true);
    };
    let name = state.active_workspace().name.clone();
    let granted = screen(tui, &path)?;
    if granted {
        persistence.trust_path(&path, &name);
        // 清掉信任页画面，避免主界面首帧 diff 残留 logo 像素。
        tui.draw(|frame| frame.render_widget(Clear, frame.area()))?;
    }
    Ok(granted)
}

/// 信任页交互循环：返回是否选择 `Yes, proceed`。
fn screen(tui: &mut Tui, path: &Path) -> io::Result<bool> {
    let mut selected = 0usize;
    loop {
        let size = tui.terminal().size()?;
        let screen = Rect::new(0, 0, size.width, size.height);
        let layout = ui::trust::layout(screen, path);
        tui.draw(|frame| ui::trust::render(frame, &layout, selected))?;
        match read()? {
            Event::Key(key) if key.kind != KeyEventKind::Release => match key.code {
                KeyCode::Up => selected = selected.saturating_sub(1),
                KeyCode::Down => selected = (selected + 1).min(layout.options.len() - 1),
                KeyCode::Enter => return Ok(selected == 0),
                KeyCode::Esc => return Ok(false),
                KeyCode::Char('y' | 'Y') => return Ok(true),
                KeyCode::Char('n' | 'N') => return Ok(false),
                _ => {}
            },
            Event::Mouse(mouse) => match mouse.kind {
                MouseEventKind::Moved => {
                    if let Some(option) = ui::trust::option_at(&layout, mouse.column, mouse.row) {
                        selected = option;
                    }
                }
                MouseEventKind::Down(MouseButton::Left) => {
                    if let Some(option) = ui::trust::option_at(&layout, mouse.column, mouse.row) {
                        return Ok(option == 0);
                    }
                }
                _ => {}
            },
            // 尺寸变化只需重绘；其余事件忽略。
            Event::Resize(_, _) => {}
            _ => {}
        }
    }
}
