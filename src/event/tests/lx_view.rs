use super::test_support::*;
use super::*;

#[test]
fn lx_view_edits_input_and_pastes_text() {
    let config = Config::default();
    let mut state = AppState::demo();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let pane = state.active_tab().layout.focus();
    scrolled_pane(&mut state, pane);
    let mut sessions = HashMap::new();
    let mut dirty = false;

    handle_terminal_event(
        TerminalEvent::Key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE)),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(dirty, "lx 输入产生重绘");
    assert_eq!(
        state.pane_anywhere(pane).expect("pane").lx.input,
        "a",
        "按键进入 lx 输入框"
    );
    assert!(
        state
            .pane_anywhere(pane)
            .expect("pane")
            .terminal
            .display_offset()
            > 0,
        "按键不得吸回隐藏终端的滚动位置"
    );

    // 普通文本粘贴弹出 lx 文本粘贴面板，回车确认首项（Paste raw）进入 lx 输入框，不写入隐藏终端。
    dirty = false;
    handle_terminal_event(
        TerminalEvent::Paste("bc".to_string()),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(dirty, "lx 视图粘贴产生重绘");
    assert!(
        state
            .pane_anywhere(pane)
            .expect("pane")
            .lx
            .text_paste_panel()
            .is_some()
    );
    update::apply_lx_input(&mut state, crate::app::actions::LxInput::Enter);
    assert_eq!(
        state.pane_anywhere(pane).expect("pane").lx.input,
        "abc",
        "粘贴进入 lx 输入框"
    );
    assert!(
        state
            .pane_anywhere(pane)
            .expect("pane")
            .terminal
            .display_offset()
            > 0,
        "粘贴不得写入隐藏终端"
    );

    // Ctrl+Q 仍优先退出。
    handle_terminal_event(
        TerminalEvent::Key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL)),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(state.should_quit);
}

#[test]
fn lx_view_path_paste_opens_reference_panel() {
    let config = Config::default();
    let mut state = AppState::demo();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let pane = state.active_tab().layout.focus();
    let dir = env!("CARGO_MANIFEST_DIR").to_string();
    let mut sessions = HashMap::new();
    let mut dirty = false;

    handle_terminal_event(
        TerminalEvent::Paste(dir.clone()),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(dirty, "路径粘贴产生重绘");
    let lx = &state.pane_anywhere(pane).expect("pane").lx;
    assert!(lx.paste_panel().is_some(), "路径粘贴打开参照面板");
    assert!(lx.input.contains(&dir), "占位路径写入 lx 输入框");

    // Enter 采纳默认高亮项（引用），面板关闭且占位被改写。
    handle_terminal_event(
        TerminalEvent::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    let lx = &state.pane_anywhere(pane).expect("pane").lx;
    assert!(lx.paste_panel().is_none(), "确认后关闭面板");
    assert!(lx.input.contains("@[refer-folder]"), "改写为引用格式");
}

#[test]
fn lx_view_content_click_focuses_and_starts_selection() {
    let config = Config::default();
    let mut state = AppState::demo();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let pane = state.active_tab().layout.focus();
    let inner = pane_inner(&geo, pane);
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target
            .lx
            .chat
            .messages
            .push(crate::app::state::ChatMessageView {
                sequence: 0,
                id: "m0".into(),
                role: "user".into(),
                status: "complete".into(),
                turn_index: 0,
                kind: "chat".into(),
                shell: None,
                steps: vec![crate::app::state::AssistantStepView::Part {
                    position: 0,
                    part: crate::app::state::ChatPartView {
                        kind: crate::chat::events::PartKind::Text,
                        content: "hello history".into(),
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
    crate::app::update::focus_prompt(&mut state);
    let mut sessions = HashMap::new();
    let mut dirty = false;

    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            inner.x + 2,
            inner.y,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(!state.prompt_focused());
    assert_eq!(state.active_tab().layout.focus(), pane);
    assert!(state.terminal_selection.is_none(), "lx 视图不启动终端选区");
    assert!(state.selection.is_none(), "lx 视图不启动 prompt 选区");
    assert!(
        state.lx_selection.is_some(),
        "lx 视图主消息列表按下开始内容选区"
    );
}

#[test]
fn lx_input_click_places_cursor_and_starts_text_selection() {
    let config = Config::default();
    let mut state = AppState::demo();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let pane = state.active_tab().layout.focus();
    {
        let lx = &mut state.pane_mut_anywhere(pane).expect("pane").lx;
        for ch in "hello".chars() {
            lx.insert_char(ch);
        }
    }
    let inner = pane_inner(&geo, pane);
    let (x, y) = state
        .pane_anywhere(pane)
        .and_then(|target| ui::main_content::lx::input_geom(inner, &target.lx))
        .map(|geom| geom.text_origin())
        .expect("input geom");
    let mut sessions = HashMap::new();
    let mut dirty = false;

    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Left), x + 2, y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    let lx = &state.pane_anywhere(pane).expect("pane").lx;
    assert_eq!(lx.cursor, 2, "点击把光标定位到单元格");
    assert!(lx.has_text_selection(), "输入框内开始文本拖拽");
    assert!(state.lx_input_drag.is_some());
    assert!(state.lx_selection.is_none(), "输入框内不再启动整页选区");
}

#[test]
fn lx_view_ignores_wheel_and_mouse_report() {
    let config = Config::default();
    let mut state = AppState::demo();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let pane = state.active_tab().layout.focus();
    {
        let target = state.pane_mut_anywhere(pane).expect("terminal pane");
        target
            .terminal
            .feed(b"\x1b[?1000h\x1b[?1002h\x1b[?1003h\x1b[?1006h");
    }
    scrolled_pane(&mut state, pane);
    let inner = pane_inner(&geo, pane);
    let mut sessions = HashMap::new();
    let mut dirty = false;

    for kind in [
        MouseEventKind::ScrollUp,
        MouseEventKind::Down(MouseButton::Left),
        MouseEventKind::Drag(MouseButton::Left),
        MouseEventKind::Up(MouseButton::Left),
    ] {
        handle_terminal_event(
            mouse(kind, inner.x, inner.y),
            &mut state,
            &mut sessions,
            &geo,
            &config,
            &mut ui::RenderCache::default(),
            &mut dirty,
        );
    }
    let offset = state
        .pane_anywhere(pane)
        .expect("pane")
        .terminal
        .display_offset();
    assert!(offset > 0, "隐藏终端不接收滚轮/鼠标上报，视口保持不动");
    assert!(state.terminal_selection.is_none());
}

#[test]
fn lx_entry_click_switches_selection() {
    use crate::app::state::LxEntry;
    let config = Config::default();
    let mut state = AppState::demo();
    let geo = geometry(&state, &config, Rect::new(0, 0, 160, 40));
    let pane = state.active_tab().layout.focus();
    let area = pane_inner(&geo, pane);
    let lx = &state.pane_anywhere(pane).expect("pane").lx;
    let mut target = None;
    for row in area.y..area.bottom() {
        for col in area.x..area.right() {
            if crate::ui::main_content::lx::entry_hit(area, lx, col, row)
                == Some(LxEntry::Automation)
            {
                target = Some((col, row));
            }
        }
    }
    let (col, row) = target.expect("assistant entry button");
    let mut sessions = HashMap::new();
    let mut dirty = false;
    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Left), col, row),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert_eq!(
        state.pane_anywhere(pane).expect("pane").lx.entry,
        LxEntry::Automation,
        "点击入口切换选中"
    );
    assert!(state.lx_selection.is_none(), "入口点击不启动选区");
}

#[test]
fn lx_mention_panel_click_accepts_candidate_before_page_selection() {
    let config = Config::default();
    let mut state = AppState::demo();
    seed_lx_mention(
        &mut state,
        "@ma",
        vec![crate::app::markdown::MentionEntry {
            path: "main.rs".into(),
            is_directory: false,
        }],
    );
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let mut sessions = HashMap::new();
    let mut dirty = false;
    let pane = state.active_tab().layout.focus();
    let (_, rect) = geo
        .rects
        .iter()
        .find(|(id, _)| *id == pane)
        .copied()
        .expect("pane rect");
    let inner = crate::layout::pane_inner_rect(rect);
    let lx = &state.pane_anywhere(pane).expect("pane").lx;
    let panel = ui::main_content::lx::mention_layout(inner, lx).expect("mention panel");
    let (x, y) = (panel.content.x, panel.content.y);
    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Left), x, y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert_eq!(
        state.pane_anywhere(pane).expect("pane").lx.input,
        "@main.rs ",
        "点击候选按 Enter 规则采纳"
    );
    assert!(state.lx_selection.is_none(), "命中提及面板不启动整页选区");
}

#[test]
fn lx_mention_wheel_scrolls_panel_not_input() {
    let config = Config::default();
    let mut state = AppState::demo();
    let entries: Vec<crate::app::markdown::MentionEntry> = (0..10)
        .map(|i| crate::app::markdown::MentionEntry {
            path: format!("f{i}.rs"),
            is_directory: false,
        })
        .collect();
    seed_lx_mention(&mut state, "@", entries);
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let mut sessions = HashMap::new();
    let mut dirty = false;
    let pane = state.active_tab().layout.focus();
    let (_, rect) = geo
        .rects
        .iter()
        .find(|(id, _)| *id == pane)
        .copied()
        .expect("pane rect");
    let inner = crate::layout::pane_inner_rect(rect);
    let input_scroll_before = state.pane_anywhere(pane).expect("pane").lx.scroll();
    let lx = &state.pane_anywhere(pane).expect("pane").lx;
    let panel = ui::main_content::lx::mention_layout(inner, lx).expect("mention panel");
    handle_terminal_event(
        mouse(MouseEventKind::ScrollDown, panel.content.x, panel.content.y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    let target = state.pane_anywhere(pane).expect("pane").lx.mention();
    assert_eq!(
        target.panel().expect("panel").viewport(),
        Some(3),
        "面板内滚轮滚动候选视口（每格 3 项）"
    );
    assert_eq!(
        state.pane_anywhere(pane).expect("pane").lx.scroll(),
        input_scroll_before,
        "候选面板内滚轮不滚动输入框"
    );
}

/// 回归：lx 卡片「按下 → 松开（未拖拽）」必须真的切换折叠。
///
/// 真实 bug：`begin_lx_selection` 内部先 `clear_selection`（会把 `lx_card_press` 清空），
/// 早期代码先记 press 再 begin，等于记了个寂寞——每次松开都被当成拖拽，卡片永远展不开。
#[test]
fn lx_card_click_toggles_expansion_on_release() {
    let config = Config::default();
    let mut state = AppState::demo();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let pane = state.active_tab().layout.focus();
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.view = PaneView::Lx;
        let mut chat = crate::app::state::LxChat::new();
        chat.messages.push(crate::app::state::ChatMessageView {
            sequence: 0,
            id: "m-param".into(),
            role: "assistant".into(),
            status: "complete".into(),
            turn_index: 0,
            kind: "chat".into(),
            shell: None,
            steps: vec![crate::app::state::AssistantStepView::Tool {
                position: 0,
                call: crate::app::state::ToolCallView {
                    call_id: "call-param".into(),
                    name: "write".into(),
                    position: 0,
                    arguments_json: String::new(),
                    status: crate::app::state::ToolStatus::Completed,
                    summary: Some(Box::new(crate::app::state::ToolSummary {
                        // 足够长：折叠 2 行装不下，展开才多行。
                        path: Some(
                            "src/some/really/deeply/nested/path/that/keeps/going/mod.rs".repeat(3),
                        ),
                        added: Some(12),
                        removed: Some(4),
                        ..Default::default()
                    })),
                    error: None,
                    result_json: None,
                    detail: None,
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
        chat.follow_bottom = true;
        target.lx.chat = chat;
    }
    let inner = pane_inner(&geo, pane);
    let card = {
        let target = state.pane_anywhere(pane).expect("pane");
        let mut cache = crate::ui::RenderCache::default();
        crate::ui::main_content::lx::history_view(inner, &target.lx, cache.pane(pane))
            .and_then(|view| {
                crate::ui::main_content::lx::history::layout(
                    view.body,
                    &target.lx,
                    cache.pane(pane),
                )
                .cards
                .into_iter()
                .next()
            })
            .expect("可展开卡片")
    };
    let (column, row) = (inner.x + 3, card.card.y);
    assert!(
        !state
            .pane_anywhere(pane)
            .expect("pane")
            .lx
            .chat
            .detail_is_expanded("m-param", "call-param"),
        "初始折叠"
    );
    let mut sessions = HashMap::new();
    let mut dirty = false;
    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Left), column, row),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(
        state.lx_card_press.is_some(),
        "按下必须记录「可能是一次点击」（不能被 begin_lx_selection 清掉）"
    );
    handle_terminal_event(
        mouse(MouseEventKind::Up(MouseButton::Left), column, row),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(
        state
            .pane_anywhere(pane)
            .expect("pane")
            .lx
            .chat
            .detail_is_expanded("m-param", "call-param"),
        "未拖拽的点击必须展开卡片"
    );
}

#[test]
fn lx_history_selection_waits_for_copy_shortcut_and_follows_wheel_scroll() {
    let config = Config::default();
    let mut state = AppState::demo();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 18));
    let pane = state.active_tab().layout.focus();
    let inner = pane_inner(&geo, pane);
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.view = PaneView::Lx;
        target.lx.insert_str("keep my draft");
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
    }
    let mut cache = ui::RenderCache::default();
    let geom = {
        let target = state.pane_anywhere(pane).expect("pane");
        ui::main_content::lx::content_geom(inner, &target.lx, cache.pane(pane)).expect("geom")
    };
    let mut sessions = HashMap::new();
    let mut dirty = false;

    // 1. 按下首行（内容第 0 行），按住左键向下滚轮 2 次，再拖拽到可视区第 2 行后松开。
    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            geom.rect.x,
            geom.rect.y,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut cache,
        &mut dirty,
    );
    handle_terminal_event(
        mouse(MouseEventKind::ScrollDown, geom.rect.x + 4, geom.rect.y + 2),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut cache,
        &mut dirty,
    );
    handle_terminal_event(
        mouse(
            MouseEventKind::ScrollDown,
            geom.rect.x + 15,
            geom.rect.y + 2,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut cache,
        &mut dirty,
    );
    handle_terminal_event(
        mouse(
            MouseEventKind::Up(MouseButton::Left),
            geom.rect.x + 15,
            geom.rect.y + 2,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut cache,
        &mut dirty,
    );

    let selection = state.lx_selection.expect("松开鼠标后保留选区");
    assert!(!selection.is_dragging(), "松开后结束拖拽态");
    assert_eq!(
        selection.range(),
        Some(((0, 0), (8, 15))),
        "锚点钉在内容第 0 行，终点随两次滚轮（每次 3 行）延伸到内容第 8 行"
    );
    assert!(
        state.pane_anywhere(pane).expect("pane").lx.toast.is_none(),
        "松开鼠标不直接触发复制 toast"
    );

    // 2. 松开鼠标后继续滚轮滚动：选区不被清除，仍钉在原内容行范围上。
    handle_terminal_event(
        mouse(MouseEventKind::ScrollDown, geom.rect.x + 4, geom.rect.y + 2),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut cache,
        &mut dirty,
    );
    assert_eq!(
        state.lx_selection.and_then(|sel| sel.range()),
        Some(((0, 0), (8, 15))),
        "松开后滚轮滚动不丢失选区"
    );

    // 3. 按下 Ctrl+C 复制：弹出全局 toast 并显示复制字符数、保留选区高亮、不清空底部输入框草稿。
    handle_terminal_event(
        TerminalEvent::Key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut cache,
        &mut dirty,
    );
    let lx = &state.pane_anywhere(pane).expect("pane").lx;
    assert_eq!(
        lx.input, "keep my draft",
        "有选区时按 Ctrl+C 不清空输入框草稿"
    );
    assert!(
        state.toast.is_none(),
        "lx 视图复制反馈落入窗格专属槽，不占全局 state.toast"
    );
    let toast = lx
        .toast
        .as_ref()
        .expect("按 Ctrl+C 后在该 lx 窗格内弹出复制 toast");
    assert_eq!(toast.anchor, Some(pane), "lx 复制 toast 锚定到当前 lx 窗格");
    assert_eq!(
        toast.title.as_deref(),
        Some(crate::ui::text::TOAST_CLIPBOARD_TITLE)
    );
    assert!(
        toast.message.starts_with("Copied ") && toast.message.ends_with(" chars"),
        "显示复制字符数：{}",
        toast.message
    );
    assert!(state.lx_selection.is_some(), "复制后选区高亮继续保留");
}

#[test]
fn lx_mention_left_right_keys_and_mouse_switch_tabs_and_insert_ref_folder() {
    let config = Config::default();
    let mut state = AppState::demo();
    seed_lx_mention(
        &mut state,
        "@",
        vec![crate::app::markdown::MentionEntry {
            path: "main.rs".into(),
            is_directory: false,
        }],
    );
    state.referenced.folders = vec![crate::app::referenced::ReferencedFolder {
        id: 1,
        path: std::path::PathBuf::from("/Users/yonah/projects/codex-main"),
    }];
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let pane = state.active_tab().layout.focus();
    let mut sessions = HashMap::new();
    let mut dirty = false;

    // 按 Right 方向键切到 refDir Tab（不移动输入框光标）。
    handle_terminal_event(
        TerminalEvent::Key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE)),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(dirty);
    assert_eq!(
        state.pane_anywhere(pane).expect("pane").lx.mention_tab(),
        crate::app::prompt::MentionTab::RefDir
    );

    // 鼠标点击 Files Tab 切回 Files。
    let (_, rect) = geo
        .rects
        .iter()
        .find(|(id, _)| *id == pane)
        .copied()
        .expect("pane rect");
    let inner = crate::layout::pane_inner_rect(rect);
    let panel =
        ui::main_content::lx::mention_layout(inner, &state.pane_anywhere(pane).expect("pane").lx)
            .expect("mention panel");
    dirty = false;
    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            panel.tabs[0].x,
            panel.tabs[0].y,
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
        state.pane_anywhere(pane).expect("pane").lx.mention_tab(),
        crate::app::prompt::MentionTab::Files
    );

    // 鼠标点击 refDir Tab 切到 refDir，再按 Enter 确认插入引用文件夹。
    let panel =
        ui::main_content::lx::mention_layout(inner, &state.pane_anywhere(pane).expect("pane").lx)
            .expect("mention panel in Files tab");
    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            panel.tabs[1].x,
            panel.tabs[1].y,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert_eq!(
        state.pane_anywhere(pane).expect("pane").lx.mention_tab(),
        crate::app::prompt::MentionTab::RefDir
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
    assert_eq!(
        state.pane_anywhere(pane).expect("pane").lx.input,
        "@[refer-folder](/Users/yonah/projects/codex-main) "
    );
}

#[test]
fn lx_command_panel_non_item_clicks_do_not_penetrate_to_underlying_entries() {
    let config = Config::default();
    let mut state = AppState::demo();
    // 空候选（显示 no matches 状态行，非交互行）：面板位于欢迎页入口区上方或重叠处。
    seed_lx_mention(&mut state, "@nomatch", Vec::new());
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let pane = state.active_tab().layout.focus();
    let (_, rect) = geo
        .rects
        .iter()
        .find(|(id, _)| *id == pane)
        .copied()
        .expect("pane rect");
    let inner = crate::layout::pane_inner_rect(rect);
    let panel =
        ui::main_content::lx::mention_layout(inner, &state.pane_anywhere(pane).expect("pane").lx)
            .expect("empty mention panel");
    let mut sessions = HashMap::new();
    let mut dirty = false;

    // 点击面板顶栏横线右端空白或空态占位行：由浮层吞掉，不开启整页文本选区。
    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            panel.rect.right() - 2,
            panel.rect.y,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(state.lx_selection.is_none(), "点击面板顶栏不启动底层选区");
    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            panel.content.x + 2,
            panel.content.y,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(state.lx_selection.is_none(), "点击面板空态行不启动底层选区");
}

#[test]
fn lx_history_wheel_scrolls_by_wheel_lines_and_clamps() {
    let config = Config::default();
    let mut state = AppState::demo();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 18));
    let pane = state.active_tab().layout.focus();
    let inner = pane_inner(&geo, pane);
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.view = PaneView::Lx;
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
    }
    let mut cache = ui::RenderCache::default();
    let (total, visible) = {
        let target = state.pane_anywhere(pane).expect("pane");
        ui::main_content::lx::history_rows(inner, &target.lx, cache.pane(pane))
            .expect("history rows")
    };
    let max_top = total.saturating_sub(visible);
    assert!(max_top >= 6, "夹具：历史最大滚动偏移需不小于 6 行");
    let region = ui::main_content::lx::history_wheel_region(
        inner,
        &state.pane_anywhere(pane).expect("pane").lx,
    )
    .expect("wheel region");

    let mut sessions = HashMap::new();
    let mut dirty = false;

    // 1. 从顶部（history_scroll = 0）向下滚轮 1 格：精确前进 WHEEL_LINES（3 行）。
    handle_terminal_event(
        mouse(MouseEventKind::ScrollDown, region.x + 2, region.y + 1),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut cache,
        &mut dirty,
    );
    assert!(dirty);
    assert_eq!(
        state
            .pane_anywhere(pane)
            .expect("pane")
            .lx
            .chat
            .history_scroll,
        crate::app::update::WHEEL_LINES as usize,
        "lx 聊天历史单格滚轮向下移动 WHEEL_LINES（3）行"
    );

    // 2. 向上滚轮 1 格回到顶部 0；继续向上滚轮被钳制在 0 且不产生无效重绘。
    dirty = false;
    handle_terminal_event(
        mouse(MouseEventKind::ScrollUp, region.x + 2, region.y + 1),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut cache,
        &mut dirty,
    );
    assert!(dirty);
    assert_eq!(
        state
            .pane_anywhere(pane)
            .expect("pane")
            .lx
            .chat
            .history_scroll,
        0
    );
    dirty = false;
    handle_terminal_event(
        mouse(MouseEventKind::ScrollUp, region.x + 2, region.y + 1),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut cache,
        &mut dirty,
    );
    assert!(!dirty, "触顶后继续上滚不置脏");

    // 3. 距底部不足 3 行（max_top - 2）时向下滚轮 1 格：钳制到 max_top 并恢复 follow_bottom。
    state
        .pane_mut_anywhere(pane)
        .expect("pane")
        .lx
        .chat
        .history_scroll = max_top - 2;
    dirty = false;
    handle_terminal_event(
        mouse(MouseEventKind::ScrollDown, region.x + 2, region.y + 1),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut cache,
        &mut dirty,
    );
    let chat = &state.pane_anywhere(pane).expect("pane").lx.chat;
    assert!(dirty);
    assert!(chat.follow_bottom, "滚回底部恢复跟随");
    assert_eq!(chat.history_scroll, max_top);
}

#[test]
fn lx_expanded_thinking_table_double_click_selects_word_without_collapsing_card() {
    let config = Config::default();
    let mut state = AppState::demo();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let pane = state.active_tab().layout.focus();
    let inner = pane_inner(&geo, pane);
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.view = PaneView::Lx;
        target
            .lx
            .chat
            .messages
            .push(crate::app::state::ChatMessageView {
                sequence: 0,
                id: "m-thinking-table".into(),
                role: "assistant".into(),
                status: "complete".into(),
                turn_index: 0,
                kind: "chat".into(),
                shell: None,
                steps: vec![crate::app::state::AssistantStepView::Part {
                    position: 0,
                    part: crate::app::state::ChatPartView {
                        kind: crate::chat::events::PartKind::Reasoning,
                        content:
                            "| 阶段 | 结论 |\n| --- | --- |\n| 验证 | alpha_token beta_token |"
                                .into(),
                        provider_metadata_json: None,
                        started_at_ms: Some(0),
                        ended_at_ms: Some(500),
                        completed_at_ms: Some(500),
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
        target.lx.chat.toggle_reasoning(0, 0);
        assert!(target.lx.chat.reasoning_expanded(0, 0));
    }

    let mut cache = ui::RenderCache::default();
    let geom = ui::main_content::lx::content_geom(
        inner,
        &state.pane_anywhere(pane).expect("pane").lx,
        cache.pane(pane),
    )
    .expect("content geom");
    // 思考块标题在第 0 行，表格从第 1 行开始，第 4 行为首条数据行（`| 验证 | alpha_token beta_token |`）。
    let click_y = geom.rect.y + 4;
    let click_x = geom.rect.x + 16;
    assert!(ui::main_content::lx::is_table_cell_at(
        inner,
        &state.pane_anywhere(pane).expect("pane").lx,
        (4, 16),
        cache.pane(pane),
    ));

    let mut sessions = HashMap::new();
    let mut dirty = false;
    for _ in 0..2 {
        handle_terminal_event(
            mouse(MouseEventKind::Down(MouseButton::Left), click_x, click_y),
            &mut state,
            &mut sessions,
            &geo,
            &config,
            &mut cache,
            &mut dirty,
        );
        handle_terminal_event(
            mouse(MouseEventKind::Up(MouseButton::Left), click_x, click_y),
            &mut state,
            &mut sessions,
            &geo,
            &config,
            &mut cache,
            &mut dirty,
        );
    }

    assert!(
        state
            .pane_anywhere(pane)
            .expect("pane")
            .lx
            .chat
            .reasoning_expanded(0, 0),
        "在已展开思考块内的表格上双击选词不得把思考块折叠收起"
    );
    let sel = state.lx_selection.as_ref().expect("word selection active");
    let copied = ui::main_content::lx::copy_text(
        inner,
        &state.pane_anywhere(pane).expect("pane").lx,
        (sel.anchor(), sel.cursor()),
        cache.pane(pane),
    );
    assert_eq!(copied, "alpha_token");
}

#[test]
fn lx_code_block_copy_button_click_shows_pane_toast_keeps_selection_and_ignores_empty_block() {
    let config = Config::default();
    let mut state = AppState::demo();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let pane = state.active_tab().layout.focus();
    let inner = pane_inner(&geo, pane);

    if let Some(target) = state.pane_mut_anywhere(pane) {
        target.view = PaneView::Lx;
        target
            .lx
            .chat
            .messages
            .push(crate::app::state::ChatMessageView {
                sequence: 1,
                id: "m-code".into(),
                role: "assistant".into(),
                status: "complete".into(),
                turn_index: 0,
                kind: "chat".into(),
                shell: None,
                steps: vec![crate::app::state::AssistantStepView::Part {
                    position: 0,
                    part: crate::app::state::ChatPartView {
                        kind: crate::chat::events::PartKind::Text,
                        content: "```rust\nlet answer = 42;\n```\n\n```\n```".into(),
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

    let mut cache = ui::RenderCache::default();
    let geom = ui::main_content::lx::content_geom(
        inner,
        &state.pane_anywhere(pane).expect("pane").lx,
        cache.pane(pane),
    )
    .expect("content geom");

    // 预先设置一个选区，验证点击代码块 `[❐]` 不会清除已有选区。
    state.lx_selection = Some(crate::app::selection::Selection::text_range(
        pane,
        (2, 0),
        (2, 10),
    ));

    // 第一个代码块：第 0 行头部 `│rust ... [❐]`，按钮位于右缘。
    let btn_x = geom.rect.right() - 2;
    let first_header_y = geom.rect.y;
    // 第二个（空）代码块：第 1 行是 `let answer = 42;`，第 2 行头部 `│text ... [❐]`。
    let empty_header_y = geom.rect.y + 2;

    let mut sessions = HashMap::new();
    let mut dirty = false;

    // 1. 先点空代码块的 `[❐]`：不得弹出 `Copied 0 chars` toast。
    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            btn_x,
            empty_header_y,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut cache,
        &mut dirty,
    );
    assert!(
        state.pane_anywhere(pane).expect("pane").lx.toast.is_none(),
        "空代码块点击 `[❐]` 不应弹出 toast"
    );

    // 2. 再点非空代码块的 `[❐]`：在当前 lx 窗格内弹出 toast（全局 state.toast 保持为 None），且选区保留。
    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            btn_x,
            first_header_y,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut cache,
        &mut dirty,
    );
    assert!(
        state.toast.is_none(),
        "lx 代码块复制 toast 必须落在窗格内而非全局"
    );
    assert!(
        state.pane_anywhere(pane).expect("pane").lx.toast.is_some(),
        "非空代码块点击 `[❐]` 应在当前 lx 窗格内弹出剪贴板 toast"
    );
    assert!(
        state.lx_selection.is_some(),
        "点击 `[❐]` 复制按钮不得清除已有文本选区"
    );
}
