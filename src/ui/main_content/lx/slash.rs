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
/// 面板条目；`extra_tags` 是停止/报告面板的右端标签（计数或禁用原因）。
pub(super) fn items<'a>(
    panel: &'a crate::app::state::SlashPanel,
    time_labels: &'a [String],
    extra_tags: &'a [String],
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
                crate::app::state::SlashEntry::Detail(command) => CommandItem::Inline {
                    label: command.text(),
                    preview: command.description(),
                    tag: text::SLASH_TAG_BUILTIN,
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
            if history.loading {
                return vec![CommandItem::Inline {
                    label: text::LX_HISTORY_LOADING,
                    preview: "",
                    tag: "",
                }];
            }
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
                    label: if panel.report
                        && *action == crate::app::state::RewindAction::RestoreCompact
                    {
                        text::LX_REPORT_REWIND_ACTION
                    } else {
                        action_label(*action)
                    },
                    preview: "",
                    tag: reason,
                }
            })
            .collect(),
        SlashPanel::Stop(panel) => panel
            .actions
            .iter()
            .enumerate()
            .map(|(index, action)| CommandItem::Inline {
                label: stop_action_label(*action),
                preview: "",
                tag: extra_tags.get(index).map_or("", String::as_str),
            })
            .collect(),
        SlashPanel::Report(panel) => panel
            .actions
            .iter()
            .map(|action| CommandItem::Inline {
                label: report_action_label(*action),
                preview: "",
                tag: if *action == crate::app::state::ReportAction::Resend
                    && !panel.enabled(*action)
                {
                    text::detail::LX_REPORT_NO_PREVIOUS
                } else {
                    ""
                },
            })
            .collect(),
        SlashPanel::Mcp(panel) => {
            if panel.entries.is_empty() {
                return vec![CommandItem::Inline {
                    label: text::LX_MCP_EMPTY,
                    preview: "",
                    tag: "",
                }];
            }
            if panel.is_filter_empty() {
                return vec![CommandItem::Inline {
                    label: text::LX_SLASH_NO_MATCHES,
                    preview: "",
                    tag: "",
                }];
            }
            panel
                .visible()
                .into_iter()
                .map(|entry| CommandItem::Inline {
                    label: entry.name.as_str(),
                    preview: entry.preview.as_str(),
                    tag: entry.status_tag(),
                })
                .collect()
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
        RewindAction::RestoreModel => text::LX_REWIND_ACTION_RESTORE_MODEL,
        RewindAction::DeleteRecap => text::LX_REWIND_ACTION_DELETE_RECAP,
    }
}

/// 当前面板的额外右端标签（只有停止面板用计数/禁用原因）。
fn panel_tags(panel: &crate::app::state::SlashPanel) -> Vec<String> {
    match panel {
        crate::app::state::SlashPanel::Stop(stop) => stop_tags(stop),
        _ => Vec::new(),
    }
}

/// 停止动作文案（集中 `ui::text`）。
fn stop_action_label(action: crate::app::state::StopAction) -> &'static str {
    use crate::app::state::StopAction;
    match action {
        StopAction::All => text::LX_STOP_ACTION_ALL,
        StopAction::Main => text::LX_STOP_ACTION_MAIN,
        StopAction::Subagents => text::LX_STOP_ACTION_SUBAGENTS,
        StopAction::Shells => text::LX_STOP_ACTION_SHELLS,
        StopAction::Cancel => text::LX_STOP_ACTION_CANCEL,
    }
}

/// 报告动作文案。
fn report_action_label(action: crate::app::state::ReportAction) -> &'static str {
    use crate::app::state::ReportAction;
    match action {
        ReportAction::Resend => text::detail::LX_REPORT_ACTION_RESEND,
        ReportAction::Regenerate => text::detail::LX_REPORT_ACTION_REGENERATE,
        ReportAction::Cancel => text::detail::LX_REPORT_ACTION_CANCEL,
    }
}

/// 停止面板每行的右端标签：可用时给计数，禁用时给原因。
fn stop_tags(panel: &crate::app::state::StopPanel) -> Vec<String> {
    use crate::app::state::StopAction;
    panel
        .actions
        .iter()
        .map(|action| match action {
            StopAction::Main if !panel.enabled(*action) => {
                text::LX_STOP_UNAVAILABLE_MAIN.to_string()
            }
            StopAction::Subagents if !panel.enabled(*action) => {
                text::LX_STOP_UNAVAILABLE_SUBAGENTS.to_string()
            }
            StopAction::Subagents => format!("{} running", panel.availability.subagents),
            StopAction::Shells if !panel.enabled(*action) => {
                text::LX_STOP_UNAVAILABLE_SHELLS.to_string()
            }
            StopAction::Shells => shell_count_label(panel.availability),
            _ => String::new(),
        })
        .collect()
}

/// 用户 shell 与受管后台进程的合并计数。
fn shell_count_label(availability: crate::app::state::StopAvailability) -> String {
    match (availability.shells, availability.processes) {
        (0, processes) => format!("{processes} processes"),
        (shells, 0) => format!("{shells} shells"),
        (shells, processes) => format!("{shells} shells · {processes} processes"),
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
        crate::app::state::SlashPanel::History(h) => {
            h.loading || h.entries.is_empty() || h.is_filter_empty()
        }
        crate::app::state::SlashPanel::Fork(_) => false,
        crate::app::state::SlashPanel::Commands { .. } => false,
        crate::app::state::SlashPanel::Rewind(_) => false,
        crate::app::state::SlashPanel::Stop(_) => false,
        crate::app::state::SlashPanel::Report(_) => false,
        crate::app::state::SlashPanel::Mcp(p) => p.entries.is_empty() || p.is_filter_empty(),
    }
}

/// 面板各可见行的置灰掩码（`/mcp` 的设置禁用行、停止/报告面板的不可用动作）。
fn disabled_mask(panel: &crate::app::state::SlashPanel) -> Option<Vec<bool>> {
    match panel {
        crate::app::state::SlashPanel::Mcp(mcp) if !is_placeholder(panel) => Some(
            mcp.visible()
                .into_iter()
                .map(crate::app::state::McpPanelEntry::is_settings_disabled)
                .collect(),
        ),
        crate::app::state::SlashPanel::Stop(stop) => Some(
            stop.actions
                .iter()
                .map(|action| !stop.enabled(*action))
                .collect(),
        ),
        crate::app::state::SlashPanel::Report(report) => Some(
            report
                .actions
                .iter()
                .map(|action| !report.enabled(*action))
                .collect(),
        ),
        _ => None,
    }
}

/// `/` 命令面板视图：与 `@` 面板同款宽度/放置/高度口径，窗口取自面板状态。
fn view<'a>(
    items: &'a [CommandItem<'a>],
    panel: &'a crate::app::state::SlashPanel,
    disabled: Option<&'a [bool]>,
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
        SlashPanel::Rewind(panel) => {
            if panel.scope() == crate::app::state::RewindScope::Detail {
                (
                    text::detail::LX_DETAIL_REWIND_PANEL_TITLE,
                    panel.window,
                    text::detail::LX_DETAIL_REWIND_FOOTER,
                )
            } else {
                (
                    text::LX_REWIND_PANEL_TITLE,
                    panel.window,
                    text::LX_REWIND_PANEL_FOOTER,
                )
            }
        }
        SlashPanel::Stop(panel) => (
            text::LX_STOP_PANEL_TITLE,
            panel.window,
            text::LX_STOP_PANEL_FOOTER,
        ),
        SlashPanel::Report(panel) => (
            text::detail::LX_REPORT_PANEL_TITLE,
            panel.window,
            text::detail::LX_REPORT_PANEL_FOOTER,
        ),
        SlashPanel::Mcp(panel) => (
            text::LX_MCP_PANEL_TITLE,
            panel.window,
            text::LX_MCP_PANEL_FOOTER,
        ),
    };
    // 撤销面板顶边右侧固定说明恢复范围。
    let right_title = match panel {
        SlashPanel::Stop(_) => Some(text::LX_STOP_SCOPE),
        SlashPanel::Report(panel) => Some(match panel.target {
            crate::app::state::DetailTarget::Agent(_) => text::detail::LX_REPORT_SCOPE_AGENT,
            crate::app::state::DetailTarget::Btw(_) => text::detail::LX_REPORT_SCOPE_BTW,
        }),
        SlashPanel::Rewind(panel) => Some(match panel.scope() {
            crate::app::state::RewindScope::Turn => text::LX_REWIND_SCOPE,
            crate::app::state::RewindScope::Compact => text::LX_COMPACT_SCOPE,
            crate::app::state::RewindScope::ModelSwitch => text::LX_MODEL_SWITCH_SCOPE,
            crate::app::state::RewindScope::Recap => text::LX_RECAP_SCOPE,
            crate::app::state::RewindScope::Detail => {
                if panel.report {
                    text::LX_REPORT_REWIND_SCOPE
                } else if panel
                    .actions
                    .contains(&crate::app::state::RewindAction::RestoreCompact)
                {
                    text::LX_COMPACT_SCOPE
                } else {
                    panel
                        .detail
                        .as_ref()
                        .map(|detail| match detail.kind {
                            crate::app::state::DetailRewindKind::Agent => {
                                text::detail::LX_DETAIL_REWIND_SCOPE
                            }
                            crate::app::state::DetailRewindKind::Btw => {
                                text::detail::LX_DETAIL_REWIND_SCOPE_BTW
                            }
                        })
                        .unwrap_or(text::detail::LX_DETAIL_REWIND_SCOPE)
                }
            }
        }),
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
        disabled,
    }
}

/// `/` 命令面板整块矩形（贴输入框上沿、与输入框等宽）。
fn area_of(input: Rect, area: Rect) -> Rect {
    Rect::new(input.x, area.y, input.width, input.y.saturating_sub(area.y))
}

/// `/` 命令面板纯几何（与 `render_slash_panel` 同源）；供滚轮与命中复用。
pub(crate) fn layout(area: Rect, lx: &Lx) -> Option<PanelLayout> {
    let editor = lx.active_editor();
    let panel = editor.slash.as_ref()?;
    let input = super::input_rect(area, lx)?;
    let time_labels = history_time_labels(panel);
    let extra_tags = panel_tags(panel);
    let items = items(panel, &time_labels, &extra_tags);
    let disabled = disabled_mask(panel);
    command_panel::layout(
        area_of(input, area),
        &view(&items, panel, disabled.as_deref(), input, area),
    )
}

/// `/` 命令面板命中：返回被点/悬停的条目索引；未命中返回 None。
pub(crate) fn item_at(area: Rect, lx: &Lx, column: u16, row: u16) -> Option<usize> {
    let editor = lx.active_editor();
    let panel = editor.slash.as_ref()?;
    // 空态占位行不可选（真无数据，或过滤后无命中）。
    if is_placeholder(panel) {
        return None;
    }
    let input = super::input_rect(area, lx)?;
    let time_labels = history_time_labels(panel);
    let extra_tags = panel_tags(panel);
    let items = items(panel, &time_labels, &extra_tags);
    let disabled = disabled_mask(panel);
    let layout = command_panel::layout(
        area_of(input, area),
        &view(&items, panel, disabled.as_deref(), input, area),
    )?;
    command_panel::item_at(&layout, &items, column, row)
}

/// 绘制 `/` 命令面板（复用 `layout` 的矩形口径）。
pub(super) fn render(area: Rect, buf: &mut ratatui::buffer::Buffer, lx: &Lx) {
    let editor = lx.active_editor();
    let Some(panel) = editor.slash.as_ref() else {
        return;
    };
    let Some(input) = super::input_rect(area, lx) else {
        return;
    };
    let time_labels = history_time_labels(panel);
    let extra_tags = panel_tags(panel);
    let items = items(panel, &time_labels, &extra_tags);
    let disabled = disabled_mask(panel);
    command_panel::render(
        area_of(input, area),
        buf,
        &view(&items, panel, disabled.as_deref(), input, area),
    );
}
