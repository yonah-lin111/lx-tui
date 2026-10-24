//! compact 状态的纯测试：默认折叠、展开保留、替换与重置。

use super::*;
use crate::db::chat::{COMPACT_ACTIVE, COMPACT_REVERTED};

fn checkpoint(id: &str, sequence: i64, active: bool) -> CompactNoticeView {
    CompactNoticeView {
        id: id.into(),
        sequence,
        model_label: "gpt-5".into(),
        created_at_ms: 0,
        start_sequence: 0,
        end_sequence: sequence.saturating_sub(1),
        summary: "sum".into(),
        active,
    }
}

fn record(id: &str, sequence: i64, state: &str) -> CompactCheckpoint {
    CompactCheckpoint {
        id: id.into(),
        sequence,
        operation_id: format!("op-{id}"),
        base_checkpoint_id: None,
        start_sequence: 0,
        end_sequence: 3,
        summary_text: "summary".into(),
        model_label: "gpt-5".into(),
        state: state.into(),
        created_at_ms: 1_700_000_000_000,
    }
}

/// 从持久化记录构造：状态映射为 `active` 布尔，范围可读。
#[test]
fn from_record_maps_state_and_range() {
    let view = CompactNoticeView::from_record(&record("c1", 4, COMPACT_ACTIVE));
    assert!(view.active);
    assert_eq!(view.range_label(), "sequence 0–3");
    let reverted = CompactNoticeView::from_record(&record("c2", 6, COMPACT_REVERTED));
    assert!(!reverted.active);
    let single = CompactNoticeView::from_record(&CompactCheckpoint {
        start_sequence: 4,
        end_sequence: 4,
        ..record("c3", 4, COMPACT_ACTIVE)
    });
    assert_eq!(single.range_label(), "sequence 4");
}

/// 默认折叠：新状态里没有任何展开项；切换后可在集合中往返。
#[test]
fn expansion_defaults_to_collapsed() {
    let mut compact = LxCompact::default();
    compact.replace(vec![checkpoint("c1", 4, true)]);
    assert!(!compact.is_expanded("c1"), "默认必须折叠");
    assert!(compact.toggle("c1"));
    assert!(compact.is_expanded("c1"));
    assert!(!compact.toggle("c1"));
    assert!(!compact.is_expanded("c1"));
}

/// 重载后仍存在的展开项保留；消失的条目被丢弃（不泄漏内存/不错展开）。
#[test]
fn replace_keeps_expansion_for_surviving_ids() {
    let mut compact = LxCompact::default();
    compact.replace(vec![checkpoint("c1", 4, true), checkpoint("c2", 6, true)]);
    compact.toggle("c1");
    compact.toggle("c2");
    compact.replace(vec![checkpoint("c2", 6, true)]);
    assert!(!compact.is_expanded("c1"));
    assert!(compact.is_expanded("c2"));
}

/// 最新 active 检查点就是上下文起点；已撤销的不算。
#[test]
fn active_is_the_latest_active_checkpoint() {
    let mut compact = LxCompact::default();
    assert!(compact.active().is_none());
    compact.replace(vec![
        checkpoint("c1", 4, true),
        checkpoint("c2", 6, false),
        checkpoint("c3", 8, true),
    ]);
    assert_eq!(compact.active().map(|item| item.id.as_str()), Some("c3"));
}

/// 忙碌标记：重置会话时清掉，避免新会话继承“压缩中”。
#[test]
fn reset_clears_running_and_entries() {
    let mut compact = LxCompact::default();
    compact.replace(vec![checkpoint("c1", 4, true)]);
    compact.running = Some("op".into());
    assert!(compact.is_running());
    compact.reset();
    assert!(!compact.is_running());
    assert!(compact.is_empty());
}
