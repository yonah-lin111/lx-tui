//! 集成测试：右侧容器的 tab、prompt 标题行 / 页名 / 页控件（真实 AppState + 渲染 + 命中几何）。
//!
//! 覆盖需求：编辑视图的条目 title 与页名内嵌在**功能按钮行下方的分割线**上，
//! 页控件 `‹ n/m ›` 位于工具栏 `[refDir]` 与保存点之间；树视图标题行为纯 `─`；
//! 容器顶边框只承载 `Prompt` / `Todo` 标签页。

use lx_tui::app::prompt::{PromptItem, PromptPage};
use lx_tui::app::state::AppState;
use lx_tui::app::update;
use lx_tui::config::Config;
use lx_tui::layout;
use lx_tui::ui;
use lx_tui::ui::text;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;

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

/// 标题行（工具栏下一行，承载条目名/页名）。
fn title_row(terminal: &Terminal<TestBackend>, view: &ui::layout::ViewLayout) -> String {
    panel_row(
        terminal,
        view,
        layout::pane_inner_rect(view.prompt).y + layout::PROMPT_TOOLBAR_HEIGHT,
    )
}

/// 工具栏行（承载模式、`[rec]`、页控件与保存点）。
fn toolbar_row(terminal: &Terminal<TestBackend>, view: &ui::layout::ViewLayout) -> String {
    panel_row(terminal, view, layout::pane_inner_rect(view.prompt).y)
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
    assert!(border.starts_with('┌'), "顶边框为直角边框: {border}");
    assert!(border.ends_with('┐'), "{border}");
    assert!(border.contains(text::RIGHT_TAB_PROMPT_LABEL), "{border}");
    assert!(border.contains(text::RIGHT_TAB_TODO_LABEL), "{border}");

    // 条目名与页名都落在那条分割线上。
    let title = title_row(&terminal, &view);
    assert!(title.starts_with(text::DIVIDER_LEFT_JOIN), "{title}");
    assert!(title.ends_with(text::DIVIDER_RIGHT_JOIN), "{title}");
    assert!(title.contains(" paged "), "条目名进标题行: {title}");
    assert!(title.contains("· Page 1"), "当前页名进标题行: {title}");
    assert!(!title.contains("1/2"), "页控件已移到工具栏: {title}");
    assert!(!title.contains('‹') && !title.contains('›'), "{title}");

    // 页控件位于工具栏 `[refDir]` 与保存点之间。
    let toolbar = toolbar_row(&terminal, &view);
    assert!(toolbar.contains("1/2"), "页控件进工具栏: {toolbar}");
    assert!(toolbar.contains('‹') && toolbar.contains('›'), "{toolbar}");

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
    assert!(toolbar.contains("1/2"), "工具栏承载页控件: {toolbar}");
    assert!(toolbar.contains('‹') && toolbar.contains('›'), "{toolbar}");
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
    let toolbar = toolbar_row(&terminal, &view);
    let row = layout::pane_inner_rect(view.prompt).y;

    // 由真实渲染结果反推控件列：`‹` 切上一页、`›` 切下一页、`n/m` 开页菜单。
    let prev_col = column_of(&toolbar, text::PROMPT_PAGE_PREV, view.prompt.x);
    let next_col = column_of(&toolbar, text::PROMPT_PAGE_NEXT, view.prompt.x);
    let counter_col = column_of(&toolbar, "1/2", view.prompt.x);

    assert_eq!(
        ui::prompt_page_control_at(view.prompt, &state, prev_col, row),
        Some(ui::prompt::PageControl::Prev),
        "左箭头命中: {toolbar}"
    );
    assert_eq!(
        ui::prompt_page_control_at(view.prompt, &state, next_col, row),
        Some(ui::prompt::PageControl::Next),
        "右箭头命中: {toolbar}"
    );
    assert_eq!(
        ui::prompt_page_control_at(view.prompt, &state, counter_col, row),
        Some(ui::prompt::PageControl::Counter),
        "页码命中（打开页菜单）: {toolbar}"
    );

    // 条目名与边框空白不产生控件。
    assert_eq!(
        ui::prompt_page_control_at(view.prompt, &state, view.prompt.x + 3, row),
        None,
        "模式按钮不响应: {toolbar}"
    );
    assert_eq!(
        ui::prompt_page_control_at(view.prompt, &state, view.prompt.x, row),
        None,
        "左端 `│` 不响应: {toolbar}"
    );
    let title_y = layout::pane_inner_rect(view.prompt).y + layout::PROMPT_TOOLBAR_HEIGHT;
    assert_eq!(
        ui::prompt_page_control_at(view.prompt, &state, prev_col, title_y),
        None,
        "标题行无页控件命中: {toolbar}"
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
    let title = title_row(&terminal, &view);
    assert!(title.contains("Page 2"), "标题行展示当前页名: {title}");
    let toolbar = toolbar_row(&terminal, &view);
    assert!(toolbar.contains("2/2"), "{toolbar}");

    // 末页右箭头置灰（Reset 前景），左箭头可用（Cyan）。
    let buffer = terminal.backend().buffer();
    let row = layout::pane_inner_rect(view.prompt).y;
    let next_col = column_of(&toolbar, text::PROMPT_PAGE_NEXT, view.prompt.x);
    let prev_col = column_of(&toolbar, text::PROMPT_PAGE_PREV, view.prompt.x);
    assert_eq!(
        buffer[(next_col, row)].fg,
        ratatui::style::Color::Reset,
        "末页右箭头置灰: {toolbar}"
    );
    assert_eq!(
        buffer[(prev_col, row)].fg,
        ratatui::style::Color::Cyan,
        "左箭头可用: {toolbar}"
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
fn border_title_truncates_item_title_when_narrow() {
    let (mut state, config, view) = ready_state();
    let long_title = "long-item-title-".repeat(6);
    let item = PromptItem::single(11, None, &long_title, "body");
    state.workspaces[0]
        .prompt_panel
        .tree
        .replace(vec![], vec![item]);
    assert!(update::edit_prompt_item(&mut state, 0, 11));
    update::focus_prompt(&mut state);
    let terminal = draw(&state, &config);
    let border = title_row(&terminal, &view);
    let toolbar = toolbar_row(&terminal, &view);

    assert!(toolbar.contains("1/1"), "页控件在工具栏: {toolbar}");
    assert!(!border.contains(&long_title), "条目名过长被截断: {border}");
    assert!(border.contains('…'), "条目名以省略号收尾: {border}");
}
