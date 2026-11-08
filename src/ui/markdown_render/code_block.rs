//! 面向阅读的 Markdown 代码块边框装饰、头部复制按钮布局与源码提取。
//!
//! - 顶部独立一行铺满下 1/4 块 `▂`（[`text::CODE_BLOCK_TOP_BORDER`]），样式 [`style::md_read_rule`]；
//! - 下一行左侧由 `▎` 边栏闭合，右侧显示右对齐的 `[❐]` 复制按钮（无边框，语言左侧与按钮右侧各留 1 列空白）；
//! - 代码正文每个视觉行（含软换行续行）首列绘制 `▎`（[`text::CODE_BLOCK_BAR`]）；制表符按 4 列展开、控制字符替换为空格。

use std::borrow::Cow;

use ratatui::style::Style;
use unicode_width::UnicodeWidthChar;

use crate::ui::{style, text};

use super::{MdLine, display_width, highlight, is_indented_code, list_item};

/// 无语言标记或 4 空格缩进代码块的默认语言标签。
pub const DEFAULT_CODE_LANG: &str = "text";

/// 代码块单条渲染行的角色（仅在助手正文启用代码块边框装饰时附着于 [`MdLine`]）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodeLineKind {
    /// 顶部边框行（整行铺满 `▂`）。
    TopBorder,
    /// 顶部边框下一行的头部：左侧 `▎` 与顶边闭合，右侧 `[❐]` 复制按钮，无边框。
    Header {
        /// 当前 Markdown 文本块内的代码块序号（0-based，与 [`extract_code_block`] 一致）。
        block_index: usize,
        /// 复制按钮在代码块行内的列区间 `[start_col, end_col)`（相对 `MdLine` 起点）。
        button_cols: (u16, u16),
    },
    /// 代码块正文行（含软换行续行），首列固定为 `▎` 左栏。
    Body,
}

/// 构造代码块顶部边框行（铺满整行 `▂`）。
pub(super) fn top_border_line(width: Option<usize>) -> MdLine {
    let cols = width.unwrap_or(40).max(1);
    MdLine {
        indent: 0,
        hang: 0,
        spans: vec![(
            text::CODE_BLOCK_TOP_BORDER.repeat(cols),
            style::md_read_rule(),
        )],
        table: None,
        code: Some(CodeLineKind::TopBorder),
    }
}

/// 构造代码块头部：`▎ <lang> ... [❐] `（无边框；按钮右侧留 1 列空白）。
///
/// 左栏 `▎` 与顶边闭合；语言名左侧固定 1 列空白，语言与按钮之间至少 1 列空白，
/// 按钮右侧固定 1 列空白；语言名超出可用宽度时以 `…` 截断。
pub(super) fn header_line(lang: &str, block_index: usize, width: Option<usize>) -> MdLine {
    let cols = width.unwrap_or(40).max(1);
    let label = normalize_lang_label(lang);
    let btn = text::TEMPLATE_BUTTON_COPY;
    let btn_w = display_width(btn);
    let bar_w = 1;
    let left_pad = 1;
    let right_pad = 1;
    let rule = style::md_read_rule();
    // 固定开销：左栏 + 语言左空白 + 语言与按钮间至少 1 列 + 按钮 + 按钮右空白。
    let fixed = bar_w + left_pad + 1 + btn_w + right_pad;

    // 极窄兜底：左栏 + 1 列空白 + 可点击的按钮片段。
    if cols < fixed {
        let btn_display = clip_width(btn, cols.saturating_sub(bar_w + left_pad));
        let btn_w = display_width(&btn_display);
        let btn_start = bar_w + left_pad;
        let mut spans = vec![(text::CODE_BLOCK_BAR.to_string(), rule)];
        if cols > bar_w {
            spans.push((" ".to_string(), style::text()));
        }
        spans.push((btn_display, style::template_button()));
        return MdLine {
            indent: 0,
            hang: 0,
            spans,
            table: None,
            code: Some(CodeLineKind::Header {
                block_index,
                button_cols: (
                    u16::try_from(btn_start).unwrap_or(u16::MAX),
                    u16::try_from(btn_start + btn_w).unwrap_or(u16::MAX),
                ),
            }),
        };
    }

    let lang_budget = cols.saturating_sub(fixed);
    let lang_display = truncate_with_ellipsis(label, lang_budget);
    let lang_w = display_width(&lang_display);
    let fill = cols.saturating_sub(bar_w + left_pad + lang_w + btn_w + right_pad);
    let btn_start = bar_w + left_pad + lang_w + fill;
    let btn_end = btn_start + btn_w;

    let mut spans = Vec::with_capacity(6);
    spans.push((text::CODE_BLOCK_BAR.to_string(), rule));
    spans.push((" ".to_string(), style::text()));
    if !lang_display.is_empty() {
        spans.push((lang_display, style::muted()));
    }
    if fill > 0 {
        spans.push((" ".repeat(fill), style::text()));
    }
    spans.push((btn.to_string(), style::template_button()));
    spans.push((" ".to_string(), style::text()));

    let button_cols = (
        u16::try_from(btn_start).unwrap_or(u16::MAX),
        u16::try_from(btn_end).unwrap_or(u16::MAX),
    );
    MdLine {
        indent: 0,
        hang: 0,
        spans,
        table: None,
        code: Some(CodeLineKind::Header {
            block_index,
            button_cols,
        }),
    }
}

/// 渲染代码块中的单行代码正文（`code_chrome = true` 时标记为 [`CodeLineKind::Body`]，
/// 由 `wrap_md_line` 折行时在每个视觉行首列统一前置 `▎`）。
pub(super) fn render_code_line(
    line: &str,
    lang: Option<&str>,
    code_chrome: bool,
    lines: &mut Vec<MdLine>,
) {
    // 代码里常见制表符与 CRLF：先展开/净化，再交给高亮器。
    let sanitized = sanitize_code_line(line);
    let spans = highlight::highlight_line(lang, &sanitized);
    lines.push(MdLine {
        indent: 0,
        hang: 0,
        spans,
        table: None,
        code: code_chrome.then_some(CodeLineKind::Body),
    });
}

/// 代码块制表符宽度（按列对齐）。
const TAB_WIDTH: usize = 4;

/// 把代码行中不能直接落格的字符转换为可见文本：`\t` 按 4 列制表位展开，
/// 其余控制字符（`\r`、`\x1b` 等）替换为空格，避免 ratatui 的 cell_width panic。
fn sanitize_code_line(line: &str) -> Cow<'_, str> {
    if !line.chars().any(|ch| ch == '\t' || ch.is_control()) {
        return Cow::Borrowed(line);
    }
    let mut out = String::with_capacity(line.len().saturating_add(TAB_WIDTH));
    let mut col = 0usize;
    for ch in line.chars() {
        match ch {
            '\t' => {
                let pad = TAB_WIDTH - col % TAB_WIDTH;
                for _ in 0..pad {
                    out.push(' ');
                }
                col = col.saturating_add(pad);
            }
            ch if ch.is_control() => {
                out.push(' ');
                col = col.saturating_add(1);
            }
            ch => {
                out.push(ch);
                col = col.saturating_add(ch.width().unwrap_or(0));
            }
        }
    }
    Cow::Owned(out)
}

/// 为折行后的单个代码块正文视觉行前置左侧栏 `▎`。
pub(super) fn prepend_body_bar(
    mut spans: Vec<(String, Style)>,
    width: usize,
) -> Vec<(String, Style)> {
    if width == 0 {
        return spans;
    }
    let mut out = Vec::with_capacity(spans.len() + 1);
    out.push((text::CODE_BLOCK_BAR.to_string(), style::md_read_rule()));
    if width > 1 {
        out.append(&mut spans);
    }
    out
}

/// 规范化语言名：空白或未指定时回退为 `"text"`。
pub(super) fn normalize_lang_label(lang: &str) -> &str {
    let trimmed = lang.trim();
    if trimmed.is_empty() {
        DEFAULT_CODE_LANG
    } else {
        trimmed
    }
}

fn truncate_with_ellipsis(text: &str, max_width: usize) -> String {
    if max_width == 0 {
        return String::new();
    }
    if display_width(text) <= max_width {
        return text.to_string();
    }
    if max_width == 1 {
        return "…".to_string();
    }
    let budget = max_width - 1;
    let mut out = String::new();
    let mut used = 0;
    for ch in text.chars() {
        let w = ch.width().unwrap_or(0).min(2);
        if used + w > budget {
            break;
        }
        out.push(ch);
        used += w;
    }
    out.push('…');
    out
}

fn clip_width(text: &str, max_width: usize) -> String {
    let mut out = String::new();
    let mut used = 0;
    for ch in text.chars() {
        let w = ch.width().unwrap_or(0).min(2);
        if used + w > max_width {
            break;
        }
        out.push(ch);
        used += w;
    }
    out
}

/// 按与 [`super::render_blocks_stateful`] 完全一致的块级扫描口径，提取第 `target_index` 个
/// 代码块（围栏或 4 空格缩进代码块）的原始未折行代码正文。
///
/// 未闭合的流式围栏代码块返回当前已生成的全部代码行。
pub fn extract_code_block(source: &str, target_index: usize) -> Option<String> {
    let source_lines: Vec<&str> = source.split('\n').collect();
    let mut index = 0;
    let mut open_fence = false;
    let mut current_block = 0usize;
    let mut fence_lines: Vec<&str> = Vec::new();

    while index < source_lines.len() {
        let line = source_lines[index];
        if open_fence {
            if line.trim_start().starts_with("```") {
                if current_block == target_index {
                    return Some(fence_lines.join("\n"));
                }
                fence_lines.clear();
                open_fence = false;
                current_block += 1;
                index += 1;
                continue;
            }
            if current_block == target_index {
                fence_lines.push(line);
            }
            index += 1;
            continue;
        }
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            index += 1;
            continue;
        }
        if trimmed.trim_start().starts_with("```") {
            open_fence = true;
            fence_lines.clear();
            index += 1;
            continue;
        }
        if list_item(line).is_some() {
            index += 1;
            continue;
        }
        if is_indented_code(line) {
            let mut body = Vec::new();
            while index < source_lines.len()
                && (is_indented_code(source_lines[index]) || source_lines[index].trim().is_empty())
            {
                if source_lines[index].trim().is_empty() {
                    if index + 1 < source_lines.len() && is_indented_code(source_lines[index + 1]) {
                        body.push("");
                        index += 1;
                        continue;
                    }
                    break;
                }
                body.push(
                    source_lines[index]
                        .strip_prefix("    ")
                        .unwrap_or(source_lines[index]),
                );
                index += 1;
            }
            if current_block == target_index {
                return Some(body.join("\n"));
            }
            current_block += 1;
            continue;
        }
        if trimmed.trim_start().starts_with('|') {
            while index < source_lines.len() && source_lines[index].trim_start().starts_with('|') {
                index += 1;
            }
            continue;
        }
        if trimmed.trim_start().starts_with('>') {
            while index < source_lines.len() && source_lines[index].trim_start().starts_with('>') {
                index += 1;
            }
            continue;
        }
        index += 1;
    }

    if open_fence && current_block == target_index {
        return Some(fence_lines.join("\n"));
    }
    None
}

#[cfg(test)]
mod tests;
