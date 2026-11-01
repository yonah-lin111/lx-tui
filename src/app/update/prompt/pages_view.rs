//! 页列表视图（`PromptMode::Pages`）行为：进入/返回、选中与滚动、搜索过滤、双击与右键菜单。

use std::time::Instant;

use crate::app::actions::EditorCommand;
use crate::app::effects::Effect;
use crate::app::overlay::{Menu, MenuCommand, Overlay, OverlayTarget};
use crate::app::prompt::{PromptEditing, PromptMode, PromptPageId, PromptPanel, page_rows};
use crate::app::state::AppState;
use crate::ui::text;

use super::{
    PROMPT_DOUBLE_CLICK_WINDOW, commit_editing_prompt, prompt_tree_visible_rows, select_prompt_page,
};

/// 显示中工作区是否处于页列表视图。
pub fn prompt_pages_mode(state: &AppState) -> bool {
    state
        .workspaces
        .get(state.prompt_workspace())
        .is_some_and(|workspace| workspace.prompt_panel.mode == PromptMode::Pages)
}

/// 进入页列表：需编辑视图且已绑定目标；先结算当前页，选中当前页、清空搜索。
pub fn show_prompt_pages(state: &mut AppState) -> bool {
    let workspace = state.prompt_workspace();
    let is_editor_with_target = state.workspaces.get(workspace).is_some_and(|ws| {
        ws.prompt_panel.mode == PromptMode::Editor && ws.prompt_panel.editing.is_some()
    });
    if !is_editor_with_target {
        return false;
    }
    commit_editing_prompt(state);
    state.prompt.clear_panel();
    state.selection = None;
    state.prompt_tree_hover = None;
    let visible = prompt_tree_visible_rows(state);
    let Some(ws) = state.workspaces.get_mut(workspace) else {
        return false;
    };
    let panel = &mut ws.prompt_panel;
    let (selected, total) = panel
        .editing_pages()
        .map(|(pages, active)| {
            let idx = pages.iter().position(|page| page.id == active).unwrap_or(0);
            (idx, pages.len())
        })
        .unwrap_or((0, 0));
    panel.mode = PromptMode::Pages;
    panel.pages_view.search.clear();
    panel.pages_view.selected = selected;
    panel.pages_view.scroll = 0;
    clamp_page_view(panel, total, visible);
    true
}

/// 页列表返回编辑视图：`mode = Editor`；编辑缓冲未动，光标/撤销/滚动原样保留。
pub fn hide_prompt_pages(state: &mut AppState) -> bool {
    if !prompt_pages_mode(state) {
        return false;
    }
    let workspace = state.prompt_workspace();
    let Some(ws) = state.workspaces.get_mut(workspace) else {
        return false;
    };
    ws.prompt_panel.mode = PromptMode::Editor;
    state.prompt_tree_hover = None;
    true
}

/// 切换页列表与编辑视图（`Pages ↔ Editor`）；树视图或无编辑绑定时返回 false。
pub fn toggle_prompt_pages(state: &mut AppState) -> bool {
    if prompt_pages_mode(state) {
        hide_prompt_pages(state)
    } else {
        show_prompt_pages(state)
    }
}

/// 打开选中页：先回编辑视图，再切页（同页则仅返回，保留撤销栈）。
pub fn open_selected_page(state: &mut AppState) -> bool {
    let Some(page) = selected_page_id(state) else {
        return false;
    };
    if !hide_prompt_pages(state) {
        return false;
    }
    select_prompt_page(state, page);
    true
}

/// 当前选中可见行对应的页 id。
pub fn selected_page_id(state: &AppState) -> Option<PromptPageId> {
    let panel = &state.workspaces.get(state.prompt_workspace())?.prompt_panel;
    let (pages, active) = panel.editing_pages()?;
    let rows = page_rows(pages, active, panel.pages_view.search.text());
    rows.get(panel.pages_view.selected).map(|row| row.page)
}

/// 指定可见行索引对应的页 id。
pub fn page_id_at_row(state: &AppState, row: usize) -> Option<PromptPageId> {
    let panel = &state.workspaces.get(state.prompt_workspace())?.prompt_panel;
    let (pages, active) = panel.editing_pages()?;
    let rows = page_rows(pages, active, panel.pages_view.search.text());
    rows.get(row).map(|item| item.page)
}

/// 页列表上移/下移选中行（两端钳制不循环），滚动同步跟随。
pub fn move_page_selection(state: &mut AppState, delta: isize, visible: usize) -> bool {
    let workspace = state.prompt_workspace();
    let Some(ws) = state.workspaces.get_mut(workspace) else {
        return false;
    };
    let panel = &mut ws.prompt_panel;
    let Some((pages, active)) = panel.editing_pages() else {
        return false;
    };
    let len = page_rows(pages, active, panel.pages_view.search.text()).len();
    if len == 0 {
        return false;
    }
    let current = panel.pages_view.selected.min(len - 1);
    let next = (current as isize + delta).clamp(0, len as isize - 1) as usize;
    if next == panel.pages_view.selected {
        return false;
    }
    panel.pages_view.selected = next;
    clamp_page_view(panel, len, visible);
    true
}

/// 选中指定可见行（鼠标单击）；越界忽略。
pub fn select_page_row(state: &mut AppState, row: usize) -> bool {
    let workspace = state.prompt_workspace();
    let Some(ws) = state.workspaces.get_mut(workspace) else {
        return false;
    };
    let panel = &mut ws.prompt_panel;
    let Some((pages, active)) = panel.editing_pages() else {
        return false;
    };
    let len = page_rows(pages, active, panel.pages_view.search.text()).len();
    if row >= len || panel.pages_view.selected == row {
        return false;
    }
    panel.pages_view.selected = row;
    true
}

/// 页列表搜索框编辑：按单行输入语义执行命令；过滤词变化后选中落回首项并钳制滚动。
pub fn apply_page_search(state: &mut AppState, command: &EditorCommand) -> bool {
    let workspace = state.prompt_workspace();
    let visible = prompt_tree_visible_rows(state);
    let Some(ws) = state.workspaces.get_mut(workspace) else {
        return false;
    };
    let panel = &mut ws.prompt_panel;
    let before = panel.pages_view.search.text().to_string();
    match command {
        EditorCommand::InsertChar(ch) => panel.pages_view.search.insert_char(*ch),
        EditorCommand::InsertText(text) => {
            panel.pages_view.search.insert_str(text);
        }
        EditorCommand::Backspace => panel.pages_view.search.backspace(),
        EditorCommand::Delete => panel.pages_view.search.delete(),
        EditorCommand::Left => panel.pages_view.search.move_left(),
        EditorCommand::Right => panel.pages_view.search.move_right(),
        EditorCommand::Home | EditorCommand::LineStart => panel.pages_view.search.move_home(),
        EditorCommand::End | EditorCommand::LineEnd => panel.pages_view.search.move_end(),
        EditorCommand::DeleteToLineStart => panel.pages_view.search.delete_to_line_start(),
        EditorCommand::DeleteToLineEnd => panel.pages_view.search.delete_to_line_end(),
        EditorCommand::Undo => {
            panel.pages_view.search.undo();
        }
        EditorCommand::Redo => {
            panel.pages_view.search.redo();
        }
        _ => return false,
    }
    if panel.pages_view.search.text() != before {
        panel.pages_view.selected = 0;
        let len = panel
            .editing_pages()
            .map(|(pages, active)| page_rows(pages, active, panel.pages_view.search.text()).len())
            .unwrap_or(0);
        clamp_page_view(panel, len, visible);
    }
    true
}

/// 清空页列表搜索过滤词（`Ctrl+C`）：按清空前的选中页（无则当前页）重定位。
pub fn clear_page_search(state: &mut AppState) -> bool {
    let workspace = state.prompt_workspace();
    let visible = prompt_tree_visible_rows(state);
    let Some(ws) = state.workspaces.get_mut(workspace) else {
        return false;
    };
    let panel = &mut ws.prompt_panel;
    if panel.pages_view.search.text().is_empty() {
        return false;
    }
    let Some((pages, active)) = panel.editing_pages() else {
        return false;
    };
    let selected = page_rows(pages, active, panel.pages_view.search.text())
        .get(panel.pages_view.selected)
        .map(|row| row.page)
        .unwrap_or(active);
    panel.pages_view.search.clear();
    let Some((pages, active)) = panel.editing_pages() else {
        return false;
    };
    let rows = page_rows(pages, active, "");
    if let Some(pos) = rows.iter().position(|row| row.page == selected) {
        panel.pages_view.selected = pos;
    }
    clamp_page_view(panel, rows.len(), visible);
    true
}

/// 滚轮滚动页列表视口：只动 `scroll`，不动选中行。
pub fn scroll_page_list(state: &mut AppState, lines: isize, visible: usize) -> bool {
    let workspace = state.prompt_workspace();
    let Some(ws) = state.workspaces.get_mut(workspace) else {
        return false;
    };
    let panel = &mut ws.prompt_panel;
    let Some((pages, active)) = panel.editing_pages() else {
        return false;
    };
    let len = page_rows(pages, active, panel.pages_view.search.text()).len();
    let max = len.saturating_sub(visible.max(1));
    let target = (panel.pages_view.scroll.min(max) as isize).saturating_add(lines);
    let next = target.clamp(0, max as isize) as usize;
    let moved = next != panel.pages_view.scroll;
    panel.pages_view.scroll = next;
    moved
}

/// 直接设置页列表滚动偏移（滚动条轨道点击/拖拽）。
pub fn set_page_list_scroll(state: &mut AppState, offset: usize, visible: usize) -> bool {
    let workspace = state.prompt_workspace();
    let Some(ws) = state.workspaces.get_mut(workspace) else {
        return false;
    };
    let panel = &mut ws.prompt_panel;
    let Some((pages, active)) = panel.editing_pages() else {
        return false;
    };
    let len = page_rows(pages, active, panel.pages_view.search.text()).len();
    let max = len.saturating_sub(visible.max(1));
    let next = offset.min(max);
    let changed = next != panel.pages_view.scroll;
    panel.pages_view.scroll = next;
    changed
}

/// 登记页列表左键点击并判定是否构成双击；双击后点击序列复位。
pub fn page_row_double_click(state: &mut AppState, row: usize, now: Instant) -> bool {
    let double = state.page_row_click.is_some_and(|(last_row, at)| {
        last_row == row && now.saturating_duration_since(at) <= PROMPT_DOUBLE_CLICK_WINDOW
    });
    state.page_row_click = (!double).then_some((row, now));
    double
}

/// 页行右键菜单：打开、重命名、复制、新建、删除（仅多页时）；不改动选中行。
pub fn open_page_menu(state: &mut AppState, page: PromptPageId, anchor: (u16, u16)) {
    let workspace = state.prompt_workspace();
    let Some((owner, page_count)) = state.workspaces.get(workspace).and_then(|ws| {
        let owner = ws.prompt_panel.editing?;
        let (pages, _) = ws.prompt_panel.editing_pages()?;
        pages
            .iter()
            .any(|candidate| candidate.id == page)
            .then_some((owner, pages.len()))
    }) else {
        return;
    };
    let mut commands = vec![
        MenuCommand::OpenPage,
        MenuCommand::CopyPageContent,
        MenuCommand::NewPage,
    ];
    if page_count > 1 {
        commands.push(MenuCommand::DeletePage);
    }
    state.overlay = Some(Overlay::Menu(Menu {
        anchor,
        target: OverlayTarget::PromptPage {
            workspace,
            owner,
            page,
        },
        commands,
        selected: 0,
        confirming: false,
    }));
}

/// 页列表空白区域右键菜单：仅新建页。
pub fn open_pages_blank_menu(state: &mut AppState, anchor: (u16, u16)) {
    let workspace = state.prompt_workspace();
    let Some(owner) = state
        .workspaces
        .get(workspace)
        .and_then(|ws| ws.prompt_panel.editing)
    else {
        return;
    };
    state.overlay = Some(Overlay::Menu(Menu {
        anchor,
        target: OverlayTarget::PromptPages { workspace, owner },
        commands: vec![MenuCommand::NewPage],
        selected: 0,
        confirming: false,
    }));
}

/// 复制指定页全文到剪贴板，并在右侧面板弹出剪贴板 Toast。
pub fn copy_page_content(
    state: &mut AppState,
    workspace: usize,
    owner: PromptEditing,
    page: PromptPageId,
) -> bool {
    let Some(content) = state.workspaces.get(workspace).and_then(|ws| {
        let pages = match owner {
            PromptEditing::Item(id) => &ws.prompt_panel.tree.item(id)?.pages,
            PromptEditing::Temp(id) => &ws.prompt_panel.temp(id)?.pages,
        };
        pages
            .iter()
            .find(|candidate| candidate.id == page)
            .map(|candidate| candidate.content.clone())
    }) else {
        return false;
    };
    let anchor = Some(state.prompt.id());
    super::super::show_toast_at(
        state,
        anchor,
        crate::app::toast::Toast::new(
            crate::app::toast::ToastKind::Default,
            text::toast_copied(&content),
            anchor,
            Instant::now(),
        )
        .with_title(text::TOAST_CLIPBOARD_TITLE),
    );
    state.effects.push(Effect::CopyToClipboard {
        text: content,
        anchor,
    });
    true
}

/// 钳制页列表选中行与滚动偏移，使选中行在视口内可见。
pub(crate) fn clamp_page_view(panel: &mut PromptPanel, rows_len: usize, visible: usize) {
    let visible = visible.max(1);
    if rows_len == 0 {
        panel.pages_view.selected = 0;
        panel.pages_view.scroll = 0;
        return;
    }
    panel.pages_view.selected = panel.pages_view.selected.min(rows_len - 1);
    panel.pages_view.scroll = panel
        .pages_view
        .scroll
        .min(rows_len.saturating_sub(visible));
    if panel.pages_view.selected < panel.pages_view.scroll {
        panel.pages_view.scroll = panel.pages_view.selected;
    } else if panel.pages_view.selected >= panel.pages_view.scroll + visible {
        panel.pages_view.scroll = panel.pages_view.selected + 1 - visible;
    }
}

#[cfg(test)]
mod tests;
