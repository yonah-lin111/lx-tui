//! 阅读锚点：内容变化前后把同一块钉回原来的屏幕位置。
//!
//! 每帧在重建快照**之前**从上一份快照捕获锚点，重建后再按新快照求同一块的
//! 新行号；两者的差值就是视口需要补偿的位移。锚点不落状态：内容没变时快照
//! 不重建，锚点行也不会变，无需跨帧保存。

use super::BlockSource;
use super::snapshot::SnapEntry;
use super::{HistorySnapshot, HistorySource, SnapEntryKind};

/// 内容变化前的阅读锚点（块级）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HistoryAnchor {
    /// 锚点条目身份。
    pub(crate) entry: AnchorEntry,
    /// 条目内块序号（`SnapEntry::blocks` 下标）；块身份失配时的回退。
    pub(crate) block: usize,
    /// 块的身份（步骤/汇报/工具组等）；优先按它找块，避免块前插导致序号漂移。
    pub(crate) source: BlockSource,
    /// 块内行偏移。
    pub(crate) offset: usize,
    /// 捕获时的绝对内容行（补偿基准）。
    pub(crate) row: usize,
    /// 捕获时的内容宽度；宽度变化直接放弃补偿。
    pub(crate) width: u16,
}

/// 锚点条目身份：优先用稳定 ID，避免内容前插导致下标漂移。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AnchorEntry {
    Message {
        index: usize,
        id: String,
    },
    Notices {
        last: usize,
        id: String,
    },
    Compact {
        index: usize,
        id: String,
    },
    ModelSwitch {
        last: usize,
        id: String,
    },
    /// 正在压缩的临时条目等没有稳定身份的条目：只记录，不补偿。
    Raw,
}

/// 捕获 `scroll` 所在块。`snapshot` 必须是内容变化前的快照。
pub(crate) fn anchor_at(
    snapshot: &HistorySnapshot,
    scroll: usize,
    width: u16,
) -> Option<HistoryAnchor> {
    let entry = entry_at(snapshot, scroll)?;
    let key = entry_key(entry)?;
    let (block_index, offset) = block_at(entry, scroll)?;
    let block = entry.blocks.get(block_index)?;
    Some(HistoryAnchor {
        entry: key,
        block: block_index,
        source: block.source,
        offset,
        row: block.start.saturating_add(offset),
        width,
    })
}

/// 按新快照求锚点块的新绝对行；块/消息消失时按规则回退。
pub(crate) fn anchor_row(
    snapshot: &HistorySnapshot,
    source: HistorySource<'_>,
    anchor: &HistoryAnchor,
    width: u16,
) -> Option<usize> {
    if anchor.width != width {
        return None;
    }
    let entry = find_entry(snapshot, source, &anchor.entry)?;
    // 先按块身份找（块前插/汇报追加时序号会变）；身份也失配时退回序号。
    let block = entry
        .blocks
        .iter()
        .find(|block| block.source == anchor.source)
        .or_else(|| entry.blocks.get(anchor.block));
    let Some(block) = block else {
        // 块被折叠/撤销/压缩删除：消息还在就回退。正文块回退消息起始行；
        // Stats/Recap/Attachments/Notice 这类尾部块回退消息末尾附近——否则一条
        // 统计行消失会把视口向上甩出整条消息。
        let fallback = match anchor.source {
            BlockSource::Stats { .. }
            | BlockSource::Recap { .. }
            | BlockSource::RecapRunning { .. }
            | BlockSource::Attachments { .. }
            | BlockSource::Notice { .. } => entry.end.saturating_sub(1).max(entry.start),
            _ => entry.start,
        };
        return match anchor.entry {
            AnchorEntry::Message { .. } => Some(fallback),
            _ => None,
        };
    };
    let offset = anchor.offset.min(block.rows.saturating_sub(1));
    Some(block.start.saturating_add(offset))
}

/// 找包含 `scroll` 的条目；落在条目间空行时取下一个条目，末尾呼吸行取最后一条。
fn entry_at(snapshot: &HistorySnapshot, scroll: usize) -> Option<&SnapEntry> {
    let entries = &snapshot.entries;
    if entries.is_empty() {
        return None;
    }
    let after = entries.partition_point(|entry| entry.start <= scroll);
    if after == 0 {
        return entries.first();
    }
    let index = after - 1;
    let entry = entries.get(index)?;
    if scroll < entry.end {
        return Some(entry);
    }
    entries.get(index + 1).or(Some(entry))
}

/// 找条目内包含 `scroll` 的块；空行取下一块首行，末尾取最后一块末行。
fn block_at(entry: &SnapEntry, scroll: usize) -> Option<(usize, usize)> {
    let blocks = &entry.blocks;
    if blocks.is_empty() {
        return None;
    }
    let after = blocks.partition_point(|block| block.start <= scroll);
    if after == 0 {
        return Some((0, 0));
    }
    let index = after - 1;
    let block = blocks.get(index)?;
    if scroll < block.start.saturating_add(block.rows) {
        return Some((index, scroll.saturating_sub(block.start)));
    }
    if index + 1 < blocks.len() {
        return Some((index + 1, 0));
    }
    Some((index, block.rows.saturating_sub(1)))
}

/// 由快照条目映射稳定身份（ID 在构建快照时已从同一数据源取好）。
fn entry_key(entry: &SnapEntry) -> Option<AnchorEntry> {
    match entry.kind {
        SnapEntryKind::Message(index) => Some(AnchorEntry::Message {
            index,
            id: entry.id.clone()?,
        }),
        SnapEntryKind::Notices { last, .. } => Some(AnchorEntry::Notices {
            last,
            id: entry.id.clone()?,
        }),
        SnapEntryKind::Compact(index) => Some(AnchorEntry::Compact {
            index,
            id: entry.id.clone()?,
        }),
        SnapEntryKind::ModelSwitch { last, .. } => Some(AnchorEntry::ModelSwitch {
            last,
            id: entry.id.clone()?,
        }),
        SnapEntryKind::RunningCompact => Some(AnchorEntry::Raw),
    }
}

/// 按稳定身份在新快照里找条目。
fn find_entry<'a>(
    snapshot: &'a HistorySnapshot,
    source: HistorySource<'_>,
    key: &AnchorEntry,
) -> Option<&'a SnapEntry> {
    match key {
        AnchorEntry::Message { index, id } => {
            let resolved =
                resolve_index(source.messages, *index, id, |message| message.id.as_str())?;
            snapshot
                .entries
                .iter()
                .find(|entry| matches!(entry.kind, SnapEntryKind::Message(i) if i == resolved))
        }
        AnchorEntry::Notices { last, id } => {
            let index = resolve_index(source.notices, *last, id, |notice| notice.id.as_str())?;
            snapshot.entries.iter().find(|entry| {
                matches!(
                    entry.kind,
                    SnapEntryKind::Notices { last, count }
                        if index
                            >= last.saturating_sub(count.saturating_sub(1))
                            && index <= last
                )
            })
        }
        AnchorEntry::Compact { index, id } => {
            let resolved = resolve_index(source.checkpoints, *index, id, |item| item.id.as_str())?;
            snapshot
                .entries
                .iter()
                .find(|entry| matches!(entry.kind, SnapEntryKind::Compact(i) if i == resolved))
        }
        AnchorEntry::ModelSwitch { last, id } => {
            let index = resolve_index(source.switches, *last, id, |item| item.id.as_str())?;
            snapshot.entries.iter().find(|entry| {
                matches!(
                    entry.kind,
                    SnapEntryKind::ModelSwitch { first, last, .. }
                        if index >= first && index <= last
                )
            })
        }
        AnchorEntry::Raw => None,
    }
}

/// 先按捕获下标验证 ID，再全表按 ID 回退；两者都失败说明条目已被删除。
fn resolve_index<T>(
    items: &[T],
    captured: usize,
    id: &str,
    id_of: impl Fn(&T) -> &str,
) -> Option<usize> {
    if items.get(captured).is_some_and(|item| id_of(item) == id) {
        return Some(captured);
    }
    items.iter().position(|item| id_of(item) == id)
}
