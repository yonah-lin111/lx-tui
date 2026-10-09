//! 光标指令去重后端：只在物理状态变化时下发 Hide/Show/MoveTo。
//!
//! ratatui 每帧经 `apply_buffer_with_cursor` 无条件重发 `Show` + `MoveTo`；
//! 高频重绘（如终端持续输出）会不断重置终端光标闪烁相位，macOS Terminal 等
//! 终端上表现为光标高速闪烁。这里按物理状态去重下发：
//! - `Show`/`Hide` 只在可见性变化时下发；
//! - `MoveTo` 只在内容 diff 可能移动了物理光标、或目标位置变化时下发。

use std::io::{self, Write};

use ratatui::backend::{Backend, ClearType, WindowSize};
use ratatui::buffer::Cell;
use ratatui::layout::{Position, Size};

/// 去重包装后端：`inner` 为真实后端，字段是对物理光标状态的认知。
pub(crate) struct QuietCursor<B> {
    inner: B,
    visible: bool,
    /// 物理光标当前位置；`None` 表示位置不可信（有内容写入或清屏），必须重定位。
    caret: Option<Position>,
}

impl<B> QuietCursor<B> {
    /// 备用屏激活时终端光标默认可见，此时位置不可知。
    pub(crate) const fn new(inner: B) -> Self {
        Self {
            inner,
            visible: true,
            caret: None,
        }
    }
}

impl<B: Backend> Backend for QuietCursor<B> {
    type Error = B::Error;

    fn draw<'a, I>(&mut self, content: I) -> Result<(), Self::Error>
    where
        I: Iterator<Item = (u16, u16, &'a Cell)>,
    {
        let mut painted = false;
        let content = content.inspect(|_| painted = true);
        self.inner.draw(content)?;
        if painted {
            self.caret = None;
        }
        Ok(())
    }

    fn hide_cursor(&mut self) -> Result<(), Self::Error> {
        if self.visible {
            self.inner.hide_cursor()?;
            self.visible = false;
        }
        Ok(())
    }

    fn show_cursor(&mut self) -> Result<(), Self::Error> {
        if !self.visible {
            self.inner.show_cursor()?;
            self.visible = true;
        }
        Ok(())
    }

    fn get_cursor_position(&mut self) -> Result<Position, Self::Error> {
        let position = self.inner.get_cursor_position()?;
        self.caret = Some(position);
        Ok(position)
    }

    fn set_cursor_position<P: Into<Position>>(&mut self, position: P) -> Result<(), Self::Error> {
        let position = position.into();
        if self.caret != Some(position) {
            self.inner.set_cursor_position(position)?;
        }
        self.caret = Some(position);
        Ok(())
    }

    fn clear(&mut self) -> Result<(), Self::Error> {
        self.caret = None;
        self.inner.clear()
    }

    fn clear_region(&mut self, clear_type: ClearType) -> Result<(), Self::Error> {
        self.caret = None;
        self.inner.clear_region(clear_type)
    }

    fn size(&self) -> Result<Size, Self::Error> {
        self.inner.size()
    }

    fn window_size(&mut self) -> Result<WindowSize, Self::Error> {
        self.inner.window_size()
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        self.inner.flush()
    }
}

/// 恢复流程直接把转义序列写入终端，需要透传 `Write`。
impl<B: Write> Write for QuietCursor<B> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.inner.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

#[cfg(test)]
mod tests;
