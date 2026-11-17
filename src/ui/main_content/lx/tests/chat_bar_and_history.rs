//! 聊天历史滚动条、聊天栏（loading / 扫描高亮 / token 估算 / 回到底部）、置顶行与代码块测试。

use super::*;
use crate::app::state::{ChatMessageView, ChatPartView, LxChat};

#[test]
fn initial_page_shows_status_above_pane_bottom_border() {
    // 模拟窗格：内容区 `inner` + 底边框行（`inner.bottom()`，位于 inner 之外）。
    let rect = Rect::new(0, 0, 80, 24);
    let inner = crate::tiling::pane_inner_rect(rect);
    let mut lx = Lx::new();
    lx.chat.status = crate::app::state::lx::chat::ChatStatus {
        model_name: Some("deepseek-v4.1-flash".into()),
        provider_name: Some("9router-lx-t".into()),
        variant: None,
        context_limit: Some(1_000_000),
        input_tokens: Some(720_000),
        cache_hit_percent: None,
        streaming: false,
        omitted_turns: None,
        blocked_subagents: 0,
    };
    let lay = layout(inner, &lx);
    let input = lay.input.expect("input");
    // 输入框下沿紧贴底部状态行（状态画在输入框下方、窗格底边框上方）。
    assert_eq!(input.bottom(), inner.bottom() - BOTTOM_BAR_HEIGHT);
    // 底边框行预铺 `─` 后渲染，检查状态在 `inner.bottom() - 1` 且底边框保持纯净。
    let mut buf = Buffer::empty(rect);
    let border_row = inner.bottom();
    for x in rect.x..rect.right() {
        if let Some(cell) = buf.cell_mut((x, border_row)) {
            cell.set_char('─');
        }
    }
    render(inner, &mut buf, &lx, false, None, &mut PaneCache::default());
    let status_row = inner.bottom() - 1;
    let status_line: String = (rect.x..rect.right())
        .map(|x| buf.cell((x, status_row)).map(|c| c.symbol()).unwrap_or(" "))
        .collect();
    let border: String = (rect.x..rect.right())
        .map(|x| buf.cell((x, border_row)).map(|c| c.symbol()).unwrap_or(" "))
        .collect();
    assert!(
        status_line.contains("deepseek-v4.1-flash"),
        "状态行显示模型名：{status_line}"
    );
    assert!(
        !status_line.contains("9router-lx-t"),
        "供应商不再显示：{status_line}"
    );
    assert!(
        status_line.contains("720k(72%)"),
        "状态行显示上下文占用 `720k(72%)`（无 context: 前缀）：{status_line}"
    );
    assert_eq!(
        border,
        "─".repeat(usize::from(rect.width)),
        "窗格底边框必须保持纯净：{border}"
    );
}

#[test]
fn chat_history_scrollbar_geometry_appears_when_overflowing() {
    let area = Rect::new(0, 0, 40, 6);
    let mut lx = Lx::new();
    let mut chat = LxChat::new();
    for index in 0..12 {
        chat.messages.push(ChatMessageView {
            sequence: 0,
            id: format!("m{index}"),
            role: "user".into(),
            status: "complete".into(),
            turn_index: index,
            kind: "chat".into(),
            shell: None,
            steps: vec![crate::app::state::AssistantStepView::Part {
                position: 0,
                part: ChatPartView {
                    kind: crate::chat::events::PartKind::Text,
                    content: format!("消息 {index}"),
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
    lx.chat = chat;
    let bar = history_scrollbar(area, &lx, &mut PaneCache::default()).expect("溢出时有滚动条几何");
    // 滚动条贴历史区（与输入框同宽）右缘一列。
    let input = layout(area, &lx).input.expect("input");
    assert_eq!(bar.track.x, input.right() - 1, "滚动条在输入框右缘内一列");
    assert!(bar.total > bar.visible);
}

#[test]
fn overflowing_history_draws_scrollbar_glyphs() {
    let area = Rect::new(0, 0, 40, 10);
    let mut lx = Lx::new();
    let mut chat = LxChat::new();
    for index in 0..12 {
        chat.messages.push(ChatMessageView {
            sequence: 0,
            id: format!("m{index}"),
            role: "user".into(),
            status: "complete".into(),
            turn_index: index,
            kind: "chat".into(),
            shell: None,
            steps: vec![crate::app::state::AssistantStepView::Part {
                position: 0,
                part: ChatPartView {
                    kind: crate::chat::events::PartKind::Text,
                    content: format!("消息 {index}"),
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
    lx.chat = chat;
    let mut buf = Buffer::empty(area);
    let _ = render(area, &mut buf, &lx, false, None, &mut PaneCache::default());
    let text = buffer_text(&buf);
    // 历史滚动条与公共组件同口径：只有 thumb，没有轨道竖线。
    assert!(text.contains('▐'), "溢出时历史滚动条可见：{text}");
    assert!(!text.contains('▕'), "轨道不再绘制竖线：{text}");
}

// ===== 输入框上方聊天栏（loading 转轮 + 回到底部按钮）=====

/// 构造带一条 assistant 流中消息的 lx 状态。
fn streaming_lx(streaming: bool, follow_bottom: bool) -> Lx {
    let mut lx = Lx::new();
    let mut chat = LxChat::new();
    chat.messages.push(ChatMessageView {
        sequence: 0,
        id: "m0".into(),
        role: "assistant".into(),
        status: if streaming { "streaming" } else { "complete" }.into(),
        turn_index: 0,
        kind: "chat".into(),
        shell: None,
        steps: vec![crate::app::state::AssistantStepView::Part {
            position: 0,
            part: ChatPartView {
                kind: crate::chat::events::PartKind::Text,
                content: "答".into(),
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
    chat.follow_bottom = follow_bottom;
    chat.status.streaming = streaming;
    chat.loading_frame = 0;
    lx.chat = chat;
    lx
}

#[test]
fn streaming_shows_loading_bar_above_input() {
    let area = Rect::new(0, 0, 60, 16);
    let lx = streaming_lx(true, true);
    let lay = layout(area, &lx);
    let input = lay.input.expect("input");
    let bar = lay.chat_bar.expect("流中显示聊天栏");
    assert_eq!(bar.bottom(), input.y, "聊天栏紧贴输入框上沿");
    assert_eq!(bar.height, 1);
    let mut buf = Buffer::empty(area);
    let _ = render(area, &mut buf, &lx, false, None, &mut PaneCache::default());
    let text = buffer_text(&buf);
    assert!(
        text.contains(text::chat::LOADING_LABEL),
        "显示 loading 文案：{text}"
    );
    assert!(
        text.contains(crate::ui::text::agent::STEP_MARK_DOT_FILLED),
        "流中的步骤标记用圆点（不再是 braille 转轮）：{text}"
    );
    assert!(
        !text
            .chars()
            .any(|ch| ('\u{2800}'..='\u{28ff}').contains(&ch)),
        "不再出现 braille 转轮：{text}"
    );
}

#[test]
fn idle_following_hides_chat_bar() {
    let area = Rect::new(0, 0, 60, 16);
    let lx = streaming_lx(false, true);
    assert!(
        layout(area, &lx).chat_bar.is_none(),
        "空闲且跟随底部：无聊天栏"
    );
}

/// 主轮自然结束、后台子 agent 仍在跑：继续显示 waiting loading，并在底部状态行计数。
#[test]
fn idle_main_with_running_subagent_shows_loading_and_counts() {
    let lx = lx_with_running_subagent(false);
    let rect = Rect::new(0, 0, 60, 17);
    let area = crate::tiling::pane_inner_rect(rect);
    assert!(
        layout(area, &lx).chat_bar.is_some(),
        "主轮自然结束后，子 agent 运行中仍显示 loading"
    );
    let text = render_with_border(area, rect, &lx);
    assert!(
        text.contains(text::chat::WAITING_AGENTS_LABEL),
        "显示等待子 agent 文案：{text}"
    );
    assert!(
        text.contains("1 process · 1 subagent"),
        "运行指标在底部状态行显示：{text}"
    );
}

/// 用户显式停止主轮后、后台子 agent 仍在跑：不显示 loading，只在底部状态行计数。
#[test]
fn stopped_main_with_running_subagent_shows_only_counts() {
    let lx = lx_with_running_subagent(true);
    let rect = Rect::new(0, 0, 60, 17);
    let area = crate::tiling::pane_inner_rect(rect);
    assert!(
        layout(area, &lx).chat_bar.is_none(),
        "主轮被用户停止后，子 agent 运行中也不再显示 loading"
    );
    let text = render_with_border(area, rect, &lx);
    assert!(
        !text.contains(text::chat::WAITING_AGENTS_LABEL),
        "不显示等待子 agent 的 loading：{text}"
    );
    assert!(
        text.contains("1 process · 1 subagent"),
        "运行指标仍在底部状态行显示：{text}"
    );
}

/// 构造一个主轮 idle、带 1 个后台任务和 1 个 running 子 agent 的 lx。
fn lx_with_running_subagent(main_stopped: bool) -> Lx {
    let mut lx = streaming_lx(false, true);
    lx.background_tasks = 1;
    lx.chat.main_stopped = main_stopped;
    lx.agents
        .runs
        .push(crate::app::state::SubagentRun::from_record(
            &crate::db::chat::SubagentRecord {
                agent_id: "sub-a".into(),
                nickname: None,
                agent_path: "/root/agent".into(),
                child_session_id: "sess-a".into(),
                parent_session_id: "parent".into(),
                spawn_message_id: None,
                role: crate::chat::agent::AgentRole::Explorer,
                model_provider: String::new(),
                model_id: String::new(),
                model_variant: None,
                status: "running".into(),
                title: "investigate".into(),
                created_at_ms: 0,
                completed_at_ms: None,
                report_json: None,
                report_delivered: false,
            },
        ));
    lx
}

/// 窗格内容区 + 底边框行预铺 `─` 后渲染，返回整个缓冲区文本。
fn render_with_border(area: Rect, rect: Rect, lx: &Lx) -> String {
    let mut buf = Buffer::empty(rect);
    let row = area.bottom();
    for x in rect.x..rect.right() {
        if let Some(cell) = buf.cell_mut((x, row)) {
            cell.set_char('─');
        }
    }
    let _ = render(area, &mut buf, lx, false, None, &mut PaneCache::default());
    buffer_text(&buf)
}

/// `/compact` 期间：输入框上方那一行出现「压缩中」并带本次压缩耗时。
#[test]
fn compacting_shows_loading_bar_above_input() {
    let area = Rect::new(0, 0, 60, 16);
    let mut lx = streaming_lx(false, true);
    lx.chat.turn_elapsed_ms = Some(9_000);
    lx.compact.start("op".into(), 0);
    lx.compact.elapsed_ms = Some(4_100);
    let lay = layout(area, &lx);
    let input = lay.input.expect("input");
    let bar = lay.chat_bar.expect("压缩中显示聊天栏");
    assert_eq!(bar.bottom(), input.y, "聊天栏紧贴输入框上沿");
    let mut buf = Buffer::empty(area);
    let _ = render(area, &mut buf, &lx, false, None, &mut PaneCache::default());
    let text = buffer_text(&buf);
    assert!(
        text.contains(text::chat::COMPACTING_LABEL),
        "显示压缩中文案：{text}"
    );
    assert!(text.contains("(4s)"), "显示本次压缩耗时：{text}");
    assert!(!text.contains("9s"), "不得显示上一轮聊天耗时：{text}");
    // 压缩结束后整行消失。
    lx.compact.finish();
    assert!(
        layout(area, &lx).chat_bar.is_none(),
        "压缩结束且空闲：无聊天栏"
    );
}

#[test]
fn scrolled_up_shows_jump_bottom_button() {
    let area = Rect::new(0, 0, 60, 16);
    let lx = streaming_lx(false, false);
    let rect = chat_bar::jump_bottom_rect(area, &lx).expect("上滚时显示回到底部控件");
    let bar = layout(area, &lx).chat_bar.expect("chat bar");
    assert_eq!(rect.right(), bar.right() - 1, "控件贴聊天栏右端（留 1 列）");
    let mut buf = Buffer::empty(area);
    let _ = render(area, &mut buf, &lx, false, None, &mut PaneCache::default());
    let text = buffer_text(&buf);
    assert!(text.contains(text::LX_JUMP_BOTTOM), "显示 ↓ 箭头：{text}");
    assert!(
        text.contains(text::LX_JUMP_BOTTOM_HINT),
        "箭头右侧显示说明：{text}"
    );
}

#[test]
fn jump_bottom_hit_matches_button_rect() {
    use crate::app::state::AppState;
    let area = Rect::new(0, 0, 60, 16);
    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    if let Some(target) = state.pane_mut_anywhere(pane) {
        target.lx = streaming_lx(false, false);
    }
    // 直接构造窗格矩形（内缩 1 边框），命中按钮内部一格。
    let rects = [(pane, Rect::new(0, 0, 62, 18))];
    let inner = crate::tiling::pane_inner_rect(rects[0].1);
    let button =
        chat_bar::jump_bottom_rect(inner, &state.pane_anywhere(pane).unwrap().lx).expect("button");
    assert_eq!(
        chat_bar::jump_bottom_hit(&rects, &state, button.x, button.y),
        Some(pane),
        "命中回到底部按钮"
    );
    assert_eq!(
        chat_bar::jump_bottom_hit(&rects, &state, 0, 0),
        None,
        "未命中"
    );
    let _ = area;
}

#[test]
fn esc_armed_replaces_loading_label_with_stop_hint() {
    let area = Rect::new(0, 0, 60, 16);
    let mut lx = streaming_lx(true, true);
    lx.chat.stop_armed_at = Some(std::time::Instant::now());
    let mut buf = Buffer::empty(area);
    let _ = render(area, &mut buf, &lx, false, None, &mut PaneCache::default());
    let text = buffer_text(&buf);
    assert!(text.contains("Press esc again to stop"), "待命提示：{text}");
    assert!(
        !text.contains(text::chat::LOADING_LABEL),
        "提示替换 Working：{text}"
    );
    // 未待命（超时/未按 Esc）时仍是普通 loading 文案。
    lx.chat.stop_armed_at = None;
    let mut buf = Buffer::empty(area);
    let _ = render(area, &mut buf, &lx, false, None, &mut PaneCache::default());
    assert!(buffer_text(&buf).contains(text::chat::LOADING_LABEL));
}

#[test]
fn esc_hint_shortens_on_narrow_bar() {
    // 窄栏：停止反馈优先取得文字预算，短停止短语仍按宽度降级显示；
    // 统计不再整段让位——空间够时保留（设计 §6.4），极窄才被裁切。
    let area = Rect::new(0, 0, 22, 16);
    let mut lx = streaming_lx(true, true);
    lx.chat.stop_armed_at = Some(std::time::Instant::now());
    let mut buf = Buffer::empty(area);
    let _ = render(area, &mut buf, &lx, false, None, &mut PaneCache::default());
    let text = buffer_text(&buf);
    assert!(
        text.contains(text::LX_CHAT_STOP_PHRASES[1])
            || text.contains(text::LX_CHAT_STOP_PHRASES[2])
            || text.contains(text::LX_CHAT_STOP_PHRASES[3]),
        "窄栏取短短语：{text}"
    );
    assert!(
        !text.contains("Press esc again to stop"),
        "放不下时不画长短语：{text}"
    );
    // 统计仍在同一行（不再是「窄栏先丢 token」的旧行为）。
    assert!(text.contains("↓1 tok"), "待命期间统计保留：{text}");
}

// ===== 置顶用户消息行（命中与跳转目标）=====

#[test]
fn sticky_prompt_row_is_clickable_and_jumps_to_the_prompt() {
    let area = Rect::new(0, 0, 40, 16);
    let mut lx = sticky_history_lx();
    lx.chat.follow_bottom = false;
    // 扫一个"顶行落在回复里"的滚动位置（几何随布局变化，不硬编码行号）。
    let mut found = None;
    for scroll in 0..32 {
        lx.chat.history_scroll = scroll;
        if let Some(view) = history_view(area, &lx, &mut PaneCache::default())
            && let Some(message) = view.sticky
        {
            found = Some((message, view.area.y, view.messages[message].start));
            break;
        }
    }
    let (message, row_y, start) = found.expect("存在置顶行");
    assert_eq!(message, 0, "钉第一轮 prompt（顶行在第一轮回复里）");
    assert_eq!(
        chat_hit(area, &lx, area.x + 4, row_y, &mut PaneCache::default()),
        Some(LxChatHit::PromptHeader),
        "置顶行整宽可点"
    );
    assert_eq!(
        sticky_prompt_jump(area, &lx, &mut PaneCache::default()),
        Some(start),
        "跳转目标 = 该消息起始行"
    );
    // 非置顶行不命中（body 第一行是正文）。
    assert_eq!(
        chat_hit(area, &lx, area.x + 4, row_y + 1, &mut PaneCache::default()),
        None
    );
}

#[test]
fn history_rows_uses_body_height_while_sticky_is_shown() {
    let area = Rect::new(0, 0, 40, 16);
    let mut lx = sticky_history_lx();
    lx.chat.follow_bottom = false;
    let mut with_sticky = None;
    for scroll in 0..32 {
        lx.chat.history_scroll = scroll;
        if let Some(view) = history_view(area, &lx, &mut PaneCache::default())
            && view.sticky.is_some()
        {
            with_sticky = Some((view.body.height, view.total_rows));
            break;
        }
    }
    let (body_height, total) = with_sticky.expect("存在置顶行");
    assert_eq!(
        history_rows(area, &lx, &mut PaneCache::default()),
        Some((total, usize::from(body_height))),
        "滚轮钳制用 body 高度（否则滚不到最后一行）"
    );
}

// ===== 聊天栏：Native 循环色卡片 + 扫描高亮 + 临时警告 =====

/// 取聊天栏整行（文本, 每列样式）。
fn chat_bar_cells(area: Rect, lx: &Lx) -> (String, Vec<ratatui::style::Style>) {
    let lay = layout(area, lx);
    let bar = lay.chat_bar.expect("聊天栏可见");
    let mut buf = Buffer::empty(area);
    let _ = render(area, &mut buf, lx, false, None, &mut PaneCache::default());
    let text: String = (bar.x..bar.right())
        .map(|x| buf[(x, bar.y)].symbol())
        .collect();
    let styles = (bar.x..bar.right())
        .map(|x| buf[(x, bar.y)].style())
        .collect();
    (text, styles)
}

/// loading **不再有方块**：整行只有 `Working...`，且它带扫描高亮。
#[test]
fn loading_shows_only_the_label_without_a_mark() {
    let area = Rect::new(0, 0, 60, 16);
    let mut lx = streaming_lx(true, true);
    lx.chat.loading_frame = 0;
    let (text, _styles) = chat_bar_cells(area, &lx);
    // 不含任何方块字形（实心/空心都不该出现）。
    assert!(
        !text.contains(crate::ui::text::agent::STEP_MARK_FILLED)
            && !text.contains(crate::ui::text::agent::STEP_MARK_HOLLOW),
        "loading 不该再有方块：{text:?}"
    );
    // 只有文案，且从第 1 列（贴左边距）开始。
    let trimmed = text.trim_start();
    assert!(
        trimmed.starts_with(text::chat::LOADING_LABEL),
        "只有 Working... 文案：{text:?}"
    );
}

/// 扫描高亮：窗口内字符取当前帧色（按 Native 标签 4 档循环），窗口外是 dim。
#[test]
fn working_label_scans_left_to_right() {
    let area = Rect::new(0, 0, 60, 16);
    let cycle = crate::ui::text::agent::native_label_cycle();
    let mut seen_hits = false;
    for frame in 0..12 {
        let mut lx = streaming_lx(true, true);
        lx.chat.loading_frame = frame;
        // 直接取渲染后的单元格样式：第 1 列起就是文案。
        let lay = layout(area, &lx);
        let bar = lay.chat_bar.expect("流中显示聊天栏");
        let mut buf = Buffer::empty(area);
        let _ = render(area, &mut buf, &lx, false, None, &mut PaneCache::default());
        let cells: Vec<&ratatui::buffer::Cell> =
            (bar.x + 1..bar.right()).map(|x| &buf[(x, bar.y)]).collect();
        let hits: Vec<usize> = cells
            .iter()
            .enumerate()
            .filter(|(_, cell)| {
                cell.style()
                    .add_modifier
                    .contains(ratatui::style::Modifier::BOLD)
            })
            .map(|(index, _)| index)
            .collect();
        if hits.is_empty() {
            continue;
        }
        seen_hits = true;
        assert!(hits.len() <= 3, "frame {frame}: 窗口不超过 3 字符 {hits:?}");
        // 窗口内颜色 = 当前帧的循环色。
        let expected = cycle[frame % cycle.len()];
        for index in &hits {
            assert_eq!(
                cells[*index].style().fg,
                Some(Color::Indexed(expected)),
                "frame {frame}: 被扫到的字符应为该帧色"
            );
        }
        // 窗口连续。
        assert!(
            hits.windows(2).all(|pair| pair[1] == pair[0] + 1),
            "frame {frame}: 窗口不连续 {hits:?}"
        );
    }
    assert!(seen_hits, "扫描窗口应当命中过文案");
}

/// 扫描自左向右推进：每帧前进一步，走完一轮后从左侧重新进入。
#[test]
fn scan_advances_one_char_per_frame_and_wraps() {
    let area = Rect::new(0, 0, 60, 16);
    let mut heads: Vec<usize> = Vec::new();
    for frame in 0..12 {
        let mut lx = streaming_lx(true, true);
        lx.chat.loading_frame = frame;
        let lay = layout(area, &lx);
        let bar = lay.chat_bar.expect("bar");
        let mut buf = Buffer::empty(area);
        let _ = render(area, &mut buf, &lx, false, None, &mut PaneCache::default());
        let head = (bar.x + 1..bar.right())
            .rfind(|x| {
                buf[(*x, bar.y)]
                    .style()
                    .add_modifier
                    .contains(ratatui::style::Modifier::BOLD)
            })
            .map(|x| usize::from(x - bar.x - 1));
        if let Some(head) = head {
            heads.push(head);
        }
    }
    assert!(heads.len() >= 4, "应收集到足够帧：{heads:?}");
    let wraps = heads.windows(2).filter(|pair| pair[1] < pair[0]).count();
    assert!(wraps <= 1, "扫描应只回绕一次：{heads:?}");
}

/// 未流中且没有警告时，聊天栏不该出现加载文案（避免残留）。
#[test]
fn idle_chat_bar_has_no_loading_mark() {
    let area = Rect::new(0, 0, 60, 16);
    let lx = streaming_lx(false, true);
    let mut buf = Buffer::empty(area);
    let _ = render(area, &mut buf, &lx, false, None, &mut PaneCache::default());
    let text = buffer_text(&buf);
    assert!(!text.contains(text::chat::LOADING_LABEL), "{text}");
}

#[test]
fn code_block_renders_left_bar_with_lang_and_copy_button() {
    // 助手正文代码块：顶部 `▂` 边框 + 下一行 `▎ js ... [❐] ` 头部（无边框）+ 正文左侧 `▎`。
    let area = Rect::new(0, 0, 60, 24);
    let mut lx = Lx::new();
    let mut chat = LxChat::new();
    chat.messages.push(ChatMessageView {
        sequence: 0,
        id: "a-0".into(),
        role: "assistant".into(),
        status: "complete".into(),
        turn_index: 0,
        kind: "chat".into(),
        shell: None,
        steps: vec![crate::app::state::AssistantStepView::Part {
            position: 0,
            part: ChatPartView {
                kind: crate::chat::events::PartKind::Text,
                content: "```js\n\tconst a = 1;\r\n\nconst b = 2;\n```".into(),
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
    lx.chat = chat;

    let mut buf = Buffer::empty(area);
    let _ = render(area, &mut buf, &lx, false, None, &mut PaneCache::default());
    let text = buffer_text(&buf);
    assert!(text.contains("▂▂▂▂▂▂"), "顶部 `▂` 边框：{text}");
    assert!(text.contains("▎ js"), "头部左侧显示语言名：{text}");
    assert!(text.contains("[❐]"), "头部右侧显示 `[❐]` 复制按钮：{text}");
    assert!(
        text.contains("▎    const a = 1;"),
        "代码正文的制表符按 4 列展开：{text}"
    );
    assert!(
        !text.chars().any(|ch| ch.is_control() && ch != '\n'),
        "渲染结果不得包含控制字符：{text:?}"
    );
    assert!(
        text.contains("▎const b = 2;"),
        "空行后的正文带左侧 `▎` 单栏：{text}"
    );
}

/// 流中 loading 行右侧给出输出 token 估算 `↓1.2k tok`；不流时不显示。
#[test]
fn streaming_loading_shows_estimated_output_tokens() {
    let area = Rect::new(0, 0, 60, 16);
    let mut lx = streaming_lx(true, true);
    // 4k ASCII 字符 ≈ 1k token。
    if let Some(message) = lx.chat.messages.last_mut()
        && let Some(part) = message.steps.first_mut().and_then(|step| step.part_mut())
    {
        part.content = "a".repeat(4_800);
    }
    let mut buf = Buffer::empty(area);
    let _ = render(area, &mut buf, &lx, false, None, &mut PaneCache::default());
    let text = buffer_text(&buf);
    assert!(text.contains("↓1.2k tok"), "显示 token 估算：{text}");
    assert!(text.contains(text::chat::LOADING_LABEL), "loading 文案仍在");

    // 流结束：那一行不再画 token（整行随 loading 消失）。
    lx.chat.status.streaming = false;
    if let Some(message) = lx.chat.messages.last_mut() {
        message.status = "complete".into();
    }
    let mut buf = Buffer::empty(area);
    let _ = render(area, &mut buf, &lx, false, None, &mut PaneCache::default());
    assert!(
        !buffer_text(&buf).contains(" tok"),
        "非流中不显示 token 估算"
    );
}

/// 估算口径：ASCII 4 字符 ≈ 1 token，非 ASCII 1 字符 ≈ 1 token；无输出为 None。
#[test]
fn stream_token_estimate_counts_wide_chars_heavier() {
    let mut lx = streaming_lx(true, true);
    assert_eq!(
        lx.chat.stream_output_tokens(),
        Some(1),
        "「答」算 1 个 token"
    );
    if let Some(message) = lx.chat.messages.last_mut()
        && let Some(part) = message.steps.first_mut().and_then(|step| step.part_mut())
    {
        part.content = "abcdefgh".into();
    }
    assert_eq!(
        lx.chat.stream_output_tokens(),
        Some(2),
        "8 个 ASCII ≈ 2 token"
    );
    if let Some(message) = lx.chat.messages.last_mut()
        && let Some(part) = message.steps.first_mut().and_then(|step| step.part_mut())
    {
        part.content.clear();
    }
    assert_eq!(lx.chat.stream_output_tokens(), None, "没有输出就不显示");
}

/// loading 行的统计紧跟 `Working...`（不是贴栏右缘），只含输出 token，耗时为秒。
#[test]
fn loading_stats_follow_the_label_with_seconds_and_output_tokens() {
    let area = Rect::new(0, 0, 80, 16);
    let mut lx = streaming_lx(true, true);
    lx.chat.turn_elapsed_ms = Some(2_700);
    if let Some(message) = lx.chat.messages.last_mut()
        && let Some(step) = message.steps.first_mut()
        && let Some(part) = step.part_mut()
    {
        part.content = "a".repeat(4_800);
    }
    let mut buf = Buffer::empty(area);
    let _ = render(area, &mut buf, &lx, false, None, &mut PaneCache::default());
    let row = buffer_text(&buf)
        .lines()
        .find(|line| line.contains(text::chat::LOADING_LABEL))
        .expect("loading 行")
        .to_string();
    let label_at = row.find(text::chat::LOADING_LABEL).expect("文案");
    let stats_at = row.find('(').expect("统计括号");
    assert_eq!(
        stats_at,
        label_at + text::chat::LOADING_LABEL.len() + 1,
        "统计紧贴文案右侧一格：{row}"
    );
    assert!(row.contains("3s"), "耗时按秒取整（2.7s → 3s）：{row}");
    assert!(!row.contains("ms"), "不出现毫秒：{row}");
    assert!(row.contains("↓1.2k tok"), "含输出 token：{row}");
    assert!(!row.contains('↑'), "不再显示输入方向：{row}");
}

/// 非首页（已有历史）不登记入口标签命中，点击消息内容不会穿透切换 agent。
#[test]
fn entry_tabs_are_not_hittable_once_history_exists() {
    let area = Rect::new(0, 0, 100, 40);
    let homepage = Lx::new();
    let lay = layout(area, &homepage);
    let (entry, rect) = *lay.entries.first().expect("homepage entries");
    let (x, y) = (rect.x + 1, rect.y);
    assert_eq!(
        entry_hit(area, &homepage, x, y),
        Some(entry),
        "首页仍可切换入口"
    );
    let history = streaming_lx(false, true);
    assert_eq!(
        entry_hit(area, &history, x, y),
        None,
        "有历史时入口不得参与命中"
    );
}
