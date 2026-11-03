//! 单元测试；仅测试构建编译。

use super::*;
use crate::app::markdown::MentionEntry;
use crate::app::markdown::TemplateStatus;
use crate::app::state::AppState;
use crate::app::update;
use crate::layout::PaneId;
use ratatui::style::{Color, Modifier};

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

    // 编辑器高亮是**独立于阅读渲染**的 ANSI 语义色板（与 `md_read_*` 无关）：
    // 这里用真实色值钉死，防止有人再把两侧合并。
    assert_eq!(buf[(0, 0)].symbol(), "#");
    // 标题的 `#` 有专用高亮（Yellow + dim），不再并入通用 dim 标记。
    assert_eq!(
        buf[(0, 0)].fg,
        style::markdown_heading_marker().fg.unwrap_or(Color::Reset),
        "标题 `#` 走 markdown_heading_marker"
    );
    assert!(
        buf[(0, 0)].modifier.contains(Modifier::DIM),
        "标题标记压暗以区别于正文"
    );
    assert_eq!(buf[(3, 0)].symbol(), "t");
    assert_eq!(buf[(3, 0)].fg, Color::Yellow, "标题着色（Yellow）");
    assert!(buf[(3, 0)].modifier.contains(Modifier::BOLD));

    assert_eq!(buf[(0, 1)].symbol(), "*");
    assert_eq!(buf[(2, 1)].symbol(), "b");
    assert_eq!(buf[(2, 1)].fg, Color::Yellow, "粗体 Yellow");
    assert!(buf[(2, 1)].modifier.contains(Modifier::BOLD));

    assert_eq!(buf[(1, 2)].symbol(), "c");
    assert_eq!(buf[(1, 2)].fg, Color::LightRed, "行内代码亮红");
    assert_eq!(buf[(1, 2)].bg, Color::Reset, "行内代码不铺背景");

    assert_eq!(buf[(2, 3)].symbol(), "q");
    assert_eq!(buf[(2, 3)].fg, Color::LightMagenta, "引用亮品红");
    assert!(buf[(2, 3)].modifier.contains(Modifier::ITALIC), "引用斜体");

    assert_eq!(buf[(2, 4)].symbol(), "i");
    assert_eq!(buf[(2, 4)].fg, Color::Reset);
    assert!(!buf[(2, 4)].modifier.contains(Modifier::DIM));

    assert_eq!(buf[(1, 5)].symbol(), "a");
    assert_eq!(buf[(1, 5)].fg, Color::LightBlue, "链接文字亮蓝");
    assert!(buf[(1, 5)].modifier.contains(Modifier::UNDERLINED));
    assert_eq!(buf[(4, 5)].symbol(), "u");
    assert_eq!(buf[(4, 5)].fg, Color::Cyan, "URL 青色");
}

#[test]
fn fence_lines_are_markers_and_content_stays_default() {
    let area = Rect::new(0, 0, 10, 4);
    let prompt = prompt(10, 4, "```\nx\n```");
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &prompt, None);
    assert_eq!(
        buf[(0, 0)].fg,
        style::markdown_marker().fg.unwrap_or(Color::Reset),
        "围栏标记走编辑器 markdown_marker（dim）"
    );
    assert_eq!(buf[(0, 1)].symbol(), "x");
    assert_eq!(buf[(0, 1)].fg, Color::Reset, "代码正文仍用终端默认前景");
    assert!(!buf[(0, 1)].modifier.contains(Modifier::DIM));
    assert_eq!(
        buf[(0, 2)].fg,
        style::markdown_marker().fg.unwrap_or(Color::Reset)
    );
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
        style::markdown_refer_folder().fg.unwrap_or(Color::Reset),
        "引用整段着 refer-folder 色"
    );
    assert!(buf[(0, 1)].modifier.contains(Modifier::UNDERLINED));
    assert_eq!(buf[(7, 1)].symbol(), "/");
    assert_eq!(
        buf[(7, 1)].fg,
        style::markdown_refer_folder().fg.unwrap_or(Color::Reset),
        "路径与标记同色"
    );
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
    let area = Rect::new(0, 0, 30, 12);
    let mut editor = Prompt::new(PaneId::from_raw_for_test(1));
    editor.resize(30, 12);
    editor.insert_str("#");
    assert!(editor.panel().is_some());
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &editor, None);

    assert_eq!(buf[(1, 1)].symbol(), "┌");
    assert_eq!(buf[(1, 3)].symbol(), "│");
    assert_eq!(buf[(1, 1)].bg, Color::Reset);
    let row: String = (0..area.width).map(|x| buf[(x, 4)].symbol()).collect();
    assert!(row.contains("Heading 1"));
    assert_eq!(buf[(3, 4)].bg, Color::Indexed(240));
    assert_eq!(buf[(3, 5)].bg, Color::Reset);

    let mut empty = Buffer::empty(area);
    let blank = prompt(10, 3, "plain");
    render(area, &mut empty, &blank, None);
    assert_eq!(empty[(1, 1)].symbol(), " ");
}

#[test]
fn block_panel_renders_commands_title() {
    let area = Rect::new(0, 0, 30, 12);
    let mut editor = Prompt::new(PaneId::from_raw_for_test(1));
    editor.resize(30, 12);
    editor.insert_str("#");
    let rect = panel_rect(&editor, area).expect("panel visible");
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &editor, None);

    let top: String = (0..area.width)
        .map(|x| buf[(x, rect.y + 1)].symbol())
        .collect();
    assert!(top.contains(text::BLOCK_PANEL_TITLE), "{top}");
}

#[test]
fn block_panel_rect_and_items_hit_test() {
    let area = Rect::new(0, 0, 30, 14);
    let mut editor = Prompt::new(PaneId::from_raw_for_test(1));
    editor.resize(30, 14);
    editor.insert_str("#");
    let rect = panel_rect(&editor, area).expect("panel visible");
    assert_eq!(rect.y, 1, "面板贴在锚点行下方");
    assert_eq!(panel_item_at(&editor, area, 2, rect.y + 3), Some(0));
    assert_eq!(panel_item_at(&editor, area, 2, rect.y + 4), Some(1));
    assert_eq!(panel_item_at(&editor, area, 2, rect.y + 8), Some(5));
    assert_eq!(
        panel_item_at(&editor, area, 2, rect.y + 9),
        None,
        "底栏分割线不命中"
    );
    assert_eq!(
        panel_item_at(&editor, area, 2, rect.y + 2),
        None,
        "顶栏分割线不命中"
    );
}

#[test]
fn block_panel_window_keeps_still_when_hovering_visible_item() {
    let area = Rect::new(0, 0, 30, 11);
    let mut editor = Prompt::new(PaneId::from_raw_for_test(1));
    editor.resize(30, 11);
    editor.insert_str("#");
    editor.panel_move(5);
    let rect = panel_rect(&editor, area).expect("panel visible");
    let top = panel_item_at(&editor, area, 2, rect.y + 3);

    // 悬停窗口内条目：窗口保持不动（锚点不跟随悬停）。
    assert!(editor.panel_set_active(3));
    assert_eq!(editor.panel().map(|panel| panel.active()), Some(3));
    assert_eq!(panel_rect(&editor, area), Some(rect));
    assert_eq!(panel_item_at(&editor, area, 2, rect.y + 3), top);

    // 键盘移动：锚点跟随高亮，窗口滚动。
    editor.panel_move(-1);
    assert_ne!(panel_item_at(&editor, area, 2, rect.y + 3), top);
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

    assert_eq!(buf[(1, 1)].symbol(), "┌");
    assert_eq!(buf[(1, 1)].bg, Color::Reset);
    let name_row: String = (0..area.width).map(|x| buf[(x, 4)].symbol()).collect();
    let dir_row: String = (0..area.width).map(|x| buf[(x, 5)].symbol()).collect();
    assert!(name_row.contains("app.rs"));
    assert!(name_row.contains(text::MENTION_FILE_ICON), "{name_row}");
    assert_eq!(buf[(4, 4)].symbol(), text::MENTION_FILE_ICON);
    assert!(buf[(4, 4)].modifier.contains(Modifier::DIM));
    assert!(dir_row.contains("src"));
    assert!(dir_row.contains(text::WORKSPACE_TREE_LAST), "{dir_row}");
    assert!(buf[(4, 5)].modifier.contains(Modifier::DIM));
    assert_eq!(buf[(3, 4)].bg, Color::Indexed(240));

    assert_eq!(mention_item_at(&editor, area, 2, 4), Some(0));
    assert_eq!(mention_item_at(&editor, area, 2, 5), Some(0));
    assert_eq!(mention_item_at(&editor, area, 2, 6), Some(1));
    assert_eq!(mention_item_at(&editor, area, 2, 7), Some(1));
    assert_eq!(mention_item_at(&editor, area, 2, 8), None);
    assert_eq!(mention_item_at(&editor, area, 2, 3), None);
}

#[test]
fn template_separator_renders_block_border_line() {
    let area = Rect::new(0, 0, 30, 10);
    let mut editor = Prompt::new(PaneId::from_raw_for_test(1));
    editor.resize(30, 10);
    editor.set_text("&&& addTemplate --start\n{title: }\n\nbody\n&&& addTemplate --end\n");
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &editor, None);
    // y = 0 为顶边行，y = 3 为标题下分割线：首列画色条 `▎`，其余列画满 `─`。
    let row: String = (0..area.width).map(|x| buf[(x, 3)].symbol()).collect();
    assert!(row.starts_with("▎──"), "{row}");
    assert!(row.ends_with("─"), "{row}");
    assert_eq!(buf[(8, 3)].fg, style::LX_PINK, "&&& 分割线为 lx 粉");
}

#[test]
fn template_bottom_gap_renders_as_plain_blank_row() {
    let area = Rect::new(0, 0, 30, 8);
    let editor = prompt(30, 8, "&&& x --start\n{title: }\n\nbody\n\n&&& x --end");
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &editor, None);
    // 标题下空行（y = 3）渲染为分割线；结束行上方的块底部空行（y = 5）按普通空行渲染（只有首列色条）。
    let title_sep: String = (0..area.width).map(|x| buf[(x, 3)].symbol()).collect();
    assert!(title_sep.starts_with("▎──"), "{title_sep}");
    assert!(title_sep.ends_with("─"), "{title_sep}");
    let bottom_row: String = (0..area.width).map(|x| buf[(x, 5)].symbol()).collect();
    assert_eq!(buf[(0, 5)].symbol(), "▎", "{bottom_row}");
    assert_eq!(buf[(29, 5)].symbol(), " ", "{bottom_row}");
    assert!(
        !bottom_row.contains('─'),
        "底部空行不画分割线: {bottom_row}"
    );
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
    assert!(
        painted.contains("Files") && painted.contains("wt"),
        "标题带取根标识: {painted}"
    );

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
    assert!(!painted.contains("wt"), "未绑定无标题后缀: {painted}");
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

    let dir_row: String = (0..area.width).map(|x| buf[(x, 4)].symbol()).collect();
    let file_row: String = (0..area.width).map(|x| buf[(x, 5)].symbol()).collect();
    assert!(dir_row.contains(text::MENTION_DIR_ICON), "{dir_row}");
    assert!(dir_row.contains("src/"), "{dir_row}");
    assert!(file_row.contains(text::MENTION_FILE_ICON), "{file_row}");
    assert!(file_row.contains("app.rs"), "{file_row}");
    assert_ne!(text::MENTION_DIR_ICON, text::MENTION_FILE_ICON);
    // 目录条目按目录色（黄色）着色，文件条目保持默认前景。
    assert_eq!(buf[(4, 4)].fg, Color::Yellow, "目录图标用目录色");
    assert_eq!(buf[(6, 4)].fg, Color::Yellow, "目录名称用目录色");
    assert_eq!(buf[(6, 5)].fg, Color::Reset, "文件名称沿用终端默认前景");
    assert!(!buf[(6, 5)].modifier.contains(Modifier::BOLD));
}

#[test]
fn mention_panel_renders_title_folder_name_and_footer() {
    let area = Rect::new(0, 0, 50, 20);
    let mut editor = Prompt::new(PaneId::from_raw_for_test(1));
    editor.resize(50, 20);
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

    // 根范围：左上 @Command 与 Tab，底边提示，右上不显示目录名。
    let root_rect = mention_panel_rect(&editor, area).expect("panel visible");
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &editor, None);
    let root_top: String = (0..area.width)
        .map(|x| buf[(x, root_rect.y + 1)].symbol())
        .collect();
    let root_bottom: String = (0..area.width)
        .map(|x| buf[(x, root_rect.bottom() - 2)].symbol())
        .collect();
    assert!(root_top.contains("@Command"), "{root_top}");
    assert!(root_top.contains(text::MENTION_TAB_FILES), "{root_top}");
    assert!(root_top.contains(text::MENTION_TAB_REF_DIR), "{root_top}");
    assert!(root_bottom.contains("tab"), "{root_bottom}");

    // 进入 src：右上显示目录名，底边提示保持。
    assert!(editor.mention_enter_folder());
    let rect = mention_panel_rect(&editor, area).expect("scoped panel visible");
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &editor, None);
    let top: String = (0..area.width)
        .map(|x| buf[(x, rect.y + 1)].symbol())
        .collect();
    let bottom: String = (0..area.width)
        .map(|x| buf[(x, rect.bottom() - 2)].symbol())
        .collect();
    assert!(top.contains("@Command"), "{top}");
    assert!(top.contains(text::MENTION_TAB_FILES), "{top}");
    assert!(top.contains("src"), "{top}");
    assert!(bottom.contains("tab"), "{bottom}");
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
        .map(|x| buf[(x, rect.y + 3)].symbol())
        .collect();
    let dir_row: String = (0..area.width)
        .map(|x| buf[(x, rect.y + 4)].symbol())
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
    let area = Rect::new(0, 0, 30, 28);
    let mut editor = Prompt::new(PaneId::from_raw_for_test(1));
    editor.resize(30, 28);
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
    assert_eq!(mention_item_at(&editor, area, 2, rect.y + 3), Some(6));

    // 悬停窗口内条目：窗口保持不动，不因悬停向上滚动。
    editor.mention_set_active(8);
    assert_eq!(mention_panel_rect(&editor, area), Some(rect));
    assert_eq!(mention_item_at(&editor, area, 2, rect.y + 3), Some(6));
    assert_eq!(mention_item_at(&editor, area, 2, rect.y + 9), Some(9));
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
        false,
        None,
    );

    let bar = crate::layout::prompt_toolbar_rect(panel).expect("toolbar visible");
    let render_label = |buf: &Buffer, rect: Rect| -> String {
        (rect.x..rect.right())
            .map(|x| buf[(x, rect.y)].symbol())
            .collect()
    };
    let recent = crate::layout::prompt_toolbar_button_rect(
        panel,
        crate::layout::PromptToolbarButton::Recent,
        PromptMode::Editor,
        Some("1/2"),
    )
    .expect("recent visible");
    assert_eq!(render_label(&buf, recent), text::PROMPT_RECENT_LABEL);
    assert_ne!(
        buf[(recent.x, bar.y)].fg,
        Color::Cyan,
        "未展开抽屉时不高亮青色"
    );
    assert!(buf[(recent.x, bar.y)].modifier.contains(Modifier::DIM));
    paint_recent_button(panel, &mut buf, PromptMode::Editor, Some("1/2"), true);
    assert_eq!(
        buf[(recent.x, bar.y)].fg,
        Color::Cyan,
        "展开抽屉后高亮为青色"
    );
    assert!(buf[(recent.x, bar.y)].modifier.contains(Modifier::BOLD));

    let undo = crate::layout::prompt_toolbar_button_rect(
        panel,
        crate::layout::PromptToolbarButton::Undo,
        PromptMode::Editor,
        Some("1/2"),
    )
    .expect("undo visible");
    assert_eq!(render_label(&buf, undo), text::PROMPT_UNDO_LABEL);
    assert_eq!(buf[(undo.x, bar.y)].fg, Color::Cyan, "可撤销时强调色");
    assert!(buf[(undo.x, bar.y)].modifier.contains(Modifier::BOLD));

    let redo = crate::layout::prompt_toolbar_button_rect(
        panel,
        crate::layout::PromptToolbarButton::Redo,
        PromptMode::Editor,
        Some("1/2"),
    )
    .expect("redo visible");
    assert_eq!(render_label(&buf, redo), text::PROMPT_REDO_LABEL);
    assert!(buf[(redo.x, bar.y)].modifier.contains(Modifier::DIM));

    let save = crate::layout::prompt_toolbar_button_rect(
        panel,
        crate::layout::PromptToolbarButton::Save,
        PromptMode::Editor,
        Some("1/2"),
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
        false,
        None,
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
        true,
        None,
    );
    assert_eq!(buf[(save_x, bar.y)].symbol(), text::PROMPT_DRAFT_DOT);
    assert_eq!(buf[(save_x, bar.y)].fg, Color::DarkGray, "草稿恒灰空心点");
}

#[test]
fn divider_stays_plain_and_border_carries_the_title() {
    let panel = toolbar_panel();
    let editor = Prompt::new(PaneId::from_raw_for_test(1));
    let divider_y = crate::layout::prompt_toolbar_rect(panel)
        .expect("toolbar visible")
        .y
        + crate::layout::PROMPT_TOOLBAR_HEIGHT;
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
        false,
        None,
    );
    let row = divider_row(&buf);
    assert_eq!(
        row,
        format!(
            "{}{}{}",
            text::DIVIDER_LEFT_JOIN,
            text::DIVIDER_MID.repeat(usize::from(panel.width) - 2),
            text::DIVIDER_RIGHT_JOIN
        ),
        "分割线保持空行、不再承载文字: {row}"
    );
}

#[test]
fn toolbar_folders_button_renders_left_of_page_controls_and_hides_in_tree() {
    let panel = toolbar_panel();
    let editor = Prompt::new(PaneId::from_raw_for_test(1));
    let page = page_data(1, 2, "Page 1");
    let counter = Some("1/2");
    let folders = crate::layout::prompt_toolbar_button_rect(
        panel,
        crate::layout::PromptToolbarButton::Folders,
        PromptMode::Editor,
        counter,
    )
    .expect("folders visible");
    let controls = toolbar_page_controls(
        crate::layout::prompt_toolbar_rect(panel).expect("bar"),
        "1/2",
        true,
    )
    .expect("controls visible");
    assert_eq!(folders.right() + 1, controls.prev.x, "紧邻页控件左侧");
    let mut buf = Buffer::empty(panel);
    render_header(
        panel,
        &mut buf,
        &editor,
        false,
        PromptMode::Editor,
        false,
        Some(&page),
    );
    let label: String = (folders.x..folders.right())
        .map(|x| buf[(x, folders.y)].symbol())
        .collect();
    assert_eq!(label, text::PROMPT_FOLDERS_LABEL);

    // 树视图不显示引用文件夹按钮。
    let mut buf = Buffer::empty(panel);
    render_header(
        panel,
        &mut buf,
        &editor,
        false,
        PromptMode::Tree,
        false,
        None,
    );
    let tree_folders = crate::layout::prompt_toolbar_button_rect(
        panel,
        crate::layout::PromptToolbarButton::Folders,
        PromptMode::Tree,
        None,
    );
    assert!(tree_folders.is_none());
}

#[test]
fn divider_stays_muted_across_focus_states() {
    let panel = toolbar_panel();
    let editor = Prompt::new(PaneId::from_raw_for_test(1));
    let divider_y = crate::layout::prompt_toolbar_rect(panel)
        .expect("toolbar visible")
        .y
        + crate::layout::PROMPT_TOOLBAR_HEIGHT;

    for focused in [true, false] {
        let mut buf = Buffer::empty(panel);
        render_header(
            panel,
            &mut buf,
            &editor,
            focused,
            PromptMode::Editor,
            false,
            None,
        );
        for x in [panel.x, 5, panel.right() - 1] {
            let cell = &buf[(x, divider_y)];
            assert_ne!(cell.fg, Color::Cyan, "分割线恒为暗灰、不随焦点变色: x={x}");
            assert!(cell.modifier.contains(Modifier::DIM));
            assert!(!cell.modifier.contains(Modifier::BOLD));
        }
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
        false,
        None,
    );
    assert_eq!(buf[(1, 1)].symbol(), " ", "内容区过矮不绘制工具栏");
    assert_eq!(buf[(2, 1)].symbol(), " ");
}

// ---- 斜杠命令面板与模板块按钮 ----

#[test]
fn slash_panel_renders_templates_title_and_hit_tests_items() {
    let area = Rect::new(0, 0, 60, 10);
    let prompt = prompt(60, 10, "/");
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
    let area = Rect::new(0, 0, 60, 5);
    let editor = prompt(
        60,
        5,
        "&&& addTemplate --start {title: }\n&&& addTemplate --end\n@src/app.rs",
    );
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &editor, None);

    // y = 0 为顶部边框，y = 1 为模板块起始文本行（左侧色条占第 0 列，固定为 lx 品牌粉）。
    assert_eq!(buf[(0, 0)].symbol(), text::PROMPT_BLOCK_TOP_BORDER);
    assert_eq!(buf[(0, 0)].fg, style::LX_PINK);
    assert_eq!(buf[(0, 1)].symbol(), "▎");
    assert_eq!(buf[(0, 1)].fg, style::LX_PINK, "&&& 色条固定 lx 粉");

    assert!(buf[(1, 1)].modifier.contains(Modifier::DIM), "&&& 为 dim");
    assert_eq!(buf[(5, 1)].symbol(), "a");
    assert_eq!(buf[(5, 1)].fg, Color::Green, "add 命令为绿色");
    assert!(buf[(5, 1)].modifier.contains(Modifier::BOLD));
    assert_eq!(buf[(17, 1)].symbol(), "-", "--start 为 dim 标记");
    assert!(buf[(17, 1)].modifier.contains(Modifier::DIM));

    assert_eq!(buf[(25, 1)].symbol(), "{");
    assert_eq!(buf[(25, 1)].fg, Color::Cyan, "标题占位符为 Cyan");
    assert!(buf[(25, 1)].modifier.contains(Modifier::UNDERLINED));

    // 非模板块行不缩进、无色条。
    assert_eq!(buf[(0, 3)].symbol(), "@");
    assert_eq!(
        buf[(0, 3)].fg,
        style::markdown_file_mention().fg.unwrap_or(Color::Reset),
        "@文件与其它引用共用一档灰绿"
    );
    assert!(buf[(0, 3)].modifier.contains(Modifier::UNDERLINED));
}

#[test]
fn template_block_draws_status_colored_bar() {
    let area = Rect::new(0, 0, 60, 5);
    let running = prompt(
        60,
        5,
        "&&& bugTemplate --start {title: }\n# Fix\n&&& bugTemplate --end in_progress",
    );
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &running, None);

    assert_eq!(buf[(0, 0)].symbol(), text::PROMPT_BLOCK_TOP_BORDER);
    assert_eq!(buf[(0, 0)].fg, style::LX_PINK);
    for y in 1..4 {
        assert_eq!(buf[(0, y)].symbol(), "▎");
        assert_eq!(buf[(0, y)].fg, style::LX_PINK, "&&& 色条固定 lx 粉");
    }
    assert_eq!(buf[(59, 2)].symbol(), " ", "右侧不再绘制竖框");
    assert_eq!(buf[(40, 3)].symbol(), " ", "结束行不再补 ─");
    assert_eq!(buf[(0, 4)].symbol(), " ");

    let done = prompt(
        60,
        5,
        "&&& bugTemplate --start {title: }\n&&& bugTemplate --end done",
    );
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &done, None);
    assert_eq!(buf[(0, 1)].fg, style::LX_PINK, "&&& 色条固定 lx 粉");
    assert_eq!(buf[(0, 2)].fg, style::LX_PINK);
}

#[test]
fn template_block_keeps_transparent_background() {
    let area = Rect::new(0, 0, 60, 5);
    for text in [
        "&&& bugTemplate --start {title: }\n# Fix\n&&& bugTemplate --end",
        "&&& bugTemplate --start {title: }\n# Fix\n&&& bugTemplate --end in_progress",
        "&&& bugTemplate --start {title: }\n# Fix\n&&& bugTemplate --end done",
    ] {
        let editor = prompt(60, 5, text);
        let mut buf = Buffer::empty(area);
        render(area, &mut buf, &editor, None);
        for y in 0..5 {
            for x in 0..area.width {
                assert_eq!(buf[(x, y)].bg, Color::Reset, "{text:?} ({x},{y})");
            }
        }
    }
}

#[test]
fn template_block_selection_uses_reversed_modifier_on_transparent_background() {
    let area = Rect::new(0, 0, 60, 5);
    let editor = prompt(
        60,
        5,
        "&&& bugTemplate --start {title: }\n# Fix\n&&& bugTemplate --end in_progress",
    );
    let selection = selection(&editor, (2, 3), (2, 8));
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &editor, Some(&selection));
    assert_eq!(buf[(5, 2)].bg, Color::Reset);
    assert!(buf[(5, 2)].modifier.contains(Modifier::REVERSED));
    assert_eq!(buf[(20, 2)].bg, Color::Reset);
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
    assert_eq!(buttons.len(), 3, "复制、清理、删除");
    assert_eq!(buttons[0].button, TemplateBlockButton::Copy);
    assert_eq!(buttons[1].button, TemplateBlockButton::Clean);
    assert_eq!(buttons[2].button, TemplateBlockButton::Delete);
    assert_eq!(buttons[2].rect.right(), 70, "按钮组右对齐到文本区右边界");

    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &prompt, None);
    let row: String = (0..area.width).map(|x| buf[(x, 1)].symbol()).collect();
    assert!(
        !row.contains(text::TEMPLATE_BUTTON_TODO),
        "状态标签不在块内: {row}"
    );
    assert!(row.contains(text::TEMPLATE_BUTTON_COPY), "{row}");
    assert!(row.contains(text::TEMPLATE_BUTTON_CLEAN), "{row}");
    assert!(row.contains(text::TEMPLATE_BUTTON_DEL), "{row}");
    assert_eq!(buf[(69, 1)].symbol(), "]");

    for hit in &buttons {
        let cell = hit.rect;
        assert_eq!(
            template_button_at(&prompt, area, cell.x, cell.y),
            Some(*hit)
        );
    }
    assert_eq!(template_button_at(&prompt, area, 0, 1), None);
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
    assert_eq!(buttons.len(), 3, "窄屏按钮不隐藏");
    assert!(buttons[0].rect.x > area.x, "按钮让开左侧色条");
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &editor, None);
    let row: String = (0..area.width).map(|x| buf[(x, 1)].symbol()).collect();
    assert!(row.contains(text::TEMPLATE_BUTTON_COPY), "{row}");
    assert!(row.contains(text::TEMPLATE_BUTTON_DEL), "{row}");
    let text_side: String = (0..buttons[0].rect.x)
        .map(|x| buf[(x, 1)].symbol())
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
    let row: String = (0..narrow.width).map(|x| buf[(x, 1)].symbol()).collect();
    assert!(row.contains(text::TEMPLATE_BUTTON_DEL), "{row}");
}

#[test]
fn block_row_never_renders_template_status_label() {
    let area = Rect::new(0, 0, 70, 10);
    for (text, status_label) in [
        (
            "&&& bugTemplate --start {title: }\n&&& bugTemplate --end done",
            text::TEMPLATE_BUTTON_DONE,
        ),
        (
            "&&& bugTemplate --start {title: }\n&&& bugTemplate --end in_progress",
            text::TEMPLATE_BUTTON_RUN,
        ),
    ] {
        let prompt = prompt(70, 10, text);
        let mut buf = Buffer::empty(area);
        render(area, &mut buf, &prompt, None);
        let row: String = (0..area.width).map(|x| buf[(x, 1)].symbol()).collect();
        assert!(!row.contains(status_label), "状态标签只出现在标题行: {row}");
        assert_eq!(template_buttons(&prompt, area).len(), 3);
    }
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

#[test]
fn supple_block_renders_magenta_and_three_buttons() {
    use crate::app::markdown::BlockKind;
    let area = Rect::new(0, 0, 60, 10);
    let prompt = prompt(
        60,
        10,
        concat!(
            "&&& x --start\n",
            "{title: }\n",
            "&&& x --end\n",
            "\n",
            "+++ supplement --start\n",
            "- a\n",
            "+++ supplement --end",
        ),
    );
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &prompt, None);
    assert_eq!(buf[(0, 0)].fg, style::LX_PINK, "模板块顶边固定 lx 粉");
    assert_eq!(buf[(0, 1)].fg, style::LX_PINK, "模板块固定 lx 粉");
    assert_eq!(buf[(0, 5)].fg, Color::LightMagenta, "+++ 顶边品红色");
    assert_eq!(buf[(0, 6)].fg, Color::LightMagenta, "+++ 品红色");
    assert_eq!(buf[(2, 7)].bg, Color::Reset, "补充块保持透明底色");

    let hits = block_buttons(&prompt, area);
    assert_eq!(hits.len(), 3, "补充块复制、清理与删除三个按钮");
    assert!(hits.iter().all(|hit| hit.kind == BlockKind::Supple));
    assert_eq!(hits[0].button, BlockButton::Copy);
    assert_eq!(hits[1].button, BlockButton::Clean);
    assert_eq!(hits[2].button, BlockButton::Delete);
    let hit = hits[1];
    assert_eq!(
        block_button_at(&prompt, area, hit.rect.x, hit.rect.y).map(|hit| hit.button),
        Some(BlockButton::Clean)
    );
}

#[test]
fn refer_panel_width_grows_with_content_up_to_container() {
    let area = Rect::new(0, 0, 60, 14);
    let short = prompt(60, 14, "@a.txt one\nat");
    let narrow = refer_layout(&short, area)
        .expect("short refer panel")
        .rect
        .width;
    let long = prompt(
        60,
        14,
        "@a/very/long/path/to/some/deep/module/name.txt one\nname",
    );
    let wide = refer_layout(&long, area)
        .expect("long refer panel")
        .rect
        .width;
    assert!(
        wide > narrow,
        "panel grows with content: {narrow} -> {wide}"
    );
    assert!(wide <= area.width, "panel never exceeds container: {wide}");
    let huge = prompt(
        60,
        14,
        &format!("@{}verylongname.txt one\nvery", "p/".repeat(60)),
    );
    let capped = refer_layout(&huge, area)
        .expect("huge refer panel")
        .rect
        .width;
    assert_eq!(
        capped,
        area.width - crate::layout::PROMPT_BLOCK_GUTTER_LEFT,
        "capped at container width minus gutter"
    );
}

fn assert_full_width_separator(buf: &Buffer, width: u16, y: u16) {
    let separator: String = (0..width).map(|x| buf[(x, y)].symbol()).collect();
    let chars: Vec<char> = separator.chars().collect();
    assert_eq!(chars.first(), Some(&'▎'), "sep={separator:?}");
    assert!(chars[1..].iter().all(|ch| *ch == '─'), "sep={separator:?}");
}

#[test]
fn supple_block_renders_title_separator_like_template() {
    let area = Rect::new(0, 0, 40, 12);
    let text = "+++ supplement --start\n{title: }\n\n## S\n+++ supplement --end";
    let prompt = prompt(40, 12, text);
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &prompt, None);
    assert_full_width_separator(&buf, 40, 3);
}

// ===== 标题行标题与页控件 =====

/// 标题行渲染工具：先画直角边框与分割线，再把标题覆盖到分割线行，
/// 返回标题行整行字符串（含两端 `├` / `┤`）。
fn title_row_render(panel: Rect, data: &TitleRowData<'_>) -> String {
    title_row_render_with_status(panel, data, None)
}

/// 同上，附加本页模板块状态标签。
fn title_row_render_with_status(
    panel: Rect,
    data: &TitleRowData<'_>,
    status: Option<TemplateStatus>,
) -> String {
    let mut buf = Buffer::empty(panel);
    let block = ratatui::widgets::Block::bordered();
    ratatui::widgets::Widget::render(block, panel, &mut buf);
    if let Some(header) = crate::layout::prompt_header_rect(panel) {
        render_divider(panel, header, &mut buf, false);
    }
    let row = crate::ui::prompt_frame::title_row(panel).expect("title row");
    paint_title_row(&mut buf, row, data, status);
    (panel.x..panel.right())
        .map(|x| buf[(x, row.y)].symbol())
        .collect()
}

/// 页数据简写。
fn page_data(current: usize, total: usize, name: &str) -> PageHeaderData<'_> {
    PageHeaderData {
        current,
        total,
        name,
        arrows: true,
    }
}

#[test]
fn title_row_shows_item_name_and_page_title_only() {
    let panel = Rect::new(0, 0, 56, 8);
    let row = title_row_render(
        panel,
        &TitleRowData {
            title: Some("named prompt"),
            draft: false,
            page: Some(page_data(2, 4, "Intro")),
        },
    );
    assert!(row.contains(" named prompt "), "{row}");
    assert!(row.contains("· Intro"), "页名跟随条目名: {row}");
    assert!(!row.contains("2/4"), "页码已移到工具栏: {row}");
    assert!(
        !row.contains('‹') && !row.contains('›'),
        "标题行无箭头: {row}"
    );
    // 落在分割线行上：两端以 `├` / `┤` 衔接边框。
    assert!(row.starts_with(text::DIVIDER_LEFT_JOIN), "{row}");
    assert!(row.ends_with(text::DIVIDER_RIGHT_JOIN), "{row}");
}

#[test]
fn toolbar_page_control_hit_areas() {
    let panel = Rect::new(0, 0, 56, 8);
    let bar = crate::layout::prompt_toolbar_rect(panel).expect("toolbar");
    let data = TitleRowData {
        title: Some("named prompt"),
        draft: false,
        page: Some(page_data(2, 4, "Intro")),
    };
    let controls = toolbar_page_controls(bar, "2/4", true).expect("controls visible");
    for (rect, expected) in [
        (controls.prev, PageControl::Prev),
        (controls.counter, PageControl::Counter),
        (controls.next, PageControl::Next),
    ] {
        assert_eq!(page_control_at(bar, &data, rect.x, bar.y), Some(expected));
    }
    // 标题行页名为纯展示；工具栏左侧空白也不产生控件命中。
    let title = crate::ui::prompt_frame::title_row(panel).expect("title row");
    assert_eq!(
        page_control_at(bar, &data, bar.x + 3, bar.y),
        None,
        "左侧按钮区不响应页控件"
    );
    assert_eq!(
        page_control_at(bar, &data, controls.prev.x, title.y),
        None,
        "非工具栏行不响应"
    );
}

#[test]
fn toolbar_page_controls_degrade_when_narrow() {
    let narrow = crate::layout::prompt_toolbar_rect(Rect::new(0, 0, 12, 8)).expect("toolbar");
    assert!(
        toolbar_page_controls(narrow, "1/2", true).is_none(),
        "放不下整组时隐藏"
    );
    let wide = crate::layout::prompt_toolbar_rect(Rect::new(0, 0, 56, 8)).expect("toolbar");
    assert!(toolbar_page_controls(wide, "1/2", true).is_some());
    assert!(
        toolbar_page_controls(wide, "999/1000", true).is_some(),
        "页码变宽仍完整容纳"
    );
}

#[test]
fn title_row_degrades_page_name_before_item_name() {
    // 宽面板：条目名完整，超长页名被截断。
    let wide = Rect::new(0, 0, 56, 8);
    let long_name = "y".repeat(80);
    let row = title_row_render(
        wide,
        &TitleRowData {
            title: Some("item"),
            draft: false,
            page: Some(page_data(1, 3, &long_name)),
        },
    );
    assert!(row.contains(" item "), "条目名优先完整: {row}");
    assert!(!row.contains(&long_name), "页名被截断: {row}");
    assert!(row.contains('…'), "页名以省略号收尾: {row}");

    // 极窄：只剩截断的条目名，两端衔接符不被压到。
    let narrow = Rect::new(0, 0, 10, 8);
    let row = title_row_render(
        narrow,
        &TitleRowData {
            title: Some("a-very-long-item-name"),
            draft: false,
            page: Some(page_data(1, 3, "Page 1")),
        },
    );
    assert!(row.starts_with(text::DIVIDER_LEFT_JOIN), "{row}");
    assert!(row.ends_with(text::DIVIDER_RIGHT_JOIN), "{row}");
    assert!(!row.contains("1/3"), "标题行不承载页码: {row}");
    assert!(row.contains('…'), "条目名截断: {row}");
}

#[test]
fn title_row_draft_uses_muted_style() {
    let panel = Rect::new(0, 0, 56, 8);
    let border = crate::ui::prompt_frame::title_row(panel).expect("title row");

    // 临时提示词：标题用未聚焦灰（显式前景 + dim），不被聚焦边框染色。
    let mut buf = Buffer::empty(panel);
    paint_title_row(
        &mut buf,
        border,
        &TitleRowData {
            title: Some(text::PROMPT_DRAFT_TITLE),
            draft: true,
            page: Some(page_data(1, 1, "Page 1")),
        },
        None,
    );
    let cell = &buf[(border.x + 2, border.y)];
    assert_eq!(cell.fg, Color::Reset, "草稿标题显式重置前景");
    assert!(cell.modifier.contains(Modifier::DIM));
}

#[test]
fn title_row_status_label_sits_at_right_edge() {
    let panel = Rect::new(0, 0, 56, 8);
    let border = crate::ui::prompt_frame::title_row(panel).expect("title row");
    let data = TitleRowData {
        title: Some("item"),
        draft: false,
        page: Some(page_data(2, 4, "Page 2")),
    };

    let mut buf = Buffer::empty(panel);
    paint_title_row(&mut buf, border, &data, Some(TemplateStatus::InProgress));
    let row: String = (panel.x..panel.right())
        .map(|x| buf[(x, border.y)].symbol())
        .collect();
    assert!(row.contains(text::TEMPLATE_BUTTON_RUN), "{row}");
    let byte = row.find(text::TEMPLATE_BUTTON_RUN).expect("status label");
    let x = panel.x + u16::try_from(row[..byte].chars().count()).expect("col fits");
    assert_eq!(buf[(x, border.y)].fg, Color::Yellow, "in_progress 标签黄色");
    let width = u16::try_from(text::TEMPLATE_BUTTON_RUN.chars().count()).expect("width");
    assert_eq!(
        x + width + 1,
        panel.right() - 1,
        "标签贴右端、留 1 列边框线"
    );

    // 三态各自颜色：todo 蓝、done 绿（与旧边框语义一致）。
    for (status, color) in [
        (TemplateStatus::Todo, Color::LightBlue),
        (TemplateStatus::Done, Color::Green),
    ] {
        let mut buf = Buffer::empty(panel);
        paint_title_row(&mut buf, border, &data, Some(status));
        assert_eq!(buf[(x, border.y)].fg, color, "{status:?} 标签颜色");
    }

    // 极窄：隐藏状态标签，优先保留标题。
    let narrow = Rect::new(0, 0, 10, 8);
    let narrow_border = crate::ui::prompt_frame::title_row(narrow).expect("title row");
    let mut buf = Buffer::empty(narrow);
    paint_title_row(&mut buf, narrow_border, &data, Some(TemplateStatus::Todo));
    let row: String = (narrow.x..narrow.right())
        .map(|x| buf[(x, narrow_border.y)].symbol())
        .collect();
    assert!(!row.contains(text::TEMPLATE_BUTTON_TODO), "{row}");
}

#[test]
fn tree_view_title_row_is_plain() {
    let panel = Rect::new(0, 0, 40, 8);
    let border = crate::ui::prompt_frame::title_row(panel).expect("title row");
    let data = TitleRowData {
        title: None,
        draft: false,
        page: None,
    };
    let row = title_row_render(panel, &data);
    assert!(row.starts_with(text::DIVIDER_LEFT_JOIN), "{row}");
    assert!(row.ends_with(text::DIVIDER_RIGHT_JOIN), "{row}");
    assert!(
        row.chars()
            .skip(1)
            .take(usize::from(panel.width) - 2)
            .all(|ch| ch == text::DIVIDER_MID.chars().next().unwrap_or('─')),
        "树视图标题行为纯 `─`: {row}"
    );
    assert!(
        !row.contains('‹') && !row.contains('›'),
        "列表页无页控件: {row}"
    );
    assert_eq!(
        page_control_at(border, &data, border.x + 3, border.y),
        None,
        "树视图标题行无任何页控件命中"
    );
}

#[test]
fn short_ref_renders_with_its_own_style() {
    let area = Rect::new(0, 0, 40, 4);
    let editor = prompt(40, 4, "@ref/a/b.md");
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &editor, None);
    let cell = &buf[(0, 0)];
    assert_eq!(cell.fg, Color::Indexed(141), "柔紫");
    assert!(cell.modifier.contains(Modifier::BOLD), "加粗");
    assert!(
        !cell.modifier.contains(Modifier::UNDERLINED),
        "无下划线，区别于 refer-*"
    );
    // 与同类既有引用色都不同。
    assert_ne!(cell.fg, Color::Yellow, "不与 @ 提及黄混");
    assert_ne!(cell.fg, Color::LightGreen);
    assert_ne!(cell.fg, Color::Indexed(208));
    assert_ne!(cell.fg, Color::LightMagenta);
}

#[test]
fn editor_tokens_style_inline_code_and_list_markers() {
    // prompt 容器与 lx 输入框共用 `token_style`：两处必须同时生效。
    let inline = token_style(crate::ui::markdown::TokenKind::InlineCode);
    assert_eq!(inline.fg, Some(Color::LightRed), "行内代码前景");
    assert_eq!(inline.bg, None, "行内代码不铺背景（恢复原格式）");

    let bullet = token_style(crate::ui::markdown::TokenKind::ListBullet);
    let ordered = token_style(crate::ui::markdown::TokenKind::ListOrdered);
    assert_eq!(bullet.fg, Some(Color::Yellow), "无序符号强调色");
    assert_eq!(ordered.fg, Some(Color::Yellow), "有序序号强调色");
    assert!(
        bullet.add_modifier.contains(Modifier::BOLD),
        "列表符号加粗以便扫读"
    );
    assert_eq!(bullet, ordered, "两种列表同一强调色");

    // 任务复选框按完成状态分色（编辑侧此前被当成普通无序列表）。
    let open = token_style(crate::ui::markdown::TokenKind::TaskOpen);
    let done = token_style(crate::ui::markdown::TokenKind::TaskDone);
    assert_eq!(open.fg, Some(Color::Yellow), "未完成复选框 = 黄");
    assert_eq!(done.fg, Some(Color::Green), "已完成复选框 = 绿");
}

#[test]
fn selection_reverse_keeps_inline_code_foreground() {
    // 行内代码现在只有前景色；选区反显只叠 REVERSED，前景不被清掉。
    let area = Rect::new(0, 0, 12, 3);
    let prompt = prompt(12, 3, "a `code` b");
    let mut buf = Buffer::empty(area);
    let selection = selection(&prompt, (0, 0), (0, 8));
    render(area, &mut buf, &prompt, Some(&selection));
    for col in 0..=8 {
        assert!(
            buf[(col, 0)].modifier.contains(Modifier::REVERSED),
            "column {col} 反显"
        );
    }
    // `code` 起始列（col 3）保留行内代码前景色。
    assert_eq!(
        buf[(3, 0)].fg,
        Color::LightRed,
        "选区反显不覆盖行内代码前景"
    );
}
