//! 单元测试；仅测试构建编译。

use super::*;
use crate::app::markdown::MentionEntry;
use crate::app::state::AppState;
use crate::app::update;
use crate::layout::PaneId;
use ratatui::style::Color;

fn prompt(cols: u16, rows: u16, text: &str) -> Prompt {
    let mut prompt = Prompt::new(PaneId::from_raw_for_test(1));
    prompt.resize(cols, rows);
    prompt.insert_str(text);
    prompt
}

fn selection(prompt: &Prompt, start: (u16, u16), end: (u16, u16)) -> Selection {
    let mut selection = Selection::begin(prompt.id(), i32::from(start.0), start.1);
    selection.drag(i32::from(end.0), end.1);
    selection
}

#[test]
fn paints_markdown_styles() {
    let area = Rect::new(0, 0, 20, 10);
    let prompt = prompt(20, 10, "## t\n**b**\n`c`\n> q\n- i\n[a](u)");
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &prompt, None);

    assert_eq!(buf[(0, 0)].symbol(), "#");
    assert!(buf[(0, 0)].modifier.contains(Modifier::DIM));
    assert_eq!(buf[(3, 0)].symbol(), "t");
    assert_eq!(buf[(3, 0)].fg, Color::Yellow);
    assert!(buf[(3, 0)].modifier.contains(Modifier::BOLD));

    assert_eq!(buf[(0, 1)].symbol(), "*");
    assert_eq!(buf[(2, 1)].symbol(), "b");
    assert_eq!(buf[(2, 1)].fg, Color::Yellow);
    assert!(buf[(2, 1)].modifier.contains(Modifier::BOLD));

    assert_eq!(buf[(1, 2)].symbol(), "c");
    assert_eq!(buf[(1, 2)].fg, Color::LightRed);

    assert_eq!(buf[(2, 3)].symbol(), "q");
    assert_eq!(buf[(2, 3)].fg, Color::LightMagenta);
    assert!(buf[(2, 3)].modifier.contains(Modifier::ITALIC));

    assert_eq!(buf[(2, 4)].symbol(), "i");
    assert_eq!(buf[(2, 4)].fg, Color::Reset);
    assert!(!buf[(2, 4)].modifier.contains(Modifier::DIM));

    assert_eq!(buf[(1, 5)].symbol(), "a");
    assert_eq!(buf[(1, 5)].fg, Color::LightBlue);
    assert!(buf[(1, 5)].modifier.contains(Modifier::UNDERLINED));
    assert_eq!(buf[(4, 5)].symbol(), "u");
    assert_eq!(buf[(4, 5)].fg, Color::Cyan);
}

#[test]
fn fence_lines_are_markers_and_content_stays_default() {
    let area = Rect::new(0, 0, 10, 4);
    let prompt = prompt(10, 4, "```\nx\n```");
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &prompt, None);
    assert!(buf[(0, 0)].modifier.contains(Modifier::DIM));
    assert_eq!(buf[(0, 1)].symbol(), "x");
    assert_eq!(buf[(0, 1)].fg, Color::Reset);
    assert!(!buf[(0, 1)].modifier.contains(Modifier::DIM));
    assert!(buf[(0, 2)].modifier.contains(Modifier::DIM));
}

#[test]
fn empty_prompt_renders_input_placeholder() {
    let area = Rect::new(0, 0, 40, 4);
    let prompt = prompt(40, 4, "");
    assert!(prompt.text().is_empty());
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &prompt, None);
    let row: String = (0..40).map(|x| buf[(x, 0)].symbol()).collect();
    assert!(row.starts_with(text::PROMPT_PLACEHOLDER), "row={row:?}");
    assert!(buf[(0, 0)].modifier.contains(Modifier::DIM));
}

#[test]
fn prompt_placeholder_disappears_after_input() {
    let area = Rect::new(0, 0, 40, 4);
    let prompt = prompt(40, 4, "hi");
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &prompt, None);
    let row: String = (0..40).map(|x| buf[(x, 0)].symbol()).collect();
    assert!(!row.contains(text::PROMPT_PLACEHOLDER), "row={row:?}");
    assert_eq!(buf[(0, 0)].symbol(), "h");
    assert_eq!(buf[(1, 0)].symbol(), "i");
}

#[test]
fn renders_scrolled_viewport() {
    let area = Rect::new(0, 0, 4, 2);
    let prompt = prompt(4, 2, "abcdefghij");
    assert_eq!(prompt.scroll(), 1);
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &prompt, None);
    assert_eq!(buf[(0, 0)].symbol(), "e");
    assert_eq!(buf[(3, 0)].symbol(), "h");
    assert_eq!(buf[(0, 1)].symbol(), "i");
}

#[test]
fn wrapped_continuation_row_keeps_line_token_offsets() {
    let area = Rect::new(0, 0, 80, 3);
    let prompt = prompt(
        80,
        3,
        "- Reference: @[refer-folder](/Users/yonah/projects/agent/herdr-master) \
         @[refer-folder](/Users/yonah/projects/agent/lx-agent)",
    );
    assert_eq!(prompt.visual_rows().len(), 2);
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &prompt, None);
    assert_eq!(buf[(0, 1)].symbol(), "o");
    assert_eq!(
        buf[(0, 1)].fg,
        Color::Indexed(208),
        "引用整段着 refer-folder 色"
    );
    assert!(buf[(0, 1)].modifier.contains(Modifier::UNDERLINED));
    assert_eq!(buf[(7, 1)].symbol(), "/");
    assert_eq!(buf[(7, 1)].fg, Color::Indexed(208), "路径与标记同色");
}

#[test]
fn scrolled_viewport_hides_cursor_when_caret_leaves_view() {
    let area = Rect::new(0, 0, 4, 2);
    let mut prompt = prompt(4, 2, "abcdefghijklm");
    assert_eq!(prompt.scroll(), 2);
    prompt.scroll_by(-1);
    assert_eq!(prompt.scroll(), 1);
    assert_eq!(prompt.cursor_cell(), None);
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &prompt, None);
    assert_eq!(buf[(0, 0)].symbol(), "e");
}

#[test]
fn selection_reverses_cells_including_empty_trailing() {
    let area = Rect::new(0, 0, 10, 3);
    let prompt = prompt(10, 3, "abc");
    let mut buf = Buffer::empty(area);
    let selection = selection(&prompt, (0, 0), (0, 5));
    render(area, &mut buf, &prompt, Some(&selection));
    for col in 0..=5 {
        assert!(
            buf[(col, 0)].modifier.contains(Modifier::REVERSED),
            "column {col} should be selected"
        );
    }
    assert!(!buf[(6, 0)].modifier.contains(Modifier::REVERSED));
}

#[test]
fn scrolled_prompt_selection_highlights_visible_rows() {
    let mut state = AppState::demo();
    state.prompt.resize(10, 2);
    state.prompt.insert_str("1\n2\n3\n4\n5\n6");
    update::set_prompt_scroll(&mut state, 4);
    let prompt = state.prompt.id();
    // 视口停在底部（内容行 4/5 可见）：按下与拖拽走事件层坐标（视口行）。
    update::begin_selection(&mut state, prompt, 0, 0);
    update::drag_selection(&mut state, prompt, 1, 2);
    assert_eq!(state.prompt.scroll(), 4);

    let area = Rect::new(0, 0, 10, 2);
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &state.prompt, state.selection_for(prompt));
    for col in 0..=2 {
        assert!(
            buf[(col, 0)].modifier.contains(Modifier::REVERSED),
            "row 0 col {col} should be selected"
        );
        assert!(
            buf[(col, 1)].modifier.contains(Modifier::REVERSED),
            "row 1 col {col} should be selected"
        );
    }
    assert!(!buf[(3, 1)].modifier.contains(Modifier::REVERSED));
}

#[test]
fn wide_chars_mark_spacer_cells() {
    let area = Rect::new(0, 0, 10, 3);
    let prompt = prompt(10, 3, "你好");
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &prompt, None);
    assert_eq!(buf[(0, 0)].symbol(), "你");
    assert_eq!(buf[(1, 0)].diff_option, CellDiffOption::Skip);
    assert_eq!(buf[(2, 0)].symbol(), "好");
    assert_eq!(buf[(3, 0)].diff_option, CellDiffOption::Skip);
}

#[test]
fn renders_block_command_panel_below_cursor() {
    let area = Rect::new(0, 0, 30, 10);
    let mut editor = Prompt::new(PaneId::from_raw_for_test(1));
    editor.resize(30, 10);
    editor.insert_str("#");
    assert!(editor.panel().is_some());
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &editor, None);

    assert_eq!(buf[(0, 1)].symbol(), "╭");
    let row: String = (0..area.width).map(|x| buf[(x, 2)].symbol()).collect();
    assert!(row.contains("Heading 1"));
    assert_eq!(buf[(2, 2)].bg, Color::Indexed(240));

    let mut empty = Buffer::empty(area);
    let blank = prompt(10, 3, "plain");
    render(area, &mut empty, &blank, None);
    assert_eq!(empty[(0, 1)].symbol(), " ");
}

#[test]
fn block_panel_renders_commands_title() {
    let area = Rect::new(0, 0, 30, 10);
    let mut editor = Prompt::new(PaneId::from_raw_for_test(1));
    editor.resize(30, 10);
    editor.insert_str("#");
    let rect = panel_rect(&editor, area).expect("panel visible");
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &editor, None);

    let top: String = (0..area.width).map(|x| buf[(x, rect.y)].symbol()).collect();
    assert!(top.contains(text::BLOCK_PANEL_TITLE), "{top}");
}

#[test]
fn block_panel_rect_and_items_hit_test() {
    let area = Rect::new(0, 0, 30, 10);
    let mut editor = Prompt::new(PaneId::from_raw_for_test(1));
    editor.resize(30, 10);
    editor.insert_str("#");
    let rect = panel_rect(&editor, area).expect("panel visible");
    assert_eq!(rect.y, 1, "面板贴在锚点行下方");
    assert_eq!(panel_item_at(&editor, area, 2, rect.y + 1), Some(0));
    assert_eq!(panel_item_at(&editor, area, 2, rect.y + 2), Some(1));
    assert_eq!(panel_item_at(&editor, area, 2, rect.y + 6), Some(5));
    assert_eq!(
        panel_item_at(&editor, area, 2, rect.y + 7),
        None,
        "下边框不命中"
    );
    assert_eq!(
        panel_item_at(&editor, area, 2, rect.y),
        None,
        "上边框不命中"
    );
}

#[test]
fn block_panel_window_keeps_still_when_hovering_visible_item() {
    let area = Rect::new(0, 0, 30, 7);
    let mut editor = Prompt::new(PaneId::from_raw_for_test(1));
    editor.resize(30, 7);
    editor.insert_str("#");
    editor.panel_move(5);
    let rect = panel_rect(&editor, area).expect("panel visible");
    let top = panel_item_at(&editor, area, 2, rect.y + 1);

    // 悬停窗口内条目：窗口保持不动（锚点不跟随悬停）。
    assert!(editor.panel_set_active(3));
    assert_eq!(editor.panel().map(|panel| panel.active()), Some(3));
    assert_eq!(panel_rect(&editor, area), Some(rect));
    assert_eq!(panel_item_at(&editor, area, 2, rect.y + 1), top);

    // 键盘移动：锚点跟随高亮，窗口滚动。
    editor.panel_move(-1);
    assert_ne!(panel_item_at(&editor, area, 2, rect.y + 1), top);
}

#[test]
fn renders_mention_panel_stacked_and_hit_tests_items() {
    let area = Rect::new(0, 0, 30, 20);
    let mut editor = Prompt::new(PaneId::from_raw_for_test(1));
    editor.resize(30, 20);
    editor.set_mention_root(Some(std::path::PathBuf::from("/tmp/ws")));
    editor.insert_str("@");
    let (generation, _) = editor.take_mention_scan_request().expect("scan requested");
    editor.apply_mention_entries(
        generation,
        vec![
            MentionEntry {
                path: "src/app.rs".into(),
                is_directory: false,
            },
            MentionEntry {
                path: "src/lib.rs".into(),
                is_directory: false,
            },
        ],
    );
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &editor, None);

    assert_eq!(buf[(0, 1)].symbol(), "╭");
    let name_row: String = (0..area.width).map(|x| buf[(x, 2)].symbol()).collect();
    let dir_row: String = (0..area.width).map(|x| buf[(x, 3)].symbol()).collect();
    assert!(name_row.contains("app.rs"));
    assert!(name_row.contains(text::MENTION_FILE_ICON), "{name_row}");
    assert_eq!(buf[(3, 2)].symbol(), text::MENTION_FILE_ICON);
    assert!(buf[(3, 2)].modifier.contains(Modifier::DIM));
    assert!(dir_row.contains("src"));
    assert!(dir_row.contains(text::WORKSPACE_TREE_LAST), "{dir_row}");
    assert!(buf[(3, 3)].modifier.contains(Modifier::DIM));
    assert_eq!(buf[(2, 2)].bg, Color::Indexed(240));

    assert_eq!(mention_item_at(&editor, area, 2, 2), Some(0));
    assert_eq!(mention_item_at(&editor, area, 2, 3), Some(0));
    assert_eq!(mention_item_at(&editor, area, 2, 4), Some(1));
    assert_eq!(mention_item_at(&editor, area, 2, 5), Some(1));
    assert_eq!(mention_item_at(&editor, area, 2, 6), None);
    assert_eq!(mention_item_at(&editor, area, 2, 1), None);
}

#[test]
fn template_separator_renders_block_border_line() {
    let area = Rect::new(0, 0, 30, 10);
    let mut editor = Prompt::new(PaneId::from_raw_for_test(1));
    editor.resize(30, 10);
    editor.set_text("&&& addTemplate --start\n{title: }\n\nbody\n&&& addTemplate --end\n");
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &editor, None);
    // 分割线整行以模板块边框样式绘制：├─…─┤。
    let row: String = (0..area.width).map(|x| buf[(x, 2)].symbol()).collect();
    assert!(row.starts_with("├──"), "{row}");
    assert!(row.ends_with("┤"), "{row}");
    assert_eq!(buf[(8, 2)].fg, Color::LightBlue, "todo 边框色");
    assert!(row.contains('─'), "{row}");
}

#[test]
fn mention_panel_asks_root_choice_then_titles_with_choice() {
    let area = Rect::new(0, 0, 40, 20);
    let mut editor = Prompt::new(PaneId::from_raw_for_test(1));
    editor.resize(40, 20);
    editor.set_worktree_repo(Some(std::path::PathBuf::from("/repo")));
    editor.set_mention_root(Some(std::path::PathBuf::from("/fallback")));
    editor.set_text("&&& addTemplate --start\nbody\n\n{wt:feature/x}\n&&& addTemplate --end\n");
    editor.apply_worktree_entries(vec![crate::app::markdown::WorktreeCandidate {
        branch: "feature/x".into(),
        path: std::path::PathBuf::from("/repo/.worktrees/feat"),
        is_main: false,
    }]);
    let body_end = editor.text().find("body").expect("body line") + "body".len();
    editor.set_cursor_offset_for_test(body_end);
    editor.insert_char(' ');
    editor.insert_char('@');
    // 绑定 linked worktree：先显示 main / wt 取根选择器。
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &editor, None);
    let painted: String = buf.content.iter().map(|cell| cell.symbol()).collect();
    assert!(painted.contains("main"), "{painted}");
    assert!(painted.contains("wt"), "{painted}");
    assert!(
        !painted.contains("[wt]"),
        "选择器阶段无 [wt] 标签: {painted}"
    );
    // 选 wt 后进入文件面板：标题追加 -[wt]，条目无右侧标签。
    assert!(editor.mention_move(1));
    assert!(editor.mention_confirm());
    let (generation, root) = editor.take_mention_scan_request().expect("scan request");
    assert_eq!(root, std::path::PathBuf::from("/repo/.worktrees/feat"));
    editor.apply_mention_entries(
        generation,
        vec![MentionEntry {
            path: "src/app.rs".into(),
            is_directory: false,
        }],
    );
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &editor, None);
    let painted: String = buf.content.iter().map(|cell| cell.symbol()).collect();
    assert!(painted.contains("app.rs"), "{painted}");
    assert!(painted.contains("Files-[wt]"), "标题带取根标识: {painted}");

    // 未绑定的块：直接列出显式根候选，标题无后缀。
    let mut plain = Prompt::new(PaneId::from_raw_for_test(2));
    plain.resize(40, 20);
    plain.set_mention_root(Some(std::path::PathBuf::from("/fallback")));
    plain.set_text("&&& addTemplate --start\nbody\n&&& addTemplate --end\n");
    let body_end = plain.text().find("body").expect("body line") + "body".len();
    plain.set_cursor_offset_for_test(body_end);
    plain.insert_char(' ');
    plain.insert_char('@');
    let (generation, root) = plain.take_mention_scan_request().expect("scan request");
    assert_eq!(root, std::path::PathBuf::from("/fallback"));
    plain.apply_mention_entries(
        generation,
        vec![MentionEntry {
            path: "src/app.rs".into(),
            is_directory: false,
        }],
    );
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &plain, None);
    let painted: String = buf.content.iter().map(|cell| cell.symbol()).collect();
    assert!(painted.contains("app.rs"), "{painted}");
    assert!(painted.contains("Files"), "{painted}");
    assert!(!painted.contains("Files-["), "未绑定无标题后缀: {painted}");
}

#[test]
fn mention_panel_marks_directory_and_file_icons() {
    let area = Rect::new(0, 0, 30, 20);
    let mut editor = Prompt::new(PaneId::from_raw_for_test(1));
    editor.resize(30, 20);
    editor.set_mention_root(Some(std::path::PathBuf::from("/tmp/ws")));
    editor.insert_str("@");
    let (generation, _) = editor.take_mention_scan_request().expect("scan requested");
    editor.apply_mention_entries(
        generation,
        vec![
            MentionEntry {
                path: "src".into(),
                is_directory: true,
            },
            MentionEntry {
                path: "src/app.rs".into(),
                is_directory: false,
            },
        ],
    );
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &editor, None);

    let dir_row: String = (0..area.width).map(|x| buf[(x, 2)].symbol()).collect();
    let file_row: String = (0..area.width).map(|x| buf[(x, 3)].symbol()).collect();
    assert!(dir_row.contains(text::MENTION_DIR_ICON), "{dir_row}");
    assert!(dir_row.contains("src/"), "{dir_row}");
    assert!(file_row.contains(text::MENTION_FILE_ICON), "{file_row}");
    assert!(file_row.contains("app.rs"), "{file_row}");
    assert_ne!(text::MENTION_DIR_ICON, text::MENTION_FILE_ICON);
}

#[test]
fn mention_panel_renders_title_folder_name_and_footer() {
    let area = Rect::new(0, 0, 40, 20);
    let mut editor = Prompt::new(PaneId::from_raw_for_test(1));
    editor.resize(40, 20);
    editor.set_mention_root(Some(std::path::PathBuf::from("/tmp/ws")));
    editor.insert_str("@");
    let (generation, _) = editor.take_mention_scan_request().expect("scan requested");
    editor.apply_mention_entries(
        generation,
        vec![
            MentionEntry {
                path: "src".into(),
                is_directory: true,
            },
            MentionEntry {
                path: "src/app.rs".into(),
                is_directory: false,
            },
        ],
    );

    // 根范围：只有左上标题与底边提示，右上不显示目录名。
    let root_rect = mention_panel_rect(&editor, area).expect("panel visible");
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &editor, None);
    let root_top: String = (0..area.width)
        .map(|x| buf[(x, root_rect.y)].symbol())
        .collect();
    let root_bottom: String = (0..area.width)
        .map(|x| buf[(x, root_rect.bottom() - 1)].symbol())
        .collect();
    assert!(root_top.contains(text::MENTION_PANEL_TITLE), "{root_top}");
    assert!(
        root_bottom.contains(text::MENTION_PANEL_FOOTER),
        "{root_bottom}"
    );

    // 进入 src：右上显示目录名，底边提示保持。
    assert!(editor.mention_enter_folder());
    let rect = mention_panel_rect(&editor, area).expect("scoped panel visible");
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &editor, None);
    let top: String = (0..area.width).map(|x| buf[(x, rect.y)].symbol()).collect();
    let bottom: String = (0..area.width)
        .map(|x| buf[(x, rect.bottom() - 1)].symbol())
        .collect();
    assert!(top.contains(text::MENTION_PANEL_TITLE), "{top}");
    assert!(top.contains(" src "), "{top}");
    assert!(bottom.contains(text::MENTION_PANEL_FOOTER), "{bottom}");
}

#[test]
fn mention_panel_middle_truncates_overlong_parent_path() {
    let area = Rect::new(0, 0, 30, 20);
    let mut editor = Prompt::new(PaneId::from_raw_for_test(1));
    editor.resize(30, 20);
    editor.set_mention_root(Some(std::path::PathBuf::from("/tmp/ws")));
    editor.insert_str("@");
    let (generation, _) = editor.take_mention_scan_request().expect("scan requested");
    editor.apply_mention_entries(
        generation,
        vec![MentionEntry {
            path: "alpha/beta/gamma/delta/epsilon/zeta/file.rs".into(),
            is_directory: false,
        }],
    );
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &editor, None);

    let rect = mention_panel_rect(&editor, area).expect("panel visible");
    let name_row: String = (0..area.width)
        .map(|x| buf[(x, rect.y + 1)].symbol())
        .collect();
    let dir_row: String = (0..area.width)
        .map(|x| buf[(x, rect.y + 2)].symbol())
        .collect();
    assert!(name_row.contains("file.rs"), "{name_row}");
    assert!(dir_row.contains(text::WORKSPACE_TREE_LAST), "{dir_row}");
    assert!(dir_row.contains('…'), "超长父路径应中间截断: {dir_row}");
    // 保留路径首尾：开头目录与紧邻的父目录可见，中段被省略。
    assert!(dir_row.contains("alpha/"), "{dir_row}");
    assert!(dir_row.contains("zeta"), "{dir_row}");
    assert!(!dir_row.contains("gamma"), "{dir_row}");
    // 截断后的明细必须完整落在面板内（不能被裁切）；按字符列计。
    let tail_start = dir_row.rfind("zeta").expect("tail visible");
    let detail_end = dir_row[..tail_start].chars().count() + 4;
    assert!(
        detail_end < usize::from(rect.right()),
        "{dir_row:?} rect={rect:?} end={detail_end}"
    );
}

#[test]
fn mention_panel_window_keeps_still_when_hovering_visible_item() {
    let area = Rect::new(0, 0, 30, 20);
    let mut editor = Prompt::new(PaneId::from_raw_for_test(1));
    editor.resize(30, 20);
    editor.set_mention_root(Some(std::path::PathBuf::from("/tmp/ws")));
    editor.insert_str("@");
    let (generation, _) = editor.take_mention_scan_request().expect("scan requested");
    let entries: Vec<MentionEntry> = (0..10)
        .map(|index| MentionEntry {
            path: format!("src/f{index}.rs"),
            is_directory: false,
        })
        .collect();
    editor.apply_mention_entries(generation, entries);

    editor.mention_move(9);
    let rect = mention_panel_rect(&editor, area).expect("panel visible");
    assert_eq!(mention_item_at(&editor, area, 2, rect.y + 1), Some(6));

    // 悬停窗口内条目：窗口保持不动，不因悬停向上滚动。
    editor.mention_set_active(8);
    assert_eq!(mention_panel_rect(&editor, area), Some(rect));
    assert_eq!(mention_item_at(&editor, area, 2, rect.y + 1), Some(6));
    assert_eq!(mention_item_at(&editor, area, 2, rect.y + 7), Some(9));
}

/// 工具栏渲染测试面板：40x8，内容区含顶部 2 行表头。
fn toolbar_panel() -> Rect {
    Rect::new(0, 0, 40, 8)
}

#[test]
fn renders_toolbar_buttons_and_divider() {
    let panel = toolbar_panel();
    let mut editor = Prompt::new(PaneId::from_raw_for_test(1));
    editor.resize(36, 6);
    editor.insert_str("hi");
    let mut buf = Buffer::empty(panel);
    render_header(
        panel,
        &mut buf,
        &editor,
        false,
        PromptMode::Editor,
        None,
        false,
        false,
    );

    let bar = crate::layout::prompt_toolbar_rect(panel).expect("toolbar visible");
    let render_label = |buf: &Buffer, rect: Rect| -> String {
        (rect.x..rect.right())
            .map(|x| buf[(x, rect.y)].symbol())
            .collect()
    };
    let undo = crate::layout::prompt_toolbar_button_rect(
        panel,
        crate::layout::PromptToolbarButton::Undo,
        false,
    )
    .expect("undo visible");
    assert_eq!(render_label(&buf, undo), text::PROMPT_UNDO_LABEL);
    assert_eq!(buf[(undo.x, bar.y)].fg, Color::Cyan, "可撤销时强调色");
    assert!(buf[(undo.x, bar.y)].modifier.contains(Modifier::BOLD));

    let redo = crate::layout::prompt_toolbar_button_rect(
        panel,
        crate::layout::PromptToolbarButton::Redo,
        false,
    )
    .expect("redo visible");
    assert_eq!(render_label(&buf, redo), text::PROMPT_REDO_LABEL);
    assert!(buf[(redo.x, bar.y)].modifier.contains(Modifier::DIM));

    let select_all = crate::layout::prompt_toolbar_button_rect(
        panel,
        crate::layout::PromptToolbarButton::SelectAll,
        false,
    )
    .expect("select all visible");
    assert_eq!(
        render_label(&buf, select_all),
        text::PROMPT_SELECT_ALL_LABEL
    );

    let save = crate::layout::prompt_toolbar_button_rect(
        panel,
        crate::layout::PromptToolbarButton::Save,
        false,
    )
    .expect("save visible");
    let save_x = save.x;
    assert_eq!(buf[(save_x, bar.y)].symbol(), text::PROMPT_SAVE_DOT);
    assert_eq!(buf[(save_x, bar.y)].fg, Color::Yellow, "编辑后未保存");

    // 分割线位于按钮下一行：两端 `├` / `┤`，中段 `─`，不穿过功能按钮。
    let divider_y = bar.y + crate::layout::PROMPT_TOOLBAR_HEIGHT;
    assert_eq!(buf[(panel.x, divider_y)].symbol(), text::DIVIDER_LEFT_JOIN);
    assert_eq!(
        buf[(panel.right() - 1, divider_y)].symbol(),
        text::DIVIDER_RIGHT_JOIN
    );
    assert_eq!(buf[(20, divider_y)].symbol(), text::DIVIDER_MID);

    editor.mark_saved();
    let mut buf = Buffer::empty(panel);
    render_header(
        panel,
        &mut buf,
        &editor,
        false,
        PromptMode::Editor,
        None,
        false,
        false,
    );
    assert_eq!(buf[(save_x, bar.y)].fg, Color::Green, "保存后绿点");

    // 临时草稿：空心点（与空闲 agent 状态一致），恒灰。
    let mut buf = Buffer::empty(panel);
    render_header(
        panel,
        &mut buf,
        &editor,
        false,
        PromptMode::Editor,
        Some(text::PROMPT_DRAFT_TITLE),
        true,
        false,
    );
    assert_eq!(buf[(save_x, bar.y)].symbol(), text::PROMPT_DRAFT_DOT);
    assert_eq!(buf[(save_x, bar.y)].fg, Color::DarkGray, "草稿恒灰空心点");
}

#[test]
fn header_divider_embeds_edit_title_and_ellipsizes() {
    let panel = toolbar_panel();
    let editor = Prompt::new(PaneId::from_raw_for_test(1));
    let divider_y = crate::layout::prompt_toolbar_rect(panel)
        .expect("toolbar visible")
        .y
        + crate::layout::PROMPT_DIVIDER_HEIGHT;
    let divider_row = |buf: &Buffer| -> String {
        (panel.x..panel.right())
            .map(|x| buf[(x, divider_y)].symbol())
            .collect()
    };

    let mut buf = Buffer::empty(panel);
    render_header(
        panel,
        &mut buf,
        &editor,
        false,
        PromptMode::Editor,
        Some("my prompt"),
        false,
        false,
    );
    let row = divider_row(&buf);
    assert!(row.contains(" my prompt "), "{row}");
    assert!(row.starts_with(text::DIVIDER_LEFT_JOIN), "{row}");
    assert!(row.ends_with(text::DIVIDER_RIGHT_JOIN), "{row}");

    // 超宽标题以 `…` 截断，不覆盖右衔接符。
    let long = "x".repeat(80);
    let mut buf = Buffer::empty(panel);
    render_header(
        panel,
        &mut buf,
        &editor,
        false,
        PromptMode::Editor,
        Some(&long),
        false,
        false,
    );
    let row = divider_row(&buf);
    assert!(row.contains('…'), "{row}");
    assert!(row.ends_with(text::DIVIDER_RIGHT_JOIN), "{row}");

    // 树视图不显示 title。
    let mut buf = Buffer::empty(panel);
    render_header(
        panel,
        &mut buf,
        &editor,
        false,
        PromptMode::Tree,
        Some("my prompt"),
        false,
        false,
    );
    let row = divider_row(&buf);
    assert!(!row.contains("my prompt"), "{row}");
}

#[test]
fn toolbar_pin_renders_hollow_then_filled_left_of_select_all() {
    let panel = toolbar_panel();
    let editor = Prompt::new(PaneId::from_raw_for_test(1));
    let bar = crate::layout::prompt_toolbar_rect(panel).expect("toolbar visible");
    let render_label = |buf: &Buffer, rect: Rect| -> String {
        (rect.x..rect.right())
            .map(|x| buf[(x, rect.y)].symbol())
            .collect()
    };

    let pin = crate::layout::prompt_toolbar_button_rect(
        panel,
        crate::layout::PromptToolbarButton::Pin,
        false,
    )
    .expect("pin visible");
    let select_all = crate::layout::prompt_toolbar_button_rect(
        panel,
        crate::layout::PromptToolbarButton::SelectAll,
        false,
    )
    .expect("select all visible");
    assert_eq!(pin.y, bar.y, "钉住按钮在功能按钮行");
    assert_eq!(select_all.x - pin.right(), 1, "紧邻 [*] 左侧，间隔 1 列");

    let mut buf = Buffer::empty(panel);
    render_header(
        panel,
        &mut buf,
        &editor,
        false,
        PromptMode::Editor,
        None,
        false,
        false,
    );
    assert_eq!(render_label(&buf, pin), text::PROMPT_UNPINNED_LABEL);

    let mut buf = Buffer::empty(panel);
    render_header(
        panel,
        &mut buf,
        &editor,
        false,
        PromptMode::Editor,
        None,
        false,
        true,
    );
    assert_eq!(render_label(&buf, pin), text::PROMPT_PINNED_LABEL);
}

#[test]
fn divider_follows_prompt_focus_style() {
    let panel = toolbar_panel();
    let editor = Prompt::new(PaneId::from_raw_for_test(1));
    let divider_y = crate::layout::prompt_toolbar_rect(panel)
        .expect("toolbar visible")
        .y
        + crate::layout::PROMPT_TOOLBAR_HEIGHT;

    let mut buf = Buffer::empty(panel);
    render_header(
        panel,
        &mut buf,
        &editor,
        true,
        PromptMode::Editor,
        None,
        false,
        false,
    );
    for x in [panel.x, 5, panel.right() - 1] {
        let cell = &buf[(x, divider_y)];
        assert_eq!(cell.fg, Color::Cyan, "聚焦分割线跟随强调色: x={x}");
        assert!(cell.modifier.contains(Modifier::BOLD));
    }

    let mut buf = Buffer::empty(panel);
    render_header(
        panel,
        &mut buf,
        &editor,
        false,
        PromptMode::Editor,
        None,
        false,
        false,
    );
    for x in [panel.x, 5, panel.right() - 1] {
        let cell = &buf[(x, divider_y)];
        assert_ne!(cell.fg, Color::Cyan, "失焦分割线不强调: x={x}");
        assert!(cell.modifier.contains(Modifier::DIM));
    }
}

#[test]
fn toolbar_hidden_when_prompt_too_short() {
    let panel = Rect::new(0, 0, 40, 3);
    assert_eq!(crate::layout::prompt_header_rect(panel), None);
    let mut editor = Prompt::new(PaneId::from_raw_for_test(1));
    editor.insert_str("hi");
    let mut buf = Buffer::empty(panel);
    render_header(
        panel,
        &mut buf,
        &editor,
        false,
        PromptMode::Editor,
        None,
        false,
        false,
    );
    assert_eq!(buf[(1, 1)].symbol(), " ", "内容区过矮不绘制工具栏");
    assert_eq!(buf[(2, 1)].symbol(), " ");
}

// ---- 斜杠命令面板与模板块按钮 ----

#[test]
fn slash_panel_renders_templates_title_and_hit_tests_items() {
    let area = Rect::new(0, 0, 40, 10);
    let prompt = prompt(40, 10, "/");
    let layout = slash_layout(&prompt, area).expect("slash panel visible");
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &prompt, None);

    let lines: Vec<String> = (layout.rect.y..layout.rect.bottom())
        .map(|y| {
            (layout.rect.x..layout.rect.right())
                .map(|x| buf[(x, y)].symbol())
                .collect()
        })
        .collect();
    assert!(
        lines
            .iter()
            .any(|line| line.contains(text::SLASH_PANEL_TITLE)),
        "顶边标题为 Templates: {lines:?}"
    );
    assert!(lines.iter().any(|line| line.contains("/add")));
    assert!(lines.iter().any(|line| line.contains("addTemplate")));

    assert_eq!(
        slash_item_at(&prompt, area, layout.content.x + 1, layout.content.y),
        Some(0)
    );
    assert_eq!(
        slash_item_at(&prompt, area, layout.rect.x, layout.rect.y),
        None
    );
    assert_eq!(
        slash_panel_rect(&prompt, area).map(|rect| rect.height),
        Some(layout.rect.height)
    );
}

#[test]
fn template_command_and_title_and_mention_styles() {
    let area = Rect::new(0, 0, 60, 4);
    let editor = prompt(
        60,
        4,
        "&&& addTemplate --start {title: }\n&&& addTemplate --end\n@src/app.rs",
    );
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &editor, None);

    // 模板块左边框占前两列，文本整体右移。
    assert_eq!(buf[(0, 0)].symbol(), "╭");
    assert_eq!(buf[(0, 0)].fg, Color::LightBlue, "todo 边框为靛蓝");
    assert_eq!(buf[(1, 0)].symbol(), "─");

    assert!(buf[(2, 0)].modifier.contains(Modifier::DIM), "&&& 为 dim");
    assert_eq!(buf[(6, 0)].symbol(), "a");
    assert_eq!(buf[(6, 0)].fg, Color::Green, "add 命令为绿色");
    assert!(buf[(6, 0)].modifier.contains(Modifier::BOLD));
    assert_eq!(buf[(18, 0)].symbol(), "-", "--start 为 dim 标记");
    assert!(buf[(18, 0)].modifier.contains(Modifier::DIM));

    assert_eq!(buf[(26, 0)].symbol(), "{");
    assert_eq!(buf[(26, 0)].fg, Color::Cyan, "标题占位符为 Cyan");
    assert!(buf[(26, 0)].modifier.contains(Modifier::UNDERLINED));

    // 非模板块行不缩进、无边框。
    assert_eq!(buf[(0, 2)].symbol(), "@");
    assert_eq!(buf[(0, 2)].fg, Color::Yellow, "@文件为 Yellow 下划线");
    assert!(buf[(0, 2)].modifier.contains(Modifier::UNDERLINED));
}

#[test]
fn template_block_draws_status_colored_border_box() {
    let area = Rect::new(0, 0, 60, 4);
    let running = prompt(
        60,
        4,
        "&&& bugTemplate --start {title: }\n# Fix\n&&& bugTemplate --end in_progress",
    );
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &running, None);

    assert_eq!(buf[(0, 0)].symbol(), "╭");
    assert_eq!(buf[(0, 0)].fg, Color::Yellow, "in_progress 边框为黄");
    assert_eq!(buf[(59, 0)].symbol(), "╮");
    assert_eq!(buf[(59, 0)].fg, Color::Yellow);
    assert_eq!(buf[(0, 1)].symbol(), "│");
    assert_eq!(buf[(0, 1)].fg, Color::Yellow);
    assert_eq!(buf[(59, 1)].symbol(), "│");
    assert_eq!(buf[(0, 2)].symbol(), "╰");
    assert_eq!(buf[(59, 2)].symbol(), "╯");
    assert_eq!(buf[(0, 2)].fg, Color::Yellow);
    // 结束行底边以 `─` 补满文本之后。
    assert_eq!(buf[(40, 2)].symbol(), "─");
    // 块外行无边框。
    assert_eq!(buf[(0, 3)].symbol(), " ");

    let done = prompt(
        60,
        4,
        "&&& bugTemplate --start {title: }\n&&& bugTemplate --end done",
    );
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &done, None);
    assert_eq!(buf[(0, 0)].fg, Color::Green, "done 边框为绿");
    assert_eq!(buf[(0, 1)].fg, Color::Green);
}

#[test]
fn template_block_paints_status_background() {
    let area = Rect::new(0, 0, 60, 4);
    let cases = [
        (
            "&&& bugTemplate --start {title: }\n# Fix\n&&& bugTemplate --end",
            Color::Rgb(35, 40, 56),
        ),
        (
            "&&& bugTemplate --start {title: }\n# Fix\n&&& bugTemplate --end in_progress",
            Color::Rgb(51, 41, 28),
        ),
        (
            "&&& bugTemplate --start {title: }\n# Fix\n&&& bugTemplate --end done",
            Color::Rgb(29, 46, 36),
        ),
    ];
    for (text, bg) in cases {
        let editor = prompt(60, 4, text);
        let mut buf = Buffer::empty(area);
        render(area, &mut buf, &editor, None);
        for y in 0..3 {
            for x in 0..area.width {
                assert_eq!(buf[(x, y)].bg, bg, "{text:?} ({x},{y})");
            }
        }
        assert_eq!(buf[(0, 3)].bg, Color::Reset, "块外行不着色");
    }
}

#[test]
fn template_block_background_skips_selected_cells() {
    let area = Rect::new(0, 0, 60, 4);
    let editor = prompt(
        60,
        4,
        "&&& bugTemplate --start {title: }\n# Fix\n&&& bugTemplate --end in_progress",
    );
    let selection = selection(&editor, (1, 3), (1, 8));
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &editor, Some(&selection));
    assert_eq!(buf[(5, 1)].bg, Color::Reset, "选区格不铺块底色");
    assert!(buf[(5, 1)].modifier.contains(Modifier::REVERSED));
    assert_eq!(buf[(20, 1)].bg, Color::Rgb(51, 41, 28), "非选区格铺块底色");
}

#[test]
fn template_buttons_render_and_hit_test() {
    let area = Rect::new(0, 0, 70, 10);
    let prompt = prompt(
        70,
        10,
        "&&& addTemplate --start {title: }\n# Add\n&&& addTemplate --end",
    );
    let buttons = template_buttons(&prompt, area);
    assert_eq!(buttons.len(), 4, "状态、复制、清理、删除");
    assert_eq!(buttons[0].button, TemplateBlockButton::Status);
    assert_eq!(buttons[1].button, TemplateBlockButton::Copy);
    assert_eq!(buttons[2].button, TemplateBlockButton::Clean);
    assert_eq!(buttons[3].button, TemplateBlockButton::Delete);
    assert_eq!(buttons[3].status, TemplateStatus::Todo);
    assert_eq!(
        buttons[3].rect.right(),
        69,
        "按钮组贴右角内侧、右边框在末列"
    );

    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &prompt, None);
    let row: String = (0..area.width).map(|x| buf[(x, 0)].symbol()).collect();
    assert!(row.contains(text::TEMPLATE_BUTTON_TODO), "{row}");
    assert!(row.contains(text::TEMPLATE_BUTTON_COPY), "{row}");
    assert!(row.contains(text::TEMPLATE_BUTTON_CLEAN), "{row}");
    assert!(row.contains(text::TEMPLATE_BUTTON_DEL), "{row}");
    assert_eq!(buf[(69, 0)].symbol(), "╮");

    for hit in &buttons {
        let cell = hit.rect;
        assert_eq!(
            template_button_at(&prompt, area, cell.x, cell.y),
            Some(*hit)
        );
    }
    assert_eq!(template_button_at(&prompt, area, 0, 0), None);
}

#[test]
fn template_buttons_stay_visible_and_text_ellipsizes() {
    let area = Rect::new(0, 0, 40, 10);
    let editor = prompt(
        40,
        10,
        "&&& addTemplate --start {title: }\n&&& addTemplate --end",
    );
    let buttons = template_buttons(&editor, area);
    assert_eq!(buttons.len(), 4, "窄屏按钮不隐藏");
    assert!(buttons[0].rect.x >= area.x + 2, "按钮让开左边框");
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &editor, None);
    let row: String = (0..area.width).map(|x| buf[(x, 0)].symbol()).collect();
    assert!(row.contains(text::TEMPLATE_BUTTON_TODO), "{row}");
    assert!(row.contains(text::TEMPLATE_BUTTON_DEL), "{row}");
    let text_side: String = (0..buttons[0].rect.x)
        .map(|x| buf[(x, 0)].symbol())
        .collect();
    assert!(
        text_side.contains('…'),
        "按钮组左侧正文省略号截断: {text_side:?}"
    );

    // 极窄区域仍绘制按钮（按可用宽度截断）。
    let narrow = Rect::new(0, 0, 20, 10);
    let narrow_prompt = prompt(
        20,
        10,
        "&&& addTemplate --start {title: }\n&&& addTemplate --end",
    );
    let mut buf = Buffer::empty(narrow);
    render(narrow, &mut buf, &narrow_prompt, None);
    let row: String = (0..narrow.width).map(|x| buf[(x, 0)].symbol()).collect();
    assert!(row.contains("[todo]"), "{row}");
}

#[test]
fn template_status_button_label_follows_block_status() {
    let area = Rect::new(0, 0, 70, 10);
    let done = prompt(
        70,
        10,
        "&&& bugTemplate --start {title: }\n&&& bugTemplate --end done",
    );
    let buttons = template_buttons(&done, area);
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &done, None);
    let row: String = (0..area.width).map(|x| buf[(x, 0)].symbol()).collect();
    assert!(row.contains(text::TEMPLATE_BUTTON_DONE), "{row}");
    assert_eq!(buttons[0].button, TemplateBlockButton::Status);

    let run = prompt(
        70,
        10,
        "&&& bugTemplate --start {title: }\n&&& bugTemplate --end in_progress",
    );
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &run, None);
    let row: String = (0..area.width).map(|x| buf[(x, 0)].symbol()).collect();
    assert!(row.contains(text::TEMPLATE_BUTTON_RUN), "{row}");
}

// ===== 粘贴/字母快捷面板 =====

#[test]
fn paste_panel_hit_and_layout_rows() {
    let mut prompt = prompt(40, 10, "");
    let anchor = crate::app::prompt::PasteAnchor::at_cursor(0);
    let request = prompt.begin_paste_read(anchor);
    let candidates = vec![
        crate::app::prompt::PasteCandidate::external(
            std::path::PathBuf::from("/tmp/a.txt"),
            crate::app::prompt::PasteCandidateKind::File,
        ),
        crate::app::prompt::PasteCandidate::external(
            std::path::PathBuf::from("/tmp/dir"),
            crate::app::prompt::PasteCandidateKind::Folder,
        ),
    ];
    assert!(prompt.paste_stage(request, candidates));
    let area = Rect::new(0, 0, 40, 10);
    let layout = paste_layout(&prompt, area).expect("paste layout");
    assert_eq!(layout.visible, 2);
    let hit = paste_item_at(&prompt, area, layout.content.x + 1, layout.content.y);
    assert_eq!(hit, Some(0));
    let second = paste_item_at(&prompt, area, layout.content.x + 1, layout.content.y + 1);
    assert_eq!(second, Some(1));
}

#[test]
fn refer_panel_hit_maps_candidates() {
    let prompt = prompt(40, 10, "@abc one\nac");
    let panel = prompt.refer_panel().expect("refer panel");
    assert_eq!(panel.items().len(), 1);
    let area = Rect::new(0, 0, 40, 10);
    let layout = refer_layout(&prompt, area).expect("refer layout");
    assert_eq!(
        refer_item_at(&prompt, area, layout.content.x + 1, layout.content.y),
        Some(0)
    );
}
