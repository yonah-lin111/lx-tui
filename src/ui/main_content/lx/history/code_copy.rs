//! 代码块头部复制按钮的几何收集：命中矩形 + 该块正文。
//!
//! 阅读渲染（[`crate::ui::markdown_render`]）是纯函数、不认识屏幕坐标，因此按钮的
//! 「在屏幕第几列」由这里按**同一条渲染管线**（`render_wrapped` + 步骤缩进）重算：
//! 行序与 `history::render` 完全一致，命中矩形与实际绘制不会漂移。
//!
//! 每个代码块头部的 [`MdLine::code_copy`] 给出按钮的行内起始列，紧随其后的
//! 以竖条起头的行就是该块正文（直到遇到非竖条行为止）。复制内容**不含**围栏与
//! `language:` 头部行，但**保留代码自身的缩进**——缩进是代码的一部分。

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

/// 代码块竖条（与 `markdown_render` 同一个直角风格符号）。
const BAR_MARKER: &str = "│";

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
    let mut block = 0usize;
    for (offset, line) in markdown_render::render_wrapped(content, inner)
        .into_iter()
        .enumerate()
    {
        let text = line.text();
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
            continue;
        }
        let Some(current) = body.as_mut() else {
            continue;
        };
        // 以竖条起头的行属于代码块正文；其余行表示代码块结束。
        let Some(rest) = text.strip_prefix(BAR_MARKER) else {
            close(&mut body, out);
            continue;
        };
        if !current.is_empty() {
            current.push('\n');
        }
        // 只去掉竖条与其后那 1 个空格——**不 trim_start**：代码块里的缩进
        // 是代码本身的一部分，复制时必须原样保留。
        current.push_str(rest.strip_prefix(' ').unwrap_or(rest));
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
