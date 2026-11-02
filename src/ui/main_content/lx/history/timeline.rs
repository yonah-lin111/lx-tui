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
    /// 连续模型切换合并为一条；`first` 取初始起点模型，`last` 取最终目标模型与提示词，`count` 用于 `xN`。
    ModelSwitch {
        first: usize,
        last: usize,
        count: usize,
    },
    /// 正在执行的 compact 压缩请求（动态 loading 条目）。
    RunningCompact,
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
    let mut out = Vec::with_capacity(
        source.messages.len()
            + source.notices.len()
            + source.checkpoints.len()
            + source.switches.len(),
    );
    let mut message = 0usize;
    let mut notice = 0usize;
    let mut checkpoint = 0usize;
    let mut switch_idx = 0usize;
    while message < source.messages.len()
        || notice < source.notices.len()
        || checkpoint < source.checkpoints.len()
        || switch_idx < source.switches.len()
    {
        // 旧版 wait 残留：没有同轮 assistant 的报告 user 消息不进入时间线。
        while message < source.messages.len() && is_legacy_report_row(source.messages, message) {
            message += 1;
        }
        let next_message = source.messages.get(message).map(|item| item.sequence);
        let next_notice = source.notices.get(notice).map(|item| item.sequence);
        let next_checkpoint = source.checkpoints.get(checkpoint).map(|item| item.sequence);
        let next_switch = source.switches.get(switch_idx).map(|item| item.sequence);
        let Some(smallest) = [next_message, next_notice, next_checkpoint, next_switch]
            .into_iter()
            .flatten()
            .min()
        else {
            break;
        };
        if next_message == Some(smallest) {
            out.push(TimelineEntry::Message(message));
            message += 1;
            continue;
        }
        if next_notice == Some(smallest) {
            // 连续提示合并：吸收下一条条目前的所有提示（它们按 sequence 相邻）。
            notice += 1;
            let mut count = 1;
            while notice < source.notices.len() {
                let next = source.notices[notice].sequence;
                let blocked = [next_message, next_checkpoint, next_switch]
                    .into_iter()
                    .flatten()
                    .any(|sequence| sequence <= next);
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
        if next_checkpoint == Some(smallest) {
            out.push(TimelineEntry::Compact(checkpoint));
            checkpoint += 1;
            continue;
        }
        // 连续模型切换合并为一条 `xN`
        let first = switch_idx;
        switch_idx += 1;
        let mut count = 1;
        while switch_idx < source.switches.len() {
            let next = source.switches[switch_idx].sequence;
            let blocked = [next_message, next_notice, next_checkpoint]
                .into_iter()
                .flatten()
                .any(|sequence| sequence <= next);
            if blocked {
                break;
            }
            count += 1;
            switch_idx += 1;
        }
        out.push(TimelineEntry::ModelSwitch {
            first,
            last: switch_idx - 1,
            count,
        });
    }
    if source.compact.is_running() {
        out.push(TimelineEntry::RunningCompact);
    }
    out
}
