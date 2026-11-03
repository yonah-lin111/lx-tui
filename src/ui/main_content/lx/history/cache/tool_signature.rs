//! 工具调用块的指纹：身份、状态、折叠状态与详情中会改变行数的量。

use crate::app::state::{CardSection, ChatMessageView, ToolCallView, ToolDetailView};

use super::super::source::HistorySource;
use super::signature::Fnv;

pub(super) fn tool_signature(
    hash: &mut Fnv,
    source: HistorySource<'_>,
    message: &ChatMessageView,
    call: &ToolCallView,
) {
    hash.write_str(&call.call_id);
    hash.write_str(&call.name);
    hash.write_u8(tool_status_code(call.status));
    // 折叠状态：参数区（Body）与输出区（Output）各自独立。
    hash.write_u8(u8::from(source.chat.section_is_expanded(
        &message.id,
        &call.call_id,
        CardSection::Body,
    )));
    hash.write_u8(u8::from(source.chat.section_is_expanded(
        &message.id,
        &call.call_id,
        CardSection::Output,
    )));
    // 摘要：文件工具的详情行正文由它决定（路径/计数/截断/省略文件数）。
    if let Some(summary) = call.summary.as_deref() {
        hash.write_u8(1);
        hash.write_str(summary.path.as_deref().unwrap_or(""));
        hash.write_usize(summary.range.map_or(usize::MAX, |(start, _)| start));
        hash.write_usize(summary.range.map_or(usize::MAX, |(_, end)| end));
        hash.write_usize(summary.added.unwrap_or(usize::MAX));
        hash.write_usize(summary.removed.unwrap_or(usize::MAX));
        hash.write_usize(summary.matches.unwrap_or(usize::MAX));
        hash.write_str(summary.pattern.as_deref().unwrap_or(""));
        hash.write_u8(match summary.read_kind {
            None => 0,
            Some(crate::chat::tools::ReadKind::File) => 1,
            Some(crate::chat::tools::ReadKind::Directory) => 2,
            Some(crate::chat::tools::ReadKind::Image) => 3,
        });
        hash.write_usize(summary.read_lines.unwrap_or(usize::MAX));
        hash.write_usize(summary.total_lines.unwrap_or(usize::MAX));
        hash.write_u8(u8::from(summary.truncated));
        hash.write_u8(u8::from(summary.partial));
        hash.write_u8(u8::from(summary.counts_known));
        hash.write_u8(u8::from(summary.simplified));
        hash.write_usize(summary.omitted_files);
    } else {
        hash.write_u8(0);
    }
    hash.write_str(call.error.as_deref().unwrap_or(""));
    // 参数 JSON：文件工具的摘要缺字段时会回退解析它（首尾字节限住成本）。
    hash.write_usize(call.arguments_json.len());
    hash.write_edges(&call.arguments_json);
    match call.detail.as_deref() {
        None => hash.write_u8(0),
        Some(ToolDetailView::Mcp(view)) => {
            hash.write_u8(8);
            hash.write_str(&view.server);
            hash.write_str(&view.tool);
            hash.write_str(&view.arguments.join("\n"));
            hash.write_str(&view.result.join("\n"));
            hash.write_usize(view.result_more);
            hash.write_usize(view.arguments_more);
            hash.write_usize(
                view.duration_ms
                    .map(|duration| duration as usize)
                    .unwrap_or(0),
            );
            hash.write_usize(view.images.len());
            hash.write_usize(view.images_omitted);
        }
        Some(ToolDetailView::Subagent(view)) => {
            hash.write_u8(7);
            hash.write_str(view.role.map_or("", |role| role.as_str()));
            hash.write_str(view.title.as_deref().unwrap_or(""));
            hash.write_str(view.task.as_deref().unwrap_or(""));
            hash.write_str(view.agent_id.as_deref().unwrap_or(""));
            hash.write_str(view.agent_status.as_deref().unwrap_or(""));
            hash.write_usize(view.targets.len());
            for target in &view.targets {
                hash.write_str(target);
            }
            hash.write_u8(u8::from(view.all));
            hash.write_str(view.mode.as_str());
            hash.write_u64(view.timeout_ms.unwrap_or(0));
            hash.write_u8(u8::from(view.timed_out));
            hash.write_usize(view.agents.len());
            for agent in &view.agents {
                hash.write_str(&agent.agent_id);
                hash.write_str(&agent.title);
                hash.write_str(agent.role.as_str());
                hash.write_str(&agent.status);
                if let Some(report) = &agent.report {
                    hash.write_u8(1);
                    hash.write_str(report);
                } else {
                    hash.write_u8(0);
                }
            }
            if let Some(report) = &view.report {
                hash.write_u8(1);
                hash.write_str(&report.agent_id);
                hash.write_str(&report.title);
                hash.write_str(report.role.as_str());
                hash.write_str(&report.status);
                hash.write_str(&report.body);
            } else {
                hash.write_u8(0);
            }
            hash.write_str(view.closed_status.as_deref().unwrap_or(""));
        }
        Some(ToolDetailView::Command(view)) => {
            hash.write_u8(1);
            hash.write_str(&view.command);
            hash.write_str(&view.cwd);
            hash.write_u8(u8::from(view.tty));
            process_state_signature(hash, view.state);
            hash.write_u8(u8::from(view.truncated));
            hash.write_u64(view.dropped_bytes);
            hash.write_usize(view.output_tail.len());
            hash.write_edges(&view.output_tail);
            hash.write_i64(view.started_at_ms.unwrap_or(-1));
            hash.write_i64(view.ended_at_ms.unwrap_or(-1));
            hash.write_u64(view.duration_ms.unwrap_or(u64::MAX));
            hash.write_u64(view.process_session_id.unwrap_or(u64::MAX));
        }
        Some(ToolDetailView::Interaction(view)) => {
            hash.write_u8(2);
            hash.write_str(view.action.as_str());
            hash.write_u64(view.process_session_id);
            hash.write_str(&view.command);
            hash.write_str(view.input_preview.as_deref().unwrap_or(""));
            hash.write_usize(view.output.len());
            hash.write_edges(&view.output);
            process_state_signature(hash, view.state);
            hash.write_i64(view.exit_code.map_or(-1, i64::from));
            hash.write_u64(view.duration_ms.unwrap_or(u64::MAX));
            hash.write_u8(u8::from(view.truncated));
            hash.write_u64(view.dropped_bytes);
        }
        Some(ToolDetailView::Plan(view)) => {
            hash.write_u8(3);
            hash.write_u64(view.revision);
            hash.write_str(view.explanation.as_deref().unwrap_or(""));
            hash.write_usize(view.steps.len());
            for (step, status) in &view.steps {
                hash.write_str(step);
                hash.write_u8(plan_status_code(*status));
            }
        }
        Some(ToolDetailView::Ask(view)) => {
            // `ask` 的行数取决于题干/答案的折行，文本必须全进指纹（题干限 500 字）。
            hash.write_u8(5);
            hash.write_usize(view.questions.len());
            for question in &view.questions {
                hash.write_str(&question.header);
                hash.write_str(&question.question);
                hash.write_usize(question.answers.len());
                for answer in &question.answers {
                    hash.write_str(answer);
                }
            }
        }
        Some(ToolDetailView::Search(view)) => {
            hash.write_u8(9);
            hash.write_u8(u8::from(view.truncated));
            hash.write_usize(view.files_scanned);
            hash.write_usize(view.matches.len());
            for hit in &view.matches {
                hash.write_str(&hit.path);
                hash.write_usize(hit.line);
                hash.write_str(&hit.text);
                hash.write_u8(u8::from(hit.line_truncated));
            }
        }
        Some(ToolDetailView::WebSearch(view)) => {
            hash.write_u8(10);
            hash.write_str(&view.search_type);
            hash.write_usize(view.num_results);
            hash.write_str(view.provider.as_deref().unwrap_or(""));
            // 逐行写长度与首尾字节：不 `join`、不扫全文，仍能区分行边界变化。
            hash.write_usize(view.result.len());
            for line in &view.result {
                hash.write_usize(line.len());
                hash.write_edges(line);
            }
            hash.write_usize(view.result_more);
            hash.write_u64(view.duration_ms.unwrap_or(u64::MAX));
            hash.write_u8(u8::from(view.truncated));
            hash.write_usize(view.queries.len());
            for query in &view.queries {
                hash.write_str(query);
            }
        }
        Some(ToolDetailView::File(view)) => {
            hash.write_u8(4);
            hash.write_usize(view.files.len());
            hash.write_usize(view.files_total);
            hash.write_usize(view.added.unwrap_or(usize::MAX));
            hash.write_usize(view.removed.unwrap_or(usize::MAX));
            hash.write_u8(u8::from(view.simplified));
            hash.write_usize(view.omitted_files);
            hash.write_u8(u8::from(view.single));
            for file in &view.files {
                hash.write_str(&file.path);
                hash.write_str(file.to.as_deref().unwrap_or(""));
                file_kind_signature(hash, &file.kind);
                hash.write_usize(file.added.unwrap_or(usize::MAX));
                hash.write_usize(file.removed.unwrap_or(usize::MAX));
                hash.write_u8(u8::from(file.counts_known));
                hash.write_u8(u8::from(file.simplified));
                match &file.diff {
                    None => hash.write_u8(0),
                    Some(crate::app::state::FileDiffView::Unavailable { reason }) => {
                        hash.write_u8(1);
                        hash.write_str(reason);
                    }
                    Some(
                        crate::app::state::FileDiffView::Complete { hunks, simplified }
                        | crate::app::state::FileDiffView::Truncated { hunks, simplified },
                    ) => {
                        // 缓存的 `Legacy` 块存的是**行数**，而差异正文的行数取决于
                        // 每行文本按列宽折行后的物理行数。只哈希 hunk/行数会让
                        // 「同形状、不同文本宽度」的两份差异撞指纹 → 命中旧块 → 行数
                        // 停在旧值，后续块整体错位。所以每行还要写文本长度与 kind：
                        // 仍是 O(行数) 的整数哈希，不扫全文。
                        hash.write_u8(2);
                        hash.write_u8(u8::from(*simplified));
                        hash.write_usize(hunks.len());
                        for hunk in hunks {
                            hash.write_usize(hunk.lines.len());
                            for line in &hunk.lines {
                                hash.write_usize(line.text.len());
                                hash.write_u8(match line.kind {
                                    crate::chat::tools::DiffLineKind::Context => 0,
                                    crate::chat::tools::DiffLineKind::Added => 1,
                                    crate::chat::tools::DiffLineKind::Removed => 2,
                                });
                            }
                        }
                    }
                }
            }
        }
    }
}

/// 进程状态（含退出码，标题会显示 `exit N`）。
fn process_state_signature(hash: &mut Fnv, state: crate::app::state::ProcessStateView) {
    hash.write_str(state.as_str());
    hash.write_i64(state.exit_code().map_or(-1, i64::from));
}

/// 文件变更种类（`Moved` 的目标路径会出现在文件头）。
fn file_kind_signature(hash: &mut Fnv, kind: &crate::chat::tools::FileChangeKind) {
    match kind {
        crate::chat::tools::FileChangeKind::Added => hash.write_u8(0),
        crate::chat::tools::FileChangeKind::Updated => hash.write_u8(1),
        crate::chat::tools::FileChangeKind::Deleted => hash.write_u8(2),
        crate::chat::tools::FileChangeKind::Moved { to } => {
            hash.write_u8(3);
            hash.write_str(&to.to_string_lossy());
        }
    }
}

fn tool_status_code(status: crate::app::state::ToolStatus) -> u8 {
    match status {
        crate::app::state::ToolStatus::InputStreaming => 0,
        crate::app::state::ToolStatus::Pending => 1,
        crate::app::state::ToolStatus::Running => 2,
        crate::app::state::ToolStatus::Completed => 3,
        crate::app::state::ToolStatus::Failed => 4,
        crate::app::state::ToolStatus::Cancelled => 5,
        crate::app::state::ToolStatus::Interrupted => 6,
    }
}

fn plan_status_code(status: crate::app::state::PlanStepView) -> u8 {
    match status {
        crate::app::state::PlanStepView::Pending => 0,
        crate::app::state::PlanStepView::InProgress => 1,
        crate::app::state::PlanStepView::Completed => 2,
    }
}
