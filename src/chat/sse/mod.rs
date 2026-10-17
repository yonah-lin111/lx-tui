//! 可增量喂入的 SSE 帧解析器：字节层只组行，行层按空行组帧。
//!
//! 覆盖：chunk 切在 UTF-8 字符中、CRLF/LF、多行 `data:`、空心跳（注释行）、
//! `[DONE]`、超长帧与半帧 EOF。`data:` 多行按规范以换行连接后交给上层 JSON 解析。

/// 解析出的一个 SSE 事件帧。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseFrame {
    /// `event:` 字段值（一帧内首个为准）。
    pub event: Option<String>,
    /// 全部 `data:` 行以 `\n` 连接后的完整载荷（不含 `data:` 前缀与引导空格）。
    pub data: String,
}

/// 喂入错误：帧超限（按协议失败处理）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameTooLarge;

/// SSE 增量解析器：喂入任意切割的字节块，完整帧经 `take_frames` 取走。
///
/// 行缓冲按字节处理（允许 chunk 切在 UTF-8 字符中间）；`lossy` 转换只发生在
/// 行完成时，正常 UTF-8 流无损。
#[derive(Debug, Default)]
pub struct SseParser {
    /// 行缓冲（字节级）。
    line: Vec<u8>,
    /// 当前帧已收的 data 行。
    data: Vec<String>,
    /// 当前帧 event 行。
    event: Option<String>,
    /// 上一字节是 CR（CRLF 去重用）。
    pending_cr: bool,
    /// 已产出的完整帧。
    out: Vec<SseFrame>,
}

impl SseParser {
    pub fn new() -> Self {
        Self::default()
    }

    /// 喂入一块字节；帧超限返回错误。
    ///
    /// 行边界识别兼容 `\n`、`\r\n` 与独立 `\r`：CR 即结束当前行，
    /// 紧跟 LF 时由 LF 分支去重（`pending_cr`）；连续 CR 的第一个构成空行
    /// （帧边界）。chunk 切在 UTF-8 字符中由字节级行缓冲无损处理。
    pub fn feed(&mut self, chunk: &[u8], max_frame_bytes: usize) -> Result<(), FrameTooLarge> {
        for &byte in chunk {
            match byte {
                b'\n' => {
                    self.pending_cr = false;
                    self.complete_line(max_frame_bytes)?;
                }
                b'\r' => {
                    // CR 结束当前行（行缓冲此刻不含 CR）；
                    // 行缓冲为空时 complete_line 即为空行（帧边界）。
                    self.complete_line(max_frame_bytes)?;
                    self.pending_cr = true;
                }
                _ => {
                    // CR 后跟非 LF：CR 已作为行结束符消费，不作为内容字节回填。
                    self.pending_cr = false;
                    if self.line.len() >= max_frame_bytes {
                        return Err(FrameTooLarge);
                    }
                    self.line.push(byte);
                }
            }
        }
        Ok(())
    }

    /// 输入结束时冲刷：未以空行终止的半帧也吐出（协议层按需判定完成性）。
    pub fn flush(&mut self) -> Option<SseFrame> {
        if !self.line.is_empty() {
            let line = String::from_utf8_lossy(&self.line).into_owned();
            self.line.clear();
            self.absorb_line(&line);
        }
        self.pending_cr = false;
        if self.data.is_empty() && self.event.is_none() {
            return None;
        }
        Some(SseFrame {
            event: self.event.take(),
            data: std::mem::take(&mut self.data).join("\n"),
        })
    }

    /// 取走已产出的完整帧。
    pub fn take_frames(&mut self) -> Vec<SseFrame> {
        std::mem::take(&mut self.out)
    }

    /// 完成一行；空行时当前帧入列。
    fn complete_line(&mut self, max_frame_bytes: usize) -> Result<(), FrameTooLarge> {
        if self.line.len() > max_frame_bytes {
            return Err(FrameTooLarge);
        }
        let line = String::from_utf8_lossy(&self.line).into_owned();
        self.line.clear();
        if line.is_empty() {
            // 空行：当前帧结束（心跳的空行直接丢弃）。
            if !self.data.is_empty() || self.event.is_some() {
                self.out.push(SseFrame {
                    event: self.event.take(),
                    data: std::mem::take(&mut self.data).join("\n"),
                });
            }
            return Ok(());
        }
        self.absorb_line(&line);
        Ok(())
    }

    /// 吸收一个非空行到当前帧。
    fn absorb_line(&mut self, line: &str) {
        if let Some(rest) = line.strip_prefix("data:") {
            self.data
                .push(rest.strip_prefix(' ').unwrap_or(rest).to_string());
        } else if let Some(rest) = line.strip_prefix("event:")
            && self.event.is_none()
        {
            self.event = Some(rest.strip_prefix(' ').unwrap_or(rest).to_string());
        }
        // 注释行（`:` 开头）与未知字段忽略（心跳）。
    }
}

#[cfg(test)]
mod tests;
