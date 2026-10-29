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

#[test]
fn refer_escape_suppressed_recovers_on_backspace() {
    let mut lx = Lx::new();
    lx.chat
        .messages
        .push(user_message("test", Some(("src/main.rs", false))));
    lx.insert_char('m');
    lx.insert_char('a');
    lx.insert_char('i');
    assert!(lx.refer_panel().is_some());

    assert!(lx.refer_escape());
    assert!(lx.refer_panel().is_none());
    assert!(lx.refer_suppressed);

    // 按退格键删除字符解除压制并重新激活面板
    lx.backspace();
    assert!(!lx.refer_suppressed);
    assert!(lx.refer_panel().is_some());
}

#[test]
fn refer_panel_upgrades_short_path_when_history_has_full_path() {
    let mut lx = Lx::new();
    // 历史中有完整路径
    lx.chat.messages.push(user_message(
        "history",
        Some(("src/ui/main_content/lx/refer.rs", false)),
    ));
    // 输入框中先出现了末两段形式的快捷引用
    lx.insert_str("@[ref-content]/lx/refer.rs ");
    // 用户再次输入 ref 触发
    lx.insert_char('r');
    lx.insert_char('e');

    let panel = lx.refer_panel().expect("panel open");
    // 应当成功去重，且 path 升级为全相对路径
    assert_eq!(panel.items().len(), 1);
    assert_eq!(panel.items()[0].path, "src/ui/main_content/lx/refer.rs");
}

#[test]
fn refer_scroll_moves_viewport() {
    let mut lx = Lx::new();
    // 塞入多个历史项
    for i in 0..10 {
        let name = format!("src/file_{i}.rs");
        lx.chat
            .messages
            .push(user_message("msg", Some((&name, false))));
    }
    lx.insert_char('f');
    lx.insert_char('i');

    let panel = lx.refer_panel().expect("panel open");
    assert_eq!(panel.items().len(), 10);
    assert_eq!(panel.viewport(), None);

    assert!(lx.refer_scroll(3, 0));
    assert_eq!(lx.refer_panel().unwrap().viewport(), Some(3));
}
