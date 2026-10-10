//! 启动信任页渲染：LX logo、边框对话框、高亮提示与键鼠可选项；纯渲染，无 IO。
//!
//! 布局与命中共用 `layout`：对话框内容按宽度换行，屏幕高度不足时依次省略说明、
//! 空行与页脚，logo 仅在放得下时显示。

use std::path::Path;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph};
use unicode_width::UnicodeWidthStr;

use crate::ui::main_content::lx;
use crate::ui::{style, text};

/// 对话框目标宽度（列，含边框；屏幕更窄时夹取）。
const DIALOG_WIDTH: u16 = 64;
/// 对话框左右内边距合计（列）。
const DIALOG_PADDING: u16 = 2;
/// logo 与对话框之间的空行。
const LOGO_GAP: u16 = 1;

/// 对话框内容行语义。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LineKind {
    Question,
    Path,
    Hint,
    Option(usize),
    Footer,
    Blank,
}

/// 一行内容：语义与最终文本（Path 行只存路径值，渲染时补前缀）。
#[derive(Debug)]
struct DialogLine {
    kind: LineKind,
    text: String,
}

/// 信任页几何：logo 区域（可选）、对话框、内容行与两个选项的命中矩形。
pub struct TrustLayout {
    pub logo: Option<Rect>,
    pub dialog: Rect,
    /// 选项行命中矩形：`[Yes, No]`。
    pub options: [Rect; 2],
    lines: Vec<DialogLine>,
}

/// 按屏幕与路径计算信任页布局；内容放不下时逐级降级。
pub fn layout(screen: Rect, path: &Path) -> TrustLayout {
    let width = DIALOG_WIDTH.min(screen.width).max(4);
    let text_width = usize::from(width.saturating_sub(2 + DIALOG_PADDING)).max(1);
    let path_text = path.display().to_string();

    let mut lines = build_lines(&path_text, text_width, true, true);
    let mut height = lines.len() as u16 + 2;
    if height > screen.height {
        lines = build_lines(&path_text, text_width, false, true);
        height = lines.len() as u16 + 2;
    }
    if height > screen.height {
        lines = build_lines(&path_text, text_width, false, false);
        height = lines.len() as u16 + 2;
    }
    let height = height.min(screen.height);

    let dialog_x = screen.x + screen.width.saturating_sub(width) / 2;
    let dialog_y = screen.y + screen.height.saturating_sub(height) / 2;
    let logo_fits =
        screen.width >= lx::LOGO_WIDTH && screen.height >= height + LOGO_GAP + lx::LOGO_ROWS;
    let (logo, dialog_y) = if logo_fits {
        let total = height + LOGO_GAP + lx::LOGO_ROWS;
        let top = screen.y + screen.height.saturating_sub(total) / 2;
        let logo = Rect::new(
            screen.x + screen.width.saturating_sub(lx::LOGO_WIDTH) / 2,
            top,
            lx::LOGO_WIDTH,
            lx::LOGO_ROWS,
        );
        (Some(logo), top + lx::LOGO_ROWS + LOGO_GAP)
    } else {
        (None, dialog_y)
    };
    let dialog = Rect::new(dialog_x, dialog_y, width, height);
    let inner = Block::bordered().inner(dialog);
    let mut options = [Rect::default(); 2];
    for (index, line) in lines.iter().enumerate() {
        if let LineKind::Option(option) = line.kind
            && option < options.len()
        {
            options[option] = Rect::new(
                inner.x + 1,
                inner.y.saturating_add(index as u16),
                inner.width.saturating_sub(2),
                1,
            );
        }
    }
    TrustLayout {
        logo,
        dialog,
        options,
        lines,
    }
}

/// 命中选项行；未命中返回 None。
pub fn option_at(layout: &TrustLayout, column: u16, row: u16) -> Option<usize> {
    layout
        .options
        .iter()
        .position(|rect| rect.contains((column, row).into()))
}

/// 渲染信任页：清屏、logo、边框对话框与内容行；选中项整行高亮。
pub fn render(frame: &mut Frame<'_>, layout: &TrustLayout, selected: usize) {
    frame.render_widget(Clear, frame.area());
    if let Some(logo) = layout.logo {
        lx::draw_art(frame.buffer_mut(), logo.x, logo.y);
    }
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(style::accent())
        .style(style::overlay_panel())
        .title(Span::styled(
            format!(" {} ", text::TRUST_CONSOLE_TITLE),
            style::border_title(),
        ));
    let inner = block.inner(layout.dialog);
    frame.render_widget(block, layout.dialog);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let row_width = inner.width.saturating_sub(2);
    for (index, line) in layout.lines.iter().enumerate() {
        let y = inner.y.saturating_add(index as u16);
        if y >= inner.bottom() {
            break;
        }
        let rect = Rect::new(inner.x + 1, y, row_width, 1);
        match line.kind {
            LineKind::Blank => {}
            LineKind::Question => {
                frame.render_widget(
                    Paragraph::new(Span::styled(line.text.clone(), style::strong())),
                    rect,
                );
            }
            LineKind::Path => {
                frame.render_widget(
                    Paragraph::new(Line::from(vec![
                        Span::styled(
                            format!("{}: ", text::TRUST_CONSOLE_PATH_PREFIX),
                            style::muted(),
                        ),
                        Span::styled(line.text.clone(), style::accent()),
                    ])),
                    rect,
                );
            }
            LineKind::Hint => {
                frame.render_widget(
                    Paragraph::new(Span::styled(line.text.clone(), style::muted())),
                    rect,
                );
            }
            LineKind::Option(option) => {
                let highlighted = option == selected;
                let (marker, marker_style, row_style) = if highlighted {
                    (
                        text::TRUST_SELECTION_MARKER,
                        style::accent().patch(style::overlay_selection()),
                        style::overlay_selection(),
                    )
                } else {
                    (" ", style::text(), style::text())
                };
                frame.render_widget(
                    Paragraph::new(Line::from(vec![
                        Span::styled(marker, marker_style),
                        Span::styled(format!(" {}", line.text), row_style),
                    ]))
                    .style(row_style),
                    rect,
                );
            }
            LineKind::Footer => {
                frame.render_widget(
                    Paragraph::new(Span::styled(line.text.clone(), style::muted())),
                    rect,
                );
            }
        }
    }
}

/// 构建内容行：问题与说明按宽度换行；`hint`/`blanks` 为降级开关。
fn build_lines(path: &str, width: usize, hint: bool, blanks: bool) -> Vec<DialogLine> {
    let mut lines = Vec::new();
    for text in wrap_text(text::TRUST_WORKSPACE_QUESTION, width) {
        lines.push(DialogLine {
            kind: LineKind::Question,
            text,
        });
    }
    push_blank(&mut lines, blanks);
    let prefix_width = text::TRUST_CONSOLE_PATH_PREFIX.width() + 2;
    lines.push(DialogLine {
        kind: LineKind::Path,
        text: text::ellipsize_middle(path, width.saturating_sub(prefix_width)),
    });
    push_blank(&mut lines, blanks);
    if hint {
        for text in wrap_text(text::TRUST_CONSOLE_HINT, width) {
            lines.push(DialogLine {
                kind: LineKind::Hint,
                text,
            });
        }
        push_blank(&mut lines, blanks);
    }
    lines.push(DialogLine {
        kind: LineKind::Option(0),
        text: text::TRUST_OPTION_YES.to_string(),
    });
    lines.push(DialogLine {
        kind: LineKind::Option(1),
        text: text::TRUST_OPTION_NO.to_string(),
    });
    push_blank(&mut lines, blanks);
    if text::TRUST_CONSOLE_FOOTER.width() <= width {
        lines.push(DialogLine {
            kind: LineKind::Footer,
            text: text::TRUST_CONSOLE_FOOTER.to_string(),
        });
    }
    lines
}

/// 按需插入空行。
fn push_blank(lines: &mut Vec<DialogLine>, blanks: bool) {
    if blanks {
        lines.push(DialogLine {
            kind: LineKind::Blank,
            text: String::new(),
        });
    }
}

/// 按显示宽度做空白分词换行；单个超宽词保持原样（文案均为短词）。
fn wrap_text(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        let candidate = if current.is_empty() {
            word.width()
        } else {
            current.width() + 1 + word.width()
        };
        if candidate > width && !current.is_empty() {
            lines.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(word);
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

#[cfg(test)]
mod tests;
