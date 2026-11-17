//! 输入框上方的沙箱审批命令面板：复用 `CommandPanelView`，与 `/model` 同款样式与交互。
//!
//! 纯渲染与命中：只读 `Lx`，不写状态；裁决键位与点击由 `app::update::chat::approval` 处理。

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use crate::app::state::{ApprovalOption, Lx};
use crate::chat::sandbox::{SandboxApprovalKind, SandboxApprovalRequest};
use crate::ui::text;
use crate::ui::widgets::command_panel::{
    self, CommandItem, CommandPanelView, PanelContextLine, PanelLayout, PanelPlacement,
};

use super::mention;

/// 当前是否展示审批命令面板（后台任务页优先；高于 `ask` 面板）。
pub(crate) fn visible(lx: &Lx) -> bool {
    lx.task.is_none() && !lx.approvals.is_empty()
}

/// 审批面板锚点矩形：输入框优先，挂起 `ask` 时锚定在 `ask` 面板上方。
fn anchor_rect(area: Rect, lx: &Lx) -> Option<Rect> {
    super::input_rect(area, lx).or_else(|| super::ask_rect(area, lx))
}

/// 审批面板布局（贴输入框或 ask 上沿、与底部容器等宽）；未激活或空间不足返回 None。
pub(crate) fn layout(area: Rect, lx: &Lx) -> Option<PanelLayout> {
    if !visible(lx) {
        return None;
    }
    let front = lx.approvals.front()?;
    let anchor = anchor_rect(area, lx)?;
    let options = front.options();
    let additional = front
        .request
        .additional
        .as_ref()
        .map(|item| item.summary())
        .filter(|summary| !summary.is_empty());
    let prepared = PreparedApproval::new(
        &front.request,
        additional.as_deref(),
        &options,
        lx.approvals.len(),
        anchor,
        area,
    );
    let view = prepared.view(front.window.active);
    command_panel::layout(prepared.panel_area, &view)
}

/// 审批面板条目命中：返回被点/悬停的选项索引；未打开或未命中返回 None。
pub(crate) fn item_at(area: Rect, lx: &Lx, column: u16, row: u16) -> Option<usize> {
    if !visible(lx) {
        return None;
    }
    let front = lx.approvals.front()?;
    let anchor = anchor_rect(area, lx)?;
    let options = front.options();
    let additional = front
        .request
        .additional
        .as_ref()
        .map(|item| item.summary())
        .filter(|summary| !summary.is_empty());
    let prepared = PreparedApproval::new(
        &front.request,
        additional.as_deref(),
        &options,
        lx.approvals.len(),
        anchor,
        area,
    );
    let view = prepared.view(front.window.active);
    let lay = command_panel::layout(prepared.panel_area, &view)?;
    command_panel::item_at(&lay, &prepared.items, column, row)
}

/// 绘制审批命令面板（锚定输入框或 ask 上沿、复用无边框蓝色分割线命令面板）。
pub(super) fn render(area: Rect, buf: &mut Buffer, lx: &Lx) {
    if !visible(lx) {
        return;
    }
    let Some(front) = lx.approvals.front() else {
        return;
    };
    let Some(anchor) = anchor_rect(area, lx) else {
        return;
    };
    let options = front.options();
    let additional = front
        .request
        .additional
        .as_ref()
        .map(|item| item.summary())
        .filter(|summary| !summary.is_empty());
    let prepared = PreparedApproval::new(
        &front.request,
        additional.as_deref(),
        &options,
        lx.approvals.len(),
        anchor,
        area,
    );
    let view = prepared.view(front.window.active);
    command_panel::render(prepared.panel_area, buf, &view);
}

struct PreparedApproval<'a> {
    panel_area: Rect,
    title: &'static str,
    right_title: Option<String>,
    footer: &'static str,
    context_lines: Vec<PanelContextLine<'a>>,
    items: Vec<CommandItem<'a>>,
    cap: u16,
    above: u16,
    width: u16,
}

impl<'a> PreparedApproval<'a> {
    fn new(
        request: &'a SandboxApprovalRequest,
        additional_summary: Option<&'a str>,
        options: &'a [ApprovalOption],
        queue_len: usize,
        input: Rect,
        area: Rect,
    ) -> Self {
        let title = match request.kind {
            SandboxApprovalKind::Escalation => text::LX_APPROVAL_ESCALATION,
            SandboxApprovalKind::Retry => text::LX_APPROVAL_RETRY,
            SandboxApprovalKind::Rule => text::LX_APPROVAL_RULE,
        };
        let footer = match request.kind {
            SandboxApprovalKind::Retry => text::LX_APPROVAL_FOOTER_RETRY,
            SandboxApprovalKind::Escalation | SandboxApprovalKind::Rule
                if request.can_approve_for_session() =>
            {
                text::LX_APPROVAL_FOOTER_ESCALATION
            }
            SandboxApprovalKind::Escalation | SandboxApprovalKind::Rule => {
                text::LX_APPROVAL_FOOTER_ONCE
            }
        };
        let mut context_lines = Vec::with_capacity(4);
        let subagent_source = request
            .source
            .as_deref()
            .map(str::trim)
            .filter(|source| !source.is_empty());
        if let Some(source) = subagent_source {
            context_lines.push(PanelContextLine::emphasized(
                text::LX_APPROVAL_SUBAGENT_LABEL,
                source,
            ));
        }
        let reason_label = if request.kind == SandboxApprovalKind::Retry {
            text::LX_APPROVAL_BLOCKED_LABEL
        } else {
            text::LX_APPROVAL_REASON_LABEL
        };
        let reason_value = match request.kind {
            SandboxApprovalKind::Escalation | SandboxApprovalKind::Rule => {
                request.justification.as_str()
            }
            SandboxApprovalKind::Retry => request
                .denial_excerpt
                .as_deref()
                .unwrap_or(text::LX_APPROVAL_DENIED_FALLBACK),
        };
        if !reason_value.trim().is_empty() {
            context_lines.push(PanelContextLine::new(reason_label, reason_value));
        }
        context_lines.push(PanelContextLine::new(
            text::LX_APPROVAL_COMMAND_LABEL,
            request.command.as_str(),
        ));
        if let Some(summary) = additional_summary {
            context_lines.push(PanelContextLine::new(
                text::LX_APPROVAL_REQUESTED_LABEL,
                summary,
            ));
        }

        let above = input.y.saturating_sub(area.y);
        // 极矮窗格空间不足时主动省略上下文行，保证选项仍可显示与选择；
        // 若空间仅够留出 1 行上下文，优先保留子 agent 来源行，其余让位给选项。
        let min_with_context = u16::try_from(context_lines.len())
            .unwrap_or(u16::MAX)
            .saturating_add(6);
        if above < min_with_context {
            if above >= 7 && subagent_source.is_some() {
                context_lines.truncate(1);
            } else {
                context_lines.clear();
            }
        }
        let items: Vec<CommandItem<'a>> = options
            .iter()
            .map(|option| CommandItem::Inline {
                label: option.label,
                preview: option.preview.as_str(),
                tag: "",
            })
            .collect();
        let cap = above.min(
            mention::MAX_POPUP_ROWS
                .saturating_add(mention::PANEL_BORDER_ROWS)
                .saturating_add(mention::PANEL_FOOTER_ROWS)
                .saturating_add(5),
        );
        Self {
            panel_area: Rect::new(input.x, area.y, input.width, above),
            title,
            right_title: text::approval_right_title(
                subagent_source.is_some(),
                queue_len.saturating_sub(1),
            ),
            footer,
            context_lines,
            items,
            cap,
            above,
            width: input.width,
        }
    }

    fn view(&'a self, active: usize) -> CommandPanelView<'a> {
        CommandPanelView {
            items: &self.items,
            active,
            current: None,
            window_anchor: Some(active),
            window_start: None,
            anchor_row: self.above,
            max_height: Some(self.cap),
            fixed_height: false,
            title: Some(self.title),
            tabs: None,
            right_title: self.right_title.as_deref(),
            footer: Some(self.footer),
            width: Some(self.width),
            placement: PanelPlacement::Above,
            borderless: true,
            hide_active_marker: false,
            disabled: None,
            context_lines: &self.context_lines,
        }
    }
}
