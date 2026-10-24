//! 上下文压缩的纯状态：检查点展示视图、展开状态与执行中标记。
//!
//! 只保存 UI 需要的只读数据与一次执行的幂等键；所有 IO（摘要请求、DB 事务、重载）
//! 都经 `Effect` 交给事件层。`ui/` 只读本状态，渲染不修改它（design §6.1）。

use crate::db::chat::CompactCheckpoint;

/// 一条 compact 检查点的展示视图（独立时间线条目，不进入模型上下文）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactNoticeView {
    /// 检查点 ID（展开状态与撤销都以它定位）。
    pub id: String,
    /// 时间线序号（与消息、撤销提示共用）。
    pub sequence: i64,
    /// 实际摘要模型显示名。
    pub model_label: String,
    /// 创建时刻（UTC 毫秒）；渲染层按它格式化为 `HH:MM`。
    pub created_at_ms: i64,
    /// 覆盖范围（原始消息 sequence 闭区间）。
    pub start_sequence: i64,
    pub end_sequence: i64,
    /// 摘要正文（展开后显示）。
    pub summary: String,
    /// 是否仍生效；已撤销的条目保留在时间线上但明确标记。
    pub active: bool,
}

impl CompactNoticeView {
    /// 从持久化记录构造。
    pub fn from_record(record: &CompactCheckpoint) -> Self {
        Self {
            id: record.id.clone(),
            sequence: record.sequence,
            model_label: record.model_label.clone(),
            created_at_ms: record.created_at_ms,
            start_sequence: record.start_sequence,
            end_sequence: record.end_sequence,
            summary: record.summary_text.clone(),
            active: record.is_active(),
        }
    }

    /// 覆盖范围文案（`sequence 0–7`）；两端相同只显示一个数字。
    pub fn range_label(&self) -> String {
        if self.start_sequence == self.end_sequence {
            format!("sequence {}", self.start_sequence)
        } else {
            format!("sequence {}–{}", self.start_sequence, self.end_sequence)
        }
    }
}

/// 每个窗格的 compact 状态。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LxCompact {
    /// 检查点列表（按 sequence 升序，含已撤销）。
    pub checkpoints: Vec<CompactNoticeView>,
    /// 已展开的检查点 ID（默认折叠：只有用户显式展开才进这个集合）。
    pub expanded: Vec<String>,
    /// 正在执行的压缩（幂等键）：Some 时拒绝新的 `/compact`。
    pub running: Option<String>,
    /// 压缩开始时刻（UTC 毫秒；`running` 为 Some 时有效）。
    ///
    /// 摘要请求要跑几秒到几十秒，输入框上方那一行必须给出「正在压缩」的可见反馈：
    /// 开始时刻随 `running` 一起写入，结束后一起清除。
    pub started_at_ms: Option<i64>,
    /// 本次压缩已耗时（毫秒），由事件循环每帧刷新（渲染层不取时钟）；空闲为 None。
    pub elapsed_ms: Option<i64>,
}

impl LxCompact {
    /// 是否有条目（历史区据此决定是否画“已压缩”条目）。
    pub fn is_empty(&self) -> bool {
        self.checkpoints.is_empty()
    }

    /// 是否正在压缩。
    pub fn is_running(&self) -> bool {
        self.running.is_some()
    }

    /// 开始一次压缩：记下幂等键与开始时刻（耗时由 tick 刷新，这里先清空）。
    pub fn start(&mut self, operation_id: String, started_at_ms: i64) {
        self.running = Some(operation_id);
        self.started_at_ms = Some(started_at_ms);
        self.elapsed_ms = None;
    }

    /// 结束一次压缩（成功/空增量/失败共用）：执行态与计时一起清除。
    pub fn finish(&mut self) {
        self.running = None;
        self.started_at_ms = None;
        self.elapsed_ms = None;
    }

    /// 该检查点是否展开（默认折叠）。
    pub fn is_expanded(&self, id: &str) -> bool {
        self.expanded.iter().any(|item| item == id)
    }

    /// 切换展开状态，返回新状态。
    pub fn toggle(&mut self, id: &str) -> bool {
        if let Some(position) = self.expanded.iter().position(|item| item == id) {
            self.expanded.remove(position);
            false
        } else {
            self.expanded.push(id.to_string());
            true
        }
    }

    /// 替换检查点列表（会话重载/压缩成功后回填）；保留仍存在的展开状态。
    pub fn replace(&mut self, checkpoints: Vec<CompactNoticeView>) {
        let ids: Vec<&str> = checkpoints.iter().map(|item| item.id.as_str()).collect();
        self.expanded.retain(|id| ids.contains(&id.as_str()));
        self.checkpoints = checkpoints;
    }

    /// 会话解绑/切换：清空全部状态。
    pub fn reset(&mut self) {
        self.checkpoints.clear();
        self.expanded.clear();
        self.finish();
    }

    /// 最新仍生效的检查点（上下文起点）。
    pub fn active(&self) -> Option<&CompactNoticeView> {
        self.checkpoints.iter().rev().find(|item| item.active)
    }

    /// 回执读库失败时的本地兜底：把目标检查点标成已撤销。
    ///
    /// 撤销事务已经提交，上下文指针必须立刻回退；继续用旧摘要会让下一轮拿到
    /// 已撤销的检查点。列表内容不变，下一次会话重载会以 DB 为准补齐。
    pub fn mark_reverted(&mut self, checkpoint_id: &str) {
        if let Some(item) = self
            .checkpoints
            .iter_mut()
            .find(|item| item.id == checkpoint_id)
        {
            item.active = false;
        }
    }
}

#[cfg(test)]
mod tests;
