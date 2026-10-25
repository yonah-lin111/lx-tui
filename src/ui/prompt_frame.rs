//! Prompt 面板体：工具栏、标题行、正文/树视图、滚动条与页码控件命中。

use ratatui::Frame;
use ratatui::layout::Rect;

use crate::app::prompt::{PromptEditing, PromptMode};
use crate::app::state::{AppState, RightTab};

use super::{layout, prompt, text, widgets};

/// 编辑视图标题：绑定条目的名称；临时提示词显示固定标题；无绑定不显示。
pub(crate) fn prompt_edit_title(state: &AppState) -> Option<String> {
    let panel = &state.prompt_owner().prompt_panel;
    match panel.editing? {
        PromptEditing::Item(id) => panel.tree.item(id).map(|item| item.name.clone()),
        PromptEditing::Temp(_) => Some(text::PROMPT_DRAFT_TITLE.to_string()),
    }
}

/// 标题行标题来源：树视图无标题（纯分割线）。
enum TitleSource {
    Tree,
    Editor {
        title: Option<String>,
        draft: bool,
        page: Option<PromptPageHeader>,
    },
}

/// 读当前面板的标题行来源（名称与页信息按所有权持有，供调用方借用组装）。
fn title_source(state: &AppState) -> TitleSource {
    let panel = &state.prompt_owner().prompt_panel;
    match panel.mode {
        PromptMode::Tree => TitleSource::Tree,
        PromptMode::Editor => TitleSource::Editor {
            title: prompt_edit_title(state),
            draft: matches!(panel.editing, Some(PromptEditing::Temp(_))),
            page: prompt_page_header(state),
        },
    }
}

/// 页信息 → 标题行渲染/命中数据。
fn page_header_data(page: Option<&PromptPageHeader>) -> Option<prompt::PageHeaderData<'_>> {
    page.map(|page| prompt::PageHeaderData {
        current: page.current,
        total: page.total,
        name: &page.name,
        rename: page
            .rename
            .as_ref()
            .map(|(input, cursor)| (input.as_str(), *cursor)),
    })
}

/// 显示中编辑目标的页信息：当前序号（1 起）、总页数、页名与重命名现场。
pub(crate) struct PromptPageHeader {
    pub(crate) current: usize,
    pub(crate) total: usize,
    pub(crate) name: String,
    pub(crate) rename: Option<(String, usize)>,
}

/// 读显示中编辑目标的页信息；非编辑视图或未绑定目标时为 None。
pub(crate) fn prompt_page_header(state: &AppState) -> Option<PromptPageHeader> {
    let panel = &state.workspaces.get(state.prompt_workspace())?.prompt_panel;
    if panel.mode != PromptMode::Editor {
        return None;
    }
    let (pages, active) = match panel.editing? {
        PromptEditing::Item(id) => {
            let item = panel.tree.item(id)?;
            (item.pages.as_slice(), item.active_page)
        }
        PromptEditing::Temp(id) => {
            let temp = panel.temp(id)?;
            (temp.pages.as_slice(), temp.active_page)
        }
    };
    let index = pages.iter().position(|page| page.id == active).unwrap_or(0);
    let name = pages
        .get(index)
        .map(|page| page.name.clone())
        .unwrap_or_default();
    let rename = panel
        .page_rename()
        .map(|rename| (rename.input.text().to_string(), rename.input.cursor()));
    Some(PromptPageHeader {
        current: index + 1,
        total: pages.len(),
        name,
        rename,
    })
}

/// 标题行几何：工具栏下一行**整行**（含两侧边框列）；表头隐藏时为 None。
///
/// 用整行而不是 `prompt_divider_rect` 的内容区矩形，是为了让页控件"距右边角留 1 列
/// 边框线"的算术与旧顶边框版本完全一致（`next = area.right() - 3`）。
pub fn title_row(panel: Rect) -> Option<Rect> {
    crate::layout::prompt_divider_rect(panel).map(|row| Rect {
        x: panel.x,
        width: panel.width,
        ..row
    })
}

/// 坐标是否落在标题行（标题/页控件所在的控件区）。
pub fn prompt_title_at(panel: Rect, column: u16, row: u16) -> bool {
    title_row(panel).is_some_and(|title| title.contains((column, row).into()))
}

/// 命中 prompt 标题行页控件（编辑视图）；
/// 页重命名输入期间命中输入区（`PageControl::Name`），条目名与空白返回 None。
pub fn prompt_page_control_at(
    panel: Rect,
    state: &AppState,
    column: u16,
    row: u16,
) -> Option<prompt::PageControl> {
    if state.prompt_collapsed {
        return None;
    }
    let area = title_row(panel)?;
    let TitleSource::Editor { title, page, .. } = title_source(state) else {
        return None;
    };
    let page = page.as_ref()?;
    let data = prompt::TitleRowData {
        title: Some(title.as_deref().unwrap_or("")),
        draft: false,
        page: page_header_data(Some(page)),
    };
    prompt::page_control_at(area, &data, column, row)
}

/// Prompt 面板体：编辑视图渲染 markdown 编辑器（聚焦时可输入、可选择复制）、
/// 树视图渲染提示词管理器。外框、tab 与底边框由容器（`ui::right_panel`）负责。
///
/// 先画工具栏与分割线，再把标题/页控件覆盖到分割线行上（顺序不可交换）。
/// 文本区固定预留最右 1 列作滚动条槽。
/// 返回编辑视图聚焦时的硬件光标位置：终端把 IME 预输入绘制在硬件光标处，需与编辑器光标同步。
pub(crate) fn render_body(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &AppState,
) -> Option<(u16, u16)> {
    if area.width == 0 || area.height == 0 {
        return None;
    }
    let focused = state.prompt_focused;
    let mode = state.prompt_owner().prompt_panel.mode;
    let title = title_source(state);
    let (title_text, draft, page) = match &title {
        TitleSource::Tree => (None, false, None),
        TitleSource::Editor { title, draft, page } => {
            (title.as_deref().or(Some("")), *draft, page.as_ref())
        }
    };
    let page_data = prompt::TitleRowData {
        title: title_text,
        draft,
        page: page_header_data(page),
    };
    prompt::render_header(
        area,
        frame.buffer_mut(),
        &state.prompt,
        focused,
        mode,
        draft,
        state.prompt_pinned.is_some(),
    );
    if let Some(row) = title_row(area) {
        prompt::paint_title_row(frame.buffer_mut(), row, &page_data);
    }
    let text_area = crate::layout::prompt_text_rect(area);
    let tree_cursor = match mode {
        PromptMode::Tree => prompt::tree::render(
            text_area,
            frame.buffer_mut(),
            &state.prompt_owner().prompt_panel,
            focused,
            state.prompt_tree_hover,
        ),
        PromptMode::Editor => {
            prompt::render(
                text_area,
                frame.buffer_mut(),
                &state.prompt,
                state.selection_for(state.prompt.id()),
            );
            None
        }
    };
    if let Some(scrollbar) = prompt_scrollbar_for(area, state) {
        widgets::scrollbar::render(frame, &scrollbar);
    }
    if mode == PromptMode::Tree {
        // 树视图硬件光标落在搜索框输入位置（与模态输入框一致），未聚焦不显示。
        return focused.then_some(tree_cursor).flatten();
    }
    if let (Some(page), Some(row)) = (page.filter(|page| page.rename.is_some()), title_row(area)) {
        return focused
            .then(|| {
                prompt::rename_cursor(
                    row,
                    title_text,
                    &prompt::PageHeaderData {
                        current: page.current,
                        total: page.total,
                        name: &page.name,
                        rename: page
                            .rename
                            .as_ref()
                            .map(|(input, cursor)| (input.as_str(), *cursor)),
                    },
                )
            })
            .flatten();
    }
    let (row, col) = focused.then(|| state.prompt.cursor_cell()).flatten()?;
    Some((
        text_area.x + col.min(text_area.width.saturating_sub(1)),
        text_area.y + row,
    ))
}

/// prompt 滚动条几何：文本溢出内容区时可见；渲染与鼠标命中共用。
///
/// Todo 停用时既不可见也不可命中。
pub fn prompt_scrollbar(
    view: &layout::ViewLayout,
    state: &AppState,
) -> Option<widgets::scrollbar::ScrollbarLayout> {
    if state.right_tab != RightTab::Prompt {
        return None;
    }
    prompt_scrollbar_for(view.prompt, state)
}

fn prompt_scrollbar_for(
    panel: Rect,
    state: &AppState,
) -> Option<widgets::scrollbar::ScrollbarLayout> {
    let owner = state.prompt_owner();
    let (gutter, total, offset) = match owner.prompt_panel.mode {
        PromptMode::Tree => (
            crate::layout::prompt_tree_scrollbar_rect(panel)?,
            owner.prompt_panel.rows().len(),
            owner.prompt_panel.scroll,
        ),
        PromptMode::Editor => (
            crate::layout::prompt_scrollbar_rect(panel)?,
            state.prompt.visual_rows().len() + state.prompt.bottom_padding(),
            state.prompt.scroll(),
        ),
    };
    widgets::scrollbar::layout(gutter, total, usize::from(gutter.height), offset)
}
