//! 会话中途沙箱权限切换的纯状态：展示视图、展开状态管理。
//!
//! 只保存 UI 需要的只读数据；DB 写入与重载经 `Effect` 交给持久化层。
//! `ui/` 只读本状态，渲染不修改它。

use crate::config::sandbox::SandboxMode;
use crate::db::chat::PermissionSwitchRecord;

/// 一条沙箱权限切换提示的展示视图（独立时间线条目，模型上下文中以 `<system-update>` 注入）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionSwitchNoticeView {
    /// 切换记录 ID（展开状态以它定位）。
    pub id: String,
    /// 时间线序号（与消息、撤销提示、compact、模型切换共用）。
    pub sequence: i64,
    pub from_mode: String,
    pub to_mode: String,
    /// 注入的系统提示词文本（展开后显示）。
    pub injected_prompt: String,
    /// 创建时刻（UTC 毫秒）。
    pub created_at_ms: i64,
}

impl PermissionSwitchNoticeView {
    /// 从持久化记录构造。
    pub fn from_record(record: &PermissionSwitchRecord) -> Self {
        Self {
            id: record.id.clone(),
            sequence: record.sequence,
            from_mode: record.from_mode.clone(),
            to_mode: record.to_mode.clone(),
            injected_prompt: record.injected_prompt.clone(),
            created_at_ms: record.created_at_ms,
        }
    }

    /// 投影回持久化记录（供上下文装配统一读取）。
    pub fn to_record(&self) -> PermissionSwitchRecord {
        PermissionSwitchRecord {
            id: self.id.clone(),
            sequence: self.sequence,
            from_mode: self.from_mode.clone(),
            to_mode: self.to_mode.clone(),
            injected_prompt: self.injected_prompt.clone(),
            created_at_ms: self.created_at_ms,
        }
    }

    /// 简短展示标签。
    pub fn to_short_label(&self) -> &str {
        match SandboxMode::parse(&self.to_mode) {
            Some(mode) => mode.short_label(),
            None => self.to_mode.as_str(),
        }
    }

    /// 源模式简短展示标签。
    pub fn from_short_label(&self) -> &str {
        match SandboxMode::parse(&self.from_mode) {
            Some(mode) => mode.short_label(),
            None => self.from_mode.as_str(),
        }
    }

    /// 展开态首行前后对比摘要。
    pub fn transition_line(&self) -> String {
        format!(
            "switched {} -> {}",
            self.from_short_label(),
            self.to_short_label()
        )
    }
}

/// 单个窗格持有的权限切换状态（随会话重置）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LxPermissionSwitch {
    /// 按 `sequence` 升序排列的权限切换记录。
    pub switches: Vec<PermissionSwitchNoticeView>,
    /// 已展开的切换记录 ID 集合（默认全部折叠）。
    pub expanded: Vec<String>,
}

impl LxPermissionSwitch {
    /// 空引用（子 agent 详情视图借用）。
    pub const EMPTY: Self = Self {
        switches: Vec::new(),
        expanded: Vec::new(),
    };

    /// 是否有任何切换记录。
    pub fn is_empty(&self) -> bool {
        self.switches.is_empty()
    }

    /// 指定切换记录是否展开。
    pub fn is_expanded(&self, id: &str) -> bool {
        self.expanded.iter().any(|item| item == id)
    }

    /// 合并区间（`switches[first..=last]`）内是否有任一条记录展开。
    pub fn is_range_expanded(&self, slice: &[PermissionSwitchNoticeView]) -> bool {
        slice.iter().any(|notice| self.is_expanded(&notice.id))
    }

    /// 切换指定记录的展开状态；返回是否命中有效记录。
    pub fn toggle(&mut self, id: &str) -> bool {
        if !self.switches.iter().any(|item| item.id == id) {
            return false;
        }
        if let Some(pos) = self.expanded.iter().position(|item| item == id) {
            self.expanded.remove(pos);
        } else {
            self.expanded.push(id.to_string());
        }
        true
    }

    /// 替换切换记录列表；保留仍存在条目的展开状态。
    pub fn replace(&mut self, switches: Vec<PermissionSwitchNoticeView>) {
        self.expanded
            .retain(|id| switches.iter().any(|item| &item.id == id));
        self.switches = switches;
    }

    /// 会话切换/新建时整体清空。
    pub fn reset(&mut self) {
        self.switches.clear();
        self.expanded.clear();
    }
}
