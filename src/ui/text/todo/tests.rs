//! Todo 文案守卫：状态框等宽、按钮标签纯 ASCII、tooltip 非空。

use super::*;

#[test]
fn todo_labels_align_to_three_columns() {
    for label in [TODO_STATUS_TODO, TODO_STATUS_IN_PROGRESS, TODO_STATUS_DONE] {
        assert_eq!(label.chars().count(), 3, "{label:?} 必须是 3 列");
    }
}

#[test]
fn todo_buttons_are_ascii_words() {
    for label in [TODO_NEW_LABEL, TODO_EDIT_LABEL, TODO_DELETE_LABEL] {
        assert!(label.starts_with('[') && label.ends_with(']'), "{label}");
        assert!(
            label[1..label.len() - 1]
                .chars()
                .all(|ch| ch.is_ascii_lowercase()),
            "{label} 只含小写 ASCII"
        );
    }
    assert_eq!(TODO_CLEAR_DONE_LABEL, "[clear done]");
}

#[test]
fn todo_tooltips_are_non_empty_ascii() {
    for hint in [
        TODO_TOOLTIP_NEW,
        TODO_TOOLTIP_EDIT,
        TODO_TOOLTIP_DELETE,
        TODO_TOOLTIP_CLEAR_DONE,
    ] {
        assert!(!hint.is_empty() && hint.is_ascii(), "{hint:?}");
    }
}

#[test]
fn stats_labels_are_lowercase_ascii_words() {
    for label in [
        TODO_STATS_TOTAL_LABEL,
        TODO_STATS_DOING_LABEL,
        TODO_STATS_DONE_LABEL,
    ] {
        assert!(
            label.chars().all(|ch| ch.is_ascii_lowercase()),
            "{label:?} 只含小写 ASCII"
        );
    }
    assert_eq!(TODO_STATS_SEPARATOR, " · ", "段间分隔符与身份标签同款");
    assert_eq!(TODO_STATS_PERCENT_SUFFIX, "%");
    assert!(TODO_EDIT_PLACEHOLDER.is_ascii() && !TODO_EDIT_PLACEHOLDER.is_empty());
}
