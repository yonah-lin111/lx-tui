//! 列表/步骤/块的内容指纹：只读状态、O(步骤数)，覆盖一切影响行数或正文的字段。
//!
//! **不**包含：hover、loading 帧、跟随底部、视口位置、置顶抑制记忆、滚动位置。
//!
//! 思考块（Reasoning）按 Claude Code 口径折叠流式正文：生成中只画单行头部，
//! 因此流式期间正文与时间戳变化**不**进入指纹；思考完成且展开时才一次性参与排版。

use crate::app::state::{AssistantStepView, CardSection, ChatMessageView};

use super::super::source::HistorySource;
use super::tool_signature::tool_signature;

/// FNV-1a 64 位哈希（不引第三方依赖）。
pub(super) struct Fnv(u64);

impl Fnv {
    const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;

    pub(super) fn new() -> Self {
        Self(Self::OFFSET_BASIS)
    }

    fn write(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.0 ^= u64::from(*byte);
            self.0 = self.0.wrapping_mul(Self::PRIME);
        }
    }

    pub(super) fn write_u8(&mut self, value: u8) {
        self.write(&[value]);
    }

    pub(super) fn write_u16(&mut self, value: u16) {
        self.write(&value.to_le_bytes());
    }

    pub(super) fn write_u64(&mut self, value: u64) {
        self.write(&value.to_le_bytes());
    }

    pub(super) fn write_usize(&mut self, value: usize) {
        self.write(&(value as u64).to_le_bytes());
    }

    pub(super) fn write_i64(&mut self, value: i64) {
        self.write(&value.to_le_bytes());
    }

    pub(super) fn write_str(&mut self, value: &str) {
        self.write_usize(value.len());
        self.write(value.as_bytes());
    }

    /// 只写首尾各 64 字节：流式追加、环形缓冲裁剪都必然改变其一，且不扫全文。
    pub(super) fn write_edges(&mut self, value: &str) {
        let bytes = value.as_bytes();
        let head = &bytes[..bytes.len().min(64)];
        let tail = &bytes[bytes.len().saturating_sub(64)..];
        self.write(head);
        self.write(tail);
    }

    pub(super) fn finish(self) -> u64 {
        self.0
    }
}

/// 列表指纹：只读状态、O(步骤数)，覆盖一切影响行数或正文的字段。
pub(crate) fn list_signature_source(source: HistorySource<'_>, width: u16) -> u64 {
    list_signature(source, width, false)
}

/// 「除流式末条消息外」的列表指纹：供屏幕外流式合并判定使用。
pub(crate) fn non_stream_signature_source(source: HistorySource<'_>, width: u16) -> u64 {
    list_signature(source, width, true)
}

/// 列表指纹实现；`skip_streaming_tail` 时忽略流式末条消息的内容细节。
fn list_signature(source: HistorySource<'_>, width: u16, skip_streaming_tail: bool) -> u64 {
    let mut hash = Fnv::new();
    hash.write_u16(width);
    // 统计行是否可见取决于「是否仍有子 agent 在跑」；状态变化不经过消息本身。
    hash.write_u8(u8::from(source.subagents_running));
    hash.write_usize(source.messages.len());
    hash.write_usize(source.notices.len());
    hash.write_usize(source.checkpoints.len());
    for (index, message) in source.messages.iter().enumerate() {
        hash.write_usize(index);
        hash.write_str(&message.id);
        hash.write_i64(message.sequence);
        hash.write_str(&message.role);
        hash.write_str(&message.status);
        hash.write_str(&message.kind);
        if skip_streaming_tail
            && index + 1 == source.messages.len()
            && message.status == "streaming"
        {
            // 末条流式消息：内容每帧都在变，但它整块位于视口下方时无需重排。
            hash.write_u8(u8::from(message.shell.is_some()));
            hash.write_usize(message.reports.len());
            hash.write_usize(message.steps.len());
            continue;
        }
        hash.write_str(message.model_label.as_deref().unwrap_or(""));
        hash.write_str(message.error_code.as_deref().unwrap_or(""));
        hash.write_i64(message.input_tokens.unwrap_or(-1));
        hash.write_i64(message.output_tokens.unwrap_or(-1));
        hash.write_i64(message.cache_read_tokens.unwrap_or(-1));
        hash.write_i64(message.duration_ms.unwrap_or(-1));
        if let Some(shell) = &message.shell {
            shell_signature(&mut hash, source, message, shell);
        } else {
            hash.write_u8(0);
        }
        hash.write_usize(message.reports.len());
        for report in &message.reports {
            report_signature(&mut hash, report);
            let slot = format!("report:{}", report.agent_id);
            // 汇报块的展开状态与已追加批次都会改变行数；漏进列表指纹会让快照
            // 停留在旧几何，而绘制按新状态多写行，正文就会溢出/盖住后面的块。
            hash.write_u8(u8::from(source.chat.detail_is_expanded(&message.id, &slot)));
            hash.write_usize(source.chat.report_expand_steps(&message.id, &slot));
        }
        hash.write_usize(message.steps.len());
        for (step_index, step) in message.steps.iter().enumerate() {
            hash.write_usize(step_index);
            step_signature(&mut hash, source, message, index, step_index, step);
        }
    }
    for notice in source.notices {
        hash.write_str(&notice.id);
        hash.write_i64(notice.sequence);
        hash.write_str(&notice.kind);
        hash.write_usize(notice.restored_file_count);
        hash.write_u8(u8::from(notice.files_already_restored));
    }
    for checkpoint in source.checkpoints {
        hash.write_str(&checkpoint.id);
        hash.write_i64(checkpoint.sequence);
        hash.write_str(&checkpoint.model_label);
        hash.write_i64(checkpoint.created_at_ms);
        hash.write_i64(checkpoint.start_sequence);
        hash.write_i64(checkpoint.end_sequence);
        hash.write_str(&checkpoint.summary);
        hash.write_u8(u8::from(checkpoint.active));
        hash.write_u8(u8::from(checkpoint.auto));
        hash.write_u8(u8::from(
            source
                .compact
                .expanded
                .iter()
                .any(|id| id == &checkpoint.id),
        ));
    }
    if let Some(running_id) = &source.compact.running {
        hash.write_str(running_id);
        hash.write_i64(source.compact.elapsed_ms.unwrap_or(0) / 1000);
    }
    for sw in source.switches {
        hash.write_str(&sw.id);
        hash.write_i64(sw.sequence);
        hash.write_str(sw.from_label.as_deref().unwrap_or(""));
        hash.write_str(&sw.to_provider);
        hash.write_str(&sw.to_model);
        hash.write_str(sw.to_variant.as_deref().unwrap_or(""));
        hash.write_str(&sw.to_label);
        hash.write_i64(sw.created_at_ms);
        hash.write_str(&sw.injected_prompt);
        for src in &sw.updated_sources {
            hash.write_str(src);
        }
        hash.write_u8(u8::from(source.model_switch.is_expanded(&sw.id)));
    }
    hash.finish()
}

/// 思考块正文是否应当排版并展示（对齐 Claude Code：流式生成期间恒不流式渲染
/// 思考正文，待该思考块完成后若处于展开态再一次性静态渲染）。
pub(crate) fn reasoning_body_visible(
    source: HistorySource<'_>,
    message_index: usize,
    step_index: usize,
) -> bool {
    source.chat.reasoning_expanded(message_index, step_index)
        && !source.chat.reasoning_streaming(message_index, step_index)
}

/// 本地命令卡片：正文与折叠状态影响行数。
pub(super) fn shell_signature(
    hash: &mut Fnv,
    source: HistorySource<'_>,
    message: &ChatMessageView,
    shell: &crate::app::state::UserShellView,
) {
    hash.write_u8(1);
    // 参数区/输出区各自独立折叠，必须进指纹。
    hash.write_u8(u8::from(source.chat.section_is_expanded(
        &message.id,
        crate::app::state::SHELL_SLOT,
        CardSection::Body,
    )));
    hash.write_u8(u8::from(source.chat.section_is_expanded(
        &message.id,
        crate::app::state::SHELL_SLOT,
        CardSection::Output,
    )));
    hash.write_str(&shell.command);
    hash.write_str(shell.state.as_str());
    hash.write_i64(shell.exit_code.map_or(-1, i64::from));
    hash.write_u64(shell.duration_ms.unwrap_or(u64::MAX));
    hash.write_usize(shell.output.len());
    hash.write_edges(&shell.output);
    hash.write_u64(shell.dropped_bytes);
    hash.write_u8(u8::from(shell.truncated));
    hash.write_u64(shell.revision);
}

/// 单个步骤：Part 覆盖正文/思考可见性；Tool 覆盖调用身份、状态、折叠与详情量。
fn step_signature(
    hash: &mut Fnv,
    source: HistorySource<'_>,
    message: &ChatMessageView,
    message_index: usize,
    step_index: usize,
    step: &AssistantStepView,
) {
    match step {
        AssistantStepView::Part { part, .. } => {
            hash.write_u8(0);
            match part.kind {
                crate::chat::events::PartKind::Text => {
                    hash.write_u8(0);
                    hash.write_usize(part.content.len());
                    hash.write_edges(&part.content);
                    hash.write_i64(part.started_at_ms.unwrap_or(-1));
                    hash.write_i64(part.ended_at_ms.unwrap_or(-1));
                    hash.write_i64(part.completed_at_ms.unwrap_or(-1));
                }
                crate::chat::events::PartKind::Reasoning => {
                    hash.write_u8(1);
                    let streaming = source.chat.reasoning_streaming(message_index, step_index);
                    let expanded = source.chat.reasoning_expanded(message_index, step_index);
                    let body_visible = expanded && !streaming;
                    hash.write_u8(u8::from(streaming));
                    hash.write_u8(u8::from(expanded));
                    hash.write_u8(u8::from(body_visible));
                    if !streaming {
                        // 完成后头部才显示 `Thought · 6.3s`；流式期间只显示
                        // `Thinking`，每个 delta 刷新的 `ended_at_ms` 必须忽略。
                        hash.write_i64(part.started_at_ms.unwrap_or(-1));
                        hash.write_i64(part.ended_at_ms.unwrap_or(-1));
                        hash.write_i64(part.completed_at_ms.unwrap_or(-1));
                    }
                    if body_visible {
                        // 只有「已完成且展开」才把正文字节纳入指纹，触发一次性排版。
                        hash.write_usize(part.content.len());
                        hash.write_edges(&part.content);
                    }
                }
            }
            if message.role == "user" {
                hash.write_u8(u8::from(source.chat.user_message_expanded(&message.id)));
                for (msg, slot, part, is_exp) in &source.chat.detail_expanded {
                    if msg == &message.id
                        && *part == CardSection::Body
                        && *is_exp
                        && slot.starts_with("paste:")
                    {
                        hash.write_str(slot);
                    }
                }
                hash.write_usize(message.attachments.len());
                for att in &message.attachments {
                    hash.write_u8(att.kind as u8);
                    hash.write_edges(&att.label);
                    hash.write_i64(att.position);
                    hash.write_usize(att.marker_start.unwrap_or(usize::MAX));
                    hash.write_usize(att.marker_end.unwrap_or(usize::MAX));
                }
            }
        }
        AssistantStepView::Tool { call, .. } => {
            hash.write_u8(1);
            tool_signature(hash, source, message, call);
        }
    }
}

/// 单个步骤块的指纹（宽度由 [`super::BlockKey::width`] 承担，这里不含宽度）。
pub(crate) fn part_block_signature(
    source: HistorySource<'_>,
    message: &ChatMessageView,
    message_index: usize,
    step_index: usize,
    step: &AssistantStepView,
) -> u64 {
    let mut hash = Fnv::new();
    step_signature(&mut hash, source, message, message_index, step_index, step);
    hash.finish()
}

/// 汇报块的列表指纹字段（展开状态另由块指纹承担）。
fn report_signature(hash: &mut Fnv, report: &crate::app::state::SubagentReportView) {
    hash.write_str(&report.agent_id);
    hash.write_str(report.nickname.as_deref().unwrap_or(""));
    hash.write_str(&report.title);
    hash.write_str(report.role.as_str());
    hash.write_str(&report.status);
    hash.write_usize(report.files_changed);
    hash.write_i64(report.duration_ms.unwrap_or(-1));
    hash.write_usize(report.body.len());
    hash.write_edges(&report.body);
}
