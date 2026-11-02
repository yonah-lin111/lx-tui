//! 页列表视图鼠标交互单元测试：单击选中、双击打开、点击记录隔离、滚轮滚动与计数切换。

use super::test_support::*;
use super::*;
use crate::app::actions::EditorCommand;
use crate::app::prompt::PromptPage;
use crate::ui::text;

fn multi_page_state() -> AppState {
    let mut state = paged_prompt_state();
    let item = state.workspaces[0].prompt_panel.tree.item_mut(7).unwrap();
    for index in 3..=15 {
        item.pages.push(PromptPage::saved(
            70 + index,
            format!("Page {index}"),
            format!("content {index}"),
        ));
    }
    state
}

#[test]
fn single_click_selects_page_row_without_switching_page() {
    let config = Config::default();
    let mut state = multi_page_state();
    assert!(crate::app::update::show_prompt_pages(&mut state));
    assert_eq!(state.workspaces[0].prompt_panel.mode, PromptMode::Pages);
    assert_eq!(state.workspaces[0].prompt_panel.pages_view.selected, 0);

    let geo = geometry(&state, &config, Rect::new(0, 0, 140, 30));
    let text_area = crate::layout::prompt_text_rect(geo.view.prompt);
    let mut sessions = HashMap::new();
    let mut dirty = false;

    // 点击第 2 行（y = text_area.y + 2 + 1）：只选中行 1，仍留在 Pages 视图，active_page 不变。
    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            text_area.x + 2,
            text_area.y + 3,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert_eq!(state.workspaces[0].prompt_panel.mode, PromptMode::Pages);
    assert_eq!(state.workspaces[0].prompt_panel.pages_view.selected, 1);
    assert_eq!(state.prompt.text(), "alpha", "单击不切页");
}

#[test]
fn double_click_within_window_opens_selected_page() {
    let config = Config::default();
    let mut state = multi_page_state();
    assert!(crate::app::update::show_prompt_pages(&mut state));

    let geo = geometry(&state, &config, Rect::new(0, 0, 140, 30));
    let text_area = crate::layout::prompt_text_rect(geo.view.prompt);
    let mut sessions = HashMap::new();
    let mut dirty = false;

    for _ in 0..2 {
        handle_terminal_event(
            mouse(
                MouseEventKind::Down(MouseButton::Left),
                text_area.x + 2,
                text_area.y + 3,
            ),
            &mut state,
            &mut sessions,
            &geo,
            &config,
            &mut ui::RenderCache::default(),
            &mut dirty,
        );
    }
    assert_eq!(
        state.workspaces[0].prompt_panel.mode,
        PromptMode::Editor,
        "双击页行打开该页并回到编辑视图"
    );
    assert_eq!(state.prompt.text(), "beta");
}

#[test]
fn tree_click_history_does_not_bleed_into_pages_single_click() {
    let config = Config::default();
    let mut state = multi_page_state();
    // 模拟在 Tree 视图刚刚点击过第 1 行。
    state.prompt_row_click = Some((1, std::time::Instant::now()));
    assert!(crate::app::update::show_prompt_pages(&mut state));

    let geo = geometry(&state, &config, Rect::new(0, 0, 140, 30));
    let text_area = crate::layout::prompt_text_rect(geo.view.prompt);
    let mut sessions = HashMap::new();
    let mut dirty = false;

    // 进入 Pages 后第一次单击第 1 行：不应被误判为双击。
    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            text_area.x + 2,
            text_area.y + 3,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert_eq!(
        state.workspaces[0].prompt_panel.mode,
        PromptMode::Pages,
        "Tree 的点击记录不得与 Pages 共用导致单击被误判为双击"
    );
    assert_eq!(state.workspaces[0].prompt_panel.pages_view.selected, 1);
}

#[test]
fn wheel_scrolls_page_list_without_moving_selection() {
    let config = Config::default();
    let mut state = multi_page_state();
    state.prompt.resize(40, 6);
    assert!(crate::app::update::show_prompt_pages(&mut state));

    let geo = geometry(&state, &config, Rect::new(0, 0, 140, 10));
    let text_area = crate::layout::prompt_text_rect(geo.view.prompt);
    let mut sessions = HashMap::new();
    let mut dirty = false;

    handle_terminal_event(
        mouse(MouseEventKind::ScrollDown, text_area.x + 2, text_area.y + 2),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(state.workspaces[0].prompt_panel.pages_view.scroll > 0);
    assert_eq!(
        state.workspaces[0].prompt_panel.pages_view.selected, 0,
        "滚轮不移动选中行"
    );
}

#[test]
fn clicking_title_counter_toggles_between_editor_and_pages() {
    let config = Config::default();
    let mut state = paged_prompt_state();
    let geo = geometry(&state, &config, Rect::new(0, 0, 140, 30));
    let bar = crate::layout::prompt_toolbar_rect(geo.view.prompt).expect("toolbar");
    let mut sessions = HashMap::new();
    let mut dirty = false;

    // Editor 视图下点击计数：进入 Pages 视图。
    let editor_controls = ui::prompt::toolbar_page_controls(bar, "1/2", true).expect("controls");
    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            editor_controls.counter.x,
            editor_controls.counter.y,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert_eq!(state.workspaces[0].prompt_panel.mode, PromptMode::Pages);

    // Pages 视图下点击计数（无箭头几何）：返回 Editor 视图。
    let pages_controls = ui::prompt::toolbar_page_controls(bar, "1/2", false).expect("controls");
    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            pages_controls.counter.x,
            pages_controls.counter.y,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert_eq!(state.workspaces[0].prompt_panel.mode, PromptMode::Editor);
}

#[test]
fn ctrl_shift_right_creates_page_or_toasts_when_empty_page_already_exists() {
    let mut state = AppState::demo();
    state.prompt.set_text("page 1 content");
    crate::app::update::commit_editing_prompt(&mut state);
    crate::app::update::focus_prompt(&mut state);
    state.workspaces[0].prompt_panel.mode = PromptMode::Editor;

    // 当前仅 Page 1，正文非空。按 Ctrl+Shift+Right 新建页：创建 Page 2，切入并弹 toast。
    crate::app::update::apply_editor(&mut state, EditorCommand::NewPage);
    assert!(state.toast.is_some(), "新建页弹出 toast");
    assert_eq!(
        state.toast.as_ref().map(|t| t.message.as_str()),
        Some(text::TOAST_PAGE_CREATED)
    );
    assert_eq!(state.prompt.text(), "");

    // 此时 Page 2 是空页，再次按 Ctrl+Shift+Right：不再新建，提示已有新页。
    state.toast = None;
    crate::app::update::apply_editor(&mut state, EditorCommand::NewPage);
    assert_eq!(
        state.toast.as_ref().map(|t| t.message.as_str()),
        Some(text::TOAST_PAGE_ALREADY_NEW)
    );
}
