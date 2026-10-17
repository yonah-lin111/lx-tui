//! 上下文装配：从完整轮次构造请求历史，处理上下文窗口裁剪。
//!
//! 只取完整轮次（complete user + complete assistant）；失败/中断 partial
//! 仅供展示。当前用户消息永远纳入。无上限时不假定具体窗口；
//! 溢出时最多去掉最老完整轮重试一次，复用同一轮 DB 记录。

use crate::chat::events::{ChatRequest, ChatTurn, ModelItem};
use crate::db::chat::{ChatMessage, ModelSnapshot};

/// 装配输入：已加载的消息与当前草稿。
pub struct AssemblyInput<'a> {
    /// 同 session 的全部历史消息（sequence 升序；含 streaming/interrupted/failed）。
    pub messages: &'a [ChatMessage],
    /// 本轮用户文本（尚未落库，请求构造时并入末尾）。
    pub draft: &'a str,
    /// 本轮模型快照（provider/model/variant）。
    pub model: &'a ModelSnapshot,
    /// 模型上下文上限；None 时不裁剪（按完整历史发送）。
    pub context_limit: Option<u64>,
    /// 模型输出上限（预留预算）。
    pub output_limit: Option<u64>,
    /// 思考等级参数（来自 variants）；None 时不发送推理等级字段。
    pub variant_params: Option<serde_json::Value>,
}

/// 装配结果。
#[derive(Debug, Clone, PartialEq)]
pub struct Assembly {
    pub request: ChatRequest,
    /// 装配时省略的完整轮次数（UI 显示已省略旧轮提示）。
    pub omitted_turns: usize,
    /// 当前用户消息是否单独超限（报错，不截断用户输入）。
    pub draft_exceeds: bool,
}

/// 保守估算系数：每 token 约 4 字节（只用于装配，不作为 UI 的已用 token）。
const CHARS_PER_TOKEN: usize = 4;
/// 请求封装开销预留（消息结构、role 标记等）。
const ENVELOPE_RESERVE_TOKENS: u64 = 256;

/// 装配上下文。
///
/// - 按 `turn_index` 分组；只纳入 user/assistant 均 complete 的轮次。
/// - 估算超过 `context - output_reserve - envelope_reserve` 时停止纳入更老轮。
/// - 单条当前消息超限时 `draft_exceeds` 为真（调用方报错，不截断）。
pub fn assemble(input: &AssemblyInput<'_>) -> Assembly {
    // 按 turn_index 聚合完整轮次（保持原顺序）。
    let mut complete_turns: Vec<(i64, &ChatMessage, &ChatMessage)> = Vec::new();
    let mut index = 0;
    while index + 1 < input.messages.len() {
        let user = &input.messages[index];
        let assistant = &input.messages[index + 1];
        if user.turn_index == assistant.turn_index
            && user.role == "user"
            && assistant.role == "assistant"
            && user.status == "complete"
            && assistant.status == "complete"
        {
            complete_turns.push((user.turn_index, user, assistant));
            index += 2;
            continue;
        }
        index += 1;
    }
    let draft_tokens = estimate_tokens(input.draft);
    let output_reserve = input.output_limit.unwrap_or(0).min(8192);
    let budget = input.context_limit.map(|context| {
        context
            .saturating_sub(output_reserve)
            .saturating_sub(ENVELOPE_RESERVE_TOKENS)
    });
    // 从近到远纳入，直到预算耗尽；保持原顺序回传。
    let mut kept: Vec<(i64, &ChatMessage, &ChatMessage)> = Vec::new();
    let mut used = draft_tokens as u64;
    for turn in complete_turns.iter().rev() {
        let (user, assistant) = (turn.1, turn.2);
        let user_text = user_text_of(user);
        let assistant_text = assistant_text_of(assistant);
        let tokens = (estimate_tokens(&user_text) + estimate_tokens(&assistant_text)) as u64;
        if let Some(budget) = budget
            && used + tokens > budget
        {
            break;
        }
        used += tokens;
        kept.push(*turn);
    }
    kept.reverse();
    let omitted_turns = complete_turns.len() - kept.len();
    // 结构化历史（工具化路径）：当前各 transport 仍按 `turns` 编码，本组只保证
    // 纯文本历史下两者等价；成对 tool call/result 的装配由上下文重做阶段接入。
    let mut items: Vec<ModelItem> = Vec::new();
    // 溢出重试候选：去掉最老一轮后的历史（供调用方在上游报错时收缩一次）。
    let mut turns: Vec<ChatTurn> = Vec::with_capacity(kept.len() * 2 + 1);
    for (_, user, assistant) in &kept {
        let user_text = user_text_of(user);
        let assistant_text = assistant_text_of(assistant);
        turns.push(ChatTurn {
            role: "user".into(),
            text: user_text.clone(),
            reasoning_metadata: None,
        });
        // 跨 provider 的 reasoning 元数据不回传：只回传该轮已完成正文。
        turns.push(ChatTurn {
            role: "assistant".into(),
            text: assistant_text.clone(),
            reasoning_metadata: None,
        });
        items.push(ModelItem::UserText(user_text));
        items.push(ModelItem::AssistantText(assistant_text));
    }
    // 当前用户消息必须保留。
    let draft_exceeds = budget.is_some_and(|budget| draft_tokens as u64 > budget);
    turns.push(ChatTurn {
        role: "user".into(),
        text: input.draft.to_string(),
        reasoning_metadata: None,
    });
    items.push(ModelItem::UserText(input.draft.to_string()));
    Assembly {
        request: ChatRequest {
            turns,
            items,
            model: input.model.clone(),
            variant_params: input.variant_params.clone(),
        },
        omitted_turns,
        draft_exceeds,
    }
}

/// 去掉最老一轮后的收缩视图（溢出重试用；最多一次）。
///
/// 返回收缩后的 turns；无可收缩轮时返回 None（调用方报错）。
pub fn shrink_once(assembly: &Assembly) -> Option<Vec<ChatTurn>> {
    // turns 结构：每完整轮 2 条 + 末尾当前 user 消息 1 条。
    if assembly.request.turns.len() < 3 {
        return None;
    }
    let mut turns = assembly.request.turns.clone();
    // 移除最老完整轮（前两条）。
    let removed_user = turns.remove(0);
    let removed_assistant = turns.remove(0);
    debug_assert_eq!(removed_user.role, "user");
    debug_assert_eq!(removed_assistant.role, "assistant");
    Some(turns)
}

/// 估算 token 数（保守：4 字符 = 1 token）。
fn estimate_tokens(text: &str) -> usize {
    text.chars().count().div_ceil(CHARS_PER_TOKEN)
}

/// user 消息的文本 part 内容。
fn user_text_of(message: &ChatMessage) -> String {
    message
        .parts
        .iter()
        .filter(|part| part.kind == "text")
        .map(|part| part.content.as_str())
        .collect::<Vec<_>>()
        .join("")
}

/// assistant 消息的正文（text parts；reasoning 不回传）。
fn assistant_text_of(message: &ChatMessage) -> String {
    message
        .parts
        .iter()
        .filter(|part| part.kind == "text")
        .map(|part| part.content.as_str())
        .collect::<Vec<_>>()
        .join("")
}

#[cfg(test)]
mod tests;
