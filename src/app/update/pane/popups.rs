//! lx 输入框浮层面板（`@` 提及、路径粘贴、快捷引用与 `/` 斜杠命令）的状态更新。

use std::path::PathBuf;

use crate::app::state::{AppState, PaneView};
use crate::tiling::PaneId;

/// 单一窗格的有效 `@` 扫描根：`pane.cwd` 优先，回退所属工作区 cwd；两者皆无时 None。
///
/// 不使用当前进程 cwd、右侧 prompt 根或其它 pane 的 cwd。
pub fn lx_mention_root_for_pane(state: &AppState, id: PaneId) -> Option<PathBuf> {
    let pane = state.pane_anywhere(id)?;
    pane.cwd
        .clone()
        .or_else(|| state.workspace_cwd_for_pane(id))
}

/// 为指定窗格同步 `@` 提及扫描根与引用文件夹快照；任一变化返回 true（供置脏）。
pub fn sync_lx_mention_root(state: &mut AppState, id: PaneId) -> bool {
    let root = lx_mention_root_for_pane(state, id);
    let project = state.project_root_for_pane(id);
    let referenced = state.referenced.folders.clone();
    state.pane_mut_anywhere(id).is_some_and(|pane| {
        let ref_changed = pane.lx.sync_referenced_folders(&referenced);
        let mention = pane.lx.set_mention_root(root.clone());
        let project_changed = pane.lx.set_project_root(project.clone());
        let composer = pane.lx.sync_btw_composer_context(root, project);
        ref_changed || mention || project_changed || composer
    })
}

/// 为全部 lx 视图窗格同步 `@` 提及扫描根；任一根变化返回 true。
pub fn sync_all_lx_mention_roots(state: &mut AppState) -> bool {
    let mut changed = false;
    for id in state.all_pane_ids() {
        if state
            .pane_anywhere(id)
            .is_some_and(|pane| pane.view == PaneView::Lx)
        {
            changed |= sync_lx_mention_root(state, id);
        }
    }
    changed
}

/// 取走当前可见窗格待发起的提及扫描请求（窗格、BTW QA 归属、代号、根）；仅事件层调用。
///
/// `qa` 为 `Some` 表示来自 BTW 独立 composer：回投时只有仍是该 QA 的活动线程才接收，
/// 切换 QA 后旧结果会被丢弃。主输入框为 `None`。
pub fn take_lx_mention_scan_requests(
    state: &mut AppState,
) -> Vec<(PaneId, Option<String>, u64, PathBuf)> {
    let visible = state.active_tab().layout.pane_ids();
    let mut requests = Vec::new();
    for id in visible {
        let Some(pane) = state.pane_mut_anywhere(id) else {
            continue;
        };
        if pane.view != PaneView::Lx {
            continue;
        }
        let active_qa = pane.lx.btw.active.clone();
        if let Some((generation, root)) = pane.lx.take_mention_scan_request() {
            requests.push((id, None, generation, root));
        }
        if pane.lx.btw_editing()
            && let Some(composer) = pane.lx.btw.active_composer_mut()
            && let Some((generation, root)) = composer.take_mention_scan_request()
        {
            requests.push((id, active_qa, generation, root));
        }
    }
    requests
}

/// 写入指定窗格的提及扫描结果；窗格已删除或结果过期时丢弃。
///
/// BTW 结果只落到仍显示同一 QA 的 composer；主输入框结果按原语义落到主编辑器。
pub fn apply_lx_mention_entries(
    state: &mut AppState,
    id: PaneId,
    qa: Option<String>,
    generation: u64,
    root: &std::path::Path,
    entries: Vec<crate::app::markdown::MentionEntry>,
) {
    let Some(pane) = state.pane_mut_anywhere(id) else {
        return;
    };
    match qa {
        Some(qa_id) => {
            if pane.lx.btw.active.as_deref() != Some(qa_id.as_str()) {
                // 结果只回投当前 QA；其它线程的结果丢弃，但必须清掉原 composer 的在途标记。
                if let Some(thread) = pane.lx.btw.thread_mut(&qa_id) {
                    thread
                        .composer
                        .editor_mut()
                        .abandon_mention_scan(generation, root);
                }
                return;
            }
            if let Some(composer) = pane.lx.btw.active_composer_mut() {
                composer.apply_mention_entries(generation, root, entries);
            }
        }
        None => {
            pane.lx.apply_mention_entries(generation, root, entries);
        }
    }
}

/// 鼠标悬停 `@` 提及候选：更新高亮（窗口不滚动）；返回是否变化。
pub fn hover_lx_mention(state: &mut AppState, id: PaneId, index: usize) -> bool {
    let referenced = state.referenced.folders.clone();
    state.pane_mut_anywhere(id).is_some_and(|pane| {
        pane.lx.sync_referenced_folders(&referenced);
        pane.lx
            .with_active_editor(|lx| lx.mention_set_active(index))
    })
}

/// 鼠标悬停 `lx` 粘贴面板操作项：仅接受可用项；返回是否变化。
pub fn hover_lx_paste(state: &mut AppState, id: PaneId, index: usize) -> bool {
    state.pane_mut_anywhere(id).is_some_and(|pane| {
        pane.lx.with_active_editor(|lx| {
            if lx.text_paste_panel().is_some() {
                return lx.text_paste_set_active(index);
            }
            lx.paste_set_active(index)
        })
    })
}

/// 鼠标点选 `lx` 粘贴面板操作项：按该项确认；返回是否消费。
pub fn select_lx_paste(state: &mut AppState, id: PaneId, index: usize) -> bool {
    let Some(pane) = state.pane_mut_anywhere(id) else {
        return false;
    };
    pane.lx.with_active_editor(|lx| {
        if lx.text_paste_panel().is_some() {
            let Some(action) = crate::app::paste::TextPasteAction::ALL.get(index).copied() else {
                return false;
            };
            return lx.text_paste_confirm(action);
        }
        let Some(action) = crate::app::paste::PasteAction::ALL.get(index).copied() else {
            return false;
        };
        lx.paste_confirm(action)
    })
}

/// 鼠标点击 `@` 提及面板顶部 Tab：切换当前 Tab；返回是否变化。
pub fn switch_lx_mention_tab(state: &mut AppState, id: PaneId, tab: usize) -> bool {
    let Some(tab) = crate::app::prompt::MentionTab::from_index(tab) else {
        return false;
    };
    let referenced = state.referenced.folders.clone();
    state.pane_mut_anywhere(id).is_some_and(|pane| {
        pane.lx.sync_referenced_folders(&referenced);
        pane.lx.with_active_editor(|lx| lx.mention_set_tab(tab))
    })
}

/// 鼠标点选 `@` 提及候选：设置高亮并按 `Enter` 规则采纳；返回是否消费。
pub fn select_lx_mention(state: &mut AppState, id: PaneId, index: usize) -> bool {
    let referenced = state.referenced.folders.clone();
    let Some(pane) = state.pane_mut_anywhere(id) else {
        return false;
    };
    pane.lx.sync_referenced_folders(&referenced);
    pane.lx.with_active_editor(|lx| {
        if !lx.mention_set_active(index) && !lx.mention_open() {
            return false;
        }
        lx.mention_confirm()
    })
}

/// 滚轮在 `@` 提及面板上滚动候选视口；`base` 为当前窗口起点，高亮不动。
pub fn scroll_lx_mention(state: &mut AppState, id: PaneId, delta: isize, base: usize) -> bool {
    state.pane_mut_anywhere(id).is_some_and(|pane| {
        pane.lx
            .with_active_editor(|lx| lx.mention_scroll(delta, base))
    })
}

/// 鼠标悬停快捷引用面板条目：更新高亮（窗口不滚动）；返回是否变化。
pub fn hover_lx_refer(state: &mut AppState, id: PaneId, index: usize) -> bool {
    state
        .pane_mut_anywhere(id)
        .is_some_and(|pane| pane.lx.with_active_editor(|lx| lx.refer_set_active(index)))
}

/// 鼠标点选快捷引用候选：设置高亮并采纳；返回是否消费。
pub fn select_lx_refer(state: &mut AppState, id: PaneId, index: usize) -> bool {
    let Some(pane) = state.pane_mut_anywhere(id) else {
        return false;
    };
    pane.lx.with_active_editor(|lx| {
        if !lx.refer_set_active(index) && !lx.refer_active() {
            return false;
        }
        lx.refer_confirm()
    })
}

/// 滚轮在快捷引用面板上滚动候选视口；`base` 为当前窗口起点，高亮不动。
pub fn scroll_lx_refer(state: &mut AppState, id: PaneId, delta: isize, base: usize) -> bool {
    state.pane_mut_anywhere(id).is_some_and(|pane| {
        pane.lx
            .with_active_editor(|lx| lx.refer_scroll(delta, base))
    })
}

/// 鼠标悬停 `/` 命令面板条目：更新高亮（窗口不滚动）；返回是否变化。
pub fn hover_lx_slash(state: &mut AppState, id: PaneId, index: usize) -> bool {
    state
        .pane_mut_anywhere(id)
        .is_some_and(|pane| pane.lx.slash.as_mut().is_some_and(|p| p.set_active(index)))
}

/// 鼠标点选 `/` 命令面板条目：按该条目的 Enter 语义采纳；返回是否消费。
pub fn select_lx_slash(state: &mut AppState, id: PaneId, index: usize) -> bool {
    let valid = state
        .pane_mut_anywhere(id)
        .and_then(|pane| pane.lx.slash.as_mut())
        .is_some_and(|panel| {
            if index >= panel.item_count() {
                return false;
            }
            let _ = panel.set_active(index);
            true
        });
    if !valid {
        return false;
    }
    super::super::chat::apply_slash_enter(state, id)
}

/// 滚轮在 `/` 命令面板上滚动窗口；`base` 为当前窗口起点，高亮不动。
pub fn scroll_lx_slash(state: &mut AppState, id: PaneId, delta: isize, base: usize) -> bool {
    state.pane_mut_anywhere(id).is_some_and(|pane| {
        pane.lx
            .slash
            .as_mut()
            .is_some_and(|p| p.scroll_viewport(delta, base))
    })
}
