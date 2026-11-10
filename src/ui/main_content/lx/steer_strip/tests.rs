//! pending steer 提示区测试：高度/隐藏条件、单行折叠与截断、`×` 命中边界、
//! 以及提示区从历史区底部扣减后的布局不变量。

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use super::*;
use crate::app::state::lx::task::TaskPage;
use crate::app::state::{
    AskState, AssistantStepView, ChatMessageView, ChatPartView, LxChat, RequestKey,
};
use crate::db::chat::ModelSnapshot;
use crate::db::chat::attachment::{MessageAttachmentInput, MessageAttachmentKind};
use crate::layout::PaneId;
use crate::ui::main_content::lx::grid::CONTINUATION;
use crate::ui::main_content::lx::{PaneCache, history_rect, history_wheel_region, layout};

/// 当前窗格的请求键。
fn request_key() -> RequestKey {
    RequestKey {
        pane: PaneId::alloc(),
        session: "session".into(),
        generation: 1,
    }
}

/// 指向 `assistant` 的流中请求状态。
fn streaming_request() -> RequestState {
    RequestState::Streaming {
        key: request_key(),
        assistant_id: "m0".into(),
    }
}

/// 一条普通 assistant 消息（只为让历史区成立）。
fn message() -> ChatMessageView {
    ChatMessageView {
        id: "m0".into(),
        sequence: 0,
        kind: "chat".into(),
        role: "assistant".into(),
        status: "streaming".into(),
        turn_index: 0,
        shell: None,
        steps: vec![AssistantStepView::Part {
            position: 0,
            part: ChatPartView {
                kind: crate::chat::events::PartKind::Text,
                content: "answer".into(),
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

/// 一条 pending steer；`attachments` 为附件数量。
fn steer(text: &str, attachments: usize) -> PendingSteer {
    PendingSteer {
        text: text.into(),
        attachments: (0..attachments)
            .map(|index| MessageAttachmentInput {
                kind: MessageAttachmentKind::File,
                label: format!("file-{index}"),
                marker_start: None,
                marker_end: None,
                source_path: None,
                mime: None,
                width: None,
                height: None,
            })
            .collect(),
        model_images: Vec::new(),
        model: ModelSnapshot {
            provider: "provider".into(),
            model: "model".into(),
            variant: None,
        },
    }
}

/// 流中 lx：一条历史消息 + `count` 条 pending。
fn streaming_lx(count: usize) -> Lx {
    let mut lx = Lx::new();
    let mut chat = LxChat::new();
    chat.messages.push(message());
    chat.request = streaming_request();
    chat.status.streaming = true;
    chat.pending_steers = (0..count)
        .map(|index| steer(&format!("steer {index}"), 0))
        .collect();
    lx.chat = chat;
    lx
}

/// 在独立网格上画提示区（整块宽度）。
fn draw_strip(lx: &Lx, width: u16) -> (Grid, Rect) {
    let rect = Rect::new(0, 0, width, height(lx));
    let mut grid = Grid::new(rect);
    draw(&mut grid, rect, lx);
    (grid, rect)
}

/// 指定行的可见文本（丢弃宽字符占位列）。
fn row_text(grid: &Grid, rect: Rect, y: u16) -> String {
    (rect.x..rect.right())
        .map(|x| grid.cell(x, y).ch)
        .filter(|ch| *ch != CONTINUATION)
        .collect()
}

#[test]
fn height_tracks_pending_count_up_to_three() {
    assert_eq!(height(&streaming_lx(0)), 0);
    assert_eq!(height(&streaming_lx(1)), 1);
    assert_eq!(height(&streaming_lx(3)), 3);
    assert_eq!(height(&streaming_lx(5)), 3, "超过 3 条不再增高");
}

#[test]
fn task_and_ask_hide_the_strip() {
    let mut task = streaming_lx(2);
    task.task = Some(TaskPage::default());
    assert_eq!(height(&task), 0);

    let mut ask = streaming_lx(2);
    ask.ask = Some(AskState::new(request_key(), "call".into(), Vec::new()));
    assert_eq!(height(&ask), 0);
}

#[test]
fn draw_renders_label_text_and_cancel() {
    let lx = streaming_lx(2);
    let (grid, rect) = draw_strip(&lx, 60);
    let first = row_text(&grid, rect, rect.y);
    let second = row_text(&grid, rect, rect.y + 1);
    assert!(first.starts_with(text::LX_STEER_LABEL), "{first:?}");
    assert!(first.contains("steer 0"), "{first:?}");
    assert!(second.contains("steer 1"), "{second:?}");
    assert!(first.ends_with('×'), "× 固定最右列：{first:?}");
    assert!(second.ends_with('×'), "× 固定最右列：{second:?}");
}

#[test]
fn more_than_three_shows_summary_row_without_cancel() {
    let lx = streaming_lx(5);
    let (grid, rect) = draw_strip(&lx, 60);
    assert_eq!(rect.height, 3);
    assert!(row_text(&grid, rect, rect.y).contains("steer 0"));
    assert!(row_text(&grid, rect, rect.y + 1).contains("steer 1"));
    let last = row_text(&grid, rect, rect.y + 2);
    assert!(last.contains("+3 more pending"), "{last:?}");
    assert!(!last.contains('×'), "汇总行不可点击、不画 ×：{last:?}");
}

#[test]
fn folded_text_collapses_whitespace_and_controls() {
    let mut lx = streaming_lx(1);
    lx.chat.pending_steers[0].text = "a\n\tb\u{7}c   d".into();
    let (grid, rect) = draw_strip(&lx, 60);
    let row = row_text(&grid, rect, rect.y);
    assert!(row.contains("a b c d"), "{row:?}");
    assert!(!row.contains('\n') && !row.contains('\t'), "{row:?}");
}

#[test]
fn attachments_suffix_reports_count() {
    let mut lx = streaming_lx(1);
    lx.chat.pending_steers[0] = steer("hello", 2);
    let (grid, rect) = draw_strip(&lx, 60);
    let row = row_text(&grid, rect, rect.y);
    assert!(row.contains("2 attachments"), "{row:?}");
    assert!(row.contains('·'), "{row:?}");
    assert!(
        row.contains("hello  · 2 attachments"),
        "附件后缀紧跟正文：{row:?}"
    );
}

#[test]
fn narrow_strip_keeps_cancel_and_hides_text() {
    let lx = streaming_lx(1);
    // 标签 8 列 + 空档 2 列 + ×；10 列放得下标签与 ×，但正文没有预算。
    let (grid, rect) = draw_strip(&lx, 10);
    let row = row_text(&grid, rect, rect.y);
    assert!(row.starts_with("Steering"), "{row:?}");
    assert!(row.ends_with('×'), "{row:?}");
    assert!(!row.contains("steer"), "宽度不足时正文整体省略：{row:?}");
}

#[test]
fn wide_chars_are_never_half_written() {
    let mut lx = streaming_lx(1);
    lx.chat.pending_steers[0].text = "中文中文中文中文".into();
    let (grid, rect) = draw_strip(&lx, 20);
    let row = row_text(&grid, rect, rect.y);
    assert!(row.contains('…'), "超宽正文以省略号收尾：{row:?}");
    assert!(row.ends_with('×'), "{row:?}");
    // 省略号前一个可见字符必须是完整汉字，而不是宽字符占位列。
    assert!(
        row.chars()
            .rev()
            .nth(1)
            .is_some_and(|ch| ch != CONTINUATION),
        "{row:?}"
    );

    // 正文预算只有 1 列：一个汉字都放不下时只画省略号，绝不半写。
    let mut narrow = streaming_lx(1);
    narrow.chat.pending_steers[0].text = "中".into();
    let (grid, rect) = draw_strip(&narrow, 13);
    let row = row_text(&grid, rect, rect.y);
    assert!(row.contains('…') && !row.contains('中'), "{row:?}");
}

#[test]
fn recall_hit_only_matches_the_cancel_column() {
    let area = Rect::new(0, 0, 60, 16);
    let lx = streaming_lx(2);
    let rect = layout(area, &lx).pending.expect("pending strip");
    assert_eq!(recall_hit(area, &lx, rect.right() - 1, rect.y), Some(0));
    assert_eq!(recall_hit(area, &lx, rect.right() - 1, rect.y + 1), Some(1));
    assert_eq!(
        recall_hit(area, &lx, rect.right() - 2, rect.y),
        None,
        "仅 × 单列命中"
    );
    assert_eq!(
        recall_hit(area, &lx, rect.right() - 1, rect.bottom()),
        None,
        "行边界之外"
    );
}

#[test]
fn recall_hit_skips_the_more_row_and_requires_streaming() {
    let area = Rect::new(0, 0, 60, 16);
    let lx = streaming_lx(5);
    let rect = layout(area, &lx).pending.expect("pending strip");
    assert_eq!(recall_hit(area, &lx, rect.right() - 1, rect.y), Some(0));
    assert_eq!(recall_hit(area, &lx, rect.right() - 1, rect.y + 1), Some(1));
    assert_eq!(
        recall_hit(area, &lx, rect.right() - 1, rect.y + 2),
        None,
        "汇总行不可撤回"
    );

    let mut idle = streaming_lx(2);
    idle.chat.request = RequestState::Idle;
    assert_eq!(
        recall_hit(area, &idle, 59, rect.y),
        None,
        "非 Streaming 一律不命中"
    );
}

#[test]
fn layout_reserves_pending_rows_above_chat_bar() {
    let area = Rect::new(0, 0, 60, 16);
    let lx = streaming_lx(2);
    let lay = layout(area, &lx);
    let input = lay.input.expect("input");
    let bar = lay.chat_bar.expect("流中聊天栏");
    let pending = lay.pending.expect("pending strip");
    assert_eq!(pending.height, 2);
    assert_eq!(pending.bottom(), bar.y, "提示区贴在聊天栏上沿");
    assert_eq!(bar.bottom(), input.y, "聊天栏仍贴输入框上沿");
    assert_eq!(pending.x, input.x);
    assert_eq!(pending.width, input.width);

    let history = history_rect(area, &lx).expect("history");
    assert_eq!(history.bottom(), pending.y, "历史区扣掉提示区");
    let wheel = history_wheel_region(area, &lx).expect("wheel region");
    assert_eq!(wheel.bottom(), input.y, "滚轮区仍含提示区与聊天栏");
}

#[test]
fn chat_page_renders_the_strip_above_the_chat_bar() {
    let area = Rect::new(0, 0, 60, 16);
    let lx = streaming_lx(1);
    let mut buf = Buffer::empty(area);
    let _ = super::super::render(area, &mut buf, &lx, false, None, &mut PaneCache::default());
    let screen: String = (area.y..area.bottom())
        .map(|y| {
            (area.x..area.right())
                .map(|x| buf[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(screen.contains(text::LX_STEER_LABEL), "{screen}");
    assert!(screen.contains("steer 0"), "{screen}");
}

#[test]
fn layout_hides_pending_when_task_or_ask_is_open() {
    let area = Rect::new(0, 0, 60, 16);
    let mut task = streaming_lx(2);
    task.task = Some(TaskPage::default());
    assert!(layout(area, &task).pending.is_none());

    let mut ask = streaming_lx(2);
    ask.ask = Some(AskState::new(request_key(), "call".into(), Vec::new()));
    assert!(layout(area, &ask).pending.is_none());
}

#[test]
fn steered_turn_hides_previous_assistant_stats_and_tints_steered_user_bg() {
    use crate::app::state::{AssistantStepView, ChatPartView};
    use crate::chat::events::PartKind;

    let area = Rect::new(0, 0, 60, 18);
    let mut lx = Lx::new();

    let mut user1 = message();
    user1.id = "u1".into();
    user1.sequence = 0;
    user1.role = "user".into();
    user1.steps = vec![AssistantStepView::Part {
        position: 0,
        part: ChatPartView {
            kind: PartKind::Text,
            content: "first question".into(),
            provider_metadata_json: None,
            started_at_ms: None,
            ended_at_ms: None,
            completed_at_ms: None,
        },
    }];

    // 被 steer 截断的旧 assistant：即便带有 model_label 与 duration_ms，也不得在底部显示统计行。
    let mut assistant1 = message();
    assistant1.id = "a1".into();
    assistant1.sequence = 1;
    assistant1.role = "assistant".into();
    assistant1.status = "complete".into();
    assistant1.model_label = Some("deepseek-v4.1-flash".into());
    assistant1.duration_ms = Some(3_900);
    assistant1.steps = vec![AssistantStepView::Part {
        position: 0,
        part: ChatPartView {
            kind: PartKind::Text,
            content: "partial answer".into(),
            provider_metadata_json: None,
            started_at_ms: None,
            ended_at_ms: None,
            completed_at_ms: None,
        },
    }];

    let mut steered_user = message();
    steered_user.id = "u2-steer".into();
    steered_user.sequence = 2;
    steered_user.turn_index = 1;
    steered_user.role = "user".into();
    steered_user.steps = vec![AssistantStepView::Part {
        position: 0,
        part: ChatPartView {
            kind: PartKind::Text,
            content: "steered question".into(),
            provider_metadata_json: None,
            started_at_ms: None,
            ended_at_ms: None,
            completed_at_ms: None,
        },
    }];

    let mut assistant2 = message();
    assistant2.id = "a2".into();
    assistant2.sequence = 3;
    assistant2.turn_index = 1;
    assistant2.role = "assistant".into();
    assistant2.status = "complete".into();
    assistant2.model_label = Some("deepseek-v4.1-flash".into());
    assistant2.duration_ms = Some(8_100);
    assistant2.steps = vec![AssistantStepView::Part {
        position: 0,
        part: ChatPartView {
            kind: PartKind::Text,
            content: "final answer".into(),
            provider_metadata_json: None,
            started_at_ms: None,
            ended_at_ms: None,
            completed_at_ms: None,
        },
    }];

    lx.chat.messages = vec![user1, assistant1, steered_user.clone(), assistant2];
    lx.chat.mark_steered_user_message("u2-steer");

    let mut buf = Buffer::empty(area);
    let _ = super::super::render(area, &mut buf, &lx, false, None, &mut PaneCache::default());
    let screen: String = (area.y..area.bottom())
        .map(|y| {
            (area.x..area.right())
                .map(|x| buf[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !screen.contains("3.9s"),
        "被 steer 截断的旧 assistant 底部不得出现 3.9s 统计行:\n{screen}"
    );
    assert!(
        screen.contains("8.1s"),
        "最终完成段仍正常显示底部统计行:\n{screen}"
    );

    let first_user_y = (area.y..area.bottom())
        .find(|&y| {
            let line: String = (area.x..area.right())
                .map(|x| buf[(x, y)].symbol())
                .collect();
            line.contains("first question")
        })
        .expect("first user row");
    let steered_user_y = (area.y..area.bottom())
        .find(|&y| {
            let line: String = (area.x..area.right())
                .map(|x| buf[(x, y)].symbol())
                .collect();
            line.contains("steered question")
        })
        .expect("steered user row");
    assert_eq!(
        buf[(area.x + 2, first_user_y)].bg,
        style::chat_user_bg().bg.expect("normal bg"),
        "普通用户消息使用 chat_user_bg"
    );
    assert_eq!(
        buf[(area.x + 2, steered_user_y)].bg,
        style::chat_user_steer_bg().bg.expect("steer bg"),
        "steer 插入的用户消息使用区分底色 chat_user_steer_bg"
    );

    // 置顶行同样同步 steer 底色。
    let sticky_row = super::super::history::StickyRow {
        rect: Rect::new(0, 0, 40, 1),
        message: 2,
    };
    let mut sticky_buf = Buffer::empty(sticky_row.rect);
    super::super::history::render_sticky_prompt_with_steer(
        &mut sticky_buf,
        sticky_row,
        &steered_user,
        true,
    );
    assert_eq!(
        sticky_buf[(2, 0)].bg,
        style::chat_user_steer_bg().bg.expect("steer sticky bg")
    );
}
