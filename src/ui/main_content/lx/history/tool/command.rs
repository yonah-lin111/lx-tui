//! `exec_command` / `write_stdin` 卡片：标题状态 + 命令 + 已保存输出。
//!
//! ```text
//! ■ Run command · Exited 0 · 241ms
//!   ├─ $ cargo test
//!   └─ test one ... ok
//!      … 18 more lines
//! ```
//!
//! - **标题右侧**放进程状态与耗时（`Exited 0 · 241ms` / `Running`），不再占树体行；
//! - 命令参数与输出各是一块：命令是首节点，输出正文是最后一个多行节点（首行 `└─`，
//!   其余行左对齐）；两个折叠块共用同一套连接符，不重复 `├─`/`└─`；
//! - **长任务（进程仍存活）只给一行引导**：命令 + `open task panel · live output`，
//!   消息列表不铺实时输出；整卡点击打开面板并定位该进程；
//! - 执行阶段已截断（`truncated` / `dropped_bytes`）另列事实行，不混进
//!   `… N more lines` 的 UI 省略数；
//! - 卡片只读快照：不解析 JSON、不读进程、不读数据库。

use std::borrow::Cow;

use ratatui::layout::Rect;

use crate::app::state::{
    InteractionAction, ProcessStateView, ToolCallView, ToolDetailView, ToolStatus,
};
use crate::ui::{style, text};

use super::super::{Grid, StepMark, visible_row, wrap};
use super::file_summary::{DETAIL_MAX_ROWS, DETAIL_PREVIEW_ROWS};
use super::tree::{self, Node, Preview, Section};
use super::{Folds, Segment, header_with_verb, mark_of, output_style, single_line, title_rows};

/// 单行取样上限（限住每帧成本）。
const SAMPLE_CHARS: usize = 512;
/// 终态折叠预览的物理行：首 [`OUTPUT_PREVIEW_HEAD`] + 尾 [`OUTPUT_PREVIEW_TAIL`]。
const OUTPUT_PREVIEW_HEAD: usize = 3;
const OUTPUT_PREVIEW_TAIL: usize = 2;

/// 树体两块的行数（标题之外）：命令参数与「输入 + 输出」块。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct Blocks {
    pub(super) params: usize,
    pub(super) output: usize,
}

/// 命令卡片占用的渲染行数：标题 + 树体物理行。
pub(super) fn rows(call: &ToolCallView, width: u16, folds: Folds) -> usize {
    if width == 0 {
        return 0;
    }
    let blocks = blocks(call, width, folds);
    title_rows() + blocks.params + blocks.output
}

/// 树体两块各自的高度（与 [`draw`] 共用同一份构造）。
pub(super) fn blocks(call: &ToolCallView, width: u16, folds: Folds) -> Blocks {
    if width == 0 {
        return Blocks::default();
    }
    let (params, output) = sections(call, width, folds);
    Blocks {
        params: tree::layout(&params, width).rows.len(),
        output: tree::layout(&output, width).rows.len(),
    }
}

/// 命令参数区展开后是否**真的多出内容**。
pub(super) fn params_expandable(call: &ToolCallView, width: u16) -> bool {
    if width == 0 {
        return false;
    }
    // 折叠态（preview）才有隐藏行；`false` 表示展开，不能用来判定可展开。
    tree::layout(&[param_section(call, true)], width).hidden > 0
}

/// 输出区展开后是否**真的多出内容**。
pub(super) fn output_expandable(call: &ToolCallView, width: u16) -> bool {
    if width == 0 {
        return false;
    }
    let (_, output) = sections(call, width, Folds::default());
    output_section_hidden(&output, width)
}

fn output_section_hidden(sections: &[Section], width: u16) -> bool {
    sections.iter().any(|section| {
        !section.nodes.is_empty() && tree::layout(std::slice::from_ref(section), width).hidden > 0
    })
}

/// 取出命令文本（`exec_command` 与 `write_stdin` 共用）。
fn command_of(call: &ToolCallView) -> Option<&str> {
    match call.detail.as_deref()? {
        ToolDetailView::Command(view) => Some(view.command.as_str()),
        ToolDetailView::Interaction(view) => Some(view.command.as_str()),
        _ => None,
    }
}

/// 标题右侧状态：`Exited 0 · 241ms` / `Running` / `Interrupted · 302ms`。
fn title_status(state: ProcessStateView, duration_ms: Option<u64>) -> String {
    let mut parts = Vec::new();
    if state.is_running() {
        parts.push(text::agent::proc_state_label(state).to_string());
    } else {
        parts.push(text::agent::proc_exit_label(state));
        if let Some(duration) = duration_ms {
            parts.push(text::agent::duration_label(duration));
        }
    }
    parts.join(text::LX_CHAT_TOOL_STATUS_SEP)
}

/// 命令参数块：`$ 命令`；运行中把引导语作为同一节点的续行。
fn param_section(call: &ToolCallView, folded: bool) -> Section {
    let Some(command) = command_of(call) else {
        return Section::new(Vec::new(), None);
    };
    let text = if command.starts_with(text::agent::COMMAND_PROMPT_PREFIX) {
        command.to_string()
    } else {
        format!("{}{command}", text::agent::COMMAND_PROMPT_PREFIX)
    };
    // 命令是用户/模型给出的单个字段：折行与省略都由 tree 统一算，保证
    // `… N more lines` 与展开后的真实行数一致。
    let text = single_line(&text, SAMPLE_CHARS * DETAIL_MAX_ROWS);
    let mut lines = vec![vec![Segment::new(text, style::chat_tool_param())]];
    if let Some(ToolDetailView::Command(view)) = call.detail.as_deref()
        && view.state.is_running()
    {
        lines.push(vec![Segment::new(
            text::agent::TASK_OPEN_GUIDE,
            output_style(false),
        )]);
    }
    Section::new(
        vec![Node::block(lines)],
        folded.then(|| Preview::head(DETAIL_PREVIEW_ROWS)),
    )
}

/// 输出块：已保存输出按列宽折行，整块是一个多行节点；折叠首尾预览。
fn output_section(
    output: &str,
    width: u16,
    folded: bool,
    truncated: bool,
    dropped: u64,
) -> Option<Section> {
    let body_width = tree::content_width(width);
    let mut lines: Vec<Vec<Segment>> =
        output_lines(output, u16::try_from(body_width).unwrap_or(u16::MAX))
            .into_iter()
            .map(|line| vec![Segment::new(line, output_style(!folded))])
            .collect();
    if truncated {
        lines.push(vec![Segment::new(
            text::agent::OUTPUT_LIMITED,
            style::agent_warn(),
        )]);
    }
    if dropped > 0 {
        lines.push(vec![Segment::new(
            text::agent::dropped_bytes_label(dropped),
            style::agent_warn(),
        )]);
    }
    if lines.is_empty() {
        return None;
    }
    Some(Section::new(
        vec![Node::block(lines)],
        folded.then(|| Preview::head_tail(OUTPUT_PREVIEW_HEAD, OUTPUT_PREVIEW_TAIL)),
    ))
}

/// 输入预览的转义还原（`preview_input` 转义 `\\`/`\n`/`\r`/`\t`/`\u{hex}` 并在超长时追加 `…`）。
fn unescape_input(preview: &str) -> (String, bool) {
    let (body, truncated) = match preview.strip_suffix('…') {
        Some(prefix) => (prefix, true),
        None => (preview, false),
    };
    let mut out = String::with_capacity(body.len());
    let mut chars = body.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('t') => out.push('\t'),
            Some('\\') => out.push('\\'),
            Some('u') if chars.peek() == Some(&'{') => {
                let _ = chars.next();
                let mut hex = String::new();
                while let Some(&digit) = chars.peek() {
                    let _ = chars.next();
                    if digit == '}' {
                        break;
                    }
                    hex.push(digit);
                }
                if let Ok(code) = u32::from_str_radix(&hex, 16)
                    && let Some(decoded) = char::from_u32(code)
                {
                    out.push(decoded);
                }
            }
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    (out, truncated)
}

/// 去掉输出里对输入的单次原样回显：`cat` 这类进程会同时给出 input 与 echo。
///
/// 每条输入行最多按序消费一次回显（若预览末行被 `…` 截断则按前缀匹配一次）；
/// 回显之后的真实响应（含再次输出同文内容）仍然保留。
fn without_echoed_input(input_preview: &str, output: &str) -> String {
    let (input, truncated) = unescape_input(input_preview);
    let raw_lines: Vec<&str> = input.lines().collect();
    let last_index = raw_lines.len().saturating_sub(1);
    let mut pending: Vec<(&str, bool)> = raw_lines
        .into_iter()
        .enumerate()
        .map(|(index, line)| (line.trim_end(), truncated && index == last_index))
        .filter(|(line, _)| !line.is_empty())
        .collect();
    if pending.is_empty() {
        return output.to_string();
    }
    let mut kept = Vec::new();
    for line in output.lines() {
        let trimmed = line.trim_end();
        if let Some(pos) = pending.iter().position(|(echo, prefix_match)| {
            if *prefix_match {
                trimmed.starts_with(echo)
            } else {
                trimmed == *echo
            }
        }) {
            pending.remove(pos);
            continue;
        }
        kept.push(line);
    }
    kept.join("\n")
}

/// 卡片的两块（参数 / 输入+输出），与 [`draw`] 共用。
fn sections(call: &ToolCallView, width: u16, folds: Folds) -> (Vec<Section>, Vec<Section>) {
    let params = vec![param_section(call, !folds.body)];
    let mut output = Vec::new();
    match call.detail.as_deref() {
        Some(ToolDetailView::Command(view)) => {
            if !view.state.is_running()
                && let Some(section) = output_section(
                    &view.output_tail,
                    width,
                    !folds.output,
                    view.truncated,
                    view.dropped_bytes,
                )
            {
                output.push(section);
            }
        }
        Some(ToolDetailView::Interaction(view)) => {
            let input = if view.action == InteractionAction::SendInput {
                view.input_preview
                    .as_deref()
                    .filter(|input| !input.is_empty())
            } else {
                None
            };
            if let Some(input) = input {
                output.push(Section::new(
                    vec![Node::text(
                        single_line(input, SAMPLE_CHARS * DETAIL_MAX_ROWS),
                        output_style(folds.output),
                    )],
                    (!folds.output)
                        .then(|| Preview::head_tail(OUTPUT_PREVIEW_HEAD, OUTPUT_PREVIEW_TAIL)),
                ));
            }
            let body: Cow<'_, str> = match input {
                Some(input) => Cow::Owned(without_echoed_input(input, &view.output)),
                None => Cow::Borrowed(view.output.as_str()),
            };
            if let Some(section) = output_section(
                &body,
                width,
                !folds.output,
                view.truncated,
                view.dropped_bytes,
            ) {
                output.push(section);
            }
        }
        _ => {}
    }
    (params, output)
}

/// 输出正文按列宽折行（末尾空行不占位）。
fn output_lines(output: &str, width: u16) -> Vec<String> {
    if output.is_empty() {
        return Vec::new();
    }
    let mut rows: Vec<String> = Vec::new();
    for line in output.split('\n') {
        let line = line.trim_end_matches('\r');
        if line.is_empty() {
            rows.push(String::new());
            continue;
        }
        let line = super::sample_output_line(line);
        rows.extend(wrap(&line, width));
    }
    while rows.last().is_some_and(String::is_empty) {
        rows.pop();
    }
    rows
}

/// 绘制命令或交互卡片；推进的行数必须与 [`rows`] 严格相等。
#[allow(clippy::too_many_arguments)]
pub(super) fn draw(
    grid: &mut Grid,
    rect: Rect,
    scroll: usize,
    row_index: &mut usize,
    call: &ToolCallView,
    loading_frame: usize,
    folds: Folds,
    animating: bool,
) {
    if rect.width == 0 {
        return;
    }
    let (verb, state, duration) = match call.detail.as_deref() {
        Some(ToolDetailView::Command(view)) => {
            (text::agent::VERB_EXEC_COMMAND, view.state, view.duration_ms)
        }
        Some(ToolDetailView::Interaction(view)) => (
            text::agent::interaction_verb(view.action),
            view.state,
            view.duration_ms,
        ),
        _ => {
            if let Some(y) = visible_row(rect, scroll, *row_index) {
                let status = plain_status_label(call.status);
                let segments = header_with_verb(
                    call,
                    rect.width,
                    loading_frame,
                    animating,
                    text::agent::VERB_EXEC_COMMAND,
                    status,
                );
                super::put_segments(grid, rect.x, y, rect.width, &segments);
            }
            *row_index += 1;
            return;
        }
    };
    let mark = match call.detail.as_deref() {
        Some(ToolDetailView::Command(_)) | Some(ToolDetailView::Interaction(_)) => {
            process_mark(call, state, loading_frame)
        }
        _ => mark_of(call.status, loading_frame),
    };
    if let Some(y) = visible_row(rect, scroll, *row_index) {
        let segments = super::header_with_mark(
            call,
            rect.width,
            loading_frame,
            animating,
            verb,
            Some(title_status(state, duration)),
            mark,
        );
        super::put_segments(grid, rect.x, y, rect.width, &segments);
    }
    *row_index += 1;
    let (params, output) = sections(call, rect.width, folds);
    let mut all = params;
    all.extend(output);
    tree::draw(
        grid,
        rect,
        scroll,
        row_index,
        &tree::layout(&all, rect.width).rows,
    );
}

/// 进程存活时标题的橙色档位（橙 = 还在跑，绿 = 停了）。
fn process_mark(call: &ToolCallView, state: ProcessStateView, loading_frame: usize) -> StepMark {
    if state.is_running() && call.status != ToolStatus::Failed {
        StepMark::Alive
    } else {
        mark_of(call.status, loading_frame)
    }
}

/// 没有类型化详情时的状态词（例如只有失败错误的卡片）。
fn plain_status_label(status: ToolStatus) -> Option<String> {
    match status {
        ToolStatus::Completed | ToolStatus::Failed => None,
        other => Some(text::tool_status_label(other).to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::state::{
        CommandView, InteractionAction, ProcessInteractionView, ProcessStateView, ToolDetailView,
    };

    pub(super) fn command_call(
        state: ProcessStateView,
        output: &str,
        duration_ms: Option<u64>,
    ) -> ToolCallView {
        ToolCallView {
            call_id: "call-1".into(),
            name: "exec_command".into(),
            position: 0,
            arguments_json: "{}".into(),
            status: ToolStatus::Completed,
            summary: None,
            error: None,
            result_json: None,
            detail: Some(Box::new(ToolDetailView::Command(CommandView {
                process_session_id: Some(1),
                command: "cargo test".into(),
                cwd: "/w".into(),
                tty: false,
                state,
                duration_ms,
                output_preview: Vec::new(),
                truncated: false,
                dropped_bytes: 0,
                output_tail: output.to_string(),
                started_at_ms: None,
                ended_at_ms: None,
            }))),
        }
    }

    #[test]
    fn finished_command_is_a_tree_with_status_duration_and_output() {
        let call = command_call(
            ProcessStateView::Exited { code: Some(2) },
            "boom\n",
            Some(1_200),
        );
        let text = flat(&call, 60, Folds::default());
        let title = text.lines().next().unwrap_or("");
        assert!(title.contains("Run command"), "{text}");
        assert!(title.contains("exit 2"), "状态在标题右侧：{text}");
        assert!(title.contains("1.2s"), "耗时在标题右侧：{text}");
        assert!(text.contains("$ cargo test"), "{text}");
        assert!(text.contains("boom"), "{text}");
    }

    #[test]
    fn running_command_hides_output_and_points_at_the_panel() {
        let output: String = (0..20).map(|n| format!("line {n}\n")).collect();
        let mut call = command_call(ProcessStateView::Running, &output, None);
        call.status = ToolStatus::Running;
        let text = flat(&call, 60, Folds::default());
        assert!(text.contains("Running"), "{text}");
        assert!(text.contains(text::agent::TASK_OPEN_GUIDE), "{text}");
        assert!(!text.contains("line 0"), "{text}");
        assert!(!text.contains("more lines"), "{text}");
    }

    #[test]
    fn interaction_shows_saved_output_and_status() {
        let call = ToolCallView {
            call_id: "call-2".into(),
            name: "write_stdin".into(),
            position: 0,
            arguments_json: "{}".into(),
            status: ToolStatus::Completed,
            summary: None,
            error: None,
            result_json: None,
            detail: Some(Box::new(ToolDetailView::Interaction(
                ProcessInteractionView {
                    action: InteractionAction::ReadOutput,
                    process_session_id: 1,
                    command: "cargo test".into(),
                    input_preview: None,
                    output: "late output\n".into(),
                    state: ProcessStateView::Running,
                    exit_code: None,
                    duration_ms: None,
                    truncated: false,
                    dropped_bytes: 0,
                },
            ))),
        };
        let text = flat(&call, 60, Folds::default());
        assert!(text.contains(text::agent::VERB_READ_OUTPUT), "{text}");
        assert!(text.contains("Running"), "{text}");
        assert!(text.contains("late output"), "{text}");
        assert!(text.contains("$ cargo test"), "{text}");
    }

    #[test]
    fn output_preview_keeps_head_tail_and_counts_hidden_rows() {
        let output: String = (0..20).map(|n| format!("line {n}\n")).collect();
        let call = command_call(ProcessStateView::Exited { code: Some(0) }, &output, None);
        let text = flat(&call, 40, Folds::default());
        assert!(
            text.contains("line 0") && text.contains("line 19"),
            "{text}"
        );
        assert!(text.contains("more lines"), "{text}");
        assert!(output_expandable(&call, 40));
        assert!(
            !output_expandable(&call, 40)
                || rows(&call, 40, folds(true)) > rows(&call, 40, folds(false))
        );
    }

    /// 参数与输出各是一个节点：命令首行 `├─`、输出首行 `└─`，输出其余行左对齐。
    #[test]
    fn folded_blocks_use_per_node_connectors() {
        let output: String = (0..20).map(|n| format!("line {n}\n")).collect();
        let mut call = command_call(ProcessStateView::Exited { code: Some(0) }, &output, None);
        if let Some(ToolDetailView::Command(view)) = call.detail.as_deref_mut() {
            view.command = format!("echo {}", "y".repeat(300));
        }
        for expanded in [false, true] {
            let text = flat(
                &call,
                60,
                Folds {
                    body: expanded,
                    output: expanded,
                },
            );
            let rows: Vec<&str> = text.lines().collect();
            assert!(rows.len() > 4, "{text}");
            assert!(
                rows[1].starts_with(text::agent::TREE_BRANCH),
                "折叠 {expanded}：命令首行是分支：{text}"
            );
            let output_row = rows
                .iter()
                .position(|line| line.contains("line 0"))
                .unwrap_or_else(|| panic!("缺少输出首行：{text}"));
            assert!(
                rows[output_row].starts_with(text::agent::TREE_LEAF),
                "折叠 {expanded}：输出首行收口：{text}"
            );
            assert!(
                rows[output_row + 1..]
                    .iter()
                    .all(|line| line.starts_with(text::agent::TREE_PAD)),
                "折叠 {expanded}：输出续行左对齐：{text}"
            );
            assert_eq!(
                text.matches(text::agent::TREE_BRANCH).count(),
                1,
                "折叠 {expanded}：命令只占一个分支符：{text}"
            );
            assert_eq!(
                text.matches(text::agent::TREE_LEAF).count(),
                1,
                "折叠 {expanded}：输出只占一个收口符：{text}"
            );
        }
    }

    /// `Send input`：标题给状态，正文是命令 + 输入（输入是最后一个节点）。
    #[test]
    fn send_input_shows_the_input_after_the_command() {
        let call = ToolCallView {
            call_id: "call-3".into(),
            name: "write_stdin".into(),
            position: 0,
            arguments_json: "{}".into(),
            status: ToolStatus::Running,
            summary: None,
            error: None,
            result_json: None,
            detail: Some(Box::new(ToolDetailView::Interaction(
                ProcessInteractionView {
                    action: InteractionAction::SendInput,
                    process_session_id: 1,
                    command: "bc".into(),
                    input_preview: Some("2+3*4\\n".into()),
                    output: String::new(),
                    state: ProcessStateView::Running,
                    exit_code: None,
                    duration_ms: None,
                    truncated: false,
                    dropped_bytes: 0,
                },
            ))),
        };
        let text = flat(&call, 60, Folds::default());
        let rows: Vec<&str> = text.lines().collect();
        assert!(rows[0].contains("Send input"), "{text}");
        assert!(rows[0].contains("Running"), "{text}");
        assert!(rows[1].contains("$ bc"), "{text}");
        assert!(rows[1].starts_with(text::agent::TREE_BRANCH), "{text}");
        assert!(rows[2].contains("2+3*4"), "{text}");
        assert!(rows[2].starts_with(text::agent::TREE_LEAF), "{text}");
        let input_style = |expanded: bool| {
            let (_, output) = sections(
                &call,
                60,
                Folds {
                    body: false,
                    output: expanded,
                },
            );
            tree::layout(&output, 60).rows[0].segments[0].style
        };
        assert_eq!(input_style(false), style::muted(), "默认折叠态是灰色");
        assert_eq!(input_style(true), style::text(), "展开后是默认前景色");
    }

    /// TTY 回显与输入相同：只显示输入，不重复输出。
    #[test]
    fn send_input_does_not_repeat_the_echoed_input() {
        let call = ToolCallView {
            call_id: "call-4".into(),
            name: "write_stdin".into(),
            position: 0,
            arguments_json: "{}".into(),
            status: ToolStatus::Completed,
            summary: None,
            error: None,
            result_json: None,
            detail: Some(Box::new(ToolDetailView::Interaction(
                ProcessInteractionView {
                    action: InteractionAction::SendInput,
                    process_session_id: 1,
                    command: "cat".into(),
                    input_preview: Some("hello\\n".into()),
                    output: "hello\n".into(),
                    state: ProcessStateView::Interrupted,
                    exit_code: None,
                    duration_ms: Some(1_500),
                    truncated: false,
                    dropped_bytes: 0,
                },
            ))),
        };
        let text = flat(&call, 60, Folds::default());
        let title = text.lines().next().unwrap_or("");
        assert!(title.contains("Send input"), "{text}");
        assert!(
            title.contains("Interrupted") && title.contains("1.5s"),
            "{text}"
        );
        assert_eq!(
            text.matches("hello").count(),
            1,
            "回显与输入只保留一份：{text}"
        );
        assert!(text.contains("hello\\n"), "保留输入预览：{text}");
    }

    /// 回显之后还有真实响应：删掉回显行，保留响应。
    #[test]
    fn send_input_keeps_only_the_response_after_the_echo() {
        let call = ToolCallView {
            call_id: "call-5".into(),
            name: "write_stdin".into(),
            position: 0,
            arguments_json: "{}".into(),
            status: ToolStatus::Completed,
            summary: None,
            error: None,
            result_json: None,
            detail: Some(Box::new(ToolDetailView::Interaction(
                ProcessInteractionView {
                    action: InteractionAction::SendInput,
                    process_session_id: 1,
                    command: "cat".into(),
                    input_preview: Some("hello\\n".into()),
                    output: "hello\nworld\n".into(),
                    state: ProcessStateView::Running,
                    exit_code: None,
                    duration_ms: None,
                    truncated: false,
                    dropped_bytes: 0,
                },
            ))),
        };
        let text = flat(&call, 60, Folds::default());
        assert_eq!(text.matches("hello").count(), 1, "回显不重复：{text}");
        assert!(text.contains("world"), "后续响应仍显示：{text}");
    }

    #[test]
    fn truncation_facts_are_separate_nodes() {
        let mut call = command_call(ProcessStateView::Exited { code: Some(0) }, "tail\n", None);
        if let Some(ToolDetailView::Command(view)) = call.detail.as_deref_mut() {
            view.truncated = true;
            view.dropped_bytes = 512;
        }
        let text = flat(&call, 60, Folds::default());
        assert!(text.contains(text::agent::OUTPUT_LIMITED), "{text}");
        assert!(text.contains("earlier bytes dropped"), "{text}");
    }

    #[test]
    fn params_and_output_fold_independently() {
        let output: String = (0..20).map(|n| format!("line {n}\n")).collect();
        let mut call = command_call(ProcessStateView::Exited { code: Some(0) }, &output, None);
        if let Some(ToolDetailView::Command(view)) = call.detail.as_deref_mut() {
            view.command = format!("echo {}", "y".repeat(200));
        }
        let folded = rows(&call, 40, Folds::default());
        let params = rows(
            &call,
            40,
            Folds {
                body: true,
                output: false,
            },
        );
        let out = rows(
            &call,
            40,
            Folds {
                body: false,
                output: true,
            },
        );
        assert!(params > folded);
        assert!(out > folded);
        assert_ne!(params, out);
    }

    #[test]
    fn interaction_with_long_output_and_echo_dedupe_behaves_consistently() {
        let long_output: String = (0..10).map(|n| format!("line {n}\n")).collect();
        let call = ToolCallView {
            call_id: "call-int-expand".into(),
            name: "write_stdin".into(),
            position: 0,
            arguments_json: "{}".into(),
            status: ToolStatus::Completed,
            summary: None,
            error: None,
            result_json: None,
            detail: Some(Box::new(ToolDetailView::Interaction(
                ProcessInteractionView {
                    action: InteractionAction::ReadOutput,
                    process_session_id: 1,
                    command: "cat".into(),
                    input_preview: None,
                    output: long_output,
                    state: ProcessStateView::Running,
                    exit_code: None,
                    duration_ms: None,
                    truncated: false,
                    dropped_bytes: 0,
                },
            ))),
        };
        assert!(
            super::super::has_expandable_content(&call, 60),
            "长输出的 write_stdin 必须判定为可展开"
        );
        // 单行输入只消费一次回显：若输出里出现两次相同行（回显 + 真实输出），保留第二次。
        assert_eq!(without_echoed_input("hello\\n", "hello\nhello\n"), "hello");
        // 控制字符转义 `\u{3}` 与截断省略号 `…` 还原。
        assert_eq!(without_echoed_input("ab\\u{3}\\n", "ab\u{3}\nok\n"), "ok");
        assert_eq!(without_echoed_input("prefix…", "prefix_full_line\n"), "");
    }

    fn folds(expanded: bool) -> Folds {
        Folds {
            body: expanded,
            output: expanded,
        }
    }

    pub(super) fn flat(call: &ToolCallView, width: u16, folds: Folds) -> String {
        let rect = Rect::new(0, 0, width, 80);
        let mut grid = Grid::new(rect);
        let mut row = 0;
        draw(&mut grid, rect, 0, &mut row, call, 0, folds, false);
        assert_eq!(row, rows(call, width, folds));
        let mut text = String::new();
        for y in 0..u16::try_from(row).unwrap_or(0) {
            for x in 0..width {
                text.push(grid.cell(x, y).ch);
            }
            text.push('\n');
        }
        text
    }
}

#[cfg(test)]
mod line_bound_tests {
    use super::*;

    #[test]
    fn oversized_single_output_line_is_sampled_with_an_ellipsis() {
        let line = "x".repeat(super::super::MAX_OUTPUT_LINE_CHARS + 100);
        let call = command_call(
            ProcessStateView::Exited { code: Some(0) },
            &format!("{line}\n"),
            None,
        );
        let total = rows(&call, 80, Folds::default());
        assert!(total <= 200, "单行输出必须有界：{total} 行");
        let text = flat(&call, 80, Folds::default());
        assert!(text.contains('…'), "截断必须显式标出：{text}");
    }

    fn command_call(
        state: ProcessStateView,
        output: &str,
        duration_ms: Option<u64>,
    ) -> ToolCallView {
        super::tests::command_call(state, output, duration_ms)
    }

    pub(super) fn flat(call: &ToolCallView, width: u16, folds: Folds) -> String {
        super::tests::flat(call, width, folds)
    }
}
