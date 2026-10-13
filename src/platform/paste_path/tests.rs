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
fn is_image_matches_known_signatures() {
    assert!(is_image(&[
        0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0x00
    ]));
    assert!(is_image(&[0xFF, 0xD8, 0xFF, 0xE0]));
    assert!(is_image(b"GIF89a...."));
    assert!(is_image(b"RIFF\x00\x00\x00\x00WEBPVP8 "));
    assert!(is_image(b"BM....."));
    assert!(is_image(b"II\x2a\x00"));
    assert!(is_image(b"MM\x00\x2a"));
    assert!(!is_image(b"plain text"));
    assert!(!is_image(b"RIFF....."));
}

#[test]
fn sniff_image_file_reads_header_and_ignores_missing() {
    let dir = std::env::temp_dir().join("lx-tui-sniff-test");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let png = dir.join("a.png");
    std::fs::write(&png, [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]).expect("write png");
    let text = dir.join("a.txt");
    std::fs::write(&text, b"hello").expect("write txt");
    assert!(sniff_image_file(&png));
    assert!(!sniff_image_file(&text));
    assert!(!sniff_image_file(&dir.join("missing.png")));
    let _ = std::fs::remove_dir_all(&dir);
}
