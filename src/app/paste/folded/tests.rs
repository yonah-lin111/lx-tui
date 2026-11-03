use super::*;

#[test]
fn escape_and_unescape_preserve_code_and_roundtrip_closing_tags() {
    let raw = "fn call(a: usize, b: (u8, u8)) {\n    println!(\"\\n\\\\)\");\n}";
    // Normal code without `</pasted-text>` has zero escaping noise.
    assert_eq!(escape_body(raw), raw);
    assert_eq!(unescape_body(raw), raw);

    let encoded = encode(raw);
    assert!(encoded.starts_with("<pasted-text lines=\"3\">"));
    assert!(encoded.ends_with("</pasted-text>"));
    let parsed = tokens(&encoded);
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].kind, FoldedKind::Text);
    assert_eq!(parsed[0].line_count, 3);
    assert_eq!(parsed[0].body, raw);
    assert_eq!(parsed[0].label(), "[Pasted text 3 lines]");

    // Text containing `</pasted-text>`, `</pasted-code>`, `</pasted-log>` and `<\/pasted-text>` roundtrips cleanly.
    let tricky =
        "before </pasted-text> mid </pasted-code> and </pasted-log> tail <\\/pasted-text> after";
    let escaped = escape_body(tricky);
    assert_eq!(
        escaped,
        "before <\\/pasted-text> mid <\\/pasted-code> and <\\/pasted-log> tail <\\\\/pasted-text> after"
    );
    assert!(!escaped.contains("</pasted-text>"));
    assert!(!escaped.contains("</pasted-code>"));
    assert!(!escaped.contains("</pasted-log>"));
    assert_eq!(unescape_body(&escaped), tricky);
}

#[test]
fn unescape_preserves_unknown_escape_sequences_and_trailing_backslash() {
    assert_eq!(unescape_body(r"hello \n \t world\"), r"hello \n \t world\");
    assert_eq!(unescape_body(r"\\ and \)"), r"\\ and \)");
}

#[test]
fn tokens_parses_multiple_and_nested_literal_tokens() {
    let inner = "<pasted-text lines=\"2\">inner\nliteral</pasted-text>";
    let first = encode(&format!("outer line 1\n{inner}\nouter line 3"));
    let second = encode("second block\nline 2\nline 3");
    let text = format!("before {first} middle {second} after");
    let parsed = tokens(&text);
    assert_eq!(parsed.len(), 2);
    assert_eq!(parsed[0].line_count, 4);
    assert!(parsed[0].body.contains(inner));
    assert_eq!(parsed[1].line_count, 3);
    assert_eq!(parsed[1].body, "second block\nline 2\nline 3");
    assert_eq!(
        expand_all(&text),
        format!(
            "before outer line 1\n{inner}\nouter line 3 middle second block\nline 2\nline 3 after"
        )
    );
}

#[test]
fn malformed_tokens_are_ignored_without_blocking_subsequent_valid_tokens() {
    let valid = encode("a\nb\nc");
    let clean = format!(
        "<pasted-text lines=\"0\">abc</pasted-text> <pasted-text lines=\"x\">def</pasted-text> <pasted-text lines=\"2\"></pasted-text> {valid}"
    );
    let parsed_clean = tokens(&clean);
    assert_eq!(parsed_clean.len(), 1);
    assert_eq!(parsed_clean[0].body, "a\nb\nc");
}

#[test]
fn fence_tracking_skips_tokens_inside_fences_and_ignores_fences_inside_token_body() {
    let tok_with_fence = encode("before fence\n```rust\nlet x = 1;\n");
    let tok_after = encode("after 1\nafter 2\nafter 3");
    let inside_fence = encode("inside 1\ninside 2\ninside 3");

    // 1. Token body contains an unclosed ``` fence; it must NOT put subsequent text into fence state.
    let doc1 = format!("{tok_with_fence}\n{tok_after}");
    let parsed1 = tokens(&doc1);
    assert_eq!(parsed1.len(), 2);
    assert_eq!(parsed1[0].body, "before fence\n```rust\nlet x = 1;\n");
    assert_eq!(parsed1[1].body, "after 1\nafter 2\nafter 3");

    // 2. Token inside an actual outer fence must be ignored.
    let doc2 = format!("```\n{inside_fence}\n```\n{tok_after}");
    let parsed2 = tokens(&doc2);
    assert_eq!(parsed2.len(), 1);
    assert_eq!(parsed2[0].body, "after 1\nafter 2\nafter 3");
}

#[test]
fn kind_specific_tokens_encode_and_format_labels_for_text_code_and_log() {
    let text_tok = &tokens(&encode_kind(FoldedKind::Text, "single"))[0];
    assert_eq!(text_tok.kind, FoldedKind::Text);
    assert_eq!(text_tok.label(), "[Pasted text 1 line]");

    let code_tok = &tokens(&encode_kind(FoldedKind::Code, "fn main() {\n}"))[0];
    assert_eq!(code_tok.kind, FoldedKind::Code);
    assert_eq!(code_tok.label(), "[Pasted code 2 lines]");

    let log_tok = &tokens(&encode_kind(FoldedKind::Log, "INFO 1\nWARN 2\nERROR 3"))[0];
    assert_eq!(log_tok.kind, FoldedKind::Log);
    assert_eq!(log_tok.label(), "[Pasted log 3 lines]");
}

#[test]
fn mask_preserves_byte_length_and_newlines_while_blanking_triggers() {
    let tok = encode("line 1 with @mention and /slash\n&&&\n+++\n```rust\n{title: fake}");
    let text = format!("head\n{tok}\ntail");
    let masked = mask(&text);
    assert_eq!(masked.len(), text.len());
    let orig_newlines: Vec<usize> = text
        .bytes()
        .enumerate()
        .filter_map(|(i, b)| (b == b'\n').then_some(i))
        .collect();
    let mask_newlines: Vec<usize> = masked
        .bytes()
        .enumerate()
        .filter_map(|(i, b)| (b == b'\n').then_some(i))
        .collect();
    assert_eq!(orig_newlines, mask_newlines);
    assert!(!masked.contains("@mention"));
    assert!(!masked.contains("&&&"));
    assert!(!masked.contains("+++"));
    assert!(!masked.contains("```"));
    assert!(!masked.contains("{title:"));
    assert!(masked.starts_with("head\n"));
    assert!(masked.ends_with("\ntail"));
}

#[test]
fn projection_handles_collapsed_expanded_and_duplicate_tokens_independently() {
    let tok = encode("alpha\nbeta\ngamma");
    let raw = format!("A {tok} B {tok} C");
    let mut expanded = ExpandedPastes::default();

    // Initially both collapsed.
    let proj0 = Projection::new(&raw, &expanded);
    let label = proj0.tokens()[0].label();
    assert_eq!(label, "[Pasted text 3 lines]");
    assert_eq!(proj0.text(), format!("A {label} B {label} C"));

    // Expand only the second occurrence (occurrence = 1).
    let id = proj0.tokens()[0].id;
    assert!(expanded.toggle(id, 1));
    let proj1 = Projection::new(&raw, &expanded);
    assert_eq!(proj1.text(), format!("A {label} B alpha\nbeta\ngamma C"));

    // Coordinate roundtrips at boundaries.
    let second_raw_start = proj1.tokens()[1].range.start;
    let second_raw_end = proj1.tokens()[1].range.end;
    let second_disp_start = proj1.raw_to_display(second_raw_start);
    let second_disp_end = proj1.raw_to_display(second_raw_end);
    assert_eq!(
        &proj1.text()[second_disp_start..second_disp_end],
        "alpha\nbeta\ngamma"
    );
    assert_eq!(proj1.display_to_raw(second_disp_start), second_raw_start);
    assert_eq!(proj1.display_to_raw(second_disp_end), second_raw_end);

    // Copy display range: collapsed chip copies raw token, expanded chip copies decoded slice.
    let copy_first = proj1.copy_display_range(0..(2 + label.len()));
    assert_eq!(copy_first, format!("A {tok}"));
    let copy_second_sub =
        proj1.copy_display_range((second_disp_start + 6)..(second_disp_start + 10));
    assert_eq!(copy_second_sub, "beta");
}

#[test]
fn atomic_boundaries_skip_and_snap_across_tokens() {
    let tok = encode("1\n2\n3");
    let text = format!("ab{tok}cd");
    let tok_start = 2;
    let tok_end = 2 + tok.len();

    assert_eq!(next_boundary(&text, 0), 1);
    assert_eq!(next_boundary(&text, 1), tok_start);
    assert_eq!(next_boundary(&text, tok_start), tok_end);
    assert_eq!(next_boundary(&text, tok_start + 3), tok_end);
    assert_eq!(next_boundary(&text, tok_end), tok_end + 1);

    assert_eq!(prev_boundary(&text, tok_end + 1), tok_end);
    assert_eq!(prev_boundary(&text, tok_end), tok_start);
    assert_eq!(prev_boundary(&text, tok_start + 3), tok_start);
    assert_eq!(prev_boundary(&text, tok_start), 1);

    assert_eq!(
        snap_range(&text, (tok_start + 1)..(tok_start + 4)),
        tok_start..tok_end
    );
    assert_eq!(snap_range(&text, 1..(tok_start + 2)), 1..tok_end);
}
