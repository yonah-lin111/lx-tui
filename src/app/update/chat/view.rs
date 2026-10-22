//! DB 聊天记录 → 展示视图的纯投影（无 IO、无时间戳重排）。
//!
//! 一条 assistant 消息的 `parts`（内容块）与 `tools`（工具调用）在 DB 中是两张表，
//! 但 UI 与上下文回放只认**一条按 `position` 合并的步骤时间线**。两表各自已有
//! `position`（消息内唯一编号空间），合并时只用它，不得改用时间戳或到达顺序。

use crate::app::state::lx::chat::{AssistantStepView, ChatMessageView, ChatPartView};
use crate::app::state::{ModelsSnapshot, ToolCallView, ToolDetailView, ToolStatus, ToolSummary};
use crate::chat::events::PartKind;
use crate::db::chat::{ChatMessage, ChatPart, ToolCallRecord};

/// 单条 DB 消息 → 展示视图（parts 与 tools 按 position 合并）。
pub fn view_message_of(message: &ChatMessage, models: &ModelsSnapshot) -> ChatMessageView {
    let mut steps: Vec<AssistantStepView> =
        Vec::with_capacity(message.parts.len() + message.tools.len());
    steps.extend(message.parts.iter().map(|part| AssistantStepView::Part {
        position: part.position,
        part: part_view_of(part),
    }));
    steps.extend(message.tools.iter().map(|tool| AssistantStepView::Tool {
        position: tool.position,
        call: tool_view_of(tool),
    }));
    // 位置唯一（DB 事务保证），稳定排序后即为唯一时间线顺序。
    steps.sort_by_key(AssistantStepView::position);
    ChatMessageView {
        sequence: message.sequence,
        id: message.external_id.clone(),
        kind: message.kind.clone(),
        role: message.role.clone(),
        status: message.status.clone(),
        turn_index: message.turn_index,
        shell: message.user_shell.as_ref().map(shell_view_of),
        steps,
        model_label: message
            .model_provider
            .as_deref()
            .zip(message.model_id.as_deref())
            .map(|(provider, model)| models.model_display_name(provider, model)),
        error_code: message.error_code.clone(),
        input_tokens: message.input_tokens,
        output_tokens: message.output_tokens,
        cache_read_tokens: message.cache_read_tokens,
        duration_ms: message
            .completed_at_ms
            .map(|completed| completed.saturating_sub(message.created_at_ms).max(0)),
    }
}

/// DB 本地命令记录 → 展示视图（状态由稳定字符串还原，退出码原样携带）。
fn shell_view_of(record: &crate::db::chat::UserShellRecord) -> crate::app::state::UserShellView {
    crate::app::state::UserShellView {
        command: record.command.clone(),
        cwd: record.cwd.clone(),
        shell: record.shell.clone(),
        state: crate::app::state::ProcessStateView::from_stored(&record.state, record.exit_code),
        exit_code: record.exit_code,
        duration_ms: record.duration_ms,
        output: record.output.clone(),
        output_start: record.output_start,
        output_end: record.output_end,
        dropped_bytes: record.dropped_bytes,
        truncated: record.truncated,
        runtime_epoch: record.runtime_epoch.clone(),
        process_id: record.process_id,
        revision: record.revision,
    }
}

/// DB 内容块 → 视图内容块。
fn part_view_of(part: &ChatPart) -> ChatPartView {
    ChatPartView {
        kind: if part.kind == "reasoning" {
            PartKind::Reasoning
        } else {
            PartKind::Text
        },
        content: part.content.clone(),
        provider_metadata_json: part.provider_metadata_json.clone(),
        started_at_ms: part.started_at_ms,
        ended_at_ms: part.ended_at_ms,
        completed_at_ms: part.completed_at_ms,
    }
}

/// DB 工具调用记录 → 视图工具步骤。
///
/// 未知状态按 `Interrupted` 保守处理（宁可显示“结果未知”，也不伪装成功）；
/// `interrupted` 必须可见，因此 `error` 保留 DB 中的错误码。
fn tool_view_of(tool: &ToolCallRecord) -> ToolCallView {
    let mut status = ToolStatus::from_stored(&tool.status).unwrap_or(ToolStatus::Interrupted);
    // 进程工具的历史状态按**结果体**重判，而不是照抄 DB 里的 `status`。
    //
    // 早期版本把"命令以非零码退出"判成工具失败，于是 `grep -c` 无匹配（退出码 1）
    // 这类正常结果在库里留下了 `failed`。结果体本身是自洽的（`process_state` +
    // `exit_code`），据此重判可以让**旧会话**也正确显示，不需要写库迁移。
    // 详见 design §14.6。
    // 文件工具的差异详情与进程/计划同源：都从落库 JSON 解码；文件分支缺失时
    // `from_result_json` 返回 None，卡片沿用原两行摘要。
    let detail = ToolDetailView::from_result_json(tool.result_json.as_deref())
        .or_else(|| ToolDetailView::file_detail(tool.result_json.as_deref()));
    if let Some(ToolDetailView::Command(view)) = detail.as_ref()
        && view.state.is_nonzero_exit()
    {
        // 非零退出是结果的一部分，不是工具失败。
        status = ToolStatus::Completed;
    }
    // 摘要 = 参数派生（path / pattern / range）**+ 结果体补全**（增删计数、命中数、
    // 实际行数、截断标记）。只靠参数重建会让重进会话的工具行丢掉这些细节
    // （写文件类操作重进 session 后只剩方块与空 `└─`）。
    let summary = ToolSummary::from_arguments(&tool.arguments_json)
        .absorb_result_json(tool.result_json.as_deref());
    // 空摘要不挂到步骤上：`Some(空摘要)` 会让 UI 画出一个孤立的空 `└─`。
    // 判空走 [`ToolSummary::is_empty`]（手写字段 `||` 链在新增字段时会静默漏掉）。
    ToolCallView {
        call_id: tool.call_id.clone(),
        name: tool.name.clone(),
        position: tool.position,
        arguments_json: tool.arguments_json.clone(),
        status,
        summary: (!summary.is_empty()).then_some(Box::new(summary)),
        // 非零退出的旧记录里存着 `command exited with code N`，那句"失败"结论
        // 已经在重判时被推翻，不能继续显示。
        error: if status == ToolStatus::Completed {
            None
        } else {
            tool.error_text.clone()
        },
        // 结果体只在历史加载时回填：上下文回放用它（UI 不铺开全文）。
        result_json: tool.result_json.clone(),
        // 进程/计划工具的展示数据从同一条结果 JSON 生成；文件工具为 None。
        detail: detail.map(Box::new),
    }
}

#[cfg(test)]
mod tests;
