//! 流式 Markdown 增量渲染：稳定前缀（`Arc<Vec<MdLine>>`）跨帧复用，
//! 每帧只解析新增的完整行与末尾未闭合的活跃尾部。
//!
//! 稳定边界只取「后续输入不会改变已产出结果」的位置：
//! - 段落 / 标题 / 列表 / 引用 / 分隔线 / 图片：每个完整行独立定型；
//! - 围栏代码块：围栏内每个完整代码行独立定型（闭合围栏行只推进状态）；
//! - 表格与缩进代码块：块宽/行数依赖整块内容，必须等块闭合后一次性定型，
//!   未闭合时整块留在尾部重排。
//!
//! 尾部用与全量渲染完全相同的块级状态机继续解析，因此 `prefix + tail`
//! 与 `render_wrapped` 的结果逐行相等。

use std::sync::Arc;

use super::{
    MdLine, ParserState, is_indented_code, list_item, md_bytes, render_blocks_stateful,
    wrap_md_line,
};

/// 单个流式 Markdown 块的增量解析状态。
#[derive(Debug, Clone, Default)]
pub(crate) struct IncrementalMarkdownState {
    /// 上一次处理的渲染宽度（宽度变化时全量重置）。
    width: u16,
    /// 已固化的源码字节偏移（恒位于 `\n` 之后）。
    committed_bytes: usize,
    /// 已固化前缀的首尾指纹（校验新源码是否严格以前缀追加）。
    committed_edges: u64,
    /// 已固化的跨行解析状态。
    parser: ParserState,
    /// 已固化的渲染行（跨帧原地追加，零克隆）。
    stable_lines: Arc<Vec<MdLine>>,
    /// `stable_lines` 的近似字节数账本。
    stable_bytes: usize,
}

impl IncrementalMarkdownState {
    /// 更新到 `source` 的当前内容，返回（稳定前缀、前缀字节数、活跃尾部行）。
    pub(crate) fn update(
        &mut self,
        source: &str,
        width: u16,
    ) -> (Arc<Vec<MdLine>>, usize, Vec<MdLine>) {
        self.reset_if_needed(source, width);
        let wrap_width = usize::from(width.max(1));
        loop {
            let rest = &source[self.committed_bytes..];
            let Some(last_newline) = rest.rfind('\n') else {
                break;
            };
            // 最后一个换行之前的完整行；末尾半行永远留在尾部。
            let lines: Vec<&str> = rest[..last_newline].split('\n').collect();
            let safe = safe_line_count(&lines, &self.parser);
            if safe == 0 {
                break;
            }
            let boundary = line_offset(&lines, safe);
            // 去掉最后一个换行：末尾空串属于**下一步**的活跃尾部，不能在这里
            // 当成一条已完成的空行（围栏代码块里会多出一条空代码行）。
            let committed = rest[..boundary]
                .strip_suffix('\n')
                .unwrap_or(&rest[..boundary]);
            let produced =
                render_blocks_stateful(committed, Some(wrap_width), true, true, &mut self.parser);
            let mut wrapped = Vec::new();
            for line in &produced {
                wrap_md_line(line, wrap_width, &mut wrapped);
            }
            self.stable_bytes = self.stable_bytes.saturating_add(md_bytes(&wrapped));
            Arc::make_mut(&mut self.stable_lines).extend(wrapped);
            self.committed_bytes = self.committed_bytes.saturating_add(boundary);
            self.committed_edges = edges_hash(&source.as_bytes()[..self.committed_bytes]);
        }
        // 尾部用状态副本解析：不得污染已固化的解析状态。
        let mut parser = self.parser.clone();
        let produced = render_blocks_stateful(
            &source[self.committed_bytes..],
            Some(wrap_width),
            true,
            true,
            &mut parser,
        );
        let mut tail = Vec::new();
        for line in &produced {
            wrap_md_line(line, wrap_width, &mut tail);
        }
        (Arc::clone(&self.stable_lines), self.stable_bytes, tail)
    }

    /// 宽度变化、源码被重写或前缀不一致时全量重置。
    fn reset_if_needed(&mut self, source: &str, width: u16) {
        let valid = self.width == width
            && self.committed_bytes <= source.len()
            && source.is_char_boundary(self.committed_bytes)
            && edges_hash(&source.as_bytes()[..self.committed_bytes]) == self.committed_edges;
        if valid {
            return;
        }
        self.width = width;
        self.committed_bytes = 0;
        self.committed_edges = 0;
        self.parser = ParserState::default();
        self.stable_lines = Arc::new(Vec::new());
        self.stable_bytes = 0;
    }
}

/// 已写定内容的译码指纹：长度 + 首尾各 64 字节（流式追加必然改变其一）。
fn edges_hash(bytes: &[u8]) -> u64 {
    if bytes.is_empty() {
        return 0;
    }
    const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = OFFSET_BASIS;
    let mut write = |slice: &[u8]| {
        for byte in slice {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(PRIME);
        }
    };
    write(&(bytes.len() as u64).to_le_bytes());
    write(&bytes[..bytes.len().min(64)]);
    write(&bytes[bytes.len().saturating_sub(64)..]);
    hash
}

/// 可从当前状态安全固化的完整行数；遇到未闭合的表格/缩进代码块时停在其起始行。
fn safe_line_count(lines: &[&str], state: &ParserState) -> usize {
    // 围栏闭合只取决于行内容，用局部标记即可。
    let mut in_fence = state.open_fence_lang.is_some();
    let mut index = 0;
    while index < lines.len() {
        let line = lines[index];
        if in_fence {
            if line.trim_start().starts_with("```") {
                in_fence = false;
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
            in_fence = true;
            index += 1;
            continue;
        }
        if list_item(line).is_some() {
            index += 1;
            continue;
        }
        if is_indented_code(line) {
            match indented_block_end(lines, index) {
                Some(end) => {
                    index = end;
                    continue;
                }
                None => return index,
            }
        }
        if trimmed.trim_start().starts_with('|') {
            let mut end = index;
            while end < lines.len() && lines[end].trim_start().starts_with('|') {
                end += 1;
            }
            if end == lines.len() {
                return index;
            }
            index = end;
            continue;
        }
        index += 1;
    }
    index
}

/// 缩进代码块的结束行下标；未闭合（一直延伸到最后一个完整行）返回 None。
fn indented_block_end(lines: &[&str], start: usize) -> Option<usize> {
    let mut index = start;
    while index < lines.len() {
        if is_indented_code(lines[index]) {
            index += 1;
            continue;
        }
        if lines[index].trim().is_empty() {
            match lines.get(index + 1) {
                // 空行后面仍是缩进行：空行属于代码块内部。
                Some(next) if is_indented_code(next) => {
                    index += 1;
                    continue;
                }
                // 末尾空行：下一行还在流式输出，可能继续缩进代码块 → 暂不固化。
                None => return None,
                Some(_) => return Some(index),
            }
        }
        return Some(index);
    }
    None
}

/// 前 `count` 个完整行的字节长度（每行都含结尾 `\n`）。
fn line_offset(lines: &[&str], count: usize) -> usize {
    lines
        .iter()
        .take(count)
        .map(|line| line.len().saturating_add(1))
        .sum()
}

#[cfg(test)]
mod tests;
