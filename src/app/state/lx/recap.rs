//! 每窗格 recap 状态：已生成条目、在途请求与自动触发进度。
//!
//! recap 是旁路视图数据：不进入模型上下文、不占时间线 sequence；按 assistant
//! 消息 id 挂接，随会话加载/rewind/fork 重建。渲染层只读本状态。

use std::collections::{BTreeSet, HashMap};
use std::time::Instant;

use crate::db::chat::MessageRecap;

/// 一条已生成的回顾展示视图。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageRecapView {
    pub id: String,
    pub summary: String,
    pub next_action: String,
    pub model_label: String,
    pub end_turn: i64,
    pub end_sequence: i64,
    pub created_at_ms: i64,
}

impl MessageRecapView {
    /// 由持久化记录构造。
    pub fn from_record(record: &MessageRecap) -> Self {
        Self {
            id: record.id.clone(),
            summary: record.summary.clone(),
            next_action: record.next_action.clone(),
            model_label: record.model_label.clone(),
            end_turn: record.end_turn,
            end_sequence: record.end_sequence,
            created_at_ms: record.created_at_ms,
        }
    }
}

/// 一次在途 recap 请求。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecapPending {
    /// 请求代次；迟到回执按它丢弃。
    pub generation: u64,
    /// 手动 `/recap` 为 true，自动触发为 false。
    pub manual: bool,
    /// 挂接的 assistant 消息 id。
    pub message_id: String,
    /// 覆盖到的轮次与 sequence。
    pub end_turn: i64,
    pub end_sequence: i64,
    /// 请求开始时刻与已耗时（渲染 `Generating recap... · Ns`）。
    pub started_at: Instant,
    pub elapsed_ms: u64,
}

/// 单个窗格持有的 recap 状态（随会话重置）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LxRecap {
    /// message_id → 已生成回顾。
    pub entries: HashMap<String, MessageRecapView>,
    /// 在途请求；None 表示空闲。
    pub pending: Option<RecapPending>,
    /// 下一个请求代次。
    pub next_generation: u64,
    /// 本会话已完成的轮次数（由 `completed_turn_indices` 派生）。
    pub completed_turns: u32,
    /// 已完成的轮次下标集合：同一轮多条 assistant（含报告自动轮）只计一次。
    pub completed_turn_indices: BTreeSet<i64>,
    /// 上次成功 recap 覆盖到的 turn_index；None 表示本会话尚未 recap。
    pub last_recap_turn: Option<i64>,
    /// 上次成功 recap 覆盖到的 DB sequence。
    pub last_recap_sequence: i64,
}

impl LxRecap {
    /// 是否没有任何回顾与在途请求。
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty() && self.pending.is_none()
    }

    /// 会话切换/新建时整体清空。
    pub fn reset(&mut self) {
        self.entries.clear();
        self.pending = None;
        self.next_generation = 0;
        self.completed_turns = 0;
        self.completed_turn_indices.clear();
        self.last_recap_turn = None;
        self.last_recap_sequence = 0;
    }

    /// 加载会话后按消息与已存回顾重建进度。
    pub fn reset_for_session(
        &mut self,
        completed_turn_indices: BTreeSet<i64>,
        last_recap_turn: Option<i64>,
        last_recap_sequence: i64,
    ) {
        self.entries.clear();
        self.pending = None;
        self.completed_turns = u32::try_from(completed_turn_indices.len()).unwrap_or(u32::MAX);
        self.completed_turn_indices = completed_turn_indices;
        self.last_recap_turn = last_recap_turn;
        self.last_recap_sequence = last_recap_sequence;
    }

    /// 一轮正常完成（StopReason::Complete）；同一轮重复完成只计一次。
    pub fn on_turn_completed(&mut self, turn_index: i64) {
        self.completed_turn_indices.insert(turn_index);
        self.completed_turns = u32::try_from(self.completed_turn_indices.len()).unwrap_or(u32::MAX);
    }

    /// 分配下一个请求代次。
    pub fn allocate_generation(&mut self) -> u64 {
        self.next_generation = self.next_generation.wrapping_add(1);
        self.next_generation
    }

    /// 是否有足够的轮次与间隔触发自动 recap。
    ///
    /// 「距上次 recap 至少 2 轮」按 turn_index 比较，不能拿轮次总数减 turn_index：
    /// 报告自动轮会让 turn_index 跳号，两者语义不同。
    pub fn auto_eligible(&self) -> bool {
        if self.completed_turns < 3 {
            return false;
        }
        let turns_since = self
            .completed_turn_indices
            .iter()
            .filter(|turn| self.last_recap_turn.is_none_or(|last| **turn > last))
            .count();
        turns_since >= 2
    }

    /// 刷新在途请求耗时；返回秒级显示是否变化。
    pub fn tick_pending(&mut self) -> bool {
        let Some(pending) = self.pending.as_mut() else {
            return false;
        };
        let elapsed = u64::try_from(pending.started_at.elapsed().as_millis()).unwrap_or(u64::MAX);
        let before = elapsed_seconds(pending.elapsed_ms);
        pending.elapsed_ms = elapsed;
        before != elapsed_seconds(elapsed)
    }

    /// 丢弃在途请求（聚焦恢复/新消息/切会话）。
    pub fn cancel_pending(&mut self) -> bool {
        self.pending.take().is_some()
    }
}

/// 秒级耗时取整：与 UI 的 `format_duration_seconds` 同一口径。
fn elapsed_seconds(ms: u64) -> u64 {
    ms.saturating_add(500) / 1_000
}
