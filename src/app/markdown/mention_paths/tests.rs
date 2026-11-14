use super::*;

#[test]
fn extracts_plain_mentions_in_order() {
    let text = "Look at @src/main.rs and folder @dir/";
    assert_eq!(
        extract_mention_paths(text),
        vec![
            ("src/main.rs".to_string(), false),
            ("dir".to_string(), true),
        ]
    );
}

#[test]
fn skips_short_and_explicit_references() {
    let text = "@ref/a/b.md @[refer-file](/tmp/x.rs) @docs/design.md";
    assert_eq!(
        extract_mention_paths(text),
        vec![("docs/design.md".to_string(), false)]
    );
}

#[test]
fn skips_mentions_inside_code_fences() {
    let text = "```\n@src/ignored.rs\n```\n@src/kept.rs";
    assert_eq!(
        extract_mention_paths(text),
        vec![("src/kept.rs".to_string(), false)]
    );
}
