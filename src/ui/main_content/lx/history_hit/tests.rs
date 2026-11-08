use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use crate::app::state::{
    AssistantStepView, ChatMessageView, ChatPartView, Lx, LxChat, SubagentRun, TaskPage, TaskTab,
};
use crate::chat::events::PartKind;
use crate::ui::PaneCache;
use crate::ui::main_content::lx::{LxChatHit, chat_hit, render};

fn find_copy_button_positions(buf: &Buffer, area: Rect) -> Vec<(u16, u16)> {
    let mut positions = Vec::new();
    for y in area.y..area.bottom() {
        for x in area.x..area.right().saturating_sub(2) {
            if buf[(x, y)].symbol() == "["
                && buf[(x + 1, y)].symbol() == "❐"
                && buf[(x + 2, y)].symbol() == "]"
            {
                positions.push((x + 1, y));
            }
        }
    }
    positions
}

fn find_copy_button_pos(buf: &Buffer, area: Rect) -> Option<(u16, u16)> {
    find_copy_button_positions(buf, area).into_iter().next()
}

fn assistant_msg(id: &str, content: &str) -> ChatMessageView {
    ChatMessageView {
        sequence: 1,
        id: id.into(),
        role: "assistant".into(),
        status: "complete".into(),
        turn_index: 0,
        kind: "chat".into(),
        shell: None,
        steps: vec![AssistantStepView::Part {
            position: 0,
            part: ChatPartView {
                kind: PartKind::Text,
                content: content.into(),
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
    }
}

#[test]
fn chat_hit_returns_copy_code_on_assistant_code_block_button_and_misses_outside() {
    let area = Rect::new(0, 0, 50, 18);
    let mut lx = Lx::new();
    let mut chat = LxChat::new();
    chat.messages.push(assistant_msg(
        "a-1",
        "```rust\nfn demo() -> usize {\n    42\n}\n```",
    ));
    lx.chat = chat;

    let mut cache = PaneCache::default();
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &lx, true, None, &mut cache);

    let (btn_x, btn_y) = find_copy_button_pos(&buf, area).expect("copy button in buffer");
    assert_eq!(
        chat_hit(area, &lx, btn_x, btn_y, &mut cache),
        Some(LxChatHit::CopyCode(
            "fn demo() -> usize {\n    42\n}".into()
        )),
        "点击 `[❐]` 应返回未折行的原始代码块内容"
    );
    // 按钮左边缘 `[` 与右边缘 `]` 命中，右侧 1 列空白不命中。
    assert_eq!(
        chat_hit(area, &lx, btn_x - 1, btn_y, &mut cache),
        Some(LxChatHit::CopyCode(
            "fn demo() -> usize {\n    42\n}".into()
        ))
    );
    assert_eq!(
        chat_hit(area, &lx, btn_x + 1, btn_y, &mut cache),
        Some(LxChatHit::CopyCode(
            "fn demo() -> usize {\n    42\n}".into()
        ))
    );
    assert_eq!(
        chat_hit(area, &lx, btn_x + 2, btn_y, &mut cache),
        None,
        "按钮右侧 1 列空白不得命中复制"
    );
    // 同一行左侧语言标签 `rust` 位置不应触发复制。
    assert_eq!(
        chat_hit(area, &lx, area.x + 4, btn_y, &mut cache),
        None,
        "点击同行的语言名区域不应命中复制按钮"
    );
}

#[test]
fn chat_hit_returns_raw_unwrapped_code_for_soft_wrapped_block() {
    let area = Rect::new(0, 0, 40, 20);
    let raw = "fn long() {\n    let value = aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa;\n\n    done();\n}";
    let mut lx = Lx::new();
    let mut chat = LxChat::new();
    chat.messages
        .push(assistant_msg("a-wrap", &format!("```rust\n{raw}\n```")));
    lx.chat = chat;

    let mut cache = PaneCache::default();
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &lx, true, None, &mut cache);

    let (btn_x, btn_y) = find_copy_button_pos(&buf, area).expect("copy button in buffer");
    assert_eq!(
        chat_hit(area, &lx, btn_x, btn_y, &mut cache),
        Some(LxChatHit::CopyCode(raw.into())),
        "窄宽度软折行后仍必须复制原始未折行代码"
    );
}

#[test]
fn chat_hit_copies_second_code_block_by_its_own_index() {
    let area = Rect::new(0, 0, 50, 20);
    let mut lx = Lx::new();
    let mut chat = LxChat::new();
    chat.messages.push(assistant_msg(
        "a-multi",
        "```rust\nfirst();\n```\n\n```python\nsecond();\n```",
    ));
    lx.chat = chat;

    let mut cache = PaneCache::default();
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &lx, true, None, &mut cache);
    let buttons = find_copy_button_positions(&buf, area);
    assert_eq!(buttons.len(), 2, "两条代码块各有一个复制按钮");
    let (btn_x, btn_y) = buttons[1];
    assert_eq!(
        chat_hit(area, &lx, btn_x, btn_y, &mut cache),
        Some(LxChatHit::CopyCode("second();".into())),
        "第二个代码块必须按 block_index=1 提取"
    );
}

#[test]
fn chat_hit_uses_scrolled_viewport_for_code_button() {
    let area = Rect::new(0, 0, 40, 14);
    let mut lx = Lx::new();
    let mut chat = LxChat::new();
    let mut content = "line\n".repeat(20);
    content.push_str("```rust\nscrolled();\n```");
    chat.messages.push(assistant_msg("a-scroll", &content));
    lx.chat = chat;

    let mut cache = PaneCache::default();
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &lx, true, None, &mut cache);
    let (btn_x, btn_y) = find_copy_button_pos(&buf, area).expect("scrolled copy button");
    assert_eq!(
        chat_hit(area, &lx, btn_x, btn_y, &mut cache),
        Some(LxChatHit::CopyCode("scrolled();".into())),
        "滚动后按钮命中必须使用当前 view.scroll"
    );
}

#[test]
fn chat_hit_does_not_copy_stale_history_while_session_is_loading() {
    let area = Rect::new(0, 0, 50, 18);
    let mut lx = Lx::new();
    let mut chat = LxChat::new();
    chat.messages
        .push(assistant_msg("a-loading", "```rust\nlet hidden = 1;\n```"));
    lx.chat = chat;

    let mut cache = PaneCache::default();
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &lx, true, None, &mut cache);
    let (btn_x, btn_y) = find_copy_button_pos(&buf, area).expect("copy button");

    lx.chat.loading_session = true;
    assert_eq!(
        chat_hit(area, &lx, btn_x, btn_y, &mut cache),
        None,
        "恢复会话加载中只画 loading 占位，旧历史按钮不得穿透命中"
    );
}

#[test]
fn chat_hit_on_sticky_prompt_header_does_not_fall_through_to_history() {
    let area = Rect::new(0, 0, 50, 14);
    let mut lx = Lx::new();
    let mut chat = LxChat::new();
    chat.messages.push(ChatMessageView {
        sequence: 0,
        id: "u-0".into(),
        role: "user".into(),
        status: "complete".into(),
        turn_index: 0,
        kind: "chat".into(),
        shell: None,
        steps: vec![AssistantStepView::Part {
            position: 0,
            part: ChatPartView {
                kind: PartKind::Text,
                content: "User question".into(),
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
    chat.messages.push(assistant_msg(
        "a-1",
        "```rust\nline_1();\nline_2();\nline_3();\nline_4();\nline_5();\nline_6();\nline_7();\nline_8();\nline_9();\n```",
    ));
    chat.follow_bottom = false;
    chat.history_scroll = 2;
    lx.chat = chat;

    let mut cache = PaneCache::default();
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &lx, true, None, &mut cache);

    // 顶行第 0 行在置顶条覆盖范围内，任何列点击都应命中 PromptHeader 而非穿透到代码块 `[❐]`。
    assert_eq!(
        chat_hit(area, &lx, area.right() - 3, area.y, &mut cache),
        Some(LxChatHit::PromptHeader)
    );
}

#[test]
fn chat_hit_supports_code_copy_button_in_task_agents_detail_view() {
    let area = Rect::new(0, 0, 60, 22);
    let mut lx = Lx::new();
    let mut run = SubagentRun::from_record(&crate::db::chat::SubagentRecord {
        agent_id: "ag-1".into(),
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
    run.chat.messages.push(assistant_msg(
        "sub-a-1",
        "```python\nprint('from subagent')\n```",
    ));
    lx.agents.runs.push(run);
    lx.task = Some(TaskPage {
        tab: TaskTab::Agents,
        ..Default::default()
    });

    let mut cache = PaneCache::default();
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &lx, true, None, &mut cache);

    let (btn_x, btn_y) = find_copy_button_pos(&buf, area).expect("subagent copy button in buffer");
    assert_eq!(
        chat_hit(area, &lx, btn_x, btn_y, &mut cache),
        Some(LxChatHit::CopyCode("print('from subagent')".into()))
    );
}
