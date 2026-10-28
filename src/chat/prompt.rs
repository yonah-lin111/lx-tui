//! `lx` agent 身份提示与环境快照：唯一的稳定系统指令来源。
//!
//! 这里只有纯函数与纯字符串常量，不依赖协议或 TUI 状态。四类 transport 在装配请求时
//! 把提示编码成各自的 system/developer 字段（OpenAI Chat 的 `messages[0].role=system`、
//! Responses 的 `instructions`、Anthropic 的顶层 `system`、Google 的
//! `systemInstruction`）。文本只描述 agent 名、真实可用的工具、相对路径基准、
//! 执行/回报规则与**本轮冻结**的环境快照，不嵌入任何第三方品牌或整份系统指令。
//!
//! 环境快照的边界（对应 design §2.2）：
//! - 只包含工作区/工作目录、OS/架构、终端与 shell、本地日期时间与时区、网络状态、
//!   受管控后台任务摘要与文件系统能力说明；
//! - **绝不**包含环境变量值、令牌、凭据、用户目录内容或完整命令行；
//! - 所有值先剥离控制字符、再做 XML 转义，并按字段与整段长度上限截断；
//! - 每个 turn 创建时生成一次并冻结，后续采样复用同一份（[`EnvironmentSnapshot`]）。

use std::path::Path;

use crate::platform::EnvironmentFacts;

/// `lx` agent 的稳定名称。**不可变**：不是配置项。
///
/// 身份名的唯一来源，系统提示首行由它生成。不得由环境变量、命令行参数、设置页
/// 或模型输出改写；改名等于换 agent，必须同时递增 [`PROMPT_VERSION`] 并更新提示
/// 正文、测试与文档（提示缓存与日志都按这个名字区分身份）。
pub const AGENT_NAME: &str = "lx";

/// 系统提示词版本：提示词语义变化时必须递增（测试与日志据此判定版本）。
pub const PROMPT_VERSION: &str = "lx-5";

/// 环境快照单字段最大字符数（转义前）。
const MAX_FIELD_CHARS: usize = 240;

/// 环境快照整段最大字节数（转义后）；超限按行丢弃尾部并显式标记。
pub const MAX_ENVIRONMENT_BYTES: usize = 4096;

/// 后台任务摘要最多列出的条数。
const MAX_BACKGROUND_TASKS: usize = 8;

/// `lx` 的稳定系统提示（英文：与工具 schema/描述保持同一语言）。
///
/// 章节结构对齐 codex `base_instructions/default.md`（personality / language /
/// AGENTS.md / responsiveness / planning / task execution / validation /
/// ambition vs precision / progress updates / final message / tools / workspace），
/// 内容只保留 lx 真正具备的能力：
/// - 八个工具；没有沙箱、审批、MCP、图片或后台审批线程，提示不得承诺这些；
/// - 相对路径以工作区根（发送时冻结的 cwd）为基准；
/// - “工具结果是唯一事实来源”：先读后改、最小修改、不得伪造结果或错误；
/// - 默认回复语言 = 用户当前消息使用的语言（不预设任何语言）。
pub const LX_SYSTEM_PROMPT: &str = "\
You are lx, a terminal coding agent working inside the user's workspace. You are expected to be concise, precise, and helpful, and to keep the user informed about what you are doing.

# How you work

## Personality

- Default tone: direct and friendly, like a competent teammate handing off work.
- State assumptions and next steps when they matter; skip filler and restating the request.
- Match the depth of your answer to the task: a one-line answer for a one-line question, a structured summary for a large change.

## Language

- Reply in the language the user is currently writing in. This is the default; do not switch languages on your own.
- Only use a specific language when the user explicitly asks for one, and then keep using it.
- Keep code, identifiers, file paths, commands, and tool arguments in their original form; do not translate them.

## Repository instructions (AGENTS.md)

- Repositories often contain `AGENTS.md` files with local instructions; they can appear in any directory.
- An `AGENTS.md` applies to the entire directory tree rooted at its folder; a deeper file wins when two conflict. Check for one with `read` before editing files in a directory.
- Instructions in this conversation take precedence over `AGENTS.md`. Your own habits come last.

## Responsiveness

- Before a group of related tool calls, send a short preamble (one or two sentences) saying what you are about to do; connect it to the previous step when continuing a task.
- Skip the preamble for a single trivial read. Do not narrate every call.
- For long tasks, post brief progress updates at natural checkpoints.

## Planning

- Use `update_plan` when the work is non-trivial, multi-step, or genuinely ambiguous. Keep it to 1-7 short steps, each one sentence.
- Do not use a plan for simple or single-step requests, and never for filler.
- Keep exactly one step `in_progress`; mark a step `completed` only when the work is actually done. When the plan changes, call `update_plan` again and say why.
- Do not repeat the full plan after calling `update_plan`; summarize what changed instead.

## Task execution

- Keep working until the request is resolved. Do not guess or invent an answer when you can check it.
- Read before you change anything: `edit` and `write` reject a file that was not read in this session or that changed on disk after the read. `search` results carry no version token, so searching alone is not enough.
- Prefer the minimal exact `edit` patch over rewriting a whole file, and never reformat or restructure unrelated code.
- New files: make sure the path does not exist yet. If it appeared, read it or pick another path; do not overwrite it blindly.
- Fix the root cause instead of a surface patch. Do not fix unrelated bugs or broken tests; mention them instead.
- Follow the existing style and naming of the code you touch. Do not add license headers, and do not add inline comments unless the code is genuinely non-obvious.
- Never revert or overwrite changes you did not make. Do not run destructive git commands (`git reset --hard`, `git checkout --`, `git clean`) unless the user explicitly asks.
- Do not commit, amend, or create branches unless the user asks.

## Validating your work

- Start with the narrowest check that covers what you changed, then broaden as confidence grows.
- Use the project's own commands when they exist: `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, `cargo test`, or the equivalent for the workspace. Do not add a test framework to a project that has none.
- Say exactly which commands ran and what they returned. A failing test is a real result; report it.
- If you could not verify something, say so plainly instead of implying it works.

## Ambition vs. precision

- For a brand-new project or an open-ended request, be ambitious and make reasonable creative choices.
- Inside an existing codebase, be surgical: do exactly what was asked, keep the diff minimal, and do not rename or restructure things on a whim.

## Presenting your work and final message

- Lead with the outcome, then what changed and why. Be concise; the user can see their own files.
- Use bullets for several points; plain prose for a single answer. Do not pad with headers.
- Wrap file paths, commands, and identifiers in backticks; reference a location as `path:line` when it helps.
- Do not paste large file contents or raw tool payloads back to the user; reference the path instead.
- End with a brief, concrete next step when one is natural (run the app, commit, add tests). State clearly what you could not verify.

# Subagents

Three tools let you hand work to a subagent: `spawn_agent`, `wait_agent`, `close_agent`.
Each subagent starts with a clean context. It sees only its role prompt, the environment
snapshot, and the task you write — not this conversation.

- `explorer` — read-only investigation. It has `read` and `search` and nothing else. Use it
  for specific questions about the codebase: where a flow lives, who calls something, how a
  module is wired.
- `executor` — implementation with a boundary. It has the file and process tools, no `ask`,
  no plan tool, and no ability to spawn agents. Use it for one well-defined change or fix.

Delegate when the work is independent, has a clear boundary, and would otherwise fill your
context with detail you do not need. Do not delegate a single-file change you can make in a
couple of calls, anything that needs the user's input, or work you cannot describe precisely.

Write the task with three things: the goal, the boundary (which files or modules belong to
this agent), and the deliverable. A subagent that has to guess will guess wrong.
Always pass a short human-readable `title` (3–8 words) separate from the task. It is what
the user sees in the subagent card and the Agents list; never paste the task prompt there.

Rules:
- At most 3 subagents may run at once per conversation; a fourth call is rejected.
- Subagents run in the same workspace and can see each other's edits. Always tell each one it
  is not alone in the codebase and must not revert or overwrite changes it did not make.
- You do not have to wait. Start a subagent, keep working, and call `wait_agent` when you need
  the result — or let the report arrive on its own.
- `wait_agent` without `targets` waits for every subagent in this conversation. Use
  `wait_for: \"any\"` when you only need the first result; the default waits for all.
- Prefer long waits (minutes) over short polling. `wait_for: \"all\"` keeps waiting for
  running agents instead of ending the turn early. `wait_for: \"any\"` may still return a
  timeout while other agents run; wait again instead of ending the turn.
- A report is short by design: the conclusion plus what it touched. The full trace stays in its
  own session; do not ask for a replay.
- `close_agent` stops a subagent and produces no report.

# Tools

You can call these tools: `read`, `search`, `edit`, `write`, `exec_command`, `write_stdin`, `update_plan`, `ask`, `spawn_agent`, `wait_agent`, and `close_agent`.
- Every relative path resolves against the workspace root (the session working directory, fixed when the turn started); absolute paths are used as given.
- `read` returns numbered lines plus a `file_version` token; `search` returns sorted path/line matches.
- `edit` applies an exact context patch to one or more files; `write` replaces a whole file.
- `exec_command` runs a shell command without an approval prompt. When the command is still running after the yield window it returns `process_state: running` and a `session_id`; reuse that id with `write_stdin` (empty `chars` reads more output, non-empty writes stdin, `terminate: true` stops it) instead of starting a second copy. `tty: true` uses a dedicated pseudo terminal.
- A non-zero exit code means the command ran and failed: report it, do not pretend it succeeded.
- `update_plan` replaces the whole plan; keep at most one step `in_progress` and mark steps `completed` only when they really are.

## Shell commands

- Prefer the dedicated file tools over shell equivalents: `read` instead of `cat`, `search` instead of `grep`/`rg`, `edit`/`write` instead of `sed` or shell redirection.
- Keep commands focused; do not chain unrelated work into one call, and do not use a shell loop to dump many files.
- Do not assume Unix tools exist. Probe with `command -v <tool>` and respect the platform in the environment block (for example macOS has no `setsid`).

## Asking the user (`ask`)

- `ask` blocks the turn until the user answers; the answer arrives as the tool result.
- Use `ask` only when a user preference really decides the outcome and the answer cannot be discovered with `read`, `search`, or `exec_command`. Prefer acting on a reasonable assumption over stopping to ask. Never use it for permission requests, and never write a multiple-choice question as a plain assistant message.
- Ask 1-3 questions, each with at most 4 options; put the recommended option first and give every option a one-sentence tradeoff. Do not add an \"Other\" option: free-form input is always available. Set `multi_select: true` only when several choices can coexist.
- When the choice is visual (layout, UI, diagram), put a plain-text monospace sketch in `preview`. It is rendered verbatim: keep alignment with spaces and do not use Markdown.

# Truthfulness

Tool results are the only source of truth:
- Never claim a file, command, test, or commit succeeded unless a tool reported it. Never invent output, exit codes, error messages, or file contents.
- When a tool fails, report the real failure and act on it; do not silently retry the same call or paraphrase the failure as success.
- Verify with real commands when the task requires working code, and say exactly which commands ran and what they returned.

# Workspace and platform

- Stay inside the workspace unless the user explicitly asks for another location. Do not touch dotfiles, credentials, or unrelated repositories.
- Keep tool calls focused: one purpose per call, and reply with concise prose; do not paste large file contents or raw tool payloads back to the user.";

/// 后台任务的语义说明：追加在稳定提示之后的环境相关段落。
///
/// 单列一段（而不是写进常量）的原因：它要按**当前平台**给出具体命令能力，
/// 而常量必须保持平台无关。
///
/// 这一段的存在理由是一条真实故障：模型在 macOS 上写出 `setsid bash -c '...' &`
/// 来起常驻任务。macOS 没有 `setsid`，命令静默失败而 shell 仍返回 0，于是模型
/// 以为任务在跑、用户却看到它「启动后马上退出」。
const LX_BACKGROUND_NOTE: &str = "\
Background and long-running work:
- `exec_command` only ends one wait window; it does not kill a process that is still running. To keep something alive across turns, start it once and drive it with `write_stdin`.
- A shell that exits stops its background children unless they are detached. `cmd &` alone does not survive the shell exiting under `tty: true`, because the pseudo terminal sends SIGHUP to the job. Detach explicitly when it must outlive the call: `nohup cmd >/dev/null 2>&1 &` works everywhere; `setsid` does not exist on macOS.
- Verify a background start instead of assuming it worked: check `pgrep -f <pattern>` or the log file in a separate call. A shell returning 0 does not mean the background job started.
- Do not reach for tools that may be missing; probe with `command -v <tool>` when unsure.";

/// 一个受管控后台任务的最小摘要（进环境快照，不整段命令行）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackgroundTask {
    /// 模型可复用的进程会话 ID。
    pub session_id: u64,
    /// 稳定状态值（`running` / `starting` / …）。
    pub state: String,
    /// 工作目录（已脱敏、截断）。
    pub cwd: String,
    /// 命令首行（截断；不含后续输出）。
    pub command: String,
}

/// 一个 turn 冻结的环境快照。
///
/// 创建一次后在本轮所有采样复用：环境事实（尤其是网络与后台任务）在同一轮内
/// 不得变化，否则模型会在两次采样间看到互相矛盾的世界。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvironmentSnapshot {
    /// 渲染后的 `<environment_context>` 文本（已转义、已限长）。
    text: String,
}

impl EnvironmentSnapshot {
    /// 渲染快照；纯函数，输入全部由调用方提供。
    pub fn render(
        facts: &EnvironmentFacts,
        workspace: Option<&Path>,
        cwd: Option<&Path>,
        background: &[BackgroundTask],
    ) -> Self {
        let mut lines: Vec<String> = Vec::new();
        lines.push(format!(
            "<environment_context version=\"{PROMPT_VERSION}\">"
        ));
        lines.push(format!("<os>{}</os>", escape_field(&facts.os)));
        lines.push(format!("<arch>{}</arch>", escape_field(&facts.arch)));
        lines.push(format!("<shell>{}</shell>", escape_field(&facts.shell)));
        if let Some(terminal) = &facts.terminal {
            lines.push(format!("<terminal>{}</terminal>", escape_field(terminal)));
        }
        let mut multiplexer = Vec::new();
        if facts.tmux {
            multiplexer.push("tmux");
        }
        if facts.screen {
            multiplexer.push("screen");
        }
        if !multiplexer.is_empty() {
            lines.push(format!(
                "<terminal_multiplexer>{}</terminal_multiplexer>",
                multiplexer.join(",")
            ));
        }
        lines.push(format!("<date>{}</date>", escape_field(&facts.date)));
        lines.push(format!("<time>{}</time>", escape_field(&facts.time)));
        if let Some(timezone) = &facts.timezone {
            lines.push(format!("<timezone>{}</timezone>", escape_field(timezone)));
        }
        // 无法判定时明确写 unknown，不猜“可用”。
        lines.push(format!(
            "<network>{}</network>",
            match facts.network {
                Some(true) => "available",
                Some(false) => "unavailable",
                None => "unknown",
            }
        ));
        if let Some(workspace) = workspace {
            lines.push(format!(
                "<workspace_root>{}</workspace_root>",
                escape_field(&workspace.display().to_string())
            ));
        }
        if let Some(cwd) = cwd {
            lines.push(format!(
                "<cwd>{}</cwd>",
                escape_field(&cwd.display().to_string())
            ));
        }
        lines.push(
            "<filesystem>file tools use UTF-8 text; shell commands are not sandboxed</filesystem>"
                .to_string(),
        );
        if background.is_empty() {
            lines.push("<background_tasks>none</background_tasks>".to_string());
        } else {
            lines.push("<background_tasks>".to_string());
            for task in background.iter().take(MAX_BACKGROUND_TASKS) {
                lines.push(format!(
                    "  <task session_id=\"{}\" state=\"{}\"><cwd>{}</cwd><command>{}</command></task>",
                    task.session_id,
                    escape_field(&task.state),
                    escape_field(&task.cwd),
                    escape_field(&task.command)
                ));
            }
            if background.len() > MAX_BACKGROUND_TASKS {
                lines.push(format!(
                    "  <truncated omitted=\"{}\"/>",
                    background.len() - MAX_BACKGROUND_TASKS
                ));
            }
            lines.push("</background_tasks>".to_string());
        }
        lines.push("</environment_context>".to_string());
        let mut text = lines.join("\n");
        if text.len() > MAX_ENVIRONMENT_BYTES {
            // 按行丢弃尾部，并补回闭合标签：模型不应看到半个字段或未闭合的 XML。
            const SUFFIX: &str = "\n<!-- environment truncated -->\n</environment_context>";
            let budget = MAX_ENVIRONMENT_BYTES.saturating_sub(SUFFIX.len());
            let mut end = budget.min(text.len());
            while end > 0 && !text.is_char_boundary(end) {
                end -= 1;
            }
            if let Some(last_newline) = text[..end].rfind('\n') {
                end = last_newline;
            }
            text.truncate(end);
            text.push_str(SUFFIX);
        }
        Self { text }
    }

    /// 渲染后的快照文本。
    pub fn text(&self) -> &str {
        &self.text
    }
}

/// 纯函数：按 turn 冻结的输入渲染环境快照。
pub fn environment_snapshot(
    facts: &EnvironmentFacts,
    workspace: Option<&Path>,
    cwd: Option<&Path>,
    background: &[BackgroundTask],
) -> String {
    EnvironmentSnapshot::render(facts, workspace, cwd, background).text
}

/// 剥离子控制字符 + XML 转义 + 截断。
///
/// 环境字段不得伪造结构：`<`/`>`/`&` 一律转义，控制字符（含 ANSI 转义）直接丢弃，
/// 换行折成空格，超长按字符截断并留 `…`。
fn escape_field(text: &str) -> String {
    let mut out = String::with_capacity(text.len().min(MAX_FIELD_CHARS));
    for ch in text.chars() {
        if out.chars().count() >= MAX_FIELD_CHARS {
            out.push('…');
            break;
        }
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            '\n' | '\r' | '\t' => out.push(' '),
            ch if ch.is_control() => {}
            ch => out.push(ch),
        }
    }
    out
}

/// 组装完整系统提示：稳定身份/工具段 + 后台任务语义 +（可选）冻结环境快照 +
/// （可选）compact 摘要。
///
/// 顺序对齐 design §3.3：`system prompt → environment snapshot → compact summary`。
/// `environment` 为本 turn 冻结的快照；None 时现场采集一次（测试与旧调用路径），
/// 但生产路径必须传入冻结值，避免同一轮两次采样看到不同的环境。
/// `compact` 为当前生效检查点的结构化摘要；None 时不注入任何压缩段落。
pub fn compose_system_prompt(environment: Option<&str>, compact: Option<&str>) -> String {
    let environment = environment.map(str::to_string).unwrap_or_else(|| {
        let facts = EnvironmentFacts::capture();
        environment_snapshot(&facts, None, None, &[])
    });
    let mut prompt = format!("{LX_SYSTEM_PROMPT}\n\n{environment}\n\n{LX_BACKGROUND_NOTE}");
    if let Some(compact) = compact.filter(|text| !text.trim().is_empty()) {
        prompt.push_str("\n\n");
        prompt.push_str(compact);
    }
    prompt
}

/// 兼容入口：不携带 compact 摘要时组装提示。
pub fn system_prompt_with(environment: Option<&str>) -> String {
    compose_system_prompt(environment, None)
}

/// 兼容入口：不携带 turn 上下文时现场采集环境快照。
pub fn system_prompt() -> String {
    compose_system_prompt(None, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts() -> EnvironmentFacts {
        EnvironmentFacts {
            os: "macOS".into(),
            arch: "aarch64".into(),
            shell: "/bin/zsh".into(),
            terminal: Some("xterm-256color".into()),
            tmux: false,
            screen: false,
            date: "2026-10-24".into(),
            time: "10:42".into(),
            timezone: Some("UTC+08:00".into()),
            network: Some(true),
        }
    }

    /// agent 名字是 `AGENT_NAME` 这一个来源，提示首行必须与它逐字一致。
    ///
    /// 身份名不可变、不可配置：这里把它和提示正文锁在一起，防止改名时只改一半
    /// （提示说 A、日志/缓存按 B 区分）。
    #[test]
    fn agent_name_is_wired_into_the_prompt() {
        assert_eq!(AGENT_NAME, "lx");
        assert!(
            LX_SYSTEM_PROMPT.starts_with(&format!("You are {AGENT_NAME},")),
            "提示首行必须由 AGENT_NAME 生成：{:.60}",
            LX_SYSTEM_PROMPT
        );
    }

    /// 子 agent 一节必须承诺真实存在的三个工具，且不引入与「自由活动」冲突的条款。
    #[test]
    fn prompt_documents_subagents_without_codex_conflicts() {
        for needle in [
            "# Subagents",
            "`spawn_agent`",
            "`wait_agent`",
            "`close_agent`",
            "`explorer`",
            "`executor`",
            "At most 3 subagents",
        ] {
            assert!(
                LX_SYSTEM_PROMPT.contains(needle),
                "提示缺少子 agent 规则 `{needle}`"
            );
        }
        for forbidden in [
            "wait for them before yielding",
            "only role becomes to coordinate",
        ] {
            assert!(
                !LX_SYSTEM_PROMPT.contains(forbidden),
                "不得引入冲突条款 `{forbidden}`"
            );
        }
        for name in crate::chat::tools::SUBAGENT_TOOL_NAMES {
            assert!(LX_SYSTEM_PROMPT.contains(name), "工具清单必须含 {name}");
        }
    }

    /// 提示只包含 lx 自己的身份与真实可用工具；不得出现第三方品牌或整份系统指令。
    #[test]
    fn prompt_is_lx_specific_and_tool_scoped() {
        assert!(LX_SYSTEM_PROMPT.contains("You are lx"));
        for tool in crate::chat::tools::TOOL_NAMES {
            assert!(LX_SYSTEM_PROMPT.contains(tool), "提示必须提到工具 {tool}");
        }
        assert!(LX_SYSTEM_PROMPT.contains("workspace root"));
        assert!(!LX_SYSTEM_PROMPT.contains("Codex"));
        assert!(!LX_SYSTEM_PROMPT.contains("OpenAI"));
        assert!(!LX_SYSTEM_PROMPT.contains("Anthropic"));
    }

    /// `ask` 的使用纪律必须写进提示：它阻塞整轮、只问偏好类问题、绝不要权限、
    /// 绝不用纯文本替代选择题，并说明 preview 是逐字渲染的等宽草图。
    #[test]
    fn prompt_documents_the_ask_tool() {
        for needle in [
            "blocks the turn",
            "permission",
            "1-3 questions",
            "`preview`",
        ] {
            assert!(
                LX_SYSTEM_PROMPT.contains(needle),
                "提示缺少 ask 规则 `{needle}`"
            );
        }
        assert_eq!(PROMPT_VERSION, "lx-5");
        assert!(
            LX_SYSTEM_PROMPT.contains("You can call these tools:")
                && LX_SYSTEM_PROMPT.contains("`ask`"),
            "工具清单必须含 ask"
        );
    }

    /// 防御性规则必须写进提示：先读后改、最小修改、结果真实、错误不可伪造、验证命令、
    /// 工作区边界。
    #[test]
    fn prompt_states_defensive_rules() {
        for needle in [
            "Read before you change anything",
            "minimal exact `edit` patch",
            "Tool results are the only source of truth",
            "Never invent output",
            "cargo test",
            "Stay inside the workspace",
            "command -v",
        ] {
            assert!(
                LX_SYSTEM_PROMPT.contains(needle),
                "提示缺少防御规则 `{needle}`"
            );
        }
    }

    /// 默认语言规则必须写进提示：跟随用户当前输入的语言，不自作主张切换，
    /// 只有用户明确要求时才改用指定语言。
    #[test]
    fn prompt_defaults_to_the_users_language() {
        for needle in [
            "Reply in the language the user is currently writing in",
            "do not switch languages on your own",
            "user explicitly asks for one",
        ] {
            assert!(
                LX_SYSTEM_PROMPT.contains(needle),
                "提示缺少语言规则 `{needle}`"
            );
        }
        // 不能反过来把某一种语言钉死成默认值。
        for forced in ["Always reply in English", "Answer in English by default"] {
            assert!(
                !LX_SYSTEM_PROMPT.contains(forced),
                "提示不得强制单一语言：`{forced}`"
            );
        }
    }

    /// 章节结构对齐 codex `base_instructions`：缺少这些段落就意味着跑偏了。
    #[test]
    fn prompt_mirrors_the_codex_section_structure() {
        for heading in [
            "# How you work",
            "## Personality",
            "## Language",
            "## Repository instructions (AGENTS.md)",
            "## Planning",
            "## Task execution",
            "## Validating your work",
            "## Ambition vs. precision",
            "## Presenting your work and final message",
            "# Tools",
            "# Workspace and platform",
        ] {
            assert!(
                LX_SYSTEM_PROMPT.contains(heading),
                "提示缺少章节 `{heading}`"
            );
        }
    }

    /// 模型可见文本一律英文（项目规范）：出现中文即视为回归。
    ///
    /// 覆盖系统提示、后台任务说明、压缩提示，以及全部工具名/描述/参数 schema。
    #[test]
    fn model_facing_prompts_are_english_only() {
        fn has_cjk(text: &str) -> bool {
            text.chars()
                .any(|ch| ('\u{4e00}'..='\u{9fff}').contains(&ch))
        }

        for text in [
            LX_SYSTEM_PROMPT,
            LX_BACKGROUND_NOTE,
            crate::chat::compact::COMPACT_SYSTEM_PROMPT,
        ] {
            assert!(!has_cjk(text), "模型提示词必须是英文：{text:.80}");
        }
        for spec in crate::chat::tools::available_tools() {
            assert!(
                !has_cjk(spec.description),
                "工具 {} 的描述含中文",
                spec.name
            );
            let parameters = serde_json::to_string(&spec.parameters()).unwrap_or_default();
            assert!(!has_cjk(&parameters), "工具 {} 的 schema 含中文", spec.name);
        }
    }

    /// 提示长度受控：它是每次请求都发送的固定开销，不能悄悄膨胀。
    ///
    /// 上限对齐 codex 的结构化提示（约 21 KB），但 lx 只保留自身能力，
    /// 预算压到 12 KiB；再加环境快照与后台任务说明仍不超过 16 KiB。
    #[test]
    fn prompt_stays_small() {
        assert!(
            LX_SYSTEM_PROMPT.len() < 12_288,
            "系统提示过大：{} 字节",
            LX_SYSTEM_PROMPT.len()
        );
        assert!(
            system_prompt().len() < 16_384,
            "完整系统提示过大：{} 字节",
            system_prompt().len()
        );
    }

    /// 组装后的提示必须带上当前平台，否则模型会假设 Linux 工具齐备。
    ///
    /// 这条对应真实故障：模型在 macOS 上写出 `setsid ... &` 起常驻任务，
    /// 而 macOS 没有 `setsid`，命令静默失败但 shell 返回 0。
    #[test]
    fn system_prompt_reports_the_platform() {
        let prompt = system_prompt();
        assert!(
            prompt.contains("<environment_context"),
            "缺少环境段：{prompt}"
        );
        assert!(prompt.contains(crate::platform::os_name()));
        // 稳定的身份/工具段必须原样保留。
        assert!(prompt.starts_with(LX_SYSTEM_PROMPT));
    }

    /// 冻结的快照必须原样进入提示，且同一份快照多次组装结果一致。
    #[test]
    fn frozen_snapshot_is_reused_verbatim() {
        let snapshot = environment_snapshot(&facts(), None, None, &[]);
        let first = system_prompt_with(Some(&snapshot));
        let second = system_prompt_with(Some(&snapshot));
        assert_eq!(first, second);
        assert!(first.contains(&snapshot));
    }

    /// 后台任务的坑必须写进提示：`yield_time_ms` 不杀进程、
    /// `tty: true` 下裸 `&` 会被 SIGHUP、启动后要主动验证。
    #[test]
    fn system_prompt_warns_about_background_jobs() {
        let prompt = system_prompt();
        for needle in [
            "does not kill a process",
            "SIGHUP",
            "nohup",
            "setsid",
            "pgrep",
            "command -v",
        ] {
            assert!(
                prompt.contains(needle),
                "后台任务说明缺少 `{needle}`：{prompt}"
            );
        }
    }

    /// 环境快照覆盖约定字段，并明确不可用字段与受管控后台任务。
    #[test]
    fn snapshot_covers_environment_fields() {
        let tasks = vec![BackgroundTask {
            session_id: 7,
            state: "running".into(),
            cwd: "/work".into(),
            command: "npm run dev".into(),
        }];
        let snapshot = environment_snapshot(
            &facts(),
            Some(Path::new("/work/repo")),
            Some(Path::new("/work/repo/src")),
            &tasks,
        );
        for needle in [
            "<os>macOS</os>",
            "<arch>aarch64</arch>",
            "<shell>/bin/zsh</shell>",
            "<terminal>xterm-256color</terminal>",
            "<date>2026-10-24</date>",
            "<time>10:42</time>",
            "<timezone>UTC+08:00</timezone>",
            "<network>available</network>",
            "<workspace_root>/work/repo</workspace_root>",
            "<cwd>/work/repo/src</cwd>",
            "<background_tasks>",
            "session_id=\"7\"",
            "npm run dev",
        ] {
            assert!(snapshot.contains(needle), "快照缺少 `{needle}`：{snapshot}");
        }
    }

    /// 不可用字段必须显式写 unknown；未知终端不渲染终端字段。
    #[test]
    fn snapshot_marks_unavailable_fields() {
        let mut facts = facts();
        facts.network = None;
        facts.terminal = None;
        facts.timezone = None;
        let snapshot = environment_snapshot(&facts, None, None, &[]);
        assert!(snapshot.contains("<network>unknown</network>"));
        assert!(!snapshot.contains("<terminal>"));
        assert!(!snapshot.contains("<timezone>"));
        assert!(snapshot.contains("<background_tasks>none</background_tasks>"));
    }

    /// XML 与控制字符必须转义：字段不得伪造结构，也不能把 ANSI 带进提示。
    #[test]
    fn snapshot_escapes_xml_and_strips_control_chars() {
        let mut facts = facts();
        facts.shell = "/bin/sh</shell><injected>".into();
        facts.os = "line\nbreak\u{1b}[31mred".into();
        let snapshot = environment_snapshot(&facts, None, None, &[]);
        assert!(!snapshot.contains("<injected>"), "不得伪造结构：{snapshot}");
        assert!(snapshot.contains("&lt;/shell&gt;&lt;injected&gt;"));
        assert!(!snapshot.contains('\u{1b}'));
        assert!(!snapshot.contains("line\nbreak"));
    }

    /// 超长字段与超长快照都必须有界，且显式标记截断。
    #[test]
    fn snapshot_is_length_bounded() {
        let mut facts = facts();
        facts.shell = "s".repeat(10_000);
        let long_tasks: Vec<BackgroundTask> = (0..20)
            .map(|index| BackgroundTask {
                session_id: index,
                state: "running".into(),
                cwd: "/work".into(),
                command: "x".repeat(500),
            })
            .collect();
        let snapshot = environment_snapshot(&facts, None, None, &long_tasks);
        assert!(
            snapshot.len() <= MAX_ENVIRONMENT_BYTES + 64,
            "快照超长：{} 字节",
            snapshot.len()
        );
        assert!(snapshot.contains("<truncated omitted=\"12\"/>"));
        assert!(snapshot.contains('…'));
    }
}
