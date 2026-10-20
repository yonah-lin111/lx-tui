//! 代码块头部复制按钮的几何收集：命中矩形 + 该块正文。
//!
//! 阅读渲染（[`crate::ui::markdown_render`]）是纯函数、不认识屏幕坐标，因此按钮的
//! 「在屏幕第几列」由这里按**同一条渲染管线**（`render_wrapped` + 步骤缩进）重算：
//! 行序与 `history::render` 完全一致，命中矩形与实际绘制不会漂移。
//!
//! 每个代码块头部的 [`MdLine::code_copy`] 给出按钮的行内起始列，紧随其后的一条
//! 分隔横线（[`MdLine::code_rule`]）要被跳过，再往后以 `code_body` 标记的行就是该块
//! 正文（直到遇到非正文行为止）。复制内容**不含**围栏、`language:` 头部行与全部横线，
//! 但**保留代码自身的缩进**——缩进是代码的一部分。

use ratatui::layout::Rect;

use crate::ui::markdown_render;

use super::{CODE_COPY_WIDTH, STEP_GUTTER};

/// 代码块头部的复制按钮：命中矩形 + 该块正文（不含围栏与语言行）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeCopyButton {
    pub message: usize,
    pub step: usize,
    /// 该步骤内的代码块序号（从 0 起）；悬停高亮只作用于被悬停的那一块。
    pub block: usize,
    pub rect: Rect,
    /// 代码块正文（`\n` 连接，末尾不带换行；缩进原样保留）。
    pub body: String,
}

/// 收集一个正文步骤里所有代码块的复制按钮矩形（按渲染行序换算屏幕坐标）。
///
/// `inner` 是该步骤正文的可用宽度（已扣步骤缩进），`start_row` 是它在内容行空间
/// 里的起始行号，`area` 是历史内容区（换算屏幕坐标用）。
pub(super) fn collect(
    message: usize,
    step: usize,
    content: &str,
    inner: u16,
    area: Rect,
    start_row: usize,
    out: &mut Vec<CodeCopyButton>,
) {
    if inner == 0 {
        return;
    }
    // 当前正在收集的代码块正文（Some 表示上一行是代码块头部）。
    let mut body: Option<String> = None;
    // 本块已收集的正文行数（用行数而不是 `body.is_empty()` 判首行——代码块开头的
    // 空行也是正文，用 `is_empty()` 会把它们吞掉）。
    let mut body_lines = 0usize;
    let mut block = 0usize;
    for (offset, line) in markdown_render::render_wrapped(content, inner)
        .into_iter()
        .enumerate()
    {
        // 头部行：记录按钮矩形，并开始收集紧随其后的正文行。
        if let Some(column) = line.code_copy {
            close(&mut body, out);
            let y = area
                .y
                .saturating_add(u16::try_from(start_row + offset).unwrap_or(u16::MAX));
            let x = area
                .x
                .saturating_add(STEP_GUTTER)
                .saturating_add(line.indent)
                .saturating_add(column);
            out.push(CodeCopyButton {
                message,
                step,
                block,
                rect: Rect::new(x, y, CODE_COPY_WIDTH, 1),
                body: String::new(),
            });
            block += 1;
            body = Some(String::new());
            body_lines = 0;
            continue;
        }
        let Some(current) = body.as_mut() else {
            continue;
        };
        // 分隔横线不参与复制、也不算正文结束：头部下方新增的那条线正好落在
        // 「头部之后、正文之前」，若按「第一个非正文行」关闭收集，复制内容会变成空串。
        if line.code_rule {
            continue;
        }
        // 只有代码块正文行属于这一块（逐行直到非正文行为止；收尾横线同样不带
        // `code_body` 标记，因此在这里结束收集）。
        if !line.code_body {
            close(&mut body, out);
            continue;
        }
        if body_lines > 0 {
            current.push('\n');
        }
        body_lines += 1;
        // **不 trim**：代码块里的缩进是代码本身的一部分，复制时必须原样保留。
        current.push_str(&line.text());
    }
    close(&mut body, out);
}

/// 把收集到的正文回填到刚推入的按钮（代码块结束或输入结束）。
fn close(body: &mut Option<String>, out: &mut [CodeCopyButton]) {
    if let Some(value) = body.take()
        && let Some(last) = out.last_mut()
    {
        last.body = value;
    }
}
