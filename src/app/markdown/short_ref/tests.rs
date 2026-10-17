//! 短引用格式单测：短名生成、类型推断、整块删除范围与区间扫描。

use super::*;

#[test]
fn short_name_keeps_last_two_segments() {
    assert_eq!(
        short_ref_name("/Users/yonah/projects/agent/lx-agent").as_deref(),
        Some("agent/lx-agent")
    );
    assert_eq!(
        short_ref_name("docs/feature/lx-at-mention/design.md").as_deref(),
        Some("lx-at-mention/design.md")
    );
    assert_eq!(short_ref_name("/tmp/a.png").as_deref(), Some("tmp/a.png"));
    assert_eq!(short_ref_name("main.rs").as_deref(), Some("main.rs"));
}

#[test]
fn short_name_ignores_trailing_separators() {
    assert_eq!(
        short_ref_name("/tmp/shots/").as_deref(),
        Some("tmp/shots"),
        "尾斜杠先剔除，与不带斜杠同一结果"
    );
    assert_eq!(short_ref_name("/").as_deref(), None);
    assert_eq!(short_ref_name("").as_deref(), None);
}

#[test]
fn short_name_handles_windows_separators() {
    assert_eq!(
        short_ref_name("C:\\Users\\me\\proj\\a.md").as_deref(),
        Some("proj/a.md")
    );
}

#[test]
fn format_appends_slash_for_directories() {
    assert_eq!(
        format_short_ref("/Users/yonah/projects/agent/lx-agent", true).as_deref(),
        Some("@ref/agent/lx-agent/")
    );
    assert_eq!(
        format_short_ref("/Users/yonah/projects/agent/lx-tui/src/main.rs", false).as_deref(),
        Some("@ref/src/main.rs")
    );
    assert_eq!(format_short_ref("/", false), None, "无段无法生成短名");
}

#[test]
fn kind_is_inferred_from_shape() {
    assert_eq!(short_ref_kind("agent/lx-agent/"), ShortRefKind::Folder);
    assert_eq!(short_ref_kind("tmp/shots/a.png"), ShortRefKind::Image);
    assert_eq!(short_ref_kind("tmp/shots/a.PNG"), ShortRefKind::Image);
    assert_eq!(
        short_ref_kind("lx-at-mention/design.md"),
        ShortRefKind::File
    );
    assert_eq!(
        short_ref_kind("pics/without-extension"),
        ShortRefKind::File,
        "无扩展名按文件处理"
    );
}

#[test]
fn ranges_stop_at_boundaries() {
    let text = "see @ref/agent/lx-agent/ and @ref/src/main.rs.";
    let ranges = short_ref_ranges(text);
    assert_eq!(ranges.len(), 2);
    assert_eq!(&text[ranges[0].clone()], "@ref/agent/lx-agent/");
    assert_eq!(&text[ranges[1].clone()], "@ref/src/main.rs", "句点终止标记");
}

#[test]
fn ranges_require_word_boundary_and_body() {
    assert!(short_ref_ranges("a@ref/x/y").is_empty(), "非边界不识别");
    assert!(short_ref_ranges("@ref/").is_empty(), "空前缀正文不成标记");
    assert!(
        short_ref_ranges("(@ref/a/b)").is_empty(),
        "括号之后不是空白边界"
    );
}

#[test]
fn deletion_range_covers_marker_and_trailing_blank() {
    let text = "see @ref/agent/lx-agent/ ";
    assert_eq!(
        short_ref_deletion_range(text, text.len()),
        Some(4..text.len())
    );
    // 光标不在空白之后时不整块删除。
    let text = "see @ref/agent/lx-agent/";
    assert_eq!(short_ref_deletion_range(text, text.len()), None);
}

#[test]
fn deletion_range_skips_unrelated_cursor() {
    let text = "@ref/a/b other";
    assert_eq!(short_ref_deletion_range(text, 5), None);
    assert_eq!(short_ref_deletion_range("", 0), None);
}
