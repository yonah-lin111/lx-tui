//! 子 agent 运行时自动压缩：阈值估算、上下文转写、摘要请求与上下文替换。
//!
//! 从 `agent_loop/mod.rs` 拆出（单文件行数上限）。只在每次工具批次全部结算后调用，
//! 因此不会切断 call/result 对；摘要走独立用途（不声明工具），生产 `stream_one`
//! 会抑制摘要增量回投，避免把摘要正文画进子 agent 时间线。

use std::future::Future;

use tokio::sync::watch;

use crate::chat::client::StreamOutcome;
use crate::chat::compact::{AutoCompactNotice, MAX_TRANSCRIPT_CHARS, validate_summary};
use crate::chat::events::{ChatDelta, ChatRequest, ModelItem, PartKind, StopReason};

use super::{
    AUTO_COMPACT_PERCENT, AgentHost, FALLBACK_CONTEXT_LIMIT_TOKENS, HostError, ModelContext,
    compact_summary_prompt,
};

impl ModelContext {
    /// 估算下一次请求的 token（与 `chat::context` 同一口径：4 字符≈1 token）。
    fn estimated_tokens(&self) -> u64 {
        let mut chars = self.request.resolve_system_prompt().chars().count();
        if let Some(summary) = &self.compact_summary {
            chars = chars.saturating_add(compact_summary_prompt(summary).chars().count());
        }
        for item in &self.items {
            chars = chars.saturating_add(item_chars(item));
        }
        // 与 `chat::context` 的估算系数一致：4 字符 ≈ 1 token。
        u64::try_from(chars.div_ceil(4)).unwrap_or(u64::MAX)
    }

    /// 把当前 items 渲染成有界转写（从最新条目往回取，避免切断最新事实）。
    ///
    /// 返回（转写文本，被输入上限丢掉的 items 数）。
    fn compact_transcript(&self) -> (String, usize) {
        let mut body = String::new();
        let mut used = 0_usize;
        let mut kept = 0_usize;
        for item in self.items.iter().rev() {
            let rendered = render_item_for_compact(item);
            let chars = rendered.chars().count();
            if kept > 0 && used.saturating_add(chars) > MAX_TRANSCRIPT_CHARS {
                break;
            }
            used = used.saturating_add(chars);
            body.insert_str(0, &rendered);
            kept = kept.saturating_add(1);
        }
        let omitted = self.items.len().saturating_sub(kept);
        if omitted > 0 {
            body.insert_str(
                0,
                &format!("[{omitted} older item(s) were omitted before this summary range]\n"),
            );
        }
        if let Some(summary) = &self.compact_summary {
            body.insert_str(0, &format!("### Earlier compact summary\n{summary}\n\n"));
        }
        (body, omitted)
    }

    /// 应用一次自动压缩：保留原始任务，丢弃旧 items，摘要进系统段。
    fn apply_auto_compact(&mut self, summary: String) {
        let task = self
            .items
            .iter()
            .find(|item| matches!(item, ModelItem::UserText(_) | ModelItem::UserContent { .. }))
            .cloned();
        self.items.clear();
        if let Some(task) = task {
            self.items.push(task);
        }
        self.compact_summary = Some(summary);
    }
}

/// 一条结构化历史项的字符数（估算上下文占用用）。
fn item_chars(item: &ModelItem) -> usize {
    match item {
        ModelItem::UserText(text) | ModelItem::AssistantText(text) => text.chars().count(),
        ModelItem::UserContent { text, images } => text.chars().count() + images.len() * 6400,
        ModelItem::AssistantToolCall { name, input, .. } => {
            name.chars().count()
                + serde_json::to_string(input)
                    .map(|text| text.chars().count())
                    .unwrap_or_default()
        }
        ModelItem::ToolResult { content, .. } => content.chars().count(),
    }
}

/// 一条结构化历史项的转写（自动压缩的输入）。
fn render_item_for_compact(item: &ModelItem) -> String {
    match item {
        ModelItem::UserText(text) => format!("### User\n{text}\n\n"),
        ModelItem::UserContent { text, .. } => format!("### User\n{text}\n\n"),
        ModelItem::AssistantText(text) => format!("### Assistant\n{text}\n\n"),
        ModelItem::AssistantToolCall {
            call_id,
            name,
            input,
        } => format!(
            "### Tool call `{name}` ({call_id})\nargs: {}\n\n",
            serde_json::to_string(input).unwrap_or_default()
        ),
        ModelItem::ToolResult {
            call_id,
            content,
            is_error,
        } => format!(
            "### Tool result ({call_id}, error={is_error})\n{}\n\n",
            crate::chat::context::truncate_result_for_replay(content)
        ),
    }
}

/// 子 agent 上下文达到阈值时执行一次自动压缩。
pub(super) async fn maybe_auto_compact<H, S, F>(
    context: &mut ModelContext,
    host: &mut H,
    stream_one: &S,
    cancel: &watch::Receiver<bool>,
) -> Result<(), HostError>
where
    H: AgentHost,
    S: Fn(ChatRequest, watch::Receiver<bool>) -> F,
    F: Future<Output = StreamOutcome>,
{
    if !context.is_subagent() || context.items.len() <= 1 {
        return Ok(());
    }
    let limit = context
        .context_limit
        .unwrap_or(FALLBACK_CONTEXT_LIMIT_TOKENS);
    let threshold = limit.saturating_mul(AUTO_COMPACT_PERCENT) / 100;
    let estimated = context.estimated_tokens();
    if estimated < threshold {
        return Ok(());
    }
    let (transcript, omitted) = context.compact_transcript();
    if transcript.trim().is_empty() {
        return Ok(());
    }
    let request = crate::chat::compact::summary_request_from_transcript(
        &transcript,
        i64::try_from(omitted).unwrap_or(i64::MAX),
        i64::try_from(context.items.len()).unwrap_or(i64::MAX),
        context.items.len(),
        omitted,
        &context.request.model,
        context.request.variant_params.clone(),
    );
    let outcome = stream_one(request, cancel.clone()).await;
    if *cancel.borrow() {
        return Err(HostError::Cancelled);
    }
    if let StopReason::Failed { code } = &outcome.stop {
        return Err(HostError::Failed {
            code: format!("auto compact failed: {code}"),
        });
    }
    if outcome.stop != StopReason::Complete {
        return Err(HostError::Cancelled);
    }
    let mut text = String::new();
    for delta in &outcome.deltas {
        match delta {
            ChatDelta::Part(part) if part.kind == PartKind::Text => text.push_str(&part.text),
            ChatDelta::Part(_) => {}
            ChatDelta::ToolInputStarted { .. }
            | ChatDelta::ToolInputDelta { .. }
            | ChatDelta::ToolInputEnded { .. } => {
                return Err(HostError::Failed {
                    code: "auto compact failed: summary returned tool calls".into(),
                });
            }
            _ => {}
        }
    }
    let summary = validate_summary(&text).map_err(|message| HostError::Failed {
        code: format!("auto compact failed: {message}"),
    })?;
    host.notify_auto_compact(&AutoCompactNotice {
        model_label: context.request.model.model.clone(),
        summary: summary.clone(),
        estimated_tokens: estimated,
        context_limit: limit,
    });
    context.apply_auto_compact(summary);
    Ok(())
}
