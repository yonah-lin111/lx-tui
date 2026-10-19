//! 全局 prompt 历史的状态测试：去重置顶、上限裁剪、过滤与载入顺序。

use super::{PROMPT_HISTORY_LIMIT, PROMPT_HISTORY_MAX_BYTES, PromptHistory};

#[test]
fn record_appends_in_order() {
    let mut history = PromptHistory::new();
    assert!(history.is_empty());
    assert!(history.record("first"));
    assert!(history.record("second"));
    assert_eq!(
        history.entries(),
        &["first".to_string(), "second".to_string()]
    );
    assert_eq!(history.get(0), Some("first"));
    assert_eq!(history.get(1), Some("second"));
    assert_eq!(history.get(2), None);
    assert_eq!(history.len(), 2);
}

#[test]
fn record_trims_whitespace() {
    let mut history = PromptHistory::new();
    assert!(history.record("  padded prompt \n"));
    assert_eq!(history.entries(), &["padded prompt".to_string()]);
}

#[test]
fn record_ignores_blank_and_oversized() {
    let mut history = PromptHistory::new();
    assert!(!history.record(""));
    assert!(!history.record("   \n\t "));
    assert!(history.is_empty());
    let oversized = "x".repeat(PROMPT_HISTORY_MAX_BYTES + 1);
    assert!(!history.record(&oversized));
    assert!(history.is_empty());
    // 恰好等于上限仍然可以记录。
    let at_limit = "y".repeat(PROMPT_HISTORY_MAX_BYTES);
    assert!(history.record(&at_limit));
    assert_eq!(history.len(), 1);
}

#[test]
fn record_deduplicates_to_newest() {
    let mut history = PromptHistory::new();
    history.record("alpha");
    history.record("beta");
    history.record("gamma");
    // 重复 alpha：移除旧位置并追加到末尾（最近使用置顶）。
    assert!(history.record("alpha"));
    assert_eq!(
        history.entries(),
        &["beta".to_string(), "gamma".to_string(), "alpha".to_string()]
    );
    assert_eq!(history.len(), 3);
}

#[test]
fn record_caps_to_limit_dropping_oldest() {
    let mut history = PromptHistory::new();
    for index in 0..PROMPT_HISTORY_LIMIT + 5 {
        history.record(&format!("prompt {index}"));
    }
    assert_eq!(history.len(), PROMPT_HISTORY_LIMIT);
    // 最旧的 5 条被丢弃，末尾仍是最新一条。
    assert_eq!(history.get(0), Some("prompt 5"));
    assert_eq!(
        history.get(PROMPT_HISTORY_LIMIT - 1),
        Some(format!("prompt {}", PROMPT_HISTORY_LIMIT + 4).as_str())
    );
}

#[test]
fn replace_all_keeps_input_order_and_filters() {
    let mut history = PromptHistory::new();
    history.record("stale");
    history.replace_all(vec![
        "old".to_string(),
        "  spaced  ".to_string(),
        String::new(),
        "new".to_string(),
        "old".to_string(),
    ]);
    // 空白被丢弃；重复项按 record 口径去重置顶，因此 "old" 落在末尾。
    assert_eq!(
        history.entries(),
        &["spaced".to_string(), "new".to_string(), "old".to_string()]
    );
}

#[test]
fn replace_all_with_empty_clears() {
    let mut history = PromptHistory::new();
    history.record("something");
    history.replace_all(Vec::new());
    assert!(history.is_empty());
}
