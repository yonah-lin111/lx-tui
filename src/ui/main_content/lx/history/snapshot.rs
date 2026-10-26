//! 历史几何快照：只依赖「内容宽度 + 列表状态」，产出内容行空间
//! （0 = 最早，不含屏幕坐标与滚动）的行区间与每块绘制数据。
//!
//! `layout` 把内容行映射成屏幕矩形；`render` 用 `entries` 跳过视口外的条目、
//! 用 `blocks` 复用每块已排好的行；命中入口用 `cards` 换算内容行。三处读同一份快照。
//!
//! 行数与绘制数据**同源**：每个 `SnapBlock` 的 `rows` 就是 `paint.rows()`。
//!
//! 快照**不含**高度/`y`/滚动偏移：这些随视口每帧变化，进了键就会每帧重建。

use std::sync::Arc;

use crate::app::state::{AssistantStepView, CardSection, UserShellView};
use crate::ui::markdown_render;

use super::cache::{
    BlockKey, PaneCache, SHELL_SLOT_BLOCK, compact_block_signature, group_block_signature,
    notice_block_signature, part_block_signature, report_block_signature, rewind_block_signature,
    shell_block_signature, stats_block_signature,
};
use super::paint::{BlockPaint, BlockSource, CardSpan};
use super::source::HistorySource;
use super::timeline::{TimelineEntry, timeline};
use super::{
    CardKind, Folds, MessageRows, compact, folds_of, notice, report_block, rewind, shell_folds,
    should_register_card, stats_line, tool, tool_rows, user_shell, wrap,
};

/// 时间线条目类别（与 [`TimelineEntry`] 一一对应，但可跨帧独立持有）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SnapEntryKind {
    Message(usize),
    Notices { last: usize, count: usize },
    Compact(usize),
}

/// 快照里的一个块：起止行 + 绘制数据。
#[derive(Clone)]
pub(crate) struct SnapBlock {
    /// 块首行的绝对内容行号。
    pub(crate) start: usize,
    /// 块占用的行数（= `paint.rows()`）。
    pub(crate) rows: usize,
    /// 绘制数据（二级缓存命中/构建的结果）。
    pub(crate) paint: Arc<BlockPaint>,
    /// 缓存键（绘制时刷新 LRU）。
    pub(crate) key: BlockKey,
    /// 块在 `Lx` 里的定位。
    pub(crate) source: BlockSource,
}

impl std::fmt::Debug for SnapBlock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SnapBlock")
            .field("start", &self.start)
            .field("rows", &self.rows)
            .field("source", &self.source)
            .finish()
    }
}

impl PartialEq for SnapBlock {
    fn eq(&self, other: &Self) -> bool {
        self.start == other.start && self.rows == other.rows && self.source == other.source
    }
}

impl Eq for SnapBlock {}

/// 一条时间线条目的内容行区间（`end` 不含）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SnapEntry {
    pub(crate) start: usize,
    pub(crate) end: usize,
    pub(crate) kind: SnapEntryKind,
    /// 条目内的块（顺序即绘制顺序）。
    pub(crate) blocks: Vec<SnapBlock>,
}

/// 一个可折叠块的内容行区间（点击面）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CardRegion {
    pub(crate) kind: CardKind,
    pub(crate) card_start: usize,
    pub(crate) card_rows: usize,
    pub(crate) clickable: bool,
}

/// 历史几何快照（内容行空间）。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct HistorySnapshot {
    /// 每条时间线条目的起止行。
    pub(crate) entries: Vec<SnapEntry>,
    /// 每条消息的行区间（置顶行归属判定用；下标即消息下标）。
    pub(crate) messages: Vec<MessageRows>,
    /// （消息索引, 步骤下标, (起始行, 行数)）：thinking 头部。
    pub(crate) thinking: Vec<(usize, usize, (usize, usize))>,
    /// 可折叠块的内容行区间。
    pub(crate) cards: Vec<CardRegion>,
    /// 内容总行数（滚动上限用）。
    pub(crate) total_rows: usize,
}

/// 构建快照。`width` 是历史内容列宽（已扣除滚动条预留列）。
pub(crate) fn build(
    width: u16,
    source: HistorySource<'_>,
    cache: &mut PaneCache,
) -> HistorySnapshot {
    if width == 0 {
        return HistorySnapshot::default();
    }
    let timeline = timeline(source);
    let mut builder = Builder {
        cache,
        width,
        snapshot: HistorySnapshot::default(),
        rows: 0,
    };
    let mut entry_has_previous = false;
    for entry in &timeline {
        if entry_has_previous {
            builder.rows += 1;
        }
        entry_has_previous = true;
        match entry {
            TimelineEntry::Notices { last, count } => builder.notices(source, *last, *count),
            TimelineEntry::Compact(index) => builder.compact(source, *index),
            TimelineEntry::Message(message_index) => builder.message(source, *message_index),
        }
    }
    // 末尾留一行呼吸位（滚动到底部时不贴死）；归最后一条消息。
    if !timeline.is_empty() {
        builder.rows += 1;
        if matches!(timeline.last(), Some(TimelineEntry::Message(_)))
            && let Some(last) = builder.snapshot.messages.last_mut()
        {
            last.end = builder.rows;
        }
    }
    builder.snapshot.total_rows = builder.rows;
    builder.snapshot
}

/// 工具卡片的绘制数据：行数与点击面几何**在同一次构建里**算好，随块键缓存。
fn legacy(
    rows: usize,
    regions: impl IntoIterator<Item = tool::CardRegion>,
    folds: Folds,
) -> BlockPaint {
    BlockPaint::Legacy {
        rows,
        spans: regions
            .into_iter()
            .map(|region| CardSpan {
                call: 0,
                section: region.section,
                expanded: folds.of(region.section),
                offset: region.card.offset,
                rows: region.card.rows,
                clickable: region.clickable,
            })
            .collect(),
    }
}

/// 合并文件工具组的点击面几何（组内每个可展开的调用一个整块区域）。
fn group_spans(
    group: &[&crate::app::state::ToolCallView],
    width: u16,
    expanded: &[bool],
) -> Vec<CardSpan> {
    let mut out = Vec::with_capacity(group.len());
    for (index, block) in tool::group_blocks(group, width, expanded)
        .iter()
        .enumerate()
    {
        let call = group[index];
        if !should_register_card(call, width) {
            continue;
        }
        out.push(CardSpan {
            call: index,
            section: CardSection::Body,
            expanded: expanded.get(index).copied().unwrap_or(false),
            offset: block.start,
            rows: block.len,
            clickable: true,
        });
    }
    out
}

/// 快照构建中的可变累加器。
struct Builder<'a> {
    cache: &'a mut PaneCache,
    width: u16,
    snapshot: HistorySnapshot,
    rows: usize,
}

impl Builder<'_> {
    /// 块间空行：**一律留**（含连续工具步骤之间、相邻正文块之间）。
    ///
    /// 每个工具调用是独立的动作记录，`■ 动词` / `  └─ 详情` 与下一个调用之间空一行
    /// 才分得清边界——不留会被读成一棵树，误以为 `└─` 属于上一个调用。规则不区分
    /// 块类别，因此没有谓词：`if has_previous { self.spacer() }` 就是全部。
    /// 空行只在这里累加（AGENTS：绘制路径不得另算）。
    fn spacer(&mut self) {
        self.rows += 1;
    }

    /// 按块键取/建绘制数据，并把行数作为唯一行数来源写进块。
    fn block(
        &mut self,
        key: BlockKey,
        source: BlockSource,
        start: usize,
        build: impl FnOnce() -> BlockPaint,
    ) -> SnapBlock {
        let paint = self.cache.block(key, || Arc::new(build()));
        let rows = paint.rows();
        SnapBlock {
            start,
            rows,
            paint,
            key,
            source,
        }
    }

    /// 撤销提示条目（连续提示合并为一条）。
    fn notices(&mut self, source: HistorySource<'_>, last: usize, count: usize) {
        let start = self.rows;
        let key = BlockKey {
            message_index: last,
            step_index: super::cache::REWIND_SLOT_BLOCK,
            width: self.width,
            sig: rewind_block_signature(&source.notices[last], count),
        };
        let rows = rewind::rows(&source.notices[last], count, self.width);
        let block = self.block(key, BlockSource::Rewind { last, count }, start, || {
            BlockPaint::Legacy {
                rows,
                spans: Vec::new(),
            }
        });
        self.rows += block.rows;
        self.snapshot.entries.push(SnapEntry {
            start,
            end: self.rows,
            kind: SnapEntryKind::Notices { last, count },
            blocks: vec![block],
        });
    }

    /// compact 检查点条目。
    fn compact(&mut self, source: HistorySource<'_>, index: usize) {
        let start = self.rows;
        let notice = &source.checkpoints[index];
        let expanded = source.compact.is_expanded(&notice.id);
        let key = BlockKey {
            message_index: index,
            step_index: super::cache::COMPACT_SLOT_BLOCK,
            width: self.width,
            sig: compact_block_signature(notice, expanded),
        };
        let rows = compact::rows(notice, 1, self.width, expanded);
        let block = self.block(key, BlockSource::Compact { index, expanded }, start, || {
            BlockPaint::Legacy {
                rows,
                spans: Vec::new(),
            }
        });
        self.rows += block.rows;
        // 整块都是点击面：折叠态只有头部两行，点哪里都应能展开。
        self.snapshot.cards.push(CardRegion {
            kind: CardKind::Compact {
                checkpoint_id: notice.id.clone(),
                expanded,
            },
            card_start: start,
            card_rows: block.rows,
            clickable: true,
        });
        self.snapshot.entries.push(SnapEntry {
            start,
            end: self.rows,
            kind: SnapEntryKind::Compact(index),
            blocks: vec![block],
        });
    }

    /// 一条消息（含本地命令卡片分支）。
    fn message(&mut self, source: HistorySource<'_>, message_index: usize) {
        let message = &source.messages[message_index];
        let start = self.rows;
        let mut blocks = Vec::new();
        if !message.reports.is_empty() {
            // 汇报行：不画用户气泡，逐条画汇报块（块间空行只在这里累加）。
            self.reports(source, message_index, message, &mut blocks);
        } else if let Some(shell) = &message.shell {
            self.shell(source, message_index, message, shell, start, &mut blocks);
        } else {
            self.steps(source, message_index, message, &mut blocks);
            self.tail(message_index, message, &mut blocks);
        }
        // 汇报行不是用户气泡：置顶行与用户底色据此跳过它。
        let is_user =
            message.role == "user" && message.shell.is_none() && message.reports.is_empty();
        self.snapshot.messages.push(MessageRows {
            start,
            end: self.rows,
            is_user,
        });
        self.snapshot.entries.push(SnapEntry {
            start,
            end: self.rows,
            kind: SnapEntryKind::Message(message_index),
            blocks,
        });
    }

    /// 汇报块：一条消息可有多份，逐条一个块。
    fn reports(
        &mut self,
        source: HistorySource<'_>,
        message_index: usize,
        message: &crate::app::state::ChatMessageView,
        blocks: &mut Vec<SnapBlock>,
    ) {
        let width = self.width;
        for (index, report) in message.reports.iter().enumerate() {
            if index > 0 {
                self.spacer();
            }
            let slot = format!("report:{}", report.agent_id);
            let expanded = source.chat.detail_is_expanded(&message.id, &slot);
            let rows = report_block::rows(report, width, expanded);
            let key = BlockKey {
                message_index,
                step_index: super::cache::REPORT_SLOT_BLOCK.saturating_sub(index),
                width,
                sig: report_block_signature(report, expanded),
            };
            let start = self.rows;
            let block = self.block(
                key,
                BlockSource::Report {
                    message_index,
                    index,
                },
                start,
                || BlockPaint::Legacy {
                    rows,
                    spans: Vec::new(),
                },
            );
            let block_rows = block.rows;
            if super::report_block::has_more(report, width) {
                self.snapshot.cards.push(CardRegion {
                    kind: CardKind::Report {
                        message_id: message.id.clone(),
                        agent_id: report.agent_id.clone(),
                        expanded,
                    },
                    card_start: start,
                    card_rows: block_rows,
                    clickable: true,
                });
            }
            blocks.push(block);
            self.rows += block_rows;
        }
    }

    /// 本地命令卡片（独立条目，不进入步骤时间线）。
    fn shell(
        &mut self,
        source: HistorySource<'_>,
        message_index: usize,
        message: &crate::app::state::ChatMessageView,
        shell: &UserShellView,
        entry_start: usize,
        blocks: &mut Vec<SnapBlock>,
    ) {
        let folds = shell_folds(source, &message.id);
        let key = BlockKey {
            message_index,
            step_index: SHELL_SLOT_BLOCK,
            width: self.width,
            sig: shell_block_signature(source, message, shell),
        };
        let start = self.rows;
        let width = self.width;
        let rows = user_shell::rows(
            shell,
            width,
            source.chat.loading_frame,
            source.chat.status.streaming || shell.is_running(),
            folds,
        );
        let block = self.block(key, BlockSource::Shell { message_index }, start, || {
            BlockPaint::Legacy {
                rows,
                spans: user_shell::regions(shell, width, folds)
                    .into_iter()
                    .map(|region| CardSpan {
                        call: 0,
                        section: region.section,
                        expanded: folds.of(region.section),
                        offset: region.offset,
                        rows: region.rows,
                        clickable: region.clickable,
                    })
                    .collect(),
            }
        });
        self.rows += block.rows;
        for span in block.paint.card_spans() {
            self.snapshot.cards.push(CardRegion {
                kind: CardKind::Shell {
                    message_id: message.id.clone(),
                    section: span.section,
                    expanded: span.expanded,
                },
                card_start: entry_start + span.offset,
                card_rows: span.rows,
                clickable: span.clickable,
            });
        }
        blocks.push(block);
    }

    /// assistant/用户消息的步骤时间线（含合并组、工具卡片与 thinking 头部）。
    fn steps(
        &mut self,
        source: HistorySource<'_>,
        message_index: usize,
        message: &crate::app::state::ChatMessageView,
        blocks: &mut Vec<SnapBlock>,
    ) {
        let mut has_previous = false;
        let mut step_index = 0usize;
        while let Some(step) = message.steps.get(step_index) {
            // 同种文件工具的连续调用共用一行合并标题，但每个调用各自独立成块。
            if let AssistantStepView::Tool { .. } = step {
                let group = tool::merge_group(&message.steps, step_index);
                if group.len() > 1 {
                    if self.merge_group(
                        source,
                        message_index,
                        message,
                        &group,
                        step_index,
                        has_previous,
                        blocks,
                    ) {
                        has_previous = true;
                    }
                    step_index += group.len();
                    continue;
                }
            }
            // 先取块（命中二级缓存时不再重排正文），行数**只**从 paint 取。
            let mut block = self.build_step(source, message_index, step_index, message, step);
            if block.rows == 0 {
                step_index += 1;
                continue; // 空正文块不占行，也不引入块间空行。
            }
            if has_previous {
                self.spacer();
            }
            has_previous = true;
            block.start = self.rows;
            let block_start = self.rows;
            if let AssistantStepView::Tool { call, .. } = step {
                self.tool_cards(
                    message,
                    call,
                    block.paint.card_spans(),
                    block.rows,
                    block_start,
                );
            }
            if matches!(
                step,
                AssistantStepView::Part { part, .. }
                    if part.kind == crate::chat::events::PartKind::Reasoning
            ) {
                let expanded = source.chat.reasoning_expanded(message_index, step_index);
                self.snapshot
                    .thinking
                    .push((message_index, step_index, (block_start, 1)));
                self.snapshot.cards.push(CardRegion {
                    kind: CardKind::Thinking {
                        message_index,
                        step_index,
                        expanded,
                    },
                    card_start: block_start,
                    card_rows: block.rows,
                    clickable: true,
                });
            }
            self.rows = block_start.saturating_add(block.rows);
            blocks.push(block);
            step_index += 1;
        }
    }

    /// 单个步骤块：用户正文 / markdown 正文 / thinking / 单个工具。
    ///
    /// `start` 先给 0；调用方决定空行后再回填真实行号。
    fn build_step(
        &mut self,
        history: HistorySource<'_>,
        message_index: usize,
        step_index: usize,
        message: &crate::app::state::ChatMessageView,
        step: &AssistantStepView,
    ) -> SnapBlock {
        let key = BlockKey {
            message_index,
            step_index,
            width: self.width,
            sig: part_block_signature(history, message, message_index, step_index, step),
        };
        let block_source = BlockSource::Step {
            message_index,
            step_index,
        };
        let width = self.width;
        let is_user = message.role == "user";
        let expanded = history.chat.reasoning_expanded(message_index, step_index);
        self.block(key, block_source, 0, || match step {
            AssistantStepView::Part { part, .. } => match part.kind {
                crate::chat::events::PartKind::Text if is_user => BlockPaint::UserText {
                    lines: wrap(&part.content, width.saturating_sub(super::USER_INDENT)),
                },
                crate::chat::events::PartKind::Text => BlockPaint::Markdown {
                    lines: markdown_render::render_wrapped(
                        &part.content,
                        width.saturating_sub(super::STEP_GUTTER),
                    ),
                },
                crate::chat::events::PartKind::Reasoning => BlockPaint::Thinking {
                    body: if expanded {
                        markdown_render::render_wrapped(
                            &part.content,
                            width.saturating_sub(super::STEP_GUTTER),
                        )
                    } else {
                        Vec::new()
                    },
                },
            },
            AssistantStepView::Tool { call, .. } => {
                let folds = folds_of(history, &message.id, call);
                let rows = tool_rows(call, width, folds);
                legacy(rows, tool::card_regions(call, width, folds, rows), folds)
            }
        })
    }

    /// 命令卡片的点击面（参数区 / 输出区各自独立）。
    ///
    /// 几何来自**块的绘制数据**（`card_spans`），不在这里重算：`card_regions` 是
    /// `(调用, 宽度, 折叠态)` 的纯函数，与块同键缓存；每帧重算在 1400 张工具卡片的
    /// 会话里要 4.7ms（实测）。
    fn tool_cards(
        &mut self,
        message: &crate::app::state::ChatMessageView,
        call: &crate::app::state::ToolCallView,
        spans: &[CardSpan],
        step_rows: usize,
        block_start: usize,
    ) {
        for span in spans {
            // 区域不能越出块本身（几何与行数同源，这里只是防御性收敛）。
            let card_rows = span.rows.min(step_rows.saturating_sub(span.offset));
            if card_rows == 0 {
                continue;
            }
            self.snapshot.cards.push(CardRegion {
                kind: CardKind::Tool {
                    message_id: message.id.clone(),
                    call_id: call.call_id.clone(),
                    section: span.section,
                    expanded: span.expanded,
                },
                card_start: block_start + span.offset,
                card_rows,
                clickable: span.clickable,
            });
        }
    }

    /// 合并的文件工具组：一个块（标题行 + 各调用文件块）。
    fn merge_group(
        &mut self,
        source: HistorySource<'_>,
        message_index: usize,
        message: &crate::app::state::ChatMessageView,
        group: &[&crate::app::state::ToolCallView],
        first_step: usize,
        has_previous: bool,
        blocks: &mut Vec<SnapBlock>,
    ) -> bool {
        let expanded: Vec<bool> = group
            .iter()
            .map(|call| source.chat.detail_is_expanded(&message.id, &call.call_id))
            .collect();
        let group_rows = tool::group_rows(group, self.width, &expanded);
        if group_rows == 0 {
            return false;
        }
        // 与单块同口径：合并组也是消息内的一个块，前面有块就空一行
        // （否则正文下面直接顶着 `▣ Write 2 files`，两段粘成一片）。
        if has_previous {
            self.spacer();
        }
        let block_start = self.rows;
        let key = BlockKey {
            message_index,
            step_index: first_step,
            width: self.width,
            sig: group_block_signature(source, message, group, &expanded),
        };
        let width = self.width;
        let block = self.block(
            key,
            BlockSource::Group {
                message_index,
                first_step,
                count: group.len(),
            },
            block_start,
            || BlockPaint::Legacy {
                rows: group_rows,
                spans: group_spans(group, width, &expanded),
            },
        );
        for span in block.paint.card_spans() {
            let Some(call) = group.get(span.call) else {
                continue;
            };
            self.snapshot.cards.push(CardRegion {
                kind: CardKind::Tool {
                    message_id: message.id.clone(),
                    call_id: call.call_id.clone(),
                    section: span.section,
                    expanded: span.expanded,
                },
                card_start: block_start + span.offset,
                card_rows: span.rows,
                clickable: span.clickable,
            });
        }
        blocks.push(block);
        self.rows += group_rows;
        true
    }

    /// 消息底部通知与统计行。
    fn tail(
        &mut self,
        message_index: usize,
        message: &crate::app::state::ChatMessageView,
        blocks: &mut Vec<SnapBlock>,
    ) {
        let mut has_previous = !blocks.is_empty();
        if let Some(notice) = notice(message) {
            if has_previous {
                self.spacer();
            }
            has_previous = true;
            let start = self.rows;
            let key = BlockKey {
                message_index,
                step_index: super::cache::NOTICE_SLOT_BLOCK,
                width: self.width,
                sig: notice_block_signature(&notice),
            };
            let rows = notice::rows(&notice, self.width);
            let block = self.block(key, BlockSource::Notice { message_index }, start, || {
                BlockPaint::Legacy {
                    rows,
                    spans: Vec::new(),
                }
            });
            self.rows += block.rows;
            blocks.push(block);
        }
        if let Some(line) = stats_line(message) {
            if has_previous {
                self.spacer();
            }
            let start = self.rows;
            let key = BlockKey {
                message_index,
                step_index: super::cache::STATS_SLOT_BLOCK,
                width: self.width,
                sig: stats_block_signature(&line),
            };
            let lines = wrap(&line, self.width);
            let block = self.block(key, BlockSource::Plain, start, || BlockPaint::Static {
                lines,
                style: crate::ui::style::muted(),
            });
            self.rows += block.rows;
            blocks.push(block);
        }
    }
}
