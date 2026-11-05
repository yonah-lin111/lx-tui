//! 侧栏工作区历史列表渲染与鼠标命中：顶部搜索框 + 分割线 + Git 树形历史条目 + 滚动条。

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use unicode_width::UnicodeWidthChar;

use crate::app::state::{AppState, WorkspaceMode};
use crate::layout::PROMPT_TREE_HEADER_HEIGHT;
use crate::ui::{layout, style, text, widgets};

use super::WORKSPACE_ITEM_INDENT;

/// 渲染工作区历史列表；当侧栏聚焦时返回搜索框硬件光标位置。
pub fn render(frame: &mut Frame<'_>, area: Rect, state: &AppState) -> Option<(u16, u16)> {
    if area.width == 0 || area.height == 0 {
        return None;
    }
    let buf = frame.buffer_mut();
    let cursor = paint_search_row(buf, area, state);
    if area.height <= 1 {
        return cursor.filter(|_| state.sidebar_focused());
    }
    paint_divider(buf, area, area.y + 1, state.sidebar_focused());
    if area.height <= PROMPT_TREE_HEADER_HEIGHT {
        return cursor.filter(|_| state.sidebar_focused());
    }
    let list_area = Rect {
        y: area.y + PROMPT_TREE_HEADER_HEIGHT,
        height: area.height - PROMPT_TREE_HEADER_HEIGHT,
        ..area
    };
    let scrollbar = scrollbar_for(list_area, state);
    let rows_area = match &scrollbar {
        Some(_) => Rect {
            width: list_area.width.saturating_sub(1),
            ..list_area
        },
        None => list_area,
    };
    paint_rows(frame.buffer_mut(), rows_area, state);
    if let Some(scrollbar) = &scrollbar {
        widgets::scrollbar::render(frame, scrollbar);
    }
    cursor.filter(|_| state.sidebar_focused())
}

/// 绘制顶部搜索框行：`> ` 前缀 + 单行输入视口；返回硬件光标坐标。
fn paint_search_row(buf: &mut Buffer, area: Rect, state: &AppState) -> Option<(u16, u16)> {
    let panel = &state.workspace_history;
    let mut x = paint_text(
        buf,
        area.x,
        area.y,
        text::INPUT_PREFIX_ICON,
        style::accent(),
        area.right(),
    );
    x = paint_text(buf, x, area.y, " ", style::text(), area.right());
    let width = usize::from(area.right().saturating_sub(x));
    let input = widgets::input::view(panel.search.text(), panel.search.cursor(), width);
    if panel.search.text().is_empty() {
        paint_text(
            buf,
            x,
            area.y,
            text::WORKSPACE_HISTORY_SEARCH_PLACEHOLDER,
            style::muted(),
            area.right(),
        );
    } else {
        for span in widgets::input::build_styled_spans(
            input.visible,
            input.start_char,
            panel.search.selection(),
        ) {
            x = paint_text(
                buf,
                x,
                area.y,
                span.content.as_ref(),
                span.style,
                area.right(),
            );
        }
    }
    Some((
        area.x
            .saturating_add(width_of(text::INPUT_PREFIX_ICON) as u16)
            .saturating_add(1)
            .saturating_add(input.cursor_col)
            .min(area.right().saturating_sub(1).max(area.x)),
        area.y,
    ))
}

/// 绘制搜索框下方分割线：两端 `├` / `┤` 衔接侧栏左右边框。
fn paint_divider(buf: &mut Buffer, area: Rect, y: u16, focused: bool) {
    let divider_style = style::border(focused);
    let left_join = area.x.saturating_sub(1);
    if left_join < area.x
        && let Some(cell) = buf.cell_mut((left_join, y))
    {
        cell.reset();
        cell.set_symbol(text::DIVIDER_LEFT_JOIN);
        cell.set_style(divider_style);
    }
    for x in area.x..area.right() {
        if let Some(cell) = buf.cell_mut((x, y)) {
            cell.reset();
            cell.set_symbol(text::DIVIDER_MID);
            cell.set_style(divider_style);
        }
    }
    if let Some(cell) = buf.cell_mut((area.right(), y)) {
        cell.reset();
        cell.set_symbol(text::DIVIDER_RIGHT_JOIN);
        cell.set_style(divider_style);
    }
}

/// 绘制历史工作区树形条目列表；路径不存在的条目使用 `style::error()` 异色高亮。
fn paint_rows(buf: &mut Buffer, rows_area: Rect, state: &AppState) {
    if rows_area.width == 0 || rows_area.height == 0 {
        return;
    }
    let panel = &state.workspace_history;
    let rows = panel.visible_rows();
    if rows.is_empty() {
        let empty = if panel.search.text().trim().is_empty() {
            text::WORKSPACE_HISTORY_EMPTY
        } else {
            text::WORKSPACE_HISTORY_NO_MATCHES
        };
        paint_text(
            buf,
            rows_area.x,
            rows_area.y,
            empty,
            style::muted(),
            rows_area.right(),
        );
        return;
    }
    let scroll = panel.scroll.min(rows.len().saturating_sub(1));
    let selected = panel.selected.min(rows.len().saturating_sub(1));
    let drag_block = panel
        .drag
        .map(|index| panel.drag_block(index))
        .unwrap_or_default();
    for (vis_row, row) in rows
        .iter()
        .enumerate()
        .skip(scroll)
        .take(usize::from(rows_area.height))
    {
        let Some(entry) = panel.entries.get(row.index) else {
            continue;
        };
        let y = rows_area.y + (vis_row - scroll) as u16;
        let dragging = panel.dragging && drag_block.contains(&row.index);
        let hovered = panel.hover == Some(vis_row);
        let highlight = if dragging || hovered {
            fill_row(buf, rows_area, y);
            style::selection()
        } else {
            Style::default()
        };
        // 磁盘路径已不存在的条目使用红色（style::error()），hover 时只借用选中底色、保留红色前景。
        let item_style = if entry.exists {
            style::text().patch(highlight)
        } else {
            highlight.patch(style::error())
        };
        let prefix_style = if row.parent && entry.exists {
            highlight.patch(style::accent())
        } else {
            item_style
        };
        let mark_style = if entry.exists {
            highlight.patch(style::accent())
        } else {
            highlight.patch(style::error())
        };
        let marker = if vis_row == selected {
            format!("{} ", text::AGENT_SELECTED_MARKER)
        } else {
            "  ".to_string()
        };
        let group_prefix = if row.parent {
            let arrow = if row.collapsed {
                text::WORKSPACE_GROUP_COLLAPSED
            } else {
                text::WORKSPACE_GROUP_EXPANDED
            };
            format!("{arrow} ")
        } else if row.child {
            let tree = if row.last_child {
                text::WORKSPACE_TREE_LAST
            } else {
                text::WORKSPACE_TREE_MIDDLE
            };
            format!("{:width$}{tree} ", "", width = WORKSPACE_ITEM_INDENT)
        } else {
            " ".repeat(WORKSPACE_ITEM_INDENT)
        };
        let label = entry.display_label(row.child);
        let index_text = row
            .child_index
            .map(|index| format!(" {index}"))
            .unwrap_or_default();

        let mut x = paint_text(buf, rows_area.x, y, &marker, item_style, rows_area.right());
        x = paint_text(buf, x, y, &group_prefix, prefix_style, rows_area.right());
        let max_width = usize::from(rows_area.right().saturating_sub(x))
            .saturating_sub(index_text.chars().count());
        let truncated = text::ellipsize(&label, max_width);
        x = paint_text(buf, x, y, &truncated, item_style, rows_area.right());
        if !index_text.is_empty() {
            paint_text(buf, x, y, &index_text, mark_style, rows_area.right());
        }
    }
}

/// 工作区历史列表区域（上半区 `sections.workspaces`，仅在 `WorkspaceMode::History` 下有效）。
fn history_section_rect(view: &layout::ViewLayout, state: &AppState) -> Option<Rect> {
    if state.sidebar_collapsed || state.workspace_history.mode != WorkspaceMode::History {
        return None;
    }
    let sections = layout::sidebar_sections(view.sidebar, state.agents_collapsed)?;
    (sections.workspaces.width > 0 && sections.workspaces.height > 0).then_some(sections.workspaces)
}

/// 历史列表条目区矩形（扣除顶部搜索框与分割线 2 行）。
fn history_list_rect(view: &layout::ViewLayout, state: &AppState) -> Option<Rect> {
    let section = history_section_rect(view, state)?;
    if section.height <= PROMPT_TREE_HEADER_HEIGHT {
        return None;
    }
    Some(Rect {
        y: section.y + PROMPT_TREE_HEADER_HEIGHT,
        height: section.height - PROMPT_TREE_HEADER_HEIGHT,
        ..section
    })
}

fn scrollbar_for(list: Rect, state: &AppState) -> Option<widgets::scrollbar::ScrollbarLayout> {
    let total = state.workspace_history.visible_rows().len();
    widgets::scrollbar::layout(
        list,
        total,
        usize::from(list.height),
        state.workspace_history.scroll,
    )
}

/// 历史列表滚动条几何；不需要滚动或非历史模式时返回 None。
pub fn workspace_history_scrollbar(
    view: &layout::ViewLayout,
    state: &AppState,
) -> Option<widgets::scrollbar::ScrollbarLayout> {
    let list = history_list_rect(view, state)?;
    scrollbar_for(list, state)
}

/// 历史列表可见条目行数（扣除搜索框与分割线）。
pub fn workspace_history_visible_rows(
    view: &layout::ViewLayout,
    state: &AppState,
) -> Option<usize> {
    let list = history_list_rect(view, state)?;
    Some(usize::from(list.height))
}

/// 历史列表搜索框点击列对应的全文字符索引；非搜索框行或落在 `>` 前缀上时返回 None。
pub fn workspace_history_search_char_at(
    view: &layout::ViewLayout,
    state: &AppState,
    column: u16,
    row: u16,
) -> Option<usize> {
    let section = history_section_rect(view, state)?;
    if !section.contains((column, row).into()) || row != section.y {
        return None;
    }
    let text_x = section
        .x
        .saturating_add(width_of(text::INPUT_PREFIX_ICON) as u16)
        .saturating_add(1);
    if column < text_x {
        return Some(0);
    }
    let width = usize::from(section.right().saturating_sub(text_x));
    let search = &state.workspace_history.search;
    Some(widgets::input::char_index_at_col(
        search.text(),
        search.cursor(),
        width,
        column - text_x,
    ))
}

/// 历史列表内容区行槽（排除滚动条列）。
fn history_slot_at(
    view: &layout::ViewLayout,
    state: &AppState,
    column: u16,
    row: u16,
) -> Option<usize> {
    let list = history_list_rect(view, state)?;
    let content_width = match scrollbar_for(list, state) {
        Some(_) => list.width.saturating_sub(1),
        None => list.width,
    };
    let content = Rect {
        width: content_width,
        ..list
    };
    if content.width == 0 || !content.contains((column, row).into()) {
        return None;
    }
    let total = state.workspace_history.visible_rows().len();
    let scroll = state.workspace_history.scroll.min(total.saturating_sub(1));
    Some(scroll.saturating_add(usize::from(row - content.y)))
}

/// 历史列表父节点折叠箭头命中：命中父节点左侧箭头区（前 4 列）时返回该可见行下标 `visible_row`。
pub fn workspace_history_group_toggle_at(
    view: &layout::ViewLayout,
    state: &AppState,
    column: u16,
    row: u16,
) -> Option<usize> {
    let list = history_list_rect(view, state)?;
    if column >= list.x.saturating_add(4) {
        return None;
    }
    let slot = history_slot_at(view, state, column, row)?;
    let rows = state.workspace_history.visible_rows();
    rows.get(slot).filter(|item| item.parent).map(|_| slot)
}

/// 历史列表条目命中：返回过滤后的可见行下标 `visible_row`。
pub fn workspace_history_row_at(
    view: &layout::ViewLayout,
    state: &AppState,
    column: u16,
    row: u16,
) -> Option<usize> {
    let slot = history_slot_at(view, state, column, row)?;
    let total = state.workspace_history.visible_rows().len();
    (slot < total).then_some(slot)
}

/// 拖动排序目标可见行：指针行纵向钳到列表首/末行（横向不限，对齐活跃列表）。
pub fn workspace_history_drop_index(
    view: &layout::ViewLayout,
    state: &AppState,
    row: u16,
) -> Option<usize> {
    let list = history_list_rect(view, state)?;
    let total = state.workspace_history.visible_rows().len();
    if total == 0 {
        return None;
    }
    let clamped = row.clamp(list.y, list.bottom().saturating_sub(1));
    let scroll = state.workspace_history.scroll.min(total.saturating_sub(1));
    let slot = scroll.saturating_add(usize::from(clamped - list.y));
    (slot < total).then_some(slot)
}

/// 历史列表空白区域命中：内容区内（排除搜索框、分割线与滚动条列）且行槽越出已渲染条目。
pub fn workspace_history_blank_at(
    view: &layout::ViewLayout,
    state: &AppState,
    column: u16,
    row: u16,
) -> bool {
    let total = state.workspace_history.visible_rows().len();
    history_slot_at(view, state, column, row).is_some_and(|slot| slot >= total)
}

fn fill_row(buf: &mut Buffer, area: Rect, y: u16) {
    let highlight = style::selection();
    for x in area.x..area.right() {
        if let Some(cell) = buf.cell_mut((x, y)) {
            cell.reset();
            cell.set_style(highlight);
        }
    }
}

fn paint_text(buf: &mut Buffer, start_x: u16, y: u16, text: &str, style: Style, right: u16) -> u16 {
    let mut x = start_x;
    for ch in text.chars() {
        let ch_width = ch.width().unwrap_or(0) as u16;
        if ch_width == 0 {
            continue;
        }
        if x.saturating_add(ch_width) > right {
            break;
        }
        if let Some(cell) = buf.cell_mut((x, y)) {
            cell.reset();
            cell.set_char(ch);
            cell.set_style(style);
        }
        if ch_width == 2
            && let Some(trailing) = buf.cell_mut((x + 1, y))
        {
            trailing.reset();
            trailing.set_symbol("");
            trailing.set_style(style);
        }
        x += ch_width;
    }
    x
}

fn width_of(text: &str) -> usize {
    text.chars().map(|ch| ch.width().unwrap_or(0)).sum()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::style::Color;

    use super::*;
    use crate::app::state::{WorkspaceGit, WorkspaceHistoryEntry};
    use crate::app::update::show_workspace_history;
    use crate::config::Config;
    use crate::ui;

    #[test]
    fn workspace_mode_button_sits_right_of_add_button_and_flips_label() {
        let mut state = AppState::demo();
        let config = Config::default();
        let area = Rect::new(0, 0, 100, 30);
        let view = ui::layout::compute(
            area,
            &config,
            state.sidebar_collapsed,
            state.sidebar_width,
            state.prompt_collapsed,
            state.prompt_width,
            state.prompt_left,
            state.panes_collapsed,
        );
        let add_btn = ui::add_workspace_button(&view).expect("add button");
        let mode_btn = ui::workspace_mode_button(&view).expect("mode button");
        assert_eq!(mode_btn.y, add_btn.y);
        assert_eq!(mode_btn.width, 3);
        assert_eq!(mode_btn.right(), view.sidebar.right() - 1);
        assert_eq!(
            add_btn.right() + 1,
            mode_btn.x,
            "[+] 位于 [≡] 左侧并留 1 列间隔"
        );

        let mut terminal = Terminal::new(TestBackend::new(100, 30)).expect("terminal");
        terminal
            .draw(|frame| ui::render(frame, &state, &config))
            .expect("draw active");
        let buf = terminal.backend().buffer();
        let active_label: String = (0..mode_btn.width)
            .map(|dx| buf[(mode_btn.x + dx, mode_btn.y)].symbol())
            .collect();
        assert_eq!(active_label, text::PROMPT_MODE_LIST_LABEL);

        show_workspace_history(&mut state);
        terminal
            .draw(|frame| ui::render(frame, &state, &config))
            .expect("draw history");
        let buf = terminal.backend().buffer();
        let history_label: String = (0..mode_btn.width)
            .map(|dx| buf[(mode_btn.x + dx, mode_btn.y)].symbol())
            .collect();
        assert_eq!(history_label, text::PROMPT_MODE_EDIT_LABEL);
    }

    #[test]
    fn missing_directory_entry_renders_in_error_color_and_git_tree_renders_connectors() {
        let mut state = AppState::demo();
        let config = Config::default();
        let repo_root = PathBuf::from("/tmp/lx-tui");
        let wt_path = PathBuf::from("/tmp/lx-tui/.worktrees/workspace-history-list");
        state.workspace_history.entries = vec![
            WorkspaceHistoryEntry {
                name: "lx-tui".into(),
                path: repo_root.clone(),
                exists: true,
                name_is_manual: false,
                position: 0,
                git: Some(WorkspaceGit {
                    repo_root: repo_root.clone(),
                    checkout_path: repo_root.clone(),
                    is_linked: false,
                    branch: Some("dev".into()),
                    main_branch: Some("dev".into()),
                }),
            },
            WorkspaceHistoryEntry {
                name: "workspace-history-list".into(),
                path: wt_path.clone(),
                exists: false,
                name_is_manual: false,
                position: 1,
                git: Some(WorkspaceGit {
                    repo_root,
                    checkout_path: wt_path,
                    is_linked: true,
                    branch: Some("feature/workspace-history-list".into()),
                    main_branch: Some("dev".into()),
                }),
            },
        ];
        state.workspace_history.mode = WorkspaceMode::History;

        let mut terminal = Terminal::new(TestBackend::new(100, 30)).expect("terminal");
        terminal
            .draw(|frame| ui::render(frame, &state, &config))
            .expect("draw");
        let view = ui::layout::compute(
            Rect::new(0, 0, 100, 30),
            &config,
            state.sidebar_collapsed,
            state.sidebar_width,
            state.prompt_collapsed,
            state.prompt_width,
            state.prompt_left,
            state.panes_collapsed,
        );
        let sections =
            ui::layout::sidebar_sections(view.sidebar, state.agents_collapsed).expect("sections");
        let row0_y = sections.workspaces.y + PROMPT_TREE_HEADER_HEIGHT;
        let row1_y = row0_y + 1;
        let name_x = sections.workspaces.x + 8;

        let buf = terminal.backend().buffer();
        let row0_text: String = (sections.workspaces.x..sections.workspaces.right())
            .map(|x| buf[(x, row0_y)].symbol())
            .collect();
        let row1_text: String = (sections.workspaces.x..sections.workspaces.right())
            .map(|x| buf[(x, row1_y)].symbol())
            .collect();
        assert!(
            row0_text.contains("▾ lx-tui"),
            "父行应显示主工作区名: {row0_text}"
        );
        assert!(
            row1_text.contains("└─ feature/work"),
            "子行应缩进显示树连接符与分支名: {row1_text}"
        );
        assert_ne!(buf[(name_x, row0_y)].fg, Color::Red);
        assert_eq!(buf[(name_x, row1_y)].fg, Color::Red);
        assert_eq!(
            ui::workspace_history_group_toggle_at(&view, &state, sections.workspaces.x + 2, row0_y),
            Some(0)
        );
        assert_eq!(
            ui::workspace_history_row_at(&view, &state, name_x, row0_y),
            Some(0)
        );
        assert_eq!(
            ui::workspace_history_row_at(&view, &state, name_x, row1_y),
            Some(1)
        );
        assert!(ui::workspace_history_blank_at(
            &view,
            &state,
            name_x,
            row1_y + 1
        ));
    }

    #[test]
    fn dragged_history_item_renders_selection_across_full_row() {
        let mut state = AppState::demo();
        let config = Config::default();
        state.workspace_history.entries = vec![
            WorkspaceHistoryEntry {
                name: "alpha".into(),
                path: PathBuf::from("/tmp/lx-drag-alpha"),
                exists: true,
                name_is_manual: false,
                position: 0,
                git: None,
            },
            WorkspaceHistoryEntry {
                name: "beta".into(),
                path: PathBuf::from("/tmp/lx-drag-beta"),
                exists: true,
                name_is_manual: false,
                position: 1,
                git: None,
            },
        ];
        state.workspace_history.mode = WorkspaceMode::History;
        state.workspace_history.drag = Some(0);
        state.workspace_history.dragging = true;

        let mut terminal = Terminal::new(TestBackend::new(100, 30)).expect("terminal");
        terminal
            .draw(|frame| ui::render(frame, &state, &config))
            .expect("draw");
        let view = ui::layout::compute(
            Rect::new(0, 0, 100, 30),
            &config,
            state.sidebar_collapsed,
            state.sidebar_width,
            state.prompt_collapsed,
            state.prompt_width,
            state.prompt_left,
            state.panes_collapsed,
        );
        let sections =
            ui::layout::sidebar_sections(view.sidebar, state.agents_collapsed).expect("sections");
        let row0_y = sections.workspaces.y + PROMPT_TREE_HEADER_HEIGHT;
        let row1_y = row0_y + 1;
        let buf = terminal.backend().buffer();
        assert_eq!(
            buf[(sections.workspaces.x, row0_y)].bg,
            Color::Indexed(240),
            "拖动中的历史条目整行反显"
        );
        assert_eq!(
            buf[(sections.workspaces.right() - 1, row0_y)].bg,
            Color::Indexed(240),
            "反显覆盖整行空白区"
        );
        assert_ne!(
            buf[(sections.workspaces.x, row1_y)].bg,
            Color::Indexed(240),
            "未拖动的条目不高亮"
        );

        // 只按块反显：拖拽子项时同组父项一起高亮由 drag_block 决定。
        state.workspace_history.drag = None;
        state.workspace_history.dragging = false;
        terminal
            .draw(|frame| ui::render(frame, &state, &config))
            .expect("draw");
        let buf = terminal.backend().buffer();
        assert_ne!(buf[(sections.workspaces.x, row0_y)].bg, Color::Indexed(240));
    }
}
