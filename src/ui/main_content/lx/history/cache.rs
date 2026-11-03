//! 历史渲染缓存：跨帧持有几何快照，按「内容宽度 + 列表指纹」失效。
//!
//! `RenderCache` 由事件循环持有、按窗格隔离；`PaneCache` 不持有 `Lx` 借用，
//! 只持有 `Arc<HistorySnapshot>`。渲染与命中共用同一份快照，避免每帧多次全量排版。

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::app::state::Lx;
use crate::layout::PaneId;
use crate::ui::markdown_render::IncrementalMarkdownState;

use super::paint::BlockPaint;
use super::snapshot::{self, HistorySnapshot};

mod block_signature;
mod signature;
#[cfg(test)]
mod tests;
mod tool_signature;

pub(crate) use block_signature::*;
pub(crate) use signature::*;

/// 二级缓存（绘制数据）的字节上限，**所有 lx 窗格共享**；超限按 `last_frame` 淘汰。
///
/// 由 [`RenderCache`] 平摊到各窗格（每格 `MAX_CACHED_BYTES / 窗格数`）：预算挂在
/// `PaneCache` 上只是记账实现，口径必须是总量——否则开 N 个窗格就是 N×32MB。
pub(crate) const MAX_CACHED_BYTES: usize = 32 * 1024 * 1024;

/// 块级缓存的键：消息/步骤定位 + 内容宽度 + 该块自身的内容指纹。
///
/// `step_index` 对非步骤块使用稳定哨兵（见 [`SHELL_SLOT_BLOCK`] 等）。
/// 指纹**不**包含 hover / loading 帧 / 滚动 / 选中态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct BlockKey {
    pub(crate) message_index: usize,
    pub(crate) step_index: usize,
    pub(crate) width: u16,
    pub(crate) sig: u64,
}

impl BlockKey {
    /// 物理槽位：指纹变化时原地覆写，不堆积流式中间版本。
    fn slot(self) -> BlockSlot {
        BlockSlot {
            message_index: self.message_index,
            step_index: self.step_index,
            width: self.width,
        }
    }
}

/// 块在缓存中的物理槽位（不含内容指纹；同一槽位始终只保留最新绘制数据）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct BlockSlot {
    message_index: usize,
    step_index: usize,
    width: u16,
}

/// 消息级本地命令卡片（一条消息一个块）。
pub(crate) const SHELL_SLOT_BLOCK: usize = usize::MAX;
/// 消息底部失败/中断通知。
pub(crate) const NOTICE_SLOT_BLOCK: usize = usize::MAX - 1;
/// 消息底部统计行。
pub(crate) const STATS_SLOT_BLOCK: usize = usize::MAX - 2;
/// 时间线里的独立撤销提示（无消息下标）。
pub(crate) const REWIND_SLOT_BLOCK: usize = usize::MAX - 3;
/// 时间线里的 compact 检查点。
pub(crate) const COMPACT_SLOT_BLOCK: usize = usize::MAX - 4;
/// 消息内的子 agent 汇报块（一条消息可有多份，按序号排列）。
pub(crate) const REPORT_SLOT_BLOCK: usize = usize::MAX - 5;
/// 时间线里的模型切换提示。
pub(crate) const MODEL_SWITCH_SLOT_BLOCK: usize = usize::MAX - 6;
/// 时间线里的正在压缩 loading 条目。
pub(crate) const RUNNING_COMPACT_SLOT_MESSAGE: usize = usize::MAX;
pub(crate) const RUNNING_COMPACT_SLOT_BLOCK: usize = usize::MAX - 7;

/// 滚离底部时，屏幕外流式增量的快照合并窗口（约 5 帧 @60fps）。
#[cfg(not(test))]
const OFFSCREEN_STREAM_COALESCE: Duration = Duration::from_millis(80);
#[cfg(test)]
const OFFSCREEN_STREAM_COALESCE: Duration = Duration::ZERO;

/// 二级缓存项。
#[derive(Debug)]
struct BlockEntry {
    /// 当前内容指纹（命中时校验）。
    sig: u64,
    paint: Arc<BlockPaint>,
    bytes: usize,
    /// 最近一次被**绘制路径**命中的帧号（构建快照不算使用）。
    last_frame: u64,
}

/// 跨帧渲染缓存：事件循环持有，按窗格隔离。
#[derive(Debug, Default)]
pub(crate) struct RenderCache {
    frame: u64,
    panes: HashMap<PaneId, PaneCache>,
}

impl RenderCache {
    /// 每帧开始时推进帧号，并按当前窗格数重算各自的字节份额。
    pub(crate) fn begin_frame(&mut self) {
        self.frame = self.frame.wrapping_add(1);
        self.share_budget();
    }

    /// 取（或创建）指定窗格的缓存；同时把当前帧号写进窗格。
    pub(crate) fn pane(&mut self, id: PaneId) -> &mut PaneCache {
        let frame = self.frame;
        let created = !self.panes.contains_key(&id);
        if created {
            self.panes.entry(id).or_default();
            // 窗格数变了：份额重新平摊（新窗格不许多占，老窗格也要收紧）。
            self.share_budget();
        }
        let pane = self.panes.entry(id).or_default();
        pane.frame = frame;
        pane
    }

    /// 窗格关闭后清理，避免缓存泄漏。
    pub(crate) fn retain_panes(&mut self, alive: &[PaneId]) {
        self.panes.retain(|id, _| alive.contains(id));
        self.share_budget();
    }

    /// 把 [`MAX_CACHED_BYTES`] 平摊给当前所有窗格。
    ///
    /// 平摊是**记账口径**：预算本就是总量，各窗格按份数收着，超出的部分在下一次
    /// `render` 的回收点被淘汰。没有窗格时不设限（无缓存可记）。
    fn share_budget(&mut self) {
        if self.panes.is_empty() {
            return;
        }
        let per = MAX_CACHED_BYTES / self.panes.len();
        for pane in self.panes.values_mut() {
            pane.set_limit(per);
        }
    }
}

/// 单窗格缓存：一级几何快照 + 二级绘制数据。
#[derive(Debug)]
pub(crate) struct PaneCache {
    /// 本帧帧号（由 [`RenderCache::pane`] 写入）。
    frame: u64,
    snapshot: Option<(u64, Arc<HistorySnapshot>)>,
    /// 当前快照的内容宽度（`snapshot` 为 None 时无意义）。
    snapshot_width: Option<u16>,
    /// 构建快照时「除流式末条消息外」的列表指纹（屏幕外节流判定用）。
    snapshot_non_stream: u64,
    /// 最近一次快照构建时刻。
    snapshot_at: Option<Instant>,
    /// 历史视口行数（`history::view` 每帧写入；0 = 未知，不做屏幕外节流）。
    viewport_rows: usize,
    /// 屏幕外流式合并窗口（测试可调；生产固定 80ms）。
    offscreen_coalesce: Duration,
    /// 快照重建次数；仅供测试断言「同一份状态只重建一次」。
    builds: u64,
    blocks: HashMap<BlockSlot, BlockEntry>,
    /// 流式 Markdown 的增量解析状态（按物理槽位，含已固化前缀）。
    md_streams: HashMap<BlockSlot, IncrementalMarkdownState>,
    /// 已缓存绘制数据的近似字节数。
    bytes: usize,
    /// 二级缓存字节上限（生产恒为 [`MAX_CACHED_BYTES`]；测试可调小）。
    limit: usize,
}

impl Default for PaneCache {
    fn default() -> Self {
        Self {
            frame: 0,
            snapshot: None,
            snapshot_width: None,
            snapshot_non_stream: 0,
            snapshot_at: None,
            viewport_rows: 0,
            offscreen_coalesce: OFFSCREEN_STREAM_COALESCE,
            builds: 0,
            blocks: HashMap::new(),
            md_streams: HashMap::new(),
            bytes: 0,
            limit: MAX_CACHED_BYTES,
        }
    }
}

impl PaneCache {
    /// 调小二级缓存上限（测试与 [`RenderCache`] 平摊预算用）。
    pub(crate) fn set_limit(&mut self, limit: usize) {
        self.limit = limit;
        self.evict();
    }

    /// 当前宽度与列表状态下的快照；指纹不变时直接复用同一个 `Arc`。
    pub(crate) fn snapshot(&mut self, width: u16, lx: &Lx) -> Arc<HistorySnapshot> {
        self.snapshot_source(width, super::source::HistorySource::from(lx))
    }

    /// 数据源版本（主窗格与子 agent 详情共用同一份缓存实现）。
    pub(crate) fn snapshot_source(
        &mut self,
        width: u16,
        source: super::source::HistorySource<'_>,
    ) -> Arc<HistorySnapshot> {
        let signature = list_signature_source(source, width);
        if let Some((cached, snapshot)) = &self.snapshot
            && *cached == signature
        {
            return Arc::clone(snapshot);
        }
        if self.offscreen_stream_throttled(width, source)
            && let Some((_, snapshot)) = &self.snapshot
        {
            return Arc::clone(snapshot);
        }
        // 先释放上一帧快照持有的 `BlockPaint`（含增量 Markdown 前缀），
        // 让 `IncrementalMarkdownState::update` 的 `Arc::make_mut` 能原地追加。
        self.snapshot = None;
        let snapshot = Arc::new(snapshot::build(width, source, self));
        self.builds = self.builds.saturating_add(1);
        self.snapshot = Some((signature, Arc::clone(&snapshot)));
        self.snapshot_width = Some(width);
        self.snapshot_non_stream = non_stream_signature_source(source, width);
        self.snapshot_at = Some(Instant::now());
        self.evict();
        snapshot
    }

    /// 读取当前宽度下**已就绪**的几何快照（供高频鼠标滚轮、拖拽、Hover 调用）。
    ///
    /// 已有同宽度快照时直接 O(1) 克隆返回，绝不在鼠标事件回调内同步扫描指纹与重排
    /// Markdown；仅在冷启动或宽度变化尚未构建快照时回退到完整构建。
    pub(crate) fn snapshot_ready_source(
        &mut self,
        width: u16,
        source: super::source::HistorySource<'_>,
    ) -> Arc<HistorySnapshot> {
        if self.snapshot_width == Some(width)
            && let Some((_, snapshot)) = &self.snapshot
        {
            return Arc::clone(snapshot);
        }
        self.snapshot_source(width, source)
    }

    /// 记录历史视口行数（屏幕外流式节流用）。
    pub(crate) fn set_viewport_rows(&mut self, rows: usize) {
        self.viewport_rows = rows;
    }

    /// 调小屏幕外流式合并窗口（测试用；0 = 关闭节流）。
    #[cfg(test)]
    pub(crate) fn set_offscreen_coalesce(&mut self, interval: Duration) {
        self.offscreen_coalesce = interval;
    }

    /// 仅当变化局限在**视口下方**的流式末条消息时才允许合并更新。
    fn offscreen_stream_throttled(
        &self,
        width: u16,
        source: super::source::HistorySource<'_>,
    ) -> bool {
        if self.offscreen_coalesce.is_zero() || source.chat.follow_bottom || self.viewport_rows == 0
        {
            return false;
        }
        let Some((_, snapshot)) = &self.snapshot else {
            return false;
        };
        if self.snapshot_width != Some(width) {
            return false;
        }
        let Some(built_at) = self.snapshot_at else {
            return false;
        };
        if built_at.elapsed() >= self.offscreen_coalesce {
            return false;
        }
        // 只有流式末条消息变了（未读界面的其他状态一律不合并）。
        if non_stream_signature_source(source, width) != self.snapshot_non_stream {
            return false;
        }
        let Some(last) = snapshot.messages.last() else {
            return false;
        };
        // 流式消息整块位于视口下方：重排不会改变可见像素。
        last.start
            >= source
                .chat
                .history_scroll
                .saturating_add(self.viewport_rows)
    }

    /// 取块级绘制数据：命中直接复用；未命中调用 `build` 并记账。
    ///
    /// **命中不刷新 `last_frame`**：快照重建时会遍历全部块，若在这里刷新，
    /// 一次流式追加就会把所有缓存标成「本帧使用」，LRU 将永远无法淘汰。
    /// 「本帧真的画过」由 [`Self::mark_used`] 记录。
    pub(crate) fn block(
        &mut self,
        key: BlockKey,
        build: impl FnOnce() -> Arc<BlockPaint>,
    ) -> Arc<BlockPaint> {
        if let Some(entry) = self.blocks.get(&key.slot())
            && entry.sig == key.sig
        {
            return Arc::clone(&entry.paint);
        }
        let paint = build();
        let bytes = paint.bytes();
        self.insert_block(key, Arc::clone(&paint), bytes);
        paint
    }

    /// 助手正文的增量 Markdown 绘制数据：同一槽位原地覆写，只重排尾部。
    pub(crate) fn markdown_block(
        &mut self,
        key: BlockKey,
        content: &str,
        width: u16,
    ) -> Arc<BlockPaint> {
        let slot = key.slot();
        if let Some(entry) = self.blocks.get(&slot)
            && entry.sig == key.sig
        {
            return Arc::clone(&entry.paint);
        }
        // 旧绘制数据持有前缀 `Arc`：必须先释放，`Arc::make_mut` 才能原地追加。
        if let Some(previous) = self.blocks.remove(&slot) {
            self.bytes = self.bytes.saturating_sub(previous.bytes);
        }
        let state = self.md_streams.entry(slot).or_default();
        let (prefix, prefix_bytes, tail) = state.update(content, width);
        let paint = Arc::new(BlockPaint::Markdown {
            prefix,
            prefix_bytes,
            tail,
        });
        let bytes = paint.bytes();
        self.insert_block(key, Arc::clone(&paint), bytes);
        paint
    }

    /// 推进本窗格帧号（测试用：模拟事件循环的 `begin_frame`）。
    #[cfg(test)]
    pub(crate) fn advance_frame(&mut self) {
        self.frame = self.frame.wrapping_add(1);
    }

    /// 标记该块本帧被绘制过（刷新 LRU 时间戳）。
    pub(crate) fn mark_used(&mut self, key: BlockKey) {
        let frame = self.frame;
        if let Some(entry) = self.blocks.get_mut(&key.slot()) {
            entry.last_frame = frame;
        }
    }

    fn insert_block(&mut self, key: BlockKey, paint: Arc<BlockPaint>, bytes: usize) {
        // 新块算「本帧使用过一次」：本帧正在构建的快照不许在插入过程中被自己淘汰。
        let last_frame = self.frame;
        if let Some(previous) = self.blocks.insert(
            key.slot(),
            BlockEntry {
                sig: key.sig,
                paint,
                bytes,
                last_frame,
            },
        ) {
            self.bytes = self.bytes.saturating_sub(previous.bytes);
        }
        self.bytes = self.bytes.saturating_add(bytes);
        self.evict();
    }

    /// 按当前 LRU 时间戳回收，守住字节上限。
    ///
    /// **调用点必须在绘制之后**（`render` 的 `mark_used` 循环之后）：`evict` 只淘汰
    /// `last_frame < frame` 的块，早于 `mark_used` 调用会把本帧刚画过的块当成冷块淘汰，
    /// 下一帧又整块重排，缓存反而变成抖动源。
    ///
    /// 两次快照重建之间（冷启动后静置、只滚动不产生新内容）也需要回收点：重建时插入的
    /// 块全是「本帧使用」，当帧 `evict` 无处可淘汰；没有这里，`bytes` 会一直停在上限之上。
    pub(crate) fn reclaim(&mut self) {
        self.evict();
    }

    /// 超出字节上限时按 `last_frame` 升序淘汰；**本帧画过的块不淘汰**。
    fn evict(&mut self) {
        while self.bytes > self.limit {
            let frame = self.frame;
            let victim = self
                .blocks
                .iter()
                .filter(|(_, entry)| entry.last_frame < frame)
                .min_by_key(|(_, entry)| entry.last_frame)
                .map(|(key, _)| *key);
            let Some(victim) = victim else {
                break;
            };
            if let Some(entry) = self.blocks.remove(&victim) {
                self.bytes = self.bytes.saturating_sub(entry.bytes);
                // 增量状态与块同生命周期：块被回收后前缀不再与任何绘制数据共享，
                // 留着只会白占内存。
                self.md_streams.remove(&victim);
            }
        }
    }

    /// 已缓存绘制数据的近似字节数（测试断言上限用）。
    #[cfg(test)]
    pub(crate) fn bytes(&self) -> usize {
        self.bytes
    }

    /// 已缓存的块数（测试断言淘汰用）。
    #[cfg(test)]
    pub(crate) fn block_count(&self) -> usize {
        self.blocks.len()
    }

    /// 已重建次数（测试断言用）。
    #[cfg(test)]
    pub(crate) fn builds(&self) -> u64 {
        self.builds
    }
}
