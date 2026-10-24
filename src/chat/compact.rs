//! `/compact` 的纯逻辑：增量范围、摘要提示词、上下文投影与长度校验。
//!
//! 本模块**不做 IO**：输入是已加载的消息记录，输出是请求骨架或视图数据。DB 事务、
//! 网络请求与状态刷新分别由 `db::chat::compact`、`event::chat::compact` 与
//! `app::update::chat::compact` 负责。
//!
//! 契约（对应 design §2.3）：
//! - 摘要覆盖「上一个仍生效检查点之后 → 当前最新完整消息」，不重复压缩已压缩内容；
//! - 摘要正文保留文件路径、命令、错误、验证结果、关键决策与未完成事项；
//! - 输出有硬上限，超限即整体失败（不静默截断、不写检查点）；
//! - 摘要请求不是普通聊天轮次：不落库、不追加 user/assistant 消息。

use crate::chat::events::{ChatRequest, ChatTurn, ModelItem};
use crate::db::chat::ChatMessage;
use crate::db::chat::ModelSnapshot;

/// 摘要正文的字符上限（约 12k token 的保守上限）。
pub const MAX_SUMMARY_CHARS: usize = 24_000;

/// 输入侧（被压缩的历史正文）字符上限：超出时先丢最老的完整轮次并显式标记。
///
/// 这与输出上限是两件不同的事：输入太大仍可压缩（丢老的、标注省略），
/// 输出超限才是失败（否则“摘要”会悄悄变成第二份历史）。
pub const MAX_TRANSCRIPT_CHARS: usize = 120_000;

/// 摘要请求的系统提示：**逐字采用 codex 的压缩提示词**。
///
/// 出处：codex `codex-rs/prompts/templates/compact/prompt.md`（`SUMMARIZATION_PROMPT`）。
/// 结构、语气与措辞保持原样，不自造第二套清单；lx 特有的保留口径（路径/命令/错误/
/// 验证结果、用用户语言书写）放在请求的 user 段（见 [`summary_request`]），
/// 这样「压缩提示词」与「本次请求的要求」各自单一来源。
pub const COMPACT_SYSTEM_PROMPT: &str = "\
You are performing a CONTEXT CHECKPOINT COMPACTION. Create a handoff summary for another LLM \
that will resume the task.

Include:
- Current progress and key decisions made
- Important context, constraints, or user preferences
- What remains to be done (clear next steps)
- Any critical data, examples, or references needed to continue

Be concise, structured, and focused on helping the next LLM seamlessly continue the work.";

/// 一个待压缩区间：消息、覆盖的 sequence 范围与渲染后的转写文本。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactInput {
    /// 参与压缩的消息（按 sequence 升序，只含可回放的完整轮次与终态命令）。
    pub messages: Vec<ChatMessage>,
    /// 覆盖的起始 sequence（含）。
    pub start_sequence: i64,
    /// 覆盖的结束 sequence（含）。
    pub end_sequence: i64,
    /// 渲染后的转写文本（有界，超限时显式标记省略）。
    pub transcript: String,
    /// 因输入上限被省略的最老轮数（0 表示全部保留）。
    pub omitted_turns: usize,
}

impl CompactInput {
    /// 是否有可压缩内容。
    pub fn is_empty(&self) -> bool {
        self.messages.is_empty() || self.transcript.trim().is_empty()
    }
}

/// 从已加载的会话消息里切出 `after_sequence` 之后的增量区间。
///
/// 只纳入可回放的完整轮次（user 与 assistant 均 complete）与终态本地命令：
/// 失败/中断/流中的轮次既不能压缩成“成功”，也不能凭空补全。
pub fn incremental(messages: &[ChatMessage], after_sequence: Option<i64>) -> CompactInput {
    let start = after_sequence.map_or(i64::MIN, |sequence| sequence.saturating_add(1));
    let selected: Vec<ChatMessage> = playable_messages(messages)
        .into_iter()
        .filter(|message| message.sequence >= start)
        .collect();
    let (kept, omitted_turns, transcript) = bounded_transcript(&selected);
    let start_sequence = kept
        .first()
        .map(|message| message.sequence)
        .unwrap_or_else(|| start.max(0));
    let end_sequence = kept
        .last()
        .map(|message| message.sequence)
        .unwrap_or(start_sequence);
    CompactInput {
        messages: kept,
        start_sequence,
        end_sequence,
        transcript,
        omitted_turns,
    }
}

/// 可回放的完整轮次与终态本地命令。
///
/// 判据与上下文装配一致（`chat::context`）：只有 user 与 assistant 都 complete 的轮次、
/// 以及已终态的本地命令才允许成为摘要的事实来源。失败/中断/流中的轮次既不能压缩成
/// “成功”，也不能凭空补全，因此整体排除。
pub fn playable_messages(messages: &[ChatMessage]) -> Vec<ChatMessage> {
    use std::collections::HashMap;

    let mut by_turn: HashMap<i64, (bool, bool)> = HashMap::new();
    for message in messages {
        if message.kind == "user_shell" {
            continue;
        }
        let entry = by_turn.entry(message.turn_index).or_insert((false, false));
        match message.role.as_str() {
            "user" if message.status == "complete" => entry.0 = true,
            "assistant" if message.status == "complete" => entry.1 = true,
            _ => {}
        }
    }
    messages
        .iter()
        .filter(|message| {
            if message.kind == "user_shell" {
                return message
                    .user_shell
                    .as_ref()
                    .is_some_and(|record| record.is_terminal())
                    && message.status != "streaming";
            }
            by_turn
                .get(&message.turn_index)
                .is_some_and(|(user, assistant)| *user && *assistant)
        })
        .cloned()
        .collect()
}

/// 把消息渲染成有界转写文本；超限时**从最老的完整轮次开始丢**并显式标注。
///
/// 返回（保留的消息、丢弃的轮数、转写文本）。
fn bounded_transcript(messages: &[ChatMessage]) -> (Vec<ChatMessage>, usize, String) {
    let groups = group_by_turn(messages);
    let total = groups.len();
    // 从最新的组往回纳入，直到超出输入上限。
    let mut kept_groups: Vec<&Vec<ChatMessage>> = Vec::new();
    let mut used = 0usize;
    for group in groups.iter().rev() {
        let rendered = render_group(group);
        if !kept_groups.is_empty() && used + rendered.chars().count() > MAX_TRANSCRIPT_CHARS {
            break;
        }
        used += rendered.chars().count();
        kept_groups.push(group);
    }
    kept_groups.reverse();
    let omitted = total.saturating_sub(kept_groups.len());
    let kept_messages: Vec<ChatMessage> = kept_groups
        .iter()
        .flat_map(|group| group.iter().cloned())
        .collect();
    let mut transcript = String::new();
    if omitted > 0 {
        transcript.push_str(&format!(
            "[{omitted} older turn(s) were omitted before this summary range]\n"
        ));
    }
    for group in &kept_groups {
        transcript.push_str(&render_group(group));
    }
    (kept_messages, omitted, transcript)
}

/// 按 `turn_index` 分组；本地命令各自独立成组。
fn group_by_turn(messages: &[ChatMessage]) -> Vec<Vec<ChatMessage>> {
    let mut groups: Vec<Vec<ChatMessage>> = Vec::new();
    let mut current_turn: Option<i64> = None;
    for message in messages {
        if message.kind == "user_shell" {
            groups.push(vec![message.clone()]);
            current_turn = None;
            continue;
        }
        match current_turn {
            Some(turn) if turn == message.turn_index => {
                if let Some(group) = groups.last_mut() {
                    group.push(message.clone());
                }
            }
            _ => {
                current_turn = Some(message.turn_index);
                groups.push(vec![message.clone()]);
            }
        }
    }
    groups
}

/// 渲染一组消息（完整轮次或单条命令）为转写文本。
fn render_group(group: &[ChatMessage]) -> String {
    let mut text = String::new();
    for message in group {
        if message.kind == "user_shell" {
            text.push_str(&render_user_shell(message));
            continue;
        }
        let body = message_text(message);
        match message.role.as_str() {
            "user" => {
                text.push_str("### User\n");
                text.push_str(&body);
                text.push('\n');
            }
            "assistant" => {
                text.push_str("### Assistant\n");
                if !body.trim().is_empty() {
                    text.push_str(&body);
                    text.push('\n');
                }
                for tool in &message.tools {
                    text.push_str(&render_tool(tool));
                }
            }
            _ => {}
        }
    }
    text.push('\n');
    text
}

/// 本地命令渲染：命令、状态与已产生的输出（有界）。
fn render_user_shell(message: &ChatMessage) -> String {
    let Some(record) = message.user_shell.as_ref() else {
        return String::new();
    };
    let output = crate::chat::context::truncate_result_for_replay(&record.output);
    format!(
        "### Command\n$ {}\nstate: {} exit: {}\noutput:\n{}\n",
        record.command,
        record.state,
        record
            .exit_code
            .map_or_else(|| "unknown".to_string(), |code| code.to_string()),
        if output.is_empty() {
            "(no output)"
        } else {
            &output
        }
    )
}

/// 工具调用渲染：名称、参数与结果（结果按回放上限截断）。
fn render_tool(tool: &crate::db::chat::ToolCallRecord) -> String {
    let mut text = String::new();
    text.push_str(&format!(
        "### Tool `{}` ({})\nargs: {}\n",
        tool.name, tool.status, tool.arguments_json
    ));
    if let Some(error) = tool.error_text.as_deref().filter(|text| !text.is_empty()) {
        text.push_str(&format!("error: {error}\n"));
    }
    if let Some(result) = tool.result_json.as_deref().filter(|text| !text.is_empty()) {
        let bounded = crate::chat::context::truncate_result_for_replay(result);
        text.push_str(&format!("result: {bounded}\n"));
    }
    text
}

/// 一条消息的可见正文（text parts 顺序拼接）。
fn message_text(message: &ChatMessage) -> String {
    message
        .parts
        .iter()
        .filter(|part| part.kind == "text")
        .map(|part| part.content.as_str())
        .collect::<Vec<_>>()
        .join("")
}

/// 构造摘要请求：独立系统提示 + 单条转写消息，不携带工具定义。
///
/// `model` 是当前会话模型的快照；`variant_params` 保留思考等级（摘要不需要工具，
/// 但仍应使用用户选择的推理等级，避免与配置不一致）。
pub fn summary_request(
    input: &CompactInput,
    model: &ModelSnapshot,
    variant_params: Option<serde_json::Value>,
) -> ChatRequest {
    let mut user = String::new();
    user.push_str("<transcript>\n");
    user.push_str(&input.transcript);
    user.push_str("</transcript>\n\n");
    user.push_str(&format!(
        "Compress the transcript above. Cover message sequence {}..{} ({} message(s)).",
        input.start_sequence,
        input.end_sequence,
        input.messages.len()
    ));
    if input.omitted_turns > 0 {
        user.push_str(&format!(
            " {} older turn(s) were omitted from the transcript because of the input limit.",
            input.omitted_turns
        ));
    }
    // 系统段是 codex 的压缩提示词（通用口径），lx 侧对摘要内容的硬要求放这里：
    // 两段各自单一来源，改其一不必改另一处。
    user.push_str(
        "\n\nThis handoff summary must keep, when present: exact file paths created, modified, \
         deleted or moved; commands that were run with their exit status and the important \
         output; errors and how they were handled (never invent or hide them); decisions, \
         constraints and rejected options; verification results (build, tests, lints) with the \
         exact command; and unfinished work, next steps or open questions. Write in the language \
         the user used when it is clear. Do not repeat raw file contents or long logs, and do not \
         include this instruction or the <transcript> wrapper in your answer.",
    );
    ChatRequest {
        turns: vec![
            ChatTurn {
                role: "system".into(),
                text: COMPACT_SYSTEM_PROMPT.to_string(),
                reasoning_metadata: None,
            },
            ChatTurn {
                role: "user".into(),
                text: user.clone(),
                reasoning_metadata: None,
            },
        ],
        items: vec![ModelItem::UserText(user)],
        model: model.clone(),
        variant_params,
        purpose: crate::chat::events::RequestPurpose::CompactSummary,
        system_prompt: Some(COMPACT_SYSTEM_PROMPT.to_string()),
    }
}

/// 校验摘要正文：去空白、长度上限与最小长度。
///
/// 超限返回错误码（不截断），空摘要同样是失败——空压缩会静默丢掉整段上下文。
pub fn validate_summary(text: &str) -> Result<String, &'static str> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err("compact summary is empty");
    }
    if trimmed.chars().count() > MAX_SUMMARY_CHARS {
        return Err("compact summary exceeds the output limit");
    }
    Ok(trimmed.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::chat::ChatPart;

    fn message(id: &str, sequence: i64, turn: i64, role: &str, text: &str) -> ChatMessage {
        ChatMessage {
            external_id: id.into(),
            sequence,
            turn_index: turn,
            kind: "chat".into(),
            role: role.into(),
            status: "complete".into(),
            model_provider: None,
            model_id: None,
            variant: None,
            stop_reason: None,
            error_code: None,
            input_tokens: None,
            output_tokens: None,
            cache_read_tokens: None,
            cache_write_tokens: None,
            created_at_ms: 0,
            completed_at_ms: None,
            user_shell: None,
            parts: vec![ChatPart {
                position: 0,
                kind: "text".into(),
                content: text.into(),
                provider_metadata_json: None,
                started_at_ms: None,
                ended_at_ms: None,
                completed_at_ms: None,
            }],
            tools: Vec::new(),
        }
    }

    /// 增量范围只取检查点之后的完整消息，并给出精确的起止 sequence。
    #[test]
    fn incremental_starts_after_checkpoint() {
        let messages = vec![
            message("u1", 0, 0, "user", "old question"),
            message("a1", 1, 0, "assistant", "old answer"),
            message("u2", 2, 1, "user", "new question"),
            message("a2", 3, 1, "assistant", "new answer"),
        ];
        let input = incremental(&messages, Some(1));
        assert_eq!(input.messages.len(), 2);
        assert_eq!(input.start_sequence, 2);
        assert_eq!(input.end_sequence, 3);
        assert!(input.transcript.contains("new question"));
        assert!(!input.transcript.contains("old question"));

        // 没有检查点时覆盖全部。
        let all = incremental(&messages, None);
        assert_eq!(all.messages.len(), 4);
        assert_eq!(all.start_sequence, 0);
    }

    /// 摘要请求：不带工具、使用专用系统提示、正文包含转写与范围。
    #[test]
    fn summary_request_is_standalone() {
        let messages = vec![
            message("u1", 0, 0, "user", "do the thing"),
            message("a1", 1, 0, "assistant", "working on it"),
        ];
        let input = incremental(&messages, None);
        let request = summary_request(
            &input,
            &ModelSnapshot {
                provider: "p".into(),
                model: "m".into(),
                variant: None,
            },
            None,
        );
        assert_eq!(
            request.purpose,
            crate::chat::events::RequestPurpose::CompactSummary
        );
        assert!(!request.includes_tools(), "摘要请求不得携带工具定义");
        assert_eq!(
            request.system_prompt.as_deref(),
            Some(COMPACT_SYSTEM_PROMPT)
        );
        assert!(
            request.items[0]
                == ModelItem::UserText(match &request.items[0] {
                    ModelItem::UserText(text) => text.clone(),
                    _ => unreachable!(),
                })
        );
        let ModelItem::UserText(text) = &request.items[0] else {
            panic!("expected user text");
        };
        assert!(text.contains("<transcript>"));
        assert!(text.contains("do the thing"));
        assert!(
            text.contains("sequence 0..1"),
            "范围必须覆盖完整轮次：{text}"
        );
    }

    /// 摘要提示词必须**逐字**是 codex 的压缩提示词（`prompts/templates/compact/prompt.md`）。
    ///
    /// 锁死原文：这不是可以随手改文案的地方，改它等于换掉压缩协议。
    #[test]
    fn compact_prompt_matches_codex_verbatim() {
        const CODEX: &str = "\
You are performing a CONTEXT CHECKPOINT COMPACTION. Create a handoff summary for another LLM \
that will resume the task.

Include:
- Current progress and key decisions made
- Important context, constraints, or user preferences
- What remains to be done (clear next steps)
- Any critical data, examples, or references needed to continue

Be concise, structured, and focused on helping the next LLM seamlessly continue the work.";
        assert_eq!(COMPACT_SYSTEM_PROMPT, CODEX);
        // lx 侧的保留要求不写进系统段（否则就不再是 codex 原文），只在 user 段。
        assert!(!COMPACT_SYSTEM_PROMPT.contains("exact file paths"));
        assert!(!COMPACT_SYSTEM_PROMPT.contains("exit status"));
    }

    /// lx 的摘要硬要求必须出现在请求正文里（系统段是通用提示词，不能丢信息口径）。
    #[test]
    fn summary_request_states_the_lx_requirements() {
        let messages = vec![
            message("u1", 0, 0, "user", "改一下布局"),
            message("a1", 1, 0, "assistant", "done"),
        ];
        let input = incremental(&messages, None);
        let request = summary_request(
            &input,
            &ModelSnapshot {
                provider: "p".into(),
                model: "m".into(),
                variant: None,
            },
            None,
        );
        let ModelItem::UserText(text) = &request.items[0] else {
            panic!("expected user text");
        };
        for needle in [
            "exact file paths",
            "exit status",
            "never invent or hide them",
            "verification results",
            "unfinished work",
            "language the user used",
            "Do not repeat raw file contents",
        ] {
            assert!(text.contains(needle), "缺少要求 `{needle}`：{text}");
        }
        // 要求不能反过来混进系统段。
        assert_eq!(request.turns[0].text, COMPACT_SYSTEM_PROMPT);
    }

    /// 输出校验：空摘要失败、超限失败、正常值去空白。
    #[test]
    fn summary_validation_is_strict() {
        assert!(validate_summary("   ").is_err());
        let mut long = "x".repeat(MAX_SUMMARY_CHARS);
        assert!(validate_summary(&long).is_ok(), "恰好到上限仍合法");
        long.push('x');
        assert!(validate_summary(&long).is_err(), "超限必须失败而不是截断");
        assert_eq!(validate_summary("  ok  ").unwrap(), "ok");
    }

    /// 输入超限：从最老轮次开始丢并显式标注，最新的内容必须保留。
    #[test]
    fn transcript_input_is_bounded_from_the_oldest_turn() {
        let big = "y".repeat(MAX_TRANSCRIPT_CHARS / 2 + 1);
        let messages = vec![
            message("u1", 0, 0, "user", &big),
            message("a1", 1, 0, "assistant", &big),
            message("u2", 2, 1, "user", "latest request"),
            message("a2", 3, 1, "assistant", "latest answer"),
        ];
        let input = incremental(&messages, None);
        assert!(input.omitted_turns >= 1, "必须丢最老的轮次");
        assert!(input.transcript.contains("latest request"));
        assert!(input.transcript.contains("older turn(s) were omitted"));
        assert!(
            input.transcript.chars().count() <= MAX_TRANSCRIPT_CHARS + 512,
            "转写必须有界"
        );
    }

    /// 只压缩完整轮次：失败/中断轮次不进入转写（不能把失败写成成功）。
    #[test]
    fn incomplete_turns_are_not_part_of_the_transcript() {
        let mut messages = vec![
            message("u1", 0, 0, "user", "first"),
            message("a1", 1, 0, "assistant", "done"),
            message("u2", 2, 1, "user", "second"),
            message("a2", 3, 1, "assistant", "second answer"),
        ];
        messages[1].status = "failed".into();
        let input = incremental(&messages, None);
        // 失败轮次整体排除：既不进转写，也不占范围。
        assert!(!input.transcript.contains("done"));
        assert!(
            input.transcript.contains("second"),
            "失败的轮次不影响其它完整轮次"
        );
        assert_eq!(input.messages.len(), 2, "只剩完整轮次");
    }

    /// 终态本地命令进入转写；运行中的命令不进入。
    #[test]
    fn user_shell_commands_enter_the_transcript_only_when_terminal() {
        use crate::db::chat::UserShellRecord;
        let mut running = message("s1", 2, 1, "user", "npm run dev");
        running.kind = "user_shell".into();
        running.status = "streaming".into();
        running.user_shell = Some(UserShellRecord {
            command: "npm run dev".into(),
            cwd: "/w".into(),
            shell: "/bin/zsh".into(),
            state: "running".into(),
            exit_code: None,
            duration_ms: None,
            output: String::new(),
            output_start: 0,
            output_end: 0,
            dropped_bytes: 0,
            truncated: false,
            runtime_epoch: None,
            process_id: None,
            revision: 0,
        });
        let mut done = running.clone();
        done.external_id = "s2".into();
        done.sequence = 3;
        done.status = "complete".into();
        if let Some(record) = done.user_shell.as_mut() {
            record.state = "exited".into();
            record.exit_code = Some(0);
        }
        let input = incremental(&[running, done], None);
        assert_eq!(input.messages.len(), 1, "运行中的命令不参与压缩");
        assert!(input.transcript.contains("npm run dev"));
        assert!(input.transcript.contains("exit: 0"));
    }
}
