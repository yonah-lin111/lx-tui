//! 提示词分页行为：切页、新建/删除与当前页持久化。
//!
//! 所有离开当前正文的路径共享同一结算顺序：先提交编辑缓冲到当前页，
//! 再关闭正文相关面板/选区，最后切换当前页 id 并载入目标正文。

use crate::app::effects::Effect;
use crate::app::prompt::{
    PromptEditing, PromptMode, PromptPage, PromptPageId, PromptPanel, page_rows,
};
use crate::app::state::AppState;
use crate::ui::text;

use super::pages_view::{clamp_page_view, hide_prompt_pages, prompt_pages_mode};
use super::{commit_editing_prompt, prompt_tree_visible_rows};

/// 相对切页（标题行箭头与 `Ctrl/Cmd+←/→`）；越界或无法切页返回 false。
pub fn switch_prompt_page(state: &mut AppState, delta: isize) -> bool {
    let Some((pages, active)) = displayed_prompt_pages(state) else {
        return false;
    };
    let index = pages.iter().position(|page| page.id == active).unwrap_or(0);
    let target = index as isize + delta;
    if target < 0 || target as usize >= pages.len() {
        return false;
    }
    let page = pages[target as usize].id;
    select_prompt_page(state, page)
}

/// 切到指定页：结算旧页 → 关闭正文面板/选区 → 改写当前页 → 载入目标正文。
pub fn select_prompt_page(state: &mut AppState, page: PromptPageId) -> bool {
    let workspace = state.prompt_workspace();
    let Some((pages, active)) = displayed_prompt_pages(state) else {
        return false;
    };
    if active == page || !pages.iter().any(|candidate| candidate.id == page) {
        return false;
    }
    commit_editing_prompt(state);
    state.prompt.clear_panel();
    state.selection = None;
    let Some(target) = state
        .workspaces
        .get(workspace)
        .and_then(|workspace| workspace.prompt_panel.editing)
    else {
        return false;
    };
    let switched = with_panel(state, workspace, |panel| {
        set_active_page(panel, workspace, target, page)
    });
    if !switched {
        return false;
    }
    if let (Some(id), Some(page_id), Some(cwd)) = (
        item_id(target),
        page.saved(),
        workspace_cwd(state, workspace),
    ) {
        state.effects.push(Effect::SetActivePage {
            workspace: cwd,
            id,
            page_id,
        });
    }
    reload_editor_from_active_page(state, workspace);
    true
}

/// 在指定工作区面板上执行可变操作；工作区缺失返回 false。
fn with_panel(
    state: &mut AppState,
    workspace: usize,
    action: impl FnOnce(&mut PromptPanel) -> bool,
) -> bool {
    state
        .workspaces
        .get_mut(workspace)
        .is_some_and(|workspace| action(&mut workspace.prompt_panel))
}

/// 编辑目标中的正式条目 id；临时提示词为 None。
fn item_id(target: PromptEditing) -> Option<i64> {
    match target {
        PromptEditing::Item(id) => Some(id),
        PromptEditing::Temp(_) => None,
    }
}

/// 把编辑目标的当前页改写为 `page`。
fn set_active_page(
    panel: &mut PromptPanel,
    _workspace: usize,
    target: PromptEditing,
    page: PromptPageId,
) -> bool {
    match target {
        PromptEditing::Item(id) => {
            let Some(item) = panel.tree.item_mut(id) else {
                return false;
            };
            if item.page(page).is_none() {
                return false;
            }
            item.active_page = page;
            true
        }
        PromptEditing::Temp(id) => {
            let Some(temp) = panel.temp_mut(id) else {
                return false;
            };
            if temp.pages.iter().all(|candidate| candidate.id != page) {
                return false;
            }
            temp.active_page = page;
            true
        }
    }
}

/// `Ctrl/Cmd+Shift+→`：非末页时切换到下一页；末页时新建页（或跳转到已有空白页并 Toast）。
pub fn advance_or_create_prompt_page(state: &mut AppState) -> bool {
    let Some((pages, active)) = displayed_prompt_pages(state) else {
        return false;
    };
    let index = pages.iter().position(|page| page.id == active).unwrap_or(0);
    if index + 1 < pages.len() {
        state.prompt.clear_panel();
        return switch_prompt_page(state, 1);
    }
    create_or_toast_new_page(state)
}

/// 新建空页。
pub fn open_new_page(state: &mut AppState) -> bool {
    open_new_page_with_content(state, String::new())
}

/// 新建页并载入初始正文（Fork 复用）：处于页列表视图时保留在列表并选中新页；
/// 处于编辑视图时切到新页。
pub fn open_new_page_with_content(state: &mut AppState, content: String) -> bool {
    let workspace = state.prompt_workspace();
    let visible = prompt_tree_visible_rows(state);
    let Some(target) = state
        .workspaces
        .get(workspace)
        .and_then(|workspace| workspace.prompt_panel.editing)
    else {
        return false;
    };
    let in_pages = state.prompt_owner().prompt_panel.mode == PromptMode::Pages;
    if !in_pages {
        hide_prompt_pages(state);
    }
    // 先取消粘贴提案，再做结算判定；调用方（快捷键新建 / Fork）可能已结算过当前缓冲，
    // 内容与当前页一致时再提交只会向已 `page_pending` 的页重复登记同一条 `SavePromptContent`。
    state.prompt.clear_panel();
    if !buffer_matches_active_page(state) {
        commit_editing_prompt(state);
    }
    state.selection = None;
    state.overlay = None;
    match target {
        PromptEditing::Item(id) => {
            let Some(name) = state
                .workspaces
                .get(workspace)
                .and_then(|workspace| workspace.prompt_panel.tree.item(id))
                .map(|item| next_default_page_name(&item.pages))
            else {
                return false;
            };
            let Some(cwd) = workspace_cwd(state, workspace) else {
                return false;
            };
            state.effects.push(Effect::CreatePage {
                workspace: cwd,
                id,
                name,
                content,
            });
            true
        }
        PromptEditing::Temp(id) => {
            let Some(name) = state
                .workspaces
                .get(workspace)
                .and_then(|workspace| workspace.prompt_panel.temp(id))
                .map(|temp| next_default_page_name(&temp.pages))
            else {
                return false;
            };
            let added = with_panel(state, workspace, |panel| {
                let page_id = panel.alloc_temp_page_id();
                let page = PromptPageId::Temp(page_id);
                let mode = panel.mode;
                let position = {
                    if mode == PromptMode::Pages {
                        // 列表视图：新页标题不匹配过滤词，先清空搜索。
                        panel.pages_view.search.clear();
                    }
                    let Some(temp) = panel.temp_mut(id) else {
                        return false;
                    };
                    temp.pages.push(PromptPage::temp(page_id, &name, content));
                    if mode != PromptMode::Pages {
                        temp.active_page = page;
                        None
                    } else {
                        // 按完整列表（无搜索词）定位新页。
                        let rows = crate::app::prompt::pages_list::page_rows(
                            &temp.pages,
                            temp.active_page,
                            "",
                        );
                        let pos = rows
                            .iter()
                            .position(|row| row.page == page)
                            .unwrap_or(rows.len().saturating_sub(1));
                        Some((pos, rows.len()))
                    }
                };
                match position {
                    Some((pos, rows_len)) => {
                        panel.pages_view.selected = pos;
                        clamp_page_view(panel, rows_len, visible);
                    }
                    None => panel.mode = PromptMode::Editor,
                }
                true
            });
            if !added {
                return false;
            }
            if !in_pages {
                reload_editor_from_active_page(state, workspace);
            }
            true
        }
    }
}

/// 快捷键新建页（`Ctrl/Cmd+Shift+→`）：
/// 若已有未填写的空页，切到该空页并 Toast 提示已有新页；
/// 否则新建页、切入并 Toast 提示已新建。
pub fn create_or_toast_new_page(state: &mut AppState) -> bool {
    state.prompt.clear_panel();
    if !buffer_matches_active_page(state) {
        commit_editing_prompt(state);
    }
    let workspace = state.prompt_workspace();
    let visible = prompt_tree_visible_rows(state);
    let Some((pages, active)) = displayed_prompt_pages(state) else {
        return false;
    };
    let anchor = Some(state.prompt.id());
    if let Some(empty) = pages.iter().find(|page| page.content.trim().is_empty()) {
        let empty_id = empty.id;
        if prompt_pages_mode(state) {
            // 列表视图：清空搜索并把该空白页滚入视口，不切回编辑视图。
            with_panel(state, workspace, |panel| {
                panel.pages_view.search.clear();
                let Some((pages, active)) = panel.editing_pages() else {
                    return false;
                };
                let rows = page_rows(pages, active, "");
                if let Some(pos) = rows.iter().position(|row| row.page == empty_id) {
                    panel.pages_view.selected = pos;
                }
                clamp_page_view(panel, rows.len(), visible);
                true
            });
        } else if empty_id != active {
            select_prompt_page(state, empty_id);
        }
        state.overlay = None;
        super::super::show_toast_at(
            state,
            anchor,
            crate::app::toast::Toast::new(
                crate::app::toast::ToastKind::Warning,
                text::TOAST_PAGE_ALREADY_NEW,
                anchor,
                std::time::Instant::now(),
            ),
        );
        return true;
    }
    let created = open_new_page(state);
    if created {
        super::super::show_toast_at(
            state,
            anchor,
            crate::app::toast::Toast::new(
                crate::app::toast::ToastKind::Default,
                text::TOAST_PAGE_CREATED,
                anchor,
                std::time::Instant::now(),
            ),
        );
    }
    created
}

/// Fork 页：结算当前缓冲；源页为空时复用空页防呆，否则复制正文（标题追加 `(fork)`）
/// 到末尾新页并切入，最后弹出 `Page forked`。
pub fn fork_prompt_page(
    state: &mut AppState,
    workspace: usize,
    owner: PromptEditing,
    page: PromptPageId,
) -> bool {
    if workspace != state.prompt_workspace() {
        return false;
    }
    // 只处理显示中面板正在编辑且在页集合中的目标：避免提交别的缓冲后才发现目标无效。
    let valid = state.workspaces.get(workspace).is_some_and(|ws| {
        let panel = &ws.prompt_panel;
        if panel.editing != Some(owner) {
            return false;
        }
        let pages = match owner {
            PromptEditing::Item(id) => panel.tree.item(id).map(|item| &item.pages),
            PromptEditing::Temp(id) => panel.temp(id).map(|temp| &temp.pages),
        };
        pages.is_some_and(|pages| pages.iter().any(|candidate| candidate.id == page))
    });
    if !valid {
        return false;
    }
    let in_pages = prompt_pages_mode(state);
    commit_editing_prompt(state);
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
    if content.trim().is_empty() {
        return create_or_toast_new_page(state);
    }
    if in_pages {
        hide_prompt_pages(state);
    }
    let anchor = Some(state.prompt.id());
    let created =
        open_new_page_with_content(state, crate::app::markdown::fork_page_content(&content));
    if created {
        super::super::show_toast_at(
            state,
            anchor,
            crate::app::toast::Toast::new(
                crate::app::toast::ToastKind::Default,
                text::TOAST_PAGE_FORKED,
                anchor,
                std::time::Instant::now(),
            ),
        );
    }
    created
}

/// 重命名页标题：回到编辑视图并选中目标页，滚回顶部后按首个模板块的 `{title: ...}`
/// 选中标题值、定位光标或补入 `{title: }`；页面无模板块时弹出 Warning Toast。
pub fn rename_prompt_page_title(
    state: &mut AppState,
    workspace: usize,
    owner: PromptEditing,
    page: PromptPageId,
) -> bool {
    if workspace != state.prompt_workspace() {
        return false;
    }
    let Some(active) = state.workspaces.get(workspace).and_then(|ws| {
        let panel = &ws.prompt_panel;
        if panel.editing != Some(owner) {
            return None;
        }
        let owned = match owner {
            PromptEditing::Item(id) => panel
                .tree
                .item(id)
                .is_some_and(|item| item.pages.iter().any(|candidate| candidate.id == page)),
            PromptEditing::Temp(id) => panel
                .temp(id)
                .is_some_and(|temp| temp.pages.iter().any(|candidate| candidate.id == page)),
        };
        if !owned {
            return None;
        }
        panel.editing_pages().map(|(_, active)| active)
    }) else {
        return false;
    };
    // 先切到目标页（失败即放弃），再离开页列表：顺序反了会在失败时把用户留在编辑视图。
    if active != page && !select_prompt_page(state, page) {
        return false;
    }
    if prompt_pages_mode(state) {
        hide_prompt_pages(state);
    }
    super::super::focus_prompt(state);
    state.prompt.clear_panel();
    crate::app::update::clear_selection(state);
    let lookup = crate::app::markdown::find_first_template_title_span(state.prompt.text());
    match lookup {
        crate::app::markdown::TemplateTitleLookup::Found {
            value_start,
            value_end,
        } => {
            // 先回顶部，再按光标定位：标题在首屏内时钉住顶部，超出首屏时由 settle 滚入视口。
            state.prompt.scroll_to(0);
            if value_start == value_end {
                state.prompt.set_cursor_offset(value_start);
            } else if let Some((start, end)) = state
                .prompt
                .cell_range_for_byte_span(value_start, value_end)
            {
                state.selection = Some(crate::app::selection::Selection::programmatic_range(
                    state.prompt.id(),
                    start,
                    end,
                ));
                state.prompt.set_cursor_offset(value_end);
                if usize::try_from(end.0).unwrap_or(0) < usize::from(state.prompt.size().1) {
                    state.prompt.scroll_to(0);
                }
            } else {
                state.prompt.set_cursor_offset(value_end);
            }
            true
        }
        crate::app::markdown::TemplateTitleLookup::MissingTitleLine { start_line_end } => {
            // 先回顶部再插入：新标题仍在首屏时保持顶部，长前置正文下保留滚入视口的位置。
            state.prompt.scroll_to(0);
            let cursor = state.prompt.insert_template_title_line(start_line_end);
            state.prompt.set_cursor_offset(cursor);
            let settled = state.prompt.scroll();
            state.prompt.scroll_to(0);
            if state.prompt.cursor_cell().is_none() {
                state.prompt.scroll_to(settled);
            }
            true
        }
        crate::app::markdown::TemplateTitleLookup::NoTemplateBlock => {
            let anchor = Some(state.prompt.id());
            super::super::show_toast_at(
                state,
                anchor,
                crate::app::toast::Toast::new(
                    crate::app::toast::ToastKind::Warning,
                    text::TOAST_PAGE_NO_TITLE,
                    anchor,
                    std::time::Instant::now(),
                ),
            );
            false
        }
    }
}

/// 服务回调：正式条目新页已落库并重载；页列表视图下保持在列表，编辑视图下切到新页。
pub fn open_created_page(
    state: &mut AppState,
    workspace: usize,
    item_id: i64,
    page_id: i64,
    _name: &str,
) -> bool {
    let page = PromptPageId::Saved(page_id);
    let visible = prompt_tree_visible_rows(state);
    let mut was_pages = false;
    let opened = with_panel(state, workspace, |panel| {
        let mode = panel.mode;
        let Some(item) = panel.tree.item_mut(item_id) else {
            return false;
        };
        if item.page(page).is_none() {
            return false;
        }
        if mode == PromptMode::Pages {
            was_pages = true;
            // 列表视图：新页标题不匹配过滤词，先清空搜索再按完整列表定位并滚入视口。
            panel.pages_view.search.clear();
            let (pos, rows_len) = {
                let rows = page_rows(&item.pages, item.active_page, "");
                let pos = rows
                    .iter()
                    .position(|row| row.page == page)
                    .unwrap_or(rows.len().saturating_sub(1));
                (pos, rows.len())
            };
            panel.pages_view.selected = pos;
            clamp_page_view(panel, rows_len, visible);
            return true;
        }
        item.active_page = page;
        panel.mode = PromptMode::Editor;
        true
    });
    if !opened {
        return false;
    }
    if !was_pages && workspace == state.prompt_workspace() {
        reload_editor_from_active_page(state, workspace);
        state.prompt_collapsed = false;
        super::super::focus_prompt(state);
    }
    true
}

/// 服务回调：临时草稿升格后，按保存时的当前页索引绑定新条目。
pub fn open_saved_draft(
    state: &mut AppState,
    workspace: usize,
    item_id: i64,
    active: usize,
) -> bool {
    let Some((page, cwd)) = state
        .workspaces
        .get(workspace)
        .and_then(|workspace| {
            workspace.prompt_panel.tree.item(item_id).map(|item| {
                (
                    item.pages.get(active).map(|page| page.id),
                    workspace.cwd.clone(),
                )
            })
        })
        .and_then(|(page, cwd)| page.map(|page| (page, cwd)))
    else {
        return false;
    };
    let assigned = with_panel(state, workspace, |panel| {
        panel
            .tree
            .item_mut(item_id)
            .map(|item| item.active_page = page)
            .is_some()
    });
    if !assigned {
        return false;
    }
    if let (Some(page_id), Some(cwd)) = (page.saved(), cwd) {
        state.effects.push(Effect::SetActivePage {
            workspace: cwd,
            id: item_id,
            page_id,
        });
    }
    super::edit_prompt_item(state, workspace, item_id)
}

/// 服务回调：页删除已落库并重载；若当前编辑页正文已变化则同步编辑缓冲。
pub fn rebind_after_page_delete(state: &mut AppState, workspace: usize, item_id: i64) -> bool {
    if workspace != state.prompt_workspace() {
        return false;
    }
    let needs_reload = state
        .workspaces
        .get(workspace)
        .and_then(|ws| {
            let panel = &ws.prompt_panel;
            if panel.editing != Some(PromptEditing::Item(item_id)) || panel.mode == PromptMode::Tree
            {
                return None;
            }
            let (pages, active) = editing_pages(panel)?;
            let content = pages
                .iter()
                .find(|page| page.id == active)?
                .content
                .as_str();
            Some(state.prompt.text() != content)
        })
        .unwrap_or(false);
    if !needs_reload {
        return false;
    }
    state.prompt.clear_panel();
    state.selection = None;
    reload_editor_from_active_page(state, workspace)
}

/// 删除页的内存变更与持久化效应；删除当前页时按「下一页优先，末页取上一页」接续并重载编辑缓冲，
/// 删除非当前页时保持 `active_page` 与编辑缓冲（含撤销栈）不变。
pub fn perform_delete_page(
    state: &mut AppState,
    workspace: usize,
    owner: PromptEditing,
    page: PromptPageId,
) -> bool {
    let visible = prompt_tree_visible_rows(state);
    let mut was_active = false;
    let deleted = with_panel(state, workspace, |panel| {
        let query = panel.pages_view.search.text().to_string();
        let selected_index = panel.pages_view.selected;
        let (rows_len, target_pos) = {
            let (pages, active) = match owner {
                PromptEditing::Item(id) => {
                    let Some(item) = panel.tree.item_mut(id) else {
                        return false;
                    };
                    (&mut item.pages, &mut item.active_page)
                }
                PromptEditing::Temp(id) => {
                    let Some(temp) = panel.temp_mut(id) else {
                        return false;
                    };
                    (&mut temp.pages, &mut temp.active_page)
                }
            };
            let Some(index) = pages.iter().position(|candidate| candidate.id == page) else {
                return false;
            };
            if pages.len() <= 1 {
                return false;
            }
            was_active = *active == page;
            // 删除前记录选中行对应的页：被删行位于其上方时按 id 重定位，避免选中箭头漂移。
            let selected_page = page_rows(pages, *active, &query)
                .get(selected_index)
                .map(|row| row.page);
            pages.remove(index);
            if was_active {
                let next = pages
                    .get(index)
                    .or_else(|| pages.last())
                    .map(|candidate| candidate.id);
                let Some(next) = next else {
                    return false;
                };
                *active = next;
            }
            let rows = page_rows(pages, *active, &query);
            let target_pos = (selected_page != Some(page))
                .then(|| selected_page.and_then(|id| rows.iter().position(|row| row.page == id)))
                .flatten();
            (rows.len(), target_pos)
        };
        if let Some(pos) = target_pos {
            panel.pages_view.selected = pos;
        }
        clamp_page_view(panel, rows_len, visible);
        true
    });
    if !deleted {
        return false;
    }
    if let (Some(item_id), Some(page_id), Some(cwd)) = (
        item_id(owner),
        page.saved(),
        workspace_cwd(state, workspace),
    ) {
        state.effects.push(Effect::DeletePage {
            workspace: cwd,
            id: item_id,
            page_id,
        });
    }
    if was_active {
        state.prompt.clear_panel();
        state.selection = None;
        reload_editor_from_active_page(state, workspace);
    }
    if workspace == state.prompt_workspace() {
        let target = super::recent::current_recent_target(state);
        super::recent::sync_prompt_recent_selection(state, target);
        state.prompt_resize_pending = true;
    }
    true
}

/// 用编辑目标的当前页内容重装编辑缓冲（切换/删除当前页/新建后；Pages 与 Editor 视图下均生效）。
pub fn reload_editor_from_active_page(state: &mut AppState, workspace: usize) -> bool {
    if workspace != state.prompt_workspace() {
        return false;
    }
    let content = {
        let Some(panel) = state
            .workspaces
            .get(workspace)
            .map(|workspace| &workspace.prompt_panel)
        else {
            return false;
        };
        if panel.mode == PromptMode::Tree {
            return false;
        }
        editing_pages(panel).and_then(|(pages, active)| {
            pages
                .iter()
                .find(|page| page.id == active)
                .map(|page| page.content.clone())
        })
    };
    let Some(content) = content else {
        return false;
    };
    state.prompt.set_text(&content);
    true
}

/// 编辑目标的页集合与当前页 id。
pub(super) fn editing_pages(panel: &PromptPanel) -> Option<(&[PromptPage], PromptPageId)> {
    panel.editing_pages()
}

/// 显示中面板的编辑目标页集合与当前页 id。
fn displayed_prompt_pages(state: &AppState) -> Option<(&[PromptPage], PromptPageId)> {
    let panel = &state.workspaces.get(state.prompt_workspace())?.prompt_panel;
    panel.editing_pages()
}

/// 编辑缓冲是否已与当前页正文一致；一致时再次提交只会向已待确认的页重复登记保存。
fn buffer_matches_active_page(state: &AppState) -> bool {
    displayed_prompt_pages(state).is_some_and(|(pages, active)| {
        pages
            .iter()
            .find(|page| page.id == active)
            .is_some_and(|page| page.content == state.prompt.text())
    })
}

/// 下一个未使用的默认页名 `Page N`。
pub(super) fn next_default_page_name(pages: &[PromptPage]) -> String {
    let mut index = 1;
    loop {
        let candidate = text::default_page_name(index);
        if pages.iter().all(|page| page.name != candidate) {
            return candidate;
        }
        index += 1;
    }
}

/// 工作区 cwd；工作区已关闭时为 None。
pub(super) fn workspace_cwd(state: &AppState, workspace: usize) -> Option<std::path::PathBuf> {
    state
        .workspaces
        .get(workspace)
        .and_then(|workspace| workspace.cwd.clone())
}
