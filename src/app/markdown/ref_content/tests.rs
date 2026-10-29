use super::*;

#[test]
fn format_ref_content_handles_file_and_folder() {
    assert_eq!(
        format_ref_content("src/main.rs", false),
        "@[ref-content]/src/main.rs"
    );
    assert_eq!(format_ref_content("src/ui", true), "@[ref-content]/src/ui/");
    assert_eq!(
        format_ref_content("/src/ui/", true),
        "@[ref-content]/src/ui/"
    );
}

#[test]
fn ref_content_ranges_detects_clean_markers() {
    let text = "@[ref-content]/src/main.rs other @[ref-content]/src/ui/";
    let ranges = ref_content_ranges(text);
    assert_eq!(ranges.len(), 2);
    assert_eq!(&text[ranges[0].clone()], "@[ref-content]/src/main.rs");
    assert_eq!(&text[ranges[1].clone()], "@[ref-content]/src/ui/");
}

#[test]
fn ref_content_ranges_ignores_in_code_fence() {
    let text = "```\n@[ref-content]/src/main.rs\n```\n@[ref-content]/valid.rs";
    let ranges = ref_content_ranges(text);
    assert_eq!(ranges.len(), 1);
    assert_eq!(&text[ranges[0].clone()], "@[ref-content]/valid.rs");
}

#[test]
fn ref_content_ranges_trims_sentence_dot() {
    let text = "Check @[ref-content]/src/main.rs.";
    let ranges = ref_content_ranges(text);
    assert_eq!(ranges.len(), 1);
    assert_eq!(&text[ranges[0].clone()], "@[ref-content]/src/main.rs");
}

#[test]
fn ref_content_deletion_range_requires_trailing_blank() {
    let text = "@[ref-content]/src/main.rs ";
    assert_eq!(
        ref_content_deletion_range(text, text.len()),
        Some(0..text.len())
    );

    // 光标在空格前（路径末尾）-> None
    assert_eq!(ref_content_deletion_range(text, text.len() - 1), None);

    // 路径中 -> None
    assert_eq!(ref_content_deletion_range(text, 10), None);

    // 没有尾随空格 -> None
    let text_no_space = "@[ref-content]/src/main.rs";
    assert_eq!(
        ref_content_deletion_range(text_no_space, text_no_space.len()),
        None
    );
}

#[test]
fn extract_mention_paths_gathers_both_kinds() {
    let text = "Look at @src/main.rs and @[ref-content]/docs/design.md with folder @dir/";
    let extracted = extract_mention_paths(text);
    assert_eq!(
        extracted,
        vec![
            ("docs/design.md".to_string(), false),
            ("src/main.rs".to_string(), false),
            ("dir".to_string(), true),
        ]
    );
}
