use super::*;

#[test]
fn from_arguments_merges_query_and_queries_with_defaults() {
    let view = WebSearchDetailView::from_arguments(
        r#"{"query":" ratatui 0.29 ","queries":["ratatui 0.29","", "tokio 1.40"]}"#,
    )
    .expect("valid object");
    assert_eq!(view.queries, ["ratatui 0.29", "tokio 1.40"]);
    assert_eq!(view.search_type, "auto");
    assert_eq!(view.num_results, 8);
    assert_eq!(view.provider, None);
    assert!(view.result.is_empty());
    assert_eq!(view.duration_ms, None);
    assert!(!view.truncated);
}

#[test]
fn from_arguments_reads_non_default_parameters_and_camel_case_count() {
    let view =
        WebSearchDetailView::from_arguments(r#"{"queries":["a"],"numResults":5,"type":"deep"}"#)
            .expect("valid object");
    assert_eq!(view.num_results, 5);
    assert_eq!(view.search_type, "deep");
    assert!(!view.num_results_is_default());
    assert!(!view.search_type_is_default());
}

#[test]
fn from_arguments_keeps_partial_objects_and_rejects_non_objects() {
    assert_eq!(
        WebSearchDetailView::from_arguments("{}"),
        Some(empty()),
        "形状合法但没有查询词"
    );
    assert!(WebSearchDetailView::from_arguments("not json").is_none());
    assert!(WebSearchDetailView::from_arguments("[]").is_none());
}

#[test]
fn from_parts_reads_result_json_and_keeps_queries_from_arguments() {
    let view = WebSearchDetailView::from_parts(
        Some(
            r#"{"tool":"web_search","queries":["b"],"num_results":3,"search_type":"fast",
                "provider":"exa","text":"1. Title\nhttps://example.com\nsummary",
                "truncated":true,"duration_ms":840}"#,
        ),
        r#"{"query":"a"}"#,
    )
    .expect("web_search result");
    assert_eq!(view.queries, ["b"]);
    assert_eq!(view.num_results, 3);
    assert_eq!(view.search_type, "fast");
    assert_eq!(view.provider.as_deref(), Some("exa"));
    assert_eq!(view.result, ["1. Title", "https://example.com", "summary"]);
    assert_eq!(view.result_more, 0);
    assert_eq!(view.duration_ms, Some(840));
    assert!(view.truncated);
}

#[test]
fn from_parts_caps_the_result_window_and_counts_the_rest() {
    let text = (0..45)
        .map(|index| format!("line {index}"))
        .collect::<Vec<_>>()
        .join("\n");
    let result_json = serde_json::json!({"tool": "web_search", "text": text}).to_string();
    let view = WebSearchDetailView::from_parts(Some(&result_json), "{}").expect("result");
    assert_eq!(view.result.len(), RESULT_LINES);
    assert_eq!(view.result_more, 5);
    assert!(view.queries.is_empty());
}

#[test]
fn from_parts_rejects_other_tools_and_garbage() {
    assert!(WebSearchDetailView::from_parts(None, "{}").is_none());
    assert!(WebSearchDetailView::from_parts(Some("not json"), "{}").is_none());
    assert!(WebSearchDetailView::from_parts(Some(r#"{"tool":"search"}"#), "{}").is_none());
}

#[test]
fn from_tool_falls_back_to_arguments_without_a_result() {
    let view = WebSearchDetailView::from_tool(None, r#"{"query":"a","type":"deep"}"#)
        .expect("arguments fallback");
    assert_eq!(view.queries, ["a"]);
    assert_eq!(view.search_type, "deep");
    assert_eq!(view.provider, None);
    assert!(WebSearchDetailView::from_tool(None, "not json").is_none());
}
