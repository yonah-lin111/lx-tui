//! 集成测试：右侧容器的 tab、prompt 标题行 / 页名 / 页控件（真实 AppState + 渲染 + 命中几何）。
//!
//! 覆盖需求：编辑视图的条目 title 与分页控件内嵌在**功能按钮行下方的分割线**上，
//! 树视图标题行为纯 `─`、不显示任何分页控件；容器顶边框只承载 `Prompt` / `Todo` 标签页。

use lx_tui::app::actions::OverlayKey;
use lx_tui::app::prompt::{PromptItem, PromptPage};
use lx_tui::app::state::AppState;
use lx_tui::app::update;
use lx_tui::config::Config;
use lx_tui::layout;
use lx_tui::ui;
use lx_tui::ui::text;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::{Position, Rect};

/// 单个真实字符的终端列宽（用于把 ASCII 字符串位置换算成列）。
const PANEL_WIDTH: u16 = 64;

/// 构造已同步几何的 prompt 场景（140x30，右栏展开到足够宽）。
fn ready_state() -> (AppState, Config, ui::layout::ViewLayout) {
    let config = Config::default();
    let mut state = AppState::demo();
    state.prompt_width = PANEL_WIDTH;
    let screen = Rect::new(0, 0, 140, 30);
    let view = ui::layout::compute(
        screen,
        &config,
        state.sidebar_collapsed,
        state.sidebar_width,
        state.prompt_collapsed,
        state.prompt_width,
        state.prompt_left,
        state.panes_collapsed,
    );
    let mut rects = layout::pane_rects(
        &state.active_tab().layout,
        view.panes,
        config.min_pane_width,
        config.min_pane_height,
    );
    rects.push((state.prompt.id(), view.prompt));
    update::resize_panes(&mut state, &rects);
    (state, config, view)
}

/// 建一个两页的正式条目并进入编辑视图。
fn paged_prompt(state: &mut AppState) {
    let mut item = PromptItem::single(7, None, "paged", "alpha");
    item.pages.push(PromptPage::saved(70, "Page 2", "beta"));
    state.workspaces[0]
        .prompt_panel
        .tree
        .replace(vec![], vec![item]);
    assert!(update::edit_prompt_item(state, 0, 7));
}

fn draw(state: &AppState, config: &Config) -> Terminal<TestBackend> {
    let mut terminal =
        Terminal::new(TestBackend::new(140, 30)).expect("test backend is infallible");
    terminal
        .draw(|frame| ui::render(frame, state, config))
        .expect("draw succeeds");
    terminal
}

/// 面板内某一行（按面板列范围裁剪）。
fn panel_row(terminal: &Terminal<TestBackend>, view: &ui::layout::ViewLayout, y: u16) -> String {
    let buffer = terminal.backend().buffer();
    (view.prompt.x..view.prompt.right())
        .map(|x| buffer[(x, y)].symbol())
        .collect()
}

/// 容器顶边框整行（含左右圆角，承载 tab）。
fn border_row(terminal: &Terminal<TestBackend>, view: &ui::layout::ViewLayout) -> String {
    panel_row(terminal, view, view.prompt.y)
}

/// 标题行（工具栏下一行，承载条目名/页名/页控件）。
fn title_row(terminal: &Terminal<TestBackend>, view: &ui::layout::ViewLayout) -> String {
    panel_row(
        terminal,
        view,
        layout::pane_inner_rect(view.prompt).y + layout::PROMPT_TOOLBAR_HEIGHT,
    )
}

/// 字符串首字符在屏幕上的列。
fn column_of(row: &str, needle: &str, origin: u16) -> u16 {
    let byte = row.find(needle).expect("needle visible");
    origin + u16::try_from(row[..byte].chars().count()).expect("col fits")
}

#[test]
fn editor_title_row_shows_item_title_page_name_and_pager() {
    let (mut state, config, view) = ready_state();
    paged_prompt(&mut state);
    update::focus_prompt(&mut state);
    let terminal = draw(&state, &config);

    // 容器顶边框只承载 tab。
    let border = border_row(&terminal, &view);
    assert!(border.starts_with('╭'), "顶边框仍是圆角边框: {border}");
    assert!(border.ends_with('╮'), "{border}");
    assert!(border.contains(text::RIGHT_TAB_PROMPT_LABEL), "{border}");
    assert!(border.contains(text::RIGHT_TAB_TODO_LABEL), "{border}");

    // 条目名、页名与页控件都落在那条分割线上。
    let title = title_row(&terminal, &view);
    assert!(title.starts_with(text::DIVIDER_LEFT_JOIN), "{title}");
    assert!(title.ends_with(text::DIVIDER_RIGHT_JOIN), "{title}");
    assert!(title.contains(" paged "), "条目名进标题行: {title}");
    assert!(title.contains("· Page 1"), "页名跟随条目名: {title}");
    assert!(title.contains("1/2"), "页控件进标题行: {title}");
    assert!(title.contains('‹') && title.contains('›'), "{title}");

    // 旧的固定标题彻底消失。
    let all: String = {
        let buffer = terminal.backend().buffer();
        buffer.content.iter().map(|cell| cell.symbol()).collect()
    };
    assert!(!all.contains("Prompt Manager"), "全屏不再出现旧标题");
}

#[test]
fn toolbar_row_carries_buttons_not_title() {
    let (mut state, config, view) = ready_state();
    paged_prompt(&mut state);
    update::focus_prompt(&mut state);
    let terminal = draw(&state, &config);
    let inner = layout::pane_inner_rect(view.prompt);
    let toolbar = panel_row(&terminal, &view, inner.y);

    assert!(toolbar.contains(text::PROMPT_MODE_LIST_LABEL), "{toolbar}");
    assert!(!toolbar.contains(" paged "), "工具栏不承载标题: {toolbar}");
    assert!(!toolbar.contains("1/2"), "工具栏不承载页控件: {toolbar}");
    assert!(
        !toolbar.contains(text::DIVIDER_MID),
        "分割线不穿过工具栏: {toolbar}"
    );
}

#[test]
fn border_pager_hit_matches_rendered_columns() {
    let (mut state, config, view) = ready_state();
    paged_prompt(&mut state);
    update::focus_prompt(&mut state);
    let terminal = draw(&state, &config);
    let border = title_row(&terminal, &view);
    let row = layout::pane_inner_rect(view.prompt).y + layout::PROMPT_TOOLBAR_HEIGHT;

    // 由真实渲染结果反推控件列：`‹` 切上一页、`›` 切下一页、`n/m` 开页菜单。
    let prev_col = column_of(&border, text::PROMPT_PAGE_PREV, view.prompt.x);
    let next_col = column_of(&border, text::PROMPT_PAGE_NEXT, view.prompt.x);
    let counter_col = column_of(&border, "1/2", view.prompt.x);

    assert_eq!(
        ui::prompt_page_control_at(view.prompt, &state, prev_col, row),
        Some(ui::prompt::PageControl::Prev),
        "左箭头命中: {border}"
    );
    assert_eq!(
        ui::prompt_page_control_at(view.prompt, &state, next_col, row),
        Some(ui::prompt::PageControl::Next),
        "右箭头命中: {border}"
    );
    assert_eq!(
        ui::prompt_page_control_at(view.prompt, &state, counter_col, row),
        Some(ui::prompt::PageControl::Counter),
        "页码命中（打开页菜单）: {border}"
    );

    // 页名段命中进入重命名；条目名与边框空白不产生控件。
    let name_col = column_of(&border, "· Page 1", view.prompt.x) + 2;
    assert_eq!(
        ui::prompt_page_control_at(view.prompt, &state, name_col, row),
        Some(ui::prompt::PageControl::Name),
        "页名命中: {border}"
    );
    assert_eq!(
        ui::prompt_page_control_at(view.prompt, &state, view.prompt.x + 3, row),
        None,
        "条目名不响应: {border}"
    );
    assert_eq!(
        ui::prompt_page_control_at(view.prompt, &state, view.prompt.x, row),
        None,
        "左端 `├` 不响应: {border}"
    );
    assert_eq!(
        ui::prompt_page_control_at(view.prompt, &state, prev_col, row - 1),
        None,
        "工具栏行无页控件命中: {border}"
    );
}

#[test]
fn border_switches_page_and_rename_roundtrip() {
    let (mut state, config, view) = ready_state();
    paged_prompt(&mut state);
    update::focus_prompt(&mut state);

    assert!(update::switch_prompt_page(&mut state, 1));
    assert_eq!(state.prompt.text(), "beta");
    let terminal = draw(&state, &config);
    let border = title_row(&terminal, &view);
    assert!(border.contains("· Page 2"), "{border}");
    assert!(border.contains("2/2"), "{border}");

    // 末页右箭头置灰（Reset 前景），左箭头可用（Cyan）。
    let buffer = terminal.backend().buffer();
    let row = layout::pane_inner_rect(view.prompt).y + layout::PROMPT_TOOLBAR_HEIGHT;
    let next_col = column_of(&border, text::PROMPT_PAGE_NEXT, view.prompt.x);
    let prev_col = column_of(&border, text::PROMPT_PAGE_PREV, view.prompt.x);
    assert_eq!(
        buffer[(next_col, row)].fg,
        ratatui::style::Color::Reset,
        "末页右箭头置灰: {border}"
    );
    assert_eq!(
        buffer[(prev_col, row)].fg,
        ratatui::style::Color::Cyan,
        "左箭头可用: {border}"
    );
}

#[test]
fn tree_title_row_is_plain_without_pager() {
    let (mut state, config, view) = ready_state();
    paged_prompt(&mut state);
    update::show_prompt_tree(&mut state);
    let terminal = draw(&state, &config);
    let title = title_row(&terminal, &view);

    assert!(title.starts_with(text::DIVIDER_LEFT_JOIN), "{title}");
    assert!(title.ends_with(text::DIVIDER_RIGHT_JOIN), "{title}");
    assert!(
        title
            .chars()
            .skip(1)
            .take(usize::from(view.prompt.width) - 2)
            .all(|ch| ch == text::DIVIDER_MID.chars().next().unwrap_or('─')),
        "树视图标题行为纯分割线: {title}"
    );
    assert!(!title.contains("1/2"), "列表页不分页: {title}");
    assert!(!title.contains('‹') && !title.contains('›'), "{title}");
    let row = layout::pane_inner_rect(view.prompt).y + layout::PROMPT_TOOLBAR_HEIGHT;
    assert_eq!(
        ui::prompt_page_control_at(view.prompt, &state, view.prompt.x + 3, row),
        None,
        "树视图标题行无页控件命中"
    );
}

#[test]
fn border_title_truncates_page_name_before_item_title() {
    let (mut state, config, view) = ready_state();
    let long_name = "long-page-name-".repeat(6);
    let mut item = PromptItem::single(11, None, "item", "body");
    item.pages[0].name = long_name.clone();
    state.workspaces[0]
        .prompt_panel
        .tree
        .replace(vec![], vec![item]);
    assert!(update::edit_prompt_item(&mut state, 0, 11));
    update::focus_prompt(&mut state);
    let terminal = draw(&state, &config);
    let border = title_row(&terminal, &view);

    assert!(border.contains(" item "), "条目名优先完整: {border}");
    assert!(border.contains("1/1"), "页控件完整: {border}");
    assert!(!border.contains(&long_name), "页名被截断: {border}");
    assert!(border.contains('…'), "页名以省略号收尾: {border}");
}

#[test]
fn border_rename_input_cursor_tracks_hardware_position() {
    let (mut state, config, view) = ready_state();
    paged_prompt(&mut state);
    update::focus_prompt(&mut state);
    assert!(update::begin_page_rename(&mut state));
    update::apply_page_rename_key(&mut state, OverlayKey::Char('X'));

    let mut terminal = draw(&state, &config);
    let border = title_row(&terminal, &view);
    let row = layout::pane_inner_rect(view.prompt).y + layout::PROMPT_TOOLBAR_HEIGHT;
    assert!(border.contains("· Page 1X"), "输入写入标题行: {border}");

    // 硬件光标落在输入末尾（`X` 之后一列）。
    let cursor = terminal.get_cursor_position().expect("cursor position");
    let x_col = column_of(&border, "Page 1X", view.prompt.x);
    assert_eq!(
        cursor,
        Position::new(x_col + "Page 1X".len() as u16, row),
        "硬件光标跟随页名输入: {border}"
    );
}

#[test]
fn focused_page_rename_keeps_top_border_line() {
    let (mut state, config, view) = ready_state();
    paged_prompt(&mut state);
    update::focus_prompt(&mut state);

    // 失焦态基准：`├─ paged · Page 1 ──────‹ 1/2 ›─┤`。
    let idle = draw(&state, &config);
    let idle_border = title_row(&idle, &view);
    assert!(idle_border.ends_with("›─┤"), "{idle_border}");

    // 回归：聚焦重命名（输入仍是当前页名）后标题行应逐字符不变，
    // 右侧边框线不得被清成空白。
    assert!(update::begin_page_rename(&mut state));
    let focused = draw(&state, &config);
    let focused_border = title_row(&focused, &view);
    assert_eq!(
        focused_border, idle_border,
        "重命名输入不改变标题行：聚焦={focused_border} 失焦={idle_border}"
    );

    // 继续输入也不得擦掉右侧边框线：输入后只多出输入的字符与 1 列空白。
    update::apply_page_rename_key(&mut state, OverlayKey::Char('X'));
    let typed = draw(&state, &config);
    let typed_border = title_row(&typed, &view);
    assert!(typed_border.contains("· Page 1X "), "{typed_border}");
    let tail = typed_border.split("· Page 1X").nth(1).expect("输入可见");
    let before_controls = tail.split('‹').next().unwrap_or_default();
    assert!(
        before_controls
            .strip_prefix(' ')
            .is_some_and(|rest| !rest.is_empty() && rest.chars().all(|ch| ch == '─')),
        "输入右侧应是 1 列空白加连续边框线: {typed_border}"
    );
}
