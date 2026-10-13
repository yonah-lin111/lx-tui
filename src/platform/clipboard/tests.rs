use super::*;

#[test]
fn parse_path_text_accepts_absolute_lines() {
    let parsed = parse_path_text("/tmp/a b.png\n/tmp/中文 目录").expect("paths");
    assert_eq!(
        parsed,
        vec![
            std::path::PathBuf::from("/tmp/a b.png"),
            std::path::PathBuf::from("/tmp/中文 目录")
        ]
    );
}

#[test]
fn parse_path_text_rejects_empty_lines_and_relative() {
    assert_eq!(parse_path_text("/tmp/a\n\n/tmp/b"), None);
    assert_eq!(parse_path_text("relative/path"), None);
    assert_eq!(parse_path_text(""), None);
    assert_eq!(parse_path_text("/tmp/a\nrelative"), None);
}

#[test]
fn parse_path_text_keeps_inner_and_edge_spaces() {
    let parsed = parse_path_text("/tmp/a  b\n").expect("path");
    assert_eq!(parsed, vec![std::path::PathBuf::from("/tmp/a  b")]);
}

#[cfg(not(windows))]
#[test]
fn parse_file_uri_decodes_percent_and_localhost() {
    let parsed = parse_path_text("file:///tmp/a%20b.png").expect("uri");
    assert_eq!(parsed, vec![std::path::PathBuf::from("/tmp/a b.png")]);
    let parsed = parse_path_text("file://localhost/tmp/中文").expect("uri");
    assert_eq!(parsed, vec![std::path::PathBuf::from("/tmp/中文")]);
}

#[test]
fn parse_file_uri_rejects_remote_host_and_bad_escapes() {
    assert_eq!(parse_path_text("file://example.com/tmp/a"), None);
    assert_eq!(parse_path_text("file:///tmp/%2"), None);
    assert_eq!(parse_path_text("file:///tmp/%zz"), None);
    assert_eq!(parse_path_text("/tmp/a\0b"), None);
}

#[test]
fn sniff_image_detects_magic() {
    assert_eq!(
        sniff_image(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0x00]),
        Some(ImageFormat::Png)
    );
    assert_eq!(
        sniff_image(&[0xFF, 0xD8, 0xFF, 0xE0]),
        Some(ImageFormat::Jpeg)
    );
    assert_eq!(sniff_image(b"GIF89a...."), Some(ImageFormat::Gif));
    assert_eq!(
        sniff_image(b"RIFF\x00\x00\x00\x00WEBPVP8 "),
        Some(ImageFormat::WebP)
    );
    assert_eq!(sniff_image(b"BM....."), Some(ImageFormat::Bmp));
    assert_eq!(sniff_image(b"II\x2a\x00"), Some(ImageFormat::Tiff));
    assert_eq!(sniff_image(b"plain text"), None);
}

#[test]
fn image_format_extensions_are_stable() {
    assert_eq!(ImageFormat::Png.extension(), "png");
    assert_eq!(ImageFormat::Jpeg.extension(), "jpg");
    assert_eq!(ImageFormat::Gif.extension(), "gif");
    assert_eq!(ImageFormat::WebP.extension(), "webp");
    assert_eq!(ImageFormat::Bmp.extension(), "bmp");
    assert_eq!(ImageFormat::Tiff.extension(), "tiff");
}

#[test]
fn secure_token_is_32_lower_hex() {
    let token = secure_token();
    assert_eq!(token.len(), 32);
    assert!(
        token
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    );
    assert_ne!(token, secure_token());
}

#[test]
fn managed_name_patterns_match_writers() {
    let token = "0123456789abcdef0123456789abcdef";
    assert!(crate::platform::is_managed_asset_name(&format!(
        "{token}.png"
    )));
    assert!(crate::platform::is_managed_temp_name(&format!(
        ".tmp-{token}.png"
    )));
    assert!(!crate::platform::is_managed_asset_name(&format!(
        "{token}.txt"
    )));
    assert!(!crate::platform::is_managed_asset_name(&format!(
        "{}.png",
        token.to_uppercase()
    )));
    assert!(!crate::platform::is_managed_asset_name("1234.png"));
    assert!(!crate::platform::is_managed_temp_name(&format!(
        "{token}.png"
    )));
}
