//! Prompt 面板外框、顶底边框状态、滚动条与页码控件命中。

use std::path::Path;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Paragraph, Widget};
use unicode_width::UnicodeWidthStr;

use crate::app::prompt::{PromptEditing, PromptMode};
use crate::app::state::AppState;

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

/// Prompt 顶栏路径名：优先显式绑定根路径末段，回退显示中工作区路径末段。
fn prompt_path_name(state: &AppState) -> Option<&str> {
    state
        .prompt_root
        .as_deref()
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .or_else(|| workspace_path_name(state))
}

/// 显示中 prompt 所属工作区的路径末段名：优先 checkout 路径，回退工作区 cwd；
/// 无可用路径或路径无末段时为 None。
fn workspace_path_name(state: &AppState) -> Option<&str> {
    state
        .workspaces
        .get(state.prompt_workspace())
        .and_then(|workspace| {
            workspace
                .git
                .as_ref()
                .map(|git| git.checkout_path.as_path())
                .or(workspace.cwd.as_deref())
        })
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
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

/// prompt 底边框：左侧 git 状态 `b:分支`（linked worktree 追加 ` wt:工作区名`），
/// 右侧 `ws:路径末段名`；`ws:` 右端与折叠按钮留 1 列空档、左界不越过 `b:` 内容，
/// 放不下时先截断名字、再整体隐藏（`b:` 与折叠按钮优先级最高）。
fn render_prompt_bottom(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    let row = area.bottom().saturating_sub(1);
    let status_start = area.x.saturating_add(2);
    let status_end = area
        .right()
        .saturating_sub(PANEL_BUTTON_WIDTH + PANEL_BUTTON_MARGIN);
    if status_end <= status_start {
        return;
    }
    let spans = branch_status_spans(state, usize::from(status_end - status_start));
    let status_width = spans_width(&spans);
    if !spans.is_empty() {
        let line: Vec<Span<'_>> = spans
            .iter()
            .map(|(text, span_style)| Span::styled(text.as_str(), *span_style))
            .collect();
        Paragraph::new(Line::from(line)).render(
            Rect::new(status_start, row, status_end - status_start, 1),
            frame.buffer_mut(),
        );
    }
    // `ws:` 右端与折叠按钮之间留 1 列空档；左界取 `b:` 内容右端再空 1 列。
    let left = if status_width == 0 {
        status_start
    } else {
        status_start
            .saturating_add(status_width as u16)
            .saturating_add(1)
    };
    let right = status_end.saturating_sub(1);
    render_workspace_label(frame, row, left, right, state);
}

/// 底边框右侧 `ws:路径末段名`：右端贴 `right`（不含），前缀强调色、值恒为默认灰；
/// 放不下整段时保留前缀与名字开头（`…` 收尾），前缀加 2 列仍放不下时整体隐藏。
fn render_workspace_label(
    frame: &mut Frame<'_>,
    row: u16,
    left: u16,
    right: u16,
    state: &AppState,
) {
    if right <= left {
        return;
    }
    let Some(name) = prompt_path_name(state) else {
        return;
    };
    let prefix = text::PROMPT_WORKSPACE_PREFIX;
    let prefix_width = prefix.width();
    let name_width = name.width();
    let available = usize::from(right - left);
    if prefix_width.saturating_add(name_width) <= available {
        let start = right - (prefix_width + name_width) as u16;
        let buf = frame.buffer_mut();
        write_span(buf, start, row, prefix, style::accent());
        write_span(
            buf,
            start + prefix_width as u16,
            row,
            name,
            style::border_label_value(),
        );
        return;
    }
    if available < prefix_width.saturating_add(2) {
        return;
    }
    let truncated = text::truncate_to_width(name, available - prefix_width);
    let start = right - (prefix_width + truncated.width()) as u16;
    let buf = frame.buffer_mut();
    write_span(buf, start, row, prefix, style::accent());
    write_span(
        buf,
        start + prefix_width as u16,
        row,
        &truncated,
        style::border_label_value(),
    );
}

/// 在底边框写入一段带样式文本（覆盖边框线）；按显示列写入，宽字符占位正确。
fn write_span(buf: &mut ratatui::buffer::Buffer, x: u16, y: u16, content: &str, span_style: Style) {
    buf.set_stringn(x, y, content, content.width(), span_style);
}

/// 底边框分支状态片段：`b:分支` 与 linked worktree 的 ` wt:工作区名`；
/// 取显示中 prompt 所属工作区，非 git 不显示；工作区名按剩余宽度截断。
fn branch_status_spans(state: &AppState, available: usize) -> Vec<(String, Style)> {
    let Some(workspace) = state.workspaces.get(state.prompt_workspace()) else {
        return Vec::new();
    };
    let branch = workspace.git.as_ref().and_then(|git| git.status_branch());
    let linked = workspace.git.as_ref().is_some_and(|git| git.is_linked);
    if branch.is_none() && !linked {
        return Vec::new();
    }
    let mut spans = vec![(" ".to_string(), style::muted())];
    if let Some(branch) = branch {
        spans.push((text::PROMPT_BRANCH_PREFIX.to_string(), style::accent()));
        spans.push((branch.to_string(), style::border_label_value()));
    }
    if linked {
        let worktree = workspace.name.as_str();
        let separator = if branch.is_some() { " " } else { "" };
        let prefix = format!("{separator}{}", text::PROMPT_WORKTREE_PREFIX);
        let used = spans_width(&spans).saturating_add(prefix.len());
        if used < available {
            spans.push((prefix, style::accent()));
            spans.push((
                text::ellipsize(worktree, available - used),
                style::border_label_value(),
            ));
        }
    }
    spans
}

/// 片段列宽合计。
fn spans_width(spans: &[(String, Style)]) -> usize {
    spans.iter().map(|(text, _)| text.width()).sum()
}
