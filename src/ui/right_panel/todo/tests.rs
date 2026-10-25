//! Todo 面板体测试：按钮宽度阶梯、三态行对齐与样式、编辑光标、滚动条与空态。
//!
//! 面板体直接渲染到独立后端（不经过容器），几何断言用同一份 `list_rect` 复算；
//! 选中样式那条走整屏 `crate::ui::render`，确保容器聚焦边框也参与画面。

use super::*;
use crate::app::state::RightTab;
use crate::config::Config;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::style::{Color, Modifier};

/// 面板高度：足够容纳表头两行与列表区。
const PANEL_HEIGHT: u16 = 14;

const SCREEN: Rect = Rect {
    x: 0,
    y: 0,
    width: 100,
    height: 24,
};

/// 按内容区宽度造一个面板矩形（左右各 1 列边框）。
fn panel(inner_width: u16) -> Rect {
    Rect::new(0, 0, inner_width.saturating_add(2), PANEL_HEIGHT)
}

/// 渲染 Todo 面板体；返回缓冲区与编辑光标。
fn render_panel(state: &AppState, area: Rect) -> (Buffer, Option<(u16, u16)>) {
    let mut terminal = Terminal::new(TestBackend::new(60, 20)).expect("test backend is infallible");
    let mut cursor = None;
    if let Err(error) = terminal.draw(|frame| cursor = render(frame, area, state)) {
        panic!("draw failed: {error}");
    }
    (terminal.backend().buffer().clone(), cursor)
}

fn view_for(state: &AppState) -> crate::ui::layout::ViewLayout {
    crate::ui::layout::compute(
        SCREEN,
        &Config::default(),
        state.sidebar_collapsed,
        state.sidebar_width,
        state.prompt_collapsed,
        state.prompt_width,
        state.prompt_left,
        state.panes_collapsed,
    )
}

fn render_screen(state: &AppState) -> Buffer {
    let config = Config::default();
    let mut terminal =
        Terminal::new(TestBackend::new(SCREEN.width, SCREEN.height)).expect("test backend");
    if let Err(error) = terminal.draw(|frame| crate::ui::render(frame, state, &config)) {
        panic!("draw failed: {error}");
    }
    terminal.backend().buffer().clone()
}

/// 列表某一行的文本（按列表列范围裁剪）。
fn row_text(buffer: &Buffer, list: Rect, y: u16) -> String {
    (list.x..list.right())
        .map(|x| buffer[(x, y)].symbol())
        .collect()
}

fn records(count: usize) -> Vec<TodoRecord> {
    (0..count)
        .map(|index| {
            TodoRecord::new(
                format!("item {index}"),
                TodoStatus::Todo,
                i64::try_from(index).unwrap_or(i64::MAX),
            )
        })
        .collect()
}

#[test]
fn toolbar_buttons_drop_from_right_when_narrow() {
    // 宽度阶梯：5 / +1+6=12 / +1+5=18 / +1+12=31，从右往左丢弃。
    let cases: [(u16, Vec<TodoButton>); 5] = [
        (
            38,
            vec![
                TodoButton::New,
                TodoButton::Edit,
                TodoButton::Delete,
                TodoButton::ClearDone,
            ],
        ),
        (
            28,
            vec![TodoButton::New, TodoButton::Edit, TodoButton::Delete],
        ),
        (
            18,
            vec![TodoButton::New, TodoButton::Edit, TodoButton::Delete],
        ),
        (12, vec![TodoButton::New, TodoButton::Edit]),
        (4, Vec::new()),
    ];

    for (inner, want) in cases {
        let area = panel(inner);
        let got: Vec<TodoButton> = BUTTONS
            .iter()
            .map(|(button, _)| *button)
            .filter(|button| button_rect(area, *button).is_some())
            .collect();
        assert_eq!(got, want, "内宽 {inner} 时可见按钮集");
    }

    // 18 列恰好放下三个按钮，ClearDone 被丢弃。
    let area = panel(18);
    assert!(button_rect(area, TodoButton::ClearDone).is_none());
    for button in [TodoButton::New, TodoButton::Edit, TodoButton::Delete] {
        assert!(button_rect(area, button).is_some(), "{button:?} 必须可见");
    }
}

#[test]
fn list_rows_render_status_boxes_aligned() {
    let mut state = AppState::demo();
    let area = panel(38);
    state.todo.replace(vec![
        TodoRecord::new("alpha", TodoStatus::Todo, 0),
        TodoRecord::new("bravo", TodoStatus::InProgress, 1),
        TodoRecord::new("charlie", TodoStatus::Done, 2),
    ]);

    let (buffer, _) = render_panel(&state, area);
    let list = list_rect(area);
    let labels = [
        text::TODO_STATUS_TODO,
        text::TODO_STATUS_IN_PROGRESS,
        text::TODO_STATUS_DONE,
    ];
    let mut starts = Vec::new();
    for (index, label) in labels.into_iter().enumerate() {
        let y = list
            .y
            .saturating_add(u16::try_from(index).unwrap_or(u16::MAX));
        let row = row_text(&buffer, list, y);
        let start = row
            .chars()
            .position(|ch| ch == '[')
            .expect("状态框必须有左括号");
        let rendered: String = row.chars().skip(start).take(3).collect();
        assert_eq!(rendered, label, "第 {index} 行状态框");
        starts.push(start);
    }
    assert!(
        starts.windows(2).all(|pair| pair[0] == pair[1]),
        "三行状态框起始列必须一致: {starts:?}"
    );
}

#[test]
fn selected_row_uses_selection_style_without_inheriting_border_accent() {
    let mut state = AppState::demo();
    state.right_tab = RightTab::Todo;
    state.todo_focused = true;
    state.prompt_width = 40;
    state
        .todo
        .replace(vec![TodoRecord::new("alpha", TodoStatus::Todo, 0)]);

    let view = view_for(&state);
    let buffer = render_screen(&state);
    let list = list_rect(view.prompt);
    let selection = style::selection().bg;

    // 选中标记格：selection 前景 + 底色，不得继承聚焦边框的 accent（Cyan + BOLD）。
    let marker = &buffer[(list.x, list.y)];
    assert_eq!(Some(marker.bg), selection, "选中行标记格底色: {marker:?}");
    assert_ne!(
        marker.fg,
        Color::Cyan,
        "标记格不得继承边框强调色: {marker:?}"
    );
    assert!(
        !marker.modifier.contains(Modifier::BOLD),
        "标记格不得继承边框加粗: {marker:?}"
    );

    // 状态框与正文格子同样铺满选中底色（文本样式不带显式背景）。
    for x in [list.x.saturating_add(2), list.x.saturating_add(6)] {
        let cell = &buffer[(x, list.y)];
        assert_eq!(Some(cell.bg), selection, "选中行第 {x} 列底色: {cell:?}");
    }
}

#[test]
fn done_row_is_muted_and_crossed_out() {
    let mut state = AppState::demo();
    let area = panel(38);
    state
        .todo
        .replace(vec![TodoRecord::new("finished", TodoStatus::Done, 0)]);

    let (buffer, _) = render_panel(&state, area);
    let list = list_rect(area);
    let text_x = list
        .x
        .saturating_add(MARKER_WIDTH)
        .saturating_add(1)
        .saturating_add(STATUS_WIDTH)
        .saturating_add(1);
    for offset in 0..u16::try_from("finished".len()).unwrap_or(u16::MAX) {
        let cell = &buffer[(text_x.saturating_add(offset), list.y)];
        assert!(
            cell.modifier.contains(Modifier::CROSSED_OUT),
            "已完成行正文必须带删除线: {cell:?}"
        );
        assert!(
            cell.modifier.contains(Modifier::DIM),
            "已完成行正文必须弱化: {cell:?}"
        );
    }
}

#[test]
fn empty_list_shows_placeholder() {
    let state = AppState::demo();
    let area = panel(30);
    let (buffer, cursor) = render_panel(&state, area);
    let list = list_rect(area);
    assert!(cursor.is_none(), "空列表没有编辑光标");
    let row = row_text(&buffer, list, list.y);
    assert!(
        row.contains(text::TODO_EMPTY),
        "空态文案必须在首行: {row:?}"
    );
}

#[test]
fn scrollbar_only_when_overflowing() {
    let area = panel(38);
    let mut state = AppState::demo();
    let visible = visible_rows(area);
    assert!(visible > 0, "测试面板必须有可见行");

    state.todo.replace(records(visible.saturating_sub(1)));
    assert!(
        scrollbar(area, &state).is_none(),
        "条目装得下时不得出现滚动条"
    );

    let total = visible.saturating_add(5);
    state.todo.replace(records(total));
    let bar = scrollbar(area, &state).expect("溢出时必须出现滚动条");
    assert_eq!(bar.total, total);
    assert_eq!(bar.visible, visible);
    assert!(bar.thumb.height >= 1, "thumb 高度至少 1 行: {bar:?}");
}

#[test]
fn edit_row_returns_cursor_position() {
    let mut state = AppState::demo();
    let area = panel(38);
    state.todo.replace(vec![
        TodoRecord::new("alpha", TodoStatus::Todo, 0),
        TodoRecord::new("bravo", TodoStatus::Todo, 1),
    ]);
    let id = state.todo.items()[1].id.clone();
    state
        .todo
        .begin_edit(TodoEdit::new(Some(id), "bravo edit", 1));

    assert!(edit_row_width(area) > 0, "编辑行必须有可用宽度");
    let (_, cursor) = render_panel(&state, area);
    let (x, y) = cursor.expect("编辑行必须返回光标坐标");
    let list = list_rect(area);
    assert_eq!(y, list.y.saturating_add(1), "光标行必须是被编辑项所在行");
    assert!(
        x >= list.x && x < list.right(),
        "光标列 {x} 必须落在列表区 {list:?} 内"
    );
}
