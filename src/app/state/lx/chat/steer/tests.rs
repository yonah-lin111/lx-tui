use super::*;
use crate::db::chat::{MessageAttachmentInput, MessageAttachmentKind};

fn model() -> ModelSnapshot {
    ModelSnapshot {
        provider: "p".into(),
        model: "m".into(),
        variant: None,
    }
}

fn pending(text: &str) -> PendingSteer {
    PendingSteer {
        text: text.into(),
        attachments: Vec::new(),
        model_images: Vec::new(),
        model: model(),
    }
}

fn image(bytes: &[u8]) -> ModelImage {
    ModelImage {
        mime: "image/png".into(),
        bytes: std::sync::Arc::new(bytes.to_vec()),
    }
}

#[test]
fn build_batch_merges_text_in_order() {
    let batch = build_batch(&[pending("a"), pending("b"), pending("c")]).expect("batch");
    assert_eq!(batch.text, "a\n\nb\n\nc");
    assert_eq!(batch.consumed, 3);
}

#[test]
fn build_batch_shifts_attachment_markers() {
    let mut first = pending("one @a");
    first.attachments.push(MessageAttachmentInput {
        kind: MessageAttachmentKind::File,
        label: "a".into(),
        marker_start: Some(4),
        marker_end: Some(6),
        source_path: Some("a".into()),
        mime: None,
        width: None,
        height: None,
    });
    let mut second = pending("two @b");
    second.attachments.push(MessageAttachmentInput {
        kind: MessageAttachmentKind::File,
        label: "b".into(),
        marker_start: Some(4),
        marker_end: Some(6),
        source_path: Some("b".into()),
        mime: None,
        width: None,
        height: None,
    });
    let batch = build_batch(&[first, second]).expect("batch");
    assert_eq!(batch.text, "one @a\n\ntwo @b");
    assert_eq!(batch.attachments[0].marker_start, Some(4));
    assert_eq!(batch.attachments[1].marker_start, Some(12));
}

#[test]
fn build_batch_stops_at_image_limit() {
    let mut items = Vec::new();
    for index in 0..5 {
        let mut item = pending(&format!("i{index}"));
        item.model_images.push(image(&[index as u8]));
        items.push(item);
    }
    let batch = build_batch(&items).expect("batch");
    assert_eq!(batch.consumed, 4);
    assert_eq!(batch.model_images.len(), 4);
}

#[test]
fn build_batch_dedupes_images() {
    let mut first = pending("a");
    first.model_images.push(image(b"same"));
    let mut second = pending("b");
    second.model_images.push(image(b"same"));
    let batch = build_batch(&[first, second]).expect("batch");
    assert_eq!(batch.model_images.len(), 1);
}

#[test]
fn build_batch_stops_at_text_limit() {
    let mut items = vec![pending("a")];
    let mut huge = pending("");
    huge.text = "x".repeat(MAX_MESSAGE_BYTES);
    items.push(huge);
    let batch = build_batch(&items).expect("batch");
    assert_eq!(batch.consumed, 1);
}

#[test]
fn build_batch_empty_returns_none() {
    assert!(build_batch(&[]).is_none());
}

#[test]
fn build_batch_deduplicates_images_before_checking_max_attached_images() {
    let mut first = pending("first");
    first.model_images = vec![image(b"a"), image(b"b"), image(b"c")];
    let mut second = pending("second");
    // 含 1 张重复图 b"a" + 1 张新图 b"d"，去重后共 4 张（未超 4 张上限）。
    second.model_images = vec![image(b"a"), image(b"d")];
    let batch = build_batch(&[first, second]).expect("batch");
    assert_eq!(batch.consumed, 2);
    assert_eq!(batch.model_images.len(), 4);
}
