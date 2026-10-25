//! 右侧统一容器：外框 + 顶边框标签页 + 底边框身份标签，按 `right_tab` 分发面板体。

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Paragraph, Widget};

use crate::app::state::{AppState, RightTab, Workspace, WorkspaceIdentity};

use super::controls::{PANEL_BUTTON_MARGIN, PANEL_BUTTON_WIDTH, render_collapsed_strip};
use super::{layout, prompt, prompt_frame, style, text};

/// tab 文案左右内边距（列）。
const TAB_PAD: u16 = 1;

/// tab 文案（用户可见文案统一收敛在 `ui::text`）。
fn tab_label(tab: RightTab) -> &'static str {
    match tab {
        RightTab::Prompt => text::RIGHT_TAB_PROMPT_LABEL,
        RightTab::Todo => text::RIGHT_TAB_TODO_LABEL,
    }
}

/// 容器顶边框上的 tab 矩形：从 `╭─` 之后 1 列起排，放不下的尾部 tab 丢弃。
pub fn tab_rects(panel: Rect) -> Vec<(RightTab, Rect)> {
    let Some(row) = crate::layout::prompt_border_rect(panel) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut x = row.x.saturating_add(2);
    for tab in [RightTab::Prompt, RightTab::Todo] {
        let width = u16::try_from(tab_label(tab).chars().count())
            .unwrap_or(u16::MAX)
            .saturating_add(TAB_PAD.saturating_mul(2));
        // 必须给右边角留出 1 列，否则 tab 会盖掉 `╮`。
        if x.saturating_add(width) >= row.right() {
            break; // 尾部丢弃：宁可少一个 tab，也不画半个宽字符/压住圆角
        }
        out.push((tab, Rect::new(x, row.y, width, 1)));
        x = x.saturating_add(width).saturating_add(1); // 1 列为 tab 间的 `─`
    }
    out
}

/// 命中 tab；未命中返回 None。
pub fn tab_at(panel: Rect, column: u16, row: u16) -> Option<RightTab> {
    tab_rects(panel)
        .into_iter()
        .find_map(|(tab, rect)| rect.contains((column, row).into()).then_some(tab))
}

/// 坐标是否落在容器顶边框行（tab 之外的空白同样属于容器控件区，点击须吞掉）。
pub fn tab_row_at(panel: Rect, column: u16, row: u16) -> bool {
    crate::layout::prompt_border_rect(panel)
        .is_some_and(|row_rect| row_rect.contains((column, row).into()))
}

/// 右侧统一容器渲染入口。返回 prompt 面板聚焦时的硬件光标位置。
pub(crate) fn render(
    frame: &mut Frame<'_>,
    view: &layout::ViewLayout,
    state: &AppState,
) -> Option<(u16, u16)> {
    let area = view.prompt;
    if area.width == 0 || area.height == 0 {
        return None;
    }
    if state.prompt_collapsed {
        render_collapsed_strip(frame, area, false);
        return None;
    }
    let focused = state.prompt_focused && state.right_tab == RightTab::Prompt;
    let border = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(style::border(focused));
    let inner = border.inner(area);
    frame.render_widget(border, area);
    if inner.width == 0 || inner.height == 0 {
        return None;
    }
    paint_tabs(
        frame.buffer_mut(),
        area,
        state.right_tab,
        state.right_tab_hover,
    );
    render_bottom(frame, area, state);
    match state.right_tab {
        // Prompt 体：功能按钮行 + 标题行 + 正文/树 + 滚动条 + 硬件光标
        RightTab::Prompt => prompt_frame::render_body(frame, area, state),
        // Todo：功能按钮行留空，只画一条纯分割线；内容区由 ratatui 每帧清空后天然空白
        RightTab::Todo => {
            render_empty_body(area, frame.buffer_mut(), focused);
            None
        }
    }
}

/// 占位面板体：功能按钮行不画任何东西，只画工具栏下一行的分割线（带 `├`/`┤` 衔接）。
fn render_empty_body(panel: Rect, buf: &mut Buffer, focused: bool) {
    let Some(header) = crate::layout::prompt_header_rect(panel) else {
        return;
    };
    prompt::render_divider(panel, header, buf, focused);
}

/// 绘制顶边框 tab：激活 `selected_item`、悬停未激活 `selection`、其余 `muted`。
///
/// tab 之间那 1 列 `─` 由 Block 已经画好，这里不碰，避免把 tab 连成一片。
///
/// 每格先 `Style::reset()` 再套 token：`muted()` / `selection()` 都不带完整前景，
/// 直接叠加会继承聚焦边框的 `accent()`（Cyan 加粗），非激活 tab 看起来也在发亮。
pub fn paint_tabs(buf: &mut Buffer, panel: Rect, active: RightTab, hover: Option<RightTab>) {
    for (tab, rect) in tab_rects(panel) {
        let tab_style = if tab == active {
            style::selected_item()
        } else if hover == Some(tab) {
            style::selection()
        } else {
            style::muted()
        };
        let label = tab_label(tab);
        let text = format!(" {label} ");
        buf.set_style(rect, Style::reset());
        buf.set_stringn(rect.x, rect.y, &text, usize::from(rect.width), tab_style);
    }
}

/// 显示中 prompt 所属工作区的身份标签：与 lx 窗格顶边框同一份数据与格式，
/// 只读，忽略 `prompt_root`（子目录绑定不改变身份）。
fn prompt_identity(state: &AppState) -> Option<WorkspaceIdentity> {
    state
        .workspaces
        .get(state.prompt_workspace())
        .map(Workspace::identity)
}

/// 容器底边框：整段左对齐的身份标签 `项目 · b:分支 · wt:工作区名`（与 lx 窗格同款），
/// 右侧只保留折叠按钮；放不下时按同一降级阶梯省略。不随标签页变化。
fn render_bottom(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    let Some(identity) = prompt_identity(state) else {
        return;
    };
    let row = area.bottom().saturating_sub(1);
    let start = area.x.saturating_add(2);
    let end = area
        .right()
        .saturating_sub(PANEL_BUTTON_WIDTH + PANEL_BUTTON_MARGIN);
    if end <= start {
        return;
    }
    let Some(spans) = text::workspace_identity_spans(&identity, usize::from(end - start)) else {
        return;
    };
    let line: Vec<Span<'_>> = spans
        .iter()
        .map(|span| {
            let span_style = if span.emphasis {
                style::border_label_prefix()
            } else {
                style::border_label_value()
            };
            Span::styled(span.text.as_str(), span_style)
        })
        .collect();
    Paragraph::new(Line::from(line))
        .render(Rect::new(start, row, end - start, 1), frame.buffer_mut());
}

#[cfg(test)]
mod tests;
