use super::*;

#[test]
fn encode_decode_roundtrip_for_plain_paths() {
    for path in ["/tmp/a.png", "C:/Users/me/a b.txt", "src/图片 空格.png"] {
        let encoded = encode_path(path);
        assert_eq!(decode_path(&encoded), path);
    }
}

#[test]
fn encode_decode_roundtrip_for_separator_chars() {
    // 分隔符与反斜杠必须转义后仍可无损还原。
    for path in [
        "/tmp/a(1).png",
        "/tmp/a\\b.png",
        "C:\\Users\\me\\a(b).png",
        "/tmp/\\(x\\)/y.png",
    ] {
        let encoded = encode_path(path);
        assert_eq!(decode_path(&encoded), path);
    }
}

#[test]
fn format_reference_matches_syntax() {
    let marker = format_reference(ReferKind::File, Path::new("/tmp/a b.txt"));
    assert_eq!(marker.as_deref(), Some("@[refer-file](/tmp/a b.txt)"));
    let marker = format_reference(ReferKind::Folder, Path::new("/tmp/dir"));
    assert_eq!(marker.as_deref(), Some("@[refer-folder](/tmp/dir)"));
    let marker = format_reference(ReferKind::Image, Path::new("/tmp/a(1).png"));
    assert_eq!(marker.as_deref(), Some("@[refer-image](/tmp/a\\(1\\).png)"));
}

#[test]
fn format_reference_rejects_newline_paths() {
    assert_eq!(
        format_reference(ReferKind::File, Path::new("/tmp/a\nb")),
        None
    );
    assert!(!reference_representable("a\nb"));
    assert!(reference_representable("a(b)c\\d"));
}

#[test]
fn parse_references_reads_tokens_and_decodes() {
    let text = "前 @[refer-file](/tmp/a\\(1\\).txt) 后 @[refer-image](/tmp/x.png)";
    let parsed = parse_references(text);
    assert_eq!(parsed.len(), 2);
    assert_eq!(parsed[0].kind, ReferKind::File);
    assert_eq!(parsed[0].path, "/tmp/a(1).txt");
    assert_eq!(
        &text[parsed[0].range.clone()],
        "@[refer-file](/tmp/a\\(1\\).txt)"
    );
    assert_eq!(parsed[1].kind, ReferKind::Image);
    assert_eq!(parsed[1].path, "/tmp/x.png");
}

#[test]
fn parse_references_skips_bad_tokens_and_fences() {
    let text = "@[refer-nope](/tmp/a) @[refer-file]broken\n```\n@[refer-file](/tmp/fenced)\n```\n@[refer-folder](/tmp/dir)";
    let parsed = parse_references(text);
    let paths: Vec<&str> = parsed.iter().map(|token| token.path.as_str()).collect();
    assert_eq!(paths, vec!["/tmp/dir"]);
}

#[test]
fn parse_references_unclosed_skipped() {
    let parsed = parse_references("@[refer-file](/tmp/a");
    assert!(parsed.is_empty());
}

#[test]
fn parse_references_in_scopes_range() {
    let text = "@[refer-file](/a.txt) 间隔 @[refer-file](/b.txt)";
    let start = text.find(" 间隔").expect("split");
    let in_first = parse_references_in(text, 0, start);
    assert_eq!(in_first.len(), 1);
    assert_eq!(in_first[0].path, "/a.txt");
}

#[test]
fn image_reference_paths_filters_by_kind() {
    let text = "@[refer-file](/a.txt) @[refer-image](/b.png) @[refer-image](/c\\).png)";
    assert_eq!(image_reference_paths(text), vec!["/b.png", "/c).png"]);
}

#[test]
fn decode_unknown_escape_keeps_literal_backslash() {
    assert_eq!(decode_path("C:\\Users\\x"), "C:\\Users\\x");
}
