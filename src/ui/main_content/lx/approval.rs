//! 输入框上方的沙箱审批 strip：固定 3 行，宽度与输入框一致。
//!
//! 纯渲染：只读 `Lx`，不写状态；裁决键位由 `app::update::chat::approval` 处理。

use ratatui::layout::Rect;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::state::Lx;
use crate::chat::sandbox::{SandboxApprovalKind, SandboxApprovalRequest};
use crate::ui::{style, text};

use super::Grid;

/// strip 固定行数：标题/理由、命令、动作提示。
pub(crate) const HEIGHT: u16 = 3;
const INDENT: u16 = 1;

/// 当前是否展示审批 strip（后台任务页优先，ask 面板不叠加）。
pub(crate) fn visible(lx: &Lx) -> bool {
    lx.task.is_none() && lx.ask.is_none() && !lx.approvals.is_empty()
}

/// 绘制审批 strip。
pub(crate) fn draw(grid: &mut Grid, rect: Rect, lx: &Lx) {
    let Some(approval) = lx.approvals.front() else {
        return;
    };
    if rect.width == 0 || rect.height == 0 {
        return;
    }
    let x = rect.x.saturating_add(INDENT);
    let limit = rect.right().saturating_sub(INDENT);
    if x >= limit {
        return;
    }
    let width = limit - x;
    let request = &approval.request;
    let title = match request.kind {
        SandboxApprovalKind::Escalation => text::LX_APPROVAL_ESCALATION,
        SandboxApprovalKind::Retry => text::LX_APPROVAL_RETRY,
    };
    let detail = match request.kind {
        SandboxApprovalKind::Escalation => request.justification.as_str(),
        SandboxApprovalKind::Retry => request
            .denial_excerpt
            .as_deref()
            .unwrap_or("sandbox denied"),
    };
    let first = format!(
        "{title}  {}",
        folded(detail, width.saturating_sub(shown(title, width)))
    );
    grid.put_clipped(x, rect.y, &first, style::strong(), width);
    let command = folded(&request.command, width);
    grid.put_clipped(x, rect.y.saturating_add(1), &command, style::muted(), width);
    let actions = actions_text(request);
    grid.put_clipped(x, rect.y.saturating_add(2), actions, style::muted(), width);
}

fn actions_text(request: &SandboxApprovalRequest) -> &'static str {
    match request.kind {
        SandboxApprovalKind::Retry => text::LX_APPROVAL_ACTIONS_RETRY,
        SandboxApprovalKind::Escalation if request.can_approve_for_session() => {
            text::LX_APPROVAL_ACTIONS_ESCALATION
        }
        SandboxApprovalKind::Escalation => text::LX_APPROVAL_ACTIONS_ONCE,
    }
}

fn shown(text: &str, width: u16) -> u16 {
    u16::try_from(text.width()).unwrap_or(u16::MAX).min(width)
}

/// 单行化并截断到 `budget` 列；超宽以 `…` 收尾。
fn folded(input: &str, budget: u16) -> String {
    let budget = usize::from(budget);
    if budget == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut width = 0usize;
    for original in input.chars() {
        let ch = if original.is_control() || original == '\n' {
            ' '
        } else {
            original
        };
        let cell = ch.width().unwrap_or(0);
        if width + cell > budget.saturating_sub(1) {
            out.push('…');
            break;
        }
        out.push(ch);
        width += cell;
    }
    out
}
