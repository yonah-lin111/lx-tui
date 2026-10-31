use super::test_support::*;
use super::*;

#[test]
fn mouse_wheel_over_short_prompt_does_not_scroll_without_scrollbar() {
    // 回归：正文不溢出、滚动条不出现时，滚轮不得移动视口，也不产生多余重绘。
    let config = Config::default();
    let mut state = demo_terminal();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let inner = crate::layout::prompt_text_rect(geo.view.prompt);
    state.prompt.resize(inner.width, inner.height);
    state.prompt.insert_str("1\n2");
    assert!(
        ui::prompt_scrollbar(&geo.view, &state).is_none(),
        "前提：短正文没有滚动条"
    );
    let mut sessions = HashMap::new();
    let mut dirty = false;

    handle_terminal_event(
        mouse(MouseEventKind::ScrollDown, inner.x + 1, inner.y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert_eq!(state.prompt.scroll(), 0, "没有滚动条时滚轮不移动视口");
    assert!(!dirty, "没有位移就不要触发重绘");
}

#[test]
fn mouse_track_click_prompt_scrollbar_scrolls_without_focus() {
    let config = Config::default();
    let mut state = demo_terminal();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    overflow_prompt(&mut state, &geo.view);
    let bar = ui::prompt_scrollbar(&geo.view, &state).expect("scrollbar is visible");
    let mut sessions = HashMap::new();
    let mut dirty = false;

    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            bar.track.x,
            bar.track.bottom() - 1,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(state.prompt.scroll() > 0, "track click jumps down");
    assert!(
        !state.prompt_focused(),
        "scrollbar press must not focus prompt"
    );
    assert!(state.prompt_scroll_drag.is_none());
    assert!(dirty);
}

#[test]
fn mouse_track_click_prompt_tree_scrollbar_scrolls_list() {
    use crate::app::prompt::PromptItem;
    let config = Config::default();
    let mut state = demo_terminal();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    update::show_prompt_tree(&mut state);
    let panel = &mut state.workspaces[0].prompt_panel;
    panel.tree.replace(
        vec![],
        (0..40)
            .map(|index| PromptItem::single(index, None, format!("item-{index:02}"), String::new()))
            .collect(),
    );
    let bar = ui::prompt_scrollbar(&geo.view, &state).expect("scrollbar is visible");
    let gutter = crate::layout::prompt_tree_scrollbar_rect(geo.view.prompt).expect("gutter");
    assert_eq!(bar.track, gutter, "树视图滚动条槽在搜索框与分割线之下");
    let mut sessions = HashMap::new();
    let mut dirty = false;

    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            bar.track.x,
            bar.track.bottom() - 1,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(state.workspaces[0].prompt_panel.scroll > 0, "轨道点击跳转");
    assert!(!state.prompt_focused(), "滚动条按下不抢焦点");
    assert!(state.prompt_scroll_drag.is_none());
    assert!(dirty);
}

#[test]
fn mouse_drag_prompt_scrollbar_thumb() {
    let config = Config::default();
    let mut state = demo_terminal();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    overflow_prompt(&mut state, &geo.view);
    let bar = ui::prompt_scrollbar(&geo.view, &state).expect("scrollbar is visible");
    let max = ui::widgets::scrollbar::offset_from_drag_row(&bar, bar.track.bottom() - 1, 0);
    assert!(max > 0);
    let mut sessions = HashMap::new();
    let mut dirty = false;

    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            bar.thumb.x,
            bar.thumb.y,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert_eq!(state.prompt_scroll_drag, Some(0));

    handle_terminal_event(
        mouse(
            MouseEventKind::Drag(MouseButton::Left),
            bar.track.x,
            bar.track.bottom() - 1,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert_eq!(state.prompt.scroll(), max);

    handle_terminal_event(
        mouse(
            MouseEventKind::Up(MouseButton::Left),
            bar.track.x,
            bar.track.bottom() - 1,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(state.prompt_scroll_drag.is_none());
}

#[test]
fn mouse_wheel_during_prompt_selection_scrolls_viewport() {
    let config = Config::default();
    let mut state = demo_terminal();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    overflow_prompt(&mut state, &geo.view);
    let inner = crate::layout::prompt_text_rect(geo.view.prompt);
    let mut sessions = HashMap::new();
    let mut dirty = false;

    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            inner.x + 1,
            inner.y,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    handle_terminal_event(
        mouse(
            MouseEventKind::Drag(MouseButton::Left),
            inner.x + 1,
            inner.y + 1,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(
        state
            .selection
            .is_some_and(|selection| selection.is_dragging())
    );

    handle_terminal_event(
        mouse(MouseEventKind::ScrollDown, inner.x + 1, inner.y + 1),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(state.prompt.scroll() > 0, "wheel scrolls prompt viewport");
    let range = state
        .selection
        .and_then(|selection| selection.range())
        .expect("selection range");
    assert_eq!(range.0.0, 0, "anchor stays pinned to its text line");
    assert!(
        range.1.0 > 0,
        "cursor follows the mouse cell in content rows"
    );
    assert!(dirty);
}

#[test]
fn mouse_motion_tracks_prompt_tree_hover() {
    let config = Config::default();
    let mut state = demo_terminal();
    crate::app::update::show_prompt_tree(&mut state);
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let area = crate::layout::prompt_text_rect(geo.view.prompt);
    let scroll = state.prompt_owner().prompt_panel.scroll;
    let hovered = state.prompt_owner().prompt_panel.selected;
    let rows_area_y = area.y + 2;
    let mut sessions = HashMap::new();
    let mut dirty = false;

    // 悬停树行列表区（选中行可见）。
    handle_terminal_event(
        mouse(
            MouseEventKind::Moved,
            area.x + 2,
            rows_area_y + (hovered - scroll) as u16,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert_eq!(state.prompt_tree_hover, Some(hovered));
    assert!(dirty, "悬停变化置脏重绘");

    // 移到搜索行（区域内非列表行）：清除悬停。
    dirty = false;
    handle_terminal_event(
        mouse(MouseEventKind::Moved, area.x + 2, area.y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert_eq!(state.prompt_tree_hover, None);
    assert!(dirty, "离开列表行清空悬停");

    // 离开提示词面板：保持清空。
    handle_terminal_event(
        mouse(
            MouseEventKind::Moved,
            geo.view.panes.x + 2,
            geo.view.panes.y + 2,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert_eq!(state.prompt_tree_hover, None);

    // 编辑视图（非树视图）不产生列表悬停。
    crate::app::update::toggle_prompt_panel_mode(&mut state);
    assert!(
        !crate::app::update::prompt_tree_mode(&state),
        "已切到编辑视图"
    );
    handle_terminal_event(
        mouse(MouseEventKind::Moved, area.x + 2, rows_area_y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert_eq!(state.prompt_tree_hover, None);
}

#[test]
fn mouse_interacts_with_block_command_panel() {
    let config = Config::default();
    let mut state = AppState::demo();
    state.prompt.resize(40, 8);
    crate::app::update::focus_prompt(&mut state);
    crate::app::update::apply_editor(
        &mut state,
        crate::app::actions::EditorCommand::InsertChar('#'),
    );
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let area = crate::layout::prompt_text_rect(geo.view.prompt);
    let panel = crate::ui::prompt::panel_layout(&state.prompt, area).expect("block panel visible");
    let rect = panel.rect;
    let mut sessions = HashMap::new();
    let mut dirty = false;

    // 滚轮悬停在面板上：只滚动视口（记录显式视口起点），不移动高亮、不滚动 prompt 视口。
    handle_terminal_event(
        mouse(MouseEventKind::ScrollDown, rect.x + 2, rect.y + 1),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert_eq!(
        state.prompt.panel().map(|panel| panel.active()),
        Some(0),
        "滚轮不动高亮"
    );
    assert_eq!(
        state.prompt.panel().map(|panel| panel.viewport()),
        Some(Some(3))
    );

    // 悬停条目：只改高亮（内容区从 rect.y + 3 开始）。
    handle_terminal_event(
        mouse(MouseEventKind::Moved, rect.x + 2, rect.y + 6),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert_eq!(state.prompt.panel().map(|panel| panel.active()), Some(3));

    // 左键点击第 2 项：确认插入 H2 并关闭面板。
    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            rect.x + 2,
            rect.y + 4,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert_eq!(state.prompt.text(), "## ");
    assert!(state.prompt.panel().is_none());
}

#[test]
fn mouse_wheel_over_mention_panel_scrolls_viewport_only() {
    let config = Config::default();
    let mut state = AppState::demo();
    state.prompt.resize(40, 8);
    crate::app::update::focus_prompt(&mut state);
    crate::app::update::apply_editor(
        &mut state,
        crate::app::actions::EditorCommand::InsertChar('@'),
    );
    let (generation, _) = state
        .prompt
        .take_mention_scan_request()
        .expect("scan requested");
    let entries: Vec<crate::app::markdown::MentionEntry> = (0..8)
        .map(|index| crate::app::markdown::MentionEntry {
            path: format!("src/f{index}.rs"),
            is_directory: false,
        })
        .collect();
    let prompt = state.prompt.id();
    crate::app::update::apply_mention_entries(&mut state, prompt, generation, entries);

    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let area = crate::layout::prompt_text_rect(geo.view.prompt);
    let before = crate::ui::prompt::mention_layout(&state.prompt, area).expect("mention panel");
    let mut sessions = HashMap::new();
    let mut dirty = false;

    handle_terminal_event(
        mouse(
            MouseEventKind::ScrollDown,
            before.rect.x + 2,
            before.rect.y + 1,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );

    let after = crate::ui::prompt::mention_layout(&state.prompt, area).expect("mention panel");
    assert_eq!(after.start, before.start + 3, "滚轮下移视口 3 项");
    assert_eq!(
        state.prompt.mention().map(|panel| panel.active()),
        Some(0),
        "滚轮不动高亮"
    );
    assert!(dirty);
}

#[test]
fn mouse_click_prompt_pin_toggles_pin() {
    let mut state = demo_terminal();
    let (config, geo) = toolbar_geometry(&mut state);
    let mut sessions = HashMap::new();
    let mut dirty = false;

    let pin = layout::prompt_toolbar_button_rect(
        geo.view.prompt,
        layout::PromptToolbarButton::Pin,
        false,
    )
    .expect("pin visible");
    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Left), pin.x, pin.y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert_eq!(state.prompt_pinned, Some(state.active_workspace));
    assert!(dirty);

    let unpin = layout::prompt_toolbar_button_rect(
        geo.view.prompt,
        layout::PromptToolbarButton::Pin,
        false,
    )
    .expect("pin visible");
    dirty = false;
    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Left), unpin.x, unpin.y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert_eq!(state.prompt_pinned, None);
    assert!(dirty);
}

#[test]
fn mouse_click_prompt_block_focus_roundtrips_without_text_change() {
    let mut state = demo_terminal();
    let (config, geo) = toolbar_geometry(&mut state);
    let mut sessions = HashMap::new();
    let mut dirty = false;
    let text = "intro\n&&& a --start\n{title: A}\nbody-a\n&&& a --end\noutro";
    state.prompt.set_text(text);
    let cursor = state.prompt.text().find("body-a").expect("body") + 1;
    state.prompt.set_cursor_offset_for_test(cursor);

    let block_focus = toolbar_button(&geo, layout::PromptToolbarButton::BlockFocus);
    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            block_focus.x,
            block_focus.y,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(state.prompt.block_focus_active(), "点击 [⊞] 进入块模式");
    assert_eq!(state.prompt.text(), text, "进入模式不改正文");
    assert!(dirty);

    let exit = toolbar_button(&geo, layout::PromptToolbarButton::BlockFocus);
    dirty = false;
    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Left), exit.x, exit.y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(!state.prompt.block_focus_active(), "再点 [⊡] 退出块模式");
    assert_eq!(state.prompt.text(), text, "退出模式不改正文");
    assert!(dirty);
}

#[test]
fn mouse_click_prompt_toolbar_undo_redo_and_save() {
    let mut state = demo_terminal();
    let (config, geo) = toolbar_geometry(&mut state);
    let mut sessions = HashMap::new();
    let mut dirty = false;

    state.prompt.insert_str("hello");
    assert!(!state.prompt.is_saved());
    assert!(state.prompt.can_undo());

    let undo = toolbar_button(&geo, layout::PromptToolbarButton::Undo);
    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Left), undo.x, undo.y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert_eq!(state.prompt.text(), "");
    assert!(state.prompt.can_redo());
    assert!(!state.prompt_focused(), "工具栏按钮不抢焦点");
    assert!(dirty);

    let redo = toolbar_button(&geo, layout::PromptToolbarButton::Redo);
    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Left), redo.x, redo.y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert_eq!(state.prompt.text(), "hello");

    // 默认绑定临时提示词：点击保存点提交缓冲并打开保存选择器。
    let save = toolbar_button(&geo, layout::PromptToolbarButton::Save);
    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Left), save.x, save.y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(
        matches!(
            state.overlay,
            Some(crate::app::overlay::Overlay::SaveDraft(_))
        ),
        "临时提示词保存点打开保存选择器"
    );
    let Some(crate::app::prompt::PromptEditing::Temp(temp_id)) =
        state.workspaces[0].prompt_panel.editing
    else {
        panic!("temp bound");
    };
    assert_eq!(
        state.workspaces[0]
            .prompt_panel
            .temp(temp_id)
            .map(|temp| temp.active_content()),
        Some("hello"),
        "提交缓冲到 temp"
    );
}

#[test]
fn mouse_click_prompt_toolbar_select_all_then_typing_replaces_text() {
    let mut state = demo_terminal();
    let (config, geo) = toolbar_geometry(&mut state);
    let mut sessions = HashMap::new();
    let mut dirty = false;

    state.prompt.insert_str("first\nsecond");
    let select_all = toolbar_button(&geo, layout::PromptToolbarButton::SelectAll);
    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            select_all.x,
            select_all.y,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(
        state
            .selection
            .is_some_and(|selection| selection.pane() == state.prompt.id())
    );
    assert_eq!(
        update::prompt_selection_text(&state).as_deref(),
        Some("first\nsecond")
    );

    update::apply_editor(
        &mut state,
        crate::app::actions::EditorCommand::InsertText("new".into()),
    );
    assert_eq!(state.prompt.text(), "new", "全选后输入替换全文");
}

#[test]
fn mouse_click_prompt_header_swallows_without_focus_or_cursor() {
    let mut state = demo_terminal();
    let (config, geo) = toolbar_geometry(&mut state);
    state.prompt.insert_str("ab\ncd");
    let mut sessions = HashMap::new();
    let mut dirty = false;

    let bar = layout::prompt_toolbar_rect(geo.view.prompt).expect("toolbar visible");
    let select_all = toolbar_button(&geo, layout::PromptToolbarButton::SelectAll);
    let blank_x = select_all.x - 1;
    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Left), blank_x, bar.y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            select_all.right() + 2,
            bar.y,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );

    assert!(!state.prompt_focused(), "表头空白不聚焦文本视口");
    assert!(state.selection.is_none());
    assert_eq!(state.prompt.cursor_cell(), Some((1, 2)), "光标保持在文末");
    assert!(!dirty, "表头空白点击无副作用");
}

#[test]
fn mouse_click_prompt_text_row_places_cursor_below_header() {
    let mut state = demo_terminal();
    let (config, geo) = toolbar_geometry(&mut state);
    state.prompt.insert_str("ab\ncd");
    let mut sessions = HashMap::new();
    let mut dirty = false;

    let text = layout::prompt_text_rect(geo.view.prompt);
    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            text.x + 1,
            text.y + 1,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(state.prompt_focused());
    assert_eq!(state.prompt.cursor_cell(), Some((1, 1)));
    assert!(dirty);
}

#[test]
fn mouse_drag_prompt_item_into_folder_queues_move_effect() {
    use crate::app::prompt::{PromptFolder, PromptItem};
    let mut state = demo_terminal();
    let (config, geo) = toolbar_geometry(&mut state);
    let folder_row = prompt_tree_click(
        &mut state,
        &geo,
        vec![PromptFolder {
            id: 1,
            parent_id: None,
            name: "folder".into(),
            collapsed: false,
        }],
        vec![PromptItem::single(7, None, "item", "item body")],
        PromptRowKind::Item,
    );
    let area = layout::prompt_text_rect(geo.view.prompt);
    let folder_index = state.workspaces[0]
        .prompt_panel
        .rows()
        .iter()
        .position(|row| row.kind == PromptRowKind::Folder)
        .expect("folder row");
    let folder_point = (area.x + 1, area.y + 2 + folder_index as u16);
    prompt_tree_drag(&mut state, &geo, &config, folder_row, folder_point);
    assert!(
        state.effects.iter().any(|effect| matches!(
            effect,
            crate::app::effects::Effect::MovePrompt {
                id: 7,
                folder_id: Some(1),
                ..
            }
        )),
        "拖条目进文件夹登记移动效应: {:?}",
        state.effects
    );
}

/// 拖动中滚轮不得改视口：落点与跨行阈值都按绝对行号换算，滚动会让指针原地"跨行"，
/// 松手时把条目静默移进此时指针所在行的容器。
#[test]
fn mouse_wheel_during_prompt_drag_keeps_viewport_and_state() {
    use crate::app::prompt::{PromptFolder, PromptItem};
    let mut state = demo_terminal();
    let (config, geo) = toolbar_geometry(&mut state);
    // 文件夹内一个条目 + 一批根层条目，使可见行超过视口（panel.scroll 可动）。
    let mut items = vec![PromptItem::single(7, Some(1), "inside", "inside body")];
    items.extend((0..40).map(|n| PromptItem::single(100 + n, None, format!("item {n}"), "body")));
    let item_point = prompt_tree_click(
        &mut state,
        &geo,
        vec![PromptFolder {
            id: 1,
            parent_id: None,
            name: "folder".into(),
            collapsed: false,
        }],
        items,
        PromptRowKind::Item,
    );
    let visible = update::prompt_tree_visible_rows(&state);
    assert!(
        state.prompt_owner().prompt_panel.rows().len() > visible,
        "夹具：列表必须长于视口才能滚动"
    );
    assert_eq!(state.prompt_owner().prompt_panel.scroll, 0);

    let mut sessions = HashMap::new();
    let mut dirty = false;
    for kind in [
        MouseEventKind::Down(MouseButton::Left),
        MouseEventKind::ScrollDown,
        MouseEventKind::Up(MouseButton::Left),
    ] {
        handle_terminal_event(
            mouse(kind, item_point.0, item_point.1),
            &mut state,
            &mut sessions,
            &geo,
            &config,
            &mut ui::RenderCache::default(),
            &mut dirty,
        );
    }

    assert_eq!(
        state.prompt_owner().prompt_panel.scroll,
        0,
        "拖动中滚轮不改视口"
    );
    assert!(state.prompt_drag.is_none(), "松开清理拖拽现场");
    assert!(
        !state.effects.iter().any(|effect| matches!(
            effect,
            crate::app::effects::Effect::MovePrompt { .. }
                | crate::app::effects::Effect::MoveFolder { .. }
        )),
        "指针未动，不得登记移动效应: {:?}",
        state.effects
    );
}

#[test]
fn mouse_drag_prompt_row_does_not_open_editor() {
    use crate::app::prompt::{PromptFolder, PromptItem};
    let mut state = demo_terminal();
    let (config, geo) = toolbar_geometry(&mut state);
    let item_point = prompt_tree_click(
        &mut state,
        &geo,
        vec![PromptFolder {
            id: 1,
            parent_id: None,
            name: "folder".into(),
            collapsed: false,
        }],
        vec![
            PromptItem::single(7, None, "item", "item body"),
            PromptItem::single(8, None, "other", "other body"),
        ],
        PromptRowKind::Item,
    );
    let area = layout::prompt_text_rect(geo.view.prompt);
    let rows = state.workspaces[0].prompt_panel.rows().len();
    let blank = (area.x + 1, area.y + 2 + rows as u16);
    prompt_tree_drag(&mut state, &geo, &config, item_point, blank);
    assert_eq!(
        state.workspaces[0].prompt_panel.mode,
        PromptMode::Tree,
        "拖动结束不打开编辑器"
    );
}

/// 拖动中指针移出窗口会丢掉 Up：下一次无关按下必须先清场，否则它的 Up 会按旧行号
/// 消费残留现场（例如点工具栏却折叠了树里的文件夹）。
#[test]
fn mouse_down_discards_stale_prompt_drag() {
    use crate::app::prompt::{PromptFolder, PromptItem};
    let mut state = demo_terminal();
    let (config, geo) = toolbar_geometry(&mut state);
    let folder_point = prompt_tree_click(
        &mut state,
        &geo,
        vec![PromptFolder {
            id: 1,
            parent_id: None,
            name: "folder".into(),
            collapsed: false,
        }],
        vec![PromptItem::single(7, Some(1), "item", "item body")],
        PromptRowKind::Folder,
    );
    let mut sessions = HashMap::new();
    let mut dirty = false;
    // 只按下：现场登记、文件夹尚未折叠。
    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            folder_point.0,
            folder_point.1,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(state.prompt_drag.is_some(), "按下登记现场");
    // 模拟 Up 丢失后的一次全新按下（别处）：旧现场必须被丢弃。
    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Left), 1, 1),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(state.prompt_drag.is_none(), "新按下丢弃残留现场");
    assert!(
        !state.workspaces[0]
            .prompt_panel
            .tree
            .folder(1)
            .is_some_and(|folder| folder.collapsed),
        "残留现场不得在无关松开时折叠文件夹"
    );
}

#[test]
fn mouse_double_click_prompt_item_row_opens_editor() {
    use crate::app::prompt::{PromptEditing, PromptItem, PromptMode};
    let mut state = demo_terminal();
    let (config, geo) = toolbar_geometry(&mut state);
    let point = prompt_tree_click(
        &mut state,
        &geo,
        vec![],
        vec![PromptItem::single(7, None, "item", "item body")],
        PromptRowKind::Item,
    );

    prompt_tree_double_click(&mut state, &geo, &config, point);
    assert_eq!(state.workspaces[0].prompt_panel.mode, PromptMode::Editor);
    assert_eq!(
        state.workspaces[0].prompt_panel.editing,
        Some(PromptEditing::Item(7))
    );
    assert_eq!(
        state.prompt.text(),
        "item body",
        "双击进入编辑并载入条目内容"
    );
    assert!(state.prompt_focused());
}

#[test]
fn mouse_right_click_prompt_item_keeps_selection() {
    use crate::app::prompt::PromptItem;
    let mut state = demo_terminal();
    let (config, geo) = toolbar_geometry(&mut state);
    let item_row = prompt_tree_click(
        &mut state,
        &geo,
        vec![],
        vec![
            PromptItem::single(7, None, "one", "first body"),
            PromptItem::single(8, None, "two", "second body"),
        ],
        PromptRowKind::Item,
    );
    // 选中停在第二个条目上，右键落在第一个条目行。
    assert!(crate::app::update::select_prompt_item(&mut state, 0, 8));
    let selected_before = state.workspaces[0].prompt_panel.selected;
    let other = state.workspaces[0]
        .prompt_panel
        .rows()
        .into_iter()
        .position(|row| {
            state.workspaces[0]
                .prompt_panel
                .tree
                .item_of_row(row)
                .is_some_and(|item| item.id == 7)
        })
        .expect("item 7 row");
    assert_ne!(selected_before, other, "右键目标行与选中行不同");

    let mut sessions = HashMap::new();
    let mut dirty = false;
    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Right),
            item_row.0,
            item_row.1,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );

    let Some(Overlay::Menu(menu)) = state.overlay.as_ref() else {
        panic!("prompt item menu expected");
    };
    assert_eq!(
        menu.target,
        crate::app::overlay::OverlayTarget::PromptItem {
            workspace: 0,
            item_id: 7
        }
    );
    assert_eq!(
        state.workspaces[0].prompt_panel.selected, selected_before,
        "右键只挂菜单：不改动列表选中行"
    );
}

#[test]
fn mouse_double_click_prompt_temp_row_opens_editor() {
    use crate::app::prompt::{PromptEditing, PromptMode};
    let mut state = demo_terminal();
    let (config, geo) = toolbar_geometry(&mut state);
    let point = prompt_tree_click(&mut state, &geo, vec![], vec![], PromptRowKind::Temp);

    prompt_tree_double_click(&mut state, &geo, &config, point);
    assert_eq!(state.workspaces[0].prompt_panel.mode, PromptMode::Editor);
    assert!(
        matches!(
            state.workspaces[0].prompt_panel.editing,
            Some(PromptEditing::Temp(_))
        ),
        "双击临时提示词行进入其编辑视图"
    );
}

#[test]
fn mouse_double_click_prompt_folder_toggles_once() {
    use crate::app::prompt::{PromptFolder, PromptItem};
    let mut state = demo_terminal();
    let (config, geo) = toolbar_geometry(&mut state);
    let point = prompt_tree_click(
        &mut state,
        &geo,
        vec![PromptFolder {
            id: 1,
            parent_id: None,
            name: "folder".into(),
            collapsed: false,
        }],
        vec![PromptItem::single(7, Some(1), "item", "item body")],
        PromptRowKind::Folder,
    );

    prompt_tree_double_click(&mut state, &geo, &config, point);
    assert!(
        state.workspaces[0]
            .prompt_panel
            .tree
            .folder(1)
            .is_some_and(|folder| folder.collapsed),
        "双击文件夹只折叠一次，不来回切换"
    );
}

/// 悬停换算走真实入口：幽灵行打开时把鼠标移到 b 那一行，`todo_hover` 必须是 b 的**列表**下标。
#[test]
fn hover_below_ghost_row_stores_list_index() {
    let mut state = demo_terminal();
    let (config, geo) = toolbar_geometry(&mut state);
    assert!(crate::app::update::select_right_tab(
        &mut state,
        RightTab::Todo
    ));
    state.todo.replace(vec![
        crate::db::todo::TodoRecord::new("a", crate::db::todo::TodoStatus::Todo, 0),
        crate::db::todo::TodoRecord::new("b", crate::db::todo::TodoStatus::Todo, 1),
        crate::db::todo::TodoRecord::new("c", crate::db::todo::TodoStatus::Todo, 2),
    ]);
    // 幽灵行插在 1：显示序列 a(0) / 幽灵(1) / b(2) / c(3)。
    crate::app::update::begin_todo_edit(&mut state, crate::app::state::TodoEditTarget::After(0));
    assert!(state.todo.is_editing(), "前提：幽灵行打开");

    let list = crate::ui::right_panel::todo::list_rect(geo.view.prompt);
    let mut sessions = HashMap::new();
    let mut dirty = false;
    handle_terminal_event(
        mouse(MouseEventKind::Moved, list.x + 3, list.y + 2),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );

    assert_eq!(
        state.todo_hover,
        Some(1),
        "显示第 2 行是列表下标 1（b），不是显示下标 2（那会被渲染层当成 c）"
    );
}
