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
