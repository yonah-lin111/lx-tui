//! 物理行文字列区间：命中几何的唯一计算口径。
//!
//! 只做“显示文本 → 可见列区间”的纯计算，不依赖 [`Grid`](super::super::Grid)、不渲染。

use unicode_width::UnicodeWidthChar;

use crate::ui::markdown_render::MdLine;

use super::tool::Segment;

/// 从 `start_col` 起扫描若干段文本；返回首个非空白字符到末个非空白字符的列区间。
///
/// - 整行只有树线/空白 → `None`（不可点）；
/// - 行内只要有一个正文/标志字符，区间就从第一个非空白字符算起（含 `├─`/`│`）；
/// - 行尾空白不进入区间。
pub(super) fn cols_from<'a>(
    start_col: u16,
    parts: impl IntoIterator<Item = &'a str>,
) -> Option<(u16, u16)> {
    let mut col = u32::from(start_col);
    let mut first: Option<u32> = None;
    let mut last = 0u32;
    let mut has_text = false;
    for part in parts {
        for ch in part.chars() {
            let width = u32::try_from(ch.width().unwrap_or(0)).unwrap_or(u32::MAX);
            if !ch.is_whitespace() {
                first.get_or_insert(col);
                last = col.saturating_add(width);
                if !is_tree_decoration(ch) {
                    has_text = true;
                }
            }
            col = col.saturating_add(width);
        }
    }
    has_text.then(|| (clamp_u16(first.unwrap_or(0)), clamp_u16(last)))
}

/// 该字符是否只属于树体/表格装饰：连接符、竖线、横线与框线。
fn is_tree_decoration(ch: char) -> bool {
    matches!(
        ch,
        '├' | '└'
            | '│'
            | '─'
            | '┬'
            | '┴'
            | '┼'
            | '┤'
            | '┌'
            | '┐'
            | '┘'
            | '┗'
            | '┏'
            | '┓'
            | '┣'
            | '┫'
            | '┳'
            | '┻'
            | '╋'
    )
}

/// `Segment` 行（树体、汇报正文等）的文字列区间。
pub(super) fn segment_cols(segments: &[Segment]) -> Option<(u16, u16)> {
    cols_from(0, segments.iter().map(|segment| segment.text.as_str()))
}

/// 单个纯文本行的文字列区间。
pub(super) fn line_cols(text: &str) -> Option<(u16, u16)> {
    cols_from(0, std::iter::once(text))
}

/// Markdown 物理行：行首 `indent` 计入列宽，返回相对行首的区间。
pub(super) fn md_line_cols(line: &MdLine) -> Option<(u16, u16)> {
    cols_from(
        line.indent,
        line.spans.iter().map(|(text, _)| text.as_str()),
    )
}

fn clamp_u16(value: u32) -> u16 {
    u16::try_from(value).unwrap_or(u16::MAX)
}

#[cfg(test)]
mod tests;
