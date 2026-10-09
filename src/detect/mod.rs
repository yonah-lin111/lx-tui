//! Agent 探测：前台进程名定身份、终端尾部文本定状态；纯算法、无 IO、无终端依赖。

/// 支持识别的 Agent 类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AgentKind {
    Claude,
    Codex,
    Gemini,
    Antigravity,
    OpenCode,
    Cursor,
    Pi,
    Kimi,
}

/// Agent 运行状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentState {
    /// 空闲就绪（提示符可见，等待用户输入）。
    Idle,
    /// 运行中（模型生成、执行工具或命令）。
    Working,
    /// 阻塞卡点（等待权限确认、工具调用或选项回答）。
    Blocked,
    /// 普通 Shell / 未识别程序。
    Unknown,
}

/// 窗格内的 Agent 状态快照。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneAgentSnapshot {
    pub kind: AgentKind,
    pub state: AgentState,
    /// 终端 OSC 标题（Agent 自行设置）；无标题为 None。
    pub title: Option<String>,
}

/// 状态仲裁扫描的尾部行数上限：Agent 状态提示总在屏幕尾部。
pub const STATE_TAIL_LINES: usize = 24;

/// 进程名识别 Agent 身份：按基名匹配，忽略大小写与脚本包装扩展名。
pub fn identify_agent(process_name: &str) -> Option<AgentKind> {
    match normalized_process_name(process_name).as_str() {
        "claude" | "claude-code" => Some(AgentKind::Claude),
        "codex" => Some(AgentKind::Codex),
        "gemini" | "gemini-cli" => Some(AgentKind::Gemini),
        "agy" | "antigravity" | "antigravity-cli" => Some(AgentKind::Antigravity),
        "opencode" | "opencode2" | "open-code" => Some(AgentKind::OpenCode),
        "cursor" | "cursor-agent" => Some(AgentKind::Cursor),
        "pi" | "pi-coding-agent" => Some(AgentKind::Pi),
        "kimi" | "kimi-code" => Some(AgentKind::Kimi),
        _ => None,
    }
}

/// 由前台进程名与终端尾部行构造快照；进程不是已知 Agent 时返回 None。
pub fn pane_agent_snapshot(
    process_name: &str,
    lines: &[String],
    title: Option<&str>,
) -> Option<PaneAgentSnapshot> {
    let kind = identify_agent(process_name)?;
    Some(PaneAgentSnapshot {
        kind,
        state: arbitrate_state(kind, lines),
        title: title.map(str::to_string),
    })
}

/// 由终端尾部行仲裁 Agent 状态：阻塞优先于运行，两者都不明显时为空闲。
pub fn arbitrate_state(kind: AgentKind, lines: &[String]) -> AgentState {
    let tail = Tail::new(lines);
    if tail.blocked(kind) {
        AgentState::Blocked
    } else if tail.working(kind) {
        AgentState::Working
    } else {
        AgentState::Idle
    }
}

/// 进程名归一化：取路径基名、去脚本文本扩展名、转小写。
fn normalized_process_name(process_name: &str) -> String {
    let basename = process_name
        .rsplit(['/', '\\'])
        .find(|part| !part.is_empty())
        .unwrap_or(process_name);
    let mut name = basename.trim().to_lowercase();
    for suffix in [".exe", ".cmd", ".bat", ".ps1", ".js", ".mjs", ".cjs"] {
        if let Some(stripped) = name.strip_suffix(suffix) {
            name = stripped.to_string();
            break;
        }
    }
    name
}

/// 尾部行视图：小写聚合文本用于 contains 匹配，行扫描用于形态匹配。
struct Tail<'a> {
    lines: &'a [String],
    lower: String,
}

impl<'a> Tail<'a> {
    fn new(lines: &'a [String]) -> Self {
        Self {
            lines,
            lower: lines.join("\n").to_lowercase(),
        }
    }

    fn has(&self, needle: &str) -> bool {
        self.lower.contains(needle)
    }

    fn any_has(&self, needles: &[&str]) -> bool {
        needles.iter().any(|needle| self.has(needle))
    }

    fn has_all(&self, needles: &[&str]) -> bool {
        needles.iter().all(|needle| self.has(needle))
    }

    /// 阻塞卡点提示。
    fn blocked(&self, kind: AgentKind) -> bool {
        match kind {
            AgentKind::Claude => self.any_has(&[
                "esc to cancel",
                "waiting for permission",
                "do you want to proceed",
            ]),
            AgentKind::Codex => self.any_has(&[
                "press enter to confirm",
                "enter to submit",
                "[y/n]",
                "action required",
                "do you want to",
            ]),
            AgentKind::Gemini => self.any_has(&[
                "│ apply this change",
                "│ allow execution",
                "waiting for user confirmation",
                "do you want to proceed",
            ]),
            AgentKind::Antigravity => self.has("requesting permission for:"),
            AgentKind::OpenCode => {
                self.has("permission required")
                    || self.has("esc dismiss")
                        && self.any_has(&["enter confirm", "enter submit", "enter toggle"])
            }
            AgentKind::Cursor => {
                self.has("write to this file?") && self.has("proceed (y)")
                    || self.any_has(&["waiting for approval", "(y) (enter)", "skip (esc or n)"])
            }
            AgentKind::Pi => false,
            AgentKind::Kimi => {
                self.has("↵ confirm")
                    || self.has_all(&["↑↓ select", "esc cancel"])
                    || self.has_all(&["requesting approval", "reject"])
            }
        }
    }

    /// 运行中提示。
    fn working(&self, kind: AgentKind) -> bool {
        match kind {
            AgentKind::Claude => {
                self.any_has(&[
                    "esc to interrupt",
                    "mcp tasks still running",
                    "background agent",
                ]) || self.spinner_with_gerund()
            }
            AgentKind::Codex => {
                self.any_has(&["esc to interrupt", "working ("]) || self.spinner_line()
            }
            AgentKind::Gemini => self.has("esc to cancel"),
            AgentKind::Antigravity => self.spinner_with_gerund(),
            AgentKind::OpenCode => self.has("interrupt") || self.any_has(&["■■■■", "⬝⬝⬝⬝"]),
            AgentKind::Cursor => {
                self.any_has(&["ctrl+c to stop", "background task"]) || self.spinner_with_gerund()
            }
            AgentKind::Pi => self.has("working..."),
            AgentKind::Kimi => {
                self.has("thinking") && self.any_has(&["...", "…", "using "])
                    || self.moon_line()
                    || self.spinner_with_gerund()
            }
        }
    }

    /// 任意行的首个非空白字符为转轮符。
    fn spinner_line(&self) -> bool {
        self.lines
            .iter()
            .any(|line| line.trim_start().chars().next().is_some_and(is_spinner))
    }

    /// 转轮符开头且行内出现 -ing 动词（如 `⠋ Generating…`）。
    fn spinner_with_gerund(&self) -> bool {
        self.lines.iter().any(|line| {
            let mut chars = line.trim_start().chars();
            chars.next().is_some_and(is_spinner)
                && chars.as_str().split_whitespace().any(|word| {
                    let word = word.trim_end_matches(|ch: char| !ch.is_alphanumeric());
                    word.len() > 3 && word.ends_with("ing")
                })
        })
    }

    /// 单行整行只有一个月亮转轮符（Kimi）。
    fn moon_line(&self) -> bool {
        self.lines.iter().any(|line| {
            let mut chars = line.trim().chars();
            matches!(chars.next(), Some(ch) if is_moon(ch)) && chars.next().is_none()
        })
    }
}

/// 转轮符：盲文点阵、Claude 半圆与 Cursor 六边形。
fn is_spinner(ch: char) -> bool {
    matches!(ch, '\u{2800}'..='\u{28FF}' | '\u{25D0}'..='\u{25D3}' | '⬡' | '⬢')
}

/// Kimi 月亮转轮符。
fn is_moon(ch: char) -> bool {
    matches!(ch, '🌕' | '🌖' | '🌗' | '🌘' | '🌑' | '🌒' | '🌓' | '🌔')
}

#[cfg(test)]
mod tests;
