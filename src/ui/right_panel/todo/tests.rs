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

/// `button_at` 必须与 `button_rect` 是同一份几何：右端 `right()-1` 命中，`right()` 不命中，
/// 空档（两按钮之间的 1 列）与分割线行都不命中。
#[test]
fn button_hit_matches_button_rect_boundaries() {
    let area = panel(38);
    let bar = crate::layout::prompt_toolbar_rect(area).expect("toolbar visible");

    for (button, _) in BUTTONS {
        let rect = button_rect(area, button).expect("wide panel fits all buttons");
        assert_eq!(
            button_at(area, rect.x, rect.y),
            Some(button),
            "{button:?} 左端必须命中"
        );
        assert_eq!(
            button_at(area, rect.right() - 1, rect.y),
            Some(button),
            "{button:?} 右端（含）必须命中"
        );
        assert_eq!(
            button_at(area, rect.right(), rect.y),
            None,
            "{button:?} 右端之外不得命中"
        );
        assert_eq!(
            button_at(area, rect.x, rect.y + 1),
            None,
            "{button:?} 下一行（分割线）不得命中"
        );
    }

    // 两按钮之间的 1 列空档：不命中任何按钮。
    let new = button_rect(area, TodoButton::New).expect("[new] visible");
    let edit = button_rect(area, TodoButton::Edit).expect("[edit] visible");
    assert_eq!(edit.x, new.right().saturating_add(BUTTON_GAP));
    assert_eq!(button_at(area, new.right(), bar.y), None, "按钮空档不命中");
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

/// 列表区与底部统计块互不重叠：列表底边紧邻分割线，分割线下方恰好 1 行统计。
#[test]
fn stats_block_sits_below_list_without_overlap() {
    let area = panel(38);
    let list = list_rect(area);
    let divider = stats_divider_rect(area).expect("统计块可见");
    let row = stats_rect(area).expect("统计块可见");
    let body = crate::layout::prompt_body_rect(crate::layout::pane_inner_rect(area));

    assert_eq!(list.height, body.height - STATS_BLOCK, "列表让出统计块两行");
    assert_eq!(divider.y, list.bottom(), "分割线紧贴列表底部");
    assert_eq!(row.y, divider.bottom(), "统计行紧贴分割线下方");
    assert_eq!(row.bottom(), body.bottom(), "统计行贴主体区底部");
    assert_eq!(list.right(), body.right() - 1, "列表让出最右 1 列滚动条槽");
}

/// 主体区装不下「列表 + 分割线 + 统计」时整块隐藏，列表退回整块高度。
#[test]
fn stats_hidden_when_body_too_short() {
    for height in 0..=STATS_BLOCK {
        let area = Rect::new(0, 0, 40, height);
        assert!(stats_divider_rect(area).is_none(), "高度 {height} 无分割线");
        assert!(stats_rect(area).is_none(), "高度 {height} 无统计行");
    }
}

#[test]
fn stats_row_shows_counts_and_percent() {
    let mut state = AppState::demo();
    let area = panel(46);
    state.todo.replace(vec![
        TodoRecord::new("a", TodoStatus::Todo, 0),
        TodoRecord::new("b", TodoStatus::InProgress, 1),
        TodoRecord::new("c", TodoStatus::Done, 2),
        TodoRecord::new("d", TodoStatus::Done, 3),
    ]);

    let (buffer, _) = render_panel(&state, area);
    let row = stats_rect(area).expect("统计行可见");
    let text: String = (row.x..row.right())
        .map(|x| buffer[(x, row.y)].symbol())
        .collect();
    assert_eq!(
        text.trim_end(),
        format!(
            "{total} 4{sep}{doing} 1{sep}{done_label} 2 50%",
            total = text::TODO_STATS_TOTAL_LABEL,
            sep = text::TODO_STATS_SEPARATOR,
            doing = text::TODO_STATS_DOING_LABEL,
            done_label = text::TODO_STATS_DONE_LABEL,
        ),
        "`total 4 · doing 1 · done 2 50%`（标签 muted、数值正文色，段间 ` · `）"
    );
}

#[test]
fn empty_list_still_shows_zero_stats() {
    let state = AppState::demo();
    let area = panel(46);
    let (buffer, _) = render_panel(&state, area);
    let row = stats_rect(area).expect("统计行可见");
    let text: String = (row.x..row.right())
        .map(|x| buffer[(x, row.y)].symbol())
        .collect();
    assert!(text.contains("total 0"), "空列表统计归零: {text:?}");
    assert!(text.contains("done 0 0%"), "空列表百分比为 0: {text:?}");
}

/// 统计行不得覆盖到滚动条槽：列表溢出时 thumb 与统计行同屏并存。
#[test]
fn scrollbar_track_stops_above_stats() {
    let area = panel(38);
    let mut state = AppState::demo();
    let visible = visible_rows(area);
    state.todo.replace(records(visible.saturating_add(4)));
    let bar = scrollbar(area, &state).expect("溢出时出现滚动条");
    let list = list_rect(area);
    assert_eq!(bar.track.bottom(), list.bottom(), "轨道只在列表区");
    assert!(
        bar.track.bottom() <= stats_divider_rect(area).expect("统计块可见").y,
        "轨道不得压到分割线"
    );
}

/// 编辑行复用 `widgets::input` 的视口：长文本横向滚动后光标仍在输入区右缘内。
#[test]
fn edit_row_reuses_input_view_scrolling() {
    let mut state = AppState::demo();
    let area = panel(30);
    state
        .todo
        .replace(vec![TodoRecord::new("seed", TodoStatus::Todo, 0)]);
    let id = state.todo.items()[0].id.clone();
    let long = "x".repeat(80);
    state.todo.begin_edit(TodoEdit::new(Some(id), long, 0));

    let (_, cursor) = render_panel(&state, area);
    let (x, y) = cursor.expect("编辑行必须返回光标坐标");
    let list = list_rect(area);
    assert_eq!(y, list.y);
    assert!(
        x < list.right(),
        "光标 {x} 不得越过列表右缘 {:?}",
        list.right()
    );
    assert_eq!(
        x,
        list.right() - 1,
        "文本长于输入区时视口滚到末尾、光标贴右缘"
    );
}

/// 新建态空输入显示占位；既有条目清空后不显示占位（那是「准备删除」而不是「待填写」）。
#[test]
fn placeholder_only_for_new_todo() {
    let mut state = AppState::demo();
    let area = panel(38);
    state
        .todo
        .replace(vec![TodoRecord::new("old", TodoStatus::Todo, 0)]);

    state.todo.begin_edit(TodoEdit::new(None, String::new(), 1));
    let (buffer, cursor) = render_panel(&state, area);
    let list = list_rect(area);
    let ghost_row = list.y.saturating_add(1);
    let text: String = (list.x..list.right())
        .map(|x| buffer[(x, ghost_row)].symbol())
        .collect();
    assert!(
        text.contains(text::TODO_EDIT_PLACEHOLDER),
        "新建幽灵行显示占位: {text:?}"
    );
    assert_eq!(cursor.map(|(_, y)| y), Some(ghost_row), "光标在幽灵行");

    let id = state.todo.items()[0].id.clone();
    state
        .todo
        .begin_edit(TodoEdit::new(Some(id), String::new(), 0));
    let (buffer, _) = render_panel(&state, area);
    let text: String = (list.x..list.right())
        .map(|x| buffer[(x, list.y)].symbol())
        .collect();
    assert!(
        !text.contains(text::TODO_EDIT_PLACEHOLDER),
        "既有条目清空不显示占位: {text:?}"
    );
}

/// 编辑行光标按**显示宽度**落位：宽字符占两列，按字符数算会把光标放到字中间。
#[test]
fn edit_row_cursor_uses_display_width() {
    let mut state = AppState::demo();
    let area = panel(38);
    state
        .todo
        .replace(vec![TodoRecord::new("seed", TodoStatus::Todo, 0)]);
    let id = state.todo.items()[0].id.clone();
    // 3 个 CJK 字符 = 6 显示列。
    state.todo.begin_edit(TodoEdit::new(Some(id), "中文名", 0));

    let (buffer, cursor) = render_panel(&state, area);
    let list = list_rect(area);
    let text_x = list
        .x
        .saturating_add(MARKER_WIDTH)
        .saturating_add(1)
        .saturating_add(STATUS_WIDTH)
        .saturating_add(1);
    let (x, _) = cursor.expect("编辑行必须返回光标坐标");
    assert_eq!(x, text_x + 6, "光标列 = 文本显示宽度，不是字符数");
    // 文本也确实按两列一格铺开。
    assert_eq!(buffer[(text_x, list.y)].symbol(), "中");
    assert_eq!(buffer[(text_x + 4, list.y)].symbol(), "名");
}

/// 统计行放不下时整行不画（只留分割线），绝不允许后面的短段顶到前面拼出残句。
#[test]
fn stats_row_hidden_whole_when_too_narrow() {
    let mut state = AppState::demo();
    state.todo.replace(vec![
        TodoRecord::new("a", TodoStatus::Todo, 0),
        TodoRecord::new("b", TodoStatus::Done, 1),
    ]);

    // 足够宽：整句可见。
    let wide = panel(46);
    let (buffer, _) = render_panel(&state, wide);
    let row = stats_rect(wide).expect("统计行可见");
    let text: String = (row.x..row.right())
        .map(|x| buffer[(x, row.y)].symbol())
        .collect();
    assert!(
        text.trim_end().ends_with("done 1 50%"),
        "宽面板整句: {text:?}"
    );

    // 恰好差一列：整行不画，不得留下半句。
    let needed = text.trim_end().chars().count();
    let narrow = panel(u16::try_from(needed).unwrap_or(u16::MAX) - 1);
    let (buffer, _) = render_panel(&state, narrow);
    let row = stats_rect(narrow).expect("统计行可见");
    let text: String = (row.x..row.right())
        .map(|x| buffer[(x, row.y)].symbol())
        .collect();
    assert_eq!(text.trim(), "", "放不下时整行不画: {text:?}");
    // 分割线仍在：统计块的高度不随内容有无而变化。
    let divider = stats_divider_rect(narrow).expect("统计块可见");
    assert_eq!(
        buffer[(narrow.x, divider.y)].symbol(),
        text::DIVIDER_LEFT_JOIN
    );
}
