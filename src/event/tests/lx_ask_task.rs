use super::test_support::*;
use super::*;

#[test]
fn lx_task_output_and_agents_detail_selection_scroll_and_copy() {
    use crate::app::state::{ProcessEntry, TaskPage, TaskTab};

    let config = Config::default();
    let mut state = AppState::demo();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 22));
    let pane = state.active_tab().layout.focus();
    let inner = pane_inner(&geo, pane);
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.view = PaneView::Lx;
        let mut page = TaskPage::default();
        page.set_processes(
            vec![ProcessEntry {
                session_id: 1,
                command: "cargo test".into(),
                cwd: "/w".into(),
                tty: false,
                started_at_ms: 0,
            }],
            0,
        );
        page.output = (0..30).map(|idx| format!("proc-out-{idx:02}")).collect();
        page.output_scroll = 0;
        target.lx.task = Some(page);
    }
    let mut cache = ui::RenderCache::default();
    let geom = {
        let target = state.pane_anywhere(pane).expect("pane");
        ui::main_content::lx::content_geom(inner, &target.lx, cache.pane(pane)).expect("geom")
    };
    let mut sessions = HashMap::new();
    let mut dirty = false;

    // Processes 输出区：按下第 0 行 → 滚轮下滚（每次 3 行）→ 松开保留选区 → Ctrl+C 复制。
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
        mouse(
            MouseEventKind::ScrollDown,
            geom.rect.x + 10,
            geom.rect.y + 1,
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
            geom.rect.x + 10,
            geom.rect.y + 1,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut cache,
        &mut dirty,
    );
    let selection = state.lx_selection.expect("进程输出选区保留");
    assert_eq!(
        selection.range(),
        Some(((0, 0), (4, 10))),
        "滚轮滚动 3 行后，视口第 1 行对应内容第 4 行"
    );
    assert!(
        state.pane_anywhere(pane).expect("pane").lx.toast.is_none(),
        "松开鼠标不自动复制"
    );
    handle_terminal_event(
        TerminalEvent::Key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut cache,
        &mut dirty,
    );
    assert!(
        state.toast.is_none(),
        "lx 后台任务页复制不占全局 state.toast"
    );
    let toast = state
        .pane_anywhere(pane)
        .and_then(|target| target.lx.toast.as_ref())
        .expect("后台管理页按 Ctrl+C 在当前 lx 窗格内弹出复制 toast");
    assert_eq!(toast.anchor, Some(pane), "复制 toast 锚定到当前 lx 窗格");
    assert!(
        toast.message.starts_with("Copied ") && toast.message.ends_with(" chars"),
        "显示复制字符数：{}",
        toast.message
    );

    // 切换到 Agents 页签：旧选区自动清除；在可折叠卡片上拖拽只选字不折叠，单击才折叠。
    assert!(update::select_task_tab(&mut state, pane, 1));
    assert!(state.lx_selection.is_none(), "切换后台 Tab 清除旧选区");
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        if let Some(page) = target.lx.task.as_mut() {
            page.tab = TaskTab::Agents;
        }
        let mut run =
            crate::app::state::SubagentRun::from_record(&crate::db::chat::SubagentRecord {
                agent_id: "sub-1".into(),
                nickname: None,
                agent_path: "/root/sub_1".into(),
                child_session_id: "sess-1".into(),
                parent_session_id: "parent".into(),
                spawn_message_id: None,
                role: crate::chat::agent::AgentRole::Explorer,
                model_provider: String::new(),
                model_id: String::new(),
                model_variant: None,
                status: "completed".into(),
                title: "explore".into(),
                created_at_ms: 0,
                completed_at_ms: Some(100),
                report_json: None,
                report_delivered: false,
            });
        run.loaded = true;
        run.chat.messages.push(crate::app::state::ChatMessageView {
            sequence: 0,
            id: "am-0".into(),
            role: "assistant".into(),
            status: "complete".into(),
            turn_index: 0,
            kind: "chat".into(),
            shell: None,
            steps: vec![crate::app::state::AssistantStepView::Part {
                position: 0,
                part: crate::app::state::ChatPartView {
                    kind: crate::chat::events::PartKind::Reasoning,
                    content: "thinking body".into(),
                    provider_metadata_json: None,
                    started_at_ms: Some(0),
                    ended_at_ms: Some(1000),
                    completed_at_ms: Some(1000),
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
        target.lx.agents.runs.push(run);
    }
    let agent_geom = {
        let target = state.pane_anywhere(pane).expect("pane");
        ui::main_content::lx::content_geom(inner, &target.lx, cache.pane(pane)).expect("agent geom")
    };
    // 在 Agents 详情的 thinking 卡片行上拖拽：只选中文字，不展开思考块。
    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            agent_geom.rect.x + 2,
            agent_geom.rect.y,
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
            MouseEventKind::Drag(MouseButton::Left),
            agent_geom.rect.x + 8,
            agent_geom.rect.y,
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
            agent_geom.rect.x + 8,
            agent_geom.rect.y,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut cache,
        &mut dirty,
    );
    assert!(
        !state.pane_anywhere(pane).expect("pane").lx.agents.runs[0]
            .chat
            .reasoning_expanded(0, 0),
        "在 Agents 卡片上拖拽选字不得误触展开"
    );
    assert!(state.lx_selection.is_some(), "拖拽后保留 Agents 详情选区");

    // 原地单击（无拖拽）：切换折叠状态并清除选区。
    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            agent_geom.rect.x + 2,
            agent_geom.rect.y,
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
            agent_geom.rect.x + 2,
            agent_geom.rect.y,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut cache,
        &mut dirty,
    );
    assert!(
        state.pane_anywhere(pane).expect("pane").lx.agents.runs[0]
            .chat
            .reasoning_expanded(0, 0),
        "原地单击 Agents 卡片切换展开"
    );
    assert!(state.lx_selection.is_none(), "单击后不保留空选区");

    // Agents 详情区跨屏滚轮拖选：给子 agent 塞入多条消息使其可滚动，验证边拖边滚延伸内容行坐标。
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        let run = &mut target.lx.agents.runs[0];
        for idx in 1..20 {
            run.chat.messages.push(crate::app::state::ChatMessageView {
                sequence: idx,
                id: format!("am-{idx}"),
                role: "assistant".into(),
                status: "complete".into(),
                turn_index: idx,
                kind: "chat".into(),
                shell: None,
                steps: vec![crate::app::state::AssistantStepView::Part {
                    position: 0,
                    part: crate::app::state::ChatPartView {
                        kind: crate::chat::events::PartKind::Text,
                        content: format!("agent-line-{idx:02}"),
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
        run.follow = false;
        run.scroll = 0;
        run.rows_dirty = true;
    }
    let agent_geom = {
        let target = state.pane_anywhere(pane).expect("pane");
        ui::main_content::lx::content_geom(inner, &target.lx, cache.pane(pane)).expect("agent geom")
    };
    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            agent_geom.rect.x,
            agent_geom.rect.y,
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
            MouseEventKind::ScrollDown,
            agent_geom.rect.x + 6,
            agent_geom.rect.y + 2,
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
            agent_geom.rect.x + 6,
            agent_geom.rect.y + 2,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut cache,
        &mut dirty,
    );
    assert_eq!(
        state.lx_selection.and_then(|sel| sel.range()),
        Some(((0, 0), (5, 6))),
        "Agents 详情边拖边滚（每格 3 行）后，选区锚点钉在第 0 行、终点延伸到第 5 行"
    );
}

#[test]
fn ask_click_submits_a_single_select_question_and_does_not_reach_history() {
    let config = Config::default();
    let (mut state, pane) = busy_ask_state();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let (column, row) = ask_option_point(&state, pane, &geo);
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
    // 单题单选：没有提交按钮，单击选项立即选中并提交整题。
    assert!(
        state
            .pane_anywhere(pane)
            .and_then(|pane| pane.lx.ask.as_ref())
            .is_none(),
        "单题单选单击即提交，面板随之关闭"
    );
    assert!(
        state.effects.iter().any(|effect| matches!(
            effect,
            crate::app::effects::Effect::AnswerAsk { call_id, .. } if call_id == "call"
        )),
        "点击必须推作答效应"
    );
    assert!(dirty, "点击必须重绘");
    assert!(
        state.lx_card_press.is_none(),
        "面板内点击不得登记历史卡片点击"
    );
}

#[test]
fn resolving_the_ask_list_window_feeds_page_scrolling() {
    // `PageUp`/`PageDown` 在 app 层拿不到窗格几何：事件循环必须把渲染层的窗口
    // （起点、总行数、可视行数）落进状态，否则翻页永远不动。
    let config = Config::default();
    let (mut state, pane) = busy_ask_state();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    assert!(update::resolve_ask_list_window(&mut state, &geo.rects));
    let inner = tiling::pane_inner_rect(
        geo.rects
            .iter()
            .find(|(id, _)| *id == pane)
            .map(|(_, rect)| *rect)
            .expect("pane rect"),
    );
    let lx = &state.pane_anywhere(pane).expect("pane").lx;
    let (scroll, total, visible) =
        ui::main_content::lx::ask_list_window(inner, lx).expect("list window");
    let ask = lx.ask.as_ref().expect("ask");
    assert_eq!(ask.list_scroll, scroll);
    assert_eq!(ask.list_total, total);
    assert_eq!(ask.list_visible, visible);
    assert!(total > 0 && visible > 0);
}

#[test]
fn task_page_swallows_ask_hits_so_the_list_stays_clickable() {
    // 后台任务页与 ask 面板共用输入框矩形：页打开时面板不可见，点击必须落在
    // 任务列表上，绝不能命中隐藏面板改一个看不见的答案。
    let config = Config::default();
    let (mut state, pane) = busy_ask_state();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let (column, row) = ask_option_point(&state, pane, &geo);
    let inner = tiling::pane_inner_rect(
        geo.rects
            .iter()
            .find(|(id, _)| *id == pane)
            .map(|(_, rect)| *rect)
            .expect("pane rect"),
    );
    state.pane_mut_anywhere(pane).expect("pane").lx.task = Some(crate::app::state::TaskPage {
        tab: crate::app::state::TaskTab::Processes,
        ..Default::default()
    });

    let lx = &state.pane_anywhere(pane).expect("pane").lx;
    // 任务页接管了输入框矩形，ask 几何已经不在这里：点击绝不能落到隐藏面板。
    assert!(
        ui::main_content::lx::ask_hit(inner, lx, column, row).is_none(),
        "任务页打开时 ask 几何必须失效"
    );

    let before = state
        .pane_anywhere(pane)
        .and_then(|pane| pane.lx.ask.as_ref())
        .expect("ask")
        .selected
        .clone();
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
    let after = state
        .pane_anywhere(pane)
        .and_then(|pane| pane.lx.ask.as_ref())
        .expect("ask")
        .selected
        .clone();
    assert_eq!(before, after, "隐藏的面板不得被点击改状态");
}

#[test]
fn ask_paste_only_enters_the_custom_buffer() {
    let config = Config::default();
    let (mut state, pane) = busy_ask_state();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let mut sessions = HashMap::new();

    // 列表态：整段吞掉，草稿一个字节都不动。
    let mut dirty = false;
    handle_terminal_event(
        TerminalEvent::Paste("ignored".into()),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(!dirty);
    let lx = &state.pane_anywhere(pane).expect("pane").lx;
    assert_eq!(lx.input, "draft");
    let ask = lx.ask.as_ref().expect("ask");
    assert_eq!(ask.custom[0].text(), "");

    // 编辑态：写入手写缓冲，草稿仍不变。
    state
        .pane_mut_anywhere(pane)
        .and_then(|pane| pane.lx.ask.as_mut())
        .expect("ask")
        .begin_edit();
    let mut dirty = false;
    handle_terminal_event(
        TerminalEvent::Paste("own words".into()),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(dirty);
    let lx = &state.pane_anywhere(pane).expect("pane").lx;
    assert_eq!(lx.input, "draft", "草稿不变量");
    assert_eq!(
        lx.ask.as_ref().expect("ask").custom[0].text(),
        "own words",
        "编辑态粘贴进入手写缓冲"
    );
}

#[test]
fn task_page_hides_input_popups_and_routes_chat_bar_wheel_to_output() {
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
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 24));
    let pane = state.active_tab().layout.focus();
    let inner = pane_inner(&geo, pane);
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        for ch in "\nline2\nline3\nline4\nline5".chars() {
            target.lx.insert_char(ch);
        }
        target.lx.scroll_to(2);
        target.lx.task = Some(crate::app::state::TaskPage {
            output: (0..30).map(|idx| format!("out-{idx:02}")).collect(),
            output_visible: 10,
            output_scroll: 5,
            ..Default::default()
        });
    }
    let lx = &state.pane_anywhere(pane).expect("pane").lx;
    assert!(
        ui::main_content::lx::mention_layout(inner, lx).is_none(),
        "后台任务页打开时 @ 提及浮层隐藏，不遮挡回到底部按钮"
    );
    let task_rects = ui::main_content::lx::task_rects(inner, lx).expect("task rects");
    let chat_bar_row = task_rects.output.bottom();
    assert_eq!(
        chat_bar_row + 1,
        task_rects.list.y,
        "output 与 list 之间存在 1 行回到底部栏"
    );
    let input_scroll_before = lx.scroll();
    let mut sessions = HashMap::new();
    let mut dirty = false;
    handle_terminal_event(
        mouse(
            MouseEventKind::ScrollDown,
            task_rects.output.x + 2,
            chat_bar_row,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    let lx = &state.pane_anywhere(pane).expect("pane").lx;
    assert_eq!(
        lx.task.as_ref().expect("task").output_scroll,
        8,
        "在后台任务页 chat_bar 行上滚轮滚动输出区"
    );
    assert_eq!(
        lx.scroll(),
        input_scroll_before,
        "不得泄漏到隐藏的底部输入框滚动"
    );
}

#[test]
fn processes_first_row_wheel_scrolls_the_list() {
    let config = Config::default();
    let mut state = AppState::demo();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 24));
    let pane = state.active_tab().layout.focus();
    let inner = pane_inner(&geo, pane);
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.view = PaneView::Lx;
        target.lx.task = Some(crate::app::state::TaskPage {
            processes: (0..12)
                .map(|index| crate::app::state::ProcessEntry {
                    session_id: index,
                    command: format!("cmd-{index}"),
                    cwd: "/w".into(),
                    tty: false,
                    started_at_ms: 0,
                })
                .collect(),
            ..Default::default()
        });
    }
    let list = ui::main_content::lx::task_rects(inner, &state.pane_anywhere(pane).unwrap().lx)
        .expect("task rects")
        .list;
    // 顶边框 + tab 行之下就是 Processes 的第一条；不能再用 search_rect。
    let first_row = list.y.saturating_add(2);
    let mut sessions = HashMap::new();
    let mut dirty = false;
    handle_terminal_event(
        mouse(MouseEventKind::ScrollDown, list.x + 2, first_row),
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
            .unwrap()
            .lx
            .task
            .as_ref()
            .unwrap()
            .window
            .viewport
            .is_some_and(|offset| offset > 0),
        "Processes 首行滚轮不能被搜索行守卫吞掉"
    );
}

/// 粘贴也是编辑：必须让双 Esc 的待命失效，避免带草稿选中后回填覆盖。
#[test]
fn lx_paste_cancels_rewind_arming() {
    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.view = PaneView::Lx;
        target.lx.chat.session_id = Some("sess".into());
        target.lx.rewind.armed_at = Some(std::time::Instant::now());
    }
    assert!(paste::handle_lx_paste(&mut state, "pasted".into()));
    let target = state.pane_mut_anywhere(pane).expect("pane");
    assert!(target.lx.rewind.armed_at.is_none(), "粘贴必须清除撤销待命");
    assert!(target.lx.text_paste_panel().is_some());
    assert!(
        target
            .lx
            .text_paste_confirm(crate::app::paste::TextPasteAction::Raw)
    );
    assert_eq!(target.lx.input, "pasted");
}

#[test]
fn lx_paste_image_path_opens_panel_and_confirms_refer_image() {
    let dir = std::env::temp_dir().join(format!("lx-paste-img-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("dir");
    let img_path = dir.join("PixPin_2026-10-29_09-52-07.png");
    let img: image::ImageBuffer<image::Rgba<u8>, Vec<u8>> =
        image::ImageBuffer::from_pixel(2, 2, image::Rgba([1, 2, 3, 255]));
    let mut png = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .expect("png");
    std::fs::write(&img_path, &png).expect("write");

    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    state.pane_mut_anywhere(pane).expect("pane").view = PaneView::Lx;

    let path_str = img_path.to_string_lossy().into_owned();
    assert!(paste::handle_lx_paste(&mut state, path_str.clone()));
    let target = state.pane_mut_anywhere(pane).expect("pane");
    assert!(
        target.lx.paste_panel().is_some(),
        "粘贴有路径的图片必须弹出二级面板"
    );
    assert!(
        target
            .lx
            .paste_confirm(crate::app::prompt::PasteAction::Reference)
    );
    assert_eq!(target.lx.input, format!("@[refer-image]({path_str}) "));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn task_page_does_not_cut_or_copy_hidden_composer_selection() {
    use crate::app::state::{PaneView, TaskPage};

    let config = Config::default();
    let mut state = AppState::demo();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 22));
    let pane = state.active_tab().layout.focus();
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.view = PaneView::Lx;
        target.lx.insert_str("abc");
        target.lx.begin_text_selection(0, 0);
        target.lx.drag_text_selection(0, 2);
        target.lx.task = Some(TaskPage::default());
        assert_eq!(target.lx.selected_text().as_deref(), Some("ab"));
    }
    let mut sessions = HashMap::new();
    let mut cache = ui::RenderCache::default();
    let mut dirty = false;
    for code in [KeyCode::Char('x'), KeyCode::Char('c')] {
        handle_terminal_event(
            TerminalEvent::Key(KeyEvent::new(code, KeyModifiers::CONTROL)),
            &mut state,
            &mut sessions,
            &geo,
            &config,
            &mut cache,
            &mut dirty,
        );
        let target = state.pane_anywhere(pane).expect("pane");
        assert!(target.lx.task.is_some(), "任务页仍打开");
        assert_eq!(target.lx.input, "abc", "隐藏草稿不得被剪切/复制改写");
        assert_eq!(target.lx.selected_text().as_deref(), Some("ab"));
    }
}

#[test]
fn agents_composer_supports_copy_cut_and_esc() {
    use crate::app::state::{PaneView, TaskPage, TaskTab};

    let config = Config::default();
    let mut state = AppState::demo();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 22));
    let pane = state.active_tab().layout.focus();
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.view = PaneView::Lx;
        target.lx.task = Some(TaskPage {
            tab: TaskTab::Agents,
            agent_edit: Some("ag-1".into()),
            ..TaskPage::default()
        });
        target
            .lx
            .agents
            .ensure_composer("ag-1")
            .editor_mut()
            .replace_input("hello");
        target
            .lx
            .agents
            .composer_mut("ag-1")
            .expect("composer")
            .editor_mut()
            .select_all();
    }

    let mut sessions = HashMap::new();
    let mut cache = ui::RenderCache::default();
    let mut dirty = false;
    handle_terminal_event(
        TerminalEvent::Key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut cache,
        &mut dirty,
    );
    assert!(
        state
            .pane_anywhere(pane)
            .and_then(|p| p.lx.agents.composer("ag-1"))
            .is_some_and(|composer| composer.editor().selected_text().as_deref() == Some("hello")),
        "Ctrl+C 复制 Agents composer 选区"
    );

    handle_terminal_event(
        TerminalEvent::Key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL)),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut cache,
        &mut dirty,
    );
    assert!(
        state
            .pane_anywhere(pane)
            .and_then(|p| p.lx.agents.composer("ag-1"))
            .is_some_and(|composer| composer.text().is_empty()),
        "Ctrl+X 剪切后 Agents composer 为空"
    );

    handle_terminal_event(
        TerminalEvent::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut cache,
        &mut dirty,
    );
    let page = state
        .pane_anywhere(pane)
        .and_then(|p| p.lx.task.as_ref())
        .expect("task page");
    assert!(page.agent_edit.is_none(), "Esc 返回列表态");
}

#[test]
fn task_list_scrollbar_mouse_press_and_drag_events() {
    let config = Config::default();
    let mut state = AppState::demo();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 24));
    let pane = state.active_tab().layout.focus();
    let inner = pane_inner(&geo, pane);
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.view = PaneView::Lx;
        let many_processes: Vec<crate::app::state::ProcessEntry> = (1..=10)
            .map(|i| crate::app::state::ProcessEntry {
                session_id: i,
                command: format!("proc-{i}"),
                cwd: "/w".into(),
                tty: false,
                started_at_ms: 0,
            })
            .collect();
        let mut page = crate::app::state::TaskPage::default();
        page.set_processes(many_processes, 0);
        target.lx.task = Some(page);
    }
    let bar = {
        let target = state.pane_anywhere(pane).expect("pane");
        ui::main_content::lx::task_list_scrollbar(inner, &target.lx)
            .expect("item 列表溢出时必须有滚动条")
    };
    let mut sessions = HashMap::new();
    let mut cache = ui::RenderCache::default();
    let mut dirty = false;
    let mut send = |kind, col, row, state: &mut AppState| {
        handle_terminal_event(
            mouse(kind, col, row),
            state,
            &mut sessions,
            &geo,
            &config,
            &mut cache,
            &mut dirty,
        );
    };

    // 1. 点击轨道 thumb 处按下并拖到底端，松开结束拖拽
    send(
        MouseEventKind::Down(MouseButton::Left),
        bar.track.x,
        bar.thumb.y,
        &mut state,
    );
    assert!(state.task_list_scroll_drag.is_some(), "点击 thumb 开启拖拽");
    let bottom_row = bar.track.bottom().saturating_sub(1);
    send(
        MouseEventKind::Drag(MouseButton::Left),
        bar.track.x,
        bottom_row,
        &mut state,
    );
    assert_eq!(
        state
            .pane_anywhere(pane)
            .unwrap()
            .lx
            .task
            .as_ref()
            .unwrap()
            .window
            .viewport,
        Some(4),
        "拖拽到底部对应视口偏移 4"
    );
    send(
        MouseEventKind::Up(MouseButton::Left),
        bar.track.x,
        bottom_row,
        &mut state,
    );
    assert!(state.task_list_scroll_drag.is_none(), "松开鼠标清除拖拽");

    // 2. 点击滚动条右侧边距一列（容错命中）：跳转视口到顶部
    send(
        MouseEventKind::Down(MouseButton::Left),
        inner.right().saturating_sub(1),
        bar.track.y,
        &mut state,
    );
    assert_eq!(
        state
            .pane_anywhere(pane)
            .unwrap()
            .lx
            .task
            .as_ref()
            .unwrap()
            .window
            .viewport,
        Some(0),
        "点击顶部轨道跳回视口 0"
    );

    // 3. 在滚动条列使用滚轮滚动：视口正常向下滚动
    send(
        MouseEventKind::ScrollDown,
        bar.track.x,
        bar.track.y + 1,
        &mut state,
    );
    assert_eq!(
        state
            .pane_anywhere(pane)
            .unwrap()
            .lx
            .task
            .as_ref()
            .unwrap()
            .window
            .viewport,
        Some(3),
        "滚轮向下滚动 3 行"
    );
}

#[test]
fn task_agents_list_scrollbar_preserves_viewport_across_refresh_and_selection() {
    let config = Config::default();
    let mut state = AppState::demo();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 24));
    let pane = state.active_tab().layout.focus();
    let inner = pane_inner(&geo, pane);
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.view = PaneView::Lx;
        for i in 1..=8 {
            target
                .lx
                .agents
                .runs
                .push(crate::app::state::SubagentRun::from_record(
                    &crate::db::chat::SubagentRecord {
                        agent_id: format!("sub-{i}"),
                        child_session_id: format!("child-{i}"),
                        parent_session_id: "parent".into(),
                        spawn_message_id: None,
                        role: crate::chat::agent::AgentRole::Explorer,
                        status: "completed".into(),
                        title: format!("Task {i}"),
                        created_at_ms: i,
                        completed_at_ms: Some(i + 1),
                        report_json: None,
                        report_delivered: true,
                        nickname: Some(format!("Agent{i}")),
                        agent_path: format!("/root/agent{i}"),
                        model_provider: String::new(),
                        model_id: String::new(),
                        model_variant: None,
                    },
                ));
        }
        let mut page = crate::app::state::TaskPage::default();
        page.select_tab(crate::app::state::TaskTab::Agents);
        page.agents_window.active = 2;
        page.agents_window.anchor = 2;
        target.lx.task = Some(page);
    }
    let bar = {
        let target = state.pane_anywhere(pane).expect("pane");
        ui::main_content::lx::task_list_scrollbar(inner, &target.lx)
            .expect("8 个子 agent 时列表必须有滚动条")
    };
    let mut sessions = HashMap::new();
    let mut cache = ui::RenderCache::default();
    let mut dirty = false;
    let bottom_row = bar.track.bottom().saturating_sub(1);
    for kind in [
        MouseEventKind::Down(MouseButton::Left),
        MouseEventKind::Drag(MouseButton::Left),
        MouseEventKind::Up(MouseButton::Left),
    ] {
        let row = if matches!(kind, MouseEventKind::Down(_)) {
            bar.thumb.y
        } else {
            bottom_row
        };
        handle_terminal_event(
            mouse(kind, bar.track.x, row),
            &mut state,
            &mut sessions,
            &geo,
            &config,
            &mut cache,
            &mut dirty,
        );
    }
    let agents_viewport = |state: &AppState| {
        state
            .pane_anywhere(pane)
            .unwrap()
            .lx
            .task
            .as_ref()
            .unwrap()
            .agents_window
            .viewport
    };
    assert_eq!(agents_viewport(&state), Some(3), "拖拽到底部后视口为 3");

    // 后台列表同步与点击当前视口内条目均不得把视口重置回 None/0
    update::restore_selected_task_agent(
        &mut state.pane_mut_anywhere(pane).expect("pane").lx,
        Some("sub-3"),
    );
    assert!(update::select_task_agent(&mut state, pane, 2));
    assert_eq!(
        agents_viewport(&state),
        Some(3),
        "后台刷新与点选条目保持当前滚动视口"
    );
}

#[test]
fn wheel_on_task_footer_bottom_bar_does_not_scroll_hidden_main_input() {
    use crate::app::state::{PaneView, TaskPage};

    let config = Config::default();
    let mut state = AppState::demo();
    let geo = geometry(&state, &config, Rect::new(0, 0, 80, 20));
    let pane = state.active_tab().layout.focus();
    let inner = pane_inner(&geo, pane);
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.view = PaneView::Lx;
        target.lx.resize(40, 4);
        target
            .lx
            .insert_str("r1\nr2\nr3\nr4\nr5\nr6\nr7\nr8\nr9\nr10");
        assert!(target.lx.scroll() > 0, "主输入框溢出时应处于非零滚动位置");
        target.lx.task = Some(TaskPage::default());
    }
    let before_scroll = state.pane_anywhere(pane).expect("pane").lx.scroll();
    let bottom_bar_row = inner.bottom() - 1;
    let mut sessions = HashMap::new();
    let mut cache = ui::RenderCache::default();
    let mut dirty = false;
    handle_terminal_event(
        mouse(MouseEventKind::ScrollUp, inner.x + 2, bottom_bar_row),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut cache,
        &mut dirty,
    );
    assert_eq!(
        state.pane_anywhere(pane).expect("pane").lx.scroll(),
        before_scroll,
        "任务面板打开时在 bottom_bar 行滚轮不得穿透滚动隐藏的主输入框"
    );
    assert!(!dirty, "未产生有效滚动时不得置 dirty");
}
