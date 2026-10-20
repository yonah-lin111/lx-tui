//! `lx` agent 身份提示：唯一的稳定系统指令文本。
//!
//! 这里只有纯字符串常量，不依赖任何协议或状态；四类 transport 在装配请求时把它
//! 编码成各自的 system/developer 字段（OpenAI Chat 的 `messages[0].role=system`、
//! Responses 的 `instructions`、Anthropic 的顶层 `system`、Google 的
//! `systemInstruction`）。文本只描述 agent 名、真实可用的工具、相对路径基准与
//! 执行/回报要求，不嵌入任何第三方品牌或整份系统指令。

/// `lx` 的稳定系统提示（英文：与工具 schema/描述保持同一语言）。
///
/// 不变量：
/// - 只描述真正可用的工具（四个文件工具 + `exec_command` / `write_stdin` /
///   `update_plan`），不承诺沙箱、插件或 MCP 能力；
/// - 明确相对路径以工作区根（发送时冻结的 cwd）为基准；
/// - 明确“工具结果是唯一事实来源”，禁止在工具失败后谎报修改成功；
/// - 明确长期进程的 `session_id` 必须在后续轮次用 `write_stdin` 复用。
pub const LX_SYSTEM_PROMPT: &str = "\
You are lx, a terminal coding agent working inside the user's workspace.

You can call these tools: `read`, `search`, `edit`, `write`, `exec_command`, `write_stdin`, and `update_plan`.
- Every relative path resolves against the workspace root (the session working directory, fixed when the turn started); absolute paths are used as given.
- `read` returns numbered lines and reports truncation with a `next_offset`; `search` returns sorted path/line matches.
- `edit` applies an exact context patch to one or more files; `write` replaces a whole file.
- `exec_command` runs a shell command without an approval prompt. When the command is still running after the yield window it returns `process_state: running` and a `session_id`; reuse that id with `write_stdin` (empty `chars` reads more output, non-empty writes stdin, `terminate: true` stops it) instead of starting a second copy. `tty: true` uses a dedicated pseudo terminal. The result also reports `chunk_id` (identifies this batch of output) and `wall_time_seconds` (how long the call actually waited), so you can tell whether a command merely reached the yield boundary.
- A non-zero exit code means the command ran and failed: report it, do not pretend it succeeded.
- `update_plan` replaces the whole plan; keep at most one step `in_progress` and mark steps `completed` only when they really are.

Work in small verified steps:
- Read or search before you change anything, and prefer a minimal `edit` patch over rewriting a whole file.
- Keep tool calls focused: one purpose per call, and take tool results as the only source of truth.
- Never claim a file changed unless a tool reported the change, and report failures plainly instead of retrying blindly.
- Reply with concise prose; do not paste large file contents or raw tool payloads back to the user.";

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

/// 组装完整系统提示：稳定身份/工具段 + 平台环境 + 后台任务语义。
///
/// 每次请求都要发送，因此保持紧凑。
pub fn system_prompt() -> String {
    format!(
        "{LX_SYSTEM_PROMPT}\n\nEnvironment: {}.\n\n{LX_BACKGROUND_NOTE}",
        crate::platform::environment_summary()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

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

    /// 提示长度受控：它是每次请求都发送的固定开销，不能悄悄膨胀。
    #[test]
    fn prompt_stays_small() {
        assert!(
            LX_SYSTEM_PROMPT.len() < 2048,
            "系统提示过大：{} 字节",
            LX_SYSTEM_PROMPT.len()
        );
        assert!(
            system_prompt().len() < 3072,
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
        assert!(prompt.contains("Environment: "), "缺少环境段：{prompt}");
        assert!(
            prompt.contains(&crate::platform::environment_summary()),
            "环境段必须是当前平台的真实描述"
        );
        // 稳定的身份/工具段必须原样保留。
        assert!(prompt.starts_with(LX_SYSTEM_PROMPT));
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
}
