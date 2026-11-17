//! 块绘制数据（二级缓存）：把每个内容块「排好的行」存下来跨帧复用。
//!
//! 每个变体只存**内容与静态样式**；步骤标记的呼吸字形、thinking 悬停色、用户消息 rewind
//! 选中底色、`loading_frame` 等动态内容一律在 [`BlockPaint::draw`] 时现算。
//!
//! `rows()` 与绘制共用同一份数据：快照行数、滚动条几何、点击面几何都从这里取。

use std::sync::Arc;

use ratatui::layout::Rect;
use ratatui::style::Style;
use unicode_width::UnicodeWidthChar;

use crate::app::state::{AssistantStepView, CardSection, ChatMessageView, ToolCallView};

use super::source::HistorySource;
use crate::ui::markdown_render::{MdLine, md_bytes};
use crate::ui::style;

use super::super::Grid;
use super::rows::{UserPasteSpan, UserSpanKind, UserTextLine};
use super::tool::{self, Folds};
use super::{CardCols, STEP_GUTTER, StepKind, StepMark, USER_INDENT, USER_PREFIX, visible_row};

mod legacy;
mod thinking;
use thinking::draw_thinking;
pub(crate) use thinking::thinking_header_label;

/// 块在 `Lx` 里的定位（绘制动态样式与回退路径时用）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BlockSource {
    /// 消息里的单个步骤（正文 / thinking / 单个工具）。
    Step {
        message_index: usize,
        step_index: usize,
    },
    /// 连续同名工具组的组头（一行，不登记点击面）。
    ToolGroupHeader {
        message_index: usize,
        first_step: usize,
        count: usize,
        snapshot: bool,
    },
    /// 合并组里的一个调用体（只画树体，不画标题）。
    ToolBody {
        message_index: usize,
        step_index: usize,
        group: GroupSlot,
    },
    /// 本地命令卡片（整条消息一个块）。
    Shell { message_index: usize },
    /// 消息底部通知。
    Notice { message_index: usize },
    /// 时间线里的撤销提示（合并后的最后一条 + 条数）。
    Rewind { last: usize, count: usize },
    /// 时间线里的 compact 检查点。
    Compact { index: usize, expanded: bool },
    /// 时间线里的正在压缩 loading 条目。
    RunningCompact,
    /// 时间线里的模型切换提示（合并后的起止下标与展开状态）。
    ModelSwitch {
        first: usize,
        last: usize,
        expanded: bool,
    },
    /// 时间线里的沙箱权限切换提示（合并后的起止下标与展开状态）。
    PermissionSwitch {
        first: usize,
        last: usize,
        expanded: bool,
    },
    /// 消息尾部的 recap 系统事件卡片。
    Recap { message_index: usize },
    /// 消息内的汇报块（第 `index` 份）。
    Report { message_index: usize, index: usize },
    /// 助手消息末尾的附件引用树。
    Attachments { message_index: usize },
    /// 本轮统计行。
    Stats { message_index: usize },
    /// recap 生成中的占位行。
    RecapRunning { message_index: usize },
}

/// 块内一个可点击区域（行偏移相对块首行）。
///
/// 它是 `(块内容, 宽度, 折叠态)` 的纯函数结果，因此和绘制数据存在一起、随同一个
/// `BlockKey` 失效：快照重建每帧都要登记点击面，重算一遍 `card_regions` 在 1400 张
/// 工具卡片的会话里要 4.7ms/帧（实测）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CardSpan {
    /// 该区域属于块内第几个调用：单工具块恒 0，合并组是组内下标。
    pub call: usize,
    pub section: CardSection,
    /// 该区域当前的展开状态（折叠态也进块指纹，因此能随同样的条件失效）。
    pub expanded: bool,
    pub offset: usize,
    pub rows: usize,
    pub cols: CardCols,
    pub clickable: bool,
}

/// 工具合并组内的位置：`index` 从 0 开始，`count` 是本组实际展示的调用数。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct GroupSlot {
    pub(crate) index: usize,
    pub(crate) count: usize,
}

impl GroupSlot {
    pub(crate) fn is_last(self) -> bool {
        self.index.saturating_add(1) >= self.count
    }
}

/// 消息下方单条引用树分支（用户 `@文件` 或助手生成图片）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UserRefLine {
    pub is_last: bool,
    pub kind: crate::db::chat::MessageAttachmentKind,
    pub label: String,
}

impl UserRefLine {
    /// 从消息附件元数据组装可见引用树行。
    ///
    /// 同一目标只保留首行：正文重复出现同一 `@引用` 时不重复展示。
    /// 去重只作用于文本引用（`marker_start` 有值）；草稿附件与文本引用仍是两行。
    pub(crate) fn from_attachments(
        attachments: &[crate::db::chat::MessageAttachmentMeta],
    ) -> Vec<Self> {
        let mut seen = std::collections::HashSet::new();
        let display: Vec<_> = attachments
            .iter()
            .filter(|att| {
                let key = (
                    att.kind.as_str(),
                    att.marker_start.is_some(),
                    display_target(att).trim_end_matches('/').to_string(),
                );
                seen.insert(key)
            })
            .collect();
        let len = display.len();
        display
            .into_iter()
            .enumerate()
            .map(|(i, att)| Self {
                is_last: i + 1 == len,
                kind: att.kind,
                label: display_label(att).to_string(),
            })
            .collect()
    }
}

/// 展示用目标：图片用原始 label（存储后的 source_path 是归档名，可能把不同原图折叠成同一路径），
/// 其余用 source_path（引用原文/规范化路径）。
fn display_target(att: &crate::db::chat::MessageAttachmentMeta) -> &str {
    match att.kind {
        crate::db::chat::MessageAttachmentKind::Image => &att.label,
        _ => att.source_path.as_deref().unwrap_or(&att.label),
    }
}

/// 展示用标签：旧版快捷引用 token 的 label 是内部标记，用 source_path 还原成可读路径。
fn display_label(att: &crate::db::chat::MessageAttachmentMeta) -> &str {
    if att.label.starts_with("@[")
        && let Some(path) = att.source_path.as_deref()
    {
        return path;
    }
    &att.label
}

/// 助手消息下方单条生成图片引用行（含预览标记、下划线文件名与原图下载按钮状态）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AssistantRefLine {
    pub attachment_id: String,
    pub is_last: bool,
    pub label: String,
    pub preview_path: Option<String>,
    pub download_state: Option<crate::app::state::OriginalImageDownloadState>,
}

impl AssistantRefLine {
    pub(crate) fn from_message(
        message: &crate::app::state::ChatMessageView,
        chat: &crate::app::state::LxChat,
    ) -> Vec<Self> {
        let images: Vec<_> = message
            .attachments
            .iter()
            .filter(|att| att.kind == crate::db::chat::MessageAttachmentKind::Image)
            .collect();
        let len = images.len();
        images
            .into_iter()
            .enumerate()
            .map(|(i, att)| Self {
                attachment_id: att.id.clone(),
                is_last: i + 1 == len,
                label: att.label.clone(),
                preview_path: att.source_path.clone().filter(|path| !path.is_empty()),
                download_state: chat
                    .image_originals
                    .get(&att.id)
                    .map(|orig| orig.state.clone()),
            })
            .collect()
    }

    pub(crate) fn button_text(&self) -> Option<&'static str> {
        match self.download_state.as_ref()? {
            crate::app::state::OriginalImageDownloadState::Idle => {
                Some(crate::ui::text::agent::ASSISTANT_IMAGE_DOWNLOAD_ORIGINAL)
            }
            crate::app::state::OriginalImageDownloadState::Downloading => {
                Some(crate::ui::text::agent::ASSISTANT_IMAGE_DOWNLOADING)
            }
            crate::app::state::OriginalImageDownloadState::Downloaded { .. } => {
                Some(crate::ui::text::agent::ASSISTANT_IMAGE_DOWNLOADED)
            }
        }
    }

    pub(crate) fn push_card_regions(
        &self,
        row: usize,
        cards: &mut Vec<super::snapshot::CardRegion>,
    ) {
        let conn_cols = USER_INDENT + 3;
        let prefix_cols =
            conn_cols + crate::ui::text::agent::ASSISTANT_REF_IMAGE_PREVIEW_PREFIX.len() as u16;
        let label_cols = u16::try_from(unicode_width::UnicodeWidthStr::width(self.label.as_str()))
            .unwrap_or(u16::MAX);
        if let Some(path) = &self.preview_path {
            cards.push(super::snapshot::CardRegion {
                kind: super::CardKind::Attachment { path: path.clone() },
                card_start: row,
                card_rows: 1,
                card_cols: CardCols::Chip {
                    first: USER_INDENT,
                    last: prefix_cols.saturating_add(label_cols),
                },
                clickable: true,
            });
        }
        if let Some(btn_text) = self.button_text() {
            let btn_col = prefix_cols.saturating_add(label_cols).saturating_add(2);
            let btn_cols =
                u16::try_from(unicode_width::UnicodeWidthStr::width(btn_text)).unwrap_or(u16::MAX);
            let clickable = !matches!(
                self.download_state,
                Some(crate::app::state::OriginalImageDownloadState::Downloading)
            );
            cards.push(super::snapshot::CardRegion {
                kind: super::CardKind::DownloadOriginalImage {
                    attachment_id: self.attachment_id.clone(),
                },
                card_start: row,
                card_rows: 1,
                card_cols: CardCols::Chip {
                    first: btn_col,
                    last: btn_col.saturating_add(btn_cols),
                },
                clickable,
            });
        }
    }
}

/// 一个块的绘制数据。
#[derive(Debug)]
pub(crate) enum BlockPaint {
    /// 用户正文：软换行后的行（含折叠粘贴芯片样式与缩进）、芯片行区间、折叠提示与引用树。
    UserText {
        text_lines: Vec<UserTextLine>,
        paste_spans: Vec<UserPasteSpan>,
        fold_hint: Option<String>,
        ref_lines: Vec<UserRefLine>,
    },
    /// 助手正文：已固化的稳定前缀 + 尾部动态行（流式时只重排尾部）。
    Markdown {
        prefix: Arc<Vec<MdLine>>,
        prefix_bytes: usize,
        tail: Vec<MdLine>,
    },
    /// thinking：头部之外只有正文行（折叠时为空；头部文案在绘制时算）。
    Thinking { body: Vec<MdLine> },
    /// 完全静态的单色行块（统计行）。
    Static { lines: Vec<String>, style: Style },
    /// 系统事件卡片（compact / 模型切换 / rewind / recap）：标题与树体已排版，
    /// 绘制直接复用，不再每帧重跑 Markdown 与树体排版。
    SystemEvent {
        card: Arc<super::system_event::SystemCard>,
    },
    /// 助手消息末尾的生成图片附件引用树（`├─ Image (preview): ...  [Download original]`）。
    RefLines { ref_lines: Vec<AssistantRefLine> },
    /// 尚未接入内容缓存的块：缓存行数与点击面几何，绘制仍走原路径。
    Legacy { rows: usize, spans: Vec<CardSpan> },
}

impl BlockPaint {
    /// 该块占用的内容行数（与绘制推进行数严格相等）。
    pub(crate) fn rows(&self) -> usize {
        match self {
            Self::UserText {
                text_lines,
                fold_hint,
                ref_lines,
                ..
            } => text_lines.len() + (fold_hint.is_some() as usize) + ref_lines.len(),
            Self::Markdown { prefix, tail, .. } => prefix.len() + tail.len(),
            Self::Thinking { body } => 1 + body.len(),
            Self::Static { lines, .. } => lines.len(),
            Self::SystemEvent { card } => card.rows(),
            Self::RefLines { ref_lines } => ref_lines.len(),
            Self::Legacy { rows, .. } => *rows,
        }
    }

    /// 块内的可点击区域（非工具块为空）。几何与行数同源、同一次构建。
    pub(crate) fn card_spans(&self) -> &[CardSpan] {
        match self {
            Self::Legacy { spans, .. } => spans,
            Self::UserText { .. }
            | Self::Markdown { .. }
            | Self::Thinking { .. }
            | Self::Static { .. }
            | Self::SystemEvent { .. }
            | Self::RefLines { .. } => &[],
        }
    }

    /// 近似字节数（只用于 LRU 记账，不追求精确）。
    pub(crate) fn bytes(&self) -> usize {
        match self {
            Self::UserText {
                text_lines,
                fold_hint,
                ref_lines,
                ..
            } => {
                text_lines.iter().map(UserTextLine::byte_len).sum::<usize>()
                    + fold_hint.as_ref().map_or(0, |h| h.len() + 24)
                    + ref_lines.iter().map(|r| r.label.len() + 24).sum::<usize>()
            }
            Self::Static { lines, .. } => lines.iter().map(|line| line.len() + 24).sum::<usize>(),
            Self::Markdown {
                prefix_bytes, tail, ..
            } => prefix_bytes.saturating_add(md_bytes(tail)),
            Self::Thinking { body } => md_bytes(body),
            // 系统事件卡片的正文已排版进 `SystemCard`，按行数记近似值。
            Self::SystemEvent { card } => card.rows() * 64,
            Self::RefLines { ref_lines } => ref_lines
                .iter()
                .map(|r| {
                    r.label.len()
                        + 24
                        + r.attachment_id.len()
                        + r.preview_path.as_ref().map_or(0, String::len)
                })
                .sum::<usize>(),
            // 旧路径没有缓存正文，不占二级缓存预算（行数已在一级快照里）；
            // 点击面几何是小结构，按条数记个近似值。
            Self::Legacy { spans, .. } => spans
                .iter()
                .map(|span| std::mem::size_of::<CardSpan>() + span.cols.heap_bytes())
                .sum(),
        }
    }

    /// 绘制该块；`start` 是块首行的绝对内容行号。
    ///
    /// 块内按视口裁剪：只遍历视口内的行，绝对行号照常推进。
    pub(crate) fn draw(
        &self,
        grid: &mut Grid,
        rect: Rect,
        scroll: usize,
        start: usize,
        history: HistorySource<'_>,
        source: BlockSource,
    ) {
        let viewport_end = scroll.saturating_add(usize::from(rect.height));
        match self {
            Self::UserText {
                text_lines,
                fold_hint,
                ref_lines,
                ..
            } => draw_user_text(
                grid,
                rect,
                history,
                source,
                scroll,
                start,
                viewport_end,
                text_lines,
                fold_hint.as_deref(),
                ref_lines,
            ),
            Self::Markdown { prefix, tail, .. } => draw_markdown(
                grid,
                rect,
                history,
                source,
                scroll,
                start,
                viewport_end,
                prefix,
                tail,
            ),
            Self::Thinking { body } => draw_thinking(
                grid,
                rect,
                history,
                source,
                scroll,
                start,
                viewport_end,
                body,
            ),
            Self::Static { lines, style } => {
                for (offset, line) in lines.iter().enumerate().skip(scroll.saturating_sub(start)) {
                    let row = start.saturating_add(offset);
                    if row >= viewport_end {
                        break;
                    }
                    if let Some(y) = visible_row(rect, scroll, row) {
                        grid.put_clipped(rect.x, y, line, *style, rect.width);
                    }
                }
            }
            Self::SystemEvent { card } => {
                let mut row = start;
                card.draw(grid, rect, scroll, &mut row);
                if system_event_selected(history, source) {
                    let bg = style::chat_user_target_bg();
                    for content_row in start..row {
                        if let Some(y) = visible_row(rect, scroll, content_row) {
                            grid.tint_row_bg(rect.x, y, rect.width, bg);
                        }
                    }
                }
            }
            Self::RefLines { ref_lines } => {
                draw_assistant_ref_lines(grid, rect, scroll, start, viewport_end, ref_lines);
            }
            Self::Legacy { .. } => legacy::draw_legacy(grid, rect, history, source, scroll, start),
        }
    }
}

/// 系统事件卡片（compact / 模型切换 / recap）是否被 `/rewind` 选中。
fn system_event_selected(history: HistorySource<'_>, source: BlockSource) -> bool {
    let Some(selected_id) = history.selected_user_message else {
        return false;
    };
    match source {
        BlockSource::Compact { index, .. } => history
            .checkpoints
            .get(index)
            .is_some_and(|item| item.id == selected_id),
        BlockSource::ModelSwitch { last, .. } => history
            .switches
            .get(last)
            .is_some_and(|item| item.id == selected_id),
        BlockSource::Recap { message_index } => history
            .messages
            .get(message_index)
            .is_some_and(|item| item.id == selected_id),
        _ => false,
    }
}

/// 思考头部文字列区间：标记在 0 列，标签从 [`STEP_GUTTER`] 列开始。
pub(crate) fn thinking_header_cols(
    part: &crate::app::state::ChatPartView,
    streaming: bool,
    width: u16,
) -> Option<(u16, u16)> {
    if width == 0 {
        return None;
    }
    let label = thinking_header_label(part, streaming);
    let end = STEP_GUTTER
        .saturating_add(
            u16::try_from(unicode_width::UnicodeWidthStr::width(label.as_str()))
                .unwrap_or(u16::MAX),
        )
        .min(width);
    (end > 0).then_some((0, end))
}

/// 展开的思考正文逐行文字列区间（相对内容区左缘）。
pub(crate) fn thinking_body_cols(body: &[MdLine], width: u16) -> Vec<Option<(u16, u16)>> {
    let base = super::thinking_body_indent(width);
    let tree = super::thinking_tree(width);
    body.iter()
        .enumerate()
        .map(|(index, line)| {
            let (start, end) = super::text_cols::md_line_cols(line)?;
            // 首个正文行在树模式下从 0 列画 `  └─`，因此区间起点覆盖连接符。
            let start = if tree && index == 0 {
                0
            } else {
                base.saturating_add(start)
            };
            let end = base.saturating_add(end).min(width);
            (end > start).then_some((start, end))
        })
        .collect()
}

/// 消息与步骤 part（正文 / thinking）。
fn step_of(
    history: HistorySource<'_>,
    source: BlockSource,
) -> Option<(&ChatMessageView, &crate::app::state::ChatPartView)> {
    let BlockSource::Step {
        message_index,
        step_index,
    } = source
    else {
        return None;
    };
    let message = history.messages.get(message_index)?;
    match message.steps.get(step_index)? {
        AssistantStepView::Part { part, .. } => Some((message, part)),
        AssistantStepView::Tool { .. } => None,
    }
}

/// 单个工具块对应的调用（非工具块返回空）。
fn calls_of(history: HistorySource<'_>, source: BlockSource) -> Vec<&ToolCallView> {
    let BlockSource::Step {
        message_index,
        step_index,
    } = source
    else {
        return Vec::new();
    };
    let Some(message) = history.messages.get(message_index) else {
        return Vec::new();
    };
    match message.steps.get(step_index) {
        Some(AssistantStepView::Tool { call, .. }) => vec![call],
        _ => Vec::new(),
    }
}

/// 工具的两块折叠状态（绘制时按当前状态重算，不进缓存键）。
fn folds_at(history: HistorySource<'_>, message_index: usize, call: &ToolCallView) -> Folds {
    let Some(message) = history.messages.get(message_index) else {
        return Folds::default();
    };
    super::folds_of(history, &message.id, call)
}

#[allow(clippy::too_many_arguments)]
fn draw_user_text(
    grid: &mut Grid,
    rect: Rect,
    history: HistorySource<'_>,
    source: BlockSource,
    scroll: usize,
    start: usize,
    viewport_end: usize,
    text_lines: &[UserTextLine],
    fold_hint: Option<&str>,
    ref_lines: &[UserRefLine],
) {
    let BlockSource::Step { message_index, .. } = source else {
        return;
    };
    let Some(message) = history.messages.get(message_index) else {
        return;
    };
    let selected = history.selected_user_message == Some(message.id.as_str());
    let steered = history.chat.is_steered_user_message(&message.id);
    let bg = if selected {
        style::chat_user_target_bg()
    } else if steered {
        style::chat_user_steer_bg()
    } else {
        style::chat_user_bg()
    };
    let mut current_row = start;
    let avail_w = rect.width.saturating_sub(USER_INDENT);
    let right_limit = rect.x.saturating_add(USER_INDENT).saturating_add(avail_w);

    for (offset, line) in text_lines.iter().enumerate() {
        let row = current_row;
        current_row += 1;
        if row < scroll || row >= viewport_end {
            continue;
        }
        let Some(y) = visible_row(rect, scroll, row) else {
            continue;
        };
        grid.background_row(rect.x, y, rect.width, bg);
        if offset == 0 {
            grid.put(rect.x, y, USER_PREFIX, style::accent().patch(bg));
        }
        let mut cx = rect.x.saturating_add(USER_INDENT);
        for span in &line.spans {
            if cx >= right_limit {
                break;
            }
            let span_style = match span.kind {
                UserSpanKind::Plain => bg,
                UserSpanKind::CollapsedChip(kind) => style::pasted_chip(kind),
                UserSpanKind::ExpandedChip(kind) => style::pasted_bg(kind),
            };
            grid.put_clipped(
                cx,
                y,
                &span.text,
                span_style,
                right_limit.saturating_sub(cx),
            );
            let w: usize = span.text.chars().map(|ch| ch.width().unwrap_or(0)).sum();
            cx = cx
                .saturating_add(u16::try_from(w).unwrap_or(u16::MAX))
                .min(right_limit);
        }
    }

    if let Some(hint) = fold_hint {
        let row = current_row;
        current_row += 1;
        if row >= scroll
            && row < viewport_end
            && let Some(y) = visible_row(rect, scroll, row)
        {
            grid.background_row(rect.x, y, rect.width, bg);
            grid.put_clipped(
                rect.x.saturating_add(USER_INDENT),
                y,
                hint,
                style::fold_hint().patch(bg),
                rect.width.saturating_sub(USER_INDENT),
            );
        }
    }

    draw_ref_lines(grid, rect, scroll, current_row, viewport_end, ref_lines);
}

/// 绘制 `├─ Image: ...` / `└─ File: ...` 树形附件行（用户消息与助手消息共用）。
fn draw_ref_lines(
    grid: &mut Grid,
    rect: Rect,
    scroll: usize,
    mut current_row: usize,
    viewport_end: usize,
    ref_lines: &[UserRefLine],
) {
    for ref_line in ref_lines {
        let row = current_row;
        current_row += 1;
        if row < scroll || row >= viewport_end {
            continue;
        }
        let Some(y) = visible_row(rect, scroll, row) else {
            continue;
        };
        let connector = if ref_line.is_last {
            crate::ui::text::agent::TREE_LEAF
        } else {
            crate::ui::text::agent::TREE_BRANCH
        };
        let (kind_prefix, label_style) = match ref_line.kind {
            crate::db::chat::MessageAttachmentKind::File => (
                crate::ui::text::agent::USER_REF_FILE_PREFIX,
                style::user_ref_file(),
            ),
            crate::db::chat::MessageAttachmentKind::Folder => (
                crate::ui::text::agent::USER_REF_FOLDER_PREFIX,
                style::user_ref_folder(),
            ),
            crate::db::chat::MessageAttachmentKind::Image => (
                crate::ui::text::agent::USER_REF_IMAGE_PREFIX,
                style::user_ref_image(),
            ),
            crate::db::chat::MessageAttachmentKind::Skill => (
                crate::ui::text::agent::USER_REF_SKILL_PREFIX,
                style::user_ref_file(),
            ),
        };
        let conn_cols = USER_INDENT + 3;
        let prefix_cols = conn_cols + kind_prefix.len() as u16;
        grid.put_clipped(rect.x, y, connector, Style::default(), rect.width);
        grid.put_clipped(
            rect.x.saturating_add(conn_cols),
            y,
            kind_prefix,
            Style::default(),
            rect.width.saturating_sub(conn_cols),
        );
        grid.put_clipped(
            rect.x.saturating_add(prefix_cols),
            y,
            &ref_line.label,
            label_style,
            rect.width.saturating_sub(prefix_cols),
        );
    }
}

/// 绘制助手消息末尾的生成图片附件行（带 `(preview)`、文件名下划线与原图下载按钮）。
fn draw_assistant_ref_lines(
    grid: &mut Grid,
    rect: Rect,
    scroll: usize,
    mut current_row: usize,
    viewport_end: usize,
    ref_lines: &[AssistantRefLine],
) {
    for ref_line in ref_lines {
        let row = current_row;
        current_row += 1;
        if row < scroll || row >= viewport_end {
            continue;
        }
        let Some(y) = visible_row(rect, scroll, row) else {
            continue;
        };
        let connector = if ref_line.is_last {
            crate::ui::text::agent::TREE_LEAF
        } else {
            crate::ui::text::agent::TREE_BRANCH
        };
        let kind_prefix = crate::ui::text::agent::ASSISTANT_REF_IMAGE_PREVIEW_PREFIX;
        let mut label_style = style::user_ref_image();
        if ref_line.preview_path.is_some() {
            label_style = label_style.add_modifier(ratatui::style::Modifier::UNDERLINED);
        }
        let conn_cols = USER_INDENT + 3;
        let prefix_cols = conn_cols + kind_prefix.len() as u16;
        let label_cols = u16::try_from(unicode_width::UnicodeWidthStr::width(
            ref_line.label.as_str(),
        ))
        .unwrap_or(u16::MAX);
        grid.put_clipped(rect.x, y, connector, Style::default(), rect.width);
        grid.put_clipped(
            rect.x.saturating_add(conn_cols),
            y,
            kind_prefix,
            Style::default(),
            rect.width.saturating_sub(conn_cols),
        );
        grid.put_clipped(
            rect.x.saturating_add(prefix_cols),
            y,
            &ref_line.label,
            label_style,
            rect.width.saturating_sub(prefix_cols),
        );
        if let Some(btn_text) = ref_line.button_text() {
            let btn_col = prefix_cols.saturating_add(label_cols).saturating_add(2);
            if btn_col < rect.width {
                let btn_style = match &ref_line.download_state {
                    Some(crate::app::state::OriginalImageDownloadState::Downloading) => {
                        style::assistant_image_downloading()
                    }
                    Some(crate::app::state::OriginalImageDownloadState::Downloaded { .. }) => {
                        style::user_ref_file()
                    }
                    _ => style::muted(),
                };
                grid.put_clipped(
                    rect.x.saturating_add(btn_col),
                    y,
                    btn_text,
                    btn_style,
                    rect.width.saturating_sub(btn_col),
                );
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_markdown(
    grid: &mut Grid,
    rect: Rect,
    history: HistorySource<'_>,
    source: BlockSource,
    scroll: usize,
    start: usize,
    viewport_end: usize,
    prefix: &[MdLine],
    tail: &[MdLine],
) {
    let BlockSource::Step {
        message_index,
        step_index,
    } = source
    else {
        return;
    };
    let Some(message) = history.messages.get(message_index) else {
        return;
    };
    // 正文块也是步骤：消息仍在流式且它是最后一步 → 标记呼吸，否则静态。
    let mark = if message.status == "streaming" && step_index + 1 == message.steps.len() {
        StepMark::Running
    } else {
        StepMark::Done
    };
    let total = prefix.len().saturating_add(tail.len());
    for offset in scroll.saturating_sub(start)..total {
        let row = start.saturating_add(offset);
        if row >= viewport_end {
            break;
        }
        let Some(y) = visible_row(rect, scroll, row) else {
            continue;
        };
        let line = if offset < prefix.len() {
            &prefix[offset]
        } else {
            &tail[offset.saturating_sub(prefix.len())]
        };
        if offset == 0 {
            grid.put_char(
                rect.x,
                y,
                mark.glyph(
                    StepKind::Body,
                    history.chat.loading_frame,
                    history.animating,
                ),
                mark.style(StepKind::Body, false),
            );
        }
        let base_x = rect
            .x
            .saturating_add(STEP_GUTTER)
            .saturating_add(line.indent);
        let mut cx = base_x;
        if let Some(ref line_ref) = line.table {
            grid.set_table_row(cx, y, line_ref.clone());
        }
        for link in &line.links {
            let start_x = base_x.saturating_add(link.start_col);
            let end_x = base_x.saturating_add(link.end_col).min(rect.right());
            if start_x < end_x {
                grid.set_link_span(start_x, y, end_x - start_x, &link.url);
            }
        }
        if let Some(code_kind) = line.code {
            let code_row = match code_kind {
                crate::ui::markdown_render::CodeLineKind::TopBorder
                | crate::ui::markdown_render::CodeLineKind::Header { .. } => {
                    super::super::grid::GridCodeRow::Chrome
                }
                crate::ui::markdown_render::CodeLineKind::Body => {
                    super::super::grid::GridCodeRow::Body {
                        bar_col: cx.saturating_sub(grid.area().x),
                    }
                }
            };
            grid.set_code_row(y, code_row);
        }
        for (value, span_style) in &line.spans {
            for ch in value.chars() {
                let cell = ch.width().unwrap_or(1).max(1) as u16;
                if cx.saturating_add(cell) > rect.right() {
                    break;
                }
                grid.put_char(cx, y, ch, *span_style);
                cx = cx.saturating_add(cell);
            }
        }
    }
}
