//! 启动信任页：进入应用前的行内信任页（左上对齐 + 键鼠选择，对齐 Claude Code）。
//!
//! 在普通终端屏幕（非备用屏、不进入主界面）显示独立小循环：进入时清屏，
//! `↑/↓` 移动选中、`Enter` 确认、`Esc` 取消、`y`/`n` 快捷选择，鼠标悬停跟随、点击确认；
//! 默认选中 `No, exit`（安全默认）。选择 `Yes` 放行并登记信任；选择 `No` 不进入应用
//! （由入口输出提示并以 0 退出）。

use std::io;
use std::path::Path;

use crossterm::event::{Event, KeyCode, KeyEventKind, MouseButton, MouseEventKind, read};
use ratatui::backend::Backend as _;
use ratatui::layout::Rect;

use crate::app::state::AppState;
use crate::db::service::Persistence;
use crate::tui::Tui;
use crate::ui;

/// 选项索引：`No, exit` 在前为安全默认，`Yes, I trust this folder` 在后。
const OPTION_NO: usize = 0;
const OPTION_YES: usize = 1;

/// 启动信任门：未信任时在行内显示信任页；返回是否放行（已信任或用户确认）。
pub fn startup_trust(persistence: &Persistence, state: &mut AppState) -> io::Result<bool> {
    let Some(path) = persistence.untrusted_startup_path(state) else {
        return Ok(true);
    };
    let name = state.active_workspace().name.clone();
    // 行内会话：不进入备用屏，先在当前终端屏幕显示信任页。
    let mut tui = Tui::init_inline()?;
    let outcome = screen(&mut tui, &path);
    tui.restore()?;
    let granted = outcome?;
    if granted {
        persistence.trust_path(&path, &name);
    }
    Ok(granted)
}

/// 信任页交互循环：返回是否选择 `Yes, I trust this folder`。
fn screen(tui: &mut Tui, path: &Path) -> io::Result<bool> {
    let mut selected = OPTION_NO;
    // 进入时（与尺寸变化后）清掉终端原有内容，页面以干净屏幕左上对齐绘制。
    let mut needs_clear = true;
    loop {
        let size = tui.terminal().size()?;
        if needs_clear {
            tui.terminal().backend_mut().clear()?;
            needs_clear = false;
        }
        let screen = Rect::new(0, 0, size.width, size.height);
        let layout = ui::trust::layout(screen, path);
        tui.draw(|frame| ui::trust::render(frame, &layout, selected))?;
        match read()? {
            Event::Key(key) if key.kind != KeyEventKind::Release => match key.code {
                KeyCode::Up => selected = selected.saturating_sub(1),
                KeyCode::Down => selected = (selected + 1).min(layout.options.len() - 1),
                KeyCode::Enter => return Ok(selected == OPTION_YES),
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
                        return Ok(option == OPTION_YES);
                    }
                }
                _ => {}
            },
            // 尺寸变化后清屏重绘。
            Event::Resize(_, _) => needs_clear = true,
            _ => {}
        }
    }
}
