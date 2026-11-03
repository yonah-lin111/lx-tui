//! Prompt 面板体：工具栏、标题行、正文/树/页列表视图、滚动条与页码控件命中。

use ratatui::Frame;
use ratatui::layout::Rect;

use crate::app::prompt::{PromptEditing, PromptMode, page_display_title, page_rows};
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

/// 本页 `&&&` 模板块状态（编辑视图标题行的状态标签；无闭合块时 None）。
///
/// 未闭合的 `&&&` 不显示标签：`toggle_template_status` 无法改写缺少合法 `--end` 的块。
fn prompt_template_status(state: &AppState) -> Option<crate::app::markdown::TemplateStatus> {
    crate::app::markdown::block_spans(state.prompt.text())
        .into_iter()
        .find(|span| span.kind == crate::app::markdown::BlockKind::Template && span.closed)
        .map(|span| span.status)
}

/// 标题行标题来源：树视图无标题（纯分割线），编辑视图与页列表视图展示标题与页信息。
enum TitleSource {
    Tree,
    Editor {
        title: Option<String>,
        draft: bool,
        page: Option<PromptPageHeader>,
        status: Option<crate::app::markdown::TemplateStatus>,
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
            status: prompt_template_status(state),
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
            let page = pages.get(index)?;
            Some(PromptPageHeader {
                current: index + 1,
                total: pages.len(),
                name: page_display_title(page, index, state.prompt.text()),
                arrows: true,
            })
        }
        PromptMode::Pages => {
            let rows = page_rows(pages, active, panel.pages_view.search.text());
            // 无匹配行时计数为 `0/m`，与列表正文的 `no matching pages` 保持一致。
            let current = rows
                .get(panel.pages_view.selected)
                .and_then(|row| pages.iter().position(|page| page.id == row.page))
                .map(|index| index + 1)
                .unwrap_or(0);
            Some(PromptPageHeader {
                current,
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

/// 命中 prompt 工具栏页控件（编辑视图与页列表视图）。
pub fn prompt_page_control_at(
    panel: Rect,
    state: &AppState,
    column: u16,
    row: u16,
) -> Option<prompt::PageControl> {
    if state.prompt_collapsed || state.right_tab != RightTab::Prompt {
        return None;
    }
    let bar = crate::layout::prompt_toolbar_rect(panel)?;
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
    prompt::page_control_at(bar, &data, column, row)
}

/// 标题行状态标签矩形（渲染、点击命中与 tooltip 共用）；编辑视图且本页存在 `&&&` 时非空。
pub fn prompt_status_rect(panel: Rect, state: &AppState) -> Option<Rect> {
    if state.prompt_collapsed || state.right_tab != RightTab::Prompt {
        return None;
    }
    let area = title_row(panel)?;
    let TitleSource::Editor { status, .. } = title_source(state) else {
        return None;
    };
    let status = status?;
    prompt::status_rect(area, status)
}

/// 命中标题行状态标签时返回其矩形（点击循环块状态，不移动正文光标）。
pub fn prompt_status_at(panel: Rect, state: &AppState, column: u16, row: u16) -> Option<Rect> {
    let rect = prompt_status_rect(panel, state)?;
    rect.contains((column, row).into()).then_some(rect)
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
    let panel = &state.prompt_owner().prompt_panel;
    let mode = panel.mode;
    let recent_rows = recent_rows(state);
    let split = crate::layout::prompt_body_split(area, panel.recent_open, recent_rows.len());
    let title = title_source(state);
    let (title_text, draft, page, status) = match &title {
        TitleSource::Tree => (None, false, None, None),
        TitleSource::Editor {
            title,
            draft,
            page,
            status,
        } => (
            title.as_deref().or(Some("")),
            *draft,
            page.as_ref(),
            *status,
        ),
        TitleSource::Pages { title, draft, page } => {
            (title.as_deref().or(Some("")), *draft, page.as_ref(), None)
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
        page_data.page.as_ref(),
    );
    if panel.recent_open {
        let counter = page_data
            .page
            .as_ref()
            .map(|page| text::page_counter(page.current, page.total));
        prompt::paint_recent_button(area, frame.buffer_mut(), mode, counter.as_deref(), true);
    }
    if let Some(row) = title_row(area) {
        prompt::paint_title_row(frame.buffer_mut(), row, &page_data, status);
    }
    let text_area = split.main;
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
    if let Some(scrollbar) = prompt_scrollbar_for(state, split.main_scrollbar) {
        widgets::scrollbar::render(frame, &scrollbar);
    }
    if let (Some(divider), Some(recent_text)) = (split.recent_divider, split.recent_text) {
        prompt::recent::render(
            frame.buffer_mut(),
            &prompt::recent::RecentRender {
                panel: area,
                divider,
                area: recent_text,
                rows: &recent_rows,
                scroll: state.prompt_owner().prompt_panel.recent_scroll,
                visible_entries: split.visible_entries,
                focused,
                hover: state.prompt_recent_hover,
                active: state.prompt_owner().prompt_panel.recent_active,
            },
        );
    }
    if let Some(gutter) = split.recent_scrollbar
        && split.visible_entries > 0
        && recent_rows.len() > split.visible_entries
        && let Some(scrollbar) = widgets::scrollbar::layout(
            gutter,
            recent_rows.len(),
            split.visible_entries,
            state.prompt_owner().prompt_panel.recent_scroll,
        )
    {
        widgets::scrollbar::render(frame, &scrollbar);
    }
    // 抽屉展开时键盘归 recent：不再在上方编辑器/搜索框绘制硬件光标。
    if split.recent_divider.is_some() {
        return None;
    }
    if matches!(mode, PromptMode::Tree | PromptMode::Pages) {
        // 树/页列表视图硬件光标落在搜索框输入位置（与模态输入框一致），未聚焦不显示。
        return focused.then_some(list_cursor).flatten();
    }
    let (row, col) = focused.then(|| state.prompt.cursor_cell()).flatten()?;
    Some((
        text_area.x + col.min(text_area.width.saturating_sub(1)),
        text_area.y + row.min(text_area.height.saturating_sub(1)),
    ))
}

/// 显示中编辑器的实时上下文（树视图无编辑绑定，不参与 recent 标题刷新）。
fn live_editor(state: &AppState) -> Option<crate::app::prompt::LiveEditor<'_>> {
    let panel = &state.prompt_owner().prompt_panel;
    if panel.mode == PromptMode::Tree {
        return None;
    }
    let editing = panel.editing?;
    let page = match editing {
        PromptEditing::Item(id) => panel.tree.item(id)?.active_page,
        PromptEditing::Temp(id) => panel.temp(id)?.active_page,
    };
    Some((editing, page, state.prompt.text()))
}

/// 当前工作区的最近页面行（按修改时间倒序）。
pub(crate) fn recent_rows(state: &AppState) -> Vec<crate::app::prompt::RecentPageRow> {
    crate::app::prompt::recent_page_rows(
        &state.prompt_owner().prompt_panel,
        live_editor(state),
        crate::platform::current_unix_secs(),
    )
}

/// 指针是否落在最近页面抽屉区域（含分割线、空态与滚动条槽）。
pub fn prompt_recent_contains(panel: Rect, state: &AppState, column: u16, row: u16) -> bool {
    if state.prompt_collapsed || state.right_tab != RightTab::Prompt {
        return false;
    }
    let panel_state = &state.prompt_owner().prompt_panel;
    if !panel_state.recent_open {
        return false;
    }
    let rows = recent_rows(state);
    let split = crate::layout::prompt_body_split(panel, true, rows.len());
    let Some(divider) = split.recent_divider else {
        return false;
    };
    let inner = crate::layout::pane_inner_rect(panel);
    divider.x <= column && column < inner.right() && divider.y <= row && row < inner.bottom()
}

/// 命中最近页面抽屉右上角关闭按钮。
pub fn prompt_recent_close_at(panel: Rect, state: &AppState, column: u16, row: u16) -> bool {
    prompt_recent_close_rect(panel, state).is_some_and(|area| area.contains((column, row).into()))
}

/// 最近页面抽屉右上角关闭按钮矩形；抽屉收起、空间不足或宽度不够时为 None。
pub fn prompt_recent_close_rect(panel: Rect, state: &AppState) -> Option<Rect> {
    if state.prompt_collapsed || state.right_tab != RightTab::Prompt {
        return None;
    }
    let panel_state = &state.prompt_owner().prompt_panel;
    if !panel_state.recent_open {
        return None;
    }
    let rows = recent_rows(state);
    let split = crate::layout::prompt_body_split(panel, true, rows.len());
    prompt::recent::close_rect(split.recent_divider?)
}

/// 命中最近页面抽屉条目（返回条目下标）；抽屉未展开、空态或不在内容区时为 None。
pub fn prompt_recent_row_at(panel: Rect, state: &AppState, column: u16, row: u16) -> Option<usize> {
    if state.prompt_collapsed || state.right_tab != RightTab::Prompt {
        return None;
    }
    let panel_state = &state.prompt_owner().prompt_panel;
    if !panel_state.recent_open {
        return None;
    }
    let rows = recent_rows(state);
    let split = crate::layout::prompt_body_split(panel, true, rows.len());
    let area = split.recent_text?;
    if !area.contains((column, row).into()) {
        return None;
    }
    let visible = split.visible_entries.max(1);
    let start = panel_state
        .recent_scroll
        .min(rows.len().saturating_sub(visible));
    let index = start + usize::from(row - area.y) / 2;
    (index < rows.len() && split.visible_entries > 0).then_some(index)
}

/// 最近页面抽屉独立滚动条几何（渲染与鼠标命中共用）。
pub fn prompt_recent_scrollbar(
    panel: Rect,
    state: &AppState,
) -> Option<widgets::scrollbar::ScrollbarLayout> {
    if state.prompt_collapsed || state.right_tab != RightTab::Prompt {
        return None;
    }
    let panel_state = &state.prompt_owner().prompt_panel;
    if !panel_state.recent_open {
        return None;
    }
    let rows = recent_rows(state);
    let split = crate::layout::prompt_body_split(panel, true, rows.len());
    if split.visible_entries == 0 || rows.len() <= split.visible_entries {
        return None;
    }
    widgets::scrollbar::layout(
        split.recent_scrollbar?,
        rows.len(),
        split.visible_entries,
        panel_state.recent_scroll,
    )
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
    let split = crate::layout::prompt_body_split(
        view.prompt,
        state.prompt_owner().prompt_panel.recent_open,
        recent_rows(state).len(),
    );
    prompt_scrollbar_for(state, split.main_scrollbar)
}

fn prompt_scrollbar_for(
    state: &AppState,
    gutter: Option<Rect>,
) -> Option<widgets::scrollbar::ScrollbarLayout> {
    let owner = state.prompt_owner();
    let gutter = match owner.prompt_panel.mode {
        // 树/页列表的搜索框与分割线固定不滚动：滚动条从主视图第 3 行开始。
        PromptMode::Tree | PromptMode::Pages => {
            let gutter = gutter?;
            Rect {
                y: gutter
                    .y
                    .saturating_add(crate::layout::PROMPT_TREE_HEADER_HEIGHT),
                height: gutter
                    .height
                    .saturating_sub(crate::layout::PROMPT_TREE_HEADER_HEIGHT),
                ..gutter
            }
        }
        PromptMode::Editor => gutter?,
    };
    if gutter.height == 0 {
        return None;
    }
    let (total, offset) = match owner.prompt_panel.mode {
        PromptMode::Tree => (owner.prompt_panel.rows().len(), owner.prompt_panel.scroll),
        PromptMode::Pages => (
            prompt::pages::panel_page_rows(&owner.prompt_panel).len(),
            owner.prompt_panel.pages_view.scroll,
        ),
        PromptMode::Editor => (
            state.prompt.visual_rows().len() + state.prompt.bottom_padding(),
            state.prompt.scroll(),
        ),
    };
    widgets::scrollbar::layout(gutter, total, usize::from(gutter.height), offset)
}
