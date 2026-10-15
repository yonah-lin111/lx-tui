//! 引用文件夹行为单元测试：菜单、路径输入校验回投、搜索弹窗与删除确认。

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use super::*;
use crate::app::overlay::{Overlay, ReferencedFolderMenuRow};
use crate::app::referenced::{
    PathValidation, ReferencedEntry, ReferencedFolder, ReferencedScanOutcome,
};

/// 建一个真实存在的临时目录（内容搜索需要根目录可访问）。
fn temp_dir(label: &str) -> PathBuf {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or(0);
    let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("lx-ref-{label}-{nanos:x}-{sequence:x}"));
    std::fs::create_dir_all(&path).expect("temp dir");
    path
}

/// demo 状态并绑定一个固定的 prompt 键与两条真实引用记录。
fn state_with_folders() -> AppState {
    let alpha = temp_dir("alpha");
    let beta = temp_dir("beta");
    let mut state = AppState::demo();
    state.active_workspace_mut().cwd = Some(PathBuf::from("/tmp/lx-referenced-test"));
    let key = state.referenced_key().expect("prompt key");
    state.referenced.apply(
        key,
        vec![
            ReferencedFolder { id: 1, path: alpha },
            ReferencedFolder { id: 2, path: beta },
        ],
        false,
    );
    state
}

/// 快照中 `id` 记录的根路径。
fn folder_path(state: &AppState, id: i64) -> PathBuf {
    state
        .referenced
        .folders
        .iter()
        .find(|folder| folder.id == id)
        .map(|folder| folder.path.clone())
        .expect("folder")
}

fn entry(root: &std::path::Path, relative: &str, is_directory: bool) -> ReferencedEntry {
    let name = relative.rsplit('/').next().unwrap_or(relative).to_string();
    ReferencedEntry {
        path: root.join(relative),
        relative: relative.to_string(),
        name,
        is_directory,
    }
}

#[test]
fn menu_lists_records_in_order_with_add_last() {
    let mut state = state_with_folders();
    assert!(open_referenced_folder_menu(&mut state, (5, 5)));
    let Some(Overlay::ReferencedFolderMenu(menu)) = state.overlay.as_ref() else {
        panic!("menu opened");
    };
    assert_eq!(menu.rows.len(), 3);
    assert!(matches!(
        menu.rows[0],
        ReferencedFolderMenuRow::Folder { id: 1, .. }
    ));
    assert!(matches!(
        menu.rows[1],
        ReferencedFolderMenuRow::Folder { id: 2, .. }
    ));
    assert!(matches!(menu.rows[2], ReferencedFolderMenuRow::Add));
    assert_eq!(menu.selected, 1, "默认高亮最后一条记录");
}

#[test]
fn menu_shows_empty_state_without_records() {
    let mut state = AppState::demo();
    state.active_workspace_mut().cwd = Some(PathBuf::from("/tmp/lx-referenced-empty"));
    let key = state.referenced_key().expect("key");
    state.referenced.apply(key, Vec::new(), false);
    assert!(open_referenced_folder_menu(&mut state, (0, 0)));
    let Some(Overlay::ReferencedFolderMenu(menu)) = state.overlay.as_ref() else {
        panic!("menu");
    };
    assert!(matches!(menu.rows[0], ReferencedFolderMenuRow::Empty));
    assert!(matches!(menu.rows[1], ReferencedFolderMenuRow::Add));
    assert_eq!(menu.selected, 0);
}

#[test]
fn menu_shows_unavailable_state_when_db_failed() {
    let mut state = AppState::demo();
    state.active_workspace_mut().cwd = Some(PathBuf::from("/tmp/lx-referenced-failed"));
    let key = state.referenced_key().expect("key");
    state.referenced.apply(key, Vec::new(), true);
    assert!(open_referenced_folder_menu(&mut state, (0, 0)));
    let Some(Overlay::ReferencedFolderMenu(menu)) = state.overlay.as_ref() else {
        panic!("menu");
    };
    assert!(matches!(menu.rows[0], ReferencedFolderMenuRow::Unavailable));
    assert!(matches!(menu.rows[1], ReferencedFolderMenuRow::Add));
}

#[test]
fn menu_requires_project_key_and_editor_view() {
    let mut state = AppState::demo();
    state.active_workspace_mut().cwd = None;
    assert!(!open_referenced_folder_menu(&mut state, (0, 0)));
    assert!(state.overlay.is_none());

    // 树视图下不打开。
    let mut state = state_with_folders();
    state.workspaces[0].prompt_panel.mode = crate::app::prompt::PromptMode::Tree;
    assert!(!open_referenced_folder_menu(&mut state, (0, 0)));
    assert!(state.overlay.is_none());
}

#[test]
fn activating_add_opens_input_and_activating_record_opens_search() {
    let mut state = state_with_folders();
    open_referenced_folder_menu(&mut state, (5, 5));
    // 高亮记录行索引 1（beta）。
    state_menu_select(&mut state, 1);
    assert!(activate_referenced_menu_row(&mut state));
    assert!(matches!(
        state.overlay,
        Some(Overlay::ReferencedFolderSearch(_))
    ));
    assert_eq!(state.referenced_requests.len(), 1, "打开时登记一次扫描");

    // `[add]` 行打开输入弹窗。
    let mut state = state_with_folders();
    open_referenced_folder_menu(&mut state, (5, 5));
    state_menu_select(&mut state, 2);
    assert!(activate_referenced_menu_row(&mut state));
    assert!(matches!(
        state.overlay,
        Some(Overlay::ReferencedFolderInput(_))
    ));
}

/// 设置菜单高亮到指定行。
fn state_menu_select(state: &mut AppState, index: usize) {
    set_referenced_menu_selection(state, index);
}

#[test]
fn empty_input_is_rejected_locally_without_background_request() {
    let mut state = state_with_folders();
    assert!(open_referenced_folder_input(&mut state, (0, 0)));
    submit_referenced_folder_input(&mut state);
    assert!(state.referenced_requests.is_empty(), "空值本地拒绝");
    let Some(Overlay::ReferencedFolderInput(input)) = state.overlay.as_ref() else {
        panic!("input open");
    };
    assert_eq!(input.error, Some(text::REFERENCED_ERROR_EMPTY));
    assert!(!input.validating);
}

#[test]
fn valid_input_registers_background_validation_and_blocks_double_submit() {
    let mut state = state_with_folders();
    open_referenced_folder_input(&mut state, (3, 4));
    if let Some(Overlay::ReferencedFolderInput(input)) = state.overlay.as_mut() {
        input.input.insert_str("/tmp/new");
    }
    submit_referenced_folder_input(&mut state);
    assert_eq!(state.referenced_requests.len(), 1);
    let Some(Overlay::ReferencedFolderInput(input)) = state.overlay.as_ref() else {
        panic!("input open");
    };
    assert!(input.validating);
    // 校验中重复提交被忽略。
    submit_referenced_folder_input(&mut state);
    assert_eq!(state.referenced_requests.len(), 1);
}

#[test]
fn validation_success_emits_add_effect_and_keeps_anchor() {
    let mut state = state_with_folders();
    open_referenced_folder_input(&mut state, (7, 8));
    if let Some(Overlay::ReferencedFolderInput(input)) = state.overlay.as_mut() {
        input.input.insert_str("/tmp/new");
        input.request = Some(42);
        input.validating = true;
    }
    assert!(apply_referenced_validation(
        &mut state,
        42,
        PathBuf::from("/tmp/new")
    ));
    assert!(matches!(
        state.effects.as_slice(),
        [Effect::AddReferencedFolder { path, anchor, .. }]
            if path == "/tmp/new" && *anchor == (7, 8)
    ));
    assert!(
        matches!(state.overlay, Some(Overlay::ReferencedFolderInput(_))),
        "写成功前弹窗保留（服务关窗）"
    );
}

#[test]
fn stale_or_mismatched_validation_is_discarded() {
    let mut state = state_with_folders();
    open_referenced_folder_input(&mut state, (0, 0));
    if let Some(Overlay::ReferencedFolderInput(input)) = state.overlay.as_mut() {
        input.request = Some(1);
        input.validating = true;
    }
    assert!(
        !apply_referenced_validation(&mut state, 2, PathBuf::from("/tmp/x")),
        "迟到回执丢弃"
    );
    assert!(state.effects.is_empty());
    assert!(
        !apply_referenced_validation_error(&mut state, 99, PathValidation::NotFound),
        "不匹配的错误丢弃"
    );
    assert!(apply_referenced_validation_error(
        &mut state,
        1,
        PathValidation::NotFound
    ));
    let Some(Overlay::ReferencedFolderInput(input)) = state.overlay.as_ref() else {
        panic!("input open");
    };
    assert_eq!(input.error, Some(text::REFERENCED_ERROR_NOT_FOUND));
    assert!(!input.validating);
}

#[test]
fn finish_add_closes_input_and_reopens_menu_with_new_record() {
    let mut state = state_with_folders();
    open_referenced_folder_input(&mut state, (2, 2));
    let key = state.referenced_key().expect("key");
    state.referenced.apply(
        key,
        vec![
            ReferencedFolder {
                id: 1,
                path: PathBuf::from("/tmp/alpha"),
            },
            ReferencedFolder {
                id: 2,
                path: PathBuf::from("/tmp/beta"),
            },
            ReferencedFolder {
                id: 3,
                path: PathBuf::from("/tmp/gamma"),
            },
        ],
        false,
    );
    assert!(finish_referenced_add(&mut state, (2, 2)));
    let Some(Overlay::ReferencedFolderMenu(menu)) = state.overlay.as_ref() else {
        panic!("menu reopened");
    };
    assert_eq!(menu.rows.len(), 4);
    assert_eq!(menu.selected, 2, "高亮滚动到新增记录");
    assert_eq!(menu.anchor, (2, 2));
}

#[test]
fn search_scan_results_filter_and_enter_and_back() {
    let mut state = state_with_folders();
    let alpha = folder_path(&state, 1);
    open_referenced_folder_menu(&mut state, (5, 5));
    state_menu_select(&mut state, 0);
    assert!(activate_referenced_menu_row(&mut state));
    let id = scan_request(&state);
    let outcome = ReferencedScanOutcome {
        entries: vec![
            entry(&alpha, "src/app.rs", false),
            entry(&alpha, "src", true),
            entry(&alpha, "README.md", false),
        ],
        truncated: false,
    };
    assert!(apply_referenced_scan(&mut state, id, outcome));
    let Some(Overlay::ReferencedFolderSearch(search)) = state.overlay.as_ref() else {
        panic!("search");
    };
    assert!(!search.loading);
    assert_eq!(search.items.len(), 3);

    // 输入过滤。
    if let Some(Overlay::ReferencedFolderSearch(search)) = state.overlay.as_mut() {
        search.query.insert_str("app");
    }
    refresh_referenced_search(&mut state);
    let Some(Overlay::ReferencedFolderSearch(search)) = state.overlay.as_ref() else {
        panic!("search");
    };
    assert_eq!(search.items.len(), 1);
    assert_eq!(search.items[0].relative, "src/app.rs");

    // 清空查询后进入目录 `src`。
    if let Some(Overlay::ReferencedFolderSearch(search)) = state.overlay.as_mut() {
        search.query.clear();
    }
    refresh_referenced_search(&mut state);
    let src_index = match state.overlay.as_ref() {
        Some(Overlay::ReferencedFolderSearch(search)) => search
            .items
            .iter()
            .position(|entry| entry.relative == "src")
            .expect("src entry"),
        _ => panic!("search"),
    };
    set_referenced_search_selection(&mut state, src_index);
    assert!(referenced_search_enter(&mut state));
    let Some(Overlay::ReferencedFolderSearch(search)) = state.overlay.as_ref() else {
        panic!("search");
    };
    assert_eq!(search.root, alpha.join("src"));
    assert_eq!(search.stack, vec![alpha.clone()]);
    assert!(search.query.text().is_empty(), "进入目录清空查询");
    assert_eq!(state.referenced_requests.len(), 2, "进入子目录重扫");

    // Ctrl+Z 回退到初始根。
    if let Some(Overlay::ReferencedFolderSearch(search)) = state.overlay.as_mut() {
        search.request = Some(100);
    }
    assert!(referenced_search_back(&mut state));
    let Some(Overlay::ReferencedFolderSearch(search)) = state.overlay.as_ref() else {
        panic!("search");
    };
    assert_eq!(search.root, alpha);
    assert!(search.stack.is_empty());
    // 已在根再回退为 no-op。
    assert!(!referenced_search_back(&mut state));
}

#[test]
fn entering_highlighted_file_is_ignored() {
    let mut state = state_with_folders();
    let alpha = folder_path(&state, 1);
    open_referenced_folder_search(&mut state, 1, alpha.clone(), (0, 0));
    let id = scan_request(&state);
    apply_referenced_scan(
        &mut state,
        id,
        ReferencedScanOutcome {
            entries: vec![entry(&alpha, "file.rs", false)],
            truncated: false,
        },
    );
    assert!(
        !referenced_search_enter(&mut state),
        "文件 Shift+Enter 无动作"
    );
    assert!(state.referenced_requests.len() <= 1);
}

#[test]
fn stale_scan_result_is_discarded() {
    let mut state = state_with_folders();
    let alpha = folder_path(&state, 1);
    open_referenced_folder_search(&mut state, 1, alpha.clone(), (0, 0));
    let id = scan_request(&state);
    assert!(
        !apply_referenced_scan(
            &mut state,
            id + 1,
            ReferencedScanOutcome {
                entries: vec![entry(&alpha, "x.rs", false)],
                truncated: false,
            },
        ),
        "代号不匹配丢弃"
    );
    let Some(Overlay::ReferencedFolderSearch(search)) = state.overlay.as_ref() else {
        panic!("search");
    };
    assert!(search.loading, "过期结果不改状态");
}

#[test]
fn scan_failure_marks_inaccessible_root() {
    let mut state = state_with_folders();
    let alpha = folder_path(&state, 1);
    open_referenced_folder_search(&mut state, 1, alpha, (0, 0));
    let id = scan_request(&state);
    assert!(apply_referenced_scan_failure(&mut state, id));
    let Some(Overlay::ReferencedFolderSearch(search)) = state.overlay.as_ref() else {
        panic!("search");
    };
    assert!(search.failed);
    assert!(!search.loading);
}

#[test]
fn missing_directory_scan_failure_marks_dialog_failed() {
    let mut state = state_with_folders();
    assert!(open_referenced_folder_search(
        &mut state,
        1,
        PathBuf::from("/definitely/missing/lx-path"),
        (0, 0)
    ));
    // 打开即登记后台扫描；磁盘可用性由后台判定（app 层不执行 IO）。
    let id = scan_request(&state);
    assert!(apply_referenced_scan_failure(&mut state, id));
    let Some(Overlay::ReferencedFolderSearch(search)) = state.overlay.as_ref() else {
        panic!("search");
    };
    assert!(search.failed);
    assert!(!search.loading);
}

#[test]
fn copy_reference_uses_refer_file_and_refer_folder_syntax() {
    let mut state = state_with_folders();
    let alpha = folder_path(&state, 1);
    open_referenced_folder_search(&mut state, 1, alpha.clone(), (0, 0));
    let id = scan_request(&state);
    apply_referenced_scan(
        &mut state,
        id,
        ReferencedScanOutcome {
            entries: vec![entry(&alpha, "dir", true), entry(&alpha, "file.rs", false)],
            truncated: false,
        },
    );
    // 高亮目录。
    let dir_index = match state.overlay.as_ref() {
        Some(Overlay::ReferencedFolderSearch(search)) => search
            .items
            .iter()
            .position(|entry| entry.is_directory)
            .expect("dir"),
        _ => panic!("search"),
    };
    set_referenced_search_selection(&mut state, dir_index);
    assert!(copy_referenced_entry_reference(&mut state));
    // 高亮文件。
    let file_index = match state.overlay.as_ref() {
        Some(Overlay::ReferencedFolderSearch(search)) => search
            .items
            .iter()
            .position(|entry| entry.relative == "file.rs")
            .expect("file"),
        _ => panic!("search"),
    };
    set_referenced_search_selection(&mut state, file_index);
    assert!(copy_referenced_entry_reference(&mut state));
    assert!(copy_referenced_root_path(&mut state));
    let texts: Vec<&str> = state
        .effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::CopyToClipboard(text) => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(texts.len(), 3);
    assert!(texts[0].starts_with("@[refer-folder]("), "{}", texts[0]);
    assert!(texts[0].contains(&format!("{}/dir", alpha.display())));
    assert!(texts[1].starts_with("@[refer-file]("), "{}", texts[1]);
    assert!(texts[1].contains(&format!("{}/file.rs", alpha.display())));
    assert_eq!(
        texts[2],
        alpha.display().to_string(),
        "标题按钮复制原始绝对路径"
    );
}

/// 当前搜索弹窗的在途扫描代号。
fn scan_request(state: &AppState) -> u64 {
    match state.overlay.as_ref() {
        Some(Overlay::ReferencedFolderSearch(search)) => search.request.expect("request"),
        _ => panic!("search open"),
    }
}

#[test]
fn menu_double_click_detects_same_row_within_window() {
    let mut state = state_with_folders();
    let t0 = std::time::Instant::now();
    assert!(!referenced_menu_double_click(&mut state, 1, t0));
    assert!(!referenced_menu_double_click(
        &mut state,
        2,
        t0 + std::time::Duration::from_millis(50)
    ));
    assert!(referenced_menu_double_click(
        &mut state,
        2,
        t0 + std::time::Duration::from_millis(100)
    ));
    assert!(
        !referenced_menu_double_click(
            &mut state,
            2,
            t0 + std::time::Duration::from_millis(100)
                + crate::app::update::PROMPT_DOUBLE_CLICK_WINDOW
                + std::time::Duration::from_millis(1)
        ),
        "超出双击窗口不算双击"
    );
}

#[test]
fn menu_copy_uses_refer_folder_syntax() {
    let mut state = state_with_folders();
    let beta = folder_path(&state, 2);
    open_referenced_folder_menu(&mut state, (0, 0));
    let index = menu_row_index(&state, 2);
    assert!(set_referenced_menu_selection(&mut state, index));
    assert!(copy_referenced_menu_selected(&mut state));
    let expected = format!("@[refer-folder]({})", beta.display());
    assert!(matches!(
        state.effects.as_slice(),
        [Effect::CopyToClipboard(text)] if *text == expected
    ));
}

#[test]
fn menu_remove_opens_confirmation_only_for_folder_rows() {
    let mut state = state_with_folders();
    let alpha = folder_path(&state, 1);
    open_referenced_folder_menu(&mut state, (0, 0));
    let index = menu_row_index(&state, 1);
    assert!(set_referenced_menu_selection(&mut state, index));
    assert!(remove_referenced_menu_selected(&mut state));
    assert!(matches!(
        state.overlay,
        Some(Overlay::ReferencedFolderDelete(delete)) if delete.id == 1 && delete.path == alpha
    ));
}

#[test]
fn menu_copy_and_remove_ignore_state_and_add_rows() {
    let mut state = AppState::demo();
    open_referenced_folder_menu(&mut state, (0, 0));
    // 空状态行与 `+ add folder…` 行都不产生复制/删除。
    assert!(set_referenced_menu_selection(&mut state, 0));
    assert!(!copy_referenced_menu_selected(&mut state));
    assert!(!remove_referenced_menu_selected(&mut state));
    let add_index = menu_row_index(&state, 0);
    assert!(set_referenced_menu_selection(&mut state, add_index));
    assert!(!copy_referenced_menu_selected(&mut state));
    assert!(!remove_referenced_menu_selected(&mut state));
}

/// 目录弹窗中指定记录 id 所在行下标；未找到时回退 `[add]` 行（末行）。
fn menu_row_index(state: &AppState, id: i64) -> usize {
    let Some(Overlay::ReferencedFolderMenu(menu)) = state.overlay.as_ref() else {
        panic!("menu open");
    };
    menu.rows
        .iter()
        .position(|row| matches!(row, ReferencedFolderMenuRow::Folder { id: row_id, .. } if *row_id == id))
        .unwrap_or_else(|| menu.rows.len().saturating_sub(1))
}

#[test]
fn delete_confirmation_emits_effect_and_escape_cancels() {
    let mut state = state_with_folders();
    let alpha = folder_path(&state, 1);
    open_referenced_folder_menu(&mut state, (0, 0));
    assert!(open_referenced_delete(&mut state, 1, alpha.clone()));
    assert!(matches!(
        state.overlay,
        Some(Overlay::ReferencedFolderDelete(_))
    ));
    // 取消：关闭浮层、无效应。
    crate::app::update::close_overlay(&mut state);
    assert!(state.effects.is_empty());
    assert!(state.overlay.is_none());

    // 重新打开并确认。
    open_referenced_delete(&mut state, 1, alpha);
    assert!(confirm_referenced_delete(&mut state));
    assert!(matches!(
        state.effects.as_slice(),
        [Effect::DeleteReferencedFolder { id: 1, .. }]
    ));
    assert!(state.overlay.is_none());
}

#[test]
fn invalidate_search_after_delete_closes_matching_dialog_only() {
    let mut state = state_with_folders();
    let alpha = folder_path(&state, 1);
    open_referenced_folder_search(&mut state, 1, alpha, (0, 0));
    assert!(
        !invalidate_referenced_search(&mut state, 2),
        "不相关记录不影响"
    );
    assert!(matches!(
        state.overlay,
        Some(Overlay::ReferencedFolderSearch(_))
    ));
    assert!(invalidate_referenced_search(&mut state, 1));
    assert!(state.overlay.is_none());
}

#[test]
fn invalidate_overlays_drops_all_referenced_dialogs() {
    let mut state = state_with_folders();
    open_referenced_folder_menu(&mut state, (0, 0));
    assert!(invalidate_referenced_overlays(&mut state));
    assert!(state.overlay.is_none());
    // 非引用文件夹浮层不受影响。
    let mut state = state_with_folders();
    state.overlay = Some(Overlay::NewWorkspace(crate::app::overlay::NewWorkspace {
        input: crate::app::overlay::TextInput::new(""),
    }));
    assert!(!invalidate_referenced_overlays(&mut state));
    assert!(state.overlay.is_some());
}

#[test]
fn search_escape_returns_to_menu() {
    let mut state = state_with_folders();
    let alpha = folder_path(&state, 1);
    open_referenced_folder_search(&mut state, 1, alpha, (9, 9));
    assert!(referenced_search_escape(&mut state));
    let Some(Overlay::ReferencedFolderMenu(menu)) = state.overlay.as_ref() else {
        panic!("menu");
    };
    assert_eq!(menu.anchor, (9, 9));
    assert_eq!(menu.rows.len(), 3);
}

#[test]
fn scroll_search_viewport_clamps_and_keeps_selection() {
    let mut state = state_with_folders();
    let alpha = folder_path(&state, 1);
    open_referenced_folder_search(&mut state, 1, alpha.clone(), (0, 0));
    let id = scan_request(&state);
    let entries: Vec<ReferencedEntry> = (0..30)
        .map(|index| entry(&alpha, &format!("f{index:02}.rs"), false))
        .collect();
    apply_referenced_scan(
        &mut state,
        id,
        ReferencedScanOutcome {
            entries,
            truncated: false,
        },
    );
    let selected = match state.overlay.as_ref() {
        Some(Overlay::ReferencedFolderSearch(search)) => search.selected,
        _ => panic!("search"),
    };
    assert!(scroll_referenced_search(&mut state, 1, 0, 5));
    assert!(scroll_referenced_search(&mut state, 1, 1, 5));
    let Some(Overlay::ReferencedFolderSearch(search)) = state.overlay.as_ref() else {
        panic!("search");
    };
    assert_eq!(search.selected, selected, "滚轮不动高亮");
    assert!(search.viewport.is_some());
}

#[test]
fn scroll_menu_viewport_clamps_and_keeps_selection() {
    let mut state = state_with_folders();
    open_referenced_folder_menu(&mut state, (0, 0));
    let selected = match state.overlay.as_ref() {
        Some(Overlay::ReferencedFolderMenu(menu)) => menu.selected,
        _ => panic!("menu"),
    };
    // 两条记录 + 新建行：可视窗口 2 行时列表溢出。
    assert!(scroll_referenced_menu(&mut state, 1, 0, 2));
    let Some(Overlay::ReferencedFolderMenu(menu)) = state.overlay.as_ref() else {
        panic!("menu");
    };
    assert_eq!(menu.selected, selected, "滚轮不动高亮");
    assert_eq!(menu.viewport, Some(1));
    // 越界钳制不循环：到底后再滚不移动、返回 false。
    assert!(!scroll_referenced_menu(&mut state, 1, 1, 2));
    assert!(scroll_referenced_menu(&mut state, -1, 1, 2));
    let Some(Overlay::ReferencedFolderMenu(menu)) = state.overlay.as_ref() else {
        panic!("menu");
    };
    assert_eq!(menu.viewport, Some(0));
    assert!(!scroll_referenced_menu(&mut state, -1, 0, 2));
}

#[test]
fn menu_keyboard_move_clears_viewport_and_snaps_window() {
    let mut state = state_with_folders();
    open_referenced_folder_menu(&mut state, (0, 0));
    assert!(scroll_referenced_menu(&mut state, 1, 0, 2));
    // 键盘移动高亮：清空显式视口，窗口重新跟随高亮。
    assert!(move_referenced_menu_selection(&mut state, -1));
    let Some(Overlay::ReferencedFolderMenu(menu)) = state.overlay.as_ref() else {
        panic!("menu");
    };
    assert_eq!(menu.viewport, None, "键盘移动后窗口跟随高亮");
    // 已到顶且高亮未动：仍须吸收滚轮滚开的窗口（一次性吸回并返回 true）。
    assert!(scroll_referenced_menu(&mut state, 1, 0, 2));
    assert!(move_referenced_menu_selection(&mut state, -1));
    let Some(Overlay::ReferencedFolderMenu(menu)) = state.overlay.as_ref() else {
        panic!("menu");
    };
    assert_eq!(menu.viewport, None);
    assert_eq!(menu.selected, 0, "已在首项，高亮不动但窗口吸回");
}

#[test]
fn set_menu_viewport_clamps_and_reports_change() {
    let mut state = state_with_folders();
    open_referenced_folder_menu(&mut state, (0, 0));
    assert!(set_referenced_menu_viewport(&mut state, 99, 2));
    let Some(Overlay::ReferencedFolderMenu(menu)) = state.overlay.as_ref() else {
        panic!("menu");
    };
    assert_eq!(menu.viewport, Some(1), "越界钳到末窗口");
    assert!(
        !set_referenced_menu_viewport(&mut state, 1, 2),
        "相同值不重复"
    );
}

#[test]
fn path_validation_matches_expected_reasons() {
    use crate::app::referenced::PathValidation;
    // 借助后台校验函数语义：此处只断言错误枚举映射到文案。
    assert_eq!(
        text::referenced_path_error(&PathValidation::NotAbsolute),
        Some(text::REFERENCED_ERROR_NOT_ABSOLUTE)
    );
    assert_eq!(
        text::referenced_path_error(&PathValidation::Duplicate),
        Some(text::REFERENCED_ERROR_DUPLICATE)
    );
}

#[test]
fn referenced_duplicate_detects_canonical_match() {
    let state = state_with_folders();
    let alpha = folder_path(&state, 1);
    assert!(referenced_duplicate(&state, &alpha));
    assert!(!referenced_duplicate(
        &state,
        std::path::Path::new("/tmp/none")
    ));
}
