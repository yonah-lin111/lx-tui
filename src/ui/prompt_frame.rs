//! Prompt 面板外框、顶底边框状态、滚动条与页码控件命中。

use std::path::Path;

use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
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

/// 命中 prompt 标题行页控件（编辑视图）；
/// 页重命名输入期间名称区不命中（改由键盘输入消费）。
pub fn prompt_page_control_at(
    panel: Rect,
    state: &AppState,
    column: u16,
    row: u16,
) -> Option<prompt::PageControl> {
    if state.prompt_collapsed {
        return None;
    }
    let header = crate::layout::prompt_header_rect(panel)?;
    let page = prompt_page_header(state)?;
    let title = prompt_edit_title(state)?;
    let data = prompt::PageHeaderData {
        current: page.current,
        total: page.total,
        name: &page.name,
        rename: page
            .rename
            .as_ref()
            .map(|(input, cursor)| (input.as_str(), *cursor)),
    };
    if page.rename.is_some() {
        let controls = prompt::page_controls(header, &text::page_counter(page.current, page.total));
        if let Some(controls) = controls.as_ref() {
            for (rect, control) in [
                (controls.prev, prompt::PageControl::Prev),
                (controls.counter, prompt::PageControl::Counter),
                (controls.next, prompt::PageControl::Next),
            ] {
                if rect.contains((column, row).into()) {
                    return Some(control);
                }
            }
        }
        return None;
    }
    prompt::header_page_control_at(header, &title, &data, column, row)
}

/// 右栏 prompt：编辑视图渲染 markdown 编辑器（聚焦时可输入、可选择复制）、
/// 树视图渲染提示词管理器；折叠时渲染为窄条。
///
/// 文本区固定预留最右 1 列作滚动条槽，文本/树溢出时该列显示滚动条。
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
    let mut block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(style::border(focused))
        .title(Span::styled(
            format!(" {} ", text::PROMPT_TITLE),
            style::border_title(),
        ));
    // 右侧路径名与左侧标题重叠时不绘制，保证面板标题始终可见。
    if let Some(name) = prompt_path_name(state) {
        let left = text::PROMPT_TITLE.width() + 2;
        let right = name.width() + text::PROMPT_WORKSPACE_PREFIX.width() + 1;
        if left + right < usize::from(area.width.saturating_sub(2)) {
            // 前缀与值分色，与底边框 `b:分支` 一致：前缀强调色、值 muted。
            let label = Line::from(vec![
                Span::styled(" ", style::muted()),
                Span::styled(text::PROMPT_WORKSPACE_PREFIX, style::accent()),
                Span::styled(name.to_string(), style::muted()),
                Span::styled(" ", style::muted()),
            ])
            .alignment(Alignment::Right);
            block = block.title_top(label);
        }
    }
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return None;
    }
    let title = prompt_edit_title(state);
    let draft = matches!(
        state.prompt_owner().prompt_panel.editing,
        Some(PromptEditing::Temp(_))
    );
    let page = prompt_page_header(state);
    let page_data = page.as_ref().map(|page| prompt::PageHeaderData {
        current: page.current,
        total: page.total,
        name: &page.name,
        rename: page
            .rename
            .as_ref()
            .map(|(input, cursor)| (input.as_str(), *cursor)),
    });
    prompt::render_header(
        area,
        frame.buffer_mut(),
        &state.prompt,
        focused,
        mode,
        title.as_deref(),
        draft,
        state.prompt_pinned.is_some(),
        page_data,
    );
    render_prompt_branch_status(frame, area, state);
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
    if let Some(header) = crate::layout::prompt_header_rect(area)
        && let Some(page) = page.as_ref()
        && page.rename.is_some()
    {
        let title = title.as_deref().unwrap_or("");
        let data = prompt::PageHeaderData {
            current: page.current,
            total: page.total,
            name: &page.name,
            rename: page
                .rename
                .as_ref()
                .map(|(input, cursor)| (input.as_str(), *cursor)),
        };
        return focused
            .then(|| prompt::header_rename_cursor(header, title, &data))
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

/// prompt 底边框左侧 git 状态：`b:分支`（linked worktree 显示仓库主 checkout 分支），
/// linked worktree 追加 ` wt:工作区名`；取显示中 prompt 所属工作区，非 git 不显示，
/// 右端避让折叠按钮。
fn render_prompt_branch_status(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let Some(workspace) = state.workspaces.get(state.prompt_workspace()) else {
        return;
    };
    let branch = workspace.git.as_ref().and_then(|git| git.status_branch());
    let linked = workspace.git.as_ref().is_some_and(|git| git.is_linked);
    if branch.is_none() && !linked {
        return;
    }
    let start = area.x.saturating_add(2);
    let end = area
        .right()
        .saturating_sub(PANEL_BUTTON_WIDTH + PANEL_BUTTON_MARGIN);
    if area.height == 0 || end <= start {
        return;
    }
    let spans = branch_status_spans(
        branch,
        linked.then_some(workspace.name.as_str()),
        usize::from(end - start),
    );
    if spans.is_empty() {
        return;
    }
    let line: Vec<Span<'_>> = spans
        .iter()
        .map(|(text, span_style)| Span::styled(text.as_str(), *span_style))
        .collect();
    Paragraph::new(Line::from(line)).render(
        Rect::new(start, area.bottom().saturating_sub(1), end - start, 1),
        frame.buffer_mut(),
    );
}

/// 底边框分支状态片段：`b:分支` 与 linked worktree 的 ` wt:工作区名`；
/// 工作区名按剩余宽度截断，放不下时省略 `wt:` 片段。
fn branch_status_spans(
    branch: Option<&str>,
    worktree: Option<&str>,
    available: usize,
) -> Vec<(String, Style)> {
    let mut spans = vec![(" ".to_string(), style::muted())];
    if let Some(branch) = branch {
        spans.push((text::PROMPT_BRANCH_PREFIX.to_string(), style::accent()));
        spans.push((branch.to_string(), style::muted()));
    }
    if let Some(worktree) = worktree {
        let separator = if branch.is_some() { " " } else { "" };
        let prefix = format!("{separator}{}", text::PROMPT_WORKTREE_PREFIX);
        let used = spans_width(&spans).saturating_add(prefix.len());
        if used < available {
            spans.push((prefix, style::accent()));
            spans.push((text::ellipsize(worktree, available - used), style::muted()));
        }
    }
    spans
}

/// 片段列宽合计。
fn spans_width(spans: &[(String, Style)]) -> usize {
    spans.iter().map(|(text, _)| text.width()).sum()
}
