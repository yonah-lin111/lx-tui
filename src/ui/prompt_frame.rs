//! Prompt 面板体：工具栏、标题行、正文/树/页列表视图、滚动条与页码控件命中。

use ratatui::Frame;
use ratatui::layout::Rect;

use crate::app::prompt::{PromptEditing, PromptMode, page_rows};
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

/// edit 区边框色：编辑视图 + Prompt tab + 本页存在 `&&&` 时取其状态色；否则 None（保持灰）。
pub(crate) fn edit_border_style(state: &AppState) -> Option<ratatui::style::Style> {
    if state.right_tab != crate::app::state::RightTab::Prompt
        || state.prompt_owner().prompt_panel.mode != PromptMode::Editor
    {
        return None;
    }
    crate::app::markdown::block_spans(state.prompt.text())
        .into_iter()
        .find(|span| span.kind == crate::app::markdown::BlockKind::Template)
        .map(|span| super::style::template_border(span.status))
}

/// 标题行标题来源：树视图无标题（纯分割线），编辑视图与页列表视图展示标题与页信息。
enum TitleSource {
    Tree,
    Editor {
        title: Option<String>,
        draft: bool,
        page: Option<PromptPageHeader>,
    },
    Pages {
        title: Option<String>,
        draft: bool,
        page: Option<PromptPageHeader>,
    },
}

/// 读当前面板的标题行来源（名称与页信息按所有权持有，供调用方借用组装）。
fn title_source(state: &AppState) -> TitleSource {
    let panel = &state.prompt_owner().prompt_panel;
    let draft = matches!(panel.editing, Some(PromptEditing::Temp(_)));
    match panel.mode {
        PromptMode::Tree => TitleSource::Tree,
        PromptMode::Editor => TitleSource::Editor {
            title: prompt_edit_title(state),
            draft,
            page: prompt_page_header(state),
        },
        PromptMode::Pages => TitleSource::Pages {
            title: prompt_edit_title(state),
            draft,
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
        arrows: page.arrows,
    })
}

/// 显示中编辑目标的页信息：当前序号（1 起）、总页数、副标题（页名或 `pages`）与是否显示箭头。
pub(crate) struct PromptPageHeader {
    pub(crate) current: usize,
    pub(crate) total: usize,
    pub(crate) name: String,
    pub(crate) arrows: bool,
}

/// 读显示中编辑目标的页信息；树视图或未绑定目标时为 None。
pub(crate) fn prompt_page_header(state: &AppState) -> Option<PromptPageHeader> {
    let panel = &state.workspaces.get(state.prompt_workspace())?.prompt_panel;
    let (pages, active) = panel.editing_pages()?;
    match panel.mode {
        PromptMode::Tree => None,
        PromptMode::Editor => {
            let index = pages.iter().position(|page| page.id == active).unwrap_or(0);
            Some(PromptPageHeader {
                current: index + 1,
                total: pages.len(),
                name: String::new(),
                arrows: true,
            })
        }
        PromptMode::Pages => {
            let rows = page_rows(pages, active, panel.pages_view.search.text());
            let selected_page = rows
                .get(panel.pages_view.selected)
                .map(|row| row.page)
                .unwrap_or(active);
            let index = pages
                .iter()
                .position(|page| page.id == selected_page)
                .unwrap_or(0);
            Some(PromptPageHeader {
                current: index + 1,
                total: pages.len(),
                name: text::PROMPT_PAGES_LABEL.to_string(),
                arrows: false,
            })
        }
    }
}

/// 标题行几何：工具栏下一行**整行**（含两侧边框列）；表头隐藏时为 None。
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

/// 命中 prompt 标题行页控件（编辑视图与页列表视图）。
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
    let (title, page) = match title_source(state) {
        TitleSource::Editor { title, page, .. } | TitleSource::Pages { title, page, .. } => {
            (title, page)
        }
        TitleSource::Tree => return None,
    };
    let page = page.as_ref()?;
    let data = prompt::TitleRowData {
        title: Some(title.as_deref().unwrap_or("")),
        draft: false,
        page: page_header_data(Some(page)),
    };
    prompt::page_control_at(area, &data, column, row)
}

/// Prompt 面板体：编辑视图渲染 markdown 编辑器、树视图渲染提示词管理器、页视图渲染页目录。
pub(crate) fn render_body(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &AppState,
) -> Option<(u16, u16)> {
    if area.width == 0 || area.height == 0 {
        return None;
    }
    let focused = state.prompt_focused();
    let mode = state.prompt_owner().prompt_panel.mode;
    let title = title_source(state);
    let (title_text, draft, page) = match &title {
        TitleSource::Tree => (None, false, None),
        TitleSource::Editor { title, draft, page } | TitleSource::Pages { title, draft, page } => {
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
    let list_cursor = match mode {
        PromptMode::Tree => prompt::tree::render(
            text_area,
            frame.buffer_mut(),
            &state.prompt_owner().prompt_panel,
            focused,
            state.prompt_tree_hover,
            state.prompt_drag.filter(|drag| drag.dragging),
        ),
        PromptMode::Pages => prompt::pages::render(
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
    if matches!(mode, PromptMode::Tree | PromptMode::Pages) {
        // 树/页列表视图硬件光标落在搜索框输入位置（与模态输入框一致），未聚焦不显示。
        return focused.then_some(list_cursor).flatten();
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
        PromptMode::Pages => (
            crate::layout::prompt_tree_scrollbar_rect(panel)?,
            prompt::pages::panel_page_rows(&owner.prompt_panel).len(),
            owner.prompt_panel.pages_view.scroll,
        ),
        PromptMode::Editor => (
            crate::layout::prompt_scrollbar_rect(panel)?,
            state.prompt.visual_rows().len() + state.prompt.bottom_padding(),
            state.prompt.scroll(),
        ),
    };
    widgets::scrollbar::layout(gutter, total, usize::from(gutter.height), offset)
}
