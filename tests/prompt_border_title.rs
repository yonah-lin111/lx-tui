//! 集成测试：prompt 顶边框标题 / 页名 / 页控件（真实 AppState + 渲染 + 命中几何）。
//!
//! 覆盖需求：编辑视图的条目 title 与分页控件都移到面板顶边框（不再有 `Prompt Manager`），
//! 树视图顶边框只显示固定标题、不显示任何分页控件，工具栏下方分割线保持空行。

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

/// 顶边框整行（含左右圆角）。
fn border_row(terminal: &Terminal<TestBackend>, view: &ui::layout::ViewLayout) -> String {
    panel_row(terminal, view, view.prompt.y)
}

/// 字符串首字符在屏幕上的列。
fn column_of(row: &str, needle: &str, origin: u16) -> u16 {
    let byte = row.find(needle).expect("needle visible");
    origin + u16::try_from(row[..byte].chars().count()).expect("col fits")
}

#[test]
fn editor_border_shows_item_title_page_name_and_pager() {
    let (mut state, config, view) = ready_state();
    paged_prompt(&mut state);
    update::focus_prompt(&mut state);
    let terminal = draw(&state, &config);
    let border = border_row(&terminal, &view);

    assert!(border.starts_with('╭'), "顶边框仍是圆角边框: {border}");
    assert!(border.ends_with('╮'), "{border}");
    assert!(border.contains(" paged "), "条目名进顶边框: {border}");
    assert!(border.contains("· Page 1"), "页名跟随条目名: {border}");
    assert!(border.contains("1/2"), "页控件进顶边框: {border}");
    assert!(border.contains('‹') && border.contains('›'), "{border}");

    // 旧的固定标题彻底消失。
    assert!(!border.contains("Prompt Manager"), "{border}");
    let all: String = {
        let buffer = terminal.backend().buffer();
        buffer.content.iter().map(|cell| cell.symbol()).collect()
    };
    assert!(!all.contains("Prompt Manager"), "全屏不再出现旧标题");
}

#[test]
fn divider_row_is_plain_after_title_moved_to_border() {
    let (mut state, config, view) = ready_state();
    paged_prompt(&mut state);
    update::focus_prompt(&mut state);
    let terminal = draw(&state, &config);
    let inner = layout::pane_inner_rect(view.prompt);
    let divider = panel_row(&terminal, &view, inner.y + layout::PROMPT_TOOLBAR_HEIGHT);

    assert!(divider.starts_with(text::DIVIDER_LEFT_JOIN), "{divider}");
    assert!(divider.ends_with(text::DIVIDER_RIGHT_JOIN), "{divider}");
    assert!(
        !divider.contains(" paged "),
        "分割线不再承载标题: {divider}"
    );
    assert!(!divider.contains("1/2"), "分割线不再承载页控件: {divider}");
    assert!(
        divider
            .chars()
            .skip(1)
            .take(usize::from(view.prompt.width) - 2)
            .all(|ch| ch == text::DIVIDER_MID.chars().next().unwrap_or('─')),
        "{divider}"
    );
}

#[test]
fn border_pager_hit_matches_rendered_columns() {
    let (mut state, config, view) = ready_state();
    paged_prompt(&mut state);
    update::focus_prompt(&mut state);
    let terminal = draw(&state, &config);
    let border = border_row(&terminal, &view);
    let row = view.prompt.y;

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
        "左圆角不响应: {border}"
    );
    assert_eq!(
        ui::prompt_page_control_at(view.prompt, &state, prev_col, row + 1),
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
    let border = border_row(&terminal, &view);
    assert!(border.contains("· Page 2"), "{border}");
    assert!(border.contains("2/2"), "{border}");

    // 末页右箭头置灰（Reset 前景），左箭头可用（Cyan）。
    let buffer = terminal.backend().buffer();
    let next_col = column_of(&border, text::PROMPT_PAGE_NEXT, view.prompt.x);
    let prev_col = column_of(&border, text::PROMPT_PAGE_PREV, view.prompt.x);
    assert_eq!(
        buffer[(next_col, view.prompt.y)].fg,
        ratatui::style::Color::Reset,
        "末页右箭头置灰: {border}"
    );
    assert_eq!(
        buffer[(prev_col, view.prompt.y)].fg,
        ratatui::style::Color::Cyan,
        "左箭头可用: {border}"
    );
}

#[test]
fn tree_border_shows_fixed_title_without_pager() {
    let (mut state, config, view) = ready_state();
    paged_prompt(&mut state);
    update::show_prompt_tree(&mut state);
    let terminal = draw(&state, &config);
    let border = border_row(&terminal, &view);

    assert!(
        border.contains(text::PROMPT_TREE_BORDER_TITLE),
        "树视图顶边框显示固定标题: {border}"
    );
    assert!(!border.contains("1/2"), "列表页不分页: {border}");
    assert!(!border.contains('‹') && !border.contains('›'), "{border}");
    assert!(!border.contains("Prompt Manager"), "{border}");
    assert_eq!(
        ui::prompt_page_control_at(view.prompt, &state, view.prompt.x + 3, view.prompt.y),
        None,
        "树视图顶边框无页控件命中"
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
    let border = border_row(&terminal, &view);

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
    let border = border_row(&terminal, &view);
    assert!(border.contains("· Page 1X"), "输入写入顶边框: {border}");

    // 硬件光标落在输入末尾（`X` 之后一列）。
    let cursor = terminal.get_cursor_position().expect("cursor position");
    let x_col = column_of(&border, "Page 1X", view.prompt.x);
    assert_eq!(
        cursor,
        Position::new(x_col + "Page 1X".len() as u16, view.prompt.y),
        "硬件光标跟随页名输入: {border}"
    );
}
