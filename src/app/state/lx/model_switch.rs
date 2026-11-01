//! 会话中途模型切换的纯状态：展示视图、展开状态管理。
//!
//! 只保存 UI 需要的只读数据；DB 写入与重载经 `Effect` 交给持久化层。
//! `ui/` 只读本状态，渲染不修改它。

use crate::db::chat::ModelSwitchRecord;

/// 一条模型切换提示的展示视图（独立时间线条目，模型上下文中以 `<system-update>` 注入）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelSwitchNoticeView {
    /// 切换记录 ID（展开状态以它定位）。
    pub id: String,
    /// 时间线序号（与消息、撤销提示、compact 检查点共用）。
    pub sequence: i64,
    pub from_provider: Option<String>,
    pub from_model: Option<String>,
    pub from_variant: Option<String>,
    pub to_provider: String,
    pub to_model: String,
    pub to_variant: Option<String>,
    /// 切换前模型展示名（如 `Claude Sonnet 4 (high)`）。
    pub from_label: Option<String>,
    /// 切换后模型展示名（不含 variant，如 `DeepSeek V4.1 Flash`）。
    pub to_label: String,
    /// 本次更新涉及的指令源标签列表（如 `["core/model", "core/agents_md"]`）。
    pub updated_sources: Vec<String>,
    /// 注入的系统提示词文本（展开后显示）。
    pub injected_prompt: String,
    /// 创建时刻（UTC 毫秒）。
    pub created_at_ms: i64,
}

impl ModelSwitchNoticeView {
    /// 从持久化记录构造。
    pub fn from_record(record: &ModelSwitchRecord) -> Self {
        Self {
            id: record.id.clone(),
            sequence: record.sequence,
            from_provider: record.from_provider.clone(),
            from_model: record.from_model.clone(),
            from_variant: record.from_variant.clone(),
            to_provider: record.to_provider.clone(),
            to_model: record.to_model.clone(),
            to_variant: record.to_variant.clone(),
            from_label: record.from_label.clone(),
            to_label: record.to_label.clone(),
            updated_sources: record.updated_sources.clone(),
            injected_prompt: record.injected_prompt.clone(),
            created_at_ms: record.created_at_ms,
        }
    }

    /// 投影回持久化记录（供上下文装配统一读取）。
    pub fn to_record(&self) -> ModelSwitchRecord {
        ModelSwitchRecord {
            id: self.id.clone(),
            sequence: self.sequence,
            from_provider: self.from_provider.clone(),
            from_model: self.from_model.clone(),
            from_variant: self.from_variant.clone(),
            to_provider: self.to_provider.clone(),
            to_model: self.to_model.clone(),
            to_variant: self.to_variant.clone(),
            from_label: self.from_label.clone(),
            to_label: self.to_label.clone(),
            updated_sources: self.updated_sources.clone(),
            injected_prompt: self.injected_prompt.clone(),
            created_at_ms: self.created_at_ms,
        }
    }

    /// 是否仅切换了同一模型的思考等级（provider 与 model 未变）。
    pub fn is_variant_only(&self) -> bool {
        self.from_provider.as_deref() == Some(self.to_provider.as_str())
            && self.from_model.as_deref() == Some(self.to_model.as_str())
    }

    /// 目标模型带思考等级后缀的展示名（如 `DeepSeek V4.1 Flash (max)`）。
    pub fn target_display_with_variant(&self) -> String {
        match self.to_variant.as_deref().filter(|v| !v.is_empty()) {
            Some(variant) => format!("{} ({variant})", self.to_label),
            None => self.to_label.clone(),
        }
    }

    /// 展开态首行前后对比摘要。
    pub fn transition_line(&self, from_override: Option<&str>) -> String {
        let from = from_override
            .or(self.from_label.as_deref())
            .filter(|label| !label.trim().is_empty());
        let to = self.target_display_with_variant();
        match from {
            Some(prev) => format!("switched {prev} -> {to}"),
            None => format!("switched -> {to}"),
        }
    }
}

/// 单个窗格持有的模型切换状态（随会话重置）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LxModelSwitch {
    /// 按 `sequence` 升序排列的模型切换记录。
    pub switches: Vec<ModelSwitchNoticeView>,
    /// 已展开的切换记录 ID 集合（默认全部折叠）。
    pub expanded: Vec<String>,
}

impl LxModelSwitch {
    /// 空引用（子 agent 视图无独立模型切换时间线时借用）。
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
    pub fn replace(&mut self, switches: Vec<ModelSwitchNoticeView>) {
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
