//! Prompt 面板外框、顶底边框状态、滚动条与页码控件命中。

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Paragraph, Widget};

use crate::app::prompt::{PromptEditing, PromptMode};
use crate::app::state::{AppState, Workspace, WorkspaceIdentity};

use super::controls::{PANEL_BUTTON_MARGIN, PANEL_BUTTON_WIDTH, render_collapsed_strip};
use super::{layout, prompt, style, text, widgets};

/// 编辑视图标题：绑定条目的名称；临时提示词显示固定标题；无绑定不显示。
pub(crate) fn prompt_edit_title(state: &AppState) -> Option<String> {
    let panel = &state.prompt_owner().prompt_panel;
    match panel.editing? {
        PromptEditing::Item(id) => panel.tree.item(id).map(|item| item.name.clone()),
        PromptEditing::Temp(_) => Some(text::PROMPT_DRAFT_TITLE.to_string()),
    }
}

/// 顶边框标题数据：编辑视图取绑定目标名（`None` 时整段留空），树视图用固定标题 `Prompts`。
enum BorderTitle {
    Tree,
    Editor {
        title: Option<String>,
        draft: bool,
        page: Option<PromptPageHeader>,
    },
}

/// 读当前面板的顶边框标题来源（名称与页信息按所有权持有，供调用方借用组装）。
fn prompt_border_title(state: &AppState) -> BorderTitle {
    let panel = &state.prompt_owner().prompt_panel;
    match panel.mode {
        PromptMode::Tree => BorderTitle::Tree,
        PromptMode::Editor => BorderTitle::Editor {
            title: prompt_edit_title(state),
            draft: matches!(panel.editing, Some(PromptEditing::Temp(_))),
            page: prompt_page_header(state),
        },
    }
}

/// 页信息 → 顶边框渲染/命中数据。
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

/// 命中 prompt 顶边框标题行页控件（编辑视图）；
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
    let border = crate::layout::prompt_border_rect(panel)?;
    let BorderTitle::Editor { title, page, .. } = prompt_border_title(state) else {
        return None;
    };
    let page = page.as_ref()?;
    let data = prompt::BorderTitleData {
        title: Some(title.as_deref().unwrap_or("")),
        draft: false,
        page: page_header_data(Some(page)),
    };
    prompt::border_page_control_at(border, &data, column, row)
}

/// 右栏 prompt：编辑视图渲染 markdown 编辑器（聚焦时可输入、可选择复制）、
/// 树视图渲染提示词管理器；折叠时渲染为窄条。
///
/// 面板顶边框内嵌标题（编辑视图 `条目名 · 页名` + 右侧 `‹ n/m ›`，树视图固定 `Prompts`），
/// 工具栏下方分割线保持空行。文本区固定预留最右 1 列作滚动条槽。
/// 返回编辑视图聚焦时的硬件光标位置：终端把 IME 预输入绘制在硬件光标处，需与编辑器光标同步。
pub(crate) fn render_prompt(
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
    let focused = state.prompt_focused;
    let mode = state.prompt_owner().prompt_panel.mode;
    let border = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(style::border(focused));
    let inner = border.inner(area);
    frame.render_widget(border, area);
    if inner.width == 0 || inner.height == 0 {
        return None;
    }
    let title = prompt_border_title(state);
    let (title_text, draft, page) = match &title {
        BorderTitle::Tree => (Some(text::PROMPT_TREE_BORDER_TITLE), false, None),
        BorderTitle::Editor { title, draft, page } => {
            (title.as_deref().or(Some("")), *draft, page.as_ref())
        }
    };
    let page_data = prompt::BorderTitleData {
        title: title_text,
        draft,
        page: page_header_data(page),
    };
    if let Some(border_rect) = crate::layout::prompt_border_rect(area) {
        prompt::paint_border_title(frame.buffer_mut(), border_rect, &page_data);
    }
    prompt::render_header(
        area,
        frame.buffer_mut(),
        &state.prompt,
        focused,
        mode,
        draft,
        state.prompt_pinned.is_some(),
    );
    render_prompt_bottom(frame, area, state);
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
    if let Some(page) = page.filter(|page| page.rename.is_some())
        && let Some(border_rect) = crate::layout::prompt_border_rect(area)
    {
        return focused
            .then(|| {
                prompt::border_rename_cursor(
                    border_rect,
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
pub fn prompt_scrollbar(
    view: &layout::ViewLayout,
    state: &AppState,
) -> Option<widgets::scrollbar::ScrollbarLayout> {
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

/// 显示中 prompt 所属工作区的身份标签：与 lx 窗格顶边框同一份数据与格式，
/// 只读，忽略 `prompt_root`（子目录绑定不改变身份）。
fn prompt_identity(state: &AppState) -> Option<WorkspaceIdentity> {
    state
        .workspaces
        .get(state.prompt_workspace())
        .map(Workspace::identity)
}

/// prompt 底边框：整段左对齐的身份标签 `项目 · b:分支 · wt:工作区名`（与 lx 窗格同款），
/// 右侧只保留折叠按钮；放不下时按同一降级阶梯省略。
fn render_prompt_bottom(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
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
