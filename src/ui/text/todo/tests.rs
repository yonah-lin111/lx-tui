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
    // `[+]` 是加号图标（新建），`[sort]` 为小写单词。
    assert_eq!(TODO_NEW_LABEL, "[+]");
    assert_eq!(TODO_SORT_LABEL, "[sort]");
    for label in [TODO_NEW_LABEL, TODO_SORT_LABEL] {
        assert!(label.is_ascii(), "{label} 必须纯 ASCII");
    }
}

#[test]
fn todo_tooltips_are_non_empty_ascii() {
    for hint in [TODO_TOOLTIP_NEW, TODO_TOOLTIP_SORT] {
        assert!(!hint.is_empty() && hint.is_ascii(), "{hint:?}");
    }
}

#[test]
fn todo_placeholder_is_non_empty_ascii() {
    assert!(TODO_EDIT_PLACEHOLDER.is_ascii() && !TODO_EDIT_PLACEHOLDER.is_empty());
}
