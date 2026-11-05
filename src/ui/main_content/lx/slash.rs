//! lx 页 `/` 命令面板：条目、视图、几何与命中（`/` 命令 / `/model` 两级 / `/resume`）。
//!
//! 与 `@` 提及面板同款几何：与输入框等宽、底边紧贴输入框上沿、条目区滚动、
//! 顶边标题 + 底边快捷键提示、右端来源标签；`/model`、`/resume` 无候选时显示
//! 空态占位。全部文案为 ASCII，避免双宽字符撑破面板。

use ratatui::layout::Rect;

use crate::app::state::Lx;
use crate::ui::text;
use crate::ui::widgets::command_panel::{
    self, CommandItem, CommandPanelView, PanelLayout, PanelPlacement,
};

use super::mention;

/// `/` 命令面板条目（与 `render` 同源）；面板在但无候选时给一行空态占位。
///
/// 返回的 `CommandItem` 只借用 `panel` 与 `time_labels`（含 `&'static str` 占位），
/// 无自有字符串；相对时间由调用方预先算好并以切片传入，避免在条目里持有临时值。
pub(super) fn items<'a>(
    panel: &'a crate::app::state::SlashPanel,
    time_labels: &'a [String],
) -> Vec<CommandItem<'a>> {
    use crate::app::state::{ModelPickerStage, SlashPanel};
    match panel {
        SlashPanel::Commands { items, .. } => items
            .iter()
            .map(|entry| match entry {
                crate::app::state::SlashEntry::Command(command) => CommandItem::Inline {
                    label: command.text(),
                    preview: command.description(),
                    tag: command.tag(),
                },
            })
            .collect(),
        SlashPanel::Model(picker) => {
            if picker.stage_len() == 0 {
                return vec![CommandItem::Inline {
                    label: text::LX_CHAT_NO_MODEL,
                    preview: "",
                    tag: "",
                }];
            }
            if picker.is_filter_empty() {
                return vec![CommandItem::Inline {
                    label: text::LX_SLASH_NO_MATCHES,
                    preview: "",
                    tag: "",
                }];
            }
            // 按过滤视图顺序产出（`filtered` 是原始索引序列）。
            picker
                .filtered
                .iter()
                .filter_map(|index| match &picker.stage {
                    ModelPickerStage::Models => {
                        picker
                            .candidates
                            .get(*index)
                            .map(|candidate| CommandItem::Inline {
                                label: candidate.model_name.as_str(),
                                preview: candidate.provider_name.as_str(),
                                tag: "",
                            })
                    }
                    ModelPickerStage::Variants { variants, .. } => {
                        variants.get(*index).map(|variant| CommandItem::Inline {
                            label: variant.as_str(),
                            preview: "",
                            tag: "",
                        })
                    }
                })
                .collect()
        }
        SlashPanel::History(history) => {
            if history.entries.is_empty() {
                return vec![CommandItem::Inline {
                    label: text::LX_HISTORY_EMPTY,
                    preview: "",
                    tag: "",
                }];
            }
            if history.is_filter_empty() {
                return vec![CommandItem::Inline {
                    label: text::LX_SLASH_NO_MATCHES,
                    preview: "",
                    tag: "",
                }];
            }
            // 行右端是相对时间（对齐 opencode）；会话 id 不再上桌（只用于内部绑定）。
            // 按过滤视图顺序产出，`time_labels` 已是同序。
            history
                .visible()
                .iter()
                .enumerate()
                .map(|(index, entry)| CommandItem::Inline {
                    label: entry.title.as_str(),
                    preview: "",
                    tag: time_labels.get(index).map_or("", String::as_str),
                })
                .collect()
        }
        SlashPanel::Fork(panel) => panel
            .visible()
            .into_iter()
            .enumerate()
            .map(|(index, entry)| CommandItem::Inline {
                label: if entry.is_full_session() {
                    text::LX_FORK_FULL_SESSION
                } else {
                    entry.label.as_str()
                },
                preview: "",
                tag: time_labels.get(index).map_or("", String::as_str),
            })
            .collect(),
        SlashPanel::Rewind(panel) => panel
            .actions
            .iter()
            .map(|action| {
                let reason = if !panel.enabled(*action) {
                    panel
                        .snapshot
                        .unavailable_reason()
                        .unwrap_or(text::LX_REWIND_SNAPSHOT_UNAVAILABLE)
                } else if action.reverts_files()
                    && panel.snapshot == crate::app::state::SnapshotState::AlreadyRestored
                {
                    text::LX_REWIND_FILES_ALREADY_RESTORED
                } else {
                    ""
                };
                CommandItem::Inline {
                    label: action_label(*action),
                    preview: "",
                    tag: reason,
                }
            })
            .collect(),
        SlashPanel::Connect(panel) => connect_items(panel),
    }
}

/// `/connect` 各阶段的条目（全部借用面板内的稳定字符串）。
fn connect_items(panel: &crate::app::state::ConnectPanel) -> Vec<CommandItem<'_>> {
    use crate::app::state::{ConnectPromptStep, ConnectStage};
    match &panel.stage {
        ConnectStage::Providers => {
            if panel.filtered.is_empty() {
                return vec![CommandItem::Inline {
                    label: if panel.is_filter_empty() {
                        text::LX_SLASH_NO_MATCHES
                    } else {
                        text::LX_CONNECT_EMPTY
                    },
                    preview: "",
                    tag: "",
                }];
            }
            panel
                .visible()
                .into_iter()
                .map(|entry| CommandItem::Inline {
                    label: &entry.name,
                    preview: &entry.note,
                    tag: &entry.tag,
                })
                .collect()
        }
        ConnectStage::Manage { actions, .. } => actions
            .iter()
            .map(|action| CommandItem::Inline {
                label: action.label(),
                preview: "",
                tag: "",
            })
            .collect(),
        ConnectStage::Methods { methods, .. } => methods
            .iter()
            .map(|method| CommandItem::Inline {
                label: method.label(),
                preview: "",
                tag: if method.oauth_flow().is_some() {
                    text::LX_CONNECT_METHOD_OAUTH_TAG
                } else {
                    text::LX_CONNECT_METHOD_API_KEY_TAG
                },
            })
            .collect(),
        ConnectStage::InputPrompt {
            provider_name,
            step,
            error,
            ..
        } => match step {
            ConnectPromptStep::FormField { field, .. } => {
                if field.options.is_empty() {
                    vec![CommandItem::Inline {
                        label: field.label,
                        preview: error
                            .as_deref()
                            .filter(|error| !error.is_empty())
                            .unwrap_or(field.placeholder),
                        tag: text::LX_CONNECT_INPUT_TAG,
                    }]
                } else {
                    field
                        .options
                        .iter()
                        .map(|option| CommandItem::Inline {
                            label: option.label,
                            preview: option.description,
                            tag: option.value,
                        })
                        .collect()
                }
            }
            ConnectPromptStep::ApiKey => vec![CommandItem::Inline {
                label: provider_name,
                preview: text::LX_CONNECT_API_KEY_PROMPT,
                tag: error
                    .as_deref()
                    .filter(|error| !error.is_empty())
                    .unwrap_or(text::LX_CONNECT_METHOD_API_KEY_TAG),
            }],
        },
        ConnectStage::OAuthWaiting {
            url,
            user_code,
            instructions,
            error,
            ..
        } => {
            let status = if instructions.is_empty() {
                text::LX_CONNECT_OAUTH_WAITING
            } else {
                instructions.as_str()
            };
            if url.is_empty() {
                return vec![CommandItem::Inline {
                    label: status,
                    preview: error.as_deref().unwrap_or(""),
                    tag: "",
                }];
            }
            vec![
                CommandItem::Inline {
                    label: text::LX_CONNECT_OPEN_BROWSER,
                    preview: url,
                    tag: "",
                },
                CommandItem::Inline {
                    label: text::LX_CONNECT_COPY_CODE,
                    preview: user_code.as_deref().unwrap_or(url),
                    tag: "",
                },
                CommandItem::Inline {
                    label: status,
                    preview: error.as_deref().unwrap_or(""),
                    tag: "",
                },
            ]
        }
    }
}

/// 撤销动作文案（集中 `ui::text`）。
fn action_label(action: crate::app::state::RewindAction) -> &'static str {
    use crate::app::state::RewindAction;
    match action {
        RewindAction::RevertAndDelete => text::LX_REWIND_ACTION_REVERT_AND_DELETE,
        RewindAction::DeleteOnly => text::LX_REWIND_ACTION_DELETE_ONLY,
        RewindAction::RevertOnly => text::LX_REWIND_ACTION_REVERT_ONLY,
        RewindAction::Cancel => text::LX_REWIND_ACTION_CANCEL,
        RewindAction::RestoreCompact => text::LX_COMPACT_ACTION_RESTORE,
    }
}

/// 历史/分支条目的相对时间标签（与 `items` 的过滤视图同序）；其它面板为空。
pub(super) fn history_time_labels(panel: &crate::app::state::SlashPanel) -> Vec<String> {
    match panel {
        crate::app::state::SlashPanel::History(history) => history
            .visible()
            .iter()
            .map(|entry| entry.time_label(history.now_ms))
            .collect(),
        crate::app::state::SlashPanel::Fork(fork) => fork
            .visible()
            .iter()
            .map(|entry| entry.time_label(fork.now_ms))
            .collect(),
        _ => Vec::new(),
    }
}

/// 面板当前是否仅渲染不可选的空态占位行。
fn is_placeholder(panel: &crate::app::state::SlashPanel) -> bool {
    match panel {
        crate::app::state::SlashPanel::Model(p) => p.stage_len() == 0 || p.is_filter_empty(),
        crate::app::state::SlashPanel::History(h) => h.entries.is_empty() || h.is_filter_empty(),
        crate::app::state::SlashPanel::Fork(_) => false,
        crate::app::state::SlashPanel::Commands { .. } => false,
        crate::app::state::SlashPanel::Rewind(_) => false,
        crate::app::state::SlashPanel::Connect(panel) => panel.is_placeholder(),
    }
}

/// `/` 命令面板视图：与 `@` 面板同款宽度/放置/高度口径，窗口取自面板状态。
fn view<'a>(
    items: &'a [CommandItem<'a>],
    panel: &'a crate::app::state::SlashPanel,
    input: Rect,
    area: Rect,
) -> CommandPanelView<'a> {
    use crate::app::state::{ModelPickerStage, SlashPanel};
    let (title, window, footer) = match panel {
        SlashPanel::Commands { window, .. } => (
            text::LX_SLASH_PANEL_TITLE,
            *window,
            text::LX_SLASH_PANEL_FOOTER,
        ),
        SlashPanel::Model(picker) => (
            if matches!(picker.stage, ModelPickerStage::Models) {
                text::LX_MODEL_PANEL_TITLE
            } else {
                text::LX_MODEL_VARIANT_PANEL_TITLE
            },
            picker.window,
            text::LX_SLASH_PANEL_FOOTER,
        ),
        SlashPanel::History(history) => (
            text::LX_HISTORY_PANEL_TITLE,
            history.window,
            text::LX_HISTORY_PANEL_FOOTER,
        ),
        SlashPanel::Fork(panel) => (
            text::LX_FORK_PANEL_TITLE,
            panel.window,
            text::LX_FORK_PANEL_FOOTER,
        ),
        SlashPanel::Rewind(panel) => (
            text::LX_REWIND_PANEL_TITLE,
            panel.window,
            text::LX_REWIND_PANEL_FOOTER,
        ),
        SlashPanel::Connect(panel) => {
            use crate::app::state::{ConnectPromptStep, ConnectStage};
            let (title, footer) = match &panel.stage {
                ConnectStage::Providers => (
                    text::LX_CONNECT_PANEL_TITLE,
                    text::LX_CONNECT_PROVIDERS_FOOTER,
                ),
                ConnectStage::Manage { .. } => (
                    text::LX_CONNECT_MANAGE_TITLE,
                    text::LX_CONNECT_SELECT_FOOTER,
                ),
                ConnectStage::Methods { .. } => (
                    text::LX_CONNECT_METHOD_TITLE,
                    text::LX_CONNECT_SELECT_FOOTER,
                ),
                ConnectStage::InputPrompt { step, .. } => {
                    let footer = match step {
                        ConnectPromptStep::FormField { field, .. } if !field.options.is_empty() => {
                            text::LX_CONNECT_SELECT_FOOTER
                        }
                        _ => text::LX_CONNECT_INPUT_FOOTER,
                    };
                    (text::LX_CONNECT_PANEL_TITLE, footer)
                }
                ConnectStage::OAuthWaiting { mode, .. } => (
                    text::LX_CONNECT_OAUTH_TITLE,
                    if *mode == crate::config::ai::oauth::OAuthPromptMode::Code {
                        text::LX_CONNECT_OAUTH_CODE_FOOTER
                    } else {
                        text::LX_CONNECT_OAUTH_FOOTER
                    },
                ),
            };
            (title, panel.window(), footer)
        }
    };
    // 撤销面板顶边右侧固定说明恢复范围；连接面板显示当前供应商名。
    let right_title = match panel {
        SlashPanel::Rewind(_) => Some(text::LX_REWIND_SCOPE),
        SlashPanel::Connect(panel) => match &panel.stage {
            crate::app::state::ConnectStage::Manage { provider_name, .. }
            | crate::app::state::ConnectStage::Methods { provider_name, .. }
            | crate::app::state::ConnectStage::OAuthWaiting { provider_name, .. } => {
                Some(provider_name.as_str())
            }
            _ => None,
        },
        _ => None,
    };
    let above = input.y.saturating_sub(area.y);
    let cap = above.min(
        mention::MAX_POPUP_ROWS
            .saturating_add(mention::PANEL_BORDER_ROWS)
            .saturating_add(mention::PANEL_FOOTER_ROWS),
    );
    CommandPanelView {
        items,
        active: window.active,
        // 空态占位行时 current 越界，渲染侧按索引匹配不到，自然不画标记。
        current: panel.current_index(),
        window_anchor: Some(window.anchor),
        window_start: window.viewport,
        anchor_row: above,
        max_height: Some(cap),
        fixed_height: false,
        title: Some(title),
        tabs: None,
        right_title,
        footer: Some(footer),
        width: Some(input.width),
        placement: PanelPlacement::Above,
        borderless: true,
        hide_active_marker: is_placeholder(panel),
        disabled: None,
    }
}

/// `/` 命令面板整块矩形（贴输入框上沿、与输入框等宽）。
fn area_of(input: Rect, area: Rect) -> Rect {
    Rect::new(input.x, area.y, input.width, input.y.saturating_sub(area.y))
}

/// `/` 命令面板纯几何（与 `render_slash_panel` 同源）；供滚轮与命中复用。
pub(crate) fn layout(area: Rect, lx: &Lx) -> Option<PanelLayout> {
    let panel = lx.slash.as_ref()?;
    let input = super::input_rect(area, lx)?;
    let time_labels = history_time_labels(panel);
    let items = items(panel, &time_labels);
    command_panel::layout(area_of(input, area), &view(&items, panel, input, area))
}

/// `/` 命令面板命中：返回被点/悬停的条目索引；未命中返回 None。
pub(crate) fn item_at(area: Rect, lx: &Lx, column: u16, row: u16) -> Option<usize> {
    let panel = lx.slash.as_ref()?;
    // 空态占位行不可选（真无数据，或过滤后无命中）。
    if is_placeholder(panel) {
        return None;
    }
    let input = super::input_rect(area, lx)?;
    let time_labels = history_time_labels(panel);
    let items = items(panel, &time_labels);
    let layout = command_panel::layout(area_of(input, area), &view(&items, panel, input, area))?;
    command_panel::item_at(&layout, &items, column, row)
}

/// 绘制 `/` 命令面板（复用 `layout` 的矩形口径）。
pub(super) fn render(area: Rect, buf: &mut ratatui::buffer::Buffer, lx: &Lx) {
    let Some(panel) = lx.slash.as_ref() else {
        return;
    };
    let Some(input) = super::input_rect(area, lx) else {
        return;
    };
    let time_labels = history_time_labels(panel);
    let items = items(panel, &time_labels);
    command_panel::render(area_of(input, area), buf, &view(&items, panel, input, area));
}
