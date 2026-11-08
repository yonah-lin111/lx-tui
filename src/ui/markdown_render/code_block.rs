//! 面向阅读的 Markdown 代码块边框装饰、头部复制按钮布局与源码提取。
//!
//! - 顶部独立一行铺满下 1/4 块 `▂`（[`text::CODE_BLOCK_TOP_BORDER`]），样式 [`style::md_read_rule`]；
//! - 下一行头部是一条规则线：`─ <lang> ───── <复制按钮> ─`（语言缺省 `text`，按钮右对齐）；
//! - 代码正文每个视觉行（含软换行续行）首列绘制左 1/4 块 `▎`（[`text::CODE_BLOCK_BAR`]）。

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
    /// 顶部边框下一行的头部：`─ <lang> ───── [❐] ─` 规则线，语言名在左、复制按钮在右。
    Header {
        /// 当前 Markdown 文本块内的代码块序号（0-based，与 [`extract_code_block`] 一致）。
        block_index: usize,
        /// 复制按钮在代码块行内的列区间 `[start_col, end_col)`（相对 `MdLine` 起点）。
        button_cols: (u16, u16),
    },
    /// 代码块正文行（含软换行续行），首列固定为 `▎` 左栏。
    Body,
}

/// 构造代码块顶部边框行（整行铺满 `▂`）。
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

/// 构造代码块头部：`─ <lang> ───── [❐] ─`。
///
/// 两端固定为 `─ ` 与 ` ─`，语言名与按钮之间的空余宽度用 `─` 铺满；
/// 极窄时先压缩语言名（超出以 `…` 截断），再压缩规则填充。
pub(super) fn header_line(lang: &str, block_index: usize, width: Option<usize>) -> MdLine {
    let cols = width.unwrap_or(40).max(1);
    let label = normalize_lang_label(lang);
    let btn = text::TEMPLATE_BUTTON_COPY;
    let btn_w = display_width(btn);
    // 固定开销：`─ ` + 语言后空格 + 按钮前空格 + 按钮 + ` ─`。
    let fixed = 2 + 1 + 1 + btn_w + 2;
    let rule = style::md_read_rule();

    if cols < fixed {
        // 极窄兜底：宽度不足以容纳规则线时只保留可点击的按钮片段。
        let btn_display = clip_width(btn, cols);
        let btn_end = u16::try_from(display_width(&btn_display)).unwrap_or(u16::MAX);
        return MdLine {
            indent: 0,
            hang: 0,
            spans: vec![(btn_display, style::template_button())],
            table: None,
            code: Some(CodeLineKind::Header {
                block_index,
                button_cols: (0, btn_end),
            }),
        };
    }

    let inner = cols.saturating_sub(fixed);
    let lang_display = truncate_with_ellipsis(label, inner);
    let lang_w = display_width(&lang_display);
    let fill = inner.saturating_sub(lang_w);
    let btn_start = 2 + lang_w + 1 + fill + 1;
    let btn_end = btn_start + btn_w;

    let mut spans = Vec::with_capacity(7);
    spans.push(("─ ".to_string(), rule));
    if !lang_display.is_empty() {
        spans.push((lang_display, style::muted()));
    }
    spans.push((" ".to_string(), style::text()));
    if fill > 0 {
        spans.push(("─".repeat(fill), rule));
    }
    spans.push((" ".to_string(), style::text()));
    spans.push((btn.to_string(), style::template_button()));
    spans.push((" ─".to_string(), rule));

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
    let spans = highlight::highlight_line(lang, line);
    lines.push(MdLine {
        indent: 0,
        hang: 0,
        spans,
        table: None,
        code: code_chrome.then_some(CodeLineKind::Body),
    });
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
