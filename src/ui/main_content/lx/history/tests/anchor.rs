//! 块级阅读锚点：内容在上方变高/消失时，锚点行按规则位移或回退。

use super::*;
use crate::ui::main_content::lx::history::anchor::AnchorEntry;
use crate::ui::main_content::lx::history::snapshot;

fn with_id(id: &str, role: &str, parts: Vec<ChatPartView>) -> ChatMessageView {
    let mut message = message(role, "complete", parts);
    message.id = id.into();
    message
}

fn build(lx: &Lx, cache: &mut PaneCache) -> HistorySnapshot {
    snapshot::build(30, HistorySource::from(lx), cache)
}

#[test]
fn anchor_block_stays_after_above_growth() {
    let mut lx = lx_with(vec![
        with_id("u0", "user", vec![text_part("0")]),
        with_id("a1", "assistant", vec![text_part("middle")]),
        with_id("a2", "assistant", vec![text_part("bottom")]),
    ]);
    let mut cache = PaneCache::default();
    let first = build(&lx, &mut cache);
    let scroll = first.messages[1].start + 1;
    let anchor = anchor_at(&first, scroll, 30).expect("锚点");
    let old_row = anchor.row;
    assert_eq!(anchor.block, 0, "单步消息的锚点落在第一个块");

    // 上方用户消息从 1 行变 6 行：锚点块整体下移 5 行。
    lx.chat.messages[0] = with_id("u0", "user", vec![text_part("0\n1\n2\n3\n4\n5")]);
    let second = build(&lx, &mut cache);
    let new_row = anchor_row(&second, HistorySource::from(&lx), &anchor, 30).expect("新行");
    assert_eq!(new_row, old_row + 5, "锚点块应随上方内容同步下移");
    assert_eq!(
        second.messages[1].start,
        first.messages[1].start + 5,
        "消息起始行同步下移"
    );
}

#[test]
fn anchor_falls_back_to_message_start_when_block_gone() {
    let lx = lx_with(vec![
        with_id("u0", "user", vec![text_part("0")]),
        with_id("a1", "assistant", vec![text_part("middle")]),
    ]);
    let mut cache = PaneCache::default();
    let snapshot = build(&lx, &mut cache);
    let scroll = snapshot.messages[1].start;
    let mut anchor = anchor_at(&snapshot, scroll, 30).expect("锚点");

    // 块身份与序号同时失配，才会走「回退消息起始行」。
    anchor.block = usize::MAX;
    anchor.source = super::BlockSource::Notice { message_index: 1 };
    let row = anchor_row(&snapshot, HistorySource::from(&lx), &anchor, 30).expect("回退行");
    let entry = snapshot
        .entries
        .iter()
        .find(|entry| matches!(entry.kind, SnapEntryKind::Message(1)))
        .expect("消息条目");
    assert_eq!(row, entry.start, "块消失时回退到消息起始行");
}

#[test]
fn anchor_returns_none_when_message_gone() {
    let mut lx = lx_with(vec![
        with_id("u0", "user", vec![text_part("0")]),
        with_id("a1", "assistant", vec![text_part("middle")]),
        with_id("a2", "assistant", vec![text_part("bottom")]),
    ]);
    let mut cache = PaneCache::default();
    let first = build(&lx, &mut cache);
    let scroll = first.messages[1].start;
    let anchor = anchor_at(&first, scroll, 30).expect("锚点");

    lx.chat.messages.remove(1);
    let second = build(&lx, &mut cache);
    assert!(
        anchor_row(&second, HistorySource::from(&lx), &anchor, 30).is_none(),
        "消息被撤销/压缩删除后不能再拿旧块定位"
    );
}

#[test]
fn anchor_dropped_on_width_change() {
    let lx = lx_with(vec![with_id("a1", "assistant", vec![text_part("middle")])]);
    let mut cache = PaneCache::default();
    let snapshot = build(&lx, &mut cache);
    let anchor = anchor_at(&snapshot, 0, 30).expect("锚点");
    assert!(
        anchor_row(&snapshot, HistorySource::from(&lx), &anchor, 31).is_none(),
        "宽度变化后行号重新钳制，不能按旧几何补偿"
    );
}

#[test]
fn anchor_at_clamps_beyond_last_block() {
    let lx = lx_with(vec![
        with_id("u0", "user", vec![text_part("0")]),
        with_id("a1", "assistant", vec![text_part("middle")]),
    ]);
    let mut cache = PaneCache::default();
    let snapshot = build(&lx, &mut cache);
    let anchor = anchor_at(&snapshot, snapshot.total_rows + 100, 30).expect("末尾呼吸行也要有锚点");
    assert!(anchor.row < snapshot.total_rows, "行号必须钳在内容内");
    assert_eq!(
        anchor.entry,
        AnchorEntry::Message {
            index: 1,
            id: "a1".into()
        }
    );
}
