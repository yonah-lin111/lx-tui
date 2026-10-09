//! 全局按钮 tooltip 覆盖层：命中与锚点共用按钮几何；贴钮显示、末行翻上、水平夹取。

use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph};

use crate::app::state::{AppState, PaneKind};
use crate::app::tooltip::TooltipTarget;
use crate::layout::{self, PaneId};
use crate::ui::layout::ViewLayout;
use crate::ui::{self, main_content, sidebar, style, tab_bar, text};

/// 命中全局按钮 tooltip 目标；目标互不重叠，按固定顺序取首个命中。
pub fn target_at(
    view: &ViewLayout,
    state: &AppState,
    pane_rects: &[(PaneId, Rect)],
    column: u16,
    row: u16,
) -> Option<TooltipTarget> {
    targets(view, state, pane_rects)
        .into_iter()
        .find_map(|(target, area)| area.contains((column, row).into()).then_some(target))
}

/// 绘制 tooltip；必须晚于下层内容、早于 toast；浮层是模态，打开时不渲染。
pub fn render(
    frame: &mut Frame<'_>,
    screen: Rect,
    state: &AppState,
    view: &ViewLayout,
    pane_rects: &[(PaneId, Rect)],
) {
    if state.overlay.is_some() {
        return;
    }
    let Some(tooltip) = state
        .tooltip
        .as_ref()
        .filter(|tooltip| tooltip.is_visible())
    else {
        return;
    };
    let Some(anchor) = targets(view, state, pane_rects)
        .into_iter()
        .find_map(|(target, area)| (target == tooltip.target).then_some(area))
    else {
        return;
    };
    let hint = text::tooltip_hint(tooltip.target, state);
    let Some(area) = rect(screen, anchor, hint) else {
        return;
    };
    frame.render_widget(Clear, area);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(style::muted())
        .style(style::overlay_panel());
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            text::ellipsize(hint, usize::from(inner.width)),
            style::text(),
        )))
        .alignment(Alignment::Center),
        inner,
    );
}

/// 全部可提示按钮及其矩形；渲染与鼠标命中共用同一几何。
fn targets(
    view: &ViewLayout,
    state: &AppState,
    pane_rects: &[(PaneId, Rect)],
) -> Vec<(TooltipTarget, Rect)> {
    let mut targets = Vec::new();
    for button in ui::collapse_buttons(view, state.agents_collapsed) {
        let target = match button.target {
            ui::CollapseTarget::Sidebar => TooltipTarget::CollapseSidebar,
            ui::CollapseTarget::Prompt => TooltipTarget::CollapsePrompt,
            ui::CollapseTarget::Agents => TooltipTarget::CollapseAgents,
        };
        targets.push((target, button.area));
    }
    for (pane, rect) in pane_rects {
        // 仅 Terminal 窗格有按钮（对齐 `pane_toggle_button_at`；prompt 栏与占位窗格无）。
        if !state
            .pane_anywhere(*pane)
            .is_some_and(|target| target.kind == PaneKind::Terminal)
        {
            continue;
        }
        if let Some(area) = main_content::toggle_button(*rect) {
            targets.push((TooltipTarget::PaneToggle(*pane), area));
        }
    }
    if let Some(area) = sidebar::add_workspace_button(view) {
        targets.push((TooltipTarget::AddWorkspace, area));
    }
    let bar = tab_bar::layout(view, state.active_workspace(), state.tab_scroll);
    if let Some(area) = bar.add {
        targets.push((TooltipTarget::AddTab, area));
    }
    if let Some(area) = bar.scroll_left {
        targets.push((TooltipTarget::TabScrollLeft, area));
    }
    if let Some(area) = bar.scroll_right {
        targets.push((TooltipTarget::TabScrollRight, area));
    }
    if let Some(area) = ui::exit_button(view) {
        targets.push((TooltipTarget::Exit, area));
    }
    if !state.prompt_collapsed {
        for (button, target) in [
            (layout::PromptToolbarButton::Undo, TooltipTarget::PromptUndo),
            (layout::PromptToolbarButton::Redo, TooltipTarget::PromptRedo),
            (
                layout::PromptToolbarButton::SelectAll,
                TooltipTarget::PromptSelectAll,
            ),
            (layout::PromptToolbarButton::Save, TooltipTarget::PromptSave),
        ] {
            if let Some(area) = layout::prompt_toolbar_button_rect(view.prompt, button) {
                targets.push((target, area));
            }
        }
        if let Some(area) = ui::prompt_pin_button(view.prompt, state.prompt_pinned.is_some()) {
            targets.push((TooltipTarget::PromptPin, area));
        }
    }
    targets
}

/// tooltip 矩形：按钮下一行、以按钮中心水平对齐并夹取进屏幕；下方放不下时翻到按钮上方。
fn rect(screen: Rect, anchor: Rect, hint: &str) -> Option<Rect> {
    if screen.width == 0 || screen.height == 0 {
        return None;
    }
    let width = u16::try_from(hint.chars().count().saturating_add(4))
        .unwrap_or(u16::MAX)
        .min(screen.width);
    if width == 0 {
        return None;
    }
    let height = 3.min(screen.height);
    let below = anchor.y.saturating_add(1);
    let y = if below.saturating_add(height) <= screen.bottom() {
        below
    } else {
        anchor.y.saturating_sub(height).max(screen.y)
    };
    let centered = anchor
        .x
        .saturating_add(anchor.width.saturating_sub(width) / 2);
    let x = centered
        .min(screen.right().saturating_sub(width))
        .max(screen.x);
    Some(Rect::new(x, y, width, height))
}

#[cfg(test)]
mod tests;
