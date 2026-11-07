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
