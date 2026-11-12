//! `BTW` 页签：上方对话（复用历史渲染）+ 下方独立输入框。
//!
//! 只读状态、只写 Buffer；尺寸/可见行由事件循环在绘制前写入状态。
//!
//! 底部固定两段：
//! - tab 行与输入框之间留一行：生成中画 loading，空闲时为空行；
//! - 输入框默认一行（上下边框 + 一行正文），随软换行增长到 3 行；
//!   `QA n · <主问题>` + `To: <回答者>` 画在输入框顶边框右上角。

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use crate::app::state::{Lx, TaskPage};
use crate::ui::main_content::lx::history::{self, HistorySource, HistoryViewport};
use crate::ui::widgets::command_panel::{
    self, CommandItem, CommandPanelView, PanelLayout, PanelPlacement,
};
use crate::ui::{style, text};

use super::super::Grid;
use super::list_rect;

/// 输入框最多展示的正文行数（超出后在框内滚动）。
pub(super) const COMPOSER_MAX_ROWS: u16 = 3;
/// 归属胶囊最大宽度（含左右空格与右侧留白）；超出按标题截断。
const QA_BADGE_MAX_COLS: u16 = 48;

/// 顶边框 + tab 行已由 `draw_list` 绘制；这里排空行（loading）与输入框。
pub(crate) fn draw_bottom(
    grid: &mut Grid,
    rect: Rect,
    lx: &Lx,
    _page: &TaskPage,
) -> Option<(u16, u16)> {
    if rect.width == 0 || rect.height == 0 {
        return None;
    }
    // 调用方传进来的已经是 `list_rect`（顶边框 + tab 行之下）；这里直接使用，
    // 再减一次会把内容区压到 2 行、composer 永远放不下。
    let inner = rect;
    let Some(composer) = composer_rect_in(inner, lx) else {
        super::draw_placeholder(grid, inner, text::LX_BTW_QA_EMPTY);
        return None;
    };
    let thread = lx.btw.active_thread()?;
    // 输入框与 tab 之间固定留一行：所有 BTW loading（线程历史加载、生成回答）
    // 都落在**底部容器**左上角，不在对话区中央另起占位。
    if composer.y > inner.y {
        let label = if !thread.loaded {
            Some(text::LX_BTW_THREAD_LOADING)
        } else if thread.is_loading() {
            Some(loading_label(thread))
        } else {
            None
        };
        if let Some(label) = label {
            super::super::chat_bar::draw_inline_loading(
                grid,
                Rect::new(inner.x, inner.y, inner.width, 1),
                thread.chat.loading_frame,
                label,
            );
        }
    }
    let editor = thread.composer.editor();
    let visible = usize::from(composer.height.saturating_sub(2)).max(1);
    let total = editor.visual_rows().len().max(1);
    let max_scroll = total.saturating_sub(visible);
    let scroll = editor.scroll().min(max_scroll);
    let dims = super::super::input::draw(grid, composer, editor, true, scroll, visible);
    // 主问题归属放到输入框顶边框右上角（与主会话标题胶囊同一位置）。
    draw_qa_badge(grid, composer, lx);
    dims
}

/// 底部 loading 文案：沿用主会话的阶段口径（工具/思考/兜底），但不展示统计。
fn loading_label(thread: &crate::app::state::lx::BtwThreadState) -> &'static str {
    if let crate::app::state::RequestState::Streaming { assistant_id, .. } = &thread.chat.request
        && let Some(message) = thread
            .chat
            .messages
            .iter()
            .find(|message| message.id == *assistant_id && message.role == "assistant")
    {
        if message.tools().any(|call| {
            matches!(
                call.status,
                crate::app::state::ToolStatus::Pending | crate::app::state::ToolStatus::Running
            )
        }) {
            return text::chat::RUNNING_TOOLS_LABEL;
        }
        if matches!(
            message.steps.last(),
            Some(crate::app::state::AssistantStepView::Part { part, .. }) if part.reasoning_open()
        ) {
            return text::chat::THINKING_LABEL;
        }
    }
    text::chat::LOADING_LABEL
}

/// 输入框矩形（`task_list` 内部、tab 行与空行之下）；未打开线程时 None。
pub(crate) fn composer_rect(rect: Rect, lx: &Lx) -> Option<Rect> {
    composer_rect_in(list_rect(rect), lx)
}

fn composer_rect_in(inner: Rect, lx: &Lx) -> Option<Rect> {
    let thread = lx.btw.active_thread()?;
    let avail = inner.height;
    if avail < 3 {
        return None;
    }
    let rows = usize::from(COMPOSER_MAX_ROWS)
        .min(thread.composer.editor().visual_rows().len().max(1))
        .max(1);
    let height = u16::try_from(rows)
        .unwrap_or(COMPOSER_MAX_ROWS)
        .saturating_add(2);
    let height = height.min(avail);
    Some(Rect::new(
        inner.x,
        inner.bottom().saturating_sub(height),
        inner.width,
        height,
    ))
}

/// 主问题归属胶囊：` QA 2 · 你有什么功能 · To: lx `，画在输入框顶边框右侧。
///
/// 与主会话标题胶囊同一位置；背景/前景用固定的暗底亮字（不随壁纸或会话配色变化），
/// 保证在透明终端背景上也可读。
fn draw_qa_badge(grid: &mut Grid, rect: Rect, lx: &Lx) {
    let Some(thread) = lx.btw.active_thread() else {
        return;
    };
    let index = lx
        .btw
        .qa
        .iter()
        .position(|summary| summary.qa_id == thread.qa_id)
        .map(|index| index.saturating_add(1));
    let title = lx
        .btw
        .qa
        .iter()
        .find(|summary| summary.qa_id == thread.qa_id)
        .map(|summary| summary.title.as_str())
        .filter(|title| !title.trim().is_empty())
        .unwrap_or(text::LX_BTW_QA_UNTITLED);
    let prefix = match index {
        Some(index) => format!("QA {index} · "),
        None => "QA · ".to_string(),
    };
    let width = |value: &str| {
        u16::try_from(unicode_width::UnicodeWidthStr::width(value)).unwrap_or(u16::MAX)
    };
    let max = rect.width.saturating_sub(6).min(QA_BADGE_MAX_COLS);
    let prefix_w = width(prefix.as_str());
    let targets = format!(" · {}", thread.target_label());
    // 目标太长时先丢目标，保证主问题归属永远可见。
    let (room, targets) = if max
        .saturating_sub(prefix_w)
        .saturating_sub(width(targets.as_str()))
        >= 4
    {
        (
            max.saturating_sub(prefix_w)
                .saturating_sub(width(targets.as_str())),
            targets.as_str(),
        )
    } else {
        (max.saturating_sub(prefix_w), "")
    };
    if room < 4 {
        return;
    }
    let title = text::truncate_to_width(title, usize::from(room));
    if title.is_empty() {
        return;
    }
    let badge = format!(" {prefix}{title}{targets} ");
    let badge_w = width(badge.as_str());
    if rect.width <= badge_w.saturating_add(2) {
        return;
    }
    let x = rect.right().saturating_sub(badge_w).saturating_sub(1);
    grid.put(x, rect.y, &badge, style::btw_qa_badge());
}

/// `/btwList` 二级面板的锚定区域：整个任务面板上方的对话区。
///
/// 与主会话 `/resume` 一样画在输入区域之上：底边贴着任务面板上沿，不盖住 tab 行、
/// loading 空行和 composer。
fn picker_region(area: Rect, lx: &Lx) -> Option<(Rect, u16)> {
    if !lx.is_btw_tab() || !lx.btw.picker_open() {
        return None;
    }
    let task = super::super::layout(area, lx).task_list?;
    let height = task.y.saturating_sub(area.y);
    if height == 0 || task.width == 0 {
        return None;
    }
    Some((Rect::new(task.x, area.y, task.width, height), height))
}

/// 二级面板候选文案：空列表时是 loading/空态占位。
fn picker_labels(lx: &Lx) -> Vec<String> {
    if lx.btw.qa.is_empty() {
        let label = if lx.btw.picker_loading {
            text::LX_BTW_LIST_LOADING
        } else {
            text::LX_BTW_LIST_EMPTY
        };
        return vec![label.to_string()];
    }
    lx.btw
        .qa
        .iter()
        .enumerate()
        .map(|(index, summary)| {
            let title = if summary.title.trim().is_empty() {
                text::LX_BTW_QA_UNTITLED
            } else {
                summary.title.as_str()
            };
            format!(
                "QA {} · {}",
                index + 1,
                text::chat::single_line_label(title)
            )
        })
        .collect()
}

/// 二级面板条目：QA 标题 + 右侧旁路线程标记。
fn picker_items<'a>(lx: &Lx, labels: &'a [String]) -> Vec<CommandItem<'a>> {
    if lx.btw.qa.is_empty() {
        return vec![CommandItem::Inline {
            label: labels.first().map_or("", String::as_str),
            preview: "",
            tag: "",
        }];
    }
    lx.btw
        .qa
        .iter()
        .zip(labels.iter())
        .map(|(summary, label)| CommandItem::Inline {
            label: label.as_str(),
            preview: "",
            tag: if lx.btw.has_side_thread(summary) {
                text::LX_BTW_LIST_THREAD_MARK
            } else {
                ""
            },
        })
        .collect()
}

/// 当前旁路线程挂载的 QA 在列表中的下标（蓝色 `❯` 标记）。
fn picker_current(lx: &Lx) -> Option<usize> {
    let active = lx.btw.active.as_deref()?;
    lx.btw.qa.iter().position(|summary| summary.qa_id == active)
}

/// 复用 `/` 命令面板的边框/标题/底栏与窗口滚动，观感对齐主会话 `/resume`。
fn picker_view<'a>(
    lx: &Lx,
    items: &'a [CommandItem<'a>],
    width: u16,
    anchor_row: u16,
) -> CommandPanelView<'a> {
    let placeholder = lx.btw.qa.is_empty();
    let active = lx.btw.picking.unwrap_or(0);
    let cap = anchor_row.min(
        super::super::mention::MAX_POPUP_ROWS
            .saturating_add(super::super::mention::PANEL_BORDER_ROWS)
            .saturating_add(super::super::mention::PANEL_FOOTER_ROWS),
    );
    CommandPanelView {
        items,
        active,
        current: if placeholder {
            None
        } else {
            picker_current(lx)
        },
        window_anchor: Some(active),
        window_start: None,
        anchor_row,
        max_height: Some(cap),
        fixed_height: false,
        title: Some(text::LX_BTW_LIST_PANEL_TITLE),
        tabs: None,
        right_title: None,
        footer: Some(text::LX_BTW_LIST_PANEL_FOOTER),
        width: Some(width),
        placement: PanelPlacement::Above,
        borderless: true,
        hide_active_marker: placeholder,
        disabled: None,
    }
}

/// `/btwList` 二级面板几何；供绘制与鼠标命中复用。
pub(crate) fn picker_layout(area: Rect, lx: &Lx) -> Option<PanelLayout> {
    let (panel_area, anchor_row) = picker_region(area, lx)?;
    let labels = picker_labels(lx);
    let items = picker_items(lx, &labels);
    command_panel::layout(
        panel_area,
        &picker_view(lx, &items, panel_area.width, anchor_row),
    )
}

/// `/btwList` 二级面板命中：返回主 QA 下标；占位行不可选。
pub(crate) fn picker_item_at(area: Rect, lx: &Lx, column: u16, row: u16) -> Option<usize> {
    if lx.btw.qa.is_empty() {
        return None;
    }
    let layout = picker_layout(area, lx)?;
    let labels = picker_labels(lx);
    let items = picker_items(lx, &labels);
    command_panel::item_at(&layout, &items, column, row)
}

/// 绘制 `/btwList` 二级面板（画在任务面板上方，晚于对话区）。
pub(crate) fn render_picker(area: Rect, buf: &mut Buffer, lx: &Lx) {
    let Some((panel_area, anchor_row)) = picker_region(area, lx) else {
        return;
    };
    let labels = picker_labels(lx);
    let items = picker_items(lx, &labels);
    command_panel::render(
        panel_area,
        buf,
        &picker_view(lx, &items, panel_area.width, anchor_row),
    );
}

/// 对话区行数：与绘制同源；消息为空时至少 1 行占位。/// 对话区行数：与绘制同源；消息为空时至少 1 行占位。
pub(crate) fn detail_rows(
    rect: Rect,
    thread: &crate::app::state::lx::BtwThreadState,
    cache: &mut crate::ui::PaneCache,
) -> (usize, bool) {
    let content_width = history::content_rect(rect).width;
    if content_width == 0 {
        return (usize::from(rect.height), true);
    }
    if thread.chat.messages.is_empty() {
        return (1, true);
    }
    let source =
        HistorySource::from_chat(&thread.chat, &thread.compact).with_viewport(HistoryViewport {
            scroll: thread.scroll,
            follow_bottom: thread.follow,
        });
    cache.set_viewport_rows(usize::from(rect.height));
    history::measure_cached(content_width, source, cache)
}

/// 上方对话区（复用 Agents 详情同一套历史渲染）。
pub(crate) fn draw_conversation(
    grid: &mut Grid,
    rect: Rect,
    lx: &Lx,
    cache: &mut crate::ui::PaneCache,
) -> usize {
    if rect.width == 0 || rect.height == 0 {
        return 0;
    }
    let Some(thread) = lx.btw.active_thread() else {
        super::draw_placeholder(grid, rect, text::LX_BTW_EMPTY_THREAD);
        return 1;
    };
    if thread.chat.messages.is_empty() {
        // 历史加载中不在对话区中央画占位：底部空行已统一显示 loading。
        if thread.loaded {
            super::draw_placeholder(grid, rect, text::LX_BTW_EMPTY_THREAD);
        }
        return 1;
    }
    let source =
        HistorySource::from_chat(&thread.chat, &thread.compact).with_viewport(HistoryViewport {
            scroll: thread.scroll,
            follow_bottom: thread.follow,
        });
    let view = history::view_source_plain(rect, source, cache);
    let visible = usize::from(view.body.height);
    let max = view.total_rows.saturating_sub(visible);
    let top = if thread.follow {
        max
    } else {
        thread.scroll.min(max)
    };
    let total = history::render_source(grid, view.body, source, top, cache);
    if let Some(bar) = history::scrollbar(view.body, view.total_rows, visible, top) {
        for y in bar.thumb.y..bar.thumb.bottom() {
            grid.set(bar.thumb.x, y, '▐', style::text());
        }
    }
    total
}
