//! 参数解析测试：白名单、类型校验与上限钳制。

use super::*;

const ALLOWED: [&str; 2] = ["path", "limit"];

#[test]
fn rejects_non_object_and_unknown_fields() {
    assert!(Args::parse("[]", &ALLOWED).is_err());
    assert!(Args::parse("\"text\"", &ALLOWED).is_err());
    assert!(Args::parse("", &ALLOWED).is_err());
    let failure =
        Args::parse(r#"{"path":"a","content":"x"}"#, &ALLOWED).expect_err("unknown field");
    assert_eq!(failure.code, ToolErrorCode::InvalidArguments);
    assert!(failure.message.contains("content"));
}

#[test]
fn required_str_rejects_empty_and_wrong_types() {
    let args = Args::parse(r#"{"path":"a"}"#, &ALLOWED).expect("args");
    assert_eq!(args.required_str("path").expect("path"), "a");
    assert!(args.required_str("missing").is_err());

    let args = Args::parse(r#"{"path":"   "}"#, &ALLOWED).expect("args");
    assert!(args.required_str("path").is_err());

    let args = Args::parse(r#"{"path":7}"#, &ALLOWED).expect("args");
    assert!(args.required_str("path").is_err());
}

#[test]
fn required_text_allows_empty_string() {
    let args = Args::parse(r#"{"path":""}"#, &ALLOWED).expect("args");
    assert_eq!(args.required_text("path").expect("empty ok"), "");
}

#[test]
fn optional_field_parsing() {
    let args = Args::parse(
        r#"{"path":"a","limit":3,"flag":true}"#,
        &["path", "limit", "flag"],
    )
    .expect("args");
    assert_eq!(args.optional_positive("limit").expect("limit"), Some(3));
    assert_eq!(args.optional_bool("flag").expect("flag"), Some(true));
    assert_eq!(args.optional_str("path").expect("path"), Some("a"));
    assert_eq!(args.optional_str("absent").expect("absent"), None);
}

#[test]
fn optional_positive_rejects_zero_negative_and_float() {
    for value in ["0", "-2", "1.5", "\"3\"", "true"] {
        let json = format!(r#"{{"path":"a","limit":{value}}}"#);
        let args = Args::parse(&json, &ALLOWED).expect("args");
        assert!(
            args.optional_positive("limit").is_err(),
            "limit {value} must be rejected"
        );
    }
}

#[test]
fn optional_bool_rejects_other_types() {
    let args = Args::parse(r#"{"path":"a","flag":"yes"}"#, &["path", "flag"]).expect("args");
    assert!(args.optional_bool("flag").is_err());
}

#[test]
fn clamp_limit_caps_and_defaults() {
    assert_eq!(clamp_limit(None, 10), 10);
    assert_eq!(clamp_limit(Some(5), 10), 5);
    assert_eq!(clamp_limit(Some(50), 10), 10);
}
