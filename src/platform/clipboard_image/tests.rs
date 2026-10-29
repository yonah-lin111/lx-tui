use image::{ImageBuffer, ImageFormat, Rgba};
use std::io::Cursor;

use super::*;

fn create_test_png(w: u32, h: u32) -> Vec<u8> {
    let img: ImageBuffer<Rgba<u8>, Vec<u8>> = ImageBuffer::from_fn(w, h, |x, y| {
        Rgba([(x % 255) as u8, (y % 255) as u8, 128, 255])
    });
    let mut bytes = Vec::new();
    img.write_to(&mut Cursor::new(&mut bytes), ImageFormat::Png)
        .expect("write png");
    bytes
}

fn create_test_jpeg(w: u32, h: u32) -> Vec<u8> {
    let img: ImageBuffer<image::Rgb<u8>, Vec<u8>> = ImageBuffer::from_fn(w, h, |x, y| {
        image::Rgb([(x % 255) as u8, (y % 255) as u8, 128])
    });
    let mut bytes = Vec::new();
    img.write_to(&mut Cursor::new(&mut bytes), ImageFormat::Jpeg)
        .expect("write jpeg");
    bytes
}

#[test]
fn process_small_png_success() {
    let png = create_test_png(100, 50);
    let processed = process_image_bytes(&png, "test.png", None).expect("process png");
    assert_eq!(processed.source_name, "test.png");
    assert_eq!(processed.mime, "image/png");
    assert_eq!(processed.width, 100);
    assert_eq!(processed.height, 50);
    assert!(!processed.bytes.is_empty());
}

#[test]
fn process_small_jpeg_success() {
    let jpeg = create_test_jpeg(80, 60);
    let processed = process_image_bytes(&jpeg, "test.jpg", None).expect("process jpeg");
    assert_eq!(processed.source_name, "test.jpg");
    assert_eq!(processed.mime, "image/jpeg");
    assert_eq!(processed.width, 80);
    assert_eq!(processed.height, 60);
    assert!(!processed.bytes.is_empty());
}

#[test]
fn rejects_non_image_data() {
    let data = b"this is not an image file";
    let err = process_image_bytes(data, "fake.png", None).unwrap_err();
    assert_eq!(err, ImagePrepareError::NotAnImage);
    assert_eq!(err.code(), "not_an_image");
}

#[test]
fn rejects_oversized_single_file() {
    // 构造伪数据超过 20 MiB
    let dummy = vec![0u8; MAX_IMAGE_RAW_BYTES + 1];
    let err = process_image_bytes(&dummy, "big.png", None).unwrap_err();
    assert_eq!(err, ImagePrepareError::SingleTooLarge);
    assert_eq!(err.code(), "single_too_large");
}

#[test]
fn scales_large_dimension_image() {
    // 构造大于 2048 的图像：3000 x 1500
    let png = create_test_png(3000, 1500);
    let processed = process_image_bytes(&png, "large.png", None).expect("process large");
    assert!(processed.width <= 2048);
    assert!(processed.height <= 2048);
    assert_eq!(processed.width, 2048);
    assert_eq!(processed.height, 1024);
}

#[test]
fn file_loading_non_existent() {
    let err =
        load_and_process_image_file(Path::new("/tmp/non_existent_file_123456.png")).unwrap_err();
    assert_eq!(err, ImagePrepareError::FileNotFound);
}

#[test]
fn archive_session_image_stores_under_session_id_and_prunes_after_30_days() {
    let dir = std::env::temp_dir().join(format!("lx-images-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let src_dir = dir.join("desktop");
    let images_root = dir.join("images");
    std::fs::create_dir_all(&src_dir).expect("src dir");

    let src1 = src_dir.join("PixPin_1.png");
    let src2 = src_dir.join("PixPin_2.png");
    std::fs::write(&src1, create_test_png(16, 16)).expect("write src1");
    std::fs::write(&src2, create_test_png(32, 16)).expect("write src2");

    let path1 = archive_session_image_in(&images_root, "sess-abc", &src1).expect("archive 1");
    let path1_again =
        archive_session_image_in(&images_root, "sess-abc", &src1).expect("archive 1 again");
    assert_eq!(path1, path1_again, "同会话内相同内容图片应复用同一归档文件");
    assert!(
        path1.starts_with(images_root.join("sess-abc")),
        "必须按 <images_root>/<session_id>/ 组织: {path1:?}"
    );

    let path2 = archive_session_image_in(&images_root, "sess-def", &src2).expect("archive 2");
    assert!(path2.starts_with(images_root.join("sess-def")));

    // 30 天保留期内（IMAGE_RETENTION_SECS），即使未引用也不清理
    assert_eq!(IMAGE_RETENTION_SECS, 30 * 24 * 3600);
    let empty_refs = std::collections::HashSet::new();
    assert_eq!(
        prune_session_images_in(
            &images_root,
            &empty_refs,
            std::time::Duration::from_secs(IMAGE_RETENTION_SECS)
        ),
        0
    );
    assert!(path1.exists());
    assert!(path2.exists());

    // 超过保留期（max_age = 0）时，保留仍在数据库引用的 path1，清理未引用的 path2 及其空会话目录
    let mut referenced = std::collections::HashSet::new();
    referenced.insert(path1.to_string_lossy().into_owned());
    let removed = prune_session_images_in(&images_root, &referenced, std::time::Duration::ZERO);
    assert_eq!(removed, 1);
    assert!(path1.exists(), "仍在数据库引用中的图片不得被清理");
    assert!(!path2.exists(), "未引用且过期的图片应被清理");
    assert!(
        !images_root.join("sess-def").exists(),
        "空会话图片目录应一并清理"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
