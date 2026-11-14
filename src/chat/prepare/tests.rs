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
fn prepare_keeps_repeated_text_reference_markers() {
    let dir = temp_dir("dedupe-ref");
    std::fs::write(dir.join("Cargo.toml"), "[package]").expect("write");

    let text = "@Cargo.toml 这个说什么\n\n@Cargo.toml 有什么功能";
    let prepared = prepare_user_message(&dir, text, &[]).expect("prepare");
    assert_eq!(
        prepared.attachments.len(),
        2,
        "每个出现位置都保留 marker，标题与 rewind 才能处理全部引用"
    );
    assert_eq!(prepared.attachments[0].marker_start, Some(0));
    assert_eq!(prepared.attachments[1].marker_start, Some(29));
    assert!(
        prepared
            .attachments
            .iter()
            .all(|att| att.kind == MessageAttachmentKind::File)
    );
    let stripped = strip_references_from_inputs(text, &prepared.attachments);
    assert!(
        !stripped.contains("@Cargo.toml"),
        "标题剥离必须覆盖重复引用: {stripped}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn prepare_keeps_same_target_across_syntaxes() {
    let dir = temp_dir("dedupe-syntax");
    std::fs::write(dir.join("src.rs"), "fn main() {}").expect("write");

    let text = "看 @src.rs 和 @[refer-file](src.rs)";
    let prepared = prepare_user_message(&dir, text, &[]).expect("prepare");
    assert_eq!(
        prepared.attachments.len(),
        2,
        "不同写法都保留 marker，展示层再按目标收敛"
    );
    assert_eq!(
        prepared.attachments[0].source_path.as_deref(),
        Some("src.rs")
    );
    assert_eq!(
        prepared.attachments[1].source_path.as_deref(),
        Some("src.rs")
    );
    assert!(
        prepared.attachments[0].marker_start < prepared.attachments[1].marker_start,
        "按正文出现顺序排列"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn prepare_keeps_duplicate_image_markers_but_sends_one_image() {
    let dir = temp_dir("dedupe-image");
    let png = create_png_bytes(10, 10);
    std::fs::write(dir.join("photo.png"), &png).expect("write image");

    let text = "看 @photo.png 和 @photo.png";
    let prepared = prepare_user_message(&dir, text, &[]).expect("prepare");
    assert_eq!(prepared.attachments.len(), 2, "两个 marker 都保留");
    assert!(
        prepared
            .attachments
            .iter()
            .all(|att| att.kind == MessageAttachmentKind::Image)
    );
    assert_eq!(prepared.model_images.len(), 1, "模型只收到一份图片");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn prepare_keeps_image_kind_when_explicit_file_reference_comes_first() {
    let dir = temp_dir("dedupe-kind");
    let png = create_png_bytes(10, 10);
    std::fs::write(dir.join("photo.png"), &png).expect("write image");

    let text = "看 @[refer-file](photo.png) 和 @photo.png";
    let prepared = prepare_user_message(&dir, text, &[]).expect("prepare");
    assert_eq!(prepared.attachments.len(), 2);
    assert_eq!(prepared.attachments[0].kind, MessageAttachmentKind::File);
    assert_eq!(
        prepared.attachments[1].kind,
        MessageAttachmentKind::Image,
        "显式 File 引用不得吞掉后面的 Image 附件"
    );
    assert_eq!(prepared.model_images.len(), 1, "模型图片仍按内容去重");

    let _ = std::fs::remove_dir_all(&dir);
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

#[test]
fn prepare_sorts_attachments_by_marker_order_and_skips_inline_code() {
    let dir = temp_dir("order-and-code");
    std::fs::write(dir.join("early.rs"), "fn a() {}").expect("write early");
    std::fs::write(dir.join("late.rs"), "fn b() {}").expect("write late");

    // `@early.rs` 在文本前面，`@[refer-file](late.rs)` 在后面，且行内反引号里的 `@early.rs` 应被跳过
    let text = "先看 @early.rs 再看 `@early.rs` 最后看 @[refer-file](late.rs) ";
    let prepared = prepare_user_message(&dir, text, &[]).expect("prepare");
    assert_eq!(prepared.attachments.len(), 2);
    assert_eq!(prepared.attachments[0].label, "early.rs");
    assert_eq!(prepared.attachments[1].label, "late.rs");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn prepare_ai_web_message_handles_local_files_cloud_keep_files_and_rejects_folders() {
    let dir = temp_dir("ai-web-prepare");
    let file_path = dir.join("note.txt");
    let sub_dir = dir.join("sub");
    std::fs::create_dir_all(&sub_dir).expect("create sub dir");
    std::fs::write(&file_path, "hello").expect("write note");

    // 1. Local file + retained cloud image
    let raw = format!(
        "总结一下 @[refer-file]({}) @[refer-image](cloud_pic.png)",
        file_path.display()
    );
    let prepared =
        prepare_ai_web_message(&dir, "u-1", &raw, &["cloud_pic.png".to_string()]).expect("ok");
    assert_eq!(prepared.web_prompt, "总结一下");
    assert_eq!(prepared.files.len(), 1);
    assert_eq!(prepared.keep_files, vec!["cloud_pic.png".to_string()]);
    assert_eq!(prepared.attachments.len(), 2);
    assert_eq!(prepared.attachments[0].kind, MessageAttachmentKind::File);
    assert_eq!(prepared.attachments[1].kind, MessageAttachmentKind::Image);

    // 2. Folder reference is rejected
    let folder_raw = format!("看目录 @[refer-folder]({})", sub_dir.display());
    assert_eq!(
        prepare_ai_web_message(&dir, "u-2", &folder_raw, &[]),
        Err(AiWebPrepareError::FolderUnsupported)
    );

    // 3. Skill/MCP reference is rejected
    assert_eq!(
        prepare_ai_web_message(&dir, "u-3", "用 @[refer-skill](/tmp/s) 处理", &[]),
        Err(AiWebPrepareError::LocalReferenceUnsupported)
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn prepare_ai_web_keeps_repeated_reference_markers_but_uploads_once() {
    let dir = temp_dir("ai-web-dedup");
    std::fs::write(dir.join("note.txt"), "hello").expect("write note");

    let prepared =
        prepare_ai_web_message(&dir, "u-1", "看 @note.txt 以及 @note.txt", &[]).expect("prepare");
    assert_eq!(prepared.attachments.len(), 2, "每个出现位置都保留 marker");
    assert_eq!(prepared.files.len(), 1, "上传路径去重");
    assert!(prepared.files[0].ends_with("note.txt"));
    assert_eq!(
        prepared.web_prompt, "看 以及",
        "重复引用标记都必须从网页 prompt 剥离"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn prepare_treats_legacy_ref_content_token_as_plain_text() {
    let dir = temp_dir("legacy-ref-content");
    std::fs::write(dir.join("refer.rs"), "fn main() {}").expect("write");

    let text = "看 @[ref-content]/refer.rs 这个";
    let prepared = prepare_user_message(&dir, text, &[]).expect("prepare");
    assert!(prepared.attachments.is_empty(), "旧 token 不再挂载文件");
    assert_eq!(prepared.raw_text, text, "正文原样保留");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn prepare_resolves_short_ref_when_full_reference_exists() {
    let dir = temp_dir("short-ref");
    std::fs::create_dir_all(dir.join("docs/feature/x")).expect("mkdir");
    std::fs::write(dir.join("docs/feature/x/design.md"), "design").expect("write");

    let text = "见 @docs/feature/x/design.md 和 @ref/x/design.md";
    let prepared = prepare_user_message(&dir, text, &[]).expect("prepare");
    assert_eq!(prepared.attachments.len(), 2, "完整引用 + 短引用各一条");
    assert_eq!(
        prepared.attachments[0].source_path.as_deref(),
        Some("docs/feature/x/design.md")
    );
    assert_eq!(
        prepared.attachments[1].source_path.as_deref(),
        Some("docs/feature/x/design.md")
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn prepare_ai_web_keeps_duplicate_cloud_reference_without_error() {
    let dir = temp_dir("ai-web-cloud-dup");
    let prepared = prepare_ai_web_message(
        &dir,
        "u-1",
        "看 @[refer-image](cloud.png) 和 @[refer-image](cloud.png)",
        &["cloud.png".to_string()],
    )
    .expect("duplicate cloud reference is not an error");
    assert_eq!(prepared.attachments.len(), 2);
    assert_eq!(prepared.keep_files, vec!["cloud.png".to_string()]);
    assert!(prepared.files.is_empty());
    assert_eq!(prepared.web_prompt, "看 和");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn prepare_keeps_subagent_references_as_plain_text() {
    let dir = temp_dir("subagent-ref");
    let text = "Ask @[refer-subagent](explorer) and @[refer-subagent](Turing:explorer) to help";
    let prepared = prepare_user_message(&dir, text, &[]).expect("prepare");
    assert_eq!(prepared.raw_text, text);
    assert!(prepared.attachments.is_empty());
    assert!(prepared.model_images.is_empty());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn prepare_ai_web_rejects_subagent_reference() {
    let dir = temp_dir("ai-web-subagent");
    assert_eq!(
        prepare_ai_web_message(&dir, "u-1", "Ask @[refer-subagent](explorer) now", &[]),
        Err(AiWebPrepareError::LocalReferenceUnsupported)
    );

    let _ = std::fs::remove_dir_all(&dir);
}
