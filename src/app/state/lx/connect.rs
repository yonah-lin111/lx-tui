//! `/connect` 面板状态：预设供应商列表、认证方式选择、表单/API Key 输入与 OAuth 等待。
//!
//! 纯状态与纯函数：磁盘读写、环境变量探测结果由 `app/state.rs` 的快照提供，
//! 网络与剪贴板动作以 `Effect` 交给事件层执行。

use std::collections::BTreeMap;

use crate::config::ai::connect::{ConnectSettings, ConnectionSource};
use crate::config::ai::oauth::OAuthPromptMode;
use crate::config::ai::presets::{PresetAuthMethod, PresetFormField, all_presets};
use crate::ui::text;

use super::slash::{PanelWindow, best_score};

/// `/connect` 面板中的单个预设供应商条目（已按 A-Z 排序）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectProviderEntry {
    pub id: String,
    pub name: String,
    pub note: String,
    pub source: Option<ConnectionSource>,
    pub methods: &'static [PresetAuthMethod],
    /// 右侧连接状态标签（`✓ API key` / `✓ OAuth` / `✓ $ENV`），未连接为空串。
    ///
    /// 预先算好是为了让 `ui` 的 `CommandItem` 能直接借用，避免渲染层持有临时字符串。
    pub tag: String,
}

/// 已连接供应商的管理动作。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectManageAction {
    SelectModel,
    Reconnect,
    Disconnect,
}

impl ConnectManageAction {
    /// 展示文案。
    pub fn label(self) -> &'static str {
        match self {
            Self::SelectModel => text::LX_CONNECT_ACTION_SELECT_MODEL,
            Self::Reconnect => text::LX_CONNECT_ACTION_RECONNECT,
            Self::Disconnect => text::LX_CONNECT_ACTION_DISCONNECT,
        }
    }
}

/// 表单/API Key 输入子阶段。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectPromptStep {
    /// 正在填写第 `field_index` 个前置表单字段。
    FormField {
        field_index: usize,
        field: PresetFormField,
        option_window: PanelWindow,
    },
    /// 正在通过底部输入框输入 API Key。
    ApiKey,
}

/// `/connect` 面板所处阶段。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectStage {
    /// 阶段 1：A-Z 预设供应商列表（底部输入框实时过滤）。
    Providers,
    /// 阶段 1b：已连接供应商的管理菜单。
    Manage {
        provider_id: String,
        provider_name: String,
        source: ConnectionSource,
        actions: Vec<ConnectManageAction>,
        window: PanelWindow,
    },
    /// 阶段 2：多种认证方式选择（OAuth 在前，API Key 在后）。
    Methods {
        provider_id: String,
        provider_name: String,
        methods: &'static [PresetAuthMethod],
        window: PanelWindow,
    },
    /// 阶段 3a：前置表单字段或 API Key 输入。
    InputPrompt {
        provider_id: String,
        provider_name: String,
        method_index: usize,
        step: ConnectPromptStep,
        answers: BTreeMap<String, String>,
        error: Option<String>,
    },
    /// 阶段 3b：等待 OAuth 浏览器/设备码授权完成。
    OAuthWaiting {
        attempt_id: String,
        provider_id: String,
        provider_name: String,
        method_id: String,
        form_answers: BTreeMap<String, String>,
        /// 授权链接；启动请求返回前为空串。
        url: String,
        user_code: Option<String>,
        instructions: String,
        mode: OAuthPromptMode,
        error: Option<String>,
        window: PanelWindow,
    },
}

/// `/connect` 命令面板完整状态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectPanel {
    pub entries: Vec<ConnectProviderEntry>,
    pub filtered: Vec<usize>,
    pub filter: String,
    pub window: PanelWindow,
    pub stage: ConnectStage,
}

impl ConnectPanel {
    /// 从磁盘快照构造；环境变量探测使用当前进程。
    pub fn new(connect: &ConnectSettings) -> Self {
        Self::new_with_env(connect, |name| std::env::var(name).ok())
    }

    /// 可注入环境变量查询的构造器（测试用，避免宿主环境影响结果）。
    pub fn new_with_env<F>(connect: &ConnectSettings, env: F) -> Self
    where
        F: Fn(&str) -> Option<String>,
    {
        let entries = all_presets()
            .iter()
            .map(|spec| {
                let source = crate::config::ai::connect::resolve_preset_connection_source_with_env(
                    spec,
                    connect.providers.get(spec.id),
                    &env,
                );
                let tag = source
                    .as_ref()
                    .map(|source| format!("✓ {}", source.summary_label()))
                    .unwrap_or_default();
                ConnectProviderEntry {
                    id: spec.id.to_string(),
                    name: spec.name.to_string(),
                    note: spec.note.to_string(),
                    source,
                    methods: spec.methods,
                    tag,
                }
            })
            .collect::<Vec<_>>();
        let filtered = (0..entries.len()).collect();
        Self {
            entries,
            filtered,
            filter: String::new(),
            window: PanelWindow::default(),
            stage: ConnectStage::Providers,
        }
    }

    /// 按 `query` 重算供应商过滤视图：匹配名称或 id，同分保持 A-Z 原序。
    pub fn refilter(&mut self, query: &str) {
        if self.filter != query {
            self.filter = query.to_string();
            self.window = PanelWindow::default();
        }
        let query = query.trim().to_lowercase();
        if query.is_empty() {
            self.filtered = (0..self.entries.len()).collect();
            self.clamp_window();
            return;
        }
        let mut scored: Vec<(u32, usize)> = self
            .entries
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| {
                best_score(&[&entry.name, &entry.id], &query).map(|score| (score, index))
            })
            .collect();
        scored.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.cmp(&right.1)));
        self.filtered = scored.into_iter().map(|(_, index)| index).collect();
        self.clamp_window();
    }

    /// 过滤后可见的供应商（视图顺序）。
    pub fn visible(&self) -> Vec<&ConnectProviderEntry> {
        self.filtered
            .iter()
            .filter_map(|index| self.entries.get(*index))
            .collect()
    }

    /// 当前高亮的供应商。
    pub fn active_entry(&self) -> Option<&ConnectProviderEntry> {
        self.filtered
            .get(self.window.active)
            .and_then(|index| self.entries.get(*index))
    }

    /// 高亮项在 `entries` 中的真实索引。
    pub fn active_entry_index(&self) -> Option<usize> {
        self.filtered.get(self.window.active).copied()
    }

    /// 可见项数变化后把窗口三索引钳进新范围。
    pub fn clamp_window(&mut self) {
        self.window.clamp(self.filtered.len());
    }

    /// 过滤为空（有过滤词但无命中）。
    pub fn is_filter_empty(&self) -> bool {
        !self.filter.trim().is_empty() && self.filtered.is_empty()
    }

    /// 当前阶段是否只有不可选占位（供应商列表无命中）。
    pub fn is_placeholder(&self) -> bool {
        matches!(self.stage, ConnectStage::Providers) && self.filtered.is_empty()
    }

    /// 当前阶段是否允许在底部输入框打字。
    pub fn accepts_text_input(&self) -> bool {
        match &self.stage {
            ConnectStage::Providers => true,
            ConnectStage::InputPrompt { step, .. } => match step {
                ConnectPromptStep::ApiKey => true,
                ConnectPromptStep::FormField { field, .. } => field.options.is_empty(),
            },
            ConnectStage::OAuthWaiting { mode, .. } => *mode == OAuthPromptMode::Code,
            _ => false,
        }
    }

    /// OAuth 等待阶段的 `o` / `c` 快捷键（仅自动模式）。
    pub fn shortcut_char(&self, ch: char) -> bool {
        matches!(
            self.stage,
            ConnectStage::OAuthWaiting {
                mode: OAuthPromptMode::Auto,
                ..
            }
        ) && matches!(ch, 'o' | 'c')
    }

    /// 当前阶段的可选行数（渲染与导航共用）。
    pub fn item_count(&self) -> usize {
        match &self.stage {
            ConnectStage::Providers => self.filtered.len(),
            ConnectStage::Manage { actions, .. } => actions.len(),
            ConnectStage::Methods { methods, .. } => methods.len(),
            ConnectStage::InputPrompt { step, .. } => match step {
                ConnectPromptStep::FormField { field, .. } => field.options.len(),
                ConnectPromptStep::ApiKey => 0,
            },
            ConnectStage::OAuthWaiting { url, .. } => {
                if url.is_empty() {
                    1
                } else {
                    3
                }
            }
        }
    }

    /// 当前视图的窗口状态。
    pub fn window(&self) -> PanelWindow {
        match &self.stage {
            ConnectStage::Providers => self.window,
            ConnectStage::Manage { window, .. }
            | ConnectStage::Methods { window, .. }
            | ConnectStage::OAuthWaiting { window, .. } => *window,
            // 表单/ApiKey 不维护独立窗口；选项窗口在 step 内。
            ConnectStage::InputPrompt {
                step: ConnectPromptStep::FormField { option_window, .. },
                ..
            } => *option_window,
            ConnectStage::InputPrompt { .. } => PanelWindow::default(),
        }
    }

    /// 写回窗口状态。
    pub fn set_window(&mut self, window: PanelWindow) {
        match &mut self.stage {
            ConnectStage::Providers => self.window = window,
            ConnectStage::Manage {
                window: current, ..
            }
            | ConnectStage::Methods {
                window: current, ..
            }
            | ConnectStage::OAuthWaiting {
                window: current, ..
            } => *current = window,
            ConnectStage::InputPrompt {
                step: ConnectPromptStep::FormField { option_window, .. },
                ..
            } => *option_window = window,
            ConnectStage::InputPrompt { .. } => {}
        }
    }

    /// 键盘移动高亮；返回是否变化。
    pub fn move_active(&mut self, delta: isize) -> bool {
        let len = self.item_count();
        if len == 0 {
            return false;
        }
        let mut window = self.window();
        window.move_active(delta, len);
        self.set_window(window);
        true
    }
}
