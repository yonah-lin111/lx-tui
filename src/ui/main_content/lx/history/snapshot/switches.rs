//! 模型切换与权限切换时间线卡片的快照构造（从 `snapshot.rs` 拆分）。

use std::sync::Arc;

use super::super::cache::{
    BlockKey, MODEL_SWITCH_SLOT_BLOCK, PERMISSION_SWITCH_SLOT_BLOCK, model_switch_block_signature,
    permission_switch_block_signature,
};
use super::super::paint::{BlockPaint, BlockSource};
use super::super::source::HistorySource;
use super::super::{CardCols, CardKind, model_switch, permission_switch};
use super::{Builder, CardRegion, SnapEntry, SnapEntryKind};

impl Builder<'_> {
    /// 模型切换提示条目（连续多条合并为一条 `xN`）。
    pub(super) fn model_switch(
        &mut self,
        source: HistorySource<'_>,
        first: usize,
        last: usize,
        count: usize,
    ) {
        let start = self.rows;
        let slice = &source.switches[first..=last];
        let expanded = source.model_switch.is_range_expanded(slice);
        let key = BlockKey {
            message_index: last,
            step_index: MODEL_SWITCH_SLOT_BLOCK,
            width: self.width,
            sig: model_switch_block_signature(slice, expanded),
        };
        let card = model_switch::build(slice, self.width, expanded);
        let click = card.click_span();
        let block = self.block(
            key,
            BlockSource::ModelSwitch {
                first,
                last,
                expanded,
            },
            start,
            || BlockPaint::SystemEvent {
                card: Arc::new(card),
            },
        );
        self.rows += block.rows;
        // 点击面只覆盖注入提示词（标题、模型与跃迁链不响应）。
        if let Some(span) = click {
            self.snapshot.cards.push(CardRegion {
                kind: CardKind::ModelSwitch {
                    // 锚定区间首条：追加切换时 `first` 不变，展开态天然保持。
                    switch_id: source.switches[first].id.clone(),
                    expanded,
                },
                card_start: start + span.offset,
                card_rows: span.rows,
                card_cols: CardCols::Text(span.cols),
                clickable: true,
            });
        }
        self.snapshot.entries.push(SnapEntry {
            start,
            end: self.rows,
            kind: SnapEntryKind::ModelSwitch { first, last, count },
            id: source.switches.get(last).map(|item| item.id.clone()),
            blocks: vec![block],
        });
    }

    /// 权限切换提示条目（连续多条合并为一条 `xN`）。
    pub(super) fn permission_switch(
        &mut self,
        source: HistorySource<'_>,
        first: usize,
        last: usize,
        count: usize,
    ) {
        let start = self.rows;
        let slice = &source.perm_switches[first..=last];
        let expanded = source.permission_switch.is_range_expanded(slice);
        let key = BlockKey {
            message_index: last,
            step_index: PERMISSION_SWITCH_SLOT_BLOCK,
            width: self.width,
            sig: permission_switch_block_signature(slice, expanded),
        };
        let card = permission_switch::build(slice, self.width, expanded);
        let click = card.click_span();
        let block = self.block(
            key,
            BlockSource::PermissionSwitch {
                first,
                last,
                expanded,
            },
            start,
            || BlockPaint::SystemEvent {
                card: Arc::new(card),
            },
        );
        self.rows += block.rows;
        if let Some(span) = click {
            let switch_id = if expanded {
                slice
                    .iter()
                    .find(|item| source.permission_switch.is_expanded(&item.id))
                    .map(|item| item.id.clone())
                    .unwrap_or_else(|| source.perm_switches[first].id.clone())
            } else {
                source.perm_switches[first].id.clone()
            };
            self.snapshot.cards.push(CardRegion {
                kind: CardKind::PermissionSwitch {
                    switch_id,
                    expanded,
                },
                card_start: start + span.offset,
                card_rows: span.rows,
                card_cols: CardCols::Text(span.cols),
                clickable: true,
            });
        }
        self.snapshot.entries.push(SnapEntry {
            start,
            end: self.rows,
            kind: SnapEntryKind::PermissionSwitch { first, last, count },
            id: source.perm_switches.get(last).map(|item| item.id.clone()),
            blocks: vec![block],
        });
    }
}
