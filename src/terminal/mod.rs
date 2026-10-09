//! 终端仿真：alacritty_terminal 的纯内存封装，字节进、画面出，不触碰 IO。

use std::fmt;
use std::sync::{Arc, Mutex, PoisonError};

use alacritty_terminal::event::{Event as EmulatorEvent, EventListener};
use alacritty_terminal::grid::{Dimensions, Grid, Scroll};
use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection as TermSelection, SelectionType};
use alacritty_terminal::term::cell::{Cell, Flags};
use alacritty_terminal::term::{
    Config as EmulatorConfig, RenderableContent, Term, TermMode, viewport_to_point,
};
use alacritty_terminal::vte::ansi::{CursorShape, Processor};
use alacritty_terminal::vte::{Params, Parser, Perform};

/// 回滚缓冲行数。
const SCROLLBACK_LINES: usize = 10_000;

/// 终端网格尺寸。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GridSize {
    pub cols: u16,
    pub rows: u16,
}

impl Dimensions for GridSize {
    fn total_lines(&self) -> usize {
        usize::from(self.rows)
    }

    fn screen_lines(&self) -> usize {
        usize::from(self.rows)
    }

    fn columns(&self) -> usize {
        usize::from(self.cols)
    }
}

/// 收集仿真器回调事件；Term 持有其克隆，Terminal 读取同一队列。
#[derive(Debug, Clone, Default)]
struct Listener {
    events: Arc<Mutex<Vec<EmulatorEvent>>>,
}

impl EventListener for Listener {
    fn send_event(&self, event: EmulatorEvent) {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(event);
    }
}

/// 滚轮去向：由终端当前模式决定。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WheelRouting {
    /// 应用启用了鼠标上报（1000/1002/1003）：滚轮编码后写入 PTY。
    MouseReport,
    /// 备用屏且启用 alternate scroll（DECSET 1007）：滚轮转成方向键写入 PTY。
    AlternateScroll,
    /// 默认：滚动本地回滚缓冲。
    HostScroll,
}

/// 旁观解析器：只记录本批字节是否出现清屏（ED2），供 `feed` 决定丢弃回滚。
#[derive(Debug, Default)]
struct ClearWatcher {
    erase_display: bool,
}

impl Perform for ClearWatcher {
    fn csi_dispatch(&mut self, params: &Params, intermediates: &[u8], _ignore: bool, action: char) {
        if action == 'J' && intermediates.is_empty() {
            let mode = params
                .iter()
                .next()
                .and_then(|param| param.first())
                .copied()
                .unwrap_or(0);
            if mode == 2 {
                self.erase_display = true;
            }
        }
    }
}

/// 单个窗格的终端仿真状态。
pub struct Terminal {
    term: Term<Listener>,
    parser: Processor,
    clear_parser: Parser,
    clear_watcher: ClearWatcher,
    events: Arc<Mutex<Vec<EmulatorEvent>>>,
    size: GridSize,
    title: Option<String>,
}

impl Terminal {
    /// 创建指定尺寸的仿真终端。
    pub fn new(cols: u16, rows: u16) -> Self {
        let size = GridSize {
            cols: cols.max(1),
            rows: rows.max(1),
        };
        let events = Arc::new(Mutex::new(Vec::new()));
        let listener = Listener {
            events: Arc::clone(&events),
        };
        let term = Term::new(
            EmulatorConfig {
                scrolling_history: SCROLLBACK_LINES,
                ..EmulatorConfig::default()
            },
            &size,
            listener,
        );
        Self {
            term,
            parser: Processor::new(),
            clear_parser: Parser::new(),
            clear_watcher: ClearWatcher::default(),
            events,
            size,
            title: None,
        }
    }

    /// 喂入 PTY 字节；返回需要写回 PTY 的响应（DSR/DA 等）。
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<u8> {
        self.clear_watcher.erase_display = false;
        self.clear_parser.advance(&mut self.clear_watcher, bytes);
        let erase_display = self.clear_watcher.erase_display;

        self.parser.advance(&mut self.term, bytes);

        // alacritty 的 ED2 会把屏上内容推进回滚缓冲（xterm 是原位擦除），使 clear 后
        // 仍能向上翻出旧画面；主屏上按“清屏即清空”丢弃回滚，备用屏不动主屏历史。
        if erase_display && !self.term.mode().contains(TermMode::ALT_SCREEN) {
            self.term.grid_mut().clear_history();
        }

        let mut responses = Vec::new();
        let events = Arc::clone(&self.events);
        let mut events = events.lock().unwrap_or_else(PoisonError::into_inner);
        for event in events.drain(..) {
            match event {
                EmulatorEvent::PtyWrite(text) => responses.extend_from_slice(text.as_bytes()),
                EmulatorEvent::Title(title) => self.title = Some(title),
                EmulatorEvent::ResetTitle => self.title = None,
                _ => {}
            }
        }
        responses
    }

    /// 同步网格尺寸。
    pub fn resize(&mut self, cols: u16, rows: u16) {
        let size = GridSize {
            cols: cols.max(1),
            rows: rows.max(1),
        };
        if size != self.size {
            self.size = size;
            self.term.resize(size);
        }
    }

    /// 当前网格尺寸。
    pub fn size(&self) -> GridSize {
        self.size
    }

    /// 终端模式位（按键编码与粘贴行为依赖它）。
    pub fn mode(&self) -> TermMode {
        *self.term.mode()
    }

    /// 程序通过 OSC 设置的标题。
    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    /// 当前滚轮去向：鼠标上报优先，其次备用屏 alternate scroll，最后本地回滚。
    pub fn wheel_routing(&self) -> WheelRouting {
        let mode = self.mode();
        if mode.intersects(TermMode::MOUSE_MODE) {
            WheelRouting::MouseReport
        } else if mode.contains(TermMode::ALT_SCREEN | TermMode::ALTERNATE_SCROLL) {
            WheelRouting::AlternateScroll
        } else {
            WheelRouting::HostScroll
        }
    }

    /// 滚轮滚动回滚视口；`delta` 为正查看更旧内容（视口上移），返回视口是否移动。
    pub fn scroll_display(&mut self, delta: i32) -> bool {
        if delta == 0 {
            return false;
        }
        let before = self.display_offset();
        self.term.scroll_display(Scroll::Delta(delta));
        self.display_offset() != before
    }

    /// 回滚视口回到底部（最新输出）；返回视口是否移动。
    pub fn scroll_to_bottom(&mut self) -> bool {
        let before = self.display_offset();
        self.term.scroll_display(Scroll::Bottom);
        self.display_offset() != before
    }

    /// 回滚缓冲行数（不含当前视口）。
    pub fn history_size(&self) -> usize {
        self.term.grid().history_size()
    }

    /// 回滚视口距底部的行数（0 表示位于底部）。
    pub fn display_offset(&self) -> usize {
        self.term.grid().display_offset()
    }

    /// 把视口移动到距内容顶部 `offset` 行（0 为最旧一屏）；返回视口是否移动。
    pub fn scroll_to_content_offset(&mut self, offset: usize) -> bool {
        let history = self.history_size();
        let target = history.saturating_sub(offset.min(history));
        self.scroll_display(target as i32 - self.display_offset() as i32)
    }

    /// 渲染视图内容（只读）。
    pub fn renderable_content(&self) -> RenderableContent<'_> {
        self.term.renderable_content()
    }

    /// 网格只读访问，测试使用。
    pub fn grid(&self) -> &Grid<Cell> {
        self.term.grid()
    }

    /// 视口尾部文本行（至多 `max_lines` 行，自底向上截取）；宽字符占位跳过、控制字符空白化。
    ///
    /// 供 Agent 状态仲裁扫描屏幕尾部提示；只读，不改变终端状态。
    pub fn tail_lines(&self, max_lines: usize) -> Vec<String> {
        if max_lines == 0 || self.size.rows == 0 {
            return Vec::new();
        }
        let grid = self.term.grid();
        let rows = usize::from(self.size.rows);
        let start = rows.saturating_sub(max_lines);
        let mut lines = Vec::with_capacity(rows - start);
        for row in start..rows {
            let mut text = String::new();
            for col in 0..usize::from(self.size.cols) {
                let cell = &grid[Line(row as i32)][Column(col)];
                if cell
                    .flags
                    .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
                {
                    continue;
                }
                text.push(if cell.c.is_control() { ' ' } else { cell.c });
            }
            lines.push(text.trim_end().to_string());
        }
        lines
    }

    /// 光标在视口中的位置（行、列）；隐藏或滚出视口时返回 None。
    pub fn cursor_viewport(&self) -> Option<(usize, usize)> {
        let content = self.term.renderable_content();
        if content.cursor.shape == CursorShape::Hidden {
            return None;
        }
        let row = content.cursor.point.line.0 + content.display_offset as i32;
        if row < 0 || row as usize >= usize::from(self.size.rows) {
            return None;
        }
        Some((row as usize, content.cursor.point.column.0))
    }

    /// 开始一次简单选区；入参为视口 0 基行列。
    ///
    /// 选区由仿真器按内容坐标持有：输出滚动时由其随内容旋转，滚回历史不改变锚点。
    pub fn start_selection(&mut self, row: u16, col: u16) {
        let point = self.viewport_point(row, col);
        self.term.selection = Some(TermSelection::new(SelectionType::Simple, point, Side::Left));
    }

    /// 更新选区终点（视口 0 基行列）；无选区时忽略。
    ///
    /// 终点与锚点的先后方向决定端点归属；`include_all` 保证两个端点单元格都入选，
    /// 与反向拖拽时"所见即所得"一致。
    pub fn update_selection(&mut self, row: u16, col: u16) {
        let point = self.viewport_point(row, col);
        if let Some(selection) = self.term.selection.as_mut() {
            selection.update(point, Side::Right);
            selection.include_all();
        }
    }

    /// 提取并清空选区文本；宽字符与换行交给仿真器处理。
    pub fn take_selection_text(&mut self) -> Option<String> {
        let text = self.term.selection_to_string();
        self.term.selection = None;
        text
    }

    /// 清空选区。
    pub fn clear_selection(&mut self) {
        self.term.selection = None;
    }

    /// 视口 0 基行列转换为网格点（计入回滚偏移）。
    fn viewport_point(&self, row: u16, col: u16) -> Point {
        viewport_to_point(
            self.display_offset(),
            Point::new(usize::from(row), Column(usize::from(col))),
        )
    }
}

impl fmt::Debug for Terminal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Terminal")
            .field("size", &self.size)
            .field("title", &self.title)
            .finish()
    }
}

#[cfg(test)]
mod tests;
