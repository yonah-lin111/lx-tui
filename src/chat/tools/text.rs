//! 文本文件编解码：BOM / 换行风格保留与行切分。
//!
//! 工具层所有读写都经过 [`SourceText`]：读入时记录 BOM 与换行风格，写回时按原样
//! 还原（CRLF 文件不会变成 LF，带 BOM 的文件不会丢 BOM）；未改动的行保留原始字节。

use super::types::{ToolErrorCode, ToolFailure};

/// UTF-8 BOM 字节序列。
const BOM: [u8; 3] = [0xEF, 0xBB, 0xBF];

/// 文件换行风格（写入时统一采用该风格）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Newline {
    Lf,
    Crlf,
}

impl Newline {
    /// 风格对应的字节序列。
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Lf => "\n",
            Self::Crlf => "\r\n",
        }
    }
}

/// 解码后的文本文件：保留 BOM、换行风格与末行换行状态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SourceText {
    /// 原文件是否以 UTF-8 BOM 开头。
    pub(crate) bom: bool,
    /// 主要换行风格。
    pub(crate) newline: Newline,
    /// 逐行文本（不含行尾 `\r`/`\n`）。
    pub(crate) lines: Vec<String>,
    /// 原文件是否以换行结尾。
    pub(crate) trailing_newline: bool,
}

impl SourceText {
    /// 从原始字节解码。
    ///
    /// 含 NUL 判定为二进制（`Binary`），非法 UTF-8 返回 `NotUtf8`。
    pub(crate) fn parse(bytes: &[u8]) -> Result<Self, ToolFailure> {
        if bytes.contains(&0) {
            return Err(ToolFailure::new(
                ToolErrorCode::Binary,
                "file looks binary (contains NUL bytes); use a text editor instead",
            ));
        }
        let (bom, body) = match bytes.strip_prefix(BOM.as_slice()) {
            Some(rest) => (true, rest),
            None => (false, bytes),
        };
        let text = std::str::from_utf8(body).map_err(|error| {
            ToolFailure::new(
                ToolErrorCode::NotUtf8,
                format!(
                    "file is not valid UTF-8 (invalid byte at offset {})",
                    error.valid_up_to()
                ),
            )
        })?;
        Ok(Self::from_text(text, bom))
    }

    /// 由已解码文本构造；`bom` 表示原文件是否带 BOM。
    pub(crate) fn from_text(text: &str, bom: bool) -> Self {
        let newline = if text.contains("\r\n") {
            Newline::Crlf
        } else {
            Newline::Lf
        };
        let trailing_newline = text.ends_with('\n');
        Self {
            bom,
            newline,
            lines: split_lines(text),
            trailing_newline,
        }
    }

    /// 以指定行集合编码回字节：保留 BOM 与换行风格；空行集合不写尾部换行。
    pub(crate) fn encode(&self, lines: &[String]) -> Vec<u8> {
        let mut text = String::new();
        let newline = self.newline.as_str();
        for (index, line) in lines.iter().enumerate() {
            if index > 0 {
                text.push_str(newline);
            }
            text.push_str(line);
        }
        // 空文件不补换行；非空且原文件以换行结尾时保留结尾换行。
        if !lines.is_empty() && self.trailing_newline {
            text.push_str(newline);
        }
        let mut bytes = Vec::with_capacity(text.len() + BOM.len());
        if self.bom {
            bytes.extend_from_slice(&BOM);
        }
        bytes.extend_from_slice(text.as_bytes());
        bytes
    }
}

/// 切分文本为行：`\r\n` 与 `\n` 都是行尾，末行无换行时仍作为一行。
///
/// 与 `str::lines` 同口径：结尾换行不产生额外的空行。
pub(crate) fn split_lines(text: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                lines.push(std::mem::take(&mut current));
            }
            '\n' => lines.push(std::mem::take(&mut current)),
            other => current.push(other),
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

/// 内容是否只由空白组成（用于判断空内容清空）。
#[cfg(test)]
pub(crate) fn is_blank(text: &str) -> bool {
    text.trim().is_empty()
}

#[cfg(test)]
mod tests;
