//! `exec` 双模式输出处理器：人类可读进度（`stderr` + 终态 `stdout`）与 JSONL 事件流。
//!
//! 分流契约（design §4.3.2）：
//! - 所有过程信息只写 `stderr`，不污染管道；
//! - `stdout` 不是终端（管道/重定向）时，整轮结束只在 `stdout` 写一份干净的最终回复；
//! - `--json` 模式下 `stdout` 只承载单行 JSON 事件，每行立即 flush；
//! - `-o/--output-last-message` 始终把最终回复原子写入目标文件。
//!
//! 两个处理器都通过泛型 `Write` 注入输出句柄，单元测试用 `Vec<u8>` 捕获字节。

use std::collections::HashMap;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use super::events::{
    ExecUsage, ThreadError, ThreadEvent, ThreadItem, format_tool_header, format_tool_outcome_line,
    item_details_for_finish, item_details_for_start,
};
use crate::chat::agent_loop::{ReadyToolCall, ToolOutcome, TurnOutcome, TurnSummary};

/// 启动头部信息（人类模式横幅与 JSONL `thread.started` 共用）。
pub struct SessionHeader<'a> {
    pub version: &'a str,
    pub workdir: &'a std::path::Path,
    pub model: &'a str,
    pub provider: &'a str,
    pub session_id: &'a str,
    pub prompt: &'a str,
}

/// 无头运行器向输出层回报的生命周期回调。
pub trait ExecEventProcessor: Send {
    /// 会话开始：打印横幅（人类模式）或 `thread.started`（JSONL）。
    fn on_session_started(&mut self, header: &SessionHeader<'_>);
    /// 模型思考块闭合。
    fn on_reasoning(&mut self, text: &str);
    /// 模型正文块闭合（中间步骤或最终步骤）。
    fn on_assistant_text(&mut self, text: &str);
    /// 工具开始执行。
    fn on_tool_started(&mut self, call: &ReadyToolCall);
    /// 工具完成结算。
    fn on_tool_finished(&mut self, call: &ReadyToolCall, outcome: &ToolOutcome, duration_ms: i64);
    /// 告警或非致命通知（如自动 compact）。
    fn on_warning(&mut self, message: &str);
    /// 整轮结束：刷出最终 `stdout` 与 `-o` 文件。
    fn on_turn_finished(
        &mut self,
        summary: &TurnSummary,
        final_message: Option<&str>,
    ) -> io::Result<()>;
}

const RESET: &str = "\x1b[0m";
const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";
const CYAN: &str = "\x1b[36m";
const RED: &str = "\x1b[31m";
const YELLOW: &str = "\x1b[33m";

/// 人类可读输出：进度进 `stderr`，终态文本进 `stdout`（仅管道模式）。
pub struct HumanOutputProcessor<WOut: Write + Send, WErr: Write + Send> {
    stdout: WOut,
    stderr: WErr,
    use_color: bool,
    stdout_is_terminal: bool,
    last_message_file: Option<PathBuf>,
    /// 已收到但尚未判断「中间步骤还是最终回复」的正文块。
    pending_text: Option<String>,
}

impl<WOut: Write + Send, WErr: Write + Send> HumanOutputProcessor<WOut, WErr> {
    pub fn new(
        stdout: WOut,
        stderr: WErr,
        use_color: bool,
        stdout_is_terminal: bool,
        last_message_file: Option<PathBuf>,
    ) -> Self {
        Self {
            stdout,
            stderr,
            use_color,
            stdout_is_terminal,
            last_message_file,
            pending_text: None,
        }
    }

    /// 绑定进程标准输出/标准错误。
    pub fn stdio(
        use_color: bool,
        stdout_is_terminal: bool,
        last_message_file: Option<PathBuf>,
    ) -> HumanOutputProcessor<std::io::Stdout, std::io::Stderr> {
        HumanOutputProcessor::new(
            io::stdout(),
            io::stderr(),
            use_color,
            stdout_is_terminal,
            last_message_file,
        )
    }

    /// 写一行进度；管道早退不属于本轮失败，记录后继续。
    fn line(&mut self, text: &str) {
        if let Err(error) = writeln!(self.stderr, "{text}") {
            tracing::warn!(target: "lx::exec", %error, "failed to write progress output");
        }
    }

    fn paint(&self, code: &str, text: &str) -> String {
        if self.use_color {
            format!("{code}{text}{RESET}")
        } else {
            text.to_string()
        }
    }

    /// 正文块在遇到下一条步骤前都属于中间步骤：先落后一个空行再继续。
    fn flush_pending_text(&mut self) {
        let Some(text) = self.pending_text.take() else {
            return;
        };
        self.write_intermediate(&text);
    }

    fn write_intermediate(&mut self, text: &str) {
        if text.trim().is_empty() {
            return;
        }
        let heading = self.paint(CYAN, "lx");
        self.line("");
        self.line(&heading);
        self.line(text);
        self.line("");
    }
}

impl<WOut: Write + Send, WErr: Write + Send> ExecEventProcessor
    for HumanOutputProcessor<WOut, WErr>
{
    fn on_session_started(&mut self, header: &SessionHeader<'_>) {
        let title = self.paint(BOLD, &format!("lx-tui exec v{}", header.version));
        let rule = self.paint(DIM, "--------");
        let label = |name: &str, value: &str| -> String {
            if value.is_empty() {
                format!("{name}: (unknown)")
            } else {
                format!("{name}: {value}")
            }
        };
        self.line(&title);
        self.line(&rule);
        self.line(&label("workdir", &header.workdir.display().to_string()));
        self.line(&label("model", header.model));
        self.line(&label("provider", header.provider));
        self.line(&label("session id", header.session_id));
        self.line(&rule);
        self.line("user");
        self.line(header.prompt);
        self.line("");
    }

    fn on_reasoning(&mut self, text: &str) {
        self.flush_pending_text();
        let marker = self.paint(CYAN, "● Thinking...");
        self.line(&marker);
        for line in text.lines() {
            let body = self.paint(DIM, &format!("  {line}"));
            self.line(&body);
        }
        self.line("");
    }

    fn on_assistant_text(&mut self, text: &str) {
        if text.trim().is_empty() {
            return;
        }
        // 每次回调都是一段已闭合正文：前一段没有别的机会输出，先按中间步骤落盘，
        // 只把最新一段留作最终回复候选。
        if self.pending_text.is_some() {
            self.flush_pending_text();
        }
        self.pending_text = Some(text.to_string());
    }

    fn on_tool_started(&mut self, call: &ReadyToolCall) {
        self.flush_pending_text();
        let header = format_tool_header(call);
        let line = self.paint(CYAN, &format!("■ {header}"));
        self.line(&line);
    }

    fn on_tool_finished(&mut self, call: &ReadyToolCall, outcome: &ToolOutcome, duration_ms: i64) {
        let (summary, is_error) = format_tool_outcome_line(call, outcome, duration_ms);
        let detail = if is_error {
            self.paint(RED, &format!("  └─ {summary}"))
        } else {
            self.paint(DIM, &format!("  └─ {summary}"))
        };
        self.line(&detail);
        self.line("");
    }

    fn on_warning(&mut self, message: &str) {
        let line = self.paint(YELLOW, &format!("warning: {message}"));
        self.line(&line);
    }

    fn on_turn_finished(
        &mut self,
        summary: &TurnSummary,
        final_message: Option<&str>,
    ) -> io::Result<()> {
        let pending = self.pending_text.take();
        let final_text = match final_message {
            Some(text) => {
                // 与最终回复不同的正文块没有别的机会输出：先按中间步骤落盘。
                if let Some(pending) = pending.filter(|pending| pending != text) {
                    self.write_intermediate(&pending);
                }
                Some(text.to_string())
            }
            None => pending,
        };
        if let Some(path) = self.last_message_file.as_deref() {
            write_last_message(path, final_text.as_deref().unwrap_or_default())?;
        }

        match &summary.outcome {
            TurnOutcome::Complete => {
                if self.stdout_is_terminal {
                    if let Some(text) = final_text.as_deref() {
                        let heading = self.paint(CYAN, "lx");
                        self.line("");
                        self.line(&heading);
                        self.line(text);
                        self.line("");
                    }
                } else if let Some(text) = final_text.as_deref() {
                    writeln!(self.stdout, "{text}")?;
                    self.stdout.flush()?;
                }
                let usage = self.paint(DIM, &usage_line(&summary.billing));
                self.line(&usage);
            }
            TurnOutcome::Interrupted => {
                let line = self.paint(YELLOW, "turn interrupted");
                self.line(&line);
            }
            TurnOutcome::Failed { code } => {
                let line = self.paint(RED, &format!("turn failed: {code}"));
                self.line(&line);
            }
        }
        if let Err(error) = self.stderr.flush() {
            tracing::warn!(target: "lx::exec", %error, "failed to flush progress output");
        }
        Ok(())
    }
}

/// JSONL 输出：每条 `ThreadEvent` 一行，写后立即 flush。
pub struct JsonlOutputProcessor<WOut: Write + Send> {
    stdout: WOut,
    last_message_file: Option<PathBuf>,
    next_item_id: u64,
    last_emitted_text: Option<String>,
    /// 在途工具调用：`call.call_id -> item.id`，保证同一调用的
    /// `item.started` 与 `item.completed` 携带相同且稳定的 id。
    active_tool_items: HashMap<String, String>,
}

impl<WOut: Write + Send> JsonlOutputProcessor<WOut> {
    pub fn new(stdout: WOut, last_message_file: Option<PathBuf>) -> Self {
        Self {
            stdout,
            last_message_file,
            next_item_id: 0,
            last_emitted_text: None,
            active_tool_items: HashMap::new(),
        }
    }

    /// 绑定进程标准输出。
    pub fn stdio(last_message_file: Option<PathBuf>) -> JsonlOutputProcessor<std::io::Stdout> {
        JsonlOutputProcessor::new(io::stdout(), last_message_file)
    }

    /// 写入一条事件并 flush；回调不返回错误，因此失败只记录。
    fn emit(&mut self, event: &ThreadEvent) {
        if let Err(error) = self.write_event(event) {
            tracing::warn!(target: "lx::exec", %error, "failed to write JSONL event");
        }
    }

    fn write_event(&mut self, event: &ThreadEvent) -> io::Result<()> {
        let line = serde_json::to_string(event)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        writeln!(self.stdout, "{line}")?;
        self.stdout.flush()
    }

    fn next_item_id(&mut self) -> String {
        let id = format!("item_{}", self.next_item_id);
        self.next_item_id = self.next_item_id.saturating_add(1);
        id
    }

    fn next_item(&mut self, details: super::events::ThreadItemDetails) -> ThreadItem {
        ThreadItem {
            id: self.next_item_id(),
            details,
        }
    }
}

impl<WOut: Write + Send> ExecEventProcessor for JsonlOutputProcessor<WOut> {
    fn on_session_started(&mut self, header: &SessionHeader<'_>) {
        self.emit(&ThreadEvent::ThreadStarted {
            thread_id: header.session_id.to_string(),
        });
        self.emit(&ThreadEvent::TurnStarted);
    }

    fn on_reasoning(&mut self, text: &str) {
        let item = self.next_item(super::events::ThreadItemDetails::Reasoning {
            text: text.to_string(),
        });
        self.emit(&ThreadEvent::ItemCompleted { item });
    }

    fn on_assistant_text(&mut self, text: &str) {
        if text.trim().is_empty() {
            return;
        }
        self.last_emitted_text = Some(text.to_string());
        let item = self.next_item(super::events::ThreadItemDetails::AgentMessage {
            text: text.to_string(),
        });
        self.emit(&ThreadEvent::ItemCompleted { item });
    }

    fn on_tool_started(&mut self, call: &ReadyToolCall) {
        let id = self.next_item_id();
        self.active_tool_items
            .insert(call.call_id.clone(), id.clone());
        let item = ThreadItem {
            id,
            details: item_details_for_start(call),
        };
        self.emit(&ThreadEvent::ItemStarted { item });
    }

    fn on_tool_finished(&mut self, call: &ReadyToolCall, outcome: &ToolOutcome, _duration_ms: i64) {
        let id = match self.active_tool_items.remove(&call.call_id) {
            Some(id) => id,
            None => self.next_item_id(),
        };
        let item = ThreadItem {
            id,
            details: item_details_for_finish(call, outcome),
        };
        self.emit(&ThreadEvent::ItemCompleted { item });
    }

    fn on_warning(&mut self, message: &str) {
        self.emit(&ThreadEvent::Error {
            message: message.to_string(),
        });
    }

    fn on_turn_finished(
        &mut self,
        summary: &TurnSummary,
        final_message: Option<&str>,
    ) -> io::Result<()> {
        if let Some(path) = self.last_message_file.as_deref() {
            write_last_message(path, final_message.unwrap_or_default())?;
        }
        if let Some(text) = final_message
            && self.last_emitted_text.as_deref() != Some(text)
        {
            let item = self.next_item(super::events::ThreadItemDetails::AgentMessage {
                text: text.to_string(),
            });
            self.emit(&ThreadEvent::ItemCompleted { item });
        }
        let event = match &summary.outcome {
            TurnOutcome::Complete => ThreadEvent::TurnCompleted {
                usage: ExecUsage::from(summary.billing),
            },
            TurnOutcome::Interrupted => ThreadEvent::TurnFailed {
                error: ThreadError {
                    message: "turn interrupted".to_string(),
                },
            },
            TurnOutcome::Failed { code } => ThreadEvent::TurnFailed {
                error: ThreadError {
                    message: format!("turn failed: {code}"),
                },
            },
        };
        self.write_event(&event)
    }
}

/// `tokens used: 2,140 (in 1,980 · cached 1,024 · out 160)`。
fn usage_line(usage: &crate::db::chat::Usage) -> String {
    let usage = ExecUsage::from(*usage);
    format!(
        "tokens used: {} (in {} · cached {} · out {})",
        format_count(usage.input_tokens.saturating_add(usage.output_tokens)),
        format_count(usage.input_tokens),
        format_count(usage.cached_input_tokens),
        format_count(usage.output_tokens)
    )
}

/// 千分位分组（CLI 输出可读性；不引入第三方格式化依赖）。
fn format_count(value: i64) -> String {
    let text = value.to_string();
    let (sign, digits) = match text.strip_prefix('-') {
        Some(rest) => ("-", rest),
        None => ("", text.as_str()),
    };
    let mut formatted = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, ch) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index) % 3 == 0 {
            formatted.push(',');
        }
        formatted.push(ch);
    }
    format!("{sign}{formatted}")
}

/// 原子的 `-o` 落盘：必要时创建父目录。
fn write_last_message(path: &Path, text: &str) -> io::Result<()> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)?;
    }
    crate::platform::write_file_atomic(path, text.as_bytes())
}
