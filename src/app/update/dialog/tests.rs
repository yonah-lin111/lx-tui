//! 浮层输入框粘贴/剪切共享路径测试。

use super::*;
use crate::app::overlay::*;
use crate::app::state::{AppState, ConnectPanel};
use crate::config::ai::connect::ConnectSettings;
use std::path::PathBuf;

fn text_overlays(state: &AppState) -> Vec<Overlay> {
    let pane = state.active_tab().layout.focus();
    let root = PathBuf::from("/repo");
    vec![
        Overlay::Rename(Rename {
            target: RenameTarget::Workspace(0),
            input: TextInput::new(""),
        }),
        Overlay::NewWorkspace(NewWorkspace {
            input: TextInput::new(""),
        }),
        Overlay::NewNode(NewNode {
            kind: NewNodeKind::Folder { parent_id: None },
            name: TextInput::new(""),
            resume: None,
        }),
        Overlay::SaveDraft(SaveDraft {
            workspace: 0,
            name: TextInput::new(""),
            entries: vec![SaveDraftEntry::Root],
            selected: 0,
        }),
        Overlay::WorktreeOpen(WorktreeOpen {
            source: 0,
            repo_root: root.clone(),
            entries: Vec::new(),
            selected: 0,
            query: TextInput::new(""),
            loading: false,
            failed: false,
        }),
        Overlay::ReferencedFolderInput(ReferencedFolderInput {
            anchor: (0, 0),
            input: TextInput::new(""),
            key: None,
            validating: false,
            error: None,
            request: None,
        }),
        Overlay::ReferencedFolderSearch(ReferencedFolderSearch {
            anchor: (0, 0),
            folder: 1,
            key: None,
            root: root.clone(),
            initial_root: root,
            stack: Vec::new(),
            query: TextInput::new(""),
            entries: Vec::new(),
            items: Vec::new(),
            scan_truncated: false,
            truncated: false,
            selected: 0,
            viewport: None,
            loading: false,
            failed: false,
            request: None,
        }),
        Overlay::ResumeSessionEdit(ResumeSessionEdit {
            pane,
            workspace: PathBuf::from("/repo"),
            session_id: "sess".into(),
            input: TextInput::new(""),
            confirming_delete: false,
            hint: None,
        }),
        Overlay::Connect(Box::new(ConnectPanel::new_with_env(
            pane,
            &ConnectSettings::default(),
            |_| None,
        ))),
    ]
}

#[test]
fn every_text_overlay_accepts_terminal_paste() {
    let mut state = AppState::demo();
    for overlay in text_overlays(&state) {
        let kind = overlay.kind();
        state.overlay = Some(overlay);
        assert!(
            apply_overlay_paste(&mut state, "pasted"),
            "{kind:?} 应接受粘贴"
        );
        assert_eq!(
            state
                .overlay
                .as_ref()
                .and_then(Overlay::input)
                .unwrap()
                .text(),
            "pasted",
            "{kind:?}"
        );
    }
}

#[test]
fn overlay_paste_respects_guard_states() {
    let mut state = AppState::demo();
    let guarded = [
        Overlay::ReferencedFolderInput(ReferencedFolderInput {
            anchor: (0, 0),
            input: TextInput::new(""),
            key: None,
            validating: true,
            error: None,
            request: None,
        }),
        Overlay::ResumeSessionEdit(ResumeSessionEdit {
            pane: state.active_tab().layout.focus(),
            workspace: PathBuf::from("/repo"),
            session_id: "sess".into(),
            input: TextInput::new(""),
            confirming_delete: true,
            hint: None,
        }),
        {
            let mut panel = ConnectPanel::new_with_env(
                state.active_tab().layout.focus(),
                &ConnectSettings::default(),
                |_| None,
            );
            panel.validating = true;
            Overlay::Connect(Box::new(panel))
        },
    ];
    for overlay in guarded {
        let kind = overlay.kind();
        state.overlay = Some(overlay);
        assert!(
            !apply_overlay_paste(&mut state, "pasted"),
            "{kind:?} 受保护态应拒绝粘贴"
        );
        assert_eq!(
            state
                .overlay
                .as_ref()
                .and_then(Overlay::input)
                .unwrap()
                .text(),
            "",
            "{kind:?}"
        );
    }
}

#[test]
fn overlay_paste_normalizes_path_text_in_path_dialogs() {
    let dir = spaced_temp_dir("paste path");
    let text = dir.to_string_lossy().to_string();
    let forms = [
        text.replace(' ', "\\ "),
        format!("'{text}'"),
        format!("\"{text}\""),
        format!("file://{}", text.replace(' ', "%20")),
    ];
    for form in forms {
        let mut state = AppState::demo();
        state.overlay = Some(Overlay::NewWorkspace(NewWorkspace {
            input: TextInput::new(""),
        }));
        assert!(apply_overlay_paste(&mut state, &form), "{form}");
        assert_eq!(
            state
                .overlay
                .as_ref()
                .and_then(Overlay::input)
                .expect("input")
                .text(),
            text,
            "{form}"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn overlay_paste_normalizes_path_in_referenced_folder_input() {
    let dir = spaced_temp_dir("paste referenced");
    let text = dir.to_string_lossy().to_string();
    let mut state = AppState::demo();
    state.overlay = Some(Overlay::ReferencedFolderInput(ReferencedFolderInput {
        anchor: (0, 0),
        input: TextInput::new(""),
        key: None,
        validating: false,
        error: None,
        request: None,
    }));
    let escaped = text.replace(' ', "\\ ");
    assert!(apply_overlay_paste(&mut state, &escaped));
    let Some(Overlay::ReferencedFolderInput(input)) = state.overlay.as_ref() else {
        panic!("referenced input");
    };
    assert_eq!(input.input.text(), text);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn overlay_paste_replaces_full_selection_with_normalized_path() {
    let dir = spaced_temp_dir("paste selection");
    let text = dir.to_string_lossy().to_string();
    let mut state = AppState::demo();
    state.overlay = Some(Overlay::NewWorkspace(NewWorkspace {
        input: TextInput::new("/old path"),
    }));
    if let Some(input) = state.overlay.as_mut().and_then(Overlay::input_mut) {
        input.select_all();
    }
    let escaped = text.replace(' ', "\\ ");
    assert!(apply_overlay_paste(&mut state, &escaped));
    assert_eq!(
        state
            .overlay
            .as_ref()
            .and_then(Overlay::input)
            .expect("input")
            .text(),
        text
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// 建一个名字含空格的真实临时目录（路径粘贴规范化用例）。
fn spaced_temp_dir(label: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("lx {label} {}", std::process::id()));
    std::fs::create_dir_all(&path).expect("temp dir");
    path
}

#[test]
fn overlay_cut_copies_selection_and_is_one_undo_step() {
    let mut state = AppState::demo();
    state.overlay = Some(Overlay::Rename(Rename {
        target: RenameTarget::Workspace(0),
        input: TextInput::new("hello world"),
    }));
    {
        let input = state.overlay.as_mut().and_then(Overlay::input_mut).unwrap();
        input.set_selection(6, 11);
    }
    assert_eq!(
        cut_overlay_selected_text(&mut state).as_deref(),
        Some("world")
    );
    assert_eq!(
        state
            .overlay
            .as_ref()
            .and_then(Overlay::input)
            .unwrap()
            .text(),
        "hello "
    );
    apply_overlay_key(&mut state, OverlayKey::Undo);
    assert_eq!(
        state
            .overlay
            .as_ref()
            .and_then(Overlay::input)
            .unwrap()
            .text(),
        "hello world"
    );
}

fn search_overlay(query: &str, stack: Vec<PathBuf>) -> Overlay {
    let root = PathBuf::from("/repo");
    let mut query_input = TextInput::new("");
    for ch in query.chars() {
        query_input.insert_char(ch);
    }
    Overlay::ReferencedFolderSearch(ReferencedFolderSearch {
        anchor: (0, 0),
        folder: 1,
        key: None,
        root: root.clone(),
        initial_root: root,
        stack,
        query: query_input,
        entries: Vec::new(),
        items: Vec::new(),
        scan_truncated: false,
        truncated: false,
        selected: 0,
        viewport: None,
        loading: false,
        failed: false,
        request: None,
    })
}

#[test]
fn referenced_search_undo_prefers_query_then_directory_back() {
    let mut state = AppState::demo();
    state.overlay = Some(search_overlay("abc", vec![PathBuf::from("/parent")]));
    for expected in ["ab", "a", ""] {
        apply_overlay_key(&mut state, OverlayKey::Undo);
        let Some(Overlay::ReferencedFolderSearch(search)) = state.overlay.as_ref() else {
            panic!("search overlay");
        };
        assert_eq!(search.query.text(), expected);
    }
    // 查询撤销栈已空：Ctrl+Z 回退目录。
    apply_overlay_key(&mut state, OverlayKey::Undo);
    let Some(Overlay::ReferencedFolderSearch(search)) = state.overlay.as_ref() else {
        panic!("search overlay");
    };
    assert_eq!(search.root, PathBuf::from("/parent"));
    assert!(search.stack.is_empty());
    assert!(search.query.text().is_empty());
    // 换根后查询历史被重置：再次 Ctrl+Z 直接继续回退（栈空则不动）。
    apply_overlay_key(&mut state, OverlayKey::Undo);
    let Some(Overlay::ReferencedFolderSearch(search)) = state.overlay.as_ref() else {
        panic!("search overlay");
    };
    assert_eq!(search.root, PathBuf::from("/parent"));
}

#[test]
fn referenced_search_redo_and_cut_route_to_query() {
    let mut state = AppState::demo();
    state.overlay = Some(search_overlay("abc", Vec::new()));
    apply_overlay_key(&mut state, OverlayKey::Undo);
    apply_overlay_key(&mut state, OverlayKey::Redo);
    let Some(Overlay::ReferencedFolderSearch(search)) = state.overlay.as_ref() else {
        panic!("search overlay");
    };
    assert_eq!(search.query.text(), "abc");

    {
        let Some(Overlay::ReferencedFolderSearch(search)) = state.overlay.as_mut() else {
            panic!("search overlay");
        };
        search.query.set_selection(0, 2);
    }
    assert_eq!(cut_overlay_selected_text(&mut state).as_deref(), Some("ab"));
    let Some(Overlay::ReferencedFolderSearch(search)) = state.overlay.as_ref() else {
        panic!("search overlay");
    };
    assert_eq!(search.query.text(), "c");
}

#[test]
fn overlay_cut_keeps_whitespace_selection_text() {
    let mut state = AppState::demo();
    state.overlay = Some(Overlay::Rename(Rename {
        target: RenameTarget::Workspace(0),
        input: TextInput::new("a   b"),
    }));
    {
        let input = state.overlay.as_mut().and_then(Overlay::input_mut).unwrap();
        input.set_selection(1, 4);
    }
    assert_eq!(
        cut_overlay_selected_text(&mut state).as_deref(),
        Some("   ")
    );
    assert_eq!(
        state
            .overlay
            .as_ref()
            .and_then(Overlay::input)
            .unwrap()
            .text(),
        "ab"
    );
}

/// 建一个名字以空格结尾的真实临时目录（覆盖转义尾随空格的粘贴与提交）。
fn trailing_space_temp_dir(label: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("lx {label} {} ", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).expect("trailing space temp dir");
    path
}

#[test]
fn parse_workspace_path_accepts_escaped_quoted_and_file_uri_paths() {
    let dir = spaced_temp_dir("parse path");
    let canonical = dir.canonicalize().unwrap_or_else(|_| dir.clone());
    let text = canonical.to_string_lossy().to_string();
    let forms = [
        text.replace(' ', "\\ "),
        format!("'{text}'"),
        format!("\"{text}\""),
        format!("file://{}", text.replace(' ', "%20")),
    ];
    for form in forms {
        assert_eq!(
            parse_workspace_path(&form, None),
            Ok(Some(canonical.clone())),
            "{form}"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn parse_workspace_path_expands_tilde_with_escaped_space_and_rejects_regular_file() {
    let Some(home) = crate::app::state::home_dir() else {
        return;
    };
    let name = format!("lx home path {} with space", std::process::id());
    let dir = home.join(&name);
    std::fs::create_dir_all(&dir).expect("home temp dir");
    let input = format!("~/{}", name.replace(' ', "\\ "));
    let canonical = dir.canonicalize().unwrap_or_else(|_| dir.clone());
    assert_eq!(parse_workspace_path(&input, None), Ok(Some(canonical)));
    let _ = std::fs::remove_dir_all(&dir);

    // 普通文件即使存在也必须按“非目录”拒绝。
    let dir = spaced_temp_dir("parse file");
    let file = dir.join("file.txt");
    std::fs::write(&file, "x").expect("write test file");
    let escaped = file.to_string_lossy().replace(' ', "\\ ");
    assert_eq!(
        parse_workspace_path(&escaped, None),
        Err(crate::ui::text::TOAST_PATH_NOT_DIR)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn commit_new_workspace_normalizes_escaped_input_before_inline_confirm() {
    let dir = spaced_temp_dir("commit confirm");
    let canonical = dir.canonicalize().unwrap_or_else(|_| dir.clone());
    let escaped = canonical.to_string_lossy().replace(' ', "\\ ");

    let mut state = AppState::demo();
    crate::app::update::open_new_workspace_dialog(&mut state);
    if let Some(Overlay::NewWorkspace(dialog)) = state.overlay.as_mut() {
        dialog.input = TextInput::new(escaped);
    }
    commit_new_workspace(&mut state);

    let Some(Overlay::NewWorkspace(dialog)) = state.overlay.as_ref() else {
        panic!("dialog stays open for confirmation");
    };
    assert!(state.workspace_history.new_workspace_confirming);
    let expected = canonical.to_string_lossy().into_owned();
    assert_eq!(dialog.input.text(), expected);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn confirm_close_scroll_uses_real_visible_rows() {
    let mut state = AppState::demo();
    state.workspace_history.entries = (0..4)
        .map(|index| crate::app::state::WorkspaceHistoryEntry {
            name: format!("Missing{index}"),
            path: PathBuf::from(format!("/tmp/lx-missing-scroll-{index}-99999")),
            exists: false,
            name_is_manual: true,
            position: index,
            trusted: true,
            git: None,
        })
        .collect();
    state.overlay = Some(Overlay::ConfirmClose(ConfirmClose::new(
        OverlayTarget::MissingWorkspaces,
    )));
    let scroll = |state: &AppState| match state.overlay.as_ref() {
        Some(Overlay::ConfirmClose(confirm)) => confirm.scroll,
        _ => panic!("confirmation modal expected"),
    };

    // 默认按最大 5 行夹取：4 项全部可见，滚不动（旧行为）。
    assert!(!scroll_confirm_close(&mut state, 1));
    assert_eq!(scroll(&state), 0);

    // 小屏真实可见 2 行：尾部 2 项必须可达。
    assert!(scroll_confirm_close_with_rows(&mut state, 1, 2));
    assert!(scroll_confirm_close_with_rows(&mut state, 1, 2));
    assert_eq!(scroll(&state), 2);
    assert!(!scroll_confirm_close_with_rows(&mut state, 1, 2));
    assert!(set_confirm_close_scroll_with_rows(&mut state, 0, 2));
    assert_eq!(scroll(&state), 0);
}

#[test]
fn overlay_paste_replaces_whitespace_only_path_input() {
    let mut state = AppState::demo();
    state.overlay = Some(Overlay::NewWorkspace(NewWorkspace {
        input: TextInput::new("   "),
    }));
    assert!(apply_overlay_paste(&mut state, "/tmp"));
    assert_eq!(
        state
            .overlay
            .as_ref()
            .and_then(Overlay::input)
            .expect("input")
            .text(),
        "/tmp"
    );
}

#[test]
fn overlay_paste_ignores_empty_quoted_path() {
    for raw in ["''", "\"\""] {
        let mut state = AppState::demo();
        state.overlay = Some(Overlay::NewWorkspace(NewWorkspace {
            input: TextInput::new(""),
        }));
        assert!(!apply_overlay_paste(&mut state, raw), "{raw}");
        assert_eq!(
            state
                .overlay
                .as_ref()
                .and_then(Overlay::input)
                .expect("input")
                .text(),
            "",
            "{raw}"
        );
    }
}

#[test]
fn commit_new_workspace_confirmation_keeps_single_undo_step() {
    // 输入带 `..` 的非规范路径：解析后回填的绝对路径与输入不同，
    // 旧实现会在解析前后各回填一次，单次 Enter 产生 2 步 Undo。
    let dir = spaced_temp_dir("undo step");
    let input_path = dir.join("..");
    let escaped = input_path.to_string_lossy().replace(' ', "\\ ");

    let mut state = AppState::demo();
    crate::app::update::open_new_workspace_dialog(&mut state);
    if let Some(Overlay::NewWorkspace(dialog)) = state.overlay.as_mut() {
        dialog.input = TextInput::new(escaped.clone());
    }
    commit_new_workspace(&mut state);
    assert!(state.workspace_history.new_workspace_confirming);

    // 一次 Undo 必须直接回到提交前的转义原文。
    apply_overlay_key(&mut state, OverlayKey::Undo);
    let Some(Overlay::NewWorkspace(dialog)) = state.overlay.as_ref() else {
        panic!("dialog stays open");
    };
    assert_eq!(dialog.input.text(), escaped);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn pasted_trailing_space_directory_survives_inline_confirm() {
    let dir = trailing_space_temp_dir("paste trailing");
    let canonical = dir.canonicalize().unwrap_or_else(|_| dir.clone());
    let escaped = canonical.to_string_lossy().replace(' ', "\\ ");

    let mut state = AppState::demo();
    crate::app::update::open_new_workspace_dialog(&mut state);
    assert!(apply_overlay_paste(&mut state, &escaped));
    let expected = canonical.to_string_lossy().into_owned();
    assert_eq!(
        state
            .overlay
            .as_ref()
            .and_then(Overlay::input)
            .expect("input")
            .text(),
        expected
    );

    commit_new_workspace(&mut state);
    assert!(state.workspace_history.new_workspace_confirming);
    let Some(Overlay::NewWorkspace(dialog)) = state.overlay.as_ref() else {
        panic!("dialog stays open for confirmation");
    };
    assert_eq!(dialog.input.text(), expected);

    commit_new_workspace(&mut state);
    assert!(state.overlay.is_none());
    assert!(
        state
            .workspaces
            .iter()
            .any(|workspace| workspace.cwd.as_deref() == Some(canonical.as_path())),
        "trailing space must survive both inline confirmation steps"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
