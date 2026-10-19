//! 文本编解码测试：行切分、BOM/换行保留与行 diff 计数。

use super::*;

#[test]
fn split_lines_handles_lf_crlf_and_missing_trailing_newline() {
    assert_eq!(split_lines("a\nb\n"), vec!["a", "b"]);
    assert_eq!(split_lines("a\r\nb\r\n"), vec!["a", "b"]);
    assert_eq!(split_lines("a\nb"), vec!["a", "b"]);
    assert_eq!(split_lines(""), Vec::<String>::new());
    assert_eq!(split_lines("\n"), vec![""]);
    assert_eq!(split_lines("a\n\nb"), vec!["a", "", "b"]);
}

#[test]
fn parse_detects_bom_and_newline_style() {
    let source = SourceText::parse(b"\xEF\xBB\xBFa\r\nb\r\n").expect("parse");
    assert!(source.bom);
    assert_eq!(source.newline, Newline::Crlf);
    assert_eq!(source.lines, vec!["a", "b"]);
    assert!(source.trailing_newline);

    let source = SourceText::parse(b"a\nb").expect("parse");
    assert!(!source.bom);
    assert_eq!(source.newline, Newline::Lf);
    assert!(!source.trailing_newline);
}

#[test]
fn parse_rejects_binary_and_invalid_utf8() {
    assert_eq!(
        SourceText::parse(b"a\0b").expect_err("binary").code,
        ToolErrorCode::Binary
    );
    assert_eq!(
        SourceText::parse(&[0x61, 0xff]).expect_err("utf8").code,
        ToolErrorCode::NotUtf8
    );
}

#[test]
fn encode_round_trips_bom_and_newline() {
    let source = SourceText::parse(b"\xEF\xBB\xBFa\r\nb\r\n").expect("parse");
    let lines = vec!["x".to_string(), "y".to_string()];
    assert_eq!(source.encode(&lines), b"\xEF\xBB\xBFx\r\ny\r\n".to_vec());
}

#[test]
fn encode_of_empty_lines_has_no_content() {
    let source = SourceText::from_text("a\n", false);
    assert!(source.encode(&[]).is_empty());
}

#[test]
fn encode_without_trailing_newline_writes_none() {
    let source = SourceText::parse(b"a").expect("parse");
    let lines = vec!["a".to_string(), "b".to_string()];
    assert_eq!(source.encode(&lines), b"a\nb".to_vec());
}

#[test]
fn line_diff_counts_prefix_and_suffix() {
    let old: Vec<String> = ["a", "b", "c"].iter().map(|s| s.to_string()).collect();
    let new: Vec<String> = ["a", "B", "c"].iter().map(|s| s.to_string()).collect();
    assert_eq!(line_diff_counts(&old, &new), (1, 1));

    let empty: Vec<String> = Vec::new();
    assert_eq!(line_diff_counts(&empty, &old), (3, 0));
    assert_eq!(line_diff_counts(&old, &empty), (0, 3));
    assert_eq!(line_diff_counts(&old, &old), (0, 0));
}

#[test]
fn blank_detection() {
    assert!(is_blank("  \n\t\n"));
    assert!(!is_blank("a"));
}
