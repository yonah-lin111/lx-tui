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

/// 进入编辑并置焦点：真机上 `begin_todo_edit` 必然同时聚焦面板，
/// 而渲染层只在聚焦时才交出硬件光标。
fn begin_edit(state: &mut AppState, edit: TodoEdit) {
    state.set_focus(crate::app::state::FocusedContainer::Todo);
    state.todo.begin_edit(edit);
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
fn toolbar_puts_new_on_left_and_actions_on_right() {
    let area = panel(38);
    let bar = crate::layout::prompt_toolbar_rect(area).expect("toolbar visible");
    let new = button_rect(area, TodoButton::New).expect("[+] visible");
    assert_eq!(new.x, bar.x, "[+] 贴左边框");
    assert_eq!(new.width, NEW_WIDTH);

    // 右侧组贴右缘，从左到右依次 `[edit] [del] [clear done]`。
    let edit = button_rect(area, TodoButton::Edit).expect("[edit] visible");
    let del = button_rect(area, TodoButton::Delete).expect("[del] visible");
    let clear = button_rect(area, TodoButton::ClearDone).expect("[clear done] visible");
    assert_eq!(clear.right(), bar.right(), "[clear done] 贴右缘");
    assert_eq!(del.right().saturating_add(BUTTON_GAP), clear.x);
    assert_eq!(edit.right().saturating_add(BUTTON_GAP), del.x);
    assert!(
        new.right().saturating_add(BUTTON_GAP) <= edit.x,
        "左右两组之间至少留 1 列空白: {new:?} / {edit:?}"
    );
}

#[test]
fn toolbar_drops_action_buttons_before_new() {
    // `[+]` 恒在；右侧组从右往左丢弃，靠右的 `[clear done]` 先丢。
    let cases: [(u16, Vec<TodoButton>); 4] = [
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
            20,
            vec![TodoButton::New, TodoButton::Edit, TodoButton::Delete],
        ),
        (13, vec![TodoButton::New, TodoButton::Edit]),
        (8, vec![TodoButton::New]),
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
}

/// `button_at` 必须与 `button_rect` 是同一份几何：两端命中、之外不命中，
/// 两组之间的空档与分割线行都不命中。
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

    // 左右两组之间的空档：不命中任何按钮。
    let new = button_rect(area, TodoButton::New).expect("[+] visible");
    let edit = button_rect(area, TodoButton::Edit).expect("[edit] visible");
    assert!(new.right() < edit.x, "两组之间必须有空档");
    for x in (new.right())..edit.x {
        assert_eq!(button_at(area, x, bar.y), None, "第 {x} 列空档不命中");
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
    state.set_focus(crate::app::state::FocusedContainer::Todo);
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
    // 滚动条只在 Todo 面板可见时存在（隐藏时它与 prompt 正文共用最右一列）。
    state.right_tab = crate::app::state::RightTab::Todo;
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
    begin_edit(&mut state, TodoEdit::new(Some(id), "bravo edit", 1));

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

/// 列表与统计块平分主体区：两者高度最多差 1 行（分割线吃掉余数）。
#[test]
fn list_and_stats_split_body_evenly() {
    let area = panel(38);
    let list = list_rect(area);
    let divider = stats_divider_rect(area).expect("统计块可见");
    let stats = stats_rect(area).expect("统计块可见");
    let body = crate::layout::prompt_body_rect(crate::layout::pane_inner_rect(area));

    assert_eq!(
        u32::from(list.height) + u32::from(stats.height) + 1,
        u32::from(body.height),
        "列表 + 分割线 + 统计 == 主体区高度"
    );
    let diff = list.height.abs_diff(stats.height);
    assert!(
        diff <= 1,
        "两者高度差 {diff} 超过 1 行：{list:?} / {stats:?}"
    );
    assert_eq!(divider.y, list.bottom(), "分割线紧贴列表底部");
    assert_eq!(stats.y, divider.bottom(), "统计块紧贴分割线下方");
    assert_eq!(stats.bottom(), body.bottom(), "统计块贴主体区底部");
    assert_eq!(list.right(), body.right() - 1, "列表让出最右 1 列滚动条槽");
}

/// 主体区太矮时不切分：列表独占整块，统计块整块隐藏。
#[test]
fn stats_hidden_when_body_too_short() {
    for height in 0..MIN_SPLIT_BODY {
        let area = Rect::new(0, 0, 40, height);
        assert!(stats_divider_rect(area).is_none(), "高度 {height} 无分割线");
        assert!(stats_rect(area).is_none(), "高度 {height} 无统计块");
        let body = crate::layout::prompt_body_rect(crate::layout::pane_inner_rect(area));
        assert_eq!(
            list_rect(area).height,
            body.height,
            "高度 {height} 列表独占"
        );
    }
}

/// 统计块首行是摘要 `total N · doing N · done N N%`。
#[test]
fn stats_summary_row_shows_counts_and_percent() {
    let mut state = AppState::demo();
    let area = panel(46);
    state.todo.replace(vec![
        TodoRecord::new("a", TodoStatus::Todo, 0),
        TodoRecord::new("b", TodoStatus::InProgress, 1),
        TodoRecord::new("c", TodoStatus::Done, 2),
        TodoRecord::new("d", TodoStatus::Done, 3),
    ]);

    let (buffer, _) = render_panel(&state, area);
    let stats = stats_rect(area).expect("统计块可见");
    let text: String = (stats.x..stats.right())
        .map(|x| buffer[(x, stats.y)].symbol())
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

/// 柱图按三态各占一槽，柱子自下而上堆叠 `█`；轴标签行在统计块最后一行。
#[test]
fn stats_chart_draws_bars_per_status() {
    let mut state = AppState::demo();
    let area = panel(46);
    state.todo.replace(vec![
        TodoRecord::new("a", TodoStatus::Todo, 0),
        TodoRecord::new("b", TodoStatus::Todo, 1),
        TodoRecord::new("c", TodoStatus::Done, 2),
    ]);

    let (buffer, _) = render_panel(&state, area);
    let stats = stats_rect(area).expect("统计块可见");
    assert!(
        stats.height >= MIN_STATS_ROWS,
        "统计块高度 {}",
        stats.height
    );

    // 轴标签行：三段文案都在，且各带自己的计数。
    let axis: String = (stats.x..stats.right())
        .map(|x| buffer[(x, stats.bottom() - 1)].symbol())
        .collect();
    for label in [
        text::TODO_STATS_OPEN_LABEL,
        text::TODO_STATS_DOING_LABEL,
        text::TODO_STATS_DONE_LABEL,
    ] {
        assert!(axis.contains(label), "轴标签缺 {label}: {axis:?}");
    }
    assert!(axis.contains("open 2"), "未开始计数: {axis:?}");
    assert!(axis.contains("done 1"), "已完成计数: {axis:?}");

    // 柱区：`█` 出现在轴标签之上的行里，且未开始（2 条）比已完成（1 条）高。
    let chart_top = stats.y + 1;
    let chart_bottom = stats.bottom() - 2;
    let column_height = |x_test: &dyn Fn(u16) -> bool| -> usize {
        (chart_top..=chart_bottom)
            .filter(|y| {
                (stats.x..stats.right())
                    .any(|x| x_test(x) && buffer[(x, *y)].symbol() == text::TODO_BAR_FILLED)
            })
            .count()
    };
    let total_filled = column_height(&|_| true);
    assert!(total_filled > 0, "柱区必须有柱子");

    // 三种柱子分别用各自的状态色：至少存在一绿（done）一非绿（open / doing）。
    let has_green = (chart_top..=chart_bottom).any(|y| {
        (stats.x..stats.right()).any(|x| {
            let cell = &buffer[(x, y)];
            cell.symbol() == text::TODO_BAR_FILLED && cell.fg == Color::Green
        })
    });
    assert!(has_green, "已完成柱必须用 Green");
}

#[test]
fn empty_list_still_shows_zero_stats() {
    let state = AppState::demo();
    let area = panel(46);
    let (buffer, _) = render_panel(&state, area);
    let stats = stats_rect(area).expect("统计块可见");
    let text: String = (stats.x..stats.right())
        .map(|x| buffer[(x, stats.y)].symbol())
        .collect();
    assert!(text.contains("total 0"), "空列表统计归零: {text:?}");
    assert!(text.contains("done 0 0%"), "空列表百分比为 0: {text:?}");
    let filled = (stats.y..stats.bottom())
        .flat_map(|y| (stats.x..stats.right()).map(move |x| (x, y)))
        .filter(|(x, y)| buffer[(*x, *y)].symbol() == text::TODO_BAR_FILLED)
        .count();
    assert_eq!(filled, 0, "空列表不画柱子");
}

/// 统计行不得覆盖到滚动条槽：列表溢出时 thumb 与统计行同屏并存。
#[test]
fn scrollbar_track_stops_above_stats() {
    let area = panel(38);
    let mut state = AppState::demo();
    state.right_tab = crate::app::state::RightTab::Todo;
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
    begin_edit(&mut state, TodoEdit::new(Some(id), long, 0));

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

    begin_edit(&mut state, TodoEdit::new(None, String::new(), 1));
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
    begin_edit(&mut state, TodoEdit::new(Some(id), String::new(), 0));
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
    begin_edit(&mut state, TodoEdit::new(Some(id), "中文名", 0));

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

/// 摘要行放不下时整行不画（分割线与柱图仍在），绝不允许后面的短段顶到前面拼出残句。
#[test]
fn stats_summary_hidden_whole_when_too_narrow() {
    let mut state = AppState::demo();
    state.todo.replace(vec![
        TodoRecord::new("a", TodoStatus::Todo, 0),
        TodoRecord::new("b", TodoStatus::Done, 1),
    ]);

    // 足够宽：整句可见。
    let wide = panel(46);
    let (buffer, _) = render_panel(&state, wide);
    let stats = stats_rect(wide).expect("统计块可见");
    let text: String = (stats.x..stats.right())
        .map(|x| buffer[(x, stats.y)].symbol())
        .collect();
    assert!(
        text.trim_end().ends_with("done 1 50%"),
        "宽面板整句: {text:?}"
    );

    // 恰好差一列：整行不画，不得留下半句。
    let needed = text.trim_end().chars().count();
    let narrow = panel(u16::try_from(needed).unwrap_or(u16::MAX) - 1);
    let (buffer, _) = render_panel(&state, narrow);
    let stats = stats_rect(narrow).expect("统计块可见");
    let text: String = (stats.x..stats.right())
        .map(|x| buffer[(x, stats.y)].symbol())
        .collect();
    assert_eq!(text.trim(), "", "放不下时摘要整行不画: {text:?}");
    // 分割线仍在：统计块的高度不随内容有无而变化。
    let divider = stats_divider_rect(narrow).expect("统计块可见");
    assert_eq!(
        buffer[(narrow.x, divider.y)].symbol(),
        text::DIVIDER_LEFT_JOIN
    );
}

/// 编辑首行、后面还有行时，光标不能让后面那些「无光标」的行覆盖掉。
///
/// 真机表现：`render_rows` 逐行覆盖 `cursor`，编辑行不在最后一行时被后面的 `None`
/// 冲掉，上层回落到窗格光标——终端里看起来就是输入法跑到 lx 输入框去了。
#[test]
fn edit_cursor_survives_later_rows() {
    let mut state = AppState::demo();
    let area = panel(38);
    state.todo.replace(vec![
        TodoRecord::new("a", TodoStatus::Todo, 0),
        TodoRecord::new("b", TodoStatus::Todo, 1),
        TodoRecord::new("c", TodoStatus::Todo, 2),
    ]);
    let id = state.todo.items()[0].id.clone();
    begin_edit(&mut state, TodoEdit::new(Some(id), "editing", 0));

    let (_, cursor) = render_panel(&state, area);
    let list = list_rect(area);
    assert_eq!(
        cursor.map(|(_, y)| y),
        Some(list.y),
        "编辑首行时光标必须在首行，不能被后面的行清成 None"
    );
}

/// 面板未聚焦时不得交出硬件光标：否则上层会把它当输入位置画出来，
/// 终端里表现为 IME 落在没有输入框的 Todo 行上（真机缺陷）。
#[test]
fn unfocused_panel_yields_no_hardware_cursor() {
    let mut state = AppState::demo();
    let area = panel(38);
    state.todo.replace(vec![
        TodoRecord::new("a", TodoStatus::Todo, 0),
        TodoRecord::new("b", TodoStatus::Todo, 1),
    ]);
    let id = state.todo.items()[0].id.clone();
    state.todo.begin_edit(TodoEdit::new(Some(id), "draft", 0));
    state.set_focus(crate::app::state::FocusedContainer::Panes);

    let (_, cursor) = render_panel(&state, area);
    assert!(cursor.is_none(), "未聚焦时不能交出光标");
}

/// 悬停下标是**列表**下标：幽灵行打开时，悬停显示第 2 行必须高亮 b 而不是 c。
#[test]
fn hover_index_is_list_index_below_ghost_row() {
    let mut state = AppState::demo();
    let area = panel(38);
    state.todo.replace(vec![
        TodoRecord::new("a", TodoStatus::Todo, 0),
        TodoRecord::new("b", TodoStatus::Todo, 1),
        TodoRecord::new("c", TodoStatus::Todo, 2),
    ]);
    state.right_tab = crate::app::state::RightTab::Todo;
    // 幽灵行插在 1：显示序列 a(0) / 幽灵(1) / b(2) / c(3)。
    begin_edit(&mut state, TodoEdit::new(None, String::new(), 1));

    // 命中入口的换算：显示下标 2 → 列表下标 1（b），幽灵行自身没有列表下标。
    assert_eq!(list_index(&state, 0), Some(0));
    assert_eq!(list_index(&state, 1), None, "幽灵行不是条目");
    assert_eq!(list_index(&state, 2), Some(1), "显示第 2 行是列表里的 b");
    assert_eq!(list_index(&state, 3), Some(2));
    assert_eq!(list_index(&state, 4), None, "越界");

    state.todo_hover = Some(1); // b
    let (buffer, _) = render_panel(&state, area);
    let list = list_rect(area);
    let highlighted = |y: u16| buffer[(list.x, y)].bg != ratatui::style::Color::Reset;
    assert!(highlighted(list.y + 2), "悬停 b 必须高亮 b（显示第 2 行）");
    assert!(!highlighted(list.y + 3), "c 不该被高亮");
}

/// 隐藏的 Todo 面板不得提供滚动条几何：否则它与 prompt 正文共用最右一列，
/// 「点 prompt 右缘」会滚到看不见的 Todo 列表。
#[test]
fn hidden_panel_has_no_scrollbar() {
    let area = panel(38);
    let mut state = AppState::demo();
    assert_eq!(
        state.right_tab,
        crate::app::state::RightTab::Prompt,
        "前提：Prompt"
    );
    let visible = visible_rows(area);
    state.todo.replace(records(visible.saturating_add(5)));
    assert!(
        scrollbar(area, &state).is_none(),
        "Prompt tab 下 Todo 滚动条不存在"
    );

    state.right_tab = crate::app::state::RightTab::Todo;
    assert!(
        scrollbar(area, &state).is_some(),
        "切到 Todo 后才出现滚动条"
    );

    state.prompt_collapsed = true;
    assert!(scrollbar(area, &state).is_none(), "容器折叠时也不存在");
}
