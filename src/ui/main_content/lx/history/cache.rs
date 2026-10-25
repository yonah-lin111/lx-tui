//! 历史渲染缓存：跨帧持有几何快照，按「内容宽度 + 列表指纹」失效。
//!
//! `RenderCache` 由事件循环持有、按窗格隔离；`PaneCache` 不持有 `Lx` 借用，
//! 只持有 `Arc<HistorySnapshot>`。渲染与命中共用同一份快照，避免每帧多次全量排版。

use std::collections::HashMap;
use std::sync::Arc;

use crate::app::state::{
    AssistantStepView, CardSection, ChatMessageView, Lx, ToolCallView, ToolDetailView,
};
use crate::layout::PaneId;

use super::paint::BlockPaint;
use super::snapshot::{self, HistorySnapshot};

/// 二级缓存（绘制数据）的字节上限；超限按 `last_frame` 淘汰。
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

/// 二级缓存项。
#[derive(Debug)]
struct BlockEntry {
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
    /// 每帧开始时推进帧号。
    pub(crate) fn begin_frame(&mut self) {
        self.frame = self.frame.wrapping_add(1);
    }

    /// 取（或创建）指定窗格的缓存；同时把当前帧号写进窗格。
    pub(crate) fn pane(&mut self, id: PaneId) -> &mut PaneCache {
        let frame = self.frame;
        let pane = self.panes.entry(id).or_default();
        pane.frame = frame;
        pane
    }

    /// 窗格关闭后清理，避免缓存泄漏。
    pub(crate) fn retain_panes(&mut self, alive: &[PaneId]) {
        self.panes.retain(|id, _| alive.contains(id));
    }
}

/// 单窗格缓存：一级几何快照 + 二级绘制数据。
#[derive(Debug)]
pub(crate) struct PaneCache {
    /// 本帧帧号（由 [`RenderCache::pane`] 写入）。
    frame: u64,
    snapshot: Option<(u64, Arc<HistorySnapshot>)>,
    /// 快照重建次数；仅供测试断言「同一份状态只重建一次」。
    builds: u64,
    blocks: HashMap<BlockKey, BlockEntry>,
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
            builds: 0,
            blocks: HashMap::new(),
            bytes: 0,
            limit: MAX_CACHED_BYTES,
        }
    }
}

impl PaneCache {
    /// 调小二级缓存上限（测试淘汰路径用）。
    #[cfg(test)]
    pub(crate) fn set_limit(&mut self, limit: usize) {
        self.limit = limit;
        self.evict();
    }

    /// 当前宽度与列表状态下的快照；指纹不变时直接复用同一个 `Arc`。
    pub(crate) fn snapshot(&mut self, width: u16, lx: &Lx) -> Arc<HistorySnapshot> {
        let signature = list_signature(lx, width);
        if let Some((cached, snapshot)) = &self.snapshot
            && *cached == signature
        {
            return Arc::clone(snapshot);
        }
        let snapshot = Arc::new(snapshot::build(width, lx, self));
        self.builds = self.builds.saturating_add(1);
        self.snapshot = Some((signature, Arc::clone(&snapshot)));
        self.evict();
        snapshot
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
        if let Some(entry) = self.blocks.get(&key) {
            return Arc::clone(&entry.paint);
        }
        let paint = build();
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
        if let Some(entry) = self.blocks.get_mut(&key) {
            entry.last_frame = frame;
        }
    }

    fn insert_block(&mut self, key: BlockKey, paint: Arc<BlockPaint>, bytes: usize) {
        // 新块算「本帧使用过一次」：本帧正在构建的快照不许在插入过程中被自己淘汰。
        let last_frame = self.frame;
        if let Some(previous) = self.blocks.insert(
            key,
            BlockEntry {
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

/// FNV-1a 64 位哈希（不引第三方依赖）。
struct Fnv(u64);

impl Fnv {
    const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;

    fn new() -> Self {
        Self(Self::OFFSET_BASIS)
    }

    fn write(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.0 ^= u64::from(*byte);
            self.0 = self.0.wrapping_mul(Self::PRIME);
        }
    }

    fn write_u8(&mut self, value: u8) {
        self.write(&[value]);
    }

    fn write_u16(&mut self, value: u16) {
        self.write(&value.to_le_bytes());
    }

    fn write_u64(&mut self, value: u64) {
        self.write(&value.to_le_bytes());
    }

    fn write_usize(&mut self, value: usize) {
        self.write(&(value as u64).to_le_bytes());
    }

    fn write_i64(&mut self, value: i64) {
        self.write(&value.to_le_bytes());
    }

    fn write_str(&mut self, value: &str) {
        self.write_usize(value.len());
        self.write(value.as_bytes());
    }

    /// 只写首尾各 64 字节：流式追加、环形缓冲裁剪都必然改变其一，且不扫全文。
    fn write_edges(&mut self, value: &str) {
        let bytes = value.as_bytes();
        let head = &bytes[..bytes.len().min(64)];
        let tail = &bytes[bytes.len().saturating_sub(64)..];
        self.write(head);
        self.write(tail);
    }

    fn finish(self) -> u64 {
        self.0
    }
}

/// 列表指纹：只读状态、O(步骤数)，覆盖一切影响行数或正文的字段。
///
/// **不**包含：hover、loading 帧、跟随底部、视口位置、置顶抑制记忆、滚动位置。
pub(crate) fn list_signature(lx: &Lx, width: u16) -> u64 {
    let mut hash = Fnv::new();
    hash.write_u16(width);
    hash.write_usize(lx.chat.messages.len());
    hash.write_usize(lx.chat.notices.len());
    hash.write_usize(lx.compact.checkpoints.len());
    for (index, message) in lx.chat.messages.iter().enumerate() {
        hash.write_usize(index);
        hash.write_str(&message.id);
        hash.write_i64(message.sequence);
        hash.write_str(&message.role);
        hash.write_str(&message.status);
        hash.write_str(&message.kind);
        hash.write_str(message.model_label.as_deref().unwrap_or(""));
        hash.write_str(message.error_code.as_deref().unwrap_or(""));
        hash.write_i64(message.input_tokens.unwrap_or(-1));
        hash.write_i64(message.output_tokens.unwrap_or(-1));
        hash.write_i64(message.cache_read_tokens.unwrap_or(-1));
        hash.write_i64(message.duration_ms.unwrap_or(-1));
        if let Some(shell) = &message.shell {
            shell_signature(&mut hash, lx, message, shell);
        } else {
            hash.write_u8(0);
        }
        hash.write_usize(message.steps.len());
        for (step_index, step) in message.steps.iter().enumerate() {
            hash.write_usize(step_index);
            step_signature(&mut hash, lx, message, index, step_index, step);
        }
    }
    for notice in &lx.chat.notices {
        hash.write_str(&notice.id);
        hash.write_i64(notice.sequence);
        hash.write_str(&notice.kind);
        hash.write_usize(notice.restored_file_count);
        hash.write_u8(u8::from(notice.files_already_restored));
    }
    for checkpoint in &lx.compact.checkpoints {
        hash.write_str(&checkpoint.id);
        hash.write_i64(checkpoint.sequence);
        hash.write_str(&checkpoint.model_label);
        hash.write_i64(checkpoint.created_at_ms);
        hash.write_i64(checkpoint.start_sequence);
        hash.write_i64(checkpoint.end_sequence);
        hash.write_str(&checkpoint.summary);
        hash.write_u8(u8::from(checkpoint.active));
        hash.write_u8(u8::from(
            lx.compact.expanded.iter().any(|id| id == &checkpoint.id),
        ));
    }
    hash.finish()
}

/// 本地命令卡片：正文与折叠状态影响行数。
fn shell_signature(
    hash: &mut Fnv,
    lx: &Lx,
    message: &ChatMessageView,
    shell: &crate::app::state::UserShellView,
) {
    hash.write_u8(1);
    // 参数区/输出区各自独立折叠，必须进指纹。
    hash.write_u8(u8::from(lx.chat.section_is_expanded(
        &message.id,
        crate::app::state::SHELL_SLOT,
        CardSection::Body,
    )));
    hash.write_u8(u8::from(lx.chat.section_is_expanded(
        &message.id,
        crate::app::state::SHELL_SLOT,
        CardSection::Output,
    )));
    hash.write_str(&shell.command);
    hash.write_str(shell.state.as_str());
    hash.write_i64(shell.exit_code.map_or(-1, i64::from));
    hash.write_u64(shell.duration_ms.unwrap_or(u64::MAX));
    hash.write_usize(shell.output.len());
    hash.write_edges(&shell.output);
    hash.write_u64(shell.dropped_bytes);
    hash.write_u8(u8::from(shell.truncated));
    hash.write_u64(shell.revision);
}

/// 单个步骤：Part 覆盖正文长度与首尾字节；Tool 覆盖调用身份、状态、折叠与详情量。
fn step_signature(
    hash: &mut Fnv,
    lx: &Lx,
    message: &ChatMessageView,
    message_index: usize,
    step_index: usize,
    step: &AssistantStepView,
) {
    match step {
        AssistantStepView::Part { part, .. } => {
            hash.write_u8(0);
            hash.write_u8(match part.kind {
                crate::chat::events::PartKind::Text => 0,
                crate::chat::events::PartKind::Reasoning => 1,
            });
            hash.write_usize(part.content.len());
            hash.write_edges(&part.content);
            hash.write_i64(part.started_at_ms.unwrap_or(-1));
            hash.write_i64(part.ended_at_ms.unwrap_or(-1));
            hash.write_i64(part.completed_at_ms.unwrap_or(-1));
            hash.write_u8(u8::from(
                lx.chat.reasoning_expanded(message_index, step_index),
            ));
        }
        AssistantStepView::Tool { call, .. } => {
            hash.write_u8(1);
            tool_signature(hash, lx, message, call);
        }
    }
}

/// 工具调用：身份、状态、折叠状态与详情中会改变行数的量。
fn tool_signature(hash: &mut Fnv, lx: &Lx, message: &ChatMessageView, call: &ToolCallView) {
    hash.write_str(&call.call_id);
    hash.write_str(&call.name);
    hash.write_u8(tool_status_code(call.status));
    // 折叠状态：参数区（Body）与输出区（Output）各自独立。
    hash.write_u8(u8::from(lx.chat.section_is_expanded(
        &message.id,
        &call.call_id,
        CardSection::Body,
    )));
    hash.write_u8(u8::from(lx.chat.section_is_expanded(
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

/// 单个步骤块的指纹（宽度由 [`BlockKey::width`] 承担，这里不含宽度）。
pub(crate) fn part_block_signature(
    lx: &Lx,
    message: &ChatMessageView,
    message_index: usize,
    step_index: usize,
    step: &AssistantStepView,
) -> u64 {
    let mut hash = Fnv::new();
    step_signature(&mut hash, lx, message, message_index, step_index, step);
    hash.finish()
}

/// 合并的文件工具组指纹：逐调用独立（含各自折叠状态）。
pub(crate) fn group_block_signature(
    lx: &Lx,
    message: &ChatMessageView,
    group: &[&ToolCallView],
    expanded: &[bool],
) -> u64 {
    let mut hash = Fnv::new();
    hash.write_usize(group.len());
    for (index, call) in group.iter().enumerate() {
        hash.write_usize(index);
        tool_signature(&mut hash, lx, message, call);
        hash.write_u8(u8::from(expanded.get(index).copied().unwrap_or(false)));
    }
    hash.finish()
}

/// 本地命令卡片的指纹（不含宽度）。
pub(crate) fn shell_block_signature(
    lx: &Lx,
    message: &ChatMessageView,
    shell: &crate::app::state::UserShellView,
) -> u64 {
    let mut hash = Fnv::new();
    shell_signature(&mut hash, lx, message, shell);
    hash.finish()
}

/// 消息底部通知的指纹（命中消息通知文案）。
pub(crate) fn notice_block_signature(notice: &super::notice::Notice) -> u64 {
    let mut hash = Fnv::new();
    hash.write_u8(match notice.title {
        super::notice::NoticeTitle::Failed => 0,
        super::notice::NoticeTitle::Interrupted => 1,
    });
    hash.write_str(&notice.reason);
    hash.finish()
}

/// 统计行的指纹。
pub(crate) fn stats_block_signature(line: &str) -> u64 {
    let mut hash = Fnv::new();
    hash.write_str(line);
    hash.finish()
}

/// 撤销提示条目的指纹。
pub(crate) fn rewind_block_signature(
    notice: &crate::app::state::RewindNoticeView,
    count: usize,
) -> u64 {
    let mut hash = Fnv::new();
    hash.write_usize(count);
    hash.write_str(&notice.id);
    hash.write_str(&notice.kind);
    hash.write_usize(notice.restored_file_count);
    hash.write_u8(u8::from(notice.files_already_restored));
    hash.finish()
}

/// compact 检查点条目的指纹。
pub(crate) fn compact_block_signature(
    notice: &crate::app::state::CompactNoticeView,
    expanded: bool,
) -> u64 {
    let mut hash = Fnv::new();
    hash.write_str(&notice.id);
    hash.write_str(&notice.model_label);
    hash.write_i64(notice.created_at_ms);
    hash.write_str(&notice.summary);
    hash.write_u8(u8::from(notice.active));
    hash.write_u8(u8::from(expanded));
    hash.finish()
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

#[cfg(test)]
mod tests;
