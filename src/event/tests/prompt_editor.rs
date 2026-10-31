use super::test_support::*;
use super::*;

#[test]
fn ctrl_s_saves_prompt_from_editor() {
    let config = Config::default();
    let mut state = demo_terminal();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let mut sessions = HashMap::new();
    let mut dirty = false;
    state.set_focus(crate::app::state::FocusedContainer::Prompt);

    // 默认绑定临时提示词：Ctrl+S 提交缓冲并打开保存选择器。
    state.prompt.insert_str("draft body");
    handle_terminal_event(
        TerminalEvent::Key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL)),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(dirty);
    assert!(
        matches!(
            state.overlay,
            Some(crate::app::overlay::Overlay::SaveDraft(_))
        ),
        "Ctrl+S 在临时提示词上打开保存选择器"
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
        Some("draft body")
    );
}

#[test]
fn ctrl_s_saves_open_item_immediately() {
    use crate::app::prompt::PromptItem;
    let config = Config::default();
    let mut state = demo_terminal();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let mut sessions = HashMap::new();
    let mut dirty = false;
    state.workspaces[0]
        .prompt_panel
        .tree
        .replace(vec![], vec![PromptItem::single(7, None, "item", "body")]);
    assert!(crate::app::update::edit_prompt_item(&mut state, 0, 7));
    state.set_focus(crate::app::state::FocusedContainer::Prompt);
    state.prompt.insert_str("+");
    handle_terminal_event(
        TerminalEvent::Key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL)),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(state.prompt.is_saved(), "Ctrl+S 立即复位保存点");
    assert!(
        state.effects.iter().any(|effect| matches!(
            effect,
            crate::app::effects::Effect::SavePromptContent { id: 7, .. }
        )),
        "Ctrl+S 登记条目落库效应"
    );
}

#[test]
fn ctrl_s_ignores_empty_prompt_content() {
    let config = Config::default();
    let mut state = demo_terminal();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let mut sessions = HashMap::new();
    let mut dirty = false;
    state.set_focus(crate::app::state::FocusedContainer::Prompt);
    let press =
        |state: &mut AppState, sessions: &mut HashMap<PaneId, PtySession>, dirty: &mut bool| {
            handle_terminal_event(
                TerminalEvent::Key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL)),
                state,
                sessions,
                &geo,
                &config,
                &mut ui::RenderCache::default(),
                dirty,
            );
        };

    // 空草稿：不响应（不开保存选择器、无落库效应）。
    press(&mut state, &mut sessions, &mut dirty);
    assert!(state.overlay.is_none(), "空草稿 Ctrl+S 不打开选择器");
    assert!(state.effects.is_empty(), "空草稿无落库效应");

    // 仅空白同样不响应。
    state.prompt.insert_str("   ");
    press(&mut state, &mut sessions, &mut dirty);
    assert!(state.overlay.is_none(), "空白草稿 Ctrl+S 不响应");

    // 非空内容回归：照常提交缓冲并打开选择器。
    state.prompt.insert_str("body");
    press(&mut state, &mut sessions, &mut dirty);
    assert!(
        matches!(
            state.overlay,
            Some(crate::app::overlay::Overlay::SaveDraft(_))
        ),
        "非空草稿照常打开选择器"
    );
}

#[test]
fn mouse_interacts_with_slash_panel() {
    let config = Config::default();
    let mut state = AppState::demo();
    state.prompt.resize(40, 8);
    crate::app::update::focus_prompt(&mut state);
    crate::app::update::apply_editor(
        &mut state,
        crate::app::actions::EditorCommand::InsertChar('/'),
    );
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let area = crate::layout::prompt_text_rect(geo.view.prompt);
    let panel = crate::ui::prompt::slash_layout(&state.prompt, area).expect("slash panel visible");
    let rect = panel.rect;
    let mut sessions = HashMap::new();
    let mut dirty = false;

    // 悬停第 2 项：只改高亮（内容区从 panel.content.y 开始）。
    handle_terminal_event(
        mouse(MouseEventKind::Moved, rect.x + 2, panel.content.y + 1),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert_eq!(
        state.prompt.slash_panel().map(|panel| panel.active()),
        Some(1)
    );

    // 左键点击第 1 项：插入 add 模板并关闭面板。
    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            rect.x + 2,
            panel.content.y,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(state.prompt.text().starts_with("&&& addTemplate --start"));
    assert!(state.prompt.slash_panel().is_none());
}

#[test]
fn mouse_template_buttons_clean_status_and_delete() {
    let (mut state, config, geo, area) = template_button_state();
    let buttons = crate::ui::prompt::template_buttons(&state.prompt, area);
    assert_eq!(buttons.len(), 4, "宽屏下按钮可见");
    let hit = |button| {
        buttons
            .iter()
            .find(|hit| hit.button == button)
            .copied()
            .expect("button")
    };
    let mut sessions = HashMap::new();
    let mut dirty = false;

    // 状态按钮：todo → in_progress。
    let status = hit(crate::ui::prompt::TemplateBlockButton::Status);
    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            status.rect.x + 1,
            status.rect.y,
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
            .prompt
            .text()
            .contains("&&& addTemplate --end in_progress")
    );

    // 清理按钮：移除未填写空项。
    let clean = hit(crate::ui::prompt::TemplateBlockButton::Clean);
    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            clean.rect.x + 1,
            clean.rect.y,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(!state.prompt.text().contains("- Reference: "));
    assert!(
        state
            .prompt
            .text()
            .contains("&&& addTemplate --end in_progress")
    );

    // 删除按钮：整块消失。
    let buttons = crate::ui::prompt::template_buttons(&state.prompt, area);
    let delete = buttons
        .iter()
        .find(|hit| hit.button == crate::ui::prompt::TemplateBlockButton::Delete)
        .copied()
        .expect("delete button");
    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            delete.rect.x + 1,
            delete.rect.y,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert_eq!(state.prompt.text(), "");
}

#[test]
fn mouse_supple_copy_button_writes_clipboard_toast() {
    let config = Config::default();
    let mut state = AppState::demo();
    state.prompt_width = 80;
    state.prompt.resize(78, 16);
    crate::app::update::focus_prompt(&mut state);
    crate::app::update::apply_editor(
        &mut state,
        crate::app::actions::EditorCommand::InsertText(
            "&&& x --start\n+++ supplement --start\n{title: Review}\n\n- keep me\n+++ supplement --end\n&&& x --end"
                .into(),
        ),
    );
    let geo = geometry(&state, &config, Rect::new(0, 0, 220, 30));
    let area = crate::layout::prompt_text_rect(geo.view.prompt);
    let copy = crate::ui::prompt::block_buttons(&state.prompt, area)
        .into_iter()
        .find(|hit| hit.button == crate::ui::prompt::BlockButton::Copy)
        .expect("supple copy button visible");
    let mut sessions = HashMap::new();
    let mut dirty = false;

    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            copy.rect.x,
            copy.rect.y,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(dirty);
    assert!(state.toast.is_some(), "复制按钮弹出剪贴板反馈");
    assert_eq!(
        state.toast.as_ref().map(|toast| toast.title.as_deref()),
        Some(Some(crate::ui::text::TOAST_CLIPBOARD_TITLE)),
        "复制反馈沿用剪贴板 toast"
    );
    assert_eq!(
        state.toast.as_ref().map(|toast| toast.message.as_str()),
        Some("Copied 9 chars"),
        "复制反馈显示复制的字符数（`- keep me` 共 9 字符）"
    );
    assert_eq!(
        state.toast.as_ref().and_then(|toast| toast.anchor),
        Some(state.prompt.id()),
        "补充块复制按钮的 toast 必须锚定在右侧面板容器"
    );
    let toast_rect = ui::toast::rect(&state, &geo.view, &geo.rects, geo.screen, &config)
        .expect("toast rect in right panel");
    assert!(
        toast_rect.left() >= geo.view.prompt.left()
            && toast_rect.right() <= geo.view.prompt.right(),
        "右侧面板复制 toast 矩形 {toast_rect:?} 必须落在右侧面板容器 {:?} 内",
        geo.view.prompt
    );
}

#[test]
fn mouse_supple_fold_button_toggles_without_editing_text() {
    let config = Config::default();
    let mut state = AppState::demo();
    state.prompt_width = 80;
    state.prompt.resize(78, 16);
    crate::app::update::focus_prompt(&mut state);
    crate::app::update::apply_editor(
        &mut state,
        crate::app::actions::EditorCommand::InsertText(
            "&&& x --start\n+++ supplement --start\n{title: Review}\n\n- keep me\n+++ supplement --end\n&&& x --end"
                .into(),
        ),
    );
    state.prompt.mark_saved();
    let geo = geometry(&state, &config, Rect::new(0, 0, 220, 30));
    let area = crate::layout::prompt_text_rect(geo.view.prompt);
    let text = state.prompt.text().to_string();
    let mut sessions = HashMap::new();
    let mut dirty = false;

    for folded in [false, true] {
        let fold = crate::ui::prompt::block_buttons(&state.prompt, area)
            .into_iter()
            .find(|hit| hit.button == crate::ui::prompt::BlockButton::Fold)
            .expect("supple fold button visible");
        assert_eq!(fold.folded, !folded, "按钮字形跟随折叠态");
        handle_terminal_event(
            mouse(
                MouseEventKind::Down(MouseButton::Left),
                fold.rect.x,
                fold.rect.y,
            ),
            &mut state,
            &mut sessions,
            &geo,
            &config,
            &mut ui::RenderCache::default(),
            &mut dirty,
        );
        assert_eq!(state.prompt.supple_folded_at(1), folded);
        assert_eq!(state.prompt.text(), text, "折叠不修改正文");
        assert!(state.prompt.is_saved(), "折叠不置脏");
    }
}

#[test]
fn mouse_click_prompt_page_arrows_switch_and_counter_opens_menu() {
    let config = Config::default();
    let mut state = paged_prompt_state();
    let geo = geometry(&state, &config, Rect::new(0, 0, 140, 30));
    let mut sessions = HashMap::new();
    let mut dirty = false;
    let title = ui::prompt_frame::title_row(geo.view.prompt).expect("title row");
    assert_eq!(
        title.y,
        crate::layout::pane_inner_rect(geo.view.prompt).y + 1,
        "页控件已下移到标题行（工具栏下一行），不再是容器顶边框"
    );
    // 第一页左箭头置灰：点击不切页、不移动正文光标。
    state.prompt.set_cursor_offset_for_test(0);
    let controls = ui::prompt::page_controls(title, "1/2").expect("controls");
    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            controls.prev.x,
            controls.prev.y,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert_eq!(state.prompt.text(), "alpha", "边界箭头不动作");
    assert_eq!(state.prompt.cursor(), 0, "控件点击不移动正文光标");
    // 右箭头：切到第二页。
    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            controls.next.x,
            controls.next.y,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert_eq!(state.prompt.text(), "beta");
    assert!(state.prompt_focused());
    // 计数：打开页码菜单。
    let controls = ui::prompt::page_controls(title, "2/2").expect("controls");
    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            controls.counter.x,
            controls.counter.y,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(matches!(state.overlay, Some(Overlay::PageMenu(_))));
    // Esc 关闭菜单且不穿透编辑器。
    handle_terminal_event(
        TerminalEvent::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(state.overlay.is_none());
    assert_eq!(state.prompt.text(), "beta");
}

#[test]
fn mouse_click_prompt_page_name_starts_rename_and_keys_commit() {
    let config = Config::default();
    let mut state = paged_prompt_state();
    let geo = geometry(&state, &config, Rect::new(0, 0, 140, 30));
    let mut sessions = HashMap::new();
    let mut dirty = false;
    let title = ui::prompt_frame::title_row(geo.view.prompt).expect("title row");
    assert_eq!(
        title.y,
        crate::layout::pane_inner_rect(geo.view.prompt).y + 1,
        "页名已下移到标题行（工具栏下一行），不再是容器顶边框"
    );
    // 标题行 `├─ paged · Page 1 `：点击页名起始后两列。
    let name_col = title.x + 2 + " paged ".len() as u16 + 2;
    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Left), name_col, title.y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(
        crate::app::update::page_rename_active(&state),
        "点击页名进入重命名"
    );
    assert_eq!(state.prompt.text(), "alpha", "重命名不改正文");
    handle_terminal_event(
        TerminalEvent::Key(KeyEvent::new(KeyCode::Char('X'), KeyModifiers::NONE)),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert_eq!(
        state.workspaces[0]
            .prompt_panel
            .page_rename()
            .expect("rename active")
            .input
            .text(),
        "Page 1X",
        "字符输入进入页名"
    );
    handle_terminal_event(
        TerminalEvent::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(!crate::app::update::page_rename_active(&state));
    let item = state.workspaces[0].prompt_panel.tree.item(7).expect("item");
    assert_eq!(item.pages[0].name, "Page 1X");
}

#[test]
fn mouse_click_prompt_padding_only_focuses_without_moving_cursor() {
    let config = Config::default();
    let mut state = paged_prompt_state();
    state.prompt.set_text("short");
    state.prompt.set_cursor_offset_for_test(0);
    state.set_focus(crate::app::state::FocusedContainer::Panes);
    let geo = geometry(&state, &config, Rect::new(0, 0, 140, 30));
    let mut sessions = HashMap::new();
    let mut dirty = false;
    let text_area = crate::layout::prompt_text_rect(geo.view.prompt);
    assert!(text_area.height >= 2, "测试视口需能容纳留白");
    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            text_area.x + 2,
            text_area.bottom() - 1,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(state.prompt_focused(), "点击留白聚焦编辑器");
    assert_eq!(
        state.prompt.cursor(),
        0,
        "点击留白不移动光标到不存在的文本行"
    );
}

#[test]
fn mouse_click_current_tab_commits_page_rename() {
    // 设计：tab 与工具栏按钮同属控件，点一下即离开重命名输入态。
    // 点**当前** tab 时 `select_right_tab` 会因同 tab 早退，提交必须由 press 自己完成。
    let config = Config::default();
    let mut state = paged_prompt_state();
    let geo = geometry(&state, &config, Rect::new(0, 0, 140, 30));
    assert!(crate::app::update::begin_page_rename(&mut state));
    crate::app::update::apply_page_rename_key(&mut state, crate::app::actions::OverlayKey::Clear);
    for ch in "Intro".chars() {
        crate::app::update::apply_page_rename_key(
            &mut state,
            crate::app::actions::OverlayKey::Char(ch),
        );
    }
    assert!(
        crate::app::update::page_rename_active(&state),
        "前提：重命名中"
    );
    let mut sessions = HashMap::new();
    let mut dirty = false;

    let prompt_tab = right_tab_rect(&geo, RightTab::Prompt);
    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            prompt_tab.x,
            prompt_tab.y,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );

    assert_eq!(state.right_tab, RightTab::Prompt, "点的就是当前 tab");
    assert!(
        !crate::app::update::page_rename_active(&state),
        "提交并退出输入态"
    );
    let item = state.workspaces[0].prompt_panel.tree.item(7).expect("item");
    assert_eq!(item.pages[0].name, "Intro", "非空页名写回");
    assert!(dirty);
}

#[test]
fn mouse_click_current_tab_focuses_panel_and_dirties() {
    // 点 tab 就是「点了右侧容器」：即使 tab 没变、也没有重命名，
    // 焦点从主区挪到右栏也是一次真实变化，必须置脏（否则边框高亮不刷新）。
    let mut state = demo_terminal();
    let (config, geo) = toolbar_geometry(&mut state);
    assert!(state.panes_focused(), "前提：主区持焦点");
    let mut sessions = HashMap::new();
    let mut dirty = false;
    let prompt_tab = right_tab_rect(&geo, RightTab::Prompt);

    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            prompt_tab.x,
            prompt_tab.y,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );

    assert_eq!(state.right_tab, RightTab::Prompt);
    assert!(state.prompt_focused(), "点当前 tab 也要让右栏进入选中态");
    assert!(dirty, "焦点变化必须置脏");

    // 已经聚焦右栏后再点一次：这次真的没有任何变化，不该白白重绘。
    dirty = false;
    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            prompt_tab.x,
            prompt_tab.y,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(!dirty, "同 tab 且已聚焦时不该白白重绘");
}

#[test]
fn mouse_click_prompt_collapse_button_works_on_todo_tab() {
    // 折叠按钮在容器底边框、位于"Todo 整块吞掉"分支之前；这条用例锁住顺序：
    // 一旦把吞掉分支提到折叠按钮之前，Todo 下面板就再也折不起来。
    let config = Config::default();
    let mut state = demo_terminal();
    assert!(crate::app::update::select_right_tab(
        &mut state,
        RightTab::Todo
    ));
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let target = ui::controls::collapse_buttons(&geo.view, state.agents_collapsed)
        .into_iter()
        .find(|button| button.target == ui::CollapseTarget::Prompt)
        .expect("prompt collapse button visible");
    let mut sessions = HashMap::new();
    let mut dirty = false;

    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            target.area.x,
            target.area.y,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );

    assert!(
        state.prompt_collapsed,
        "Todo 下点折叠按钮仍要能折叠容器（顺序不能被吞掉分支遮蔽）"
    );
    assert_eq!(state.right_tab, RightTab::Todo, "折叠不改当前面板");
    assert!(dirty);
}

#[test]
fn lx_slash_wheel_prioritizes_over_history_and_clamped_history_never_leaks_to_input() {
    let config = Config::default();
    let mut state = AppState::demo();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 22));
    let pane = state.active_tab().layout.focus();
    let inner = pane_inner(&geo, pane);
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.view = PaneView::Lx;
        // 填充超一屏历史消息（位于顶部 history_scroll = 0）。
        for index in 0..20 {
            target
                .lx
                .chat
                .messages
                .push(crate::app::state::ChatMessageView {
                    sequence: index,
                    id: format!("m-{index}"),
                    role: "user".into(),
                    status: "complete".into(),
                    turn_index: index,
                    kind: "chat".into(),
                    shell: None,
                    steps: vec![crate::app::state::AssistantStepView::Part {
                        position: 0,
                        part: crate::app::state::ChatPartView {
                            kind: crate::chat::events::PartKind::Text,
                            content: format!("history-line-{index:02}"),
                            provider_metadata_json: None,
                            started_at_ms: None,
                            ended_at_ms: None,
                            completed_at_ms: None,
                        },
                    }],
                    model_label: None,
                    error_code: None,
                    input_tokens: None,
                    output_tokens: None,
                    cache_read_tokens: None,
                    duration_ms: None,
                    reports: Vec::new(),
                    attachments: Vec::new(),
                });
        }
        target.lx.chat.follow_bottom = false;
        target.lx.chat.history_scroll = 0;
        // 打开 `/` 命令面板（贴在输入框上沿，落在 history_wheel_region 内）。
        target.lx.slash = Some(crate::app::state::SlashPanel::Commands {
            items: crate::app::state::SlashPanel::command_entries(""),
            window: Default::default(),
            query: String::new(),
        });
    }
    let mut cache = ui::RenderCache::default();
    let mut sessions = HashMap::new();
    let slash_bar = {
        let target = state.pane_anywhere(pane).expect("pane");
        ui::main_content::lx::slash_layout(inner, &target.lx).expect("slash panel")
    };

    // 1. P0-1 回归：存在聊天历史时在 `/` 面板上滚轮，必须滚动 `/` 面板（3 项），不得滚动底层历史。
    let mut dirty = false;
    handle_terminal_event(
        mouse(
            MouseEventKind::ScrollDown,
            slash_bar.content.x + 1,
            slash_bar.content.y,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut cache,
        &mut dirty,
    );
    assert!(dirty);
    let target = state.pane_anywhere(pane).expect("pane");
    assert_eq!(
        target.lx.slash.as_ref().expect("slash").window().viewport,
        Some(crate::app::update::WHEEL_LINES as usize),
        "`/` 面板优先于聊天历史响应滚轮"
    );
    assert_eq!(
        target.lx.chat.history_scroll, 0,
        "`/` 面板内滚轮不得穿透到底层聊天历史"
    );

    // 2. P0-2 回归：关闭 `/` 面板、给输入框填入多行可滚文本，在历史区触顶（0）继续向上滚轮时，
    //    不得穿透到底部输入框滚动。
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.lx.slash = None;
        target.lx.input.clear();
        target.lx.cursor = 0;
        target.lx.resize(inner.width.saturating_sub(6), 3);
        target
            .lx
            .insert_str("l1\nl2\nl3\nl4\nl5\nl6\nl7\nl8\nl9\nl10");
    }
    let input_scroll_before = state.pane_anywhere(pane).expect("pane").lx.scroll();
    assert!(input_scroll_before > 0, "夹具：输入框贴底且可向上滚");
    let history_region = {
        let target = state.pane_anywhere(pane).expect("pane");
        ui::main_content::lx::history_wheel_region(inner, &target.lx).expect("history region")
    };
    dirty = false;
    handle_terminal_event(
        mouse(
            MouseEventKind::ScrollUp,
            history_region.x + 2,
            history_region.y + 1,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut cache,
        &mut dirty,
    );
    assert!(!dirty, "历史区已在顶部时上滚不置脏");
    assert_eq!(
        state.pane_anywhere(pane).expect("pane").lx.scroll(),
        input_scroll_before,
        "历史区触顶滚轮不得穿透滚动底部输入框"
    );
}

#[test]
fn prompt_refer_panel_wheel_scrolls_by_wheel_lines_without_scrolling_editor() {
    let config = Config::default();
    let mut state = AppState::demo();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let area = crate::layout::prompt_text_rect(geo.view.prompt);
    state.prompt.resize(area.width, area.height);
    // 构造 6 个不同提及加上 2 个快捷输入字符 `re`，触发 prompt 的 ReferPanel。
    state
        .prompt
        .insert_str("@ref_a.rs @ref_b.rs @ref_c.rs @ref_d.rs @ref_e.rs @ref_f.rs\nre");
    let panel = crate::ui::prompt::refer_layout(&state.prompt, area).expect("refer panel visible");
    let scroll_before = state.prompt.scroll();
    let mut sessions = HashMap::new();
    let mut dirty = false;

    // 1. P2-1 回归：已在顶部（start == 0）向上滚轮不置脏。
    handle_terminal_event(
        mouse(MouseEventKind::ScrollUp, panel.rect.x + 2, panel.rect.y + 1),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(!dirty, "面板已在顶部时向上滚轮不产生虚假重绘");

    // 2. P1-1 回归：在快捷引用面板上向下滚轮按 WHEEL_LINES（3 项）滚动面板视口，不穿透滚动编辑器。
    handle_terminal_event(
        mouse(
            MouseEventKind::ScrollDown,
            panel.rect.x + 2,
            panel.rect.y + 1,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(dirty);
    assert_eq!(
        state.prompt.refer_panel().and_then(|p| p.viewport()),
        Some(crate::app::update::WHEEL_LINES as usize),
        "prompt 快捷引用面板滚轮按 3 项移动视口"
    );
    assert_eq!(
        state.prompt.scroll(),
        scroll_before,
        "快捷引用面板内滚轮不滚动底层 prompt"
    );
}

#[test]
fn clicking_prompt_command_panel_chrome_does_not_penetrate_to_editor_cursor() {
    let config = Config::default();
    let mut state = demo_terminal();
    update::focus_prompt(&mut state);
    state.prompt.insert_str("line 0\nline 1\nline 2\n@");
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let text_area = crate::event::paste::prompt_text_area(&geo.view);
    let panel = ui::prompt::mention_layout(&state.prompt, text_area).expect("mention panel open");
    let cursor_before = state.prompt.cursor();
    let mut sessions = HashMap::new();
    let mut dirty = false;

    // 点击面板标题 @Command 所在行或底部快捷键提示行：面板保持打开、光标不跳到底层文本行。
    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            panel.rect.x + 2,
            panel.rect.y + 1,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(
        state.prompt.mention_active(),
        "点击面板标题区不得关闭提及面板"
    );
    assert_eq!(
        state.prompt.cursor(),
        cursor_before,
        "点击面板非条目区不得穿透移动底层编辑器光标"
    );
}
