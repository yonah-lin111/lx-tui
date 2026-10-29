use super::*;
use crate::app::state::ChatMessageView;

fn user_message(text_content: &str, attachment: Option<(&str, bool)>) -> ChatMessageView {
    let mut msg = ChatMessageView {
        sequence: 0,
        id: "msg1".into(),
        role: "user".into(),
        status: "complete".into(),
        turn_index: 0,
        kind: "chat".into(),
        shell: None,
        steps: Vec::new(),
        model_label: None,
        error_code: None,
        input_tokens: None,
        output_tokens: None,
        cache_read_tokens: None,
        duration_ms: None,
        reports: Vec::new(),
        attachments: Vec::new(),
    };
    if let Some((path, is_dir)) = attachment {
        msg.attachments
            .push(crate::db::chat::MessageAttachmentMeta {
                id: "att".into(),
                position: 0,
                kind: if is_dir {
                    crate::db::chat::MessageAttachmentKind::Folder
                } else {
                    crate::db::chat::MessageAttachmentKind::File
                },
                label: path.into(),
                marker_start: None,
                marker_end: None,
                source_path: Some(path.into()),
                mime: None,
                width: None,
                height: None,
            });
    }
    if !text_content.is_empty() {
        msg.steps.push(crate::app::state::AssistantStepView::Part {
            position: 0,
            part: crate::app::state::ChatPartView {
                kind: crate::chat::events::PartKind::Text,
                content: text_content.into(),
                provider_metadata_json: None,
                started_at_ms: None,
                ended_at_ms: None,
                completed_at_ms: None,
            },
        });
    }
    msg
}

#[test]
fn refer_panel_triggers_from_user_history_and_confirms() {
    let mut lx = Lx::new();
    lx.chat
        .messages
        .push(user_message("看下历史消息", Some(("src/main.rs", false))));

    lx.insert_char('m');
    assert!(lx.refer_panel().is_none(), "1个字符不触发快捷面板");

    lx.insert_char('a');
    let panel = lx.refer_panel().expect("2个字符命中历史中的 main.rs");
    assert_eq!(panel.items().len(), 1);
    assert_eq!(panel.items()[0].path, "src/main.rs");

    assert!(lx.refer_confirm());
    assert_eq!(lx.input, "@[ref-content]/src/main.rs ");
    assert_eq!(lx.cursor, lx.input.len());
    assert!(lx.refer_panel().is_none(), "确认后面板关闭");

    // 单步撤销
    lx.undo();
    assert_eq!(lx.input, "ma", "单步撤销回触发词");
}

#[test]
fn refer_quick_backspace_deletes_ref_content_as_one_block() {
    let mut lx = Lx::new();
    lx.chat
        .messages
        .push(user_message("看下", Some(("src/config.rs", false))));
    lx.insert_char('c');
    lx.insert_char('o');
    assert!(lx.refer_confirm());
    assert_eq!(lx.input, "@[ref-content]/src/config.rs ");

    // 光标紧跟空格，按 Backspace
    lx.backspace();
    assert_eq!(lx.input, "", "一键删除整段快捷引用");
}

#[test]
fn refer_quick_backspace_supports_mention_and_ref_file() {
    let mut lx = Lx::new();
    lx.insert_str("前文 @src/lib.rs ");
    lx.backspace();
    assert_eq!(lx.input, "前文 ");

    lx.insert_str("@[refer-file](src/other.rs) ");
    lx.backspace();
    assert_eq!(lx.input, "前文 ");
}

#[test]
fn refer_escape_closes_and_suppresses_until_cursor_move() {
    let mut lx = Lx::new();
    lx.chat
        .messages
        .push(user_message("test", Some(("src/main.rs", false))));
    lx.insert_char('m');
    lx.insert_char('a');
    assert!(lx.refer_panel().is_some());

    assert!(lx.refer_escape());
    assert!(lx.refer_panel().is_none());
    assert!(lx.refer_suppressed);

    // 移动光标解除压制
    lx.move_left();
    assert!(!lx.refer_suppressed);
}
