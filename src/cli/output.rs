//! 双模式输出器：支持 `--json` 结构化 JSON 与带 ANSI 颜色的人类可读表格/键值排版。

use std::io::{IsTerminal, Write};

use serde::Serialize;

use crate::chat::agent::exec::ColorMode;

/// CLI 输出控制器。
#[derive(Debug, Clone, Copy)]
pub struct Output {
    json: bool,
    color: bool,
}

impl Output {
    /// 根据 `--json`、`--color` 与终端/环境变量状态创建输出器。
    pub fn new(json: bool, color_mode: ColorMode) -> Self {
        let is_tty = std::io::stdout().is_terminal();
        let no_color = std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty());
        Self {
            json,
            color: !json && color_mode.should_color_with_env(is_tty, no_color),
        }
    }

    pub fn is_json(self) -> bool {
        self.json
    }

    pub fn use_color(self) -> bool {
        self.color
    }

    /// 输出 JSON 或执行人类可读渲染闭包。
    pub fn emit<T, F>(&self, value: &T, human: F) -> Result<(), String>
    where
        T: Serialize,
        F: FnOnce(&Self, &mut dyn Write) -> std::io::Result<()>,
    {
        let stdout = std::io::stdout();
        let mut handle = stdout.lock();
        self.emit_to(&mut handle, value, human)
    }

    /// 向任意 `Write` 输出（便于单元测试捕获输出）。
    pub fn emit_to<T, F>(&self, out: &mut dyn Write, value: &T, human: F) -> Result<(), String>
    where
        T: Serialize,
        F: FnOnce(&Self, &mut dyn Write) -> std::io::Result<()>,
    {
        if self.json {
            serde_json::to_writer_pretty(&mut *out, value)
                .map_err(|e| format!("failed to serialize JSON: {e}"))?;
            writeln!(out).map_err(|e| format!("output write failed: {e}"))?;
        } else {
            human(self, out).map_err(|e| format!("output write failed: {e}"))?;
        }
        out.flush().map_err(|e| format!("output flush failed: {e}"))
    }

    /// 加粗文本（仅启用颜色时包裹 ANSI 序列）。
    pub fn bold(&self, text: &str) -> String {
        if self.color {
            format!("\x1b[1m{text}\x1b[0m")
        } else {
            text.to_string()
        }
    }

    /// 暗色辅助文本。
    pub fn muted(&self, text: &str) -> String {
        if self.color {
            format!("\x1b[2m{text}\x1b[0m")
        } else {
            text.to_string()
        }
    }

    /// 绿色强调文本。
    pub fn green(&self, text: &str) -> String {
        if self.color {
            format!("\x1b[32m{text}\x1b[0m")
        } else {
            text.to_string()
        }
    }

    /// 黄色警告文本。
    pub fn yellow(&self, text: &str) -> String {
        if self.color {
            format!("\x1b[33m{text}\x1b[0m")
        } else {
            text.to_string()
        }
    }

    /// 青色标识文本。
    pub fn cyan(&self, text: &str) -> String {
        if self.color {
            format!("\x1b[36m{text}\x1b[0m")
        } else {
            text.to_string()
        }
    }

    /// 打印左对齐的纯文本表格（自动按列字符宽度对齐）。
    pub fn print_table(
        &self,
        out: &mut dyn Write,
        headers: &[&str],
        rows: &[Vec<String>],
    ) -> std::io::Result<()> {
        if headers.is_empty() {
            return Ok(());
        }
        let mut widths: Vec<usize> = headers.iter().map(|h| h.chars().count()).collect();
        for row in rows {
            for (idx, cell) in row.iter().enumerate() {
                if idx < widths.len() {
                    widths[idx] = widths[idx].max(cell.chars().count());
                }
            }
        }
        let mut header_line = String::new();
        for (idx, h) in headers.iter().enumerate() {
            if idx > 0 {
                header_line.push_str("  ");
            }
            if idx + 1 == headers.len() {
                header_line.push_str(h);
            } else {
                header_line.push_str(&pad_right(h, widths[idx]));
            }
        }
        writeln!(out, "{}", self.bold(&header_line))?;
        for row in rows {
            let mut line = String::new();
            for (idx, cell) in row.iter().enumerate() {
                if idx > 0 {
                    line.push_str("  ");
                }
                if idx + 1 == headers.len() {
                    line.push_str(cell);
                } else {
                    let width = widths.get(idx).copied().unwrap_or(0);
                    line.push_str(&pad_right(cell, width));
                }
            }
            writeln!(out, "{line}")?;
        }
        Ok(())
    }
}

fn pad_right(text: &str, width: usize) -> String {
    let len = text.chars().count();
    if len >= width {
        text.to_string()
    } else {
        format!("{text}{}", " ".repeat(width - len))
    }
}

/// 对敏感密钥（API Key / Token / Secret）做脱敏掩码展示。
pub fn mask_secret(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let chars: Vec<char> = trimmed.chars().collect();
    if chars.len() <= 8 {
        return "***".to_string();
    }
    let prefix: String = chars[..3].iter().collect();
    let suffix: String = chars[chars.len() - 4..].iter().collect();
    format!("{prefix}...{suffix}")
}
