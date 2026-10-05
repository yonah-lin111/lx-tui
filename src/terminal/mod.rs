//! 终端仿真：alacritty_terminal 的纯内存封装，字节进、画面出，不触碰 IO。

use std::fmt;
use std::sync::{Arc, Mutex, PoisonError};

use alacritty_terminal::event::{Event as EmulatorEvent, EventListener};
use alacritty_terminal::grid::{Dimensions, Grid};
use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection as TermSelection, SelectionType};
use alacritty_terminal::term::cell::Cell;
use alacritty_terminal::term::{Config as EmulatorConfig, RenderableContent, Term, TermMode};
use alacritty_terminal::vte::ansi::{CursorShape, Processor};

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

/// 单个窗格的终端仿真状态。
pub struct Terminal {
    term: Term<Listener>,
    parser: Processor,
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
            events,
            size,
            title: None,
        }
    }

    /// 喂入 PTY 字节；返回需要写回 PTY 的响应（DSR/DA 等）。
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<u8> {
        self.parser.advance(&mut self.term, bytes);

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

    /// 渲染视图内容（只读）。
    pub fn renderable_content(&self) -> RenderableContent<'_> {
        self.term.renderable_content()
    }

    /// 网格只读访问，测试使用。
    pub fn grid(&self) -> &Grid<Cell> {
        self.term.grid()
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

    /// 提取视口范围内（含端点）的文本；宽字符与换行交给仿真器处理。
    pub fn text_in_range(&mut self, start: (u16, u16), end: (u16, u16)) -> Option<String> {
        let (first, last) = if start <= end {
            (start, end)
        } else {
            (end, start)
        };
        let point =
            |(row, col): (u16, u16)| Point::new(Line(i32::from(row)), Column(usize::from(col)));
        let mut selection = TermSelection::new(SelectionType::Simple, point(first), Side::Left);
        selection.update(point(last), Side::Right);
        self.term.selection = Some(selection);
        let text = self.term.selection_to_string();
        self.term.selection = None;
        text
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
