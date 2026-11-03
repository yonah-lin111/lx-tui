//! `LxChat` 上下文占用：单步分帧合并与 `/compact` 边界过滤。

use super::*;
use crate::app::state::CompactNoticeView;

fn message(sequence: i64, role: &str, input: Option<i64>, output: Option<i64>) -> ChatMessageView {
    ChatMessageView {
        id: format!("m{sequence}"),
        sequence,
        kind: "chat".into(),
        role: role.into(),
        status: "complete".into(),
        turn_index: 0,
        shell: None,
        steps: Vec::new(),
        model_label: None,
        error_code: None,
        input_tokens: input,
        output_tokens: output,
        cache_read_tokens: None,
        duration_ms: None,
        reports: Vec::new(),
        attachments: Vec::new(),
    }
}

fn checkpoint(end_sequence: i64, active: bool) -> CompactNoticeView {
    CompactNoticeView {
        id: "c1".into(),
        sequence: end_sequence,
        model_label: "m".into(),
        created_at_ms: 0,
        start_sequence: 0,
        end_sequence,
        summary: "sum".into(),
        active,
        auto: false,
    }
}

/// 分帧到达的 input / output 在同一采样步内合并，下一步从空槽重新开始。
#[test]
fn absorb_usage_delta_merges_split_frames() {
    let mut chat = LxChat::new();
    chat.absorb_usage_delta(&Usage {
        input: Some(100),
        cache_read: Some(40),
        ..Default::default()
    });
    assert_eq!(
        chat.context_tokens,
        Some(100),
        "只有 input 时 output 按 0 计"
    );
    chat.absorb_usage_delta(&Usage {
        output: Some(20),
        ..Default::default()
    });
    assert_eq!(chat.context_tokens, Some(120), "同一步 input + output");
    // 下一步只报 input：不得混入上一步 output。
    chat.reset_step_usage();
    chat.absorb_usage_delta(&Usage {
        input: Some(300),
        ..Default::default()
    });
    assert_eq!(chat.context_tokens, Some(300));
}

/// 只取生效压缩边界之后的最后一条带用量 assistant 消息。
#[test]
fn recompute_uses_last_assistant_after_boundary() {
    let mut chat = LxChat::new();
    chat.messages = vec![
        message(0, "user", None, None),
        message(1, "assistant", Some(100), Some(10)),
        message(2, "user", None, None),
        message(3, "assistant", Some(200), Some(20)),
    ];
    let mut compact = crate::app::state::LxCompact::default();
    compact.replace(vec![checkpoint(1, true)]);
    chat.recompute_context_tokens(&compact);
    assert_eq!(chat.context_tokens, Some(220), "边界后最后一条 assistant");
}

/// 压缩边界覆盖全部消息：尚无新回复时清空占用（隐藏旧百分比）。
#[test]
fn recompute_clears_when_boundary_covers_every_message() {
    let mut chat = LxChat::new();
    chat.messages = vec![
        message(0, "user", None, None),
        message(1, "assistant", Some(100), Some(10)),
    ];
    let mut compact = crate::app::state::LxCompact::default();
    compact.replace(vec![checkpoint(1, true)]);
    chat.recompute_context_tokens(&compact);
    assert_eq!(chat.context_tokens, None);
}

/// 已撤销的检查点不构成边界：撤销后恢复压缩前的最后一条回复。
#[test]
fn recompute_ignores_reverted_checkpoint() {
    let mut chat = LxChat::new();
    chat.messages = vec![
        message(0, "user", None, None),
        message(1, "assistant", Some(100), Some(10)),
    ];
    let mut compact = crate::app::state::LxCompact::default();
    compact.replace(vec![checkpoint(1, false)]);
    chat.recompute_context_tokens(&compact);
    assert_eq!(chat.context_tokens, Some(110));
}

/// 无生效检查点时按全部消息重算。
#[test]
fn recompute_without_checkpoint_uses_latest_assistant() {
    let mut chat = LxChat::new();
    chat.messages = vec![
        message(0, "user", None, None),
        message(1, "assistant", Some(100), Some(10)),
    ];
    chat.recompute_context_tokens(&crate::app::state::LxCompact::default());
    assert_eq!(chat.context_tokens, Some(110));
}
