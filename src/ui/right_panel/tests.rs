//! 单元测试；仅测试构建编译。
//!
//! 渲染走 `crate::ui::render`（与 `ui::tests` 同款），这样容器底边框的折叠按钮
//! 也参与断言；几何用 `layout::compute` 复算，与生产渲染同一份状态输入。

use super::*;
use crate::app::state::AppState;
use crate::config::Config;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::style::{Color, Modifier};

const SCREEN: Rect = Rect {
    x: 0,
    y: 0,
    width: 100,
    height: 24,
};

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

fn render_buffer(state: &AppState) -> ratatui::buffer::Buffer {
    let config = Config::default();
    let mut terminal =
        Terminal::new(TestBackend::new(100, 24)).expect("test backend is infallible");
    if let Err(error) = terminal.draw(|frame| crate::ui::render(frame, state, &config)) {
        panic!("draw failed: {error}");
    }
    terminal.backend().buffer().clone()
}

/// 容器某一行的文本（按容器列范围裁剪）。
fn panel_row(
    buffer: &ratatui::buffer::Buffer,
    view: &crate::ui::layout::ViewLayout,
    y: u16,
) -> String {
    (view.prompt.x..view.prompt.right())
        .map(|x| buffer[(x, y)].symbol())
        .collect()
}

#[test]
fn tabs_render_on_top_border_with_accent_active() {
    let state = AppState::demo();
    let view = view_for(&state);
    let buffer = render_buffer(&state);
    let rects = tab_rects(view.prompt);
    assert_eq!(rects.len(), 2, "宽度充足时两个 tab 都在: {rects:?}");
    let (prompt_tab, prompt_rect) = rects[0];
    let (todo_tab, todo_rect) = rects[1];
    assert_eq!(prompt_tab, RightTab::Prompt, "默认激活 Prompt");
    assert_eq!(todo_tab, RightTab::Todo);

    // 顶边框整行含两个 tab 文案。
    let border = panel_row(&buffer, &view, view.prompt.y);
    assert!(border.contains(text::RIGHT_TAB_PROMPT_LABEL), "{border:?}");
    assert!(border.contains(text::RIGHT_TAB_TODO_LABEL), "{border:?}");

    // 激活 tab：`selected_item`（青底黑字加粗），左右内边距与文字区同款。
    let selected = style::selected_item();
    let cell = &buffer[(prompt_rect.x, prompt_rect.y)];
    assert_eq!(Some(cell.bg), selected.bg, "{cell:?}");
    assert_eq!(Some(cell.fg), selected.fg, "{cell:?}");
    assert!(cell.modifier.contains(Modifier::BOLD), "{cell:?}");
    let label_cell = &buffer[(prompt_rect.x + 1, prompt_rect.y)];
    assert_eq!(label_cell.bg, Color::Cyan, "{label_cell:?}");
    assert_eq!(label_cell.fg, Color::Black, "{label_cell:?}");
    assert!(
        label_cell.modifier.contains(Modifier::BOLD),
        "{label_cell:?}"
    );

    // 非激活 tab：muted（dim、无前景无背景、不加粗）。
    let muted = &buffer[(todo_rect.x, todo_rect.y)];
    assert_eq!(muted.bg, Color::Reset, "{muted:?}");
    assert!(
        muted.modifier.contains(Modifier::DIM),
        "非激活 tab 必须 dim: {muted:?}"
    );
    assert!(
        !muted.modifier.contains(Modifier::BOLD),
        "非激活 tab 不得加粗: {muted:?}"
    );

    // 聚焦态边框是 accent（Cyan 加粗）：`muted()` 不带前景，若不先重置本格，
    // 非激活 tab 会继承开边框的青色加粗，看起来也亮着。见 `paint_tabs`。
    let mut focused = AppState::demo();
    focused.prompt_focused = true;
    let buffer = render_buffer(&focused);
    let muted = &buffer[(todo_rect.x, todo_rect.y)];
    assert_eq!(
        muted.fg,
        Color::Reset,
        "聚焦时非激活 tab 仍须弱化: {muted:?}"
    );
    assert!(
        !muted.modifier.contains(Modifier::BOLD),
        "聚焦时非激活 tab 不得继承边框加粗: {muted:?}"
    );
    let active = &buffer[(prompt_rect.x, prompt_rect.y)];
    assert_eq!(active.bg, Color::Cyan, "激活 tab 仍为青底: {active:?}");
}

#[test]
fn tabs_drop_trailing_when_narrow() {
    // 只够一个 tab（Prompt 8 列 + 起排 2 列 + 右边角 1 列）：丢掉尾部的 Todo。
    let mut state = AppState::demo();
    state.prompt_width = 12;
    let view = view_for(&state);
    assert_eq!(view.prompt.width, 12, "布局至少保留最小窗格宽度");
    let rects = tab_rects(view.prompt);
    assert_eq!(rects.len(), 1, "只保留 Prompt: {rects:?}");
    assert_eq!(rects[0].0, RightTab::Prompt);
    let buffer = render_buffer(&state);
    let border = panel_row(&buffer, &view, view.prompt.y);
    assert!(border.contains(text::RIGHT_TAB_PROMPT_LABEL), "{border:?}");
    assert!(
        !border.contains(text::RIGHT_TAB_TODO_LABEL),
        "尾部 tab 必须丢弃: {border:?}"
    );

    // 连 Prompt 都放不下：整排 tab 不画，顶边框两端圆角仍在。
    state.prompt_width = 10;
    let view = view_for(&state);
    assert_eq!(view.prompt.width, 10);
    assert!(tab_rects(view.prompt).is_empty(), "放不下任何 tab");
    let buffer = render_buffer(&state);
    let border = panel_row(&buffer, &view, view.prompt.y);
    assert!(!border.contains(text::RIGHT_TAB_PROMPT_LABEL), "{border:?}");
    assert!(!border.contains(text::RIGHT_TAB_TODO_LABEL), "{border:?}");
    assert!(border.starts_with('╭'), "左圆角仍在: {border:?}");
    assert!(border.ends_with('╮'), "右圆角仍在: {border:?}");
}

#[test]
fn tab_hit_areas() {
    let state = AppState::demo();
    let view = view_for(&state);
    let panel = view.prompt;
    let row = panel.y;
    let rects = tab_rects(panel);
    assert_eq!(rects.len(), 2);
    let rows = render_buffer(&state);

    for (tab, rect) in &rects {
        assert_eq!(tab_at(panel, rect.x, row), Some(*tab), "tab 起点命中");
        assert_eq!(
            tab_at(panel, rect.right() - 1, row),
            Some(*tab),
            "tab 文字区右端命中"
        );
    }

    // tab 之间的那一列 `─` 不是命中区。
    let gap = rects[0].1.right();
    assert_eq!(
        rows[(gap, row)].symbol(),
        text::DIVIDER_MID,
        "tab 之间是 `─`"
    );
    assert_eq!(tab_at(panel, gap, row), None, "分隔符不命中");

    // 顶边框右侧空白属于容器控件区，但不算 tab。
    assert_eq!(tab_at(panel, panel.right() - 2, row), None);
    assert_eq!(tab_at(panel, panel.right() - 1, row), None);

    // 顶边框整行都是 tab 行；其他行不是。
    assert!(tab_row_at(panel, panel.x, row));
    assert!(tab_row_at(panel, panel.right() - 1, row));
    assert!(!tab_row_at(panel, panel.x, row + 1));
    assert!(!tab_row_at(panel, panel.x, panel.bottom() - 1));
}

#[test]
fn todo_body_is_blank_with_plain_divider() {
    let mut state = AppState::demo();
    state.prompt_width = 56;
    state.right_tab = RightTab::Todo;
    let view = view_for(&state);
    let buffer = render_buffer(&state);
    let inner = crate::layout::pane_inner_rect(view.prompt);

    // 功能按钮行：整行除两侧边框外全空白。
    for x in inner.x..inner.right() {
        assert_eq!(
            buffer[(x, inner.y)].symbol(),
            " ",
            "Todo 按钮行第 {x} 列必须空白"
        );
    }

    // 分割线行：两端 `├` / `┤`，中间全 `─`（无条目名、无页控件）。
    let divider_y = inner.y + crate::layout::PROMPT_TOOLBAR_HEIGHT;
    assert_eq!(
        buffer[(view.prompt.x, divider_y)].symbol(),
        text::DIVIDER_LEFT_JOIN
    );
    assert_eq!(
        buffer[(view.prompt.right() - 1, divider_y)].symbol(),
        text::DIVIDER_RIGHT_JOIN
    );
    for x in inner.x..inner.right() {
        assert_eq!(
            buffer[(x, divider_y)].symbol(),
            text::DIVIDER_MID,
            "Todo 分割线第 {x} 列必须为 `─`"
        );
    }

    // 内容区：零文案，整块空白。
    for y in (divider_y + 1)..view.prompt.bottom().saturating_sub(1) {
        for x in inner.x..inner.right() {
            assert_eq!(
                buffer[(x, y)].symbol(),
                " ",
                "Todo 内容区 ({x},{y}) 必须空白"
            );
        }
    }
}

#[test]
fn bottom_identity_survives_tab_switch() {
    let mut state = AppState::demo();
    state.prompt_width = 56;
    let view = view_for(&state);
    let bottom_y = view.prompt.bottom() - 1;
    let identity = state.workspaces[0].identity();

    let prompt_bottom = panel_row(&render_buffer(&state), &view, bottom_y);
    assert!(
        prompt_bottom.contains(identity.project.as_str()),
        "底边框含身份标签: {prompt_bottom:?}"
    );
    assert!(
        prompt_bottom.contains(text::RIGHT_COLLAPSE_LABEL),
        "底边框含折叠按钮: {prompt_bottom:?}"
    );

    state.right_tab = RightTab::Todo;
    let todo_bottom = panel_row(&render_buffer(&state), &view, bottom_y);
    assert_eq!(
        prompt_bottom, todo_bottom,
        "底边框（身份标签 + 折叠按钮）不得随 tab 变化"
    );
}
