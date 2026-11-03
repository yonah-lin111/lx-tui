//! 全局按钮 tooltip 覆盖层：命中与锚点共用按钮几何；贴钮显示、末行翻上、水平夹取。

use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};

use crate::app::overlay::Overlay;
use crate::app::state::{AppState, PaneKind, RightTab};
use crate::app::tooltip::{BlockButtonKind, TemplateButtonKind, TooltipTarget};
use crate::layout::{self, PaneId};
use crate::ui::layout::ViewLayout;
use crate::ui::widgets::clear;
use crate::ui::{self, main_content, sidebar, style, tab_bar, text};

/// 命中全局按钮 tooltip 目标；目标互不重叠，按固定顺序取首个命中。
pub fn target_at(
    view: &ViewLayout,
    state: &AppState,
    pane_rects: &[(PaneId, Rect)],
    screen: Rect,
    column: u16,
    row: u16,
) -> Option<TooltipTarget> {
    targets(view, state, pane_rects, screen)
        .into_iter()
        .find_map(|(target, area)| area.contains((column, row).into()).then_some(target))
}

/// 绘制 tooltip；必须晚于下层内容、早于 toast；浮层是模态，打开时不渲染。
/// 返回绘制出的盒矩形，供调用方隐藏被其覆盖的硬件光标。
pub fn render(
    frame: &mut Frame<'_>,
    screen: Rect,
    state: &AppState,
    view: &ViewLayout,
    pane_rects: &[(PaneId, Rect)],
) -> Option<Rect> {
    let command_panel_open = state.prompt.mention().is_some()
        || state.prompt.slash_panel().is_some()
        || state.prompt.panel().is_some();
    // 边框按钮提示例外（引用文件夹内容弹窗）：由调用方在浮层之后渲染（覆盖浮层）。
    let border_tooltip_open = matches!(state.overlay, Some(Overlay::ReferencedFolderSearch(_)));
    if command_panel_open || (state.overlay.is_some() && !border_tooltip_open) {
        return None;
    }
    let tooltip = state
        .tooltip
        .as_ref()
        .filter(|tooltip| tooltip.is_visible())?;
    let anchor = targets(view, state, pane_rects, screen)
        .into_iter()
        .find_map(|(target, area)| (target == tooltip.target).then_some(area))?;
    let title = text::TOOLTIP_TITLE;
    let hint = text::tooltip_hint(tooltip.target, state);
    let area = rect(screen, anchor, title, hint)?;
    clear::render(frame, area);
    let block = Block::bordered().border_style(style::muted());
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return Some(area);
    }
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            text::ellipsize(hint, usize::from(inner.width)),
            style::text(),
        )))
        .alignment(Alignment::Center),
        inner,
    );
    Some(area)
}

/// 全部可提示按钮及其矩形；渲染与鼠标命中共用同一几何。
fn targets(
    view: &ViewLayout,
    state: &AppState,
    pane_rects: &[(PaneId, Rect)],
    screen: Rect,
) -> Vec<(TooltipTarget, Rect)> {
    if let Some(Overlay::ReferencedFolderSearch(search)) = &state.overlay {
        let Some(layout) = ui::overlay::referenced_search_layout(screen, search) else {
            return Vec::new();
        };
        return vec![(TooltipTarget::ReferencedSearchCopyPath, layout.copy_path)];
    }
    let mut targets = Vec::new();
    for button in ui::collapse_buttons(view, state.agents_collapsed) {
        let target = match button.target {
            ui::CollapseTarget::Sidebar => TooltipTarget::CollapseSidebar,
            ui::CollapseTarget::Prompt => TooltipTarget::CollapsePrompt,
            ui::CollapseTarget::Panes => TooltipTarget::CollapsePanes,
            ui::CollapseTarget::Agents => TooltipTarget::CollapseAgents,
        };
        targets.push((target, button.area));
    }
    if !state.prompt_collapsed
        && let Some(area) = ui::prompt_pin_button(view.prompt, state.prompt_pinned.is_some())
    {
        targets.push((TooltipTarget::PromptPin, area));
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
    if let Some(area) = ui::settings_button(view, state) {
        targets.push((TooltipTarget::SidebarSettings, area));
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
    if let Some(area) = ui::swap_button(view) {
        targets.push((TooltipTarget::SwapPromptSide, area));
    }
    if !state.prompt_collapsed && state.right_tab == RightTab::Todo {
        for button in [
            crate::app::tooltip::TodoButton::New,
            crate::app::tooltip::TodoButton::Sort,
        ] {
            if let Some(area) = ui::right_panel::todo::button_rect(view.prompt, button) {
                targets.push((TooltipTarget::TodoButton(button), area));
            }
        }
    }
    if !state.prompt_collapsed && state.right_tab == RightTab::Prompt {
        let mode = state.prompt_owner().prompt_panel.mode;
        let page_header = ui::prompt_frame::prompt_page_header(state);
        let counter = page_header
            .as_ref()
            .map(|page| text::page_counter(page.current, page.total));
        for (button, target) in [
            (layout::PromptToolbarButton::Mode, TooltipTarget::PromptMode),
            (
                layout::PromptToolbarButton::Recent,
                TooltipTarget::PromptRecent,
            ),
            (
                layout::PromptToolbarButton::Folders,
                TooltipTarget::PromptFolders,
            ),
            (
                layout::PromptToolbarButton::TempPrompt,
                TooltipTarget::PromptTempPrompt,
            ),
            (layout::PromptToolbarButton::Undo, TooltipTarget::PromptUndo),
            (layout::PromptToolbarButton::Redo, TooltipTarget::PromptRedo),
            (layout::PromptToolbarButton::Save, TooltipTarget::PromptSave),
        ] {
            if let Some(area) =
                layout::prompt_toolbar_button_rect(view.prompt, button, mode, counter.as_deref())
            {
                targets.push((target, area));
            }
        }
        if let (Some(page), Some(counter), Some(bar)) = (
            page_header.as_ref(),
            counter.as_deref(),
            layout::prompt_toolbar_rect(view.prompt),
        ) && let Some(controls) = layout::toolbar_page_controls(bar, counter, page.arrows)
        {
            for (rect, target) in [
                (controls.prev, TooltipTarget::PromptPagePrev),
                (controls.counter, TooltipTarget::PromptPageCounter),
                (controls.next, TooltipTarget::PromptPageNext),
            ] {
                if rect.width > 0 {
                    targets.push((target, rect));
                }
            }
        }
        if let Some(rect) = ui::prompt_frame::prompt_recent_close_rect(view.prompt, state) {
            targets.push((TooltipTarget::PromptRecentClose, rect));
        }
        if mode == crate::app::prompt::PromptMode::Editor {
            if let Some(rect) = ui::prompt_frame::prompt_status_rect(view.prompt, state) {
                targets.push((TooltipTarget::PromptTemplateStatus, rect));
            }
            let text_area = ui::prompt_frame::prompt_main_rect(view.prompt, state);
            for hit in ui::prompt::template_buttons(&state.prompt, text_area) {
                targets.push((
                    TooltipTarget::TemplateButton {
                        line: hit.line,
                        button: template_button_kind(hit.button),
                    },
                    hit.rect,
                ));
            }
            for hit in ui::prompt::block_buttons(&state.prompt, text_area) {
                targets.push((
                    TooltipTarget::BlockButton {
                        line: hit.line,
                        button: block_button_kind(hit.button),
                    },
                    hit.rect,
                ));
            }
        }
    }
    targets
}

/// app 侧目标枚举与 ui 按钮种类的映射。
fn template_button_kind(button: ui::prompt::TemplateBlockButton) -> TemplateButtonKind {
    match button {
        ui::prompt::TemplateBlockButton::Copy => TemplateButtonKind::Copy,
        ui::prompt::TemplateBlockButton::Clean => TemplateButtonKind::Clean,
        ui::prompt::TemplateBlockButton::Delete => TemplateButtonKind::Delete,
    }
}

/// 补充块按钮 → app 侧种类。
fn block_button_kind(button: ui::prompt::BlockButton) -> BlockButtonKind {
    match button {
        ui::prompt::BlockButton::Copy => BlockButtonKind::Copy,
        ui::prompt::BlockButton::Clean => BlockButtonKind::Clean,
        ui::prompt::BlockButton::Delete => BlockButtonKind::Delete,
    }
}

/// tooltip 矩形：按钮下一行、以按钮中心水平对齐并夹取进屏幕；下方放不下时翻到按钮上方。
/// 宽度取标题与正文较长者加边框内边距（对齐 toast 盒宽语义）。
fn rect(screen: Rect, anchor: Rect, title: &str, hint: &str) -> Option<Rect> {
    if screen.width == 0 || screen.height == 0 {
        return None;
    }
    let text = title.chars().count().max(hint.chars().count());
    let width = u16::try_from(text.saturating_add(4))
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
