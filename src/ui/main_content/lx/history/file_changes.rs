//! 消息尾部「本轮文件变更」摘要树。
//!
//! ```text
//!   ├─ edit src/a.rs (+1 -1)
//!   │  └─ 12 + let x = 1;
//!   ├─ del src/b.rs (-3)
//!   └─ add src/c.rs (+5)
//! ```
//!
//! - 只聚合当前 assistant 消息自己的 `edit` / `write` 结果；
//! - 同路径多次修改合并，`+/-` 求和；计数未知显示 `(?)`；
//! - 超过 5 个文件默认折叠，点击 `… N more files` 展开；
//! - 点击文件行就地展开该文件 diff（复用文件工具卡的正文渲染）；
//! - 行数与点击几何在同一构造器里产出，保证严格一致。

use ratatui::layout::Rect;
use unicode_width::UnicodeWidthStr;

use crate::app::state::{AssistantStepView, ChatMessageView, FileChangeView, FileDiffView};
use crate::chat::tools::FileChangeKind;
use crate::ui::main_content::lx::history::tool::{self, Segment};
use crate::ui::{style, text};

use super::super::Grid;
use super::tool::file_diff::{self, Row};
use super::visible_row;

/// 折叠态最多显示的文件数。
const MAX_VISIBLE: usize = 5;
/// 列表折叠槽位。
pub(crate) const MORE_SLOT: &str = "files:more";

/// 一条聚合后的文件变更。
#[derive(Debug, Clone, PartialEq, Eq)]
struct FileChangeSummary {
    path: String,
    to: Option<String>,
    kind: FileChangeKind,
    added: Option<usize>,
    removed: Option<usize>,
    counts_known: bool,
    diff: Option<FileDiffView>,
}

/// 一个文件行的点击区（相对卡首）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FileChangeSpan {
    pub(crate) slot: String,
    pub(crate) offset: usize,
    pub(crate) rows: usize,
    pub(crate) clickable: bool,
}

/// 一张文件变更摘要卡（行 + 点击几何同源）。
#[derive(Debug)]
pub(crate) struct FileChangesCard {
    rows: Vec<Row>,
    spans: Vec<FileChangeSpan>,
}

impl FileChangesCard {
    /// 当前消息是否有可展示的文件变更（廉价检查，供缓存未命中前门控）。
    pub(crate) fn has_changes(message: &ChatMessageView) -> bool {
        message.steps.iter().any(|step| {
            let AssistantStepView::Tool { call, .. } = step else {
                return false;
            };
            call.file_view().is_some_and(|view| !view.files.is_empty())
        })
    }

    /// 构造卡片；没有变更或宽度为 0 时返回空卡。
    pub(crate) fn new(
        message: &ChatMessageView,
        width: u16,
        chat: &crate::app::state::LxChat,
    ) -> Self {
        if width == 0 {
            return Self {
                rows: Vec::new(),
                spans: Vec::new(),
            };
        }
        let changes = summarize(message);
        if changes.is_empty() {
            return Self {
                rows: Vec::new(),
                spans: Vec::new(),
            };
        }
        let list_expanded = chat.detail_is_expanded(&message.id, MORE_SLOT);
        let visible = if list_expanded || changes.len() <= MAX_VISIBLE {
            changes.len()
        } else {
            MAX_VISIBLE
        };
        let mut rows = Vec::new();
        let mut spans = Vec::new();
        for (index, change) in changes.iter().take(visible).enumerate() {
            let slot = format!("file:{}", change.path);
            let expanded = chat.detail_is_expanded(&message.id, &slot);
            let has_diff = change.diff.is_some();
            let mut body = if expanded {
                body_for(change, width)
            } else {
                Vec::new()
            };
            let has_body = !body.is_empty();
            let more_row = index + 1 == visible && changes.len() > visible;
            let last = index + 1 == visible && !more_row;
            let connector = if !last || has_body {
                text::agent::TREE_BRANCH
            } else {
                text::agent::TREE_LEAF
            };
            let header = change_header(change, connector, width);
            let offset = rows.len();
            rows.push(Row {
                segments: header,
                fill: None,
            });
            if has_body {
                file_diff::apply_body_prefixes(&mut body, last, width);
                rows.extend(body);
            }
            spans.push(FileChangeSpan {
                slot,
                offset,
                rows: rows.len() - offset,
                clickable: has_diff,
            });
        }
        if changes.len() > visible {
            let remaining = changes.len() - visible;
            rows.push(Row {
                segments: vec![
                    Segment::new(text::agent::TREE_LEAF, style::chat_notice_connector()),
                    Segment::new(
                        format!(
                            "{} {remaining} {}",
                            text::agent::FILE_CHANGES_MORE_PREFIX,
                            if remaining == 1 {
                                text::agent::FILE_CHANGES_MORE_FILE
                            } else {
                                text::agent::FILE_CHANGES_MORE_FILES
                            }
                        ),
                        style::fold_hint(),
                    ),
                ],
                fill: None,
            });
            spans.push(FileChangeSpan {
                slot: MORE_SLOT.to_string(),
                offset: rows.len() - 1,
                rows: 1,
                clickable: true,
            });
        }
        Self { rows, spans }
    }

    /// 卡片占用的物理行数。
    pub(crate) fn rows(&self) -> usize {
        self.rows.len()
    }

    /// 点击区。
    pub(crate) fn click_spans(&self) -> &[FileChangeSpan] {
        &self.spans
    }

    /// 绘制并推进行号；行数恒等于 [`Self::rows`]。
    pub(crate) fn draw(&self, grid: &mut Grid, rect: Rect, scroll: usize, row_index: &mut usize) {
        for row in &self.rows {
            if let Some(y) = visible_row(rect, scroll, *row_index) {
                if let Some(fill) = row.fill {
                    grid.background_row(rect.x, y, rect.width, fill);
                    let tinted: Vec<Segment> = row
                        .segments
                        .iter()
                        .map(|segment| {
                            Segment::new(segment.text.clone(), segment.style.patch(fill))
                        })
                        .collect();
                    tool::put_segments(grid, rect.x, y, rect.width, &tinted);
                } else {
                    tool::put_segments(grid, rect.x, y, rect.width, &row.segments);
                }
            }
            *row_index += 1;
        }
    }
}

/// 文件行：`  ├─ edit path (+1 -1)`。
fn change_header(change: &FileChangeSummary, connector: &str, width: u16) -> Vec<Segment> {
    let limit = usize::from(width);
    let used = connector.width();
    let verb = match &change.kind {
        FileChangeKind::Added => text::agent::CHANGE_ADD,
        FileChangeKind::Deleted => text::agent::CHANGE_DEL,
        FileChangeKind::Updated | FileChangeKind::Moved { .. } => text::agent::CHANGE_EDIT,
    };
    let verb_style = match &change.kind {
        FileChangeKind::Added => style::agent_added(),
        FileChangeKind::Deleted => style::agent_removed(),
        FileChangeKind::Updated | FileChangeKind::Moved { .. } => style::chat_tool_param(),
    };
    let mut segments = vec![
        Segment::new(connector, style::chat_notice_connector()),
        Segment::new(verb, verb_style),
        Segment::new(" ", style::text()),
    ];
    let raw_path = match &change.kind {
        FileChangeKind::Moved { .. } => {
            let to = change.to.as_deref().unwrap_or("");
            format!("{}{}{to}", change.path, text::agent::FILE_DIFF_MOVE_SEP)
        }
        _ => change.path.clone(),
    };
    let path = tool::single_line(&raw_path, 4096);
    let counts = counts_segments(change);
    let counts_cols: usize = counts.iter().map(Segment::cols).sum();
    let room = limit.saturating_sub(used + verb.len() + 1);
    if room == 0 {
        return segments;
    }
    let with_counts = room.saturating_sub(1 + counts_cols);
    let keep_counts = counts_cols > 0 && room > 1 + counts_cols && with_counts >= 8;
    let path_room = if keep_counts { with_counts } else { room };
    let path = text::ellipsize_middle(&path, path_room);
    if !path.is_empty() {
        segments.push(Segment::new(path, style::chat_tool_param()));
    }
    if keep_counts {
        segments.push(Segment::new(" ", style::text()));
        segments.extend(counts);
    }
    segments
}

/// 增删计数：`(+A -R)` / `(+A)` / `(-R)` / `(?)`。
fn counts_segments(change: &FileChangeSummary) -> Vec<Segment> {
    let unknown = || {
        vec![Segment::new(
            text::agent::FILE_DIFF_COUNTS_UNKNOWN,
            style::agent_step_detail(),
        )]
    };
    if !change.counts_known {
        return unknown();
    }
    let mut segments = Vec::new();
    match &change.kind {
        FileChangeKind::Added => {
            let Some(added) = change.added else {
                return unknown();
            };
            segments.push(Segment::new(
                format!("({}", text::agent::added_label(added)),
                style::agent_added(),
            ));
            segments.push(Segment::new(")", style::text()));
        }
        FileChangeKind::Deleted => {
            let Some(removed) = change.removed else {
                return unknown();
            };
            segments.push(Segment::new(
                format!("({})", text::agent::removed_label(removed)),
                style::agent_removed(),
            ));
        }
        FileChangeKind::Updated | FileChangeKind::Moved { .. } => {
            let (Some(added), Some(removed)) = (change.added, change.removed) else {
                return unknown();
            };
            segments.push(Segment::new(
                format!("({}", text::agent::added_label(added)),
                style::agent_added(),
            ));
            segments.push(Segment::new(" ", style::text()));
            segments.push(Segment::new(
                format!("{})", text::agent::removed_label(removed)),
                style::agent_removed(),
            ));
        }
    }
    segments
}

/// 聚合当前消息自己的文件工具结果。
fn summarize(message: &ChatMessageView) -> Vec<FileChangeSummary> {
    let mut changes: Vec<FileChangeSummary> = Vec::new();
    for step in &message.steps {
        let AssistantStepView::Tool { call, .. } = step else {
            continue;
        };
        let Some(view) = call.file_view() else {
            continue;
        };
        for file in &view.files {
            merge_file(&mut changes, file);
        }
    }
    changes
}

fn merge_file(changes: &mut Vec<FileChangeSummary>, file: &FileChangeView) {
    if let Some(existing) = changes.iter_mut().find(|item| item.path == file.path) {
        // 最终状态优先：任一删除 → del；否则首个新增 → add；否则移动 → edit old→new；否则 edit。
        existing.kind = if matches!(&file.kind, FileChangeKind::Deleted)
            || matches!(&existing.kind, FileChangeKind::Deleted)
        {
            FileChangeKind::Deleted
        } else if matches!(&existing.kind, FileChangeKind::Added) {
            existing.kind.clone()
        } else if matches!(&file.kind, FileChangeKind::Added) {
            FileChangeKind::Added
        } else {
            file.kind.clone()
        };
        if file.to.is_some() {
            existing.to = file.to.clone();
        }
        existing.added = sum_opt(existing.added, file.added);
        existing.removed = sum_opt(existing.removed, file.removed);
        existing.counts_known &= file.counts_known;
        if file.diff.is_some() {
            existing.diff = file.diff.clone();
        }
        return;
    }
    changes.push(FileChangeSummary {
        path: file.path.clone(),
        to: file.to.clone(),
        kind: file.kind.clone(),
        added: file.added,
        removed: file.removed,
        counts_known: file.counts_known,
        diff: file.diff.clone(),
    });
}

fn sum_opt(left: Option<usize>, right: Option<usize>) -> Option<usize> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.saturating_add(right)),
        (Some(value), None) | (None, Some(value)) => Some(value),
        (None, None) => None,
    }
}

/// 文件正文（预览/展开）复用文件工具卡的渲染。
fn body_for(change: &FileChangeSummary, width: u16) -> Vec<Row> {
    let view = FileChangeView {
        path: change.path.clone(),
        to: change.to.clone(),
        kind: change.kind.clone(),
        added: change.added,
        removed: change.removed,
        counts_known: change.counts_known,
        simplified: false,
        diff: change.diff.clone(),
    };
    file_diff::file_body(&view, width, true)
}
