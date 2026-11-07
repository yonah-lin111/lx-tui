//! 单元测试；仅测试构建编译。

use super::*;
use crate::app::overlay::{
    Menu, MenuCommand, Overlay, OverlayTarget, ReferencedFolderMenuRow, SaveDraft, SaveDraftEntry,
    TextInput,
};
use crate::app::referenced::ReferencedEntry;
use crate::ui::style;
use crate::ui::text;
use crate::ui::widgets;

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::style::{Color, Modifier};
use std::path::PathBuf;

const SCREEN: Rect = Rect {
    x: 0,
    y: 0,
    width: 80,
    height: 24,
};

fn draw_overlay(state: &AppState) -> (Vec<String>, Option<(u16, u16)>) {
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).expect("test backend is infallible");
    let mut cursor = None;
    if let Err(error) = terminal.draw(|frame| {
        cursor = render(frame, frame.area(), state);
    }) {
        panic!("draw failed: {error}");
    }
    let buffer = terminal.backend().buffer().clone();
    let lines = (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect()
        })
        .collect();
    (lines, cursor)
}

fn menu_state(commands: Vec<MenuCommand>, selected: usize) -> AppState {
    let mut state = AppState::demo();
    state.workspaces[0].name = "demo".into();
    state.overlay = Some(Overlay::Menu(Menu {
        anchor: (10, 5),
        target: OverlayTarget::Workspace(0),
        commands,
        selected,
        confirming: false,
    }));
    state
}

/// 模态输入行的 `> ` 前缀宽度（图标 + 空档）。
fn input_prefix_width() -> u16 {
    text::INPUT_PREFIX_ICON.chars().count() as u16 + 1
}

#[test]
fn menu_layout_maps_commands_to_text_labels() {
    let state = menu_state(
        vec![MenuCommand::RenameWorkspace, MenuCommand::CloseWorkspace],
        0,
    );
    let Some(Overlay::Menu(menu)) = state.overlay.as_ref() else {
        panic!("menu overlay expected");
    };
    let layout = menu_layout(&state, SCREEN, menu);
    assert_eq!(layout.area.x, 10);
    assert_eq!(layout.area.y, 5);
    assert_eq!(layout.item_rects.len(), 2);
}

#[test]
fn menu_renders_pure_item_list_without_title_or_divider() {
    let state = menu_state(
        vec![MenuCommand::RenameWorkspace, MenuCommand::CloseWorkspace],
        0,
    );
    let (lines, _) = draw_overlay(&state);
    assert!(lines[5].contains('┌'), "top border={:?}", lines[5]);
    assert!(
        lines[6].contains(text::MENU_RENAME_WORKSPACE),
        "first item row={:?}",
        lines[6]
    );
    assert!(
        lines[7].contains(text::MENU_CLOSE_WORKSPACE),
        "second item row={:?}",
        lines[7]
    );
    assert!(lines[8].contains('└'), "bottom border={:?}", lines[8]);
}

#[test]
fn menu_title_resolves_fallback_when_target_missing() {
    let mut state = AppState::demo();
    state.overlay = Some(Overlay::Menu(Menu {
        anchor: (10, 5),
        target: OverlayTarget::Workspace(99),
        commands: vec![MenuCommand::CloseWorkspace],
        selected: 0,
        confirming: false,
    }));
    let Some(Overlay::Menu(menu)) = state.overlay.as_ref() else {
        panic!("menu expected");
    };
    assert_eq!(menu_title(&state, menu), text::MENU_TITLE_WORKSPACE);
    let (lines, _) = draw_overlay(&state);
    assert!(lines[6].contains(text::MENU_CLOSE_WORKSPACE));
}

#[test]
fn menu_workspace_list_shows_new_workspace_label() {
    let mut state = AppState::demo();
    state.overlay = Some(Overlay::Menu(Menu {
        anchor: (10, 5),
        target: OverlayTarget::WorkspaceList,
        commands: vec![MenuCommand::NewWorkspace],
        selected: 0,
        confirming: false,
    }));
    let (lines, _) = draw_overlay(&state);
    assert!(
        lines[6].contains(text::MENU_NEW_WORKSPACE),
        "item label missing: {lines:?}"
    );
}

#[test]
fn menu_renders_labels_and_overlay_selection() {
    let mut state = menu_state(
        vec![MenuCommand::RenameWorkspace, MenuCommand::CloseWorkspace],
        1,
    );
    state.menu_hover = true;
    let (lines, cursor) = draw_overlay(&state);
    assert_eq!(cursor, None);
    assert!(lines[6].contains(text::MENU_RENAME_WORKSPACE));
    assert!(lines[7].contains(text::MENU_CLOSE_WORKSPACE));
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).expect("test backend is infallible");
    if let Err(error) = terminal.draw(|frame| {
        render(frame, frame.area(), &state);
    }) {
        panic!("draw failed: {error}");
    }
    let buffer = terminal.backend().buffer().clone();
    assert_eq!(buffer[(11, 7)].bg, Color::Indexed(240));
    assert_eq!(buffer[(11, 7)].fg, Color::Indexed(255));
    assert!(!buffer[(11, 7)].modifier.contains(Modifier::REVERSED));

    // 指针离开容器：高亮反馈整体移除。
    state.menu_hover = false;
    if let Err(error) = terminal.draw(|frame| {
        render(frame, frame.area(), &state);
    }) {
        panic!("draw failed: {error}");
    }
    let buffer = terminal.backend().buffer().clone();
    assert_eq!(
        buffer[(11, 7)].bg,
        Color::Reset,
        "离开容器后无悬停高亮且保持透明底色"
    );
}

#[test]
fn menu_labels_map_workspace_delete_and_delete_missing() {
    let mut state = menu_state(
        vec![
            MenuCommand::DeleteWorkspaceHistory,
            MenuCommand::DeleteMissingWorkspaces,
        ],
        0,
    );
    let Some(Overlay::Menu(menu)) = state.overlay.as_ref() else {
        panic!("menu overlay expected");
    };
    assert_eq!(
        menu_labels(menu),
        vec![
            text::MENU_DELETE_WORKSPACE_HISTORY,
            text::MENU_DELETE_MISSING_WORKSPACES,
        ]
    );

    if let Some(Overlay::Menu(menu)) = state.overlay.as_mut() {
        menu.confirming = true;
    }
    let Some(Overlay::Menu(menu)) = state.overlay.as_ref() else {
        panic!("menu overlay expected");
    };
    assert_eq!(
        menu_labels(menu),
        vec![
            text::MENU_CONFIRM_DELETE,
            text::MENU_DELETE_MISSING_WORKSPACES,
        ]
    );

    if let Some(Overlay::Menu(menu)) = state.overlay.as_mut() {
        menu.selected = 1;
    }
    let Some(Overlay::Menu(menu)) = state.overlay.as_ref() else {
        panic!("menu overlay expected");
    };
    assert_eq!(
        menu_labels(menu),
        vec![
            text::MENU_DELETE_WORKSPACE_HISTORY,
            text::MENU_CONFIRM_DELETE_MISSING_WORKSPACES,
        ]
    );
}

#[test]
fn no_overlay_renders_nothing() {
    let state = AppState::demo();
    let (lines, cursor) = draw_overlay(&state);
    assert_eq!(cursor, None);
    assert!(!lines.iter().any(|line| line.contains("Rename")));
}

#[test]
fn tab_menu_renders_new_rename_close_labels() {
    let state = menu_state(
        vec![
            MenuCommand::NewTab,
            MenuCommand::RenameTab,
            MenuCommand::CloseTab,
        ],
        0,
    );
    let (lines, cursor) = draw_overlay(&state);
    assert_eq!(cursor, None);
    assert!(lines[6].contains(text::MENU_NEW_TAB));
    assert!(lines[7].contains(text::MENU_RENAME_TAB));
    assert!(lines[8].contains(text::MENU_CLOSE_TAB));
}

#[test]
fn pane_menu_renders_split_switch_and_close_labels() {
    let state = menu_state(
        vec![
            MenuCommand::SplitRight,
            MenuCommand::SplitDown,
            MenuCommand::OpenPrompt,
            MenuCommand::SwitchToTerminal,
            MenuCommand::ClosePane,
        ],
        0,
    );
    let (lines, cursor) = draw_overlay(&state);
    assert_eq!(cursor, None);
    assert!(lines[6].contains(text::MENU_SPLIT_RIGHT));
    assert!(lines[7].contains(text::MENU_SPLIT_DOWN));
    assert!(lines[8].contains(text::MENU_OPEN_PROMPT));
    assert!(lines[9].contains(text::MENU_SWITCH_TO_TERMINAL));
    assert!(lines[10].contains(text::MENU_CLOSE_PANE));
}

#[test]
fn menu_labels_cover_terminal_prompt_and_workspace_path_commands() {
    let state = menu_state(
        vec![
            MenuCommand::NewTerminal,
            MenuCommand::OpenPrompt,
            MenuCommand::SwitchToWorkspaceCwd,
            MenuCommand::SyncWorkspaceToTerminalCwd,
            MenuCommand::CopyPath,
        ],
        0,
    );
    let Some(Overlay::Menu(menu)) = state.overlay.as_ref() else {
        panic!("menu overlay expected");
    };
    assert_eq!(
        menu_labels(menu),
        vec![
            text::MENU_NEW_TERMINAL,
            text::MENU_OPEN_PROMPT,
            text::MENU_SWITCH_TO_WORKSPACE_CWD,
            text::MENU_SYNC_WS_TO_TERMINAL_CWD,
            text::MENU_COPY_PATH,
        ]
    );
}

#[test]
fn workspace_menu_renders_all_labels_without_ellipsis() {
    let state = menu_state(
        vec![
            MenuCommand::NewTerminal,
            MenuCommand::OpenWorktree,
            MenuCommand::OpenPrompt,
            MenuCommand::RenameWorkspace,
            MenuCommand::CloseWorkspace,
        ],
        0,
    );
    let (lines, _) = draw_overlay(&state);
    let joined = lines.join("\n");
    for label in [
        text::MENU_NEW_TERMINAL,
        text::MENU_OPEN_WORKTREE,
        text::MENU_OPEN_PROMPT,
        text::MENU_RENAME_WORKSPACE,
        text::MENU_CLOSE_WORKSPACE,
    ] {
        assert!(joined.contains(label), "缺少 {label}: {joined}");
    }
    assert!(!joined.contains('…'), "菜单文案不得出现省略: {joined}");
}

#[test]
fn confirming_menu_item_renders_confirm_label_widens_box_and_uses_danger_style() {
    for (command, initial_label, confirm_label) in [
        (
            MenuCommand::CloseWorkspace,
            text::MENU_CLOSE_WORKSPACE,
            text::MENU_CONFIRM_CLOSE,
        ),
        (
            MenuCommand::DeletePrompt,
            text::MENU_PROMPT_DELETE,
            text::MENU_CONFIRM_DELETE,
        ),
        (
            MenuCommand::SwitchToWorkspaceCwd,
            text::MENU_SWITCH_TO_WORKSPACE_CWD,
            text::MENU_CONFIRM_SWITCH_TO_WORKSPACE_CWD,
        ),
        (
            MenuCommand::SyncWorkspaceToTerminalCwd,
            text::MENU_SYNC_WS_TO_TERMINAL_CWD,
            text::MENU_CONFIRM_SYNC_WS_TO_TERMINAL_CWD,
        ),
    ] {
        let mut state = menu_state(vec![MenuCommand::RenameWorkspace, command], 1);
        state.menu_hover = true;
        let initial_layout = {
            let Some(Overlay::Menu(menu)) = state.overlay.as_ref() else {
                panic!("menu expected");
            };
            menu_layout(&state, SCREEN, menu)
        };
        let (initial_lines, _) = draw_overlay(&state);
        let initial_joined = initial_lines.join("\n");
        assert!(initial_joined.contains(initial_label), "{initial_joined}");
        assert!(!initial_joined.contains(confirm_label), "{initial_joined}");

        // 切入行内待确认态。
        if let Some(Overlay::Menu(menu)) = state.overlay.as_mut() {
            menu.confirming = true;
        }
        let confirm_layout = {
            let Some(Overlay::Menu(menu)) = state.overlay.as_ref() else {
                panic!("menu expected");
            };
            menu_layout(&state, SCREEN, menu)
        };
        assert!(
            confirm_layout.area.width >= initial_layout.area.width,
            "菜单宽度自适应拓宽以容纳完整确认文案"
        );
        let mut terminal =
            Terminal::new(TestBackend::new(80, 24)).expect("test backend is infallible");
        terminal
            .draw(|frame| {
                render(frame, frame.area(), &state);
            })
            .expect("draw succeeds");
        let buffer = terminal.backend().buffer().clone();
        let row_rect = confirm_layout.item_rects[1];
        let row_text: String = (row_rect.x..row_rect.right())
            .map(|x| buffer[(x, row_rect.y)].symbol())
            .collect();
        assert!(
            row_text.contains(confirm_label),
            "待确认行应完整显示 {confirm_label}: {row_text:?}"
        );
        assert!(
            !row_text.contains('…'),
            "待确认行不得截断省略: {row_text:?}"
        );
        let cell = &buffer[(row_rect.x, row_rect.y)];
        assert_eq!(cell.fg, style::menu_confirm().fg.unwrap());
        assert_eq!(cell.bg, style::menu_confirm().bg.unwrap());
    }
}

fn referenced_menu_state(rows: Vec<ReferencedFolderMenuRow>, selected: usize) -> AppState {
    let mut state = AppState::demo();
    state.overlay = Some(Overlay::ReferencedFolderMenu(
        crate::app::overlay::ReferencedFolderMenu {
            anchor: (4, 3),
            key: Some(PathBuf::from("/repo")),
            rows,
            selected,
            hover: None,
            viewport: None,
        },
    ));
    state
}

#[test]
fn referenced_menu_renders_modal_rows_addresses_and_footer() {
    use crate::app::overlay::ReferencedFolderMenuRow;
    let rows = vec![
        ReferencedFolderMenuRow::Folder {
            id: 1,
            path: PathBuf::from("/tmp/alpha"),
        },
        ReferencedFolderMenuRow::Folder {
            id: 2,
            path: PathBuf::from("/tmp/work/beta"),
        },
        ReferencedFolderMenuRow::Add,
    ];
    let state = referenced_menu_state(rows, 0);
    let (lines, _) = draw_overlay(&state);
    let joined = lines.join("\n");
    assert!(
        joined.contains(text::MENU_TITLE_REFERENCED_FOLDERS),
        "{joined}"
    );
    assert!(joined.contains(text::REFERENCED_MENU_ADD_ROW), "{joined}");
    assert!(joined.contains(text::REFERENCED_MENU_FOOTER), "{joined}");
    assert!(joined.contains("alpha"), "{joined}");
    assert!(joined.contains("beta"), "{joined}");
    // 记录行右端显示绝对路径（prompt 列表式右对齐地址）。
    assert!(joined.contains("/tmp/work"), "{joined}");
}

#[test]
fn referenced_menu_keeps_fixed_size_and_centers() {
    use crate::app::overlay::ReferencedFolderMenuRow;
    let rows = vec![
        ReferencedFolderMenuRow::Folder {
            id: 1,
            path: PathBuf::from("/tmp/alpha"),
        },
        ReferencedFolderMenuRow::Add,
    ];
    let few = referenced_menu_state(rows.clone(), 0);
    let many_rows: Vec<ReferencedFolderMenuRow> = (0..40)
        .map(|id| ReferencedFolderMenuRow::Folder {
            id,
            path: PathBuf::from(format!("/tmp/alpha-{id}")),
        })
        .chain(std::iter::once(ReferencedFolderMenuRow::Add))
        .collect();
    let many = referenced_menu_state(many_rows, 0);
    let layout_for = |state: &AppState| {
        let Some(Overlay::ReferencedFolderMenu(menu)) = state.overlay.as_ref() else {
            panic!("menu");
        };
        referenced_menu_layout(SCREEN, menu)
    };
    let few = layout_for(&few);
    let many = layout_for(&many);
    assert_eq!(few.area, many.area, "弹窗几何不随记录数变化");
    assert!(few.area.x > SCREEN.x && few.area.right() < SCREEN.right());
    assert!(few.area.y > SCREEN.y && few.area.bottom() < SCREEN.bottom());
    assert_eq!(few.total, 2);
    assert_eq!(few.rows.len(), 2);
    assert_eq!(many.rows.len(), 12, "满窗口显示（扣除顶栏与底栏）");
}

#[test]
fn referenced_menu_row_hit_maps_rows_and_outside() {
    use crate::app::overlay::ReferencedFolderMenuRow;
    let rows = vec![
        ReferencedFolderMenuRow::Folder {
            id: 7,
            path: PathBuf::from("/tmp/alpha"),
        },
        ReferencedFolderMenuRow::Add,
    ];
    let state = referenced_menu_state(rows, 0);
    let menu = match state.overlay.as_ref() {
        Some(Overlay::ReferencedFolderMenu(menu)) => menu,
        _ => panic!("menu"),
    };
    let layout = referenced_menu_layout(SCREEN, menu);
    let first = layout.rows[0];
    assert_eq!(
        referenced_menu_row_at(SCREEN, menu, first.x, first.y),
        Some(0)
    );
    assert_eq!(
        referenced_menu_row_at(SCREEN, menu, first.right() - 1, first.y),
        Some(0)
    );
    let add = layout.rows[1];
    assert_eq!(referenced_menu_row_at(SCREEN, menu, add.x, add.y), Some(1));
    // 列表下方空白与弹窗之外都不命中。
    let blank = Rect::new(first.x, add.y + 1, first.width, 1);
    assert_eq!(referenced_menu_row_at(SCREEN, menu, blank.x, blank.y), None);
    assert_eq!(referenced_menu_row_at(SCREEN, menu, 0, 0), None);
}

fn referenced_search_state(items: Vec<ReferencedEntry>, selected: usize) -> AppState {
    let mut state = AppState::demo();
    state.overlay = Some(Overlay::ReferencedFolderSearch(
        crate::app::overlay::ReferencedFolderSearch {
            anchor: (0, 0),
            folder: 1,
            key: Some(PathBuf::from("/repo")),
            root: PathBuf::from("/tmp/alpha"),
            initial_root: PathBuf::from("/tmp/alpha"),
            stack: Vec::new(),
            query: TextInput::new(String::new()),
            entries: items.clone(),
            items,
            scan_truncated: false,
            truncated: false,
            selected,
            viewport: None,
            loading: false,
            failed: false,
            request: Some(1),
        },
    ));
    state
}

#[test]
fn referenced_search_renders_scope_query_items_and_footer() {
    let items = vec![
        ReferencedEntry {
            path: PathBuf::from("/tmp/alpha/docs/src"),
            relative: "docs/src".into(),
            name: "src".into(),
            is_directory: true,
        },
        ReferencedEntry {
            path: PathBuf::from("/tmp/alpha/docs/file.rs"),
            relative: "docs/file.rs".into(),
            name: "file.rs".into(),
            is_directory: false,
        },
    ];
    let state = referenced_search_state(items, 1);
    let (lines, cursor) = draw_overlay(&state);
    let joined = lines.join("\n");
    assert!(joined.contains(text::REFERENCED_SEARCH_TITLE), "{joined}");
    assert!(joined.contains("/tmp/alpha"), "范围路径: {joined}");
    assert!(joined.contains("src/"), "目录行带尾斜杠: {joined}");
    assert!(joined.contains("file.rs"), "{joined}");
    // 两条候选在下一行显示地址（父目录绝对路径）。
    assert!(joined.contains("/tmp/alpha/docs"), "候选地址行: {joined}");
    assert!(
        joined.contains(text::WORKSPACE_TREE_LAST),
        "地址行带 └─ 前缀: {joined}"
    );
    assert!(
        joined.contains(text::REFERENCED_SEARCH_FOOTER),
        "底边快捷键: {joined}"
    );
    // 搜索框硬件光标可见。
    assert!(cursor.is_some());
}

#[test]
fn referenced_search_shows_entered_folder_top_right() {
    let items = vec![ReferencedEntry {
        path: PathBuf::from("/tmp/alpha/docs/file.rs"),
        relative: "docs/file.rs".into(),
        name: "file.rs".into(),
        is_directory: false,
    }];
    let mut state = referenced_search_state(items, 0);
    if let Some(Overlay::ReferencedFolderSearch(search)) = state.overlay.as_mut() {
        search.root = PathBuf::from("/tmp/alpha/docs");
        search.stack = vec![PathBuf::from("/tmp/alpha")];
    }
    let (lines, _) = draw_overlay(&state);
    let joined = lines.join("\n");
    assert!(joined.contains("docs"), "顶边右端显示当前文件夹: {joined}");

    // 仍在初始根时不显示目录名后缀。
    let mut state = referenced_search_state(Vec::new(), 0);
    if let Some(Overlay::ReferencedFolderSearch(search)) = state.overlay.as_mut() {
        search.initial_root = PathBuf::from("/tmp/alpha");
        search.root = PathBuf::from("/tmp/alpha");
    }
    let (lines, _) = draw_overlay(&state);
    let joined = lines.join("\n");
    assert!(joined.contains(text::REFERENCED_SEARCH_TITLE));
}

#[test]
fn referenced_search_keeps_fixed_size_regardless_of_items() {
    let entry = |index: usize| ReferencedEntry {
        path: PathBuf::from(format!("/tmp/alpha/docs/file{index}.rs")),
        relative: format!("docs/file{index}.rs"),
        name: format!("file{index}.rs"),
        is_directory: false,
    };
    let layout_for = |count: usize| {
        let state = referenced_search_state((0..count).map(entry).collect(), 0);
        let Some(Overlay::ReferencedFolderSearch(search)) = state.overlay.as_ref() else {
            panic!("search");
        };
        referenced_search_layout(SCREEN, search).expect("layout")
    };
    let few = layout_for(1);
    let many = layout_for(60);
    assert_eq!(few.area, many.area, "弹窗几何不随候选数量变化");
    assert_eq!(few.visible(), 1);
    assert!(many.visible() > few.visible(), "候选多时窗口内条目更多");
    assert!(few.area.x > SCREEN.x && few.area.right() < SCREEN.right());
    assert!(few.area.y > SCREEN.y && few.area.bottom() < SCREEN.bottom());
}

#[test]
fn referenced_search_paints_solid_panel_background() {
    let items: Vec<ReferencedEntry> = (0..10)
        .map(|index| ReferencedEntry {
            path: PathBuf::from(format!("/tmp/alpha/docs/file{index}.rs")),
            relative: format!("docs/file{index}.rs"),
            name: format!("file{index}.rs"),
            is_directory: false,
        })
        .collect();
    let state = referenced_search_state(items, 0);
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).expect("backend");
    if let Err(error) = terminal.draw(|frame| {
        render(frame, frame.area(), &state);
    }) {
        panic!("draw failed: {error}");
    }
    let buf = terminal.backend().buffer().clone();
    let search = match state.overlay.as_ref() {
        Some(Overlay::ReferencedFolderSearch(search)) => search,
        _ => panic!("search"),
    };
    let layout = referenced_search_layout(SCREEN, search).expect("layout");
    let scrollbar = layout.scrollbar.expect("list overflows");
    // 范围路径文字、非选中候选与滚动条都保持透明底色；搜索框铺暗色 surface 底条，选中候选行铺高亮底色。
    for (label, x, y) in [
        ("范围路径", layout.title.x, layout.title.y),
        ("范围路径第二个字符", layout.title.x + 1, layout.title.y),
        ("非选中候选首行", layout.list.x, layout.list.y + 2),
        ("非选中候选地址行", layout.list.x + 2, layout.list.y + 3),
        ("滚动条轨道", scrollbar.track.x, scrollbar.track.y),
        ("滚动条 thumb", scrollbar.thumb.x, scrollbar.thumb.y),
    ] {
        assert_eq!(buf[(x, y)].bg, Color::Reset, "{label} 应保持透明底色");
    }
    assert_eq!(
        buf[(layout.query.x, layout.query.y)].bg,
        Color::Indexed(236),
        "搜索框铺 surface 底条"
    );
    assert_eq!(
        buf[(layout.list.x, layout.list.y)].bg,
        Color::Indexed(240),
        "选中候选底色"
    );
    assert_eq!(
        buf[(layout.list.x + 2, layout.list.y + 1)].bg,
        Color::Indexed(240),
        "选中候选地址行底色"
    );
}

#[test]
fn referenced_menu_hover_backgrounds_row_without_marker() {
    use crate::app::overlay::ReferencedFolderMenuRow;
    let rows = vec![
        ReferencedFolderMenuRow::Folder {
            id: 1,
            path: PathBuf::from("/tmp/alpha"),
        },
        ReferencedFolderMenuRow::Folder {
            id: 2,
            path: PathBuf::from("/tmp/beta"),
        },
        ReferencedFolderMenuRow::Add,
    ];
    let mut state = referenced_menu_state(rows, 0);
    // 悬停第 2 行：只记录悬停行，选中标记不动（对齐 prompt 树列表）。
    crate::app::update::hover_referenced_menu(&mut state, Some(1));
    let Some(Overlay::ReferencedFolderMenu(menu)) = state.overlay.as_ref() else {
        panic!("menu");
    };
    let layout = referenced_menu_layout(SCREEN, menu);
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).expect("backend");
    terminal
        .draw(|frame| {
            render(frame, frame.area(), &state);
        })
        .expect("draw");
    let buffer = terminal.backend().buffer();
    // 悬停行：整行铺悬停底色，行首无 `❯` 标记。
    let hovered = layout.rows[1];
    assert_eq!(buffer[(hovered.x, hovered.y)].symbol(), " ", "悬停行无标记");
    assert_eq!(
        buffer[(hovered.x, hovered.y)].bg,
        Color::Indexed(240),
        "悬停行铺选中底色"
    );
    // 选中行（未悬停）：只有 `❯` 标记，不铺底色（背景保持透明）。
    let selected = layout.rows[0];
    assert_eq!(
        buffer[(selected.x, selected.y)].symbol(),
        text::AGENT_SELECTED_MARKER,
        "选中行显示标记"
    );
    assert_eq!(
        buffer[(selected.x, selected.y)].bg,
        Color::Reset,
        "选中行不铺底色"
    );
}

#[test]
fn referenced_menu_scrollbar_only_on_overflow_and_reserves_gutter() {
    use crate::app::overlay::ReferencedFolderMenuRow;
    let few = referenced_menu_state(vec![ReferencedFolderMenuRow::Add], 0);
    let many = referenced_menu_state(
        (0..40)
            .map(|id| ReferencedFolderMenuRow::Folder {
                id,
                path: PathBuf::from(format!("/tmp/alpha-{id}")),
            })
            .chain(std::iter::once(ReferencedFolderMenuRow::Add))
            .collect(),
        0,
    );
    let layout_of = |state: &AppState| {
        let Some(Overlay::ReferencedFolderMenu(menu)) = state.overlay.as_ref() else {
            panic!("menu");
        };
        referenced_menu_layout(SCREEN, menu)
    };
    let few = layout_of(&few);
    let many = layout_of(&many);
    assert!(few.scrollbar.is_none(), "不溢出时无滚动条");
    let bar = many.scrollbar.expect("溢出时出现滚动条");
    // 列表行固定为内容区扣掉右缘 1 列滚动条槽（与 prompt 列表一致）。
    let inner = ratatui::widgets::Block::bordered().inner(few.area);
    assert_eq!(few.rows[0].width, inner.width - 1, "右缘 1 列恒预留");
    assert_eq!(few.rows[0].right(), many.rows[0].right());
    assert_eq!(bar.track.x, inner.right() - 1, "轨道贴内容区右缘");
    assert_eq!(bar.track.height, 12, "轨道覆盖整列可见区");
    // 行文本始终让出滚动条列，不与轨道重叠。
    assert!(many.rows.iter().all(|row| row.right() <= bar.track.x));
}

#[test]
fn referenced_menu_viewport_decouples_window_from_highlight() {
    use crate::app::overlay::ReferencedFolderMenuRow;
    let rows: Vec<ReferencedFolderMenuRow> = (0..30)
        .map(|id| ReferencedFolderMenuRow::Folder {
            id,
            path: PathBuf::from(format!("/tmp/alpha-{id}")),
        })
        .chain(std::iter::once(ReferencedFolderMenuRow::Add))
        .collect();
    let mut state = referenced_menu_state(rows, 0);
    let layout_of = |state: &AppState| {
        let Some(Overlay::ReferencedFolderMenu(menu)) = state.overlay.as_ref() else {
            panic!("menu");
        };
        referenced_menu_layout(SCREEN, menu)
    };
    // 高亮在第 0 行、无显式视口：窗口跟随高亮从 0 起。
    assert_eq!(layout_of(&state).start, 0);
    // 滚轮滚开显式视口：窗口离开高亮，高亮允许不在窗口内。
    let visible = layout_of(&state).visible();
    assert!(crate::app::update::set_referenced_menu_viewport(
        &mut state, 10, visible
    ));
    let layout = layout_of(&state);
    assert_eq!(layout.start, 10, "窗口取显式视口");
    let Some(Overlay::ReferencedFolderMenu(menu)) = state.overlay.as_ref() else {
        panic!("menu");
    };
    assert_eq!(menu.selected, 0, "高亮不动");
    // 键盘移动高亮：清空视口，窗口重新跟随高亮。
    crate::app::update::move_referenced_menu_selection(&mut state, 1);
    assert_eq!(layout_of(&state).start, 0, "键盘移动后窗口吸回高亮");
}

#[test]
fn referenced_menu_renders_scrollbar_thumb_without_track_border() {
    use crate::app::overlay::ReferencedFolderMenuRow;
    let rows: Vec<ReferencedFolderMenuRow> = (0..30)
        .map(|id| ReferencedFolderMenuRow::Folder {
            id,
            path: PathBuf::from(format!("/tmp/alpha-{id}")),
        })
        .chain(std::iter::once(ReferencedFolderMenuRow::Add))
        .collect();
    let mut state = referenced_menu_state(rows, 0);
    let visible = {
        let Some(Overlay::ReferencedFolderMenu(menu)) = state.overlay.as_ref() else {
            panic!("menu");
        };
        referenced_menu_layout(SCREEN, menu).visible()
    };
    assert!(crate::app::update::set_referenced_menu_viewport(
        &mut state, 10, visible
    ));
    let (lines, _) = draw_overlay(&state);
    let menu = match state.overlay.as_ref() {
        Some(Overlay::ReferencedFolderMenu(menu)) => menu,
        _ => panic!("menu"),
    };
    let layout = referenced_menu_layout(SCREEN, menu);
    let bar = layout.scrollbar.expect("overflows");
    let thumb_row: Vec<char> = lines[bar.thumb.y as usize].chars().collect();
    assert_eq!(thumb_row[usize::from(bar.thumb.x)], '▐', "thumb 在最右一列");
    // 轨道不再绘制竖线：滚动条列在 thumb 之外保持空格。
    let last = usize::from(bar.track.bottom()) - 1;
    let last_row: Vec<char> = lines[last].chars().collect();
    assert_eq!(last_row[usize::from(bar.track.x)], ' ', "轨道不画符号");
}

#[test]
fn referenced_search_hit_tests_copy_and_items() {
    let items = vec![
        ReferencedEntry {
            path: PathBuf::from("/tmp/alpha/file.rs"),
            relative: "file.rs".into(),
            name: "file.rs".into(),
            is_directory: false,
        },
        ReferencedEntry {
            path: PathBuf::from("/tmp/alpha/other.rs"),
            relative: "other.rs".into(),
            name: "other.rs".into(),
            is_directory: false,
        },
    ];
    let state = referenced_search_state(items, 0);
    let search = match state.overlay.as_ref() {
        Some(Overlay::ReferencedFolderSearch(search)) => search,
        _ => panic!("search"),
    };
    let layout = referenced_search_layout(SCREEN, search).expect("layout");
    assert!(referenced_search_copy_at(
        SCREEN,
        search,
        layout.copy_path.x,
        layout.copy_path.y
    ));
    // 每条候选占两行：首行与地址行都命中同一候选。
    assert_eq!(
        referenced_search_item_at(SCREEN, search, layout.list.x, layout.list.y),
        Some(0)
    );
    assert_eq!(
        referenced_search_item_at(SCREEN, search, layout.list.x, layout.list.y + 1),
        Some(0)
    );
    assert_eq!(
        referenced_search_item_at(SCREEN, search, layout.list.x, layout.list.y + 2),
        Some(1)
    );
    assert_eq!(
        referenced_search_item_at(SCREEN, search, layout.list.x, layout.area.bottom()),
        None
    );
}

#[test]
fn referenced_search_failed_and_empty_states_render_messages() {
    let mut state = referenced_search_state(Vec::new(), 0);
    if let Some(Overlay::ReferencedFolderSearch(search)) = state.overlay.as_mut() {
        search.failed = true;
    }
    let (lines, _) = draw_overlay(&state);
    assert!(
        lines.join("\n").contains(text::REFERENCED_SEARCH_FAILED),
        "不可访问提示"
    );

    let state = referenced_search_state(Vec::new(), 0);
    let (lines, _) = draw_overlay(&state);
    assert!(lines.join("\n").contains(text::REFERENCED_SEARCH_EMPTY));
}

#[test]
fn referenced_input_renders_placeholder_and_error() {
    let mut state = AppState::demo();
    state.overlay = Some(Overlay::ReferencedFolderInput(
        crate::app::overlay::ReferencedFolderInput {
            anchor: (0, 0),
            input: TextInput::new(""),
            key: Some(PathBuf::from("/repo")),
            validating: false,
            error: Some(text::REFERENCED_ERROR_DUPLICATE),
            request: None,
        },
    ));
    let (lines, _) = draw_overlay(&state);
    let joined = lines.join("\n");
    assert!(joined.contains(text::REFERENCED_INPUT_TITLE), "{joined}");
    assert!(
        joined.contains(text::REFERENCED_INPUT_PLACEHOLDER),
        "{joined}"
    );
    assert!(
        joined.contains(text::REFERENCED_ERROR_DUPLICATE),
        "{joined}"
    );
}

#[test]
fn referenced_input_buttons_hit_their_cells() {
    let shell = referenced_input_shell(SCREEN).expect("input shell fits");
    let buttons = widgets::modal::button_row(
        shell.inner,
        &[text::BUTTON_CREATE, text::BUTTON_CLEAR, text::BUTTON_CANCEL],
        BUTTON_GAP,
        shell.inner.height.saturating_sub(1),
    );
    assert_eq!(
        referenced_input_button_at(&shell, buttons[0].x, buttons[0].y),
        Some(ReferencedInputButton::Create)
    );
    assert_eq!(
        referenced_input_button_at(&shell, buttons[1].x, buttons[1].y),
        Some(ReferencedInputButton::Clear)
    );
    assert_eq!(
        referenced_input_button_at(&shell, buttons[2].x, buttons[2].y),
        Some(ReferencedInputButton::Cancel)
    );
    assert_eq!(
        referenced_input_button_at(&shell, shell.inner.x, shell.inner.y),
        None
    );
}

#[test]
fn referenced_input_renders_long_absolute_path_in_full() {
    let path = "/Users/yonah/projects/agent/lx-agent/src/ui/overlay/referenced";
    let mut state = AppState::demo();
    state.overlay = Some(Overlay::ReferencedFolderInput(
        crate::app::overlay::ReferencedFolderInput {
            anchor: (0, 0),
            input: TextInput::new(path),
            key: Some(PathBuf::from("/repo")),
            validating: false,
            error: None,
            request: None,
        },
    ));
    let (lines, _) = draw_overlay(&state);
    let joined = lines.join("\n");
    assert!(joined.contains(path), "长路径不被截断: {joined}");
}

fn save_draft_state(entries: Vec<SaveDraftEntry>, selected: usize) -> AppState {
    let mut state = AppState::demo();
    state.overlay = Some(Overlay::SaveDraft(SaveDraft {
        workspace: 0,
        name: TextInput::new(text::PROMPT_DEFAULT_NAME),
        entries,
        selected,
    }));
    state
}

#[test]
fn save_draft_picker_renders_name_tree_entries_and_buttons() {
    let entries = vec![
        SaveDraftEntry::Root,
        SaveDraftEntry::Folder {
            id: 1,
            depth: 0,
            name: "ideas".into(),
        },
        SaveDraftEntry::Folder {
            id: 2,
            depth: 1,
            name: "deep".into(),
        },
        SaveDraftEntry::AddFolder,
    ];
    let state = save_draft_state(entries, 1);
    let (lines, cursor) = draw_overlay(&state);
    let joined = lines.join("\n");
    assert!(joined.contains(text::SAVE_DRAFT_TITLE), "{joined}");
    assert!(
        joined.contains(&format!(
            "{} {}",
            text::INPUT_PREFIX_ICON,
            text::PROMPT_DEFAULT_NAME
        )),
        "名称输入行带 > 前缀: {joined}"
    );
    assert!(
        joined.contains(text::PROMPT_DEFAULT_NAME),
        "名称预填: {joined}"
    );
    assert!(joined.contains(text::SAVE_DRAFT_ROOT), "{joined}");
    assert!(joined.contains("└─ deep"), "子层树形连接符: {joined}");
    assert!(joined.contains(text::SAVE_DRAFT_NEW_FOLDER), "{joined}");
    assert!(joined.contains(text::BUTTON_SAVE), "{joined}");
    assert!(joined.contains(text::BUTTON_CANCEL), "{joined}");
    // 选中行非内联输入：硬件光标在名称输入。
    let shell = save_draft_shell(SCREEN, 4).expect("shell fits");
    let name_x = shell.inner.x + input_prefix_width();
    assert_eq!(
        cursor,
        Some((
            name_x + text::PROMPT_DEFAULT_NAME.chars().count() as u16,
            shell.inner.y
        )),
        "硬件光标在名称输入末尾"
    );
    // 选中行整行铺选中底色（Indexed 240）。
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).expect("backend");
    terminal
        .draw(|frame| {
            render(frame, frame.area(), &state);
        })
        .expect("draw");
    let buffer = terminal.backend().buffer();
    let list_y = shell.inner.y + 2;
    assert_eq!(buffer[(shell.inner.x, list_y + 1)].bg, Color::Indexed(240));
    assert_eq!(buffer[(shell.inner.x, list_y)].bg, Color::Reset);
}

#[test]
fn save_draft_entry_hit_maps_rows_and_buttons() {
    let dialog = SaveDraft {
        workspace: 0,
        name: TextInput::new("x"),
        entries: vec![
            SaveDraftEntry::Root,
            SaveDraftEntry::Folder {
                id: 1,
                depth: 0,
                name: "a".into(),
            },
            SaveDraftEntry::AddFolder,
        ],
        selected: 0,
    };
    let shell = save_draft_shell(SCREEN, dialog.entries.len()).expect("shell");
    let list_y = shell.inner.y + 2;
    assert_eq!(
        save_draft_entry_at(&shell, &dialog, shell.inner.x, shell.inner.y),
        None,
        "名称行不可命中条目"
    );
    assert_eq!(
        save_draft_entry_at(&shell, &dialog, shell.inner.x, list_y),
        Some(0)
    );
    assert_eq!(
        save_draft_entry_at(&shell, &dialog, shell.inner.x, list_y + 2),
        Some(2)
    );
    assert_eq!(
        save_draft_entry_at(
            &shell,
            &dialog,
            shell.inner.x,
            shell.inner.bottom().saturating_sub(1)
        ),
        None,
        "按钮行不可命中条目"
    );
    assert_eq!(
        save_draft_entry_at(&shell, &dialog, shell.inner.x.saturating_sub(1), list_y),
        None,
        "横向区域外"
    );
    let buttons = widgets::modal::button_row(
        shell.inner,
        &SAVE_DRAFT_BUTTONS,
        BUTTON_GAP,
        shell.inner.height.saturating_sub(1),
    );
    assert_eq!(
        save_draft_button_at(&shell, buttons[0].x, buttons[0].y),
        Some(SaveDraftButton::Save)
    );
    assert_eq!(
        save_draft_button_at(&shell, buttons[1].x, buttons[1].y),
        Some(SaveDraftButton::Cancel)
    );
}

#[test]
fn save_draft_picker_windows_to_selected_entry() {
    let mut entries = Vec::new();
    for index in 0..30 {
        entries.push(SaveDraftEntry::Folder {
            id: index,
            depth: 0,
            name: format!("f{index}"),
        });
    }
    entries.push(SaveDraftEntry::AddFolder);
    let dialog = SaveDraft {
        workspace: 0,
        name: TextInput::new("x"),
        entries,
        selected: 30,
    };
    let shell = save_draft_shell(SCREEN, dialog.entries.len()).expect("shell");
    let max_rows = save_draft_max_rows(&shell);
    let start = save_draft_visible_start(&dialog, max_rows);
    assert!(start > 0, "选中项在窗口内需要上移窗口");
    let last_row = shell.inner.y + 2 + (max_rows as u16).saturating_sub(1);
    assert_eq!(
        save_draft_entry_at(&shell, &dialog, shell.inner.x, last_row),
        Some(30)
    );
}

#[test]
fn save_draft_empty_or_overflow_behavior() {
    let dialog = SaveDraft {
        workspace: 0,
        name: TextInput::new("test"),
        entries: Vec::new(),
        selected: 0,
    };

    let shell = save_draft_shell(SCREEN, 0).expect("shell exists for 0 entries");
    assert!(shell.inner.height >= 5);
    let max_rows = save_draft_max_rows(&shell);
    assert_eq!(save_draft_visible_start(&dialog, max_rows), 0);
    assert_eq!(
        save_draft_entry_at(&shell, &dialog, shell.inner.x, shell.inner.y + 2),
        None
    );

    let too_small = Rect::new(0, 0, 3, 2);
    assert!(save_draft_shell(too_small, 5).is_none());
}
