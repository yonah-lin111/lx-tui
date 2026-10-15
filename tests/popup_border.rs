//! 回归测试：弹出类浮层的边框在底层宽字符（中文）跨入边框列时仍完整显示。
//!
//! 复现路径：底层窗格终端先绘制宽字符（右半格标记 diff 跳过），随后浮层打开、
//! 其左边框恰好落在该右半格上。修复前 ratatui 的 diff 因「宽字符左半格未变」整格跳过，
//! 左缘那一列永远不下发到终端，边框出现缺口；修复后应完整。

use lx_tui::app::overlay::{Overlay, ReferencedFolderMenu, ReferencedFolderMenuRow};
use lx_tui::app::state::{AppState, PaneView};
use lx_tui::app::update;
use lx_tui::config::Config;
use lx_tui::layout;
use lx_tui::ui;
use ratatui::Terminal;
use ratatui::backend::{Backend, ClearType, TestBackend, WindowSize};
use ratatui::buffer::Cell;
use ratatui::layout::{Position, Rect, Size};
use std::io;

const WIDTH: u16 = 160;
const HEIGHT: u16 = 24;

/// 引用文件夹目录弹窗宽度（与实现一致，用于推算左缘列）。
const MENU_WIDTH: u16 = 72;

/// 场景：100x24，终端窗格聚焦，几何已同步。
fn ready_state() -> (AppState, Config, Rect) {
    let config = Config::default();
    let mut state = AppState::demo();
    let screen = Rect::new(0, 0, WIDTH, HEIGHT);
    let view = ui::layout::compute(
        screen,
        &config,
        state.sidebar_collapsed,
        state.sidebar_width,
        state.prompt_collapsed,
        state.prompt_width,
        state.prompt_left,
        state.panes_collapsed,
    );
    let mut rects = layout::pane_rects(
        &state.active_tab().layout,
        view.panes,
        config.min_pane_width,
        config.min_pane_height,
    );
    rects.push((state.prompt.id(), view.prompt));
    update::resize_panes(&mut state, &rects);
    let pane = state.active_tab().layout.focus();
    if let Some(target) = state.active_tab_mut().pane_mut(pane) {
        target.view = PaneView::Terminal;
    }
    update::focus_pane(&mut state, pane);
    let pane_rect = rects
        .iter()
        .find(|(id, _)| *id == pane)
        .map(|(_, rect)| *rect)
        .expect("focused pane is tiled");
    (state, config, pane_rect)
}

/// 在窗格终端喂入内容，使一个宽字符的左半格落在屏幕列 `lead_col`、右半格落在其右侧。
///
/// 返回该宽字符所在的屏幕行。
fn feed_wide_char_straddling(
    state: &mut AppState,
    config: &Config,
    pane_rect: Rect,
    lead_col: u16,
) -> u16 {
    let inner = layout::pane_inner_rect(pane_rect);
    let _ = config;
    // 目标宽字符行：落在弹窗内部（避开上下边框）的一行。
    let popup_y = (HEIGHT - 18) / 2;
    let target_screen_y = popup_y + 5;
    let content_row = target_screen_y.saturating_sub(inner.y);
    let column = lead_col.saturating_sub(inner.x);

    let mut bytes = Vec::new();
    for _ in 0..content_row {
        bytes.extend_from_slice(b"\r\n");
    }
    bytes.extend(std::iter::repeat_n(b' ', usize::from(column)));
    bytes.extend_from_slice("页".as_bytes());

    let pane = state.active_tab().layout.focus();
    update::feed_pane(state, pane, &bytes);
    target_screen_y
}

fn draw_once(terminal: &mut Terminal<TestBackend>, state: &AppState, config: &Config) {
    terminal
        .draw(|frame| ui::render(frame, state, config))
        .expect("draw succeeds");
}

fn referenced_menu_overlay() -> Overlay {
    Overlay::ReferencedFolderMenu(ReferencedFolderMenu {
        anchor: (0, 0),
        key: None,
        rows: vec![
            ReferencedFolderMenuRow::Folder {
                id: 1,
                path: std::path::PathBuf::from("/tmp/lx-agent"),
            },
            ReferencedFolderMenuRow::Add,
        ],
        selected: 0,
        hover: None,
        viewport: None,
    })
}

#[test]
fn popup_left_border_shows_over_straddling_wide_char() {
    let (mut state, config, pane_rect) = ready_state();

    // 弹窗左缘列：72 宽居中。
    let border_x = (WIDTH - MENU_WIDTH) / 2;
    // 让底层宽字符的左半格落在 border_x - 1，右半格（diff 跳过格）落在 border_x。
    let row = feed_wide_char_straddling(&mut state, &config, pane_rect, border_x - 1);

    let mut terminal =
        Terminal::new(TestBackend::new(WIDTH, HEIGHT)).expect("test backend is infallible");

    // 第 1 帧：尚无浮层，底层宽字符绘制并落下。
    draw_once(&mut terminal, &state, &config);
    let before = terminal.backend().buffer()[(border_x, row)]
        .symbol()
        .to_string();
    assert_ne!(before, "│", "前置条件：宽字符存在前浮层边框尚未出现");

    // 第 2 帧：弹出浮层，左边框落在宽字符右半格上。
    state.overlay = Some(referenced_menu_overlay());
    draw_once(&mut terminal, &state, &config);

    let symbol = terminal.backend().buffer()[(border_x, row)]
        .symbol()
        .to_string();
    assert_eq!(
        symbol, "│",
        "弹出浮层左边框必须覆盖跨入的宽字符右半格并完整显示（row={row}, col={border_x}）"
    );

    // 被压住的宽字符左半格应被擦成空白（保底不露半个字）。
    let left = terminal.backend().buffer()[(border_x - 1, row)]
        .symbol()
        .to_string();
    assert_eq!(left, " ", "被浮层左缘压住的宽字符左半格应擦成空白");
}

/// 记录每帧实际下发到后端的单元格（diff 结果），用于断言「边框格确实被下发」。
///
/// `TestBackend` 的最终缓冲区会把跨宽字符的占位列都存成空格，
/// 无法区分「下发过空白刷新」与「从未下发」；这里记录 diff 本身，才是真实缺陷信号。
#[derive(Default)]
struct RecordingBackend {
    size: (u16, u16),
    /// 本帧下发的 (x, y) 坐标集合。
    painted: std::cell::RefCell<Vec<(u16, u16)>>,
}

impl Backend for RecordingBackend {
    type Error = io::Error;

    fn draw<'a, I>(&mut self, content: I) -> io::Result<()>
    where
        I: Iterator<Item = (u16, u16, &'a Cell)>,
    {
        let mut painted = self.painted.borrow_mut();
        for (x, y, _) in content {
            painted.push((x, y));
        }
        Ok(())
    }

    fn hide_cursor(&mut self) -> io::Result<()> {
        Ok(())
    }

    fn show_cursor(&mut self) -> io::Result<()> {
        Ok(())
    }

    fn get_cursor_position(&mut self) -> io::Result<Position> {
        Ok(Position::ORIGIN)
    }

    fn set_cursor_position<P: Into<Position>>(&mut self, _position: P) -> io::Result<()> {
        Ok(())
    }

    fn clear(&mut self) -> io::Result<()> {
        Ok(())
    }

    fn clear_region(&mut self, _clear_type: ClearType) -> io::Result<()> {
        Ok(())
    }

    fn size(&self) -> io::Result<Size> {
        Ok(Size::new(self.size.0, self.size.1))
    }

    fn window_size(&mut self) -> io::Result<WindowSize> {
        Ok(WindowSize {
            columns_rows: Size::new(self.size.0, self.size.1),
            pixels: Size::new(0, 0),
        })
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// 右缘：底层宽字符被浮层右缘切成两半时，外侧残半必须在 diff 中被刷新清除。
#[test]
fn popup_right_border_refreshes_outer_half_of_straddling_wide_char() {
    let (mut state, config, pane_rect) = ready_state();

    let left = (WIDTH - MENU_WIDTH) / 2;
    let right_border = left + MENU_WIDTH - 1;
    let spacer = right_border + 1;
    // 宽字符左半格落在浮层内最后一列（右缘），右半格露在浮层外。
    let row = feed_wide_char_straddling(&mut state, &config, pane_rect, right_border);

    let backend = RecordingBackend {
        size: (WIDTH, HEIGHT),
        ..Default::default()
    };
    let mut terminal = Terminal::new(backend).expect("terminal");

    // 第 1 帧：底层宽字符落下。
    draw_once_recording(&mut terminal, &state, &config);

    // 第 2 帧：弹出浮层。
    state.overlay = Some(referenced_menu_overlay());
    draw_once_recording(&mut terminal, &state, &config);

    let painted = terminal.backend().painted.borrow();
    assert!(
        painted.contains(&(right_border, row)),
        "浮层右边框格必须下发（row={row}, col={right_border}）"
    );
    assert!(
        painted.contains(&(spacer, row)),
        "浮层外残半格必须下发以清除残留（row={row}, col={spacer}）"
    );
}

fn draw_once_recording(
    terminal: &mut Terminal<RecordingBackend>,
    state: &AppState,
    config: &Config,
) {
    terminal.backend().painted.borrow_mut().clear();
    terminal
        .draw(|frame| ui::render(frame, state, config))
        .expect("draw succeeds");
}
