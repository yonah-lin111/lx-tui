//! 单元测试；仅测试构建编译。

use super::*;

#[test]
fn ellipsize_keeps_text_within_limit() {
    assert_eq!(ellipsize("abc", 3), "abc");
    assert_eq!(ellipsize("abc", 5), "abc");
}

#[test]
fn ellipsize_marks_truncation() {
    assert_eq!(ellipsize("Copied to clipboard", 5), "Copi…");
    assert_eq!(ellipsize("abc", 1), "…");
    assert_eq!(ellipsize("abc", 0), "");
}
