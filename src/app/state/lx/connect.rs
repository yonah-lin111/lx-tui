//! `/connect` 面板状态：预设供应商列表、认证方式选择、表单/API Key 输入与 OAuth 等待。
//!
//! 纯状态与纯函数：磁盘读写、环境变量探测结果由 `app/state.rs` 的快照提供，
//! 网络与剪贴板动作以 `Effect` 交给事件层执行。

use std::collections::BTreeMap;

use crate::app::overlay::TextInput;
use crate::config::ai::connect::{ConnectSettings, ConnectionSource};
use crate::config::ai::oauth::OAuthPromptMode;
use crate::config::ai::presets::{PresetAuthMethod, PresetFormField, all_presets};
use crate::layout::PaneId;
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
    RefreshModels,
    Disconnect,
}

impl ConnectManageAction {
    /// 展示文案。
    pub fn label(self) -> &'static str {
        match self {
            Self::SelectModel => text::LX_CONNECT_ACTION_SELECT_MODEL,
            Self::Reconnect => text::LX_CONNECT_ACTION_RECONNECT,
            Self::RefreshModels => text::LX_CONNECT_ACTION_REFRESH_MODELS,
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

/// `/connect` 居中模态的完整状态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectPanel {
    /// 触发 `/connect` 的 `lx` 窗格；回执与 Toast 都按它路由。
    pub pane: PaneId,
    /// 模态内置单行输入框：`Providers` 阶段是过滤词，输入阶段是凭证/表单值。
    pub input: TextInput,
    /// 后台校验/刷新中（阻止重复提交与粘贴）。
    pub validating: bool,
    /// 校验/刷新状态行文案。
    pub status_message: Option<String>,
    /// 指针悬停的可见行索引；只铺底色，不移动 `❯`。
    pub hover: Option<usize>,
    pub entries: Vec<ConnectProviderEntry>,
    pub filtered: Vec<usize>,
    pub filter: String,
    pub window: PanelWindow,
    pub stage: ConnectStage,
}

impl ConnectPanel {
    /// 从磁盘快照构造；环境变量探测使用当前进程。
    pub fn new(pane: PaneId, connect: &ConnectSettings) -> Self {
        Self::new_with_env(pane, connect, |name| std::env::var(name).ok())
    }

    /// 可注入环境变量查询的构造器（测试用，避免宿主环境影响结果）。
    pub fn new_with_env<F>(pane: PaneId, connect: &ConnectSettings, env: F) -> Self
    where
        F: Fn(&str) -> Option<String>,
    {
        let entries = Self::build_entries(connect, &env);
        let filtered = (0..entries.len()).collect();
        Self {
            pane,
            input: TextInput::new(""),
            validating: false,
            status_message: None,
            hover: None,
            entries,
            filtered,
            filter: String::new(),
            window: PanelWindow::default(),
            stage: ConnectStage::Providers,
        }
    }

    /// 按连接状态重建条目（模型刷新/断开后同步标签）。
    fn build_entries<F>(connect: &ConnectSettings, env: &F) -> Vec<ConnectProviderEntry>
    where
        F: Fn(&str) -> Option<String>,
    {
        all_presets()
            .iter()
            .map(|spec| {
                let source = crate::config::ai::connect::resolve_preset_connection_source_with_env(
                    spec,
                    connect.providers.get(spec.id),
                    env,
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
            .collect()
    }

    /// 用新的连接快照刷新条目与标签。
    pub fn refresh_entries(&mut self, connect: &ConnectSettings) {
        self.entries = Self::build_entries(connect, &|name| std::env::var(name).ok());
        if matches!(self.stage, ConnectStage::Providers) {
            self.refilter_from_input();
        } else {
            self.filtered = (0..self.entries.len()).collect();
        }
        self.clamp_window();
    }

    /// 按模态输入框当前文本重算供应商过滤视图。
    pub fn refilter_from_input(&mut self) {
        let query = self.input.text().to_string();
        self.refilter(&query);
    }

    /// 退回供应商列表并清空临时输入/窗口。
    pub fn reset_to_providers(&mut self) {
        self.stage = ConnectStage::Providers;
        self.input = TextInput::new("");
        self.filter.clear();
        self.filtered = (0..self.entries.len()).collect();
        self.window = PanelWindow::default();
        self.hover = None;
        self.validating = false;
        self.status_message = None;
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

    /// 过滤视图位置对应的供应商（渲染/命中共用，O(1)）。
    pub fn entry_at_view(&self, index: usize) -> Option<&ConnectProviderEntry> {
        self.filtered
            .get(index)
            .and_then(|index| self.entries.get(*index))
    }

    /// 当前高亮的供应商。
    pub fn active_entry(&self) -> Option<&ConnectProviderEntry> {
        self.entry_at_view(self.window.active)
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

    /// 当前阶段是否使用模态内置输入框。
    pub fn has_text_input(&self) -> bool {
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

    /// 当前阶段的错误文案。
    pub fn current_error(&self) -> Option<&str> {
        match &self.stage {
            ConnectStage::InputPrompt { error, .. } | ConnectStage::OAuthWaiting { error, .. } => {
                error.as_deref().filter(|error| !error.is_empty())
            }
            _ => None,
        }
    }

    /// 在当前输入/OAuth 阶段展示错误。
    pub fn set_error(&mut self, message: impl Into<String>) {
        let message = message.into();
        match &mut self.stage {
            ConnectStage::InputPrompt { error, .. } | ConnectStage::OAuthWaiting { error, .. } => {
                *error = Some(message);
            }
            _ => {}
        }
    }

    /// 清除当前阶段的错误。
    pub fn clear_error(&mut self) {
        match &mut self.stage {
            ConnectStage::InputPrompt { error, .. } | ConnectStage::OAuthWaiting { error, .. } => {
                *error = None;
            }
            _ => {}
        }
    }

    /// 当前阶段对应的供应商名（供应商列表阶段为空）。
    pub fn provider_name(&self) -> Option<&str> {
        match &self.stage {
            ConnectStage::Providers => None,
            ConnectStage::Manage { provider_name, .. }
            | ConnectStage::Methods { provider_name, .. }
            | ConnectStage::InputPrompt { provider_name, .. }
            | ConnectStage::OAuthWaiting { provider_name, .. } => Some(provider_name),
        }
    }

    /// 进行中的 OAuth 尝试 id（无则 None）。
    pub fn oauth_attempt_id(&self) -> Option<&str> {
        match &self.stage {
            ConnectStage::OAuthWaiting { attempt_id, .. } if !attempt_id.is_empty() => {
                Some(attempt_id)
            }
            _ => None,
        }
    }

    /// 已连接的供应商数量（顶栏右侧摘要）。
    pub fn connected_count(&self) -> usize {
        self.entries
            .iter()
            .filter(|entry| entry.source.is_some())
            .count()
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
