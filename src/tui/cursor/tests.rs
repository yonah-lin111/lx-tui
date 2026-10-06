//! 单元测试：用计数后端断言光标指令只在物理状态变化时下发。

use std::io;

use ratatui::backend::{Backend, ClearType, WindowSize};
use ratatui::buffer::Cell;
use ratatui::layout::{Position, Size};
use ratatui::widgets::Paragraph;
use ratatui::{Frame, Terminal};

use super::QuietCursor;

/// 计数后端：只记录收到的光标指令，不接触真实终端。
#[derive(Default)]
struct CountingBackend {
    hide_calls: usize,
    show_calls: usize,
    moves: Vec<Position>,
}

impl Backend for CountingBackend {
    type Error = io::Error;

    fn draw<'a, I>(&mut self, content: I) -> io::Result<()>
    where
        I: Iterator<Item = (u16, u16, &'a Cell)>,
    {
        // 真实后端会消费 diff；这里同样消费，否则上游的 inspect 不会触发。
        for _ in content {}
        Ok(())
    }

    fn hide_cursor(&mut self) -> io::Result<()> {
        self.hide_calls += 1;
        Ok(())
    }

    fn show_cursor(&mut self) -> io::Result<()> {
        self.show_calls += 1;
        Ok(())
    }

    fn get_cursor_position(&mut self) -> io::Result<Position> {
        Ok(self.moves.last().copied().unwrap_or(Position::ORIGIN))
    }

    fn set_cursor_position<P: Into<Position>>(&mut self, position: P) -> io::Result<()> {
        self.moves.push(position.into());
        Ok(())
    }

    fn clear(&mut self) -> io::Result<()> {
        Ok(())
    }

    fn clear_region(&mut self, _clear_type: ClearType) -> io::Result<()> {
        Ok(())
    }

    fn size(&self) -> io::Result<Size> {
        Ok(Size::new(40, 10))
    }

    fn window_size(&mut self) -> io::Result<WindowSize> {
        Ok(WindowSize {
            columns_rows: Size::new(40, 10),
            pixels: Size::new(0, 0),
        })
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn terminal() -> Terminal<QuietCursor<CountingBackend>> {
    Terminal::new(QuietCursor::new(CountingBackend::default())).expect("terminal")
}

/// 角色：把光标定在 (3, 1) 并写入给定文本。
fn draw_with(terminal: &mut Terminal<QuietCursor<CountingBackend>>, text: &str) {
    terminal
        .draw(|frame: &mut Frame| {
            frame.set_cursor_position(Position::new(3, 1));
            frame.render_widget(Paragraph::new(text), frame.area());
        })
        .expect("draw");
}

#[test]
fn unchanged_frame_emits_no_cursor_commands() {
    let mut terminal = terminal();
    draw_with(&mut terminal, "abc");
    assert_eq!(terminal.backend().inner.moves, vec![Position::new(3, 1)]);
    assert_eq!(terminal.backend().inner.show_calls, 0, "初始可见无需 Show");

    draw_with(&mut terminal, "abc");
    assert_eq!(
        terminal.backend().inner.moves.len(),
        1,
        "帧内容与光标位置都未变化时不再下发 MoveTo"
    );
    assert_eq!(terminal.backend().inner.show_calls, 0, "不重复下发 Show");
}

#[test]
fn painted_frame_repositions_the_caret() {
    let mut terminal = terminal();
    draw_with(&mut terminal, "abc");
    draw_with(&mut terminal, "abd");
    assert_eq!(
        terminal.backend().inner.moves.len(),
        2,
        "内容 diff 会移动物理光标，必须重定位"
    );
    assert_eq!(terminal.backend().inner.show_calls, 0);
}

#[test]
fn cursor_visibility_commands_are_deduplicated() {
    let mut terminal = terminal();
    let hidden = |_frame: &mut Frame| {};
    terminal.draw(hidden).expect("draw");
    assert_eq!(terminal.backend().inner.hide_calls, 1);

    terminal.draw(hidden).expect("draw");
    assert_eq!(
        terminal.backend().inner.hide_calls,
        1,
        "已隐藏时不重复下发 Hide"
    );

    draw_with(&mut terminal, "");
    assert_eq!(
        terminal.backend().inner.show_calls,
        1,
        "从隐藏恢复需重新 Show"
    );
}
