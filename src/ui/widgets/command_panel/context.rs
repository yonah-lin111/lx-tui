//! 无边框命令面板的只读上下文摘要行（位于标题行之下、可选项之上）。

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;
use unicode_width::UnicodeWidthStr;

use crate::ui::{style, text};

/// 面板顶部的单行只读上下文（标签 + 值；`emphasized` 时值用强调色加粗以突出来源）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PanelContextLine<'a> {
    pub label: &'a str,
    pub value: &'a str,
    pub emphasized: bool,
}

impl<'a> PanelContextLine<'a> {
    /// 普通只读上下文行。
    pub const fn new(label: &'a str, value: &'a str) -> Self {
        Self {
            label,
            value,
            emphasized: false,
        }
    }

    /// 突出来源或关键警示的上下文行（值采用强调色加粗）。
    pub const fn emphasized(label: &'a str, value: &'a str) -> Self {
        Self {
            label,
            value,
            emphasized: true,
        }
    }

    /// 单行显示宽度（含左缩进与标签间距）。
    pub(super) fn display_width(&self) -> usize {
        2 + self.label.width() + usize::from(!self.label.is_empty()) + self.value.width()
    }
}

/// 在无边框面板顶栏之后绘制上下文摘要行。
pub(super) fn render_lines(
    buf: &mut Buffer,
    rect: Rect,
    top_offset: u16,
    lines: &[PanelContextLine<'_>],
) {
    if lines.is_empty() || rect.width <= 3 {
        return;
    }
    let x = rect.x.saturating_add(2);
    let max_w = usize::from(rect.right().saturating_sub(x));
    if max_w == 0 {
        return;
    }
    for (idx, item) in lines.iter().enumerate() {
        let y = rect.y.saturating_add(top_offset).saturating_add(idx as u16);
        if y >= rect.bottom() {
            break;
        }
        let prefix = if item.label.is_empty() {
            String::new()
        } else {
            format!("{} ", item.label)
        };
        let prefix_w = prefix.width();
        let line = if prefix_w >= max_w {
            Line::from(Span::styled(
                text::truncate_to_width(&prefix, max_w),
                style::muted(),
            ))
        } else {
            let value = text::truncate_to_width(item.value, max_w - prefix_w);
            let value_style = if item.emphasized {
                style::accent()
            } else {
                style::text()
            };
            Line::from(vec![
                Span::styled(prefix, style::muted()),
                Span::styled(value, value_style),
            ])
        };
        line.render(Rect::new(x, y, max_w as u16, 1), buf);
    }
}
