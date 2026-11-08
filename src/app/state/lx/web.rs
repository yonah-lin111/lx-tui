//! AI Web 双站点（DeepSeek / Gemini）、模型与思考等级配置及云端会话前缀。

use crate::config::ai::ModelSelection;
use crate::ui::text;

use super::slash::{ModelCandidate, ModelPicker, ModelPickerStage, PanelWindow};

/// AI Web 内存会话的通用 ID 前缀（不写库，仅供流事件归属判断）。
pub const AI_WEB_SESSION_PREFIX: &str = "ai-web-";

/// 映射自 DeepSeek 云端真实会话的 ID 前缀。
pub const AI_WEB_DEEPSEEK_SESSION_PREFIX: &str = "ai-web-ds-";

/// 映射自 Gemini 云端真实会话的 ID 前缀。
pub const AI_WEB_GEMINI_SESSION_PREFIX: &str = "ai-web-gm-";

/// AI Web 支持的目标站点。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AiWebSite {
    #[default]
    Deepseek,
    Gemini,
}

impl AiWebSite {
    /// 全部 AI Web 站点的显示顺序。
    pub const ALL: [AiWebSite; 2] = [AiWebSite::Deepseek, AiWebSite::Gemini];

    /// 该站点关联云端真实会话时的本地 ID 前缀。
    pub fn remote_session_prefix(self) -> &'static str {
        match self {
            Self::Deepseek => AI_WEB_DEEPSEEK_SESSION_PREFIX,
            Self::Gemini => AI_WEB_GEMINI_SESSION_PREFIX,
        }
    }

    /// 将云端真实会话 ID 包装为带站点前缀的本地会话 ID。
    pub fn format_remote_session_id(self, remote_id: &str) -> String {
        format!("{}{remote_id}", self.remote_session_prefix())
    }

    /// 从 `/model` 候选的 `provider` 标签反解站点；非 AI Web 供应商返回 `None`。
    pub fn from_label(label: &str) -> Option<Self> {
        let label = label.trim();
        Self::ALL
            .into_iter()
            .find(|site| text::ai_web_site_label(*site).eq_ignore_ascii_case(label))
    }
}

/// Gemini 网页端支持的模型档位（对齐 `https://gemini.google.com/app` 模式选择器）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum GeminiWebModel {
    FlashLite35,
    #[default]
    Flash38,
    Pro31,
    Argon4,
}

impl GeminiWebModel {
    pub const ALL: [GeminiWebModel; 4] =
        [Self::FlashLite35, Self::Flash38, Self::Pro31, Self::Argon4];

    /// 模型唯一键（用于 `ModelSelection.model` 与脚本参数 `PARAMS.model`）。
    pub fn id(self) -> &'static str {
        match self {
            Self::FlashLite35 => text::GEMINI_WEB_MODEL_FLASH_LITE,
            Self::Flash38 => text::GEMINI_WEB_MODEL_FLASH,
            Self::Pro31 => text::GEMINI_WEB_MODEL_PRO,
            Self::Argon4 => text::GEMINI_WEB_MODEL_ARGON,
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|m| m.id().eq_ignore_ascii_case(id.trim()))
    }
}

/// Gemini 网页端支持的思考等级（对齐模式选择器中的「思考等级：低 / 中 / 高」）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum GeminiThinkingLevel {
    #[default]
    Low,
    Medium,
    High,
}

impl GeminiThinkingLevel {
    pub const ALL: [GeminiThinkingLevel; 3] = [Self::Low, Self::Medium, Self::High];

    pub fn id(self) -> &'static str {
        match self {
            Self::Low => text::GEMINI_THINKING_LOW,
            Self::Medium => text::GEMINI_THINKING_MEDIUM,
            Self::High => text::GEMINI_THINKING_HIGH,
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|l| l.id().eq_ignore_ascii_case(id.trim()))
    }
}

/// DeepSeek 网页端深度思考模式。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum DeepseekThinkingMode {
    #[default]
    DeepThink,
    Off,
}

impl DeepseekThinkingMode {
    pub const ALL: [DeepseekThinkingMode; 2] = [Self::DeepThink, Self::Off];

    pub fn id(self) -> &'static str {
        match self {
            Self::DeepThink => text::DEEPSEEK_THINKING_ON,
            Self::Off => text::DEEPSEEK_THINKING_OFF,
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|m| m.id().eq_ignore_ascii_case(id.trim()))
    }
}

/// 单窗格内各 AI Web 站点的独立模型与思考等级配置（纯内存态，不落库）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AiWebConfig {
    pub gemini_model: GeminiWebModel,
    pub gemini_thinking: GeminiThinkingLevel,
    pub deepseek_thinking: DeepseekThinkingMode,
}

impl AiWebConfig {
    /// 当前站点生效的 `(model, thinking)` 参数对（供驱动脚本使用）。
    pub fn active_model_and_thinking(self, site: AiWebSite) -> (&'static str, &'static str) {
        match site {
            AiWebSite::Deepseek => (text::AI_WEB_SITE_DEEPSEEK, self.deepseek_thinking.id()),
            AiWebSite::Gemini => (self.gemini_model.id(), self.gemini_thinking.id()),
        }
    }
}

/// 会话 ID 是否属于 AI Web 内存会话。
pub fn is_ai_web_session(session_id: &str) -> bool {
    session_id.starts_with(AI_WEB_SESSION_PREFIX)
}

/// 从本地会话 ID 解析所属站点与云端真实会话 ID。
pub fn ai_web_remote_session(session_id: &str) -> Option<(AiWebSite, &str)> {
    if let Some(id) = session_id
        .strip_prefix(AI_WEB_DEEPSEEK_SESSION_PREFIX)
        .filter(|id| !id.is_empty())
    {
        return Some((AiWebSite::Deepseek, id));
    }
    if let Some(id) = session_id
        .strip_prefix(AI_WEB_GEMINI_SESSION_PREFIX)
        .filter(|id| !id.is_empty())
    {
        return Some((AiWebSite::Gemini, id));
    }
    None
}

impl ModelPicker {
    /// 为 AI Web 站点构造免本地 API Key 的第一级 `/model` 候选面板。
    pub fn for_ai_web(site: AiWebSite, config: AiWebConfig) -> Self {
        let (candidates, current) = match site {
            AiWebSite::Gemini => {
                let list: Vec<ModelCandidate> = GeminiWebModel::ALL
                    .into_iter()
                    .map(|model| ModelCandidate {
                        selection: ModelSelection {
                            provider: text::AI_WEB_SITE_GEMINI.to_string(),
                            model: model.id().to_string(),
                            variant: Some(config.gemini_thinking.id().to_string()),
                        },
                        provider_name: text::AI_WEB_SITE_GEMINI.to_string(),
                        model_name: model.id().to_string(),
                    })
                    .collect();
                let current_idx = GeminiWebModel::ALL
                    .into_iter()
                    .position(|model| model == config.gemini_model);
                (list, current_idx)
            }
            AiWebSite::Deepseek => {
                let list = vec![ModelCandidate {
                    selection: ModelSelection {
                        provider: text::AI_WEB_SITE_DEEPSEEK.to_string(),
                        model: text::AI_WEB_SITE_DEEPSEEK.to_string(),
                        variant: Some(config.deepseek_thinking.id().to_string()),
                    },
                    provider_name: text::AI_WEB_SITE_DEEPSEEK.to_string(),
                    model_name: text::AI_WEB_SITE_DEEPSEEK.to_string(),
                }];
                (list, Some(0))
            }
        };
        let active = current.unwrap_or(0);
        let len = candidates.len();
        Self {
            stage: ModelPickerStage::Models,
            candidates,
            current,
            current_variant: None,
            window: PanelWindow::new(active),
            filter: String::new(),
            filtered: (0..len).collect(),
        }
    }
}
