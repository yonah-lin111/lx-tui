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

mod charts;
mod schedule;

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

    // 右侧组贴右缘：`[sort]`。
    let sort = button_rect(area, TodoButton::Sort).expect("[sort] visible");
    assert_eq!(sort.right(), bar.right(), "[sort] 贴右缘");
    assert!(
        new.right().saturating_add(BUTTON_GAP) <= sort.x,
        "左右两组之间至少留 1 列空白: {new:?} / {sort:?}"
    );
}

#[test]
fn toolbar_drops_action_buttons_before_new() {
    // `[+]` 恒在；右侧组靠右的 `[sort]` 在空间不足时丢弃。
    let cases: [(u16, Vec<TodoButton>); 3] = [
        (38, vec![TodoButton::New, TodoButton::Sort]),
        (13, vec![TodoButton::New, TodoButton::Sort]),
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
    let sort = button_rect(area, TodoButton::Sort).expect("[sort] visible");
    assert!(new.right() < sort.x, "两组之间必须有空档");
    for x in (new.right())..sort.x {
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

    // 状态框与正文格子同样铺满选中底色（文本样式不带显式背景；优先级徽标拥有独立底色）。
    for x in [list.x.saturating_add(2), row_text_x(list)] {
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
    let text_x = row_text_x(list);
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

/// 编辑行复用 `widgets::input::multiline_view` 的视口：长文本软折行，光标落在末行剩余宽度处。
#[test]
fn edit_row_reuses_multiline_view_scrolling() {
    let mut state = AppState::demo();
    let area = panel(30);
    state
        .todo
        .replace(vec![TodoRecord::new("seed", TodoStatus::Todo, 0)]);
    let id = state.todo.items()[0].id.clone();
    let width = edit_row_width(area);
    let long = "x".repeat(width.saturating_add(3));
    begin_edit(&mut state, TodoEdit::new(Some(id), long, 0));

    let (_, cursor) = render_panel(&state, area);
    let (x, y) = cursor.expect("编辑行必须返回光标坐标");
    let list = list_rect(area);
    assert_eq!(y, list.y + 1, "两行文本：光标落在第二行");
    assert_eq!(x, row_text_x(list) + 3, "光标列 = 末行剩余字符宽度");
    assert!(x < list.right(), "光标 {x} 不得越过列表右缘");
}

/// 列表正文可用显示列数。
fn text_width(list: Rect) -> usize {
    usize::from(list.right().saturating_sub(row_text_x(list)))
}

/// 中长文本折为 2~3 行：续行从 `row_text_x` 起与首行正文对齐，左侧 9 列留空。
#[test]
fn long_todo_wraps_with_nine_column_continuation_indent() {
    let mut state = AppState::demo();
    let area = panel(38);
    let list = list_rect(area);
    let width = text_width(list).max(1);
    let text_x = row_text_x(list);
    let text = format!("{}{}", "a".repeat(width), "tail");
    state
        .todo
        .replace(vec![TodoRecord::new(text, TodoStatus::Todo, 0)]);

    let layout = layout_visible_items(area, &state);
    assert_eq!(layout.items[0].rendered_rows, 2, "两行文本占 2 行");
    let (buffer, _) = render_panel(&state, area);
    assert_eq!(buffer[(text_x, list.y)].symbol(), "a");
    assert_eq!(
        buffer[(text_x, list.y + 1)].symbol(),
        "t",
        "续行正文必须从 text_x 起"
    );
    for x in list.x..text_x {
        assert_eq!(
            buffer[(x, list.y + 1)].symbol(),
            " ",
            "续行第 {x} 列必须留空（9 列缩进）"
        );
    }
}

/// 超 3 行在第 3 行末尾以 `…` 截断，条目总高固定 3 行。
#[test]
fn overlong_todo_truncates_third_line_with_ellipsis() {
    let mut state = AppState::demo();
    let area = panel(38);
    let list = list_rect(area);
    let width = text_width(list).max(1);
    let text = "a".repeat(width.saturating_mul(4));
    state
        .todo
        .replace(vec![TodoRecord::new(text, TodoStatus::Todo, 0)]);

    let layout = layout_visible_items(area, &state);
    assert_eq!(layout.items[0].rendered_rows, 3, "最多 3 行");
    let (buffer, _) = render_panel(&state, area);
    assert_eq!(
        buffer[(
            row_text_x(list) + u16::try_from(width).unwrap_or(u16::MAX) - 1,
            list.y + 2
        )]
            .symbol(),
        "…",
        "第 3 行末列必须是省略号"
    );
}

/// 选中/悬停底色铺满条目的全部 1~3 个视觉行，续行前缀也保留底色。
#[test]
fn selected_wrapped_item_fills_all_visual_rows() {
    let mut state = AppState::demo();
    let area = panel(38);
    let list = list_rect(area);
    let width = text_width(list).max(1);
    let selection = style::selection().bg;
    state.todo.replace(vec![TodoRecord::new(
        "a".repeat(width.saturating_mul(3)),
        TodoStatus::Todo,
        0,
    )]);
    state.set_focus(crate::app::state::FocusedContainer::Todo);

    let (buffer, _) = render_panel(&state, area);
    let text_x = row_text_x(list);
    for row in 0..3u16 {
        for x in [list.x, text_x, list.right() - 1] {
            assert_eq!(
                buffer[(x, list.y + row)].bg,
                selection.unwrap_or(Color::Reset),
                "第 {row} 个视觉行第 {x} 列必须铺选中底色"
            );
        }
    }

    // 悬停同样铺满整块。
    state.set_focus(crate::app::state::FocusedContainer::Panes);
    state.todo_hover = Some(0);
    let (buffer, _) = render_panel(&state, area);
    for row in 0..3u16 {
        assert_eq!(
            buffer[(list.x, list.y + row)].bg,
            selection.unwrap_or(Color::Reset),
            "悬停第 {row} 个视觉行必须铺底色"
        );
    }
}

/// 行内编辑在 1~3 行间自适应展开；超过 3 行固定 3 行并随光标滚动视口。
#[test]
fn edit_row_expands_to_three_lines_and_scrolls_with_cursor() {
    let mut state = AppState::demo();
    let area = panel(38);
    let list = list_rect(area);
    let width = edit_row_width(area).max(1);
    state.todo.replace(vec![
        TodoRecord::new("a", TodoStatus::Todo, 0),
        TodoRecord::new("b", TodoStatus::Todo, 1),
    ]);
    let id = state.todo.items()[0].id.clone();

    // 1 行：高 1，下一条目紧跟其后。
    begin_edit(
        &mut state,
        TodoEdit::new(Some(id.clone()), "x".repeat(width - 1), 0),
    );
    let layout = layout_visible_items(area, &state);
    assert_eq!(layout.items[0].rendered_rows, 1);
    assert_eq!(layout.items[1].y, list.y + 1, "单行编辑不占多余行");

    // 2 行：高 2。
    begin_edit(
        &mut state,
        TodoEdit::new(Some(id.clone()), "x".repeat(width + 1), 0),
    );
    let layout = layout_visible_items(area, &state);
    assert_eq!(layout.items[0].rendered_rows, 2);
    assert_eq!(layout.items[1].y, list.y + 2);

    // 4 行文本：固定 3 行，光标在第 4 行 → 视口从第 2 行开始。
    let long = "x".repeat(width.saturating_mul(3).saturating_add(3));
    begin_edit(&mut state, TodoEdit::new(Some(id), long, 0));
    let layout = layout_visible_items(area, &state);
    assert_eq!(layout.items[0].rendered_rows, 3);
    assert_eq!(layout.items[1].y, list.y + 3, "超过 3 行仍只占 3 行");

    let (_, cursor) = render_panel(&state, area);
    let (x, y) = cursor.expect("编辑行必须返回光标坐标");
    assert_eq!(y, list.y + 2, "光标在第 3 个可见行");
    assert_eq!(x, row_text_x(list) + 3, "光标列 = 末行剩余字符宽度");
}

/// 多行编辑的命中测试：任意可见行都命中同一条目，优先级只在首行，点击定位按行偏移换算。
#[test]
fn multiline_item_hits_rows_and_cursor_position() {
    let mut state = AppState::demo();
    let area = panel(38);
    let list = list_rect(area);
    let width = edit_row_width(area).max(1);
    let text_x = row_text_x(list);
    state.todo.replace(vec![
        TodoRecord::new("a", TodoStatus::Todo, 0),
        TodoRecord::new("b", TodoStatus::Todo, 1),
    ]);
    let id = state.todo.items()[0].id.clone();
    let long = "x".repeat(width.saturating_mul(3).saturating_add(3));
    begin_edit(&mut state, TodoEdit::new(Some(id), long, 0));

    // 三个可见行都命中条目 0。
    for row in 0..3u16 {
        assert_eq!(
            row_at(area, &state, text_x, list.y + row),
            Some(0),
            "第 {row} 个视觉行命中条目 0"
        );
    }
    assert_eq!(row_at(area, &state, text_x, list.y + 3), Some(1));

    // 优先级徽标只在首行命中。
    let px = row_priority_x(list);
    assert_eq!(priority_at(area, &state, px, list.y), Some(0));
    assert_eq!(priority_at(area, &state, px, list.y + 1), None);

    // 光标在第 4 行 → 可见窗口为第 2~4 行；点击第 2 个可见行第 2 列 → 第 3 行第 2 个字符。
    assert_eq!(
        edit_input_char_at(area, &state, text_x + 2, list.y + 1),
        Some(width.saturating_mul(2).saturating_add(2))
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
    let text_x = row_text_x(list);
    let (x, _) = cursor.expect("编辑行必须返回光标坐标");
    assert_eq!(x, text_x + 6, "光标列 = 文本显示宽度，不是字符数");
    // 文本也确实按两列一格铺开。
    assert_eq!(buffer[(text_x, list.y)].symbol(), "中");
    assert_eq!(buffer[(text_x + 4, list.y)].symbol(), "名");
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

#[test]
fn priority_badges_have_distinct_backgrounds_and_high_contrast_foregrounds() {
    let mut state = AppState::demo();
    let area = panel(38);
    let today = state.todo.today_date().to_string();
    state.todo.replace(vec![
        TodoRecord::new_on(&today, "p0", TodoStatus::Todo, TodoPriority::P0, 0),
        TodoRecord::new_on(&today, "p1", TodoStatus::Todo, TodoPriority::P1, 1),
        TodoRecord::new_on(&today, "p2", TodoStatus::Todo, TodoPriority::P2, 2),
        TodoRecord::new_on(&today, "p3", TodoStatus::Todo, TodoPriority::P3, 3),
    ]);

    let (buffer, _) = render_panel(&state, area);
    let list = list_rect(area);
    let px = row_priority_x(list);

    let mut bgs = std::collections::BTreeSet::new();
    let mut fgs = std::collections::BTreeSet::new();
    for row in 0..4u16 {
        let y = list.y + row;
        let cell = &buffer[(px, y)];
        assert_ne!(cell.bg, Color::Reset, "第 {row} 行徽标必须带专属底色");
        assert_ne!(cell.fg, Color::Reset, "第 {row} 行徽标必须带显式前景色");
        assert_ne!(cell.fg, cell.bg, "前景色与背景色不得相同");
        bgs.insert(format!("{:?}", cell.bg));
        fgs.insert(format!("{:?}", cell.fg));
        assert_eq!(priority_at(area, &state, px, y), Some(usize::from(row)));
    }
    assert_eq!(bgs.len(), 4, "四档优先级必须拥有 4 种互不相同的背景色");
    assert_eq!(fgs.len(), 4, "四档优先级必须拥有 4 种互不相同的前景色");
}

#[test]
fn ghost_row_before_selected_keeps_selection_highlight_aligned_and_scrollbar_counts_ghost() {
    let mut state = AppState::demo();
    let area = panel(44);
    let today = state.todo.today_date().to_string();
    state.todo.replace(vec![
        TodoRecord::new_on(&today, "row-0", TodoStatus::Todo, TodoPriority::P1, 0),
        TodoRecord::new_on(&today, "row-1", TodoStatus::Todo, TodoPriority::P1, 1),
    ]);
    crate::app::update::select_right_tab(&mut state, RightTab::Todo);
    // 选中第 1 项（"row-1"），但在第 0 位置打开新建幽灵行（模拟幽灵行在选中行之前）：
    state.todo.select(1);
    state
        .todo
        .begin_edit(crate::app::state::TodoEdit::new(None, String::new(), 0));

    let (buffer, _) = render_panel(&state, area);
    let list = effective_list_rect(area, &state);
    // 显示行 0 是幽灵行，显示行 1 是 row-0（未选中），显示行 2 是 row-1（选中，带 ❯ 与 selection 底色）：
    assert_eq!(buffer[(list.x, list.y + 1)].symbol(), " ");
    assert_eq!(
        buffer[(list.x, list.y + 1)].bg,
        Color::Reset,
        "未选中的 row-0 不应被幽灵行偏移误染选中底色"
    );
    assert_eq!(
        buffer[(list.x, list.y + 2)].symbol(),
        text::AGENT_SELECTED_MARKER
    );
    assert_eq!(
        buffer[(list.x, list.y + 2)].bg,
        style::selection().bg.unwrap_or(Color::Reset),
        "选中的 row-1 必须同时拥有 ❯ 标记与选中背景色"
    );
}

#[test]
fn multiline_items_scroll_into_view_on_down_and_render_scrollbar() {
    let mut state = AppState::demo();
    state.right_tab = RightTab::Todo;
    state.set_focus(crate::app::state::FocusedContainer::Todo);
    let area = panel(38);
    let list = list_rect(area);
    let width = text_width(list).max(1);
    // 列表高度约 4 行，放入 4 条各占 3 行的待办（逻辑条数 4 <= list.height，但总视觉行数 12 > list.height）：
    let three_line = "m".repeat(width.saturating_mul(3));
    state.todo.replace(
        (0..4)
            .map(|idx| TodoRecord::new(format!("{idx}-{three_line}"), TodoStatus::Todo, idx))
            .collect(),
    );

    crate::app::update::resolve_todo_viewport(&mut state, area);
    let bar0 = scrollbar(area, &state).expect("多行展开总行高溢出列表高度时必须显示滚动条");
    assert_eq!(state.todo.scroll(), 0);
    assert_eq!(bar0.thumb.y, bar0.track.y, "初始 thumb 贴顶");

    // 连续按 ↓ 导航到末项 3：必须逐项推进 scroll，使末项完整落入视口且滚动条到底。
    for expected in 1..=3 {
        assert!(crate::app::update::move_todo_selection(&mut state, 1));
        crate::app::update::resolve_todo_viewport(&mut state, area);
        assert_eq!(state.todo.selected(), Some(expected));
        let layout = layout_visible_items(area, &state);
        let item = layout
            .items
            .iter()
            .find(|item| item.display == expected)
            .expect("选中条目必须在可见列表内");
        assert_eq!(
            item.rendered_rows, 3,
            "选中条目 {expected} 的全部 3 个视觉行必须完整可见，不得被底边裁剪"
        );
    }
    let bar_end = scrollbar(area, &state).expect("末项仍显示滚动条");
    assert_eq!(
        bar_end.thumb.bottom(),
        bar_end.track.bottom(),
        "滚到最大偏移时 thumb 触底"
    );

    // 手动滚轮上滚到顶部后，resolve_todo_viewport 不得把视口强行拉回 selected (3)。
    assert!(crate::app::update::scroll_todo(&mut state, -10));
    assert_eq!(state.todo.scroll(), 0);
    crate::app::update::resolve_todo_viewport(&mut state, area);
    assert_eq!(
        state.todo.scroll(),
        0,
        "手动滚轮滚动后不应被自动吸附回选中项"
    );
}

#[test]
fn multiline_edit_keeps_top_click_stable_and_clamps_drag_coordinates() {
    let mut state = AppState::demo();
    state.right_tab = RightTab::Todo;
    state.set_focus(crate::app::state::FocusedContainer::Todo);
    let area = panel(38);
    let list = list_rect(area);
    let width = edit_row_width(area).max(1);
    let text_x = row_text_x(list);
    state
        .todo
        .replace(vec![TodoRecord::new("seed", TodoStatus::Todo, 0)]);
    let id = state.todo.items()[0].id.clone();

    // 4 行文本，初始光标在末尾（第 4 行，scroll_row = 1，显示第 1..=3 行）：
    let long = format!(
        "{}{}{}{}",
        "a".repeat(width),
        "b".repeat(width),
        "c".repeat(width),
        "de"
    );
    begin_edit(&mut state, TodoEdit::new(Some(id), long, 0));
    crate::app::update::resolve_todo_viewport(&mut state, area);
    assert_eq!(state.todo.editing().expect("edit").scroll_row, 1);

    // 点击可见第 0 行（对应第 1 行 'b' 段）第 1 列：
    let clicked = edit_input_char_at(area, &state, text_x + 1, list.y).expect("hit row 0");
    assert_eq!(clicked, width + 1);
    state
        .todo
        .edit_input_mut()
        .expect("input")
        .begin_selection(clicked);
    crate::app::update::resolve_todo_viewport(&mut state, area);
    assert_eq!(
        state.todo.editing().expect("edit").scroll_row,
        1,
        "点击可见首行后视口不得跳回第 0 行"
    );

    // 拖拽进入左侧 9 列缩进区钳为该可见行行首（width），越过右缘钳为该可见行行尾（width * 2）：
    assert_eq!(
        edit_input_drag_char_at(area, &state, list.x, list.y),
        Some(width),
        "拖入左侧缩进区延伸到当前可见行首"
    );
    assert_eq!(
        edit_input_drag_char_at(area, &state, list.right() + 5, list.y),
        Some(width * 2),
        "拖出列表右缘延伸到满宽行行尾（覆盖末字符）"
    );
    // 向上拖出编辑项顶边钳到可见第 0 行，向下拖出编辑项底边钳到可见末行（第 2 行）：
    assert_eq!(
        edit_input_drag_char_at(area, &state, text_x + 2, list.y.saturating_sub(3)),
        Some(width + 2)
    );
    assert_eq!(
        edit_input_drag_char_at(area, &state, list.right() + 5, list.y + 99),
        Some(width * 3 + 2)
    );

    // 极窄面板（list.right() <= text_x）下 render_edit_item 安全返回 None：
    let narrow = Rect::new(0, 0, 8, PANEL_HEIGHT);
    let (_, narrow_cursor) = render_panel(&state, narrow);
    assert!(narrow_cursor.is_none(), "极窄宽度无正文列时不返回越界光标");
}
