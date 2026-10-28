//! 历史时间线合并：真实消息、独立撤销提示与 compact 检查点按 DB sequence 排序。
//!
//! 三者共用同一编号空间（DB 唯一分配），因此按 sequence 升序交叉合并即可。
//! 连续撤销提示合并成一条 `xN`，compact 检查点不合并（每条摘要都要独立展开）。

use crate::app::state::Lx;

use super::source::{HistorySource, is_legacy_report_row};

/// 历史时间线条目：真实消息、独立撤销提示与 compact 检查点。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TimelineEntry {
    Message(usize),
    /// 连续撤销提示合并为一条；`last` 是最后一条（描述取它），`count` 用于 `xN`。
    Notices {
        last: usize,
        count: usize,
    },
    /// compact 检查点（独立条目；不合并，每条各自可折叠）。
    Compact(usize),
}

/// 是否有可绘制的历史（消息为空但提示/检查点非空时仍显示历史，不退回欢迎页）。
pub fn has_history(lx: &Lx) -> bool {
    has_history_source(HistorySource::from(lx))
}

/// 数据源版本（子 agent 详情复用）。
pub(super) fn has_history_source(source: HistorySource<'_>) -> bool {
    source.has_history()
}

/// 按 sequence 合并消息、撤销提示与 compact 检查点。
///
/// 同序号不可能出现（DB 唯一分配），保险起见消息在前、其次提示、最后检查点。
pub(super) fn timeline(source: HistorySource<'_>) -> Vec<TimelineEntry> {
    let mut out =
        Vec::with_capacity(source.messages.len() + source.notices.len() + source.checkpoints.len());
    let mut message = 0usize;
    let mut notice = 0usize;
    let mut checkpoint = 0usize;
    while message < source.messages.len()
        || notice < source.notices.len()
        || checkpoint < source.checkpoints.len()
    {
        // 旧版 wait 残留：没有同轮 assistant 的报告 user 消息不进入时间线。
        while message < source.messages.len() && is_legacy_report_row(source.messages, message) {
            message += 1;
        }
        let next_message = source.messages.get(message).map(|item| item.sequence);
        let next_notice = source.notices.get(notice).map(|item| item.sequence);
        let next_checkpoint = source.checkpoints.get(checkpoint).map(|item| item.sequence);
        // 取三者中最小 sequence 的条目（相等时消息优先，其次提示，最后检查点）。
        let take_message = match (next_message, next_notice, next_checkpoint) {
            (None, None, None) => break,
            (Some(sequence), notice, checkpoint) => {
                let mut smallest = sequence;
                for other in [notice, checkpoint].into_iter().flatten() {
                    smallest = smallest.min(other);
                }
                sequence == smallest
            }
            _ => false,
        };
        if take_message {
            out.push(TimelineEntry::Message(message));
            message += 1;
            continue;
        }
        let take_notice = match (next_notice, next_checkpoint) {
            (Some(sequence), Some(other)) => sequence <= other,
            (Some(_), None) => true,
            (None, _) => false,
        };
        if take_notice {
            // 连续提示合并：吸收下一条条目前的所有提示（它们按 sequence 相邻）。
            notice += 1;
            let mut count = 1;
            while notice < source.notices.len() {
                let next = source.notices[notice].sequence;
                let blocked = next_message.is_some_and(|sequence| sequence <= next)
                    || next_checkpoint.is_some_and(|sequence| sequence <= next);
                if blocked {
                    break;
                }
                count += 1;
                notice += 1;
            }
            out.push(TimelineEntry::Notices {
                last: notice - 1,
                count,
            });
            continue;
        }
        out.push(TimelineEntry::Compact(checkpoint));
        checkpoint += 1;
    }
    out
}
