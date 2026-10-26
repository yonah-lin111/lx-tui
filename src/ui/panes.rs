//! 主区窗格平铺渲染：边框、标题、视图切换按钮与 Agent 标记。

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType};
use unicode_width::UnicodeWidthStr;

use super::{main_content, style, text, widgets};
use crate::app::state::{AppState, LxEntry, Pane, PaneKind, PaneView};
use crate::layout::PaneId;

/// 主区域：BSP 平铺窗格；矩形由调用方按当前几何计算，与命中测试共用。
///
/// 返回聚焦终端窗格的光标绝对坐标：终端把 IME 预输入绘制在硬件光标处，需与仿真光标同步。
pub(crate) fn render_panes(
    frame: &mut Frame<'_>,
    pane_rects: &[(PaneId, Rect)],
    state: &AppState,
    cache: &mut crate::ui::RenderCache,
) -> Option<(u16, u16)> {
    let tab = state.active_tab();
    let focus = tab.layout.focus();
    let mut cursor = None;
    for (id, rect) in pane_rects {
        if rect.width == 0 || rect.height == 0 {
            continue;
        }
        let Some(pane) = tab.pane(*id) else {
            continue;
        };
        // prompt 持有键盘焦点时窗格让出焦点表现，避免双焦点指示。
        let focused = *id == focus && state.panes_focused();
        let mut title = match pane.view {
            PaneView::Lx => format!("lx-[{}]", text::lx_entry_label(pane.lx.entry)),
            PaneView::Terminal => pane_display_title(*id, pane),
        };
        if pane.exited {
            title.push_str(" (exited)");
        }
        // 边框标题统一淡蓝色（Cyan + dim）；焦点只由边框颜色区分，按钮除外。
        // lx 页入口名按 logo 粉→蓝渐变着色，其余保持标题色。
        let title_line = if pane.view == PaneView::Lx && !pane.exited {
            lx_title_line(pane.lx.entry)
        } else {
            Line::from(Span::styled(format!(" {title} "), style::border_title()))
        };
        let block = Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(style::border(focused))
            .title(title_line);
        let inner = block.inner(*rect);
        frame.render_widget(block, *rect);
        // 顶边框右侧标签：终端视图显示 Agent 标记，lx 视图显示工作区身份标签（不并存）。
        match (pane.kind, pane.view) {
            (PaneKind::Terminal, PaneView::Lx) => {
                render_pane_identity_label(frame, *rect, &title, state, *id);
            }
            (PaneKind::Terminal, PaneView::Terminal) => {
                render_pane_agent_label(frame, *rect, &title, pane);
            }
            (PaneKind::Placeholder, _) => {}
        }
        if inner.width == 0 || inner.height == 0 {
            continue;
        }
        match pane.kind {
            PaneKind::Terminal => {
                if let Some((row, col)) = main_content::render(
                    inner,
                    frame.buffer_mut(),
                    pane,
                    focused,
                    state
                        .lx_selection
                        .as_ref()
                        .filter(|selection| selection.pane() == *id),
                    cache.pane(*id),
                ) {
                    cursor = Some((inner.x + col, inner.y + row));
                }
                if let Some(scrollbar) = main_content::scrollbar(inner, pane) {
                    widgets::scrollbar::render(frame, &scrollbar);
                }
                main_content::draw_toggle_button(frame.buffer_mut(), *rect, pane.view);
            }
            PaneKind::Placeholder => {}
        }
    }
    cursor
}

/// 窗格顶边框右侧 Agent 标记 ` agent:xxx`：前缀强调色、值恒为默认灰
/// （不随窗格焦点变色）；右端与切换按钮间隔 1 列；按钮缺失、空间不足或会覆盖左侧标题时不绘制。
fn render_pane_agent_label(frame: &mut Frame<'_>, rect: Rect, title: &str, pane: &Pane) {
    let Some(agent) = pane.agent.as_ref() else {
        return;
    };
    let spans = [
        (" ".to_string(), style::muted()),
        (
            text::PANE_AGENT_PREFIX.to_string(),
            style::border_label_prefix(),
        ),
        (
            text::agent_label(agent.kind).to_string(),
            style::border_label_value(),
        ),
    ];
    render_border_right_label(frame, rect, title, &spans);
}

/// lx 窗格顶边框标题：` lx-[<入口名>] `，入口名按该 logo 粉→蓝渐变着色、其余用标题色。
fn lx_title_line(entry: LxEntry) -> Line<'static> {
    let label = text::lx_entry_label(entry);
    let length = label.chars().count();
    let mut spans = vec![Span::styled(" lx-[".to_string(), style::border_title())];
    for (position, ch) in label.chars().enumerate() {
        spans.push(Span::styled(
            ch.to_string(),
            style::lx_entry_label_style(entry, position, length, false),
        ));
    }
    spans.push(Span::styled("] ".to_string(), style::border_title()));
    Line::from(spans)
}

/// lx 窗格顶边框右侧身份标签 `项目 · b:分支 · wt:工作区名`（切换按钮左侧）。
///
/// 项目名与 `b:`/`wt:` 前缀取强调色、值恒为默认灰；右端与切换按钮之间留 1 列，
/// 左界不越过左侧标题。放不下时按 [`text::workspace_identity_spans`] 的阶梯降级。
fn render_pane_identity_label(
    frame: &mut Frame<'_>,
    rect: Rect,
    title: &str,
    state: &AppState,
    id: PaneId,
) {
    let Some(workspace) = state.workspace_for_pane(id) else {
        return;
    };
    let identity = workspace.identity();
    let Some(button) = main_content::toggle_button(rect) else {
        return;
    };
    let Some(anchor) = button.x.checked_sub(2) else {
        return;
    };
    // 两侧各留 1 列空档：右端贴 anchor 左侧，左界不越过标题后的 1 列间隔。
    let title_width = u16::try_from(title.width())
        .unwrap_or(u16::MAX)
        .saturating_add(2);
    let left = rect.x.saturating_add(title_width).saturating_add(1);
    if anchor <= left {
        return;
    }
    let available = usize::from(anchor.saturating_sub(left)).saturating_sub(1);
    let Some(spans) = text::workspace_identity_spans(&identity, available) else {
        return;
    };
    let width: u16 = u16::try_from(spans.iter().map(|span| span.text.width()).sum::<usize>())
        .unwrap_or(u16::MAX);
    let Some(start) = anchor.checked_sub(width) else {
        return;
    };
    let buf = frame.buffer_mut();
    let mut x = start;
    for span in &spans {
        let span_style = if span.emphasis {
            style::border_label_prefix()
        } else {
            style::border_label_value()
        };
        for symbol in span.text.chars() {
            if let Some(cell) = buf.cell_mut((x, rect.y)) {
                cell.reset();
                cell.set_char(symbol);
                cell.set_style(span_style);
            }
            x = x.saturating_add(1);
        }
    }
}

/// 在窗格顶边框右侧、切换按钮左侧绘制一段带样式文本；空间不足或会覆盖左侧标题时省略。
fn render_border_right_label(
    frame: &mut Frame<'_>,
    rect: Rect,
    title: &str,
    spans: &[(String, Style)],
) {
    let Some(button) = main_content::toggle_button(rect) else {
        return;
    };
    let label_width: u16 = spans
        .iter()
        .map(|(content, _)| content.chars().count() as u16)
        .sum();
    let Some(anchor) = button.x.checked_sub(2) else {
        return;
    };
    let Some(start) = anchor
        .checked_add(1)
        .and_then(|end| end.checked_sub(label_width))
    else {
        return;
    };
    // " {title} " 与标记之间至少留 1 列，避免覆盖左侧标题。
    let title_width = u16::try_from(title.width())
        .unwrap_or(u16::MAX)
        .saturating_add(2);
    if start < rect.x.saturating_add(title_width).saturating_add(1) {
        return;
    }
    let buf = frame.buffer_mut();
    let mut x = start;
    for (content, span_style) in spans {
        for symbol in content.chars() {
            if let Some(cell) = buf.cell_mut((x, rect.y)) {
                cell.reset();
                cell.set_char(symbol);
                cell.set_style(*span_style);
            }
            x = x.saturating_add(1);
        }
    }
}

/// 窗格标题：空占位与终端走通用标题规则。
fn pane_display_title(id: PaneId, pane: &Pane) -> String {
    text::pane_title(id, pane.terminal.title(), pane.cwd_label.as_deref())
}
