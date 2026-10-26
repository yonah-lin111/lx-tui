//! 引用文件夹内容搜索弹窗行为：打开/换根、后台扫描回投、筛选导航、复制与关闭。
//!
//! 文件系统扫描登记为 `ReferencedRequest`，由事件层后台执行后回投；本层只做状态转换。

use std::path::PathBuf;

use crate::app::effects::Effect;
use crate::app::markdown::ReferKind;
use crate::app::overlay::{Overlay, ReferencedFolderSearch, TextInput};
use crate::app::referenced::{ReferencedRequest, ReferencedScanOutcome};
use crate::app::state::AppState;

use super::{next_generation, open_referenced_folder_menu};

/// 打开内容搜索弹窗：以目录为根，登记后台扫描（代号随请求递增）。
pub fn open_referenced_folder_search(
    state: &mut AppState,
    folder: i64,
    path: PathBuf,
    anchor: (u16, u16),
) -> bool {
    let Some(key) = state.referenced_key() else {
        return false;
    };
    // 磁盘可用性由后台扫描判定（app 层不执行 IO）：根缺失/非目录/无权限时由结果回报失败。
    let id = next_generation(state);
    state.overlay = Some(Overlay::ReferencedFolderSearch(ReferencedFolderSearch {
        anchor,
        folder,
        key: Some(key),
        root: path.clone(),
        initial_root: path.clone(),
        stack: Vec::new(),
        query: TextInput::new(String::new()),
        entries: Vec::new(),
        items: Vec::new(),
        scan_truncated: false,
        truncated: false,
        selected: 0,
        viewport: None,
        loading: true,
        failed: false,
        request: Some(id),
    }));
    state.referenced_requests.push(ReferencedRequest::Scan {
        id,
        folder,
        root: path,
    });
    true
}

/// 进入高亮目录：把搜索范围收窄到该子目录、清空查询并重启扫描。
pub fn referenced_search_enter(state: &mut AppState) -> bool {
    let (folder, path) = {
        let Some(Overlay::ReferencedFolderSearch(search)) = state.overlay.as_ref() else {
            return false;
        };
        let Some(entry) = search.items.get(search.selected) else {
            return false;
        };
        if !entry.is_directory {
            return false;
        }
        (search.folder, entry.path.clone())
    };
    goto_referenced_root(state, folder, path, true)
}

/// 回退上层搜索范围：栈空（已在初始根）时保持弹窗不动。
pub fn referenced_search_back(state: &mut AppState) -> bool {
    let (folder, target) = {
        let Some(Overlay::ReferencedFolderSearch(search)) = state.overlay.as_ref() else {
            return false;
        };
        let Some(target) = search.stack.last().cloned() else {
            return false;
        };
        (search.folder, target)
    };
    if !goto_referenced_root(state, folder, target, false) {
        return false;
    }
    if let Some(Overlay::ReferencedFolderSearch(search)) = state.overlay.as_mut() {
        search.stack.pop();
    }
    true
}

/// 切换搜索根：登记新扫描并清空旧候选；`push` 为真时把旧根压栈（进入子目录）。
fn goto_referenced_root(state: &mut AppState, folder: i64, root: PathBuf, push: bool) -> bool {
    let id = next_generation(state);
    let Some(Overlay::ReferencedFolderSearch(search)) = state.overlay.as_mut() else {
        return false;
    };
    if push {
        search.stack.push(search.root.clone());
    }
    search.request = Some(id);
    search.root = root.clone();
    search.query.clear();
    search.entries.clear();
    search.items.clear();
    search.scan_truncated = false;
    search.truncated = false;
    search.selected = 0;
    search.loading = true;
    search.failed = false;
    state
        .referenced_requests
        .push(ReferencedRequest::Scan { id, folder, root });
    true
}

/// 写入后台扫描结果：代号不匹配（换根/关窗/键变化）时丢弃。
pub fn apply_referenced_scan(
    state: &mut AppState,
    id: u64,
    outcome: ReferencedScanOutcome,
) -> bool {
    let Some(Overlay::ReferencedFolderSearch(search)) = state.overlay.as_mut() else {
        return false;
    };
    if search.request != Some(id) {
        return false;
    }
    search.loading = false;
    search.failed = false;
    search.entries_loaded(outcome);
    true
}

/// 写入后台扫描失败：迟到的失败结果丢弃。
pub fn apply_referenced_scan_failure(state: &mut AppState, id: u64) -> bool {
    let Some(Overlay::ReferencedFolderSearch(search)) = state.overlay.as_mut() else {
        return false;
    };
    if search.request != Some(id) {
        return false;
    }
    search.loading = false;
    search.failed = true;
    search.entries.clear();
    search.items.clear();
    search.scan_truncated = false;
    search.truncated = false;
    true
}

/// 搜索弹窗重新筛选：输入变化后按查询重算候选并复位高亮。
pub fn refresh_referenced_search(state: &mut AppState) -> bool {
    let Some(Overlay::ReferencedFolderSearch(search)) = state.overlay.as_mut() else {
        return false;
    };
    let query = search.query.text().to_string();
    search.refilter(&query);
    true
}

/// 搜索弹窗上下移动高亮；越界钳制不循环；键盘移动清空显式视口，窗口重新跟随高亮。
pub fn move_referenced_search_selection(state: &mut AppState, delta: isize) -> bool {
    let Some(Overlay::ReferencedFolderSearch(search)) = state.overlay.as_mut() else {
        return false;
    };
    if search.items.is_empty() {
        return false;
    }
    let next =
        (search.selected as isize + delta).clamp(0, search.items.len() as isize - 1) as usize;
    if next == search.selected {
        return false;
    }
    search.selected = next;
    search.viewport = None;
    true
}

/// 搜索弹窗滚轮滚动视口：显式视口从当前窗口起偏移、越界钳制不循环；高亮不动。
///
/// `base` 为当前窗口起点、`visible` 为可视行数；返回视口是否移动。
pub fn scroll_referenced_search(
    state: &mut AppState,
    delta: isize,
    base: usize,
    visible: usize,
) -> bool {
    let Some(Overlay::ReferencedFolderSearch(search)) = state.overlay.as_mut() else {
        return false;
    };
    let max_offset = search.items.len().saturating_sub(visible);
    let current = search.viewport.unwrap_or(base).min(max_offset);
    let next = (current as isize + delta).clamp(0, max_offset as isize) as usize;
    if next == current && search.viewport.is_some() {
        return false;
    }
    search.viewport = Some(next);
    true
}

/// 设置搜索弹窗列表的显式视口起点（滚动条轨道/拖拽）；越界钳制；返回是否变化。
pub fn set_referenced_search_viewport(state: &mut AppState, offset: usize, visible: usize) -> bool {
    let Some(Overlay::ReferencedFolderSearch(search)) = state.overlay.as_mut() else {
        return false;
    };
    let max_offset = search.items.len().saturating_sub(visible);
    let next = offset.min(max_offset);
    if search.viewport == Some(next) {
        return false;
    }
    search.viewport = Some(next);
    true
}

/// 搜索弹窗鼠标选中项；越界忽略。
pub fn set_referenced_search_selection(state: &mut AppState, index: usize) -> bool {
    let Some(Overlay::ReferencedFolderSearch(search)) = state.overlay.as_mut() else {
        return false;
    };
    if index >= search.items.len() || search.selected == index {
        return false;
    }
    search.selected = index;
    true
}

/// 复制高亮项的 Markdown 引用（目录 `refer-folder`、文件 `refer-file`）。
pub fn copy_referenced_entry_reference(state: &mut AppState) -> bool {
    let text_value = {
        let Some(Overlay::ReferencedFolderSearch(search)) = state.overlay.as_ref() else {
            return false;
        };
        let Some(entry) = search.items.get(search.selected) else {
            return false;
        };
        let kind = if entry.is_directory {
            ReferKind::Folder
        } else {
            ReferKind::File
        };
        crate::app::markdown::format_reference(kind, &entry.path)
    };
    let Some(text_value) = text_value else {
        return false;
    };
    state.effects.push(Effect::CopyToClipboard {
        text: text_value,
        anchor: None,
    });
    true
}

/// 复制当前搜索根（原始绝对路径，非 Markdown 引用）。
pub fn copy_referenced_root_path(state: &mut AppState) -> bool {
    let Some(Overlay::ReferencedFolderSearch(search)) = state.overlay.as_ref() else {
        return false;
    };
    let path = search.root.to_string_lossy().into_owned();
    state.effects.push(Effect::CopyToClipboard {
        text: path,
        anchor: None,
    });
    true
}

impl ReferencedFolderSearch {
    /// 写入扫描结果并重算候选。
    pub fn entries_loaded(&mut self, outcome: ReferencedScanOutcome) {
        self.entries = outcome.entries;
        self.items.clear();
        self.truncated = false;
        self.selected = 0;
        let query = self.query.text().to_string();
        self.refilter(&query);
    }

    /// 按查询重算候选（先名称后路径，稳定排序），并复位高亮与显式视口。
    fn refilter(&mut self, query: &str) {
        let (items, overflow) = crate::app::referenced::filter_entries(&self.entries, query);
        self.items = items;
        self.truncated = self.scan_truncated || overflow;
        self.selected = 0;
        self.viewport = None;
    }
}

/// 搜索弹窗搜索框的单行编辑键；编辑后重算候选。
pub fn edit_referenced_search(state: &mut AppState, key: crate::app::actions::OverlayKey) -> bool {
    let Some(Overlay::ReferencedFolderSearch(search)) = state.overlay.as_mut() else {
        return false;
    };
    if !search.query.apply_key(key) {
        return false;
    }
    let query = search.query.text().to_string();
    search.refilter(&query);
    true
}

/// 关闭搜索弹窗并回目录弹窗（保留记录与磁盘内容）。
pub fn referenced_search_escape(state: &mut AppState) -> bool {
    let Some(Overlay::ReferencedFolderSearch(search)) = state.overlay.as_ref() else {
        return false;
    };
    let anchor = search.anchor;
    open_referenced_folder_menu(state, anchor)
}
