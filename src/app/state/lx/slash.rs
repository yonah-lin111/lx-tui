//! `/` 命令面板状态：model（两级选择）、resume（历史列表）与保留命令。
//!
//! 与 `@` 提及、粘贴参照、markdown 块面板互斥；只在行首命令位置触发。

use crate::config::ai::{AiSettings, ModelSelection};

/// 保留命令。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlashCommand {
    /// `/model`：先选模型，再选其 variants。
    Model,
    /// `/resume`：当前工作区历史列表。
    Resume,
    /// `/new`：清当前窗格会话关联并回空白草稿。
    New,
}

impl SlashCommand {
    /// 全部命令（固定顺序）。
    pub const ALL: [SlashCommand; 3] =
        [SlashCommand::Model, SlashCommand::Resume, SlashCommand::New];

    /// 命令文本（含 `/`）。
    pub fn text(self) -> &'static str {
        match self {
            Self::Model => "/model",
            Self::Resume => "/resume",
            Self::New => "/new",
        }
    }

    /// 一句话描述。
    pub fn description(self) -> &'static str {
        match self {
            Self::Model => "选择下一轮使用的模型与思考等级",
            Self::Resume => "打开当前工作区的会话历史",
            Self::New => "开始新的空白会话",
        }
    }
}

/// `/model` 面板的两级视图。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelPickerStage {
    /// 第一级：模型列表（provider / model 展示行）。
    Models,
    /// 第二级：所选模型的 variants；`variants` 为空时直接跳过（不进入本层）。
    Variants { model: ModelSelection },
}

/// `/model` 面板。
#[derive(Debug, Clone, PartialEq)]
pub struct ModelPicker {
    pub stage: ModelPickerStage,
    /// 候选：启用的 provider × 其模型。
    pub candidates: Vec<ModelSelection>,
    /// 高亮索引。
    pub active: usize,
}

impl ModelPicker {
    /// 从配置构造第一级候选（只含已启用且有模型与 Key 的 provider）。
    pub fn candidates(settings: &AiSettings) -> Vec<ModelSelection> {
        let mut out = Vec::new();
        for provider_id in &settings.enabled_providers {
            let Some(provider) = settings.providers.get(provider_id) else {
                continue;
            };
            let has_key = provider
                .options
                .api_key
                .as_deref()
                .is_some_and(|key| !key.is_empty());
            if !has_key {
                continue;
            }
            for (model_key, model) in &provider.models {
                out.push(ModelSelection {
                    provider: provider_id.clone(),
                    model: model_key.clone(),
                    variant: model.variant.clone(),
                });
            }
        }
        out
    }
}

/// `/resume` 历史列表条目。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryEntry {
    pub session_id: String,
    pub title: String,
    /// `active` / `closed`。
    pub status: String,
    pub updated_at_ms: i64,
}

/// `/resume` 历史面板。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HistoryPanel {
    pub entries: Vec<HistoryEntry>,
    pub active: usize,
}

/// `/` 命令面板（互斥浮层；同一时刻至多一个视图）。
#[derive(Debug, Clone, PartialEq)]
pub enum SlashPanel {
    /// 命令候选列表（输入 `/` 后）。
    Commands { active: usize },
    /// `/model` 两级选择。
    Model(Box<ModelPicker>),
    /// `/resume` 历史列表。
    Resume(Box<HistoryPanel>),
}

impl SlashPanel {
    /// 命令候选的过滤：按前缀匹配命令文本。
    pub fn commands_matching(query: &str) -> Vec<SlashCommand> {
        SlashCommand::ALL
            .into_iter()
            .filter(|command| command.text().starts_with(query))
            .collect()
    }
}

/// 行首命令位置判定：当前段落首个非空字符是 `/` 且光标不在围栏代码块内。
///
/// 输入 `/model` 并确认后消费整条命令，不写入草稿或 DB。
pub fn slash_trigger(input: &str, cursor: usize) -> Option<String> {
    let before = &input[..cursor];
    // 围栏代码块内不触发。
    if crate::app::prompt::list::in_fence(input, cursor) {
        return None;
    }
    // 命令面板只在行首命令位置：当前光标所在行的前缀（含换行前内容）。
    let line_start = before.rfind('\n').map_or(0, |offset| offset + 1);
    let line_prefix = &input[line_start..cursor];
    // 行首必须是 `/`（无前导文本）。
    if !line_prefix.starts_with('/') {
        return None;
    }
    Some(line_prefix.to_string())
}
