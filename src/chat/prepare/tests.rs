use std::path::PathBuf;

use super::*;
use crate::platform::process_image_bytes;

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("lx-test-prep-{}-{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create dir");
    dir
}

fn create_png_bytes(w: u32, h: u32) -> Vec<u8> {
    let img: image::ImageBuffer<image::Rgba<u8>, Vec<u8>> =
        image::ImageBuffer::from_fn(w, h, |_, _| image::Rgba([100, 150, 200, 255]));
    let mut bytes = Vec::new();
    img.write_to(
        &mut std::io::Cursor::new(&mut bytes),
        image::ImageFormat::Png,
    )
    .expect("write png");
    bytes
}

#[test]
fn prepare_with_direct_attachment_and_text() {
    let dir = temp_dir("direct");
    let png = create_png_bytes(10, 10);
    let processed = process_image_bytes(&png, "pasted.png", None).expect("process");
    let drafts = vec![DraftImageAttachment {
        id: "d1".into(),
        image: processed,
    }];

    let prepared = prepare_user_message(&dir, "hello world", &drafts).expect("prepare");
    assert_eq!(prepared.raw_text, "hello world");
    assert_eq!(prepared.attachments.len(), 1);
    assert_eq!(prepared.model_images.len(), 1);
    assert_eq!(prepared.attachments[0].kind, MessageAttachmentKind::Image);
    assert_eq!(prepared.attachments[0].label, "pasted.png");
}

#[test]
fn prepare_with_explicit_image_reference() {
    let dir = temp_dir("explicit");
    let img_path = dir.join("shot.png");
    let png = create_png_bytes(20, 20);
    std::fs::write(&img_path, &png).expect("write file");

    let text = "Look at @[refer-image](shot.png) please";
    let prepared = prepare_user_message(&dir, text, &[]).expect("prepare");
    assert_eq!(prepared.attachments.len(), 1);
    assert_eq!(prepared.model_images.len(), 1);
    assert_eq!(prepared.attachments[0].kind, MessageAttachmentKind::Image);
    assert_eq!(prepared.attachments[0].label, "shot.png");
    assert!(prepared.attachments[0].marker_start.is_some());
}

#[test]
fn prepare_fails_on_missing_explicit_image() {
    let dir = temp_dir("missing");
    let text = "Look at @[refer-image](missing.png)";
    let err = prepare_user_message(&dir, text, &[]).unwrap_err();
    assert_eq!(err, ImagePrepareError::FileNotFound);
}

#[test]
fn prepare_skips_mentions_inside_code_fences() {
    let dir = temp_dir("fences");
    let img_path = dir.join("valid.png");
    std::fs::write(&img_path, create_png_bytes(10, 10)).expect("write");

    let text = "```\n@[refer-image](valid.png)\n@valid.png\n```\noutside @valid.png ";
    let prepared = prepare_user_message(&dir, text, &[]).expect("prepare");
    assert_eq!(prepared.attachments.len(), 1);
    assert_eq!(prepared.model_images.len(), 1);
}

#[test]
fn prepare_deduplicates_same_image() {
    let dir = temp_dir("dedup");
    let img_path = dir.join("same.png");
    let png = create_png_bytes(10, 10);
    std::fs::write(&img_path, &png).expect("write");

    let processed = process_image_bytes(&png, "same.png", Some(img_path.clone())).expect("process");
    let drafts = vec![DraftImageAttachment {
        id: "d1".into(),
        image: processed,
    }];

    // 同一张图片在直接草稿和文本引用中同时出现
    let text = "Look at @same.png ";
    let prepared = prepare_user_message(&dir, text, &drafts).expect("prepare");
    // 附件展示有 2 条（草稿 + 引用行）
    assert_eq!(prepared.attachments.len(), 2);
    // 但提供给模型的图片去重后只有 1 份！
    assert_eq!(prepared.model_images.len(), 1);
}

#[test]
fn prepare_rejects_more_than_four_unique_images() {
    let dir = temp_dir("limit");
    let mut drafts = Vec::new();
    for i in 0..5 {
        let png = create_png_bytes(10 + i, 10 + i);
        let processed = process_image_bytes(&png, &format!("{i}.png"), None).expect("process");
        drafts.push(DraftImageAttachment {
            id: format!("d{i}"),
            image: processed,
        });
    }

    let err = prepare_user_message(&dir, "hi", &drafts).unwrap_err();
    assert_eq!(err, ImagePrepareError::TooManyImages);
}

#[test]
fn strip_references_cleans_display_text() {
    let text = "Please review @[refer-file](src/main.rs) and @[refer-image](img.png) carefully.";
    let parsed = parse_references(text);
    let metas = vec![
        crate::db::chat::MessageAttachmentMeta {
            id: "1".into(),
            position: 0,
            kind: MessageAttachmentKind::File,
            label: "src/main.rs".into(),
            marker_start: Some(parsed[0].range.start),
            marker_end: Some(parsed[0].range.end + 1), // including space
            source_path: None,
            mime: None,
            width: None,
            height: None,
        },
        crate::db::chat::MessageAttachmentMeta {
            id: "2".into(),
            position: 1,
            kind: MessageAttachmentKind::Image,
            label: "img.png".into(),
            marker_start: Some(parsed[1].range.start),
            marker_end: Some(parsed[1].range.end + 1), // including space
            source_path: None,
            mime: None,
            width: None,
            height: None,
        },
    ];

    let stripped = strip_references_for_display(text, &metas);
    assert_eq!(stripped, "Please review and carefully.");
}
