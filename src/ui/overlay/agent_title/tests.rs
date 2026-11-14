//! Agents 标题弹窗的几何、命中与渲染测试。

use super::*;

use crate::app::markdown::TemplateStatus;
use crate::app::overlay::TextInput;
use crate::app::prompt::{PromptPageId, RecentPageRow};
use crate::app::state::{AiWebSite, LxEntry, SidebarAgentKind};
use crate::tiling::PaneId;
use crate::ui::text;
use crate::ui::widgets;

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier};

const SCREEN: Rect = Rect {
    x: 0,
    y: 0,
    width: 80,
    height: 24,
};

fn row(index: i64, page_title: &str, prompt_name: &str) -> RecentPageRow {
    RecentPageRow {
        item_id: index,
        page_id: PromptPageId::Saved(index),
        prompt_name: prompt_name.into(),
        page_title: page_title.into(),
        status: None,
        updated_at: index,
        time_label: "2m".into(),
        current: false,
    }
}

fn row_with_status(
    index: i64,
    page_title: &str,
    prompt_name: &str,
    status: Option<TemplateStatus>,
) -> RecentPageRow {
    RecentPageRow {
        status,
        ..row(index, page_title, prompt_name)
    }
}

fn rows(count: usize) -> Vec<RecentPageRow> {
    (0..count)
        .map(|index| {
            row(
                index as i64,
                &format!("page-{index}"),
                &format!("prompt-{index}"),
            )
        })
        .collect()
}

fn dialog(rows: Vec<RecentPageRow>, selected: Option<usize>, input: &str) -> AgentTitleDialog {
    AgentTitleDialog {
        workspace: 0,
        tab: 0,
        pane: PaneId::alloc(),
        kind: SidebarAgentKind::Lx {
            entry: LxEntry::Native,
            ai_web_site: AiWebSite::Deepseek,
        },
        input: TextInput::new(input),
        binding: None,
        selected,
        rows,
    }
}

fn draw(
    dialog: &AgentTitleDialog,
    width: u16,
    height: u16,
) -> (Vec<String>, Buffer, Option<(u16, u16)>) {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("test backend");
    let mut cursor = None;
    terminal
        .draw(|frame| {
            cursor = render_agent_title(frame, frame.area(), dialog);
        })
        .expect("draw succeeds");
    let buffer = terminal.backend().buffer().clone();
    let lines = (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect()
        })
        .collect();
    (lines, buffer, cursor)
}

#[test]
fn shell_height_grows_with_rows_and_clamps() {
    let height = |count| {
        agent_title_shell(SCREEN, count)
            .expect("shell fits")
            .area
            .height
    };
    assert_eq!(height(0), 9);
    assert_eq!(height(1), 9);
    assert_eq!(height(2), 11);
    assert_eq!(height(3), 13);
    assert_eq!(height(4), 15);
    assert_eq!(height(5), 17);
    assert_eq!(height(6), 17, "超过 5 条按 5 条计算");
    assert_eq!(height(100), 17);

    let narrow = agent_title_shell(Rect::new(0, 0, 20, 8), 6).expect("narrow shell");
    assert_eq!(narrow.area.width, 20);
    assert_eq!(narrow.area.height, 8);
}

#[test]
fn max_rows_and_visible_start_follow_selection() {
    let shell = agent_title_shell(SCREEN, 2).expect("shell fits");
    assert_eq!(agent_title_max_rows(&shell), 2, "两条 Recent 需要同时可见");
    let shell = agent_title_shell(SCREEN, 3).expect("shell fits");
    assert_eq!(agent_title_max_rows(&shell), 3);

    let mut dialog = dialog(rows(6), Some(4), "");
    assert_eq!(agent_title_visible_start(&dialog, 3), 2);
    dialog.selected = None;
    assert_eq!(agent_title_visible_start(&dialog, 3), 0);
    dialog.selected = Some(1);
    assert_eq!(agent_title_visible_start(&dialog, 3), 0);
    assert_eq!(agent_title_visible_start(&dialog, 0), 0);
}

#[test]
fn row_hit_testing_maps_both_lines_and_scrolled_window() {
    let shell = agent_title_shell(SCREEN, 3).expect("shell fits");
    let max_rows = agent_title_max_rows(&shell);
    assert_eq!(max_rows, 3);
    let dialog = dialog(rows(6), Some(4), "");
    let start = agent_title_visible_start(&dialog, max_rows);
    assert_eq!(start, 2);

    let x = shell.inner.x + 1;
    let list_y = shell.inner.y + 2;
    assert_eq!(agent_title_row_at(&shell, &dialog, x, list_y), Some(2));
    assert_eq!(
        agent_title_row_at(&shell, &dialog, x, list_y + 1),
        Some(2),
        "同一条目的第二行命中同一 index"
    );
    assert_eq!(agent_title_row_at(&shell, &dialog, x, list_y + 2), Some(3));
    assert_eq!(agent_title_row_at(&shell, &dialog, x, list_y + 4), Some(4));

    assert_eq!(
        agent_title_row_at(&shell, &dialog, x, shell.inner.bottom().saturating_sub(1)),
        None,
        "按钮行不命中列表"
    );
    assert_eq!(
        agent_title_row_at(&shell, &dialog, x, list_y.saturating_sub(1)),
        None,
        "分割线不命中列表"
    );
    assert_eq!(
        agent_title_row_at(&shell, &dialog, shell.inner.x.saturating_sub(1), list_y),
        None,
        "列越界不命中"
    );
}

#[test]
fn button_hit_testing_maps_save_clear_and_cancel() {
    let shell = agent_title_shell(SCREEN, 2).expect("shell fits");
    let rects = widgets::modal::button_row(
        shell.inner,
        &AGENT_TITLE_BUTTONS,
        BUTTON_GAP,
        shell.inner.height.saturating_sub(1),
    );
    assert_eq!(
        agent_title_button_at(&shell, rects[0].x, rects[0].y),
        Some(AgentTitleButton::Save)
    );
    assert_eq!(
        agent_title_button_at(&shell, rects[1].x, rects[1].y),
        Some(AgentTitleButton::Clear)
    );
    assert_eq!(
        agent_title_button_at(&shell, rects[2].x, rects[2].y),
        Some(AgentTitleButton::Cancel)
    );
    assert_eq!(
        agent_title_button_at(&shell, rects[0].x, rects[0].y.saturating_sub(1)),
        None,
        "按钮上方不命中"
    );
    assert_eq!(
        agent_title_button_at(&shell, rects[2].right(), rects[2].y),
        None,
        "按钮右侧不命中"
    );
}

#[test]
fn renders_input_recent_rows_selection_and_buttons() {
    let dialog = dialog(rows(2), Some(1), "custom label");
    let (lines, buffer, cursor) = draw(&dialog, SCREEN.width, SCREEN.height);
    let body = lines.join("\n");
    assert!(body.contains("custom label"), "{body}");
    assert!(!body.contains(text::AGENT_TITLE_PLACEHOLDER), "{body}");
    assert!(body.contains(text::RENAME_AGENT_TITLE), "{body}");
    assert!(body.contains("page-0"), "{body}");
    assert!(body.contains("prompt-0"), "{body}");
    assert!(body.contains("page-1"), "两条 Recent 同时可见: {body}");
    assert!(body.contains("prompt-1"), "两条 Recent 同时可见: {body}");
    assert!(body.contains(text::BUTTON_SAVE), "{body}");
    assert!(body.contains(text::BUTTON_CLEAR), "{body}");
    assert!(body.contains(text::BUTTON_CANCEL), "{body}");
    assert!(cursor.is_some(), "输入行需要硬件光标");

    let shell = agent_title_shell(SCREEN, 2).expect("shell fits");
    let list_y = shell.inner.y + 2;
    // 选中项是第二条（可见槽位 1），两行都铺 selection 底色。
    for y in [list_y + 2, list_y + 3] {
        assert_eq!(
            buffer[(shell.inner.x, y)].bg,
            Color::Indexed(240),
            "选中行 {y} 起点缺少底色"
        );
        assert_eq!(
            buffer[(shell.inner.right() - 1, y)].bg,
            Color::Indexed(240),
            "选中行 {y} 整行应铺到底"
        );
    }
    // 未选中条目的第二行没有 selection 底色。
    assert_ne!(buffer[(shell.inner.x, list_y + 1)].bg, Color::Indexed(240));
}

#[test]
fn recent_rows_use_template_status_colors() {
    let rows = vec![
        row_with_status(0, "todo page", "todo prompt", Some(TemplateStatus::Todo)),
        row_with_status(
            1,
            "doing page",
            "doing prompt",
            Some(TemplateStatus::InProgress),
        ),
        row_with_status(2, "done page", "done prompt", Some(TemplateStatus::Done)),
        row_with_status(3, "plain page", "plain prompt", None),
    ];
    let dialog = dialog(rows, None, "");
    let (_, buffer, _) = draw(&dialog, SCREEN.width, SCREEN.height);
    let shell = agent_title_shell(SCREEN, 4).expect("shell fits");
    let list_y = shell.inner.y + 2;
    // 未选中条目 marker 为两列空白，页标题从 inner.x + 2 开始。
    let title_x = shell.inner.x + 2;

    let cases = [
        (0, Color::Reset, false),
        (1, Color::Yellow, false),
        (2, Color::Green, false),
        (3, Color::Reset, true),
    ];
    for (offset, color, dim) in cases {
        let cell = &buffer[(title_x, list_y + offset * 2)];
        assert_eq!(cell.fg, color, "row {offset} 页标题状态色");
        assert_eq!(
            cell.modifier.contains(Modifier::DIM),
            dim,
            "row {offset} DIM"
        );
    }
}

#[test]
fn selected_row_keeps_status_color_under_highlight() {
    let rows = vec![row_with_status(
        0,
        "doing page",
        "doing prompt",
        Some(TemplateStatus::InProgress),
    )];
    let dialog = dialog(rows, Some(0), "");
    let (_, buffer, _) = draw(&dialog, SCREEN.width, SCREEN.height);
    let shell = agent_title_shell(SCREEN, 1).expect("shell fits");
    let list_y = shell.inner.y + 2;
    // 选中项 marker 为 `❯ `，同样占两列。
    let cell = &buffer[(shell.inner.x + 2, list_y)];
    assert_eq!(cell.fg, Color::Yellow, "选中项保留状态色");
    assert_eq!(cell.bg, Color::Indexed(240), "选中底色保留");
    assert!(cell.modifier.contains(Modifier::BOLD), "选中项加粗");
}

#[test]
fn renders_empty_recent_state_and_placeholder() {
    let dialog = dialog(Vec::new(), None, "");
    let (lines, _, cursor) = draw(&dialog, SCREEN.width, SCREEN.height);
    let body = lines.join("\n");
    assert!(body.contains(text::PROMPT_RECENT_EMPTY), "{body}");
    assert!(body.contains(text::AGENT_TITLE_PLACEHOLDER), "{body}");
    assert!(body.contains(text::RENAME_AGENT_TITLE), "{body}");
    assert!(cursor.is_some());
}

#[test]
fn narrow_and_small_screens_render_without_panic() {
    let populated = dialog(rows(6), Some(5), "title");
    for (width, height) in [(20, 8), (12, 5), (10, 3)] {
        let _ = draw(&populated, width, height);
    }
    let empty = dialog(Vec::new(), None, "");
    let _ = draw(&empty, 20, 8);
}
