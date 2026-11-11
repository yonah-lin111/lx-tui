//! BTW 视图：惰性构造、按 QA 独立草稿与焦点轮转。

use super::*;

#[test]
fn default_state_does_not_construct_a_composer() {
    // `Lx::new()` 必须终止：`LxBtw::default` 不创建内层 `Lx`。
    let lx = Lx::new();
    assert!(lx.btw.is_empty());
    assert!(lx.btw.active_composer().is_none());
}

#[test]
fn each_qa_keeps_its_own_draft() {
    let mut btw = LxBtw::default();
    btw.ensure_thread("qa-1")
        .composer
        .editor_mut()
        .replace_input("first draft");
    btw.ensure_thread("qa-2")
        .composer
        .editor_mut()
        .replace_input("second draft");

    assert_eq!(btw.active.as_deref(), Some("qa-2"));
    assert_eq!(
        btw.active_composer().map(|editor| editor.input.as_str()),
        Some("second draft")
    );
    assert!(btw.select_qa("qa-1"));
    assert_eq!(
        btw.active_composer().map(|editor| editor.input.as_str()),
        Some("first draft")
    );
    assert_eq!(btw.len(), 2);
}

#[test]
fn clearing_removes_threads_and_pending_flags() {
    let mut btw = LxBtw::default();
    btw.ensure_thread("qa-1");
    btw.pending_prompt = true;
    btw.clear();
    assert!(btw.is_empty());
    assert!(btw.active.is_none());
    assert!(!btw.pending_prompt);
}

#[test]
fn focus_cycles_in_both_directions() {
    assert_eq!(BtwFocus::Composer.next(), BtwFocus::QaList);
    assert_eq!(BtwFocus::QaList.next(), BtwFocus::Conversation);
    assert_eq!(BtwFocus::Conversation.next(), BtwFocus::Composer);
    assert_eq!(BtwFocus::Composer.previous(), BtwFocus::Conversation);
}
