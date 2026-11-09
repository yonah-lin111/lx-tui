use super::*;

fn exchange(user: &str, assistant: &str) -> RecapExchange {
    RecapExchange {
        user: user.to_string(),
        assistant: assistant.to_string(),
    }
}

fn model() -> ModelSnapshot {
    ModelSnapshot {
        provider: "p".into(),
        model: "m".into(),
        variant: None,
    }
}

#[test]
fn prompt_keeps_only_recent_eight_exchanges() {
    let exchanges: Vec<RecapExchange> = (0..10)
        .map(|index| exchange(&format!("user-{index}"), &format!("assistant-{index}")))
        .collect();
    let prompt = build_recap_prompt(None, &exchanges).expect("prompt");
    assert!(!prompt.contains("user-0"), "{prompt}");
    assert!(!prompt.contains("user-1"), "{prompt}");
    assert!(prompt.contains("user-2"), "{prompt}");
    assert!(prompt.contains("user-9"), "{prompt}");
}

#[test]
fn prompt_includes_compact_summary() {
    let prompt = build_recap_prompt(Some("earlier goal"), &[exchange("u", "a")]).expect("prompt");
    assert!(prompt.contains("Earlier context (compacted):"), "{prompt}");
    assert!(prompt.contains("earlier goal"), "{prompt}");
    assert!(prompt.contains("User: u"), "{prompt}");
    assert!(prompt.contains("Assistant: a"), "{prompt}");
}

#[test]
fn prompt_omits_old_exchanges_when_over_budget() {
    let long = "x".repeat(RECAP_MAX_BYTES / 2);
    let exchanges = vec![
        exchange(&long, &long),
        exchange("new-user", "new-assistant"),
    ];
    let prompt = build_recap_prompt(None, &exchanges).expect("prompt");
    assert!(prompt.len() <= RECAP_MAX_BYTES + PROMPT_PREFIX.len());
    assert!(prompt.contains("new-user"), "最新交换必须保留");
}

#[test]
fn parse_json_recap() {
    let parsed = parse_generated_recap(
        r#"{"summary":"Done. Tests not run.","next_action":"Run cargo test"}"#,
    )
    .expect("json");
    assert_eq!(parsed.summary, "Done. Tests not run.");
    assert_eq!(parsed.next_action.as_deref(), Some("Run cargo test"));
}

#[test]
fn parse_fenced_json_and_null_next_action() {
    let parsed =
        parse_generated_recap("```json\n{\"summary\":\"Done.\",\"next_action\":null}\n```")
            .expect("fenced");
    assert_eq!(parsed.summary, "Done.");
    assert_eq!(parsed.next_action, None);
}

#[test]
fn parse_plain_text_fallback() {
    let parsed = parse_generated_recap("  A plain summary line  \nsecond line").expect("plain");
    assert_eq!(parsed.summary, "A plain summary line");
    assert_eq!(parsed.next_action, None);
}

#[test]
fn parse_rejects_empty_and_truncates_overlong_plain_text() {
    assert!(parse_generated_recap("").is_none());
    let long = parse_generated_recap(&"x".repeat(RECAP_MAX_SUMMARY_CHARS + 1)).expect("truncated");
    assert_eq!(long.summary.chars().count(), RECAP_MAX_SUMMARY_CHARS);
    assert!(parse_generated_recap(r#"{"summary":"ok","next_action":"x"}"#).is_some());
}

#[test]
fn parse_json_with_extra_fields_and_surrounding_text() {
    let parsed = parse_generated_recap(
        "Here is the recap:\n{\"summary\":\"Done.\",\"next_action\":null,\"confidence\":0.9}\nEnd.",
    )
    .expect("json");
    assert_eq!(parsed.summary, "Done.");
    assert_eq!(parsed.next_action, None);
}

#[test]
fn parse_truncates_overlong_json_fields() {
    let summary = "s".repeat(RECAP_MAX_SUMMARY_CHARS + 10);
    let next_action = "n".repeat(RECAP_MAX_NEXT_ACTION_CHARS + 10);
    let raw = format!("{{\"summary\":\"{summary}\",\"next_action\":\"{next_action}\"}}");
    let parsed = parse_generated_recap(&raw).expect("truncated");
    assert_eq!(parsed.summary.chars().count(), RECAP_MAX_SUMMARY_CHARS);
    assert_eq!(
        parsed
            .next_action
            .as_deref()
            .map(|action| action.chars().count()),
        Some(RECAP_MAX_NEXT_ACTION_CHARS)
    );
}

#[test]
fn parse_rejects_broken_json_fragments() {
    assert!(parse_generated_recap("{").is_none());
    assert!(parse_generated_recap("{\n\"summary\":").is_none());
    assert!(parse_generated_recap("\"unterminated").is_none());
}

#[test]
fn request_has_no_tools_and_compact_purpose() {
    let request = build_recap_request("prompt", &model());
    assert!(!request.includes_tools());
    assert_eq!(request.purpose, RequestPurpose::CompactSummary);
    assert_eq!(request.system_prompt.as_deref(), Some(RECAP_SYSTEM_PROMPT));
}
