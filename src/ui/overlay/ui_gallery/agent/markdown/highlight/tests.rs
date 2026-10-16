//! 单元测试：轻量语法高亮的语言识别与分色。

use super::*;

/// 取分段文本（用于断言原行不变）。
fn text_of(spans: &[(String, Style)]) -> String {
    spans.iter().map(|(value, _)| value.as_str()).collect()
}

/// 某样式在分段中是否出现。
fn has_style(spans: &[(String, Style)], style: Style) -> bool {
    spans.iter().any(|(_, current)| *current == style)
}

#[test]
fn highlighting_preserves_the_original_line() {
    for (lang, line) in [
        (
            Some("rust"),
            "    let name: String = \"world\".to_string();",
        ),
        (Some("ts"), "const x: number = 42; // note"),
        (Some("java"), "public class Main {}"),
        (None, "plain text 123"),
        (Some("go"), "func main() { defer f() }"),
    ] {
        let spans = highlight_line(lang, line);
        assert_eq!(text_of(&spans), line, "语言 {lang:?} 高亮后文本须保持原样");
    }
}

#[test]
fn rust_keywords_strings_comments_numbers_are_colored() {
    let spans = highlight_line(Some("rust"), "let s = \"hi\"; // c");
    assert!(has_style(&spans, style::code_keyword()), "应含关键字 let");
    assert!(has_style(&spans, style::code_string()), "应含字符串");
    assert!(has_style(&spans, style::code_comment()), "应含注释");

    let numbers = highlight_line(Some("rust"), "let n = 42;");
    assert!(has_style(&numbers, style::code_number()), "应含数字");
}

#[test]
fn type_names_are_colored() {
    let spans = highlight_line(Some("rust"), "pub struct AppState;");
    assert!(
        has_style(&spans, style::code_keyword()),
        "struct/pub 关键字"
    );
    assert!(has_style(&spans, style::code_type()), "AppState 类型名");
}

#[test]
fn js_ts_keywords_recognized() {
    for keyword in ["const", "function", "return", "class"] {
        let spans = highlight_line(Some("ts"), &format!("{keyword} x"));
        assert!(
            has_style(&spans, style::code_keyword()),
            "{keyword} 应识别为关键字"
        );
    }
}

#[test]
fn java_keywords_recognized() {
    let spans = highlight_line(Some("java"), "public void run() {}");
    assert!(has_style(&spans, style::code_keyword()));
}

#[test]
fn unknown_language_falls_back_to_c_like_keywords() {
    let spans = highlight_line(None, "int x = 1;");
    assert!(has_style(&spans, style::code_keyword()), "int 应识别");
    assert!(has_style(&spans, style::code_number()), "1 应识别");
}

#[test]
fn block_comment_is_colored() {
    let spans = highlight_line(Some("c"), "int x; /* inline */ int y;");
    assert!(has_style(&spans, style::code_comment()));
    assert_eq!(text_of(&spans), "int x; /* inline */ int y;");
}

#[test]
fn empty_line_yields_no_spans() {
    assert!(highlight_line(Some("rust"), "").is_empty());
}

#[test]
fn hash_starts_a_comment_only_for_script_languages() {
    // Python：`#` 起注释。
    let py = highlight_line(Some("python"), "x = 1  # note");
    assert!(has_style(&py, style::code_comment()), "python # 应为注释");

    // Rust：`#` 是属性，不是注释——`#[derive]` 不应整行变注释色。
    let rs = highlight_line(Some("rust"), "#[derive(Debug)]");
    assert!(!has_style(&rs, style::code_comment()), "rust 的 # 不是注释");
    assert_eq!(text_of(&rs), "#[derive(Debug)]");
}
