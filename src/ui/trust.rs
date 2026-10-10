//! 启动信任页渲染：LX logo、全宽顶部分隔线与左对齐的键鼠选项；纯渲染，无 IO。
//!
//! 布局对齐 Claude Code：左上对齐、不再居中；首帧由会话层清屏后绘制。
//! 长段落按显示宽度换行，放不下时依次省略说明、空行、安全提示、页脚与 logo。

use std::path::Path;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthStr;

use crate::ui::main_content::lx;
use crate::ui::{style, text};

/// 内容左侧缩进（列）。
const PADDING_X: u16 = 1;
/// 顶部分隔线与内容各占的收尾行数：分隔线 1 行 + 空行 1 行。
const HEADER_ROWS: u16 = 2;

/// 对话框内容行语义。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LineKind {
    /// `Accessing workspace:` 标题行。
    Heading,
    /// 目标路径（accent 高亮）。
    Path,
    /// 安全提示段落；`Quick safety check:` 前缀加粗。
    Safety,
    /// 能力说明。
    Hint,
    Option(usize),
    Footer,
    Blank,
}

/// 安全提示前缀；渲染时加粗强调。
const SAFETY_PREFIX: &str = "Quick safety check:";

/// 一行内容：语义与最终文本（Path 行只存路径值）。
#[derive(Debug)]
struct DialogLine {
    kind: LineKind,
    text: String,
}

/// 信任页几何：logo（可选）、全宽分隔线、内容起行与两个选项的命中矩形。
pub struct TrustLayout {
    pub logo: Option<Rect>,
    /// 全宽顶部分隔线。
    pub rule: Rect,
    /// 内容首行（分隔线下方空行之后）。
    pub content_y: u16,
    /// 选项行命中矩形（全宽）：`[No, exit, Yes, I trust this folder]`。
    pub options: [Rect; 2],
    lines: Vec<DialogLine>,
}

/// 按屏幕与路径计算信任页布局；内容放不下时逐级降级
/// （优先保留标题、路径与选项）：省略说明 → 空行 → 安全提示 → 页脚 → logo。
pub fn layout(screen: Rect, path: &Path) -> TrustLayout {
    let text_width = usize::from(screen.width.saturating_sub(2 * PADDING_X)).max(1);
    let path_text = path.display().to_string();

    // 降级档位：(logo, (说明, 空行, 安全提示, 页脚))。
    const CONFIGS: [(bool, (bool, bool, bool, bool)); 7] = [
        (true, (true, true, true, true)),
        (true, (false, true, true, true)),
        (false, (true, true, true, true)),
        (false, (false, true, true, true)),
        (false, (false, false, true, true)),
        (false, (false, false, false, true)),
        (false, (false, false, false, false)),
    ];
    let mut chosen: Option<(bool, Vec<DialogLine>)> = None;
    for (logo, (hint, blanks, safety, footer)) in CONFIGS {
        let lines = build_lines(&path_text, text_width, hint, blanks, safety, footer);
        let header = header_rows(logo);
        if usize::from(header) + lines.len() <= usize::from(screen.height) {
            chosen = Some((logo, lines));
            break;
        }
    }
    let (logo_wanted, lines) = chosen.unwrap_or_else(|| {
        (
            false,
            build_lines(&path_text, text_width, false, false, false, false),
        )
    });

    let mut y = screen.y;
    let logo = logo_wanted.then(|| {
        let rect = Rect::new(screen.x + PADDING_X, y, lx::LOGO_WIDTH, lx::LOGO_ROWS);
        y = y.saturating_add(lx::LOGO_ROWS + 1);
        rect
    });
    let rule = Rect::new(screen.x, y, screen.width, 1);
    let content_y = y.saturating_add(HEADER_ROWS);
    let mut options = [Rect::default(); 2];
    for (index, line) in lines.iter().enumerate() {
        if let LineKind::Option(option) = line.kind
            && option < options.len()
        {
            options[option] = Rect::new(
                screen.x,
                content_y.saturating_add(index as u16),
                screen.width,
                1,
            );
        }
    }
    TrustLayout {
        logo,
        rule,
        content_y,
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

/// 渲染信任页：分隔线、logo 与左对齐内容；选中项整行高亮（全宽，对齐 Claude Code）。
pub fn render(frame: &mut Frame<'_>, layout: &TrustLayout, selected: usize) {
    let screen = frame.area();
    if let Some(logo) = layout.logo {
        lx::draw_art(frame.buffer_mut(), logo.x, logo.y);
    }
    // 顶部分隔线整行。
    for x in layout.rule.x..layout.rule.right() {
        if let Some(cell) = frame.buffer_mut().cell_mut((x, layout.rule.y)) {
            cell.reset();
            cell.set_symbol(text::DIVIDER_MID);
            cell.set_style(style::border(false));
        }
    }
    let content_x = screen.x + PADDING_X;
    let content_width = screen.width.saturating_sub(2 * PADDING_X);
    for (index, line) in layout.lines.iter().enumerate() {
        let y = layout.content_y.saturating_add(index as u16);
        if y >= screen.bottom() {
            break;
        }
        let rect = Rect::new(content_x, y, content_width, 1);
        match line.kind {
            LineKind::Blank => {}
            LineKind::Heading => {
                frame.render_widget(
                    Paragraph::new(Span::styled(line.text.clone(), style::strong())),
                    rect,
                );
            }
            LineKind::Path => {
                frame.render_widget(
                    Paragraph::new(Span::styled(line.text.clone(), style::accent())),
                    rect,
                );
            }
            LineKind::Safety => {
                let spans = match line.text.strip_prefix(SAFETY_PREFIX) {
                    Some(rest) => Line::from(vec![
                        Span::styled(SAFETY_PREFIX, style::strong()),
                        Span::styled(rest.to_string(), style::text()),
                    ]),
                    None => Line::from(Span::styled(line.text.clone(), style::text())),
                };
                frame.render_widget(Paragraph::new(spans), rect);
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
                // 选中行整行高亮：铺满全宽（对齐 Claude Code 的选择条）。
                let row = Rect::new(screen.x, y, screen.width, 1);
                frame.render_widget(
                    Paragraph::new(Line::from(vec![
                        Span::styled(marker, marker_style),
                        Span::styled(format!(" {}", line.text), row_style),
                    ]))
                    .style(row_style),
                    row,
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

/// 头部行数：logo 时为其高度 + 1 空行，再加分隔线与空行；无 logo 仅分隔线与空行。
fn header_rows(logo: bool) -> u16 {
    if logo {
        lx::LOGO_ROWS + 1 + HEADER_ROWS
    } else {
        HEADER_ROWS
    }
}

/// 构建内容行：标题/路径/安全提示/说明/选项/页脚；四个开关控制降级。
fn build_lines(
    path: &str,
    width: usize,
    hint: bool,
    blanks: bool,
    safety: bool,
    footer: bool,
) -> Vec<DialogLine> {
    let mut lines = Vec::new();
    lines.push(DialogLine {
        kind: LineKind::Heading,
        text: text::TRUST_CONSOLE_HEADING.to_string(),
    });
    push_blank(&mut lines, blanks);
    lines.push(DialogLine {
        kind: LineKind::Path,
        text: text::ellipsize_middle(path, width),
    });
    if safety {
        push_blank(&mut lines, blanks);
        for text in wrap_text(text::TRUST_CONSOLE_SAFETY, width) {
            lines.push(DialogLine {
                kind: LineKind::Safety,
                text,
            });
        }
    }
    if hint {
        push_blank(&mut lines, blanks);
        for text in wrap_text(text::TRUST_CONSOLE_HINT, width) {
            lines.push(DialogLine {
                kind: LineKind::Hint,
                text,
            });
        }
    }
    push_blank(&mut lines, blanks);
    lines.push(DialogLine {
        kind: LineKind::Option(0),
        text: text::TRUST_OPTION_NO.to_string(),
    });
    lines.push(DialogLine {
        kind: LineKind::Option(1),
        text: text::TRUST_OPTION_YES.to_string(),
    });
    if footer && text::TRUST_CONSOLE_FOOTER.width() <= width {
        push_blank(&mut lines, blanks);
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
