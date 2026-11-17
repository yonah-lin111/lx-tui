//! `lx` 页选区反显与跨屏取词测试（主聊天历史、宽字符与后台面板）。

use super::*;
use crate::app::state::{ChatMessageView, ChatPartView, LxChat};

#[test]
fn copy_text_extracts_selected_rows() {
    let area = Rect::new(0, 0, 100, 20);
    let lx = sticky_history_lx();
    let text = copy_text(area, &lx, ((0, 0), (19, 99)), &mut PaneCache::default());
    assert!(text.contains("第一轮的问题"), "{text}");
    assert!(text.contains("第二轮的问题"), "{text}");
}

#[test]
fn copy_text_keeps_wide_chars_once() {
    let area = Rect::new(0, 0, 60, 12);
    let mut lx = Lx::new();
    let mut chat = LxChat::new();
    chat.messages.push(ChatMessageView {
        sequence: 0,
        id: "u0".into(),
        role: "user".into(),
        status: "complete".into(),
        turn_index: 0,
        kind: "chat".into(),
        shell: None,
        steps: vec![crate::app::state::AssistantStepView::Part {
            position: 0,
            part: ChatPartView {
                kind: crate::chat::events::PartKind::Text,
                content: "你谁".into(),
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
    let text = copy_text(
        area,
        &lx,
        ((0, 0), (0, area.width - 1)),
        &mut PaneCache::default(),
    );
    assert_eq!(text, "> 你谁");
}

#[test]
fn copy_text_excludes_scrollbar_and_extracts_offscreen_rows() {
    let area = Rect::new(0, 0, 40, 8);
    let mut lx = Lx::new();
    let mut chat = LxChat::new();
    for index in 0..15 {
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
                    content: format!("消息-{index:02}"),
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
    chat.follow_bottom = true;
    lx.chat = chat;
    // 即使当前视口钉在底部（顶部几条消息已滚出屏幕），跨屏内容行选区仍能完整提取且不含滚动条 `▐`。
    let text = copy_text(area, &lx, ((0, 0), (30, 99)), &mut PaneCache::default());
    assert!(
        text.contains("> 消息-00"),
        "滚出视口的首条消息可复制：{text}"
    );
    assert!(text.contains("> 消息-14"), "当前视口末条消息可复制：{text}");
    assert!(!text.contains('▐'), "复制结果不得包含滚动条字符：{text}");
}

#[test]
fn selection_highlight_tracks_content_scroll_and_skips_scrollbar() {
    use crate::tiling::PaneId;
    use ratatui::style::Modifier;

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
    chat.follow_bottom = false;
    chat.history_scroll = 0;
    lx.chat = chat;

    // 选中内容第 2 行（0..=100 列，覆盖整行）：scroll=0 时落在屏幕 y=2。
    let sel = Selection::text_range(PaneId::from_raw_for_test(1), (2, 0), (2, 100));
    let mut cache = PaneCache::default();
    let geom = content_geom(area, &lx, &mut cache).expect("content geom");
    let mut buf = Buffer::empty(area);
    let _ = render(area, &mut buf, &lx, false, Some(&sel), &mut cache);
    assert!(
        buf[(geom.rect.x, geom.rect.y + 2)]
            .modifier
            .contains(Modifier::REVERSED),
        "scroll=0 时内容第 2 行显示在视口第 2 行并反显"
    );
    let scrollbar_x = geom.rect.right();
    assert!(
        !buf[(scrollbar_x, geom.rect.y + 2)]
            .modifier
            .contains(Modifier::REVERSED),
        "右侧滚动条列不得被选区反显"
    );

    // 向下滚动 1 行后（scroll=1），内容第 2 行上移到视口第 1 行，高亮随之移动。
    lx.chat.history_scroll = 1;
    let mut buf2 = Buffer::empty(area);
    let _ = render(area, &mut buf2, &lx, false, Some(&sel), &mut cache);
    assert!(
        buf2[(geom.rect.x, geom.rect.y + 1)]
            .modifier
            .contains(Modifier::REVERSED),
        "scroll=1 时内容第 2 行随视口移到第 1 行并保持反显"
    );
    assert!(
        !buf2[(geom.rect.x, geom.rect.y + 2)]
            .modifier
            .contains(Modifier::REVERSED),
        "视口第 2 行已变成内容第 3 行，不再反显"
    );
}

#[test]
fn task_output_and_agents_copy_text_across_scroll_without_scrollbar() {
    use crate::app::state::{ProcessEntry, TaskPage, TaskTab};

    let area = Rect::new(0, 0, 60, 20);
    let mut lx = Lx::new();
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
    page.output = (0..30).map(|idx| format!("proc-line-{idx:02}")).collect();
    page.output_visible = 8;
    page.output_scroll = 15;
    lx.task = Some(page);

    let copied = copy_text(area, &lx, ((0, 0), (29, 99)), &mut PaneCache::default());
    assert!(
        copied.contains("proc-line-00"),
        "滚出顶部的进程输出行可复制：{copied}"
    );
    assert!(
        copied.contains("proc-line-29"),
        "底部的进程输出行可复制：{copied}"
    );
    assert!(!copied.contains('▐'), "进程输出复制不含滚动条：{copied}");

    // 切到 Agents 页签：子 agent 详情同样支持跨屏复制且不含滚动条。
    if let Some(page) = lx.task.as_mut() {
        page.tab = TaskTab::Agents;
    }
    let mut run = crate::app::state::SubagentRun::from_record(&crate::db::chat::SubagentRecord {
        agent_id: "sub-1".into(),
        nickname: None,
        agent_path: "/root/agent".into(),
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
    for idx in 0..15 {
        run.chat.messages.push(ChatMessageView {
            sequence: idx,
            id: format!("sm-{idx}"),
            role: "assistant".into(),
            status: "complete".into(),
            turn_index: idx,
            kind: "chat".into(),
            shell: None,
            steps: vec![crate::app::state::AssistantStepView::Part {
                position: 0,
                part: ChatPartView {
                    kind: crate::chat::events::PartKind::Text,
                    content: format!("agent-step-{idx:02}"),
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
    lx.agents.runs.push(run);
    let agent_copied = copy_text(area, &lx, ((0, 0), (40, 99)), &mut PaneCache::default());
    assert!(
        agent_copied.contains("agent-step-00"),
        "滚出顶部的子 agent 消息可复制：{agent_copied}"
    );
    assert!(
        agent_copied.contains("agent-step-14"),
        "底部的子 agent 消息可复制：{agent_copied}"
    );
    assert!(
        !agent_copied.contains('▐'),
        "子 agent 详情复制不含滚动条：{agent_copied}"
    );
}

#[test]
fn copy_text_blank_selection_is_empty() {
    let area = Rect::new(0, 0, 100, 20);
    assert_eq!(
        copy_text(
            area,
            &Lx::new(),
            ((0, 0), (0, 0)),
            &mut PaneCache::default()
        ),
        ""
    );
}
