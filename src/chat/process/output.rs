//! 进程输出缓冲：有界尾部、绝对字节偏移、丢弃证据与安全文本化。
//!
//! 边界：只有字节与纯函数，不含进程句柄、不含 UI 概念。
//!
//! 不变量（对应 design §7 / §10.1）：
//!
//! 1. 每个进程只保留**尾部** [`MAX_PROCESS_OUTPUT_BYTES`]；超出从头部淘汰，
//!    `dropped_bytes()` 始终等于保留区间起点（即累计丢弃量），绝不静默丢数据；
//! 2. 偏移是**原始字节**偏移（`start` / `end`），不是字符数或行数；
//! 3. 模型读取游标与 UI 预览是两个独立游标，UI 读多少次都不影响模型看到的区间；
//! 4. 文本化时剥离 ANSI/OSC 等控制序列，绝不把子进程的控制字符写进真实终端。

use std::collections::VecDeque;

/// 每进程内存保留的输出上限（原始字节，尾部）。
pub const MAX_PROCESS_OUTPUT_BYTES: usize = 256 * 1024;

/// 单次回给模型的文本字节硬上限。
pub const MAX_MODEL_OUTPUT_BYTES: usize = 64 * 1024;

/// 单条控制序列的最大长度：超限即丢弃该序列（不无限等待终止符）。
const MAX_CONTROL_SEQUENCE_BYTES: usize = 256;

/// 输出片段的来源（管道两路天然分开；PTY 合流）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputSource {
    Stdout,
    Stderr,
    /// PTY 合流输出。
    Pty,
}

impl OutputSource {
    /// 稳定标签（结果 JSON 与 UI 提示共用）。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Stdout => "stdout",
            Self::Stderr => "stderr",
            Self::Pty => "pty",
        }
    }
}

/// 一段同来源连续输出。
#[derive(Debug, Clone, PartialEq, Eq)]
struct Segment {
    source: OutputSource,
    bytes: Vec<u8>,
}

/// 一次读取的结果（绝对字节偏移 + 安全文本）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputRead {
    /// 实际返回区间的起点（绝对字节偏移）。
    pub start: u64,
    /// 实际返回区间的终点（下次读取的游标）。
    pub end: u64,
    /// 请求起点早于保留区间时丢失的字节数。
    pub dropped_bytes: u64,
    /// 是否还有未返回的输出（字节预算或缓冲淘汰）。
    pub truncated: bool,
    /// 返回的文本（已剥离控制序列、非法字节显式可见化）。
    pub text: String,
}

/// 有界输出环形缓冲：尾部保留 + 绝对偏移。
#[derive(Debug, Clone)]
pub struct OutputBuffer {
    segments: VecDeque<Segment>,
    /// 当前保留的原始字节数。
    retained: usize,
    /// 保留区间起点（绝对字节偏移，等于累计丢弃量）。
    start: u64,
    /// 累计写入的字节数（绝对终点偏移）。
    end: u64,
    capacity: usize,
}

impl Default for OutputBuffer {
    fn default() -> Self {
        Self::new(MAX_PROCESS_OUTPUT_BYTES)
    }
}

impl OutputBuffer {
    /// 以指定容量构造（测试用小容量）。
    pub fn new(capacity: usize) -> Self {
        Self {
            segments: VecDeque::new(),
            retained: 0,
            start: 0,
            end: 0,
            capacity: capacity.max(1),
        }
    }

    /// 追加一段输出；自动淘汰超出容量的头部字节。
    pub fn push(&mut self, source: OutputSource, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        // 同来源且并入后仍不超容量时合并，避免段数随输出次数无界增长。
        if let Some(last) = self.segments.back_mut()
            && last.source == source
            && last.bytes.len() + bytes.len() <= self.capacity
        {
            last.bytes.extend_from_slice(bytes);
        } else {
            self.segments.push_back(Segment {
                source,
                bytes: bytes.to_vec(),
            });
        }
        self.retained += bytes.len();
        self.end = self.end.saturating_add(bytes.len() as u64);
        self.trim();
    }

    /// 淘汰超出容量的头部字节。
    fn trim(&mut self) {
        while self.retained > self.capacity {
            let Some(front) = self.segments.front_mut() else {
                break;
            };
            let excess = self.retained - self.capacity;
            if front.bytes.len() <= excess {
                let removed = front.bytes.len();
                self.segments.pop_front();
                self.retained -= removed;
                self.start = self.start.saturating_add(removed as u64);
                continue;
            }
            // 单段就超出容量：从段内头部丢弃（可能切断 UTF-8 字符，读取时按 lossy 处理）。
            front.bytes.drain(..excess);
            self.retained -= excess;
            self.start = self.start.saturating_add(excess as u64);
        }
    }

    /// 保留区间起点（绝对字节偏移）。
    pub fn start(&self) -> u64 {
        self.start
    }

    /// 绝对终点偏移（累计写入字节数）。
    pub fn end(&self) -> u64 {
        self.end
    }

    /// 已丢弃的字节数（保留区间之前累计淘汰的量）。
    pub fn dropped_bytes(&self) -> u64 {
        self.start
    }

    /// 当前保留的原始字节数。
    pub fn retained(&self) -> usize {
        self.retained
    }

    /// 从 `cursor` 起读取最多 `max_bytes` 原始字节并文本化。
    ///
    /// 截断只推进到**实际返回**的区间：剩余输出留给下一次读取，绝不先推进游标再
    /// 裁掉正文。末尾不完整的 UTF-8 字节留到下次（`flush` 为真时按 `�` 显式收尾）。
    pub fn read_from(&self, cursor: u64, max_bytes: usize, flush: bool) -> OutputRead {
        let cursor = cursor.min(self.end);
        let from = cursor.max(self.start);
        let dropped_bytes = from.saturating_sub(cursor);
        let budget = max_bytes.clamp(1, MAX_PROCESS_OUTPUT_BYTES) as u64;
        let to = from.saturating_add(budget).min(self.end);
        let bytes = self.bytes_between(from, to);
        let (text, consumed) = sanitize_text(&bytes, flush);
        let end = from.saturating_add(consumed);
        OutputRead {
            start: from,
            end,
            dropped_bytes,
            truncated: end < self.end,
            text,
        }
    }

    /// 取出 `[from, to)` 的原始字节（跨来源段拼接）。
    fn bytes_between(&self, from: u64, to: u64) -> Vec<u8> {
        let mut out = Vec::with_capacity(to.saturating_sub(from) as usize);
        let mut position = self.start;
        for segment in &self.segments {
            if position >= to {
                break;
            }
            let seg_end = position.saturating_add(segment.bytes.len() as u64);
            if seg_end <= from {
                position = seg_end;
                continue;
            }
            let begin = from.saturating_sub(position) as usize;
            let finish = (to.saturating_sub(position) as usize).min(segment.bytes.len());
            if begin < finish {
                out.extend_from_slice(&segment.bytes[begin..finish]);
            }
            position = seg_end;
        }
        out
    }
}

/// 把原始字节转成可安全显示的文本，返回（文本，消费的原始字节数）。
///
/// - 剥离 CSI / OSC / DCS 等转义序列（超长序列丢弃，未闭合序列**留给下次读取**）；
/// - `\r\n` 与孤立 `\r` 都归一为换行（进度条不把光标写回真实终端）；
/// - 其它控制字符转空格，`\n` / `\t` 原样保留；
/// - 非法 UTF-8 用 `�` 显式可见化；`flush` 为假时尾部不完整字符不计入消费量。
fn sanitize_text(bytes: &[u8], flush: bool) -> (String, u64) {
    let mut out = String::new();
    let mut index = 0usize;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte == 0x1b {
            match skip_escape(bytes, index) {
                Escape::Complete(next) => index = next,
                // 超长：确定是垃圾，丢掉整段（连同其后的可见文本，避免无限等待）。
                Escape::Overlong => index = bytes.len(),
                // 未闭合：序列可能被读取边界切开了，**不消费**、下次带更多字节重试。
                // 若在这里整段吞掉并推进游标，后半段（如 `2mred`）会在下次被当正文
                // 输出给模型——而尾部不完整 UTF-8 的策略正是「不消费」，两者必须一致。
                Escape::Pending => break,
            }
            continue;
        }
        if byte < 0x20 || byte == 0x7f {
            index += 1;
            match byte {
                b'\r' => {
                    if index < bytes.len() && bytes[index] == b'\n' {
                        index += 1;
                    }
                    out.push('\n');
                }
                b'\n' | b'\t' => out.push(byte as char),
                _ => out.push(' '),
            }
            continue;
        }
        match std::str::from_utf8(&bytes[index..]) {
            Ok(text) => {
                // 只消费到下一个需要特殊处理的字节（控制字符或 ESC）：否则后面的
                // 转义序列会被当成普通文本原样画出。
                let end = text
                    .bytes()
                    .position(|byte| byte < 0x20 || byte == 0x7f)
                    .map_or(bytes.len(), |offset| index + offset);
                if end == index {
                    // 理论上不会发生（上面已处理控制字符）；防御性前进一步。
                    index += 1;
                    continue;
                }
                // SAFETY: 前缀是合法 UTF-8 的子串边界（`position` 落在 ASCII 字节上）。
                out.push_str(unsafe { std::str::from_utf8_unchecked(&bytes[index..end]) });
                index = end;
            }
            Err(error) => {
                let valid = error.valid_up_to();
                if valid > 0 {
                    // SAFETY: `valid_up_to` 保证该前缀是合法 UTF-8。
                    out.push_str(unsafe {
                        std::str::from_utf8_unchecked(&bytes[index..index + valid])
                    });
                    index += valid;
                    continue;
                }
                match error.error_len() {
                    Some(length) => {
                        out.push('\u{fffd}');
                        index += length;
                    }
                    None if flush => {
                        // 终态收尾：残留的不完整字节显式可见化，不再等待后续数据。
                        out.push('\u{fffd}');
                        index = bytes.len();
                    }
                    // 尾部不完整字符：不消费，留给下次读取。
                    None => break,
                }
            }
        }
    }
    (out, index as u64)
}

/// [`skip_escape`] 的结论。
///
/// 「未闭合」与「超长」必须分开：前者可能只是被读取边界切开（下次会有后续字节），
/// 后者是确定无法解析的垃圾。合并成一个 `None` 会逼调用方二选一，两种语义必错一个。
enum Escape {
    /// 序列完整结束，值是序列之后的下标。
    Complete(usize),
    /// 数据不够，序列可能还有后续字节（调用方不消费，下次重试）。
    Pending,
    /// 超过 [`MAX_CONTROL_SEQUENCE_BYTES`]：确定要丢弃。
    Overlong,
}

/// 从 `start`（一个 `ESC`）起跳过一个转义序列。
fn skip_escape(bytes: &[u8], start: usize) -> Escape {
    let cap = start + MAX_CONTROL_SEQUENCE_BYTES;
    let limit = cap.min(bytes.len());
    // 读到字节用尽 vs 读到上限：前者是「可能还有后续」，后者是「确定太长」。
    let at_cap = |index: usize| {
        if index >= cap {
            Escape::Overlong
        } else {
            Escape::Pending
        }
    };
    let mut index = start + 1;
    if index >= limit {
        return at_cap(index);
    }
    match bytes[index] {
        b'[' => {
            // CSI：参数字节（0x20..=0x3f）+ 终止字节（0x40..=0x7e）。
            index += 1;
            while index < limit {
                let byte = bytes[index];
                if (0x40..=0x7e).contains(&byte) {
                    return Escape::Complete(index + 1);
                }
                if !(0x20..=0x3f).contains(&byte) {
                    // 非法中间字节：当作序列就此结束（丢弃到此为止）。
                    return Escape::Complete(index);
                }
                index += 1;
            }
            at_cap(index)
        }
        b']' | b'P' | b'X' | b'^' | b'_' => {
            // OSC / DCS / SOS / PM / APC：以 BEL 或 ST（`ESC \`）结束。
            index += 1;
            while index < limit {
                if bytes[index] == 0x07 {
                    return Escape::Complete(index + 1);
                }
                if bytes[index] == 0x1b && index + 1 < bytes.len() && bytes[index + 1] == b'\\' {
                    return Escape::Complete(index + 2);
                }
                index += 1;
            }
            at_cap(index)
        }
        _ => Escape::Complete(index + 1),
    }
}

/// 把安全文本切成显示行（保留空行）。
pub fn display_lines(text: &str) -> Vec<&str> {
    text.split('\n').collect()
}

#[cfg(test)]
mod tests;
