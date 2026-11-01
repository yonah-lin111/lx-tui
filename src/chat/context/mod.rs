//! 上下文装配：把完整轮次装配成请求历史（纯文本视图 + 结构化视图）并按整轮裁剪。
//!
//! 两条视图必须同时成立：
//!
//! - `ChatRequest.turns`：旧纯文本形态（user 正文 + assistant 正文），只为旧 provider /
//!   旧路径保留，行为与工具化之前逐字节一致；
//! - `ChatRequest.items`：结构化有序步骤（正文与工具 call/result 按 `position` 合并），
//!   供四类 transport 按各自协议编码。
//!
//! 不变量（对应 design §7.3 / §14.2 / §18）：
//!
//! 1. 只有 `user.status == "complete"` 且 `assistant.status == "complete"` 的轮次参与回放；
//!    `failed` / `interrupted` 轮次既不回放正文也不回放工具（不伪造结果）。
//! 2. 工具只回放 `completed` / `failed` 且有可用结果内容的一对；`pending` / `running` /
//!    `cancelled` / `interrupted` 只在 UI 可见，绝不进入请求。
//! 3. 每个 `ToolResult` 前面必须有同 `call_id` 的 `AssistantToolCall` 且**紧邻**；
//!    参数不是 JSON 对象的调用整对跳过（不 panic）。
//! 4. 预算裁剪以完整轮次为单位：丢弃某轮即丢弃该轮全部 items，绝不产生 orphan result。
//! 5. reasoning 不回放（跨 provider 的思考原文与签名不注入历史）。
//! 6. 失败调用若结果体带已提交变更（`changes` 非空），回放内容 = 错误原因 +
//!    `partially applied: …` 摘要，与同轮 `format_for_model` 的口径一致（P1-3）。
//!
//! 当前轮的多步上下文不经过本模块（由 [`crate::chat::agent_loop::ModelContext`] 累计），
//! 装配只负责「DB/视图历史 + 末尾当前 user 消息」。

use crate::chat::events::{ChatRequest, ChatTurn, ModelItem};
use crate::db::chat::{ChatMessage, ModelSnapshot, ToolCallRecord};

mod user_shell;
pub use user_shell::format_user_shell;

#[cfg(test)]
mod tests;

/// 单条工具结果**回放**上限（64 KiB）：超过即截断并显式标记 `…[truncated]`。
///
/// 与 G3 工具层的落库上限（1 MiB）不同口径：落库保留较完整结果，回放只有这一个上限，
/// 避免一次请求被单条历史结果撑爆；截断必须显式告知模型，不得静默省略。
pub const MAX_TOOL_RESULT_REPLAY_BYTES: usize = 64 * 1024;

/// 结果被截断时追加的显式标记（与工具层 `…[truncated]` 同一形态）。
const TRUNCATED_MARKER: &str = "\n…[truncated]";

/// 装配输入：已加载的消息与当前草稿。
pub struct AssemblyInput<'a> {
    /// 同 session 的全部历史消息（sequence 升序；含 streaming/interrupted/failed）。
    pub messages: &'a [ChatMessage],
    /// 本轮用户文本（尚未落库，请求构造时并入末尾）。
    pub draft: &'a str,
    /// 本轮用户图片附件（当前轮发送）。
    pub draft_attachments: &'a [crate::db::chat::MessageAttachmentMeta],
    /// 本轮模型快照（provider/model/variant）。
    pub model: &'a ModelSnapshot,
    /// 模型上下文上限；None 时不裁剪（按完整历史发送）。
    pub context_limit: Option<u64>,
    /// 模型输出上限（预留预算）。
    pub output_limit: Option<u64>,
    /// 思考等级参数（来自 variants）；None 时不发送推理等级字段。
    pub variant_params: Option<serde_json::Value>,
    /// 当前生效的 compact 检查点（None 表示没有压缩过）。
    ///
    /// 有值时只回放 `end_sequence` 之后的消息，并把摘要作为**独立的前置上下文**
    /// 注入（不是 user/assistant 消息，也不进入复制选区）。
    pub compact: Option<&'a CompactContext>,
    /// 本 turn 冻结的模型身份段（`# Your Model`）；None 时省略。
    pub model_identity: Option<&'a str>,
    /// 本 turn 冻结的环境快照文本；None 时由 transport 现场采集（旧路径/测试）。
    ///
    /// 生产路径由事件层在 turn 创建时冻结一次，本轮所有采样复用同一份。
    pub environment: Option<&'a str>,
    /// 本 turn 冻结的 `AGENTS.md` / `agent.md` 指令段；None 时省略。
    pub instructions: Option<&'a str>,
    /// 本 turn 冻结的 skill 目录段；None 时省略。
    pub skills: Option<&'a str>,
    /// 本 turn 冻结的 MCP server 说明段；None 时省略。
    pub mcp: Option<&'a str>,
    /// 会话内已持久化的模型切换记录（按 `sequence` 注入 `<system-update>`）。
    pub model_switches: &'a [crate::db::chat::ModelSwitchRecord],
    /// 用户点名的 skill 全文块（只注入当前轮，不落库）。
    pub skill_blocks: &'a [String],
    /// 当前模型是否支持图片输入；false 时工具图片降级为文本占位。
    pub supports_image_input: bool,
}

/// 装配用的 compact 摘要（从持久化检查点投影而来）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactContext {
    /// 摘要覆盖到的最后一个原始消息 sequence。
    pub end_sequence: i64,
    /// 摘要正文。
    pub summary: String,
    /// 实际摘要模型显示名（只用于诊断与日志，不注入正文）。
    pub model_label: String,
}

impl CompactContext {
    /// 注入模型上下文的结构化文本。
    ///
    /// 包在 `<compact_summary>` 里并显式声明它不是新指令：模型必须把它当作既有事实，
    /// 不得把摘要当成用户刚刚提出的新要求。
    pub fn as_prompt(&self) -> String {
        format!(
            "<compact_summary model=\"{}\">\nThis is a summary of earlier conversation that was compacted. \
             Treat it as established context and do not answer it as a new request.\n\n{}\n</compact_summary>",
            escape_summary_attr(&self.model_label),
            self.summary
        )
    }
}

/// 摘要属性值转义：不得从摘要结构里伪造字段。
fn escape_summary_attr(text: &str) -> String {
    text.chars()
        .map(|ch| match ch {
            '<' => '\u{2039}',
            '>' => '\u{203A}',
            '"' => '\'',
            '&' => '\u{FF06}',
            ch if ch.is_control() => ' ',
            ch => ch,
        })
        .collect()
}

/// 装配结果中单个图片附件的引用描述。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemImageAttachment {
    pub id: String,
    pub mime: String,
    pub label: String,
}

/// 装配结果。
#[derive(Debug, Clone, PartialEq)]
pub struct Assembly {
    pub request: ChatRequest,
    /// 装配时省略的完整轮次数（UI 显示已省略旧轮提示）。
    pub omitted_turns: usize,
    /// 当前用户消息是否单独超限（报错，不截断用户输入）。
    pub draft_exceeds: bool,
    /// 每个参与回放的组的边界（普通完整聊天组或独立终态命令组）。
    ///
    /// `shrink_once` 按它整组删除，不能再用「每轮固定两条 turn / 遇到第二个 UserText」
    /// 推断边界——命令组只有一条 user、没有 assistant。
    groups: Vec<ContextGroup>,
    /// 本 turn 的系统提示（稳定提示 + 冻结环境快照 +（可选）compact 摘要）。
    ///
    /// 摘要放在系统段而非伪造 user/assistant 消息：design §3.3 的上下文顺序就是
    /// system → environment → compact summary → 检查点之后的消息。
    pub system_prompt: String,
    /// 选中的每个 UserContent 步骤对应的图片附件列表。
    pub item_image_attachments: Vec<(usize, Vec<ItemImageAttachment>)>,
}

/// 一个可回放组在两条视图中的区间（`end` 不含）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ContextGroup {
    turn_start: usize,
    turn_end: usize,
    item_start: usize,
    item_end: usize,
}

/// 一轮内本地命令结果的注入上限（字节）。
///
/// 历史装配无法预知“本轮运行中的命令何时结束”，因此装配时按同量级预留 token，
/// 否则历史刚好占满预算时再注入一条大输出会直接超过模型上下文。
pub const MAX_USER_SHELL_INJECTION_BYTES: usize = 16 * 1024;

/// 为本轮注入预留的 token（与 [`CHARS_PER_TOKEN`] 同一估算口径）。
pub(crate) const USER_SHELL_INJECTION_RESERVE_TOKENS: u64 =
    (MAX_USER_SHELL_INJECTION_BYTES / CHARS_PER_TOKEN) as u64;

/// 保守估算系数：每 token 约 4 字节（只用于装配，不作为 UI 的已用 token）。
const CHARS_PER_TOKEN: usize = 4;
/// 请求封装开销预留（消息结构、role 标记等）。
const ENVELOPE_RESERVE_TOKENS: u64 = 256;

/// 一个参与回放的组：普通完整聊天轮次，或一条独立终态本地命令。
struct PlannedTurn {
    /// 该组起点消息的 sequence（用于与模型切换时间线对齐）。
    user_sequence: i64,
    /// user 正文（聊天正文，或命令的稳定格式化文本）。
    user_text: String,
    /// assistant 可见正文（旧纯文本视图用；reasoning 不计入）；命令组为 None。
    assistant_text: Option<String>,
    /// assistant 的有序步骤：正文与成对 call/result（不含 reasoning）；命令组为空。
    items: Vec<ModelItem>,
    /// 该组估算 token（user 正文 + 上述全部 items）。
    tokens: u64,
    /// 该轮包含的图片附件元数据。
    user_images: Vec<crate::db::chat::MessageAttachmentMeta>,
}

/// 装配上下文。
///
/// - 按 `turn_index` 分组；只纳入 user/assistant 均 complete 的轮次。
/// - 估算超过 `context - output_reserve - envelope_reserve` 时停止纳入更老轮。
/// - 单条当前消息超限时 `draft_exceeds` 为真（调用方报错，不截断）。
pub fn assemble(input: &AssemblyInput<'_>) -> Assembly {
    // compact 生效时只回放检查点之后的消息：更早的原文已由摘要表达。
    let start_after = input
        .compact
        .map(|compact| compact.end_sequence)
        .unwrap_or(i64::MIN);
    let retained: Vec<ChatMessage> = input
        .messages
        .iter()
        .filter(|message| message.sequence > start_after)
        .cloned()
        .collect();
    let mut planned = plan_turns(&retained);
    strip_old_tool_images(&mut planned);
    if !input.supports_image_input {
        strip_all_tool_images(&mut planned);
    }
    let draft_image_tokens = {
        let mut unique_sources = std::collections::HashSet::new();
        let mut count = 0u64;
        for att in input.draft_attachments {
            if att.kind == crate::db::chat::MessageAttachmentKind::Image {
                let key = att.source_path.as_deref().unwrap_or(&att.label);
                if unique_sources.insert(key) {
                    count += 1;
                }
            }
        }
        count * 1600
    };
    let draft_tokens = (estimate_tokens(input.draft) as u64).saturating_add(draft_image_tokens);
    // 系统提示词动态注入段（model_identity / AGENTS.md / skills / mcp / compact）也占上下文窗口：
    // 先装配出真实系统提示词，并将动态注入段的 token 开销从历史预算扣除。
    let system_prompt =
        crate::chat::prompt::compose_system_prompt(crate::chat::prompt::PromptParts {
            model_identity: input.model_identity,
            environment: input.environment,
            instructions: input.instructions,
            skills: input.skills,
            mcp: input.mcp,
            compact: input.compact.map(CompactContext::as_prompt).as_deref(),
        });
    let extra_system_reserve: u64 = [
        input.model_identity,
        input.instructions,
        input.skills,
        input.mcp,
    ]
    .into_iter()
    .flatten()
    .map(|section| estimate_tokens(section) as u64)
    .sum::<u64>()
    .saturating_add(
        input
            .compact
            .map(|c| estimate_tokens(&c.as_prompt()) as u64)
            .unwrap_or(0),
    );
    let output_reserve = input.output_limit.unwrap_or(0).min(8192);
    let budget = input.context_limit.map(|context| {
        context
            .saturating_sub(output_reserve)
            .saturating_sub(ENVELOPE_RESERVE_TOKENS)
            .saturating_sub(extra_system_reserve)
            .saturating_sub(USER_SHELL_INJECTION_RESERVE_TOKENS)
    });
    // 从近到远纳入，直到预算耗尽；整轮为最小单位（丢一轮即丢该轮全部 items）。
    let mut kept: Vec<&PlannedTurn> = Vec::new();
    let mut used = draft_tokens;
    for turn in planned.iter().rev() {
        if let Some(budget) = budget
            && used.saturating_add(turn.tokens) > budget
        {
            break;
        }
        used = used.saturating_add(turn.tokens);
        kept.push(turn);
    }
    kept.reverse();
    let omitted_turns = planned.len() - kept.len();
    // 两条视图同步装配：turns 保持旧纯文本形态，items 承载结构化步骤。
    let mut turns: Vec<ChatTurn> = Vec::with_capacity(kept.len() * 2 + 1);
    let mut items: Vec<ModelItem> = Vec::new();
    let mut groups = Vec::with_capacity(kept.len());
    let mut item_image_attachments: Vec<(usize, Vec<ItemImageAttachment>)> = Vec::new();
    let min_switch_sequence = if omitted_turns > 0 {
        kept.first()
            .map_or(start_after, |first| first.user_sequence)
    } else {
        start_after
    };
    // 首轮之前的切换记录即使随旧轮次被裁掉，也要保留最后一条：接管当前历史的新模型
    // 必须收到 `<system-update>`，否则它不知道自己是从中途开始接手的。
    let prior_switch = if omitted_turns > 0 {
        input
            .model_switches
            .iter()
            .rfind(|sw| sw.sequence > start_after && sw.sequence < min_switch_sequence)
    } else {
        None
    };
    let mut retained_switches: Vec<&crate::db::chat::ModelSwitchRecord> = input
        .model_switches
        .iter()
        .filter(|sw| sw.sequence > min_switch_sequence)
        .collect();
    if let Some(prior) = prior_switch {
        retained_switches.insert(0, prior);
    }
    let mut switch_cursor = 0usize;

    for turn in &kept {
        let turn_start = turns.len();
        let item_start = items.len();
        while switch_cursor < retained_switches.len()
            && retained_switches[switch_cursor].sequence < turn.user_sequence
        {
            let sw = retained_switches[switch_cursor];
            if !sw.injected_prompt.trim().is_empty() {
                items.push(ModelItem::UserText(crate::chat::agent::wrap_system_update(
                    &sw.injected_prompt,
                )));
            }
            switch_cursor += 1;
        }
        turns.push(ChatTurn {
            role: "user".into(),
            text: turn.user_text.clone(),
            reasoning_metadata: None,
        });
        // 聊天组才有 assistant 正文；命令组是独立 user 事实，不伪造配对。
        if let Some(assistant_text) = &turn.assistant_text {
            // 跨 provider 的 reasoning 元数据不回传：只回传该轮已完成正文。
            turns.push(ChatTurn {
                role: "assistant".into(),
                text: assistant_text.clone(),
                reasoning_metadata: None,
            });
        }
        if turn.user_images.is_empty() {
            items.push(ModelItem::UserText(turn.user_text.clone()));
        } else {
            let item_idx = items.len();
            let att_ids = turn
                .user_images
                .iter()
                .map(|a| ItemImageAttachment {
                    id: a.id.clone(),
                    mime: a.mime.clone().unwrap_or_else(|| "image/png".into()),
                    label: a.label.clone(),
                })
                .collect();
            item_image_attachments.push((item_idx, att_ids));
            items.push(ModelItem::UserContent {
                text: turn.user_text.clone(),
                images: Vec::new(),
            });
        }
        items.extend(turn.items.iter().cloned());
        groups.push(ContextGroup {
            turn_start,
            turn_end: turns.len(),
            item_start,
            item_end: items.len(),
        });
    }
    // 冲刷最新一轮完成之后、当前草稿之前发生的模型切换 `<system-update>`（必须在 draft 之前，保持当前轮图片索引不变量）。
    while switch_cursor < retained_switches.len() {
        let sw = retained_switches[switch_cursor];
        if !sw.injected_prompt.trim().is_empty() {
            items.push(ModelItem::UserText(crate::chat::agent::wrap_system_update(
                &sw.injected_prompt,
            )));
        }
        switch_cursor += 1;
    }
    // 当前用户消息必须保留（不因预算截断用户输入）。
    let draft_exceeds = budget.is_some_and(|budget| draft_tokens > budget);
    turns.push(ChatTurn {
        role: "user".into(),
        text: input.draft.to_string(),
        reasoning_metadata: None,
    });
    let draft_images: Vec<&crate::db::chat::MessageAttachmentMeta> = input
        .draft_attachments
        .iter()
        .filter(|att| att.kind == crate::db::chat::MessageAttachmentKind::Image)
        .collect();
    if draft_images.is_empty() {
        items.push(ModelItem::UserText(input.draft.to_string()));
    } else {
        let item_idx = items.len();
        let att_ids = draft_images
            .iter()
            .map(|a| ItemImageAttachment {
                id: a.id.clone(),
                mime: a.mime.clone().unwrap_or_else(|| "image/png".into()),
                label: a.label.clone(),
            })
            .collect();
        item_image_attachments.push((item_idx, att_ids));
        items.push(ModelItem::UserContent {
            text: input.draft.to_string(),
            images: Vec::new(),
        });
    }
    for block in input.skill_blocks {
        items.push(ModelItem::UserText(block.clone()));
    }
    Assembly {
        request: ChatRequest {
            turns,
            items,
            model: input.model.clone(),
            variant_params: input.variant_params.clone(),
            purpose: crate::chat::events::RequestPurpose::Chat,
            // 冻结的系统提示随请求走：四类 transport 都编码它，不再各自现场采集环境。
            system_prompt: Some(system_prompt.clone()),
            // 主会话本轮冻结的内置工具；MCP 工具由事件层装配后替换。
            tools: std::sync::Arc::new(crate::chat::tools::builtin_tools(
                crate::chat::agent::ToolSurface::Main,
            )),
        },
        omitted_turns,
        draft_exceeds,
        groups,
        system_prompt,
        item_image_attachments,
    }
}

/// 去掉最老**完整轮次**后的收缩请求（溢出重试用；最多一次）。
///
/// 同时收缩 `turns` 与 `items` 两个视图：轮次边界在两者中一致（每轮 1 条 user turn、
/// 1 条 assistant turn；items 以 `UserText` 起始），因此被删轮次的 call/result 一并消失，
/// 不会留下 orphan result。只剩当前用户消息时返回 None（调用方报错）。
pub fn shrink_once(assembly: &Assembly) -> Option<ChatRequest> {
    let request = &assembly.request;
    // 按装配时记录的组边界整组删除：聊天组是 user+assistant，命令组只有一条 user。
    // 末尾的当前用户草稿不在任何组内，永远不会被删。
    let group = assembly.groups.first()?;
    let mut turns = request.turns.clone();
    let mut items = request.items.clone();
    if group.turn_end > turns.len() || group.item_end > items.len() {
        tracing::warn!(target: "lx::agent", "refusing to shrink: group span out of range");
        return None;
    }
    items.drain(group.item_start..group.item_end);
    turns.drain(group.turn_start..group.turn_end);
    Some(ChatRequest {
        turns,
        items,
        model: request.model.clone(),
        variant_params: request.variant_params.clone(),
        purpose: request.purpose,
        // 收缩只丢旧轮次：冻结的系统提示（含环境快照与 compact 摘要）与工具必须原样保留。
        system_prompt: request.system_prompt.clone(),
        tools: request.tools.clone(),
    })
}

/// 把单条工具结果截断到 [`MAX_TOOL_RESULT_REPLAY_BYTES`] 以内并显式标记。
///
/// 只用于**回放**（历史装配与本轮上下文累计）；不改工具层的落库结果体。
/// 截断点按 UTF-8 字符边界回退，绝不产生非法 UTF-8。
pub fn truncate_result_for_replay(content: &str) -> String {
    if content.len() <= MAX_TOOL_RESULT_REPLAY_BYTES {
        return content.to_string();
    }
    let budget = MAX_TOOL_RESULT_REPLAY_BYTES.saturating_sub(TRUNCATED_MARKER.len());
    let mut end = content.len().min(budget);
    while end > 0 && !content.is_char_boundary(end) {
        end -= 1;
    }
    let mut truncated = content[..end].to_string();
    truncated.push_str(TRUNCATED_MARKER);
    truncated
}

/// 按 `turn_index` 把消息分组，仅保留 user 与 assistant 均 complete 的完整轮次。
///
/// 分组按消息顺序（`sequence` 升序）扫描；同轮多条消息取第一条 user 与第一条 assistant，
/// 与旧「相邻完整对」行为等价（旧会话每轮恒为相邻两条）。
fn plan_turns(messages: &[ChatMessage]) -> Vec<PlannedTurn> {
    let mut planned = Vec::new();
    let mut index = 0;
    while index < messages.len() {
        // 独立命令组：终态且 JSON 完好才回放；运行中/损坏记录都不进模型。
        if messages[index].kind == "user_shell" {
            if let Some(group) = plan_user_shell(&messages[index]) {
                planned.push(group);
            }
            index += 1;
            continue;
        }
        let turn_index = messages[index].turn_index;
        let mut user: Option<&ChatMessage> = None;
        let mut assistant: Option<&ChatMessage> = None;
        while index < messages.len()
            && messages[index].turn_index == turn_index
            && messages[index].kind != "user_shell"
        {
            let message = &messages[index];
            match message.role.as_str() {
                "user" if user.is_none() => user = Some(message),
                "assistant" if assistant.is_none() => assistant = Some(message),
                _ => {}
            }
            index += 1;
        }
        let (Some(user), Some(assistant)) = (user, assistant) else {
            continue;
        };
        // 失败/中断轮次整体排除：既不回放正文，也不回放其工具（避免伪造）。
        if user.status != "complete" || assistant.status != "complete" {
            continue;
        }
        planned.push(plan_turn(user, assistant));
    }
    planned
}

/// 展开单轮：user 正文 + assistant 有序步骤 + token 估算。
fn plan_turn(user: &ChatMessage, assistant: &ChatMessage) -> PlannedTurn {
    let mut user_text = message_text_of(user);
    // skill 只在当轮注入全文；历史回放给一行来源摘要，不重新读文件。
    for attachment in user
        .attachments
        .iter()
        .filter(|att| att.kind == crate::db::chat::MessageAttachmentKind::Skill)
    {
        if let Some(path) = attachment.source_path.as_deref() {
            user_text.push('\n');
            user_text.push_str(&crate::skills::render_replay_line(
                &attachment.label,
                std::path::Path::new(path),
            ));
        }
    }
    let assistant_text = message_text_of(assistant);
    let items = assistant_items(assistant);
    let user_images: Vec<crate::db::chat::MessageAttachmentMeta> = user
        .attachments
        .iter()
        .filter(|att| att.kind == crate::db::chat::MessageAttachmentKind::Image)
        .cloned()
        .collect();
    let turn = PlannedTurn {
        user_sequence: user.sequence,
        user_text,
        assistant_text: Some(assistant_text),
        items,
        tokens: 0,
        user_images,
    };
    let tokens = planned_tokens(&turn);
    PlannedTurn { tokens, ..turn }
}

/// 一轮的 token 估算（user 图片与工具图片各 +1600）。
fn planned_tokens(turn: &PlannedTurn) -> u64 {
    let unique_img_count = {
        let mut seen = std::collections::HashSet::new();
        turn.user_images
            .iter()
            .filter(|att| seen.insert(att.source_path.as_deref().unwrap_or(&att.label)))
            .count() as u64
    };
    turn.items.iter().fold(
        (estimate_tokens(&turn.user_text) as u64).saturating_add(unique_img_count * 1600),
        |total, item| total.saturating_add(item_tokens(item)),
    )
}

/// 对一组轮次的工具结果执行图片裁剪：
/// - `supports_image_input == false`：清空全部工具图片并追加不支持占位；
/// - 否则仅保留最近 3 个含工具图片的轮次，更早轮次替换为省略占位。
///
/// 主会话与子 agent 历史装配共用同一规则（design §5.4）。
pub(crate) fn trim_tool_images_in_turn_items<'a>(
    turns: impl DoubleEndedIterator<Item = &'a mut Vec<ModelItem>>,
    supports_image_input: bool,
) {
    let mut rounds = 0usize;
    for items in turns.rev() {
        let has_images = items.iter().any(|item| {
            matches!(
                item,
                ModelItem::ToolResult { images, .. } if !images.is_empty()
            )
        });
        if !has_images {
            continue;
        }
        rounds = rounds.saturating_add(1);
        if supports_image_input && rounds <= 3 {
            continue;
        }
        let note = if supports_image_input {
            "\n[image from an earlier tool result omitted; call the tool again if needed]"
        } else {
            "\n[image omitted: this model does not support image input]"
        };
        for item in items {
            if let ModelItem::ToolResult {
                content, images, ..
            } = item
                && !images.is_empty()
            {
                images.clear();
                content.push_str(note);
            }
        }
    }
}

/// 只保留最近 3 个含工具图片的轮次；更早轮次的图片换成一行占位。
fn strip_old_tool_images(planned: &mut [PlannedTurn]) {
    trim_tool_images_in_turn_items(planned.iter_mut().map(|turn| &mut turn.items), true);
    for turn in planned {
        turn.tokens = planned_tokens(turn);
    }
}

/// 模型不支持图片：清空全部工具图片并追加一行英文占位。
fn strip_all_tool_images(planned: &mut [PlannedTurn]) {
    trim_tool_images_in_turn_items(planned.iter_mut().map(|turn| &mut turn.items), false);
    for turn in planned {
        turn.tokens = planned_tokens(turn);
    }
}

/// 从落库结果 JSON 取回工具图片字节；返回 `(可加载图片, 文件缺失张数)`。
///
/// 缺失文件不中断回放，由调用方在工具结果末尾追加 `[image no longer available]`，
/// 让模型知道结果里本来有图、只是字节已不可用。
fn replay_tool_images(result_json: Option<&str>) -> (Vec<crate::chat::events::ModelImage>, usize) {
    let Some(value) =
        result_json.and_then(|text| serde_json::from_str::<serde_json::Value>(text).ok())
    else {
        return (Vec::new(), 0);
    };
    let mut entries: Vec<&serde_json::Value> = Vec::new();
    if let Some(images) = value.get("images").and_then(serde_json::Value::as_array) {
        entries.extend(images);
    }
    if let Some(image) = value.get("image").filter(|image| image.is_object()) {
        entries.push(image);
    }
    let mut images = Vec::new();
    let mut missing = 0usize;
    for image in entries {
        let Some(path) = image.get("path").and_then(serde_json::Value::as_str) else {
            continue;
        };
        let mime = image
            .get("mime")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("image/png");
        match std::fs::read(path) {
            Ok(bytes) => images.push(crate::chat::events::ModelImage {
                mime: mime.to_string(),
                bytes: std::sync::Arc::new(bytes),
            }),
            Err(_) => missing = missing.saturating_add(1),
        }
    }
    (images, missing)
}

/// 一条本地命令组：只有终态记录可回放；格式化文本是唯一事实来源。
fn plan_user_shell(message: &ChatMessage) -> Option<PlannedTurn> {
    let record = message.user_shell.as_ref()?;
    // 运行中或未保存终态不注入模型（顺序屏障由调用方按 sequence 保证）。
    if matches!(message.status.as_str(), "streaming") || !record.is_terminal() {
        return None;
    }
    let text = format_user_shell(record);
    Some(PlannedTurn {
        user_sequence: message.sequence,
        tokens: estimate_tokens(&text) as u64,
        user_text: text,
        assistant_text: None,
        items: Vec::new(),
        user_images: Vec::new(),
    })
}

/// assistant 的有序模型项：把 `parts` 与 `tools` 按 `position` 合并成唯一时间线。
///
/// - `text` part → `AssistantText`（相邻正文合并成一条，保持顺序）；
/// - `reasoning` part → 不回放；
/// - 可回放的工具 → `AssistantToolCall` **紧随**同 `call_id` 的 `ToolResult`
///   （失败结果 `is_error = true`）；一次工具对在四类协议里都能编码成
///   「assistant tool_use/tool_calls」+「其后的 tool_result / tool / functionResponse」；
/// - 其他工具状态（pending/running/cancelled/interrupted）→ 不产生任何 item。
///
/// 不变量：结果必须紧邻其调用，且任何文本都不得插进这一对之间（禁止 orphan result）。
pub(crate) fn assistant_items(message: &ChatMessage) -> Vec<ModelItem> {
    /// 合并后的时间线步骤（借用消息内的 part / tool）。
    enum Step<'a> {
        Part(&'a crate::db::chat::ChatPart),
        Tool(&'a ToolCallRecord),
    }
    // position 在两条表内共用同一编号空间（DB 事务保证唯一），稳定排序即为真实顺序。
    let mut steps: Vec<(i64, Step<'_>)> = message
        .parts
        .iter()
        .map(|part| (part.position, Step::Part(part)))
        .chain(
            message
                .tools
                .iter()
                .map(|tool| (tool.position, Step::Tool(tool))),
        )
        .collect();
    steps.sort_by_key(|(position, _)| *position);
    let mut items: Vec<ModelItem> = Vec::new();
    let mut text = String::new();
    for (_, step) in steps {
        match step {
            Step::Part(part) => {
                if part.kind == "text" {
                    text.push_str(&part.content);
                }
            }
            Step::Tool(tool) => {
                // 不可回放的调用跳过；文本缓冲先冲刷，保持「text, call, result, text」顺序。
                let Some((call, result)) = replay_pair(tool) else {
                    continue;
                };
                if !text.is_empty() {
                    items.push(ModelItem::AssistantText(std::mem::take(&mut text)));
                }
                items.push(call);
                items.push(result);
            }
        }
    }
    if !text.is_empty() {
        items.push(ModelItem::AssistantText(text));
    } else if items.is_empty() {
        // 与旧纯文本装配逐字节等价：无正文的 assistant 轮次仍产生一条空正文项。
        items.push(ModelItem::AssistantText(String::new()));
    }
    items
}

/// 失败调用的回放内容：错误原因 + **已提交变更摘要**（跨轮与同轮口径对齐）。
///
/// 同轮给模型的 [`crate::chat::tools::format_for_model`] 会写
/// `partially applied: N file(s) were already changed on disk (this is not a rollback)`，
/// 并逐文件列出 `kind path (+a -r)`；落库的 `error_text` 只有精简原因（512 字符上限，
/// 给 UI 失败行用）。若回放只看 `error_text`，下一轮模型就不知道哪些文件已被改，
/// 与同轮信息矛盾（review P1-3）。
///
/// 因此这里把结果体里的结构化 `changes` 重新拼成同一事实：`result_json` 缺失、
/// 非法或 `changes` 为空时只回错误原因（保持原行为，不编造变更）。
fn failure_content(error: &str, result_json: Option<&str>) -> String {
    let Some(changes) = result_json
        .and_then(|json| serde_json::from_str::<serde_json::Value>(json).ok())
        .and_then(|value| value.get("changes").and_then(|c| c.as_array()).cloned())
        .filter(|changes| !changes.is_empty())
    else {
        return error.to_string();
    };
    let mut content = String::from(error);
    content.push_str(&format!(
        "\npartially applied: {} file(s) already changed on disk (not a rollback)\n",
        changes.len()
    ));
    for change in &changes {
        content.push_str(&describe_change_json(change));
        content.push('\n');
    }
    content
}

/// 回放前剥离**只服务 UI** 的文件差异正文（`edit` / `write` 的 `diff` 字段）。
///
/// 差异正文是执行后快照算出来的展示数据，不是模型看到的工具结果；把它带进历史会
/// 让下一轮上下文凭空膨胀（单次调用最多 512 KiB），且与同轮 `format_for_model` 的
/// 摘要口径不一致。路径、计数、`counts_known` / `simplified` / `file_totals` /
/// `omitted_files`、错误与 call/result 配对全部保留。
///
/// 非文件结果、坏 JSON 或形状不符时原样返回（不改变既有回放行为）。
fn strip_ui_diff_json(result: &str) -> String {
    let Ok(mut value) = serde_json::from_str::<serde_json::Value>(result) else {
        return result.to_string();
    };
    let file_tool = matches!(
        value.get("tool").and_then(serde_json::Value::as_str),
        Some("edit" | "write" | "error")
    );
    if !file_tool {
        return result.to_string();
    }
    let mut stripped = false;
    if let Some(object) = value.as_object_mut() {
        stripped |= object.remove("diff").is_some();
        if let Some(changes) = object.get_mut("changes").and_then(|c| c.as_array_mut()) {
            for change in changes {
                if let Some(change) = change.as_object_mut() {
                    stripped |= change.remove("diff").is_some();
                }
            }
        }
    }
    if !stripped {
        return result.to_string();
    }
    serde_json::to_string(&value).unwrap_or_else(|_| result.to_string())
}

/// 单条 `changes[]` 的单行描述（与工具层 `describe_change` 同形）：
/// `updated path (+1 -1)` / `moved a -> b (+0 -0)`。
fn describe_change_json(change: &serde_json::Value) -> String {
    let text = |key: &str| change.get(key).and_then(|value| value.as_str());
    let count = |key: &str| {
        change
            .get(key)
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0)
    };
    let kind = text("kind").unwrap_or("changed");
    let path = text("path").unwrap_or("");
    let counts = format!("(+{} -{})", count("added"), count("removed"));
    match (kind, text("to")) {
        ("moved", Some(to)) => format!("moved {path} -> {to} {counts}"),
        _ => format!("{kind} {path} {counts}"),
    }
}

/// 单次工具调用 → 成对的 (call, result)；不可回放时返回 None 并记 debug 日志。
///
/// 只有 `completed` / `failed` 且参数为 JSON 对象、且有可判定结果内容的调用才回放；
/// 其余状态（pending/running/cancelled/interrupted）与非法参数一律跳过，绝不伪造结果。
fn replay_pair(tool: &ToolCallRecord) -> Option<(ModelItem, ModelItem)> {
    let is_error = match tool.status.as_str() {
        "completed" => false,
        "failed" => true,
        _ => return None,
    };
    let input = match serde_json::from_str::<serde_json::Value>(&tool.arguments_json) {
        Ok(value) if value.is_object() => value,
        _ => {
            tracing::debug!(
                target: "lx::agent",
                call_id = %tool.call_id,
                name = %tool.name,
                "skip tool replay: arguments are not a JSON object"
            );
            return None;
        }
    };
    // 失败优先回放给模型看的精简原因；成功必须带结果体（否则无从判定，跳过）。
    let content = match (
        is_error,
        tool.error_text.as_deref(),
        tool.result_json.as_deref(),
    ) {
        // 失败：错误原因 +（结果体带已提交变更时）partial 摘要，见 [`failure_content`]。
        (true, Some(error), result_json) if !error.is_empty() => {
            failure_content(error, result_json)
        }
        // 成功 / 无错误文本的失败：回放落库结果体，但先剥掉只服务 UI 的差异正文。
        (_, _, Some(result)) if !result.is_empty() => strip_ui_diff_json(result),
        _ => {
            tracing::debug!(
                target: "lx::agent",
                call_id = %tool.call_id,
                name = %tool.name,
                is_error,
                "skip tool replay: call has no result body"
            );
            return None;
        }
    };
    let call_id = tool.call_id.clone();
    let (images, missing) = replay_tool_images(tool.result_json.as_deref());
    let mut content = truncate_result_for_replay(&content);
    for _ in 0..missing {
        content.push_str("\n[image no longer available]");
    }
    Some((
        ModelItem::AssistantToolCall {
            call_id: call_id.clone(),
            name: tool.name.clone(),
            input,
        },
        ModelItem::ToolResult {
            call_id,
            content,
            is_error,
            images,
        },
    ))
}

/// 单条模型项的 token 估算（含工具参数与结果内容）。
fn item_tokens(item: &ModelItem) -> u64 {
    match item {
        ModelItem::UserText(text) | ModelItem::AssistantText(text) => estimate_tokens(text) as u64,
        ModelItem::UserContent { text, images } => {
            (estimate_tokens(text) as u64) + (images.len() as u64) * 1600
        }
        ModelItem::AssistantToolCall {
            call_id,
            name,
            input,
        } => {
            // `input` 已解析为 JSON 值；按序列化长度估算（与 wire 编码同量级）。
            let arguments = input.to_string();
            (estimate_tokens(call_id) + estimate_tokens(name) + estimate_tokens(&arguments)) as u64
        }
        ModelItem::ToolResult {
            call_id,
            content,
            images,
            ..
        } => {
            (estimate_tokens(call_id) + estimate_tokens(content)) as u64
                + (images.len() as u64) * 1600
        }
    }
}

/// 估算 token 数（保守：4 字符 = 1 token）。
pub(crate) fn estimate_tokens(text: &str) -> usize {
    text.chars().count().div_ceil(CHARS_PER_TOKEN)
}

/// 一条消息的可见正文（text parts；reasoning 不回放或回传）。
fn message_text_of(message: &ChatMessage) -> String {
    message
        .parts
        .iter()
        .filter(|part| part.kind == "text")
        .map(|part| part.content.as_str())
        .collect::<Vec<_>>()
        .join("")
}
